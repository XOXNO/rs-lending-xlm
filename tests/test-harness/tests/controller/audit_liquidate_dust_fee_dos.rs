use common::math::fp::Ray;
use controller::constants::RAY;
use soroban_sdk::Vec;
use test_harness::{errors, hub_asset, xlm_preset, LendingTest};
use test_harness::{ALICE, BOB, CAROL, LIQUIDATOR};

fn xlm_supply_index(t: &LendingTest) -> i128 {
    let asset = t.resolve_asset("XLM");
    let ctrl = t.ctrl_client();
    let assets = Vec::from_array(&t.env, [hub_asset(asset)]);
    ctrl.get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap()
        .supply_index
}

fn alice_xlm_scaled(t: &LendingTest) -> i128 {
    let account_id = t.resolve_account_id(ALICE);
    let asset = t.resolve_asset("XLM");
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);
    supplies
        .get(hub_asset(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

#[test]
fn audit_liquidate_contracts_dust_fee_full_close_dos() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(xlm_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    assert_eq!(t.get_asset_config("XLM").liquidation_fees, 1200);

    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "XLM", 100_000.0);

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply_raw(ALICE, "XLM", 1);
    t.borrow(ALICE, "ETH", 2.0);

    t.supply(CAROL, "USDC", 50_000.0);
    t.borrow(CAROL, "XLM", 94_000.0);

    let mut grown = false;
    for _ in 0..1000 {
        if xlm_supply_index(&t) >= 3 * RAY / 2 {
            grown = true;
            break;
        }
        t.advance_and_sync(30 * 86_400);
    }
    let idx = xlm_supply_index(&t);
    assert!(grown, "XLM supply index never reached 1.5x RAY");
    assert!(idx < 2 * RAY, "XLM supply index overshot 2.0x RAY: {idx}");

    t.withdraw_raw(ALICE, "XLM", 1);

    let scaled = alice_xlm_scaled(&t);
    let value_ray = Ray::from(scaled).mul(&t.env, Ray::from(xlm_supply_index(&t)));
    let half_up = value_ray.to_asset(7);
    let floor = value_ray.to_asset_floor(7);
    std::println!("residual XLM leg: half_up={half_up} floor={floor} (scaled={scaled})");
    assert_eq!(half_up, 1, "residual must round half-up to 1 raw unit");
    assert_eq!(floor, 0, "residual must floor to 0 raw units");

    let debt_wad = t.total_debt_raw(ALICE);
    let target_collateral_wad = debt_wad + debt_wad / 50;
    let usdc_price_wad = target_collateral_wad / 10_000;
    t.set_price("USDC", usdc_price_wad);

    assert!(
        t.can_be_liquidated(ALICE),
        "victim must be underwater (HF < 1): HF = {}",
        t.health_factor(ALICE)
    );

    let partial = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    test_harness::assert_contract_error(partial, errors::FULL_CLOSE_REQUIRED);

    let before = t.borrow_balance(ALICE, "ETH");
    let full = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 5.0);
    assert!(
        full.is_ok(),
        "post-fix: sub-unit leg full-close must not brick liquidate; got {full:?}"
    );
    assert!(
        t.borrow_balance(ALICE, "ETH") < before,
        "liquidation must reduce the victim's ETH debt"
    );
}
