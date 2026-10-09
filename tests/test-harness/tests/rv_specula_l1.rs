//! Second opinion on Specula lending MC-1: a debt leg repaid in full is
//! credited at its ceiling-rounded token amount (`calculate_repayment_amounts`:
//! `unscale_borrow_ceil`, then `usd_value_wad`), while the account's risk debt
//! values the same leg with `position_value_ceil` at WAD precision. The gap is
//! below one token unit of the debt asset and buys `gap * (1 + bonus)` of
//! collateral. At the HF-preserving cap the bonus itself cannot lower `C / D`,
//! so that gap is the one term that can push a solvent account across
//! `C = D` in a single liquidation.
//!
//! The probe lists a 3-decimal borrowable market (the coarsest the controller
//! admits), lets a leg accrue to a fractional balance, prices the collateral a
//! quarter of the gap above the debt, and closes the coarse leg at its ceiling
//! through the public `liquidate` entry point. It pins the HEAD behaviour: the
//! estimate credits the ceiling, the account ends with `C < D`, and the next
//! quote is the insolvent one at the base bonus. The `C < D` assertion is the
//! one that must flip once the credited value is capped at the cleared debt.

use common::constants::WAD;
use common::math::fp::{Ray, Wad};
use common::rates::{position_value_ceil, unscale_borrow_ceil};
use common::types::SeizeMode;
use soroban_sdk::vec;
use test_harness::presets::{MarketPreset, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS};
use test_harness::{eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB, CAROL};

const LOW3: &str = "LOW3";
const LOW3_DECIMALS: u32 = 3;
/// One raw LOW3 unit at $1 per token, in WAD USD.
const LOW3_UNIT_USD: i128 = WAD / 1_000;
const USDC_UNIT: i128 = 10_000_000;

/// Borrowable market at `MIN_BORROWABLE_ASSET_DECIMALS`, priced at $1.
fn low3() -> MarketPreset {
    MarketPreset {
        name: LOW3,
        decimals: LOW3_DECIMALS,
        price_wad: WAD,
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// The LOW3 debt leg as the controller sees it: `(risk value, credited value,
/// ceiling units)`. The risk value is `position_value_ceil` at WAD precision,
/// as `calculate_account_risk_totals` sums it; the credited value is the
/// ceiling token amount priced by `usd_value_wad`, as the repayment plan
/// records it.
fn low3_leg(t: &LendingTest, account_id: u64) -> (i128, i128, i128) {
    let asset = t.resolve_asset(LOW3);
    let (_, borrows) = t.ctrl_client().get_account_positions(&account_id);
    let scaled = Ray::from(
        borrows
            .get(hub_asset(asset.clone()))
            .expect("LOW3 debt leg")
            .scaled_amount,
    );
    let index = Ray::from(
        t.pool_client(LOW3)
            .get_sync_data(&hub_asset(asset))
            .state
            .borrow_index,
    );
    let price = Wad::from(t.resolve_market(LOW3).price_wad);
    let risk = position_value_ceil(&t.env, scaled, index, price).raw();
    let ceil_units = unscale_borrow_ceil(&t.env, scaled, index, LOW3_DECIMALS);
    let credited = Wad::from_token(&t.env, ceil_units, LOW3_DECIMALS)
        .mul(&t.env, price)
        .raw();
    (risk, credited, ceil_units)
}

/// A solvent account (`C >= D`, `HF < 1`) whose coarse debt leg is closed at
/// its ceiling ends the call with `C < D`: the plan credits the ceiling, the
/// seizure buys collateral with the ceiling, and the risk books clear only the
/// WAD-precision debt. The band quote pays no bonus, so nothing else moves
/// `C / D`. The next quote is then the insolvent one at the base bonus.
#[test]
fn closing_a_coarse_leg_at_its_ceiling_pushes_a_solvent_account_below_c_equals_d() {
    let mut t = LendingTest::new()
        .with_market(low3())
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Real supply shares behind both debt markets.
    t.supply(BOB, LOW3, 10_000.0);
    t.supply(BOB, "USDC", 10_000.0);

    // One ETH of collateral and two debt legs; the coarse leg is the large one.
    t.supply(ALICE, "ETH", 1.0);
    t.borrow(ALICE, LOW3, 700.0);
    t.borrow(ALICE, "USDC", 100.0);
    let alice = t.resolve_account_id(ALICE);

    // Accrue until the LOW3 ceiling gap is at least 0.4 of a raw unit, so the
    // 1e-7 ETH floor on the seizure (about $0.00008) cannot mask it.
    let gap_floor = LOW3_UNIT_USD * 4 / 10;
    let mut gap = 0;
    for _ in 0..120 {
        t.advance_and_sync(86_400);
        let (risk, credited, _) = low3_leg(&t, alice);
        gap = credited - risk;
        if gap >= gap_floor {
            break;
        }
    }
    assert!(
        gap >= gap_floor && gap < LOW3_UNIT_USD,
        "ceiling gap {gap} outside [0.4, 1) LOW3 units"
    );
    let (risk, credited, ceil_units) = low3_leg(&t, alice);
    assert!(
        credited > risk,
        "the ceiling credit exceeds the WAD risk debt"
    );

    // Price the only collateral a quarter of the gap above the debt: solvent,
    // and far below HF 1 at an 80% threshold.
    let debt_quote = t.ctrl_client().get_total_borrow_usd(&alice);
    t.set_price("ETH", debt_quote + gap / 4);
    let coll_pre = t.ctrl_client().get_total_collateral_usd(&alice);
    let debt_pre = t.ctrl_client().get_total_borrow_usd(&alice);
    assert!(
        coll_pre >= debt_pre && coll_pre - debt_pre <= gap / 2,
        "pre: C {coll_pre} must sit within half a gap above D {debt_pre}"
    );
    assert!(t.can_be_liquidated(ALICE), "HF below 1 before the call");

    // Offer exactly the coarse leg's ceiling.
    let carol = t.get_or_create_user(CAROL);
    let low3_market = t.resolve_market(LOW3);
    low3_market.token_admin.mint(&carol, &ceil_units);
    let payments = vec![&t.env, (hub_asset(low3_market.asset.clone()), ceil_units)];

    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&alice, &payments, &SeizeMode::Transfer);
    assert_eq!(
        estimate.max_payment_wad, credited,
        "HEAD credits the ceiling-rounded leg, not the debt it clears ({risk})"
    );
    assert_eq!(
        estimate.bonus_rate_bps, 0,
        "a covered account at C ~ D takes the band quote at a zero cap"
    );

    t.ctrl_client()
        .liquidate(&carol, &alice, &payments, &SeizeMode::Transfer);

    assert_eq!(
        t.borrow_balance_raw(ALICE, LOW3),
        0,
        "the coarse leg closed at its ceiling"
    );
    assert!(
        t.account_exists(alice),
        "the residue is above the $5 dust cap, so no cleanup ran"
    );
    let coll_post = t.ctrl_client().get_total_collateral_usd(&alice);
    let debt_post = t.ctrl_client().get_total_borrow_usd(&alice);
    let collateral_taken = coll_pre - coll_post;
    let debt_cleared = debt_pre - debt_post;
    assert!(
        collateral_taken > debt_cleared,
        "at a zero bonus the seizure {collateral_taken} still exceeds the debt cleared {debt_cleared}"
    );
    assert!(
        collateral_taken - debt_cleared <= LOW3_UNIT_USD,
        "the excess is bounded by one LOW3 unit"
    );
    assert!(
        coll_post < debt_post,
        "MC-1 at HEAD: one liquidation took the account from C {coll_pre} >= D {debt_pre} \
         to C {coll_post} < D {debt_post}"
    );

    // The next liquidation quotes the insolvent path: base bonus, below the debt.
    let usdc_market = t.resolve_market("USDC");
    let usdc_offer = vec![
        &t.env,
        (hub_asset(usdc_market.asset.clone()), 100 * USDC_UNIT),
    ];
    let next = t
        .ctrl_client()
        .get_liquidation_estimate(&alice, &usdc_offer, &SeizeMode::Transfer);
    assert_eq!(
        next.bonus_rate_bps,
        i128::from(DEFAULT_ASSET_CONFIG.liquidation_bonus),
        "the insolvent quote pays the base bonus"
    );
    assert!(
        next.max_payment_wad > 0 && next.max_payment_wad < debt_post,
        "the insolvent quote {} is the collateral-backed amount below D {debt_post}",
        next.max_payment_wad
    );
}
