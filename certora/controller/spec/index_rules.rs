//! Index rules: zero-time stability, plus the V-9 view/accrue isomorphism and
//! time-monotonicity families (Certora Aave Hub P-09/P-10 analogues).
//!
//! ## Why the `iso_` family is written against `simulate_update_indexes`
//!
//! Production `get_market_index` reads the pool through `get_bulk_indexes`,
//! which is `simulate_update_indexes(now, load_sync_data())` — a *projection*
//! of the stored state forward to the current ledger time. The controller's
//! certora harness replaces the cross-contract call with a havoc summary
//! (`bulk_index_summary`, memoised per rule by `spec::ghost_prices`), so an
//! ABI-level "call the view twice" rule would compare one nondeterministic
//! value against itself and prove nothing.
//!
//! `common` is compiled *without* its `certora` feature in the controller's
//! certora build (`controller/Cargo.toml`'s `certora` feature does not pull
//! `common/certora`), so `common::rates::simulate_update_indexes` here is the
//! real implementation, not the monotone havoc summary the pool layer sees.
//! The `iso_`/`time_mono_` rules therefore model the pool response exactly as
//! production computes it and compare the two projections that a view would
//! return with and without a prior `update_indexes`.

use controller_interface::ControllerInterface;
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{vec, Address, Env, Map};

use crate::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, MAX_SUPPLY_INDEX_RAY, RAY, RAY_DECIMALS,
    SUPPLY_INDEX_FLOOR_RAW, WAD,
};
use crate::context::Cache;
use crate::spec::fixture;
use crate::types::{
    AccountPositionRaw, DebtPositionRaw, HubAssetKey, MarketIndex, MarketIndexRaw, MarketParamsRaw,
    PoolStateRaw, PoolSyncData,
};
use common::math::fp::Ray;
use common::rates::{simulate_update_indexes, MAX_COMPOUND_DELTA_MS};

/// Scaled-share ceiling used by the index rules. One RAY of shares is one
/// whole token at index 1.0, so 100 RAY is a realistic book size that keeps
/// every `I256` intermediate far from the `i128` bound.
const MAX_SHARES: i128 = 100 * RAY;

/// Stored-index ceiling for the seeded market. One `MAX_COMPOUND_DELTA_MS`
/// chunk at the maximum borrow rate multiplies the borrow index by at most
/// `e^2 ≈ 7.4`, so projections stay well inside the configured index caps.
const MAX_SEED_INDEX: i128 = 2 * RAY;

/// Ledger-time ceiling: keeps `last_timestamp + elapsed` inside `u64` and the
/// projection inside a single compounding chunk.
const MAX_SEED_TIMESTAMP: u64 = u64::MAX / 4;

fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    }
}

/// A rate model satisfying every constraint `MarketParamsRaw::verify` enforces
/// on a listed market, with all curve parameters left symbolic.
fn nondet_market_params(asset: &Address) -> MarketParamsRaw {
    let base_borrow_rate: i128 = cvlr::nondet::nondet();
    let slope1: i128 = cvlr::nondet::nondet();
    let slope2: i128 = cvlr::nondet::nondet();
    let slope3: i128 = cvlr::nondet::nondet();
    let mid_utilization: i128 = cvlr::nondet::nondet();
    let optimal_utilization: i128 = cvlr::nondet::nondet();
    let max_utilization: i128 = cvlr::nondet::nondet();
    let max_borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor: u32 = cvlr::nondet::nondet();
    let asset_decimals: u32 = cvlr::nondet::nondet();

    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&base_borrow_rate));
    cvlr_assume!(base_borrow_rate <= slope1);
    cvlr_assume!(slope1 <= slope2);
    cvlr_assume!(slope2 <= slope3);
    cvlr_assume!(slope3 <= MAX_BORROW_RATE_RAY);

    cvlr_assume!(mid_utilization > 0 && mid_utilization < optimal_utilization);
    cvlr_assume!(optimal_utilization < RAY);
    cvlr_assume!(max_utilization >= optimal_utilization && max_utilization <= RAY);

    cvlr_assume!(max_borrow_rate > 0 && max_borrow_rate <= MAX_BORROW_RATE_RAY);
    cvlr_assume!((0..BPS).contains(&i128::from(reserve_factor)));
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);

    MarketParamsRaw {
        max_borrow_rate,
        base_borrow_rate,
        slope1,
        slope2,
        slope3,
        mid_utilization,
        optimal_utilization,
        max_utilization,
        reserve_factor,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset.clone(),
        asset_decimals,
    }
}

/// A symbolic market as the pool would report it through `get_sync_data`,
/// last accrued at `last_timestamp`.
fn nondet_sync(asset: &Address, last_timestamp: u64) -> PoolSyncData {
    let supplied: i128 = cvlr::nondet::nondet();
    let borrowed: i128 = cvlr::nondet::nondet();
    let revenue: i128 = cvlr::nondet::nondet();
    let cash: i128 = cvlr::nondet::nondet();
    let borrow_index: i128 = cvlr::nondet::nondet();
    let supply_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=MAX_SHARES).contains(&supplied));
    cvlr_assume!((0..=supplied).contains(&borrowed));
    cvlr_assume!((0..=supplied).contains(&revenue));
    cvlr_assume!((0..=MAX_SHARES).contains(&cash));
    cvlr_assume!((RAY..=MAX_SEED_INDEX).contains(&borrow_index));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SEED_INDEX).contains(&supply_index));

    PoolSyncData {
        params: nondet_market_params(asset),
        state: PoolStateRaw {
            supplied,
            borrowed,
            revenue,
            borrow_index,
            supply_index,
            last_timestamp,
            cash,
        },
    }
}

/// The market state `update_indexes` commits at `now`: indexes replaced by the
/// projection, `last_timestamp` stamped, and protocol fee shares minted into
/// both `supplied` and `revenue`.
///
/// The minted share count is left symbolic rather than recomputed, so the rule
/// holds for *any* fee the pool could have booked — strictly stronger than
/// pinning production's exact `protocol_fee_shares` result.
fn accrued_sync(sync: &PoolSyncData, projected: &MarketIndex, now: u64) -> PoolSyncData {
    let minted: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=MAX_SHARES).contains(&minted));

    PoolSyncData {
        params: sync.params.clone(),
        state: PoolStateRaw {
            supplied: sync.state.supplied + minted,
            borrowed: sync.state.borrowed,
            revenue: sync.state.revenue + minted,
            borrow_index: projected.borrow_index.raw(),
            supply_index: projected.supply_index.raw(),
            last_timestamp: now,
            cash: sync.state.cash,
        },
    }
}

/// Draws a `(last_timestamp, now)` pair with `now - last_timestamp` inside one
/// compounding chunk, so `simulate_update_indexes` runs a single iteration.
fn nondet_accrual_window() -> (u64, u64) {
    let last_timestamp: u64 = cvlr::nondet::nondet();
    let elapsed_ms: u64 = cvlr::nondet::nondet();
    cvlr_assume!(last_timestamp <= MAX_SEED_TIMESTAMP);
    cvlr_assume!(elapsed_ms <= MAX_COMPOUND_DELTA_MS);
    (last_timestamp, last_timestamp + elapsed_ms)
}

#[rule]
fn indexes_unchanged_when_no_time_elapsed(e: Env) {
    let old_borrow_index: i128 = cvlr::nondet::nondet();
    let old_supply_index: i128 = cvlr::nondet::nondet();
    let supplied: i128 = cvlr::nondet::nondet();
    let rate: i128 = cvlr::nondet::nondet();

    cvlr_assume!((RAY..=MAX_BORROW_INDEX_RAY).contains(&old_borrow_index));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&old_supply_index));
    cvlr_assume!(supplied >= 0);
    cvlr_assume!(rate >= 0);

    let factor = common::rates::compound_interest(&e, Ray::from(rate), 0);
    cvlr_assert!(factor == Ray::ONE);

    let new_borrow = common::rates::update_borrow_index(&e, Ray::from(old_borrow_index), factor);
    cvlr_assert!(new_borrow.raw() == old_borrow_index);

    let new_supply = common::rates::update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_supply_index),
        Ray::ZERO,
    );
    cvlr_assert!(new_supply.raw() == old_supply_index);
}

#[rule]
fn index_sanity(e: Env, asset: Address) {
    let idx = crate::storage::market_index::get_market_index(&e, &asset);
    cvlr_satisfy!(idx.supply_index.raw() > 0 && idx.borrow_index.raw() > 0);
}

// ---------------------------------------------------------------------------
// V-9 family (a): view/accrue isomorphism.
// ---------------------------------------------------------------------------

/// `get_market_index` returns the same pair whether or not `update_indexes`
/// ran first.
///
/// The Blackthorn L-6 / Certora Hub L-03 shape: a view that reads unaccrued
/// state disagrees with the mutating path that accrues first, so a position
/// looks healthier (or riskier) than it is. Here both sides are the *same*
/// projection: reading before accrual projects the stored state forward to
/// `now`; reading after accrual re-projects a state already stamped at `now`,
/// which the zero-delta early return leaves untouched.
#[rule]
fn iso_market_index_invariant_across_accrual(e: Env, asset: Address) {
    let (last_timestamp, now) = nondet_accrual_window();
    let sync = nondet_sync(&asset, last_timestamp);

    // What a view returns with no prior `update_indexes`.
    let before = simulate_update_indexes(&e, now, &sync);

    // What the same view returns immediately after `update_indexes`.
    let accrued = accrued_sync(&sync, &before, now);
    let after = simulate_update_indexes(&e, now, &accrued);

    cvlr_assert!(after.borrow_index.raw() == before.borrow_index.raw());
    cvlr_assert!(after.supply_index.raw() == before.supply_index.raw());
    cvlr_assert!(after.borrow_index.raw() <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(after.supply_index.raw() <= MAX_SUPPLY_INDEX_RAY);
}

/// `get_health_factor` and `is_liquidatable` return the same values whether or
/// not `update_indexes` ran first, and `get_liquidation_estimate` reverts under
/// the same condition.
///
/// The account book is valued twice through one `Cache`, so a single frozen
/// price basis applies to both sides and the only thing that varies is the
/// market index installed by `put_market_index`: the pre-accrual projection on
/// the first pass, the post-accrual re-projection on the second. Both are
/// derived here, not assumed equal.
///
/// The `< WAD` assertion is doing double duty: it is `is_liquidatable`'s whole
/// definition (`views::can_be_liquidated`) and it is the gate
/// `build_liquidation_plan` uses to raise `HealthFactorTooHigh`, so it pins
/// `get_liquidation_estimate`'s revert condition as accrual-independent too.
#[rule]
fn iso_health_factor_invariant_across_accrual(
    e: Env,
    asset: Address,
    supply_scaled: i128,
    debt_scaled: i128,
    liquidation_threshold: u32,
    loan_to_value: u32,
) {
    cvlr_assume!((0..=MAX_SHARES).contains(&supply_scaled));
    cvlr_assume!((0..=MAX_SHARES).contains(&debt_scaled));
    cvlr_assume!(i128::from(liquidation_threshold) <= BPS);
    cvlr_assume!(i128::from(loan_to_value) <= i128::from(liquidation_threshold));

    let (last_timestamp, now) = nondet_accrual_window();
    let sync = nondet_sync(&asset, last_timestamp);
    let before = simulate_update_indexes(&e, now, &sync);
    let accrued = accrued_sync(&sync, &before, now);
    let after = simulate_update_indexes(&e, now, &accrued);

    let hub_asset = hub0(&asset);
    let mut supply_positions: Map<HubAssetKey, AccountPositionRaw> = Map::new(&e);
    supply_positions.set(
        hub_asset.clone(),
        AccountPositionRaw {
            scaled_amount: supply_scaled,
            liquidation_threshold,
            liquidation_bonus: 500,
            loan_to_value,
            liquidation_fees: 100,
        },
    );
    let mut debt_positions: Map<HubAssetKey, DebtPositionRaw> = Map::new(&e);
    debt_positions.set(
        hub_asset.clone(),
        DebtPositionRaw {
            scaled_amount: debt_scaled,
        },
    );

    crate::spec::fixture::seed_market(&e, &asset);
    let mut cache = Cache::new_view(&e);

    cache.put_market_index(&hub_asset, &MarketIndexRaw::from(&before));
    let pre = crate::risk::totals::calculate_account_risk_totals::calculate_account_risk_totals(
        &e,
        &mut cache,
        &supply_positions,
        &debt_positions,
    );

    cache.put_market_index(&hub_asset, &MarketIndexRaw::from(&after));
    let post = crate::risk::totals::calculate_account_risk_totals::calculate_account_risk_totals(
        &e,
        &mut cache,
        &supply_positions,
        &debt_positions,
    );

    // get_health_factor
    cvlr_assert!(pre.health_factor.raw() == post.health_factor.raw());
    // is_liquidatable, and get_liquidation_estimate's HealthFactorTooHigh gate
    cvlr_assert!((pre.health_factor.raw() < WAD) == (post.health_factor.raw() < WAD));
    // the legs get_liquidation_estimate sizes its plan from
    cvlr_assert!(pre.total_debt.raw() == post.total_debt.raw());
    cvlr_assert!(pre.weighted_collateral.raw() == post.weighted_collateral.raw());
    cvlr_assert!(pre.total_collateral.raw() == post.total_collateral.raw());
}

/// `update_indexes` writes no controller state, so nothing a view reads on the
/// controller side — positions, account metadata, spoke risk parameters — can
/// differ between a view called before it and the same view called after.
///
/// This is the other half of isomorphism: the projected index is invariant
/// (above), and the controller-local inputs are invariant here. Together they
/// cover the *revert* conditions too, since `get_health_factor`,
/// `is_liquidatable`, `get_liquidation_estimate` and `get_market_index` all
/// gate on account existence and spoke listing, both read from this state.
#[rule]
fn iso_update_indexes_writes_no_controller_state(
    e: Env,
    caller: Address,
    owner: Address,
    asset: Address,
    supply_scaled: i128,
    debt_scaled: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!((0..=MAX_SHARES).contains(&supply_scaled));
    cvlr_assume!((0..=MAX_SHARES).contains(&debt_scaled));

    crate::spec::fixture::seed_live_account(&e, account_id, &owner, &asset);
    fixture::seed_empty_books(&e, account_id);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, debt_scaled);

    let hub_asset = hub0(&asset);
    let spoke_id = crate::spec::fixture::SPOKE_ID;

    let pre_supply = crate::storage::get_supply_positions(&e, account_id);
    let pre_debt = crate::storage::get_debt_positions(&e, account_id);
    let pre_meta = crate::storage::get_account_meta(&e, account_id);
    let pre_config = crate::storage::get_spoke_asset(&e, spoke_id, &hub_asset).unwrap();

    crate::Controller::update_indexes(e.clone(), caller, vec![&e, hub_asset.clone()]);

    let post_supply = crate::storage::get_supply_positions(&e, account_id);
    let post_debt = crate::storage::get_debt_positions(&e, account_id);
    let post_meta = crate::storage::get_account_meta(&e, account_id);
    let post_config = crate::storage::get_spoke_asset(&e, spoke_id, &hub_asset).unwrap();

    cvlr_assert!(post_supply == pre_supply);
    cvlr_assert!(post_debt == pre_debt);
    cvlr_assert!(post_meta == pre_meta);
    cvlr_assert!(post_config.liquidation_threshold == pre_config.liquidation_threshold);
    cvlr_assert!(post_config.loan_to_value == pre_config.loan_to_value);
    cvlr_assert!(post_config.liquidation_bonus == pre_config.liquidation_bonus);
    cvlr_assert!(post_config.liquidation_fees == pre_config.liquidation_fees);
    cvlr_assert!(post_config.paused == pre_config.paused);
    cvlr_assert!(post_config.frozen == pre_config.frozen);
}

// ---------------------------------------------------------------------------
// V-9 family (b): time monotonicity with no accrual invoked.
// ---------------------------------------------------------------------------

/// `get_market_index` is monotone in ledger time when nothing accrues between
/// the two reads: a keeper who waits never sees a smaller index.
///
/// Both projections start from the *same* stored state, which is exactly the
/// "no accrual is invoked" precondition — `update_indexes` is what would move
/// `last_timestamp` forward.
#[rule]
fn time_mono_market_index_non_decreasing(e: Env, asset: Address) {
    let last_timestamp: u64 = cvlr::nondet::nondet();
    let early_delta: u64 = cvlr::nondet::nondet();
    let late_delta: u64 = cvlr::nondet::nondet();
    cvlr_assume!(last_timestamp <= MAX_SEED_TIMESTAMP);
    cvlr_assume!(early_delta <= late_delta);
    cvlr_assume!(late_delta <= MAX_COMPOUND_DELTA_MS);

    let sync = nondet_sync(&asset, last_timestamp);

    let early = simulate_update_indexes(&e, last_timestamp + early_delta, &sync);
    let late = simulate_update_indexes(&e, last_timestamp + late_delta, &sync);

    cvlr_assert!(late.borrow_index.raw() >= early.borrow_index.raw());
    cvlr_assert!(late.supply_index.raw() >= early.supply_index.raw());
}

/// The valuation legs `get_liquidation_estimate` sizes its plan from are
/// monotone in ledger time when nothing accrues between the two reads.
///
/// `build_liquidation_plan` values debt with `position_value_ceil` against the
/// borrow index and threshold-weighted collateral with `position_value_floor`
/// against the supply index (`calculate_account_risk_totals_body`). Both are
/// pinned here against the projections at two ledger times, so a keeper cannot
/// make a debt leg shrink — or a collateral leg shrink — by choosing when to
/// call the estimate.
#[rule]
fn time_mono_liquidation_estimate_valuation_non_decreasing(
    e: Env,
    asset: Address,
    supply_scaled: i128,
    debt_scaled: i128,
    price_wad: i128,
) {
    cvlr_assume!((0..=MAX_SHARES).contains(&supply_scaled));
    cvlr_assume!((0..=MAX_SHARES).contains(&debt_scaled));
    cvlr_assume!(price_wad > 0 && price_wad <= 1_000_000 * WAD);

    let last_timestamp: u64 = cvlr::nondet::nondet();
    let early_delta: u64 = cvlr::nondet::nondet();
    let late_delta: u64 = cvlr::nondet::nondet();
    cvlr_assume!(last_timestamp <= MAX_SEED_TIMESTAMP);
    cvlr_assume!(early_delta <= late_delta);
    cvlr_assume!(late_delta <= MAX_COMPOUND_DELTA_MS);

    let sync = nondet_sync(&asset, last_timestamp);
    let early = simulate_update_indexes(&e, last_timestamp + early_delta, &sync);
    let late = simulate_update_indexes(&e, last_timestamp + late_delta, &sync);

    let price = common::math::fp::Wad::from(price_wad);

    let debt_early =
        crate::risk::position_value_ceil(&e, Ray::from(debt_scaled), early.borrow_index, price);
    let debt_late =
        crate::risk::position_value_ceil(&e, Ray::from(debt_scaled), late.borrow_index, price);
    let coll_early =
        crate::risk::position_value_floor(&e, Ray::from(supply_scaled), early.supply_index, price);
    let coll_late =
        crate::risk::position_value_floor(&e, Ray::from(supply_scaled), late.supply_index, price);

    cvlr_assert!(debt_late.raw() >= debt_early.raw());
    cvlr_assert!(coll_late.raw() >= coll_early.raw());
}
