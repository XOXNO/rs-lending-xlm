//! Pool-side V-9: accrue-then-read versus read, over the pool's public view
//! surface (Certora Aave Hub P-09/P-10 analogue).
//!
//! The controller half (`certora/controller/spec/index_rules.rs`) proves that
//! the *projected* index a view returns is the same before and after
//! `update_indexes`. That argument rests on the pool actually reaching a fixed
//! point when it accrues: `last_timestamp` stamped at `now`, and a second
//! accrual at the same ledger time moving nothing. These rules discharge that
//! obligation directly against `ops::market::accrue`.
//!
//! ## Scope note
//!
//! `common::rates::simulate_update_indexes` is replaced by a monotone havoc
//! summary in the pool's certora build (`pool`'s `certora` feature pulls
//! `common/certora`, which activates the `apply_summary!` in
//! `common/src/rates/simulate.rs`). `get_bulk_indexes` is therefore not
//! provable here and is covered on the controller side instead, where `common`
//! is compiled without its `certora` feature. Everything below reads *stored*
//! state through `storage::load_sync_data` and `views::*`, all of which are
//! real code in this build.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::rates::MAX_COMPOUND_DELTA_MS;
use common::types::{HubAssetKey, PoolStateRaw};

use super::fixture::{hub, params, seed, ONE_TOKEN};

const MAX_SHARES: i128 = 100 * RAY;
const MAX_CASH: i128 = 1_000 * ONE_TOKEN;

/// Everything the pool's amount-shaped view entrypoints report, plus the raw
/// `get_sync_data` blob they are all derived from.
struct ViewSurface {
    // get_sync_data().state
    state_supplied: i128,
    state_borrowed: i128,
    state_revenue: i128,
    state_borrow_index: i128,
    state_supply_index: i128,
    state_last_timestamp: u64,
    state_cash: i128,
    // get_sync_data().params
    params_asset_decimals: u32,
    params_reserve_factor: u32,
    params_max_borrow_rate: i128,
    params_flashloan_fee: u32,
    params_is_flashloanable: bool,
    // derived views
    reserves: i128,
    supplied_amount: i128,
    borrowed_amount: i128,
    revenue: i128,
    delta_time: u64,
}

/// The three curve-derived views. Split out from [`ViewSurface`] because each
/// one re-runs the piecewise rate model; rules that already pin the whole
/// stored state get them for free (they are pure functions of that state) and
/// need not pay for the extra branches.
struct RateSurface {
    utilization: i128,
    deposit_rate: i128,
    borrow_rate: i128,
}

fn read_views(e: &Env, key: &HubAssetKey) -> ViewSurface {
    let sync = crate::storage::load_sync_data(e, key);
    ViewSurface {
        state_supplied: sync.state.supplied,
        state_borrowed: sync.state.borrowed,
        state_revenue: sync.state.revenue,
        state_borrow_index: sync.state.borrow_index,
        state_supply_index: sync.state.supply_index,
        state_last_timestamp: sync.state.last_timestamp,
        state_cash: sync.state.cash,
        params_asset_decimals: sync.params.asset_decimals,
        params_reserve_factor: sync.params.reserve_factor,
        params_max_borrow_rate: sync.params.max_borrow_rate,
        params_flashloan_fee: sync.params.flashloan_fee,
        params_is_flashloanable: sync.params.is_flashloanable,
        reserves: crate::views::reserves(e, key),
        supplied_amount: crate::views::supplied_amount(e, key),
        borrowed_amount: crate::views::borrowed_amount(e, key),
        revenue: crate::views::protocol_revenue(e, key),
        delta_time: crate::views::delta_time(e, key),
    }
}

fn read_rate_views(e: &Env, key: &HubAssetKey) -> RateSurface {
    RateSurface {
        utilization: crate::views::utilization(e, key),
        deposit_rate: crate::views::deposit_rate(e, key),
        borrow_rate: crate::views::borrow_rate(e, key),
    }
}

fn assert_same_views(pre: &ViewSurface, post: &ViewSurface) {
    cvlr_assert!(post.state_supplied == pre.state_supplied);
    cvlr_assert!(post.state_borrowed == pre.state_borrowed);
    cvlr_assert!(post.state_revenue == pre.state_revenue);
    cvlr_assert!(post.state_borrow_index == pre.state_borrow_index);
    cvlr_assert!(post.state_supply_index == pre.state_supply_index);
    cvlr_assert!(post.state_last_timestamp == pre.state_last_timestamp);
    cvlr_assert!(post.state_cash == pre.state_cash);
    cvlr_assert!(post.reserves == pre.reserves);
    cvlr_assert!(post.supplied_amount == pre.supplied_amount);
    cvlr_assert!(post.borrowed_amount == pre.borrowed_amount);
    cvlr_assert!(post.revenue == pre.revenue);
    cvlr_assert!(post.delta_time == pre.delta_time);
}

fn assert_same_params(pre: &ViewSurface, post: &ViewSurface) {
    cvlr_assert!(post.params_asset_decimals == pre.params_asset_decimals);
    cvlr_assert!(post.params_reserve_factor == pre.params_reserve_factor);
    cvlr_assert!(post.params_max_borrow_rate == pre.params_max_borrow_rate);
    cvlr_assert!(post.params_flashloan_fee == pre.params_flashloan_fee);
    cvlr_assert!(post.params_is_flashloanable == pre.params_is_flashloanable);
}

/// Seeds one market whose last accrual sits `elapsed_ms` before the current
/// ledger time, and returns its key. `elapsed_ms` is capped at one compounding
/// chunk so `interest::global_sync` runs a single `accrue_chunk`.
#[allow(clippy::too_many_arguments)]
fn seed_market_behind_by(
    e: &Env,
    admin: Address,
    asset: &Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    cash: i128,
    elapsed_ms: u64,
) -> HubAssetKey {
    cvlr_assume!(supplied >= 0 && supplied <= MAX_SHARES);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(revenue >= 0 && revenue <= supplied);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(cash >= 0 && cash <= MAX_CASH);

    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let now = crate::time::now_ms(e);
    cvlr_assume!(elapsed_ms <= MAX_COMPOUND_DELTA_MS);
    cvlr_assume!(elapsed_ms <= now);

    seed(
        e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        PoolStateRaw {
            supplied,
            borrowed,
            revenue,
            borrow_index,
            supply_index,
            last_timestamp: now - elapsed_ms,
            cash,
        },
    );

    hub(asset.clone())
}

/// With no time elapsed, every pool view returns the same value whether or not
/// `update_indexes` ran first.
///
/// This is the view-surface lift of `accrue_is_noop_when_no_time_elapsed`
/// (`lifecycle_rules.rs`), which pins the same property one level down at the
/// storage record. Stating it over the entrypoints is what makes it a V-9
/// isomorphism claim: an integrator polling `get_supplied_amount` /
/// `get_borrowed_amount` / `get_revenue` cannot be handed a different answer by
/// racing a keeper's `update_indexes` within one ledger.
#[rule]
#[allow(clippy::too_many_arguments)]
fn iso_pool_views_unchanged_by_zero_time_accrue(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    cash: i128,
) {
    let key = seed_market_behind_by(
        &e,
        admin,
        &asset,
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        cash,
        0,
    );

    let pre = read_views(&e, &key);
    let pre_rates = read_rate_views(&e, &key);

    crate::ops::market::accrue(&e, key.clone());

    let post = read_views(&e, &key);
    let post_rates = read_rate_views(&e, &key);

    assert_same_views(&pre, &post);
    assert_same_params(&pre, &post);
    cvlr_assert!(post_rates.utilization == pre_rates.utilization);
    cvlr_assert!(post_rates.deposit_rate == pre_rates.deposit_rate);
    cvlr_assert!(post_rates.borrow_rate == pre_rates.borrow_rate);
    cvlr_assert!(post.delta_time == 0);
}

/// Accrual is a fixed point of the pool's view surface: once `update_indexes`
/// has run at a ledger time, running it again at the same time moves nothing
/// any view reports.
///
/// This is the obligation the controller-side isomorphism argument depends on.
/// `simulate_update_indexes` returns the stored indexes unchanged exactly when
/// `last_timestamp == now`, so an accrual that failed to stamp the timestamp —
/// or that left residual work for a second pass — would break the equality
/// between "view before `update_indexes`" and "view after". Unlike the
/// zero-time rule above, the first `accrue` here does real work.
///
/// The three rate views are omitted deliberately: `views::utilization`,
/// `views::deposit_rate` and `views::borrow_rate` are pure functions of the
/// stored state loaded by `Cache::load`, which this rule pins field by field.
#[rule]
#[allow(clippy::too_many_arguments)]
fn iso_pool_views_fixed_point_after_accrue(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    cash: i128,
    elapsed_ms: u64,
) {
    let key = seed_market_behind_by(
        &e,
        admin,
        &asset,
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        cash,
        elapsed_ms,
    );

    crate::ops::market::accrue(&e, key.clone());
    let pre = read_views(&e, &key);

    crate::ops::market::accrue(&e, key.clone());
    let post = read_views(&e, &key);

    assert_same_views(&pre, &post);
    assert_same_params(&pre, &post);
    // The stamp that makes the projection an identity on the next read.
    cvlr_assert!(pre.delta_time == 0);
    cvlr_assert!(pre.state_last_timestamp == crate::time::now_ms(&e));
}

/// Cash reserves and the rate model survive accrual untouched, however much
/// time elapsed.
///
/// `get_reserves` is the one amount-shaped pool view that is accrual-invariant
/// unconditionally, and `get_sync_data().params` is the part of the sync blob
/// the controller reads for `asset_decimals` (`views::collateral_amount_for_hub_asset`,
/// `views::borrow_amount_for_hub_asset`). Both are therefore isomorphic to
/// accrual without the zero-elapsed precondition the rule above needs.
#[rule]
#[allow(clippy::too_many_arguments)]
fn iso_accrue_preserves_cash_and_params(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    cash: i128,
    elapsed_ms: u64,
) {
    let key = seed_market_behind_by(
        &e,
        admin,
        &asset,
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        cash,
        elapsed_ms,
    );

    let pre = read_views(&e, &key);
    crate::ops::market::accrue(&e, key.clone());
    let post = read_views(&e, &key);

    cvlr_assert!(post.state_cash == pre.state_cash);
    cvlr_assert!(post.reserves == pre.reserves);
    cvlr_assert!(post.state_borrowed == pre.state_borrowed);
    assert_same_params(&pre, &post);
}

/// The pool's stored-index views are **not** accrual-invariant once time has
/// elapsed — they lag, and this rule pins the direction and the size of the
/// lag rather than asserting an equality that does not hold.
///
/// `views::supplied_amount`, `views::borrowed_amount` and
/// `views::protocol_revenue` all read the *stored* index (`Cache::load` does
/// not accrue, by its own doc comment), so a reader who does not accrue first
/// sees a value that is at most the accrued one, short by exactly the interest
/// over `get_delta_time()` milliseconds. Consumers must use `get_bulk_indexes`
/// — which projects — for anything risk-bearing; the controller does.
///
/// The direction is the security-relevant half: the stale reading never
/// *overstates* debt, so an integrator sizing a liquidation off
/// `get_borrowed_amount` under-collects rather than over-collects.
#[rule]
#[allow(clippy::too_many_arguments)]
fn iso_unaccrued_views_lag_accrued_values(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    cash: i128,
    elapsed_ms: u64,
) {
    let key = seed_market_behind_by(
        &e,
        admin,
        &asset,
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        cash,
        elapsed_ms,
    );

    let pre = read_views(&e, &key);
    crate::ops::market::accrue(&e, key.clone());
    let post = read_views(&e, &key);

    cvlr_assert!(pre.delta_time == elapsed_ms);
    cvlr_assert!(post.delta_time == 0);
    cvlr_assert!(post.state_borrow_index >= pre.state_borrow_index);
    cvlr_assert!(post.state_supply_index >= pre.state_supply_index);
    cvlr_assert!(post.borrowed_amount >= pre.borrowed_amount);
    cvlr_assert!(post.supplied_amount >= pre.supplied_amount);
    cvlr_assert!(post.revenue >= pre.revenue);
}
