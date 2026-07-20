//! Independent exploit proof (separate from audit_liquidate_dust_fee_dos.rs).
//!
//! Hypothesis: a collateral leg whose live value sits in [0.5, 1.0) raw asset
//! units bricks full-close liquidate(). The controller full-close branch
//! (math.rs:284) emits amount = half-up(value) = 1 and bumps a sub-unit
//! protocol fee to 1 (math.rs:295). The pool resolves the same leg on
//! full-close via resolve_withdrawal -> gross = floor(value) = 0, then
//! apply_liquidation_fee asserts 0 >= 1 and panics WithdrawLessThanFee (115),
//! reverting the entire liquidate() transaction (all seize legs batched into
//! one pool.withdraw).
//!
//! Faithful because the test-harness registers controller + pool natively, so
//! try_liquidate drives the real pool.withdraw / apply_liquidation_fee path.

use common::math::fp::Ray;
use controller::constants::RAY;
use soroban_sdk::Vec;
use test_harness::{errors, eth_preset, hub_asset, usdc_preset, xlm_preset, LendingTest};
use test_harness::{ALICE, BOB, CAROL, LIQUIDATOR};

fn xlm_index(t: &LendingTest) -> i128 {
    let asset = t.resolve_asset("XLM");
    let assets = Vec::from_array(&t.env, [hub_asset(asset)]);
    t.ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap()
        .supply_index
}

fn xlm_scaled(t: &LendingTest, who: &str) -> i128 {
    let account_id = t.resolve_account_id(who);
    let asset = t.resolve_asset("XLM");
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);
    supplies
        .get(hub_asset(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

#[test]
fn audit_liquidate_contracts_subunit_leg_bricks_full_close() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset()) // dominant collateral
        .with_market(eth_preset()) // borrowable debt
        .with_market(xlm_preset()) // collateral carrying liquidation_fees > 0
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Precondition: XLM ships a positive protocol liquidation fee.
    assert!(t.get_asset_config("XLM").liquidation_fees > 0);

    // Seed liquidity: ETH to borrow, XLM supply so its index can compound.
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "XLM", 100_000.0);

    // Victim: dominant USDC collateral + 1-raw-unit XLM leg (planted at index=RAY,
    // scaled = 1e20) + ETH debt. Healthy at planting time.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply_raw(ALICE, "XLM", 1);
    t.borrow(ALICE, "ETH", 2.0);

    // Drive XLM utilization so its supply index compounds into [1.5, 2.0) x RAY.
    t.supply(CAROL, "USDC", 50_000.0);
    t.borrow(CAROL, "XLM", 94_000.0);

    let mut grown = false;
    for _ in 0..1000 {
        if xlm_index(&t) >= 3 * RAY / 2 {
            grown = true;
            break;
        }
        t.advance_and_sync(30 * 86_400);
    }
    assert!(grown, "XLM supply index never reached 1.5x RAY");
    assert!(xlm_index(&t) < 2 * RAY, "XLM index overshot 2.0x RAY");

    // Partial-withdraw 1 raw unit -> residual value ~= (index-1) raw units, which
    // lands in [0.5, 1.0) for index in [1.5, 2.0). The sub-unit leg survives.
    t.withdraw_raw(ALICE, "XLM", 1);

    // Confirm the rounding split that bricks the pool: half-up -> 1, floor -> 0.
    let scaled = xlm_scaled(&t, ALICE);
    let value_ray = Ray::from(scaled).mul(&t.env, Ray::from(xlm_index(&t)));
    let half_up = value_ray.to_asset(7);
    let floor = value_ray.to_asset_floor(7);
    std::println!("residual XLM leg: half_up={half_up} floor={floor} scaled={scaled}");
    assert_eq!(half_up, 1, "controller full-close emits amount = 1");
    assert_eq!(floor, 0, "pool full-close resolves gross = 0");

    // Push the victim into the solvent-toxic band (HF < 1, collateral just above
    // debt) so the protocol mandates a full close.
    let debt_wad = t.total_debt_raw(ALICE);
    let target_collateral_wad = debt_wad + debt_wad / 50; // 1.02x debt
    let usdc_price_wad = target_collateral_wad / 10_000;
    t.set_price("USDC", usdc_price_wad);
    assert!(
        t.can_be_liquidated(ALICE),
        "victim must be underwater; HF = {}",
        t.health_factor(ALICE)
    );

    // A partial repay is refused: the solvent-toxic band forces a full close.
    let partial = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    test_harness::assert_contract_error(partial, errors::FULL_CLOSE_REQUIRED);

    // POST-FIX regression (F3): the sub-unit XLM leg now carries fee 0 (clamped to
    // the pool's floor gross in `calculate_seized_collateral`), so the mandated full
    // close no longer trips WithdrawLessThanFee — liquidate() succeeds and reduces
    // the victim's debt. Pre-fix this reverted contract error 115.
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
