//! RV second opinion on Specula lending MC-2: a *partial* Transfer-mode seizure
//! whose floored token amount reaches the pool's half-up supply balance is
//! promoted by `resolve_withdrawal` to a full close. The pool burns every share
//! of the leg and pays the floor, so the account's sub-unit share remainder
//! (always below half of one asset unit) stays in the pool as cash that no
//! share backs.
//!
//! The test drives the deployed contracts end to end. It reproduces the
//! window at the real scale (7-decimal assets, 1e27 RAY index) and pins the
//! two facts the second opinion rests on:
//!
//! 1. The plan is partial (`max_payment_wad` below the debt, no `seize_all`),
//!    the planned token amount equals the half-up balance, and the pool still
//!    burns the whole position instead of the documented `ceil(amount/index)`.
//! 2. The residual debt is socialized regardless of the mode: the same offer in
//!    Credit mode moves floor shares and leaves the remainder with the account,
//!    yet the account is still cleaned up under the $5 dust cap.

use common::math::fp::Ray;
use common::rates::{calculate_scaled_supply_ceil, unscale_supply, unscale_supply_floor};
use common::types::{PoolStateRaw, SeizeMode};
use test_harness::{
    asset_payment_vec, hub_asset, usd, LendingTest, ALICE, BOB, CAROL, DAVE, LIQUIDATOR,
};

const DAY_SECS: u64 = 86_400;
const DECIMALS: u32 = 7;
/// One raw asset unit at 7 decimals, in RAY (1e27 / 1e7).
const UNIT_RAY: i128 = 100_000_000_000_000_000_000;
/// WAD USD per raw USDC unit at a $1 price with 7 decimals (1e18 / 1e7).
const USDC_UNIT_WAD: i128 = 100_000_000_000;
const ONE_ETH_RAW: i128 = 10_000_000;

fn pool_state(t: &LendingTest, asset_name: &str) -> PoolStateRaw {
    let asset = t.resolve_asset(asset_name);
    t.pool_client(asset_name)
        .get_sync_data(&hub_asset(asset))
        .state
}

fn scaled_supply(t: &LendingTest, account_id: u64, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.ctrl_client()
        .get_account_positions(&account_id)
        .0
        .get(hub_asset(asset))
        .map_or(0, |p| p.scaled_amount)
}

/// The pool's three views of one supply leg: exact RAY value (half-up mul),
/// half-up token units (`unscale_supply`) and floor token units.
struct LegView {
    actual_ray: i128,
    half_up_units: i128,
    floor_units: i128,
}

fn leg_view(t: &LendingTest, scaled: i128, index: i128) -> LegView {
    let env = &t.env;
    let scaled = Ray::from(scaled);
    let index = Ray::from(index);
    LegView {
        actual_ray: scaled.mul(env, index).raw(),
        half_up_units: unscale_supply(env, scaled, index, DECIMALS),
        floor_units: unscale_supply_floor(env, scaled, index, DECIMALS),
    }
}

/// Builds two identical insolvent borrowers (ALICE, CAROL) whose single ETH
/// collateral leg carries a sub-half-unit share remainder. Returns the ETH
/// supply index at liquidation time.
fn build_window_book(t: &mut LendingTest) -> i128 {
    // Dave is the ETH supplier that lets Bob borrow ETH at sane utilization, which is
    // what moves the ETH supply index off 1.0 and gives the borrowers a fractional
    // share value.
    t.supply(DAVE, "ETH", 100.0);
    t.supply(BOB, "USDC", 200_000.0);

    // Both borrowers mint shares at index 1.0, so S = 1 ETH exactly in RAY.
    t.supply_raw(ALICE, "ETH", ONE_ETH_RAW);
    t.borrow(ALICE, "USDC", 1_400.0);
    t.supply_raw(CAROL, "ETH", ONE_ETH_RAW);
    t.borrow(CAROL, "USDC", 1_400.0);

    t.borrow(BOB, "ETH", 50.0);

    let alice = t.resolve_account_id(ALICE);
    let carol = t.resolve_account_id(CAROL);
    let shares = scaled_supply(t, alice, "ETH");
    assert_eq!(
        shares,
        scaled_supply(t, carol, "ETH"),
        "twins must hold equal shares"
    );
    assert_eq!(shares, ONE_ETH_RAW * UNIT_RAY, "shares minted at index 1.0");

    // Walk the index forward a day at a time until the remainder of the leg's value
    // below its floor unit is at least 0.01 units and below 0.5 units. The remainder
    // follows the low digits of the accrued index, so a few days suffice; the bound
    // only guards against an unlucky run.
    let mut index = 0;
    for _ in 0..120 {
        t.advance_and_sync(DAY_SECS);
        index = pool_state(t, "ETH").supply_index;
        let view = leg_view(t, shares, index);
        let remainder = view.actual_ray - view.floor_units * UNIT_RAY;
        if view.half_up_units == view.floor_units && remainder >= UNIT_RAY / 100 {
            return index;
        }
    }
    panic!("no sub-half-unit remainder found within 120 days (last index {index})");
}

/// Trims a Transfer plan's `max_payment_wad` to a USDC offer that is three raw units
/// below the collateral-backed quote: inside the plan's partial regime (no
/// `seize_all`, which allows one unit per leg of slack) but within the window.
fn partial_offer_below_quote(t: &LendingTest, account_id: u64, mode: SeizeMode) -> i128 {
    let usdc = t.resolve_asset("USDC");
    let probe = asset_payment_vec(&t.env, usdc, 10_000 * ONE_ETH_RAW);
    let quote = t
        .ctrl_client()
        .get_liquidation_estimate(&account_id, &probe, &mode);
    quote.max_payment_wad / USDC_UNIT_WAD - 3
}

#[test]
fn partial_transfer_seizure_at_the_half_up_balance_burns_the_whole_leg() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let index = build_window_book(&mut t);
    let alice = t.resolve_account_id(ALICE);
    let shares = scaled_supply(&t, alice, "ETH");
    let view = leg_view(&t, shares, index);
    let remainder_ray = view.actual_ray - view.floor_units * UNIT_RAY;

    // Collateral below debt: the plan quotes the collateral-backed repayment at the
    // base bonus, so a slightly smaller offer seizes almost the whole leg.
    t.set_price("ETH", usd(1_300));
    let collateral = t.total_collateral_raw(ALICE);
    let debt = t.total_debt_raw(ALICE);
    assert!(
        collateral < debt,
        "fixture must be insolvent: C={collateral} D={debt}"
    );

    let offer_raw = partial_offer_below_quote(&t, alice, SeizeMode::Transfer);
    let usdc = t.resolve_asset("USDC");
    let payments = asset_payment_vec(&t.env, usdc, offer_raw);
    let plan = t
        .ctrl_client()
        .get_liquidation_estimate(&alice, &payments, &SeizeMode::Transfer);

    // The plan is partial: the offer is kept untrimmed and it does not clear the debt.
    assert_eq!(
        plan.max_payment_wad,
        offer_raw * USDC_UNIT_WAD,
        "offer below the quote must be kept as offered"
    );
    assert!(plan.max_payment_wad < debt, "plan must leave debt behind");
    assert_eq!(
        plan.refunds.len(),
        0,
        "a partial plan below the quote refunds nothing"
    );
    let planned = plan.seized_collaterals.get(0).expect("one ETH seizure leg");
    let fee = plan.protocol_fees.get(0).map_or(0, |f| f.amount);

    // The window: the floored partial amount equals the pool's half-up balance, while
    // the leg still holds a positive remainder below half a unit in RAY.
    assert_eq!(
        view.half_up_units, view.floor_units,
        "remainder must be below half a unit"
    );
    assert_eq!(
        planned.amount, view.floor_units,
        "planned partial amount {} must equal the half-up balance {}",
        planned.amount, view.half_up_units
    );
    assert!(remainder_ray > 0 && remainder_ray < UNIT_RAY / 2);

    std::println!(
        "RV-SPECULA-L2 MC-2 window: shares={shares} index={index} actual_ray={} floor_units={} \
         remainder_ray={remainder_ray} offer_raw={offer_raw} planned_amount={} fee={fee} \
         max_payment_wad={} debt_wad={debt}",
        view.actual_ray,
        view.floor_units,
        planned.amount,
        plan.max_payment_wad
    );

    // What the documented partial rule would burn: ceil(amount / index) shares, which
    // leaves the remainder with the account.
    let partial_burn =
        calculate_scaled_supply_ceil(&t.env, planned.amount, DECIMALS, Ray::from(index));
    assert!(
        partial_burn.raw() < shares,
        "ceil burn {} must leave shares out of {}",
        partial_burn.raw(),
        shares
    );

    let eth_before_state = pool_state(&t, "ETH");
    let usdc_index_before = pool_state(&t, "USDC").supply_index;
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("USDC")
        .token_admin
        .mint(&liquidator, &offer_raw);
    let eth_before = t.token_balance_raw(LIQUIDATOR, "ETH");

    t.ctrl_client()
        .liquidate(&liquidator, &alice, &payments, &SeizeMode::Transfer);

    // The whole offer was pulled (no refund), the liquidator got the floored amount
    // minus the fee, and the pool burned every share of the leg: the remainder was
    // not left with the account.
    assert_eq!(t.token_balance_raw(LIQUIDATOR, "USDC"), 0);
    assert_eq!(
        t.token_balance_raw(LIQUIDATOR, "ETH") - eth_before,
        planned.amount - fee
    );
    // The fee is booked as revenue shares that stay inside `supplied` (INV-ACCT-01),
    // so the burn is the supplied delta plus the revenue delta. Cleanup found no
    // collateral shares left to reclassify, so revenue moved by the fee alone.
    let eth_after_state = pool_state(&t, "ETH");
    let burned = eth_before_state.supplied - eth_after_state.supplied
        + (eth_after_state.revenue - eth_before_state.revenue);
    assert_eq!(
        burned,
        shares,
        "pool burned {burned} shares; a partial close would burn {}",
        partial_burn.raw()
    );
    // The residual debt was socialized by the dust-capped cleanup, which removed the
    // account and wrote the loss into the USDC supply index.
    assert!(
        t.find_account_id(ALICE).is_none(),
        "insolvent residue under the dust cap must be cleaned up"
    );
    assert!(pool_state(&t, "USDC").supply_index < usdc_index_before);
}

#[test]
fn the_same_partial_offer_in_credit_mode_keeps_the_remainder_and_is_still_cleaned_up() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let index = build_window_book(&mut t);
    let carol = t.resolve_account_id(CAROL);
    let shares = scaled_supply(&t, carol, "ETH");
    let view = leg_view(&t, shares, index);

    t.set_price("ETH", usd(1_300));
    assert!(t.total_collateral_raw(CAROL) < t.total_debt_raw(CAROL));

    let offer_raw = partial_offer_below_quote(&t, carol, SeizeMode::Credit(0));
    let usdc = t.resolve_asset("USDC");
    let payments = asset_payment_vec(&t.env, usdc, offer_raw);
    let plan = t
        .ctrl_client()
        .get_liquidation_estimate(&carol, &payments, &SeizeMode::Credit(0));
    assert_eq!(plan.max_payment_wad, offer_raw * USDC_UNIT_WAD);
    let planned_shares = plan.seized_collaterals.get(0).expect("one leg").amount;
    // Credit floors once in share space, so the plan itself keeps the remainder.
    assert!(
        planned_shares < shares,
        "credit plan must not take every share"
    );
    assert_eq!(
        view.half_up_units, view.floor_units,
        "same window as the transfer twin"
    );

    let supplied_before = pool_state(&t, "ETH").supplied;
    let usdc_index_before = pool_state(&t, "USDC").supply_index;
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("USDC")
        .token_admin
        .mint(&liquidator, &offer_raw);

    let receiver = t
        .ctrl_client()
        .liquidate(&liquidator, &carol, &payments, &SeizeMode::Credit(0));
    assert!(receiver != 0, "credit mode must create a receiver");

    // No shares leave the pool in credit mode; the remainder stays booked until the
    // dust-capped cleanup reclassifies it as revenue and deletes the account. The
    // residual debt is socialized in both modes, so socialization is not caused by the
    // transfer-mode full burn.
    assert_eq!(pool_state(&t, "ETH").supplied, supplied_before);
    let credited = scaled_supply(&t, receiver, "ETH");
    assert!(credited > 0 && credited < shares);
    assert!(t.find_account_id(CAROL).is_none());
    assert!(pool_state(&t, "USDC").supply_index < usdc_index_before);
}
