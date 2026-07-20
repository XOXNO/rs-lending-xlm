//! Exploit proof for the surviving hypothesis:
//! "Full-close liquidation of a sub-unit collateral leg sends the pool an
//!  (amount, protocol_fee) pair it must reject — WithdrawLessThanFee bricks
//!  liquidate for the whole account."
//!
//! End-to-end join of the two halves already proven in isolation:
//!   * controller `dust_protocol_fee_rounds_up_to_one_unit` -> emits {amount:1, fee:1}
//!   * pool `test_withdraw_rejects_fee_greater_than_withdrawn_amount` -> reverts 115
//!
//! Mechanism: a collateral leg whose true value is in [0.5, 1.0) raw asset units
//! quantizes half-up to 1 in `calculate_seized_collateral` (full-close branch,
//! math.rs:284), and its positive sub-unit protocol fee is bumped to 1 raw unit
//! (math.rs:295). `LiquidationPlan::validate` accepts {1,1}. The pool resolves the
//! same leg on full-close via `resolve_withdrawal` -> gross = floor = 0, then
//! `apply_liquidation_fee` asserts 0 >= 1 and panics WithdrawLessThanFee (115).
//!
//! The genuinely-stuck regime is the SOLVENT-TOXIC band (HF < 1, total_collateral
//! in [total_debt, total_debt*(1+base_bonus))): partial repays revert
//! FullCloseRequired, and the mandated full close hits the poison leg. The account
//! is unliquidatable.

use common::math::fp::Ray;
use controller::constants::RAY;
use soroban_sdk::Vec;
use test_harness::{errors, eth_preset, hub_asset, usdc_preset, xlm_preset, LendingTest};
use test_harness::{ALICE, BOB, CAROL, LIQUIDATOR};

fn xlm_supply_index(t: &LendingTest) -> i128 {
    let asset = t.resolve_asset("XLM");
    let ctrl = t.ctrl_client();
    let assets = Vec::from_array(&t.env, [hub_asset(asset)]);
    ctrl.get_market_indexes_detailed(&assets).get(0).unwrap().supply_index
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
        .with_market(usdc_preset()) // robust dominant collateral
        .with_market(eth_preset()) // borrowable debt
        .with_market(xlm_preset()) // collateral WITH liquidation_fees = 100bps
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // XLM ships with the default 1% protocol liquidation fee (> 0) — the precondition.
    assert_eq!(t.get_asset_config("XLM").liquidation_fees, 100);

    // Whale seeds real supplied shares so XLM utilization (and thus its supply
    // index) can actually grow, plus ETH borrow liquidity for the victim.
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "XLM", 100_000.0);

    // Victim account: dominant USDC collateral, a 1-raw-unit XLM leg planted
    // NOW while the XLM index is still RAY (scaled = 1e20), and an ETH debt.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply_raw(ALICE, "XLM", 1); // the future poison leg
    t.borrow(ALICE, "ETH", 2.0); // ~$4000 debt vs $8000 weighted => healthy

    // Driver drives XLM utilization high so its supply index compounds upward.
    t.supply(CAROL, "USDC", 50_000.0);
    t.borrow(CAROL, "XLM", 94_000.0);

    // Grow the XLM supply index into [1.5, 2.0) x RAY. At index r x RAY, a later
    // partial withdraw of 1 raw unit from the 1-unit leg leaves residual value
    // ~= (r - 1) raw units, which lands in [0.5, 1.0) for r in [1.5, 2.0).
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

    // Partial-withdraw 1 raw unit while the victim is still healthy (USDC = $1).
    // update_or_remove_supply_position only prunes at exactly Ray::ZERO, so the
    // sub-unit residual survives as a live collateral leg.
    t.withdraw_raw(ALICE, "XLM", 1);

    // Confirm the residual sits in the poison band: half-up rounds to 1 raw unit
    // (what the controller full-close branch will send), floor rounds to 0 (what
    // the pool will resolve as gross). This IS the rounding split that bricks 115.
    let scaled = alice_xlm_scaled(&t);
    let value_ray = Ray::from(scaled).mul(&t.env, Ray::from(xlm_supply_index(&t)));
    let half_up = value_ray.to_asset(7);
    let floor = value_ray.to_asset_floor(7);
    std::println!("residual XLM leg: half_up={half_up} floor={floor} (scaled={scaled})");
    assert_eq!(half_up, 1, "residual must round half-up to 1 raw unit");
    assert_eq!(floor, 0, "residual must floor to 0 raw units");

    // Drop the victim into the SOLVENT-TOXIC band: HF < 1 while total_collateral
    // stays within [total_debt, total_debt*(1+base_bonus=5%)). Price USDC so the
    // USDC leg alone values ~1.02x the ETH debt.
    let debt_wad = t.total_debt_raw(ALICE);
    let target_collateral_wad = debt_wad + debt_wad / 50; // 1.02 x debt
    let usdc_price_wad = target_collateral_wad / 10_000; // 10_000 USDC tokens
    t.set_price("USDC", usdc_price_wad);

    assert!(
        t.can_be_liquidated(ALICE),
        "victim must be underwater (HF < 1): HF = {}",
        t.health_factor(ALICE)
    );

    // KEY ASSERTION 1: a partial repay is forbidden — the solvent-toxic band
    // forces a full close (FullCloseRequired), so a keeper cannot dodge the
    // poison leg by repaying a little.
    let partial = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    test_harness::assert_contract_error(partial, errors::FULL_CLOSE_REQUIRED);

    // KEY ASSERTION 2 (POST-FIX F3): the sub-unit XLM leg now carries fee 0 (clamped
    // to the pool's floor gross), so the mandated full close no longer trips
    // WithdrawLessThanFee — liquidate() succeeds and reduces the victim's debt.
    // Pre-fix this reverted contract error 115 and bricked the whole account.
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
