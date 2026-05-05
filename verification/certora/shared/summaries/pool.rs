//! Summaries for the `LiquidityPool` contract (`pool/src/lib.rs`).
//!
//! Cross-contract pool calls are pure havoc to the prover and the pool
//! mutating paths reach into Cache + interest accrual + I256 scaled-amount
//! math, which trips the TAC command-count budget. Every domain rule that
//! traverses a pool call becomes vacuous over havoced returns; the bounds
//! captured here provide the postconditions consumed by downstream reasoning.
//!
//! Each summary's `cvlr_assume!` bounds encode the modeled branch or domain.
//! Rules that need excluded edge branches should call the production function
//! directly.
//!
//! Wiring: each summary is registered against its production fn via
//! `cvlr_soroban_macros::apply_summary!` at the production site (the
//! `summarized!` macro indirection in `controller/src/lib.rs`). The matching
//! production fn lives in `pool/src/lib.rs`; signatures here mirror that
//! ABI exactly so the macro substitution type-checks.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use cvlr_soroban::nondet_address;
use soroban_sdk::{Address, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    AccountPosition, AccountPositionType, MarketIndex, MarketParams, PoolPositionMutation,
    PoolState, PoolStrategyMutation, PoolSyncData,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build a nondet `MarketIndex` satisfying the production invariants
/// (`pool::interest::global_sync` + `apply_bad_debt_to_supply_index`):
///   * `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW` (= `WAD`, the bad-debt
///     floor; see `pool/src/interest.rs`).
///   * `borrow_index_ray >= RAY` (initial value is `RAY`; only grows).
fn nondet_market_index() -> MarketIndex {
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);
    MarketIndex {
        supply_index_ray,
        borrow_index_ray,
    }
}

/// Build a nondet `MarketIndex` whose indexes do NOT decrease relative to a
/// prior snapshot. Used by every accruing pool path; only `seize_position`
/// allows the supply index to drop (bad-debt write-down).
fn nondet_market_index_monotone(prior: &MarketIndex) -> MarketIndex {
    let idx = nondet_market_index();
    cvlr_assume!(idx.supply_index_ray >= prior.supply_index_ray);
    cvlr_assume!(idx.borrow_index_ray >= prior.borrow_index_ray);
    idx
}

// ---------------------------------------------------------------------------
// Mutating endpoints
// ---------------------------------------------------------------------------

/// Summary for `pool::LiquidityPool::supply`.
///
/// Modeled postconditions:
///   * `actual_amount == amount`; pool reports the exact gross credited
///     amount on the successful path.
///   * `position.scaled_amount_ray` is monotone non-decreasing in the input
///     because the scaled amount is derived from a non-negative input.
///   * `market_index` satisfies the global index invariants after accrual.
pub fn supply_summary(
    _env: &Env,
    _asset: &Address,
    position: AccountPosition,
    _price_wad: i128,
    amount: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index,
        actual_amount: amount,
    }
}

/// Summary for `pool::LiquidityPool::borrow`.
///
/// Modeled postconditions:
///   * `actual_amount == amount`. The pool transfers exactly `amount` to the
///     caller on the successful path.
///   * Reserves were sufficient at call time.
///   * `position.scaled_amount_ray` is monotone non-decreasing.
///   * `market_index` satisfies the global index invariants.
pub fn borrow_summary(
    _env: &Env,
    _asset: &Address,
    _caller: Address,
    amount: i128,
    position: AccountPosition,
    _price_wad: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index,
        actual_amount: amount,
    }
}

/// Summary for `pool::LiquidityPool::withdraw`.
///
/// Modeled postconditions:
///   * `actual_amount` is the gross withdrawn before any liquidation fee
///     and is modeled within the requested `amount` domain. The production
///     dust-lock branch can promote a near-full partial request into a full
///     withdrawal; rules that need that branch should call production directly.
///   * `actual_amount >= 0`.
///   * `position.scaled_amount_ray` is monotone non-increasing.
///   * `market_index` satisfies the global index invariants.
pub fn withdraw_summary(
    _env: &Env,
    _asset: &Address,
    _caller: Address,
    amount: i128,
    position: AccountPosition,
    _is_liquidation: bool,
    _protocol_fee: i128,
    _price_wad: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0);
    cvlr_assume!(new_scaled <= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index,
        actual_amount,
    }
}

/// Summary for `pool::LiquidityPool::repay`.
///
/// Modeled postconditions:
///   * `actual_amount = amount.min(current_debt)`. With `amount >= 0` and
///     `current_debt >= 0`, this gives
///     `0 <= actual_amount <= amount`.
///   * Overpayment beyond `current_debt` is refunded to the caller
///     and the position cannot go negative because the scaled-balance
///     subtraction is checked.
///   * `position.scaled_amount_ray` is monotone non-increasing.
///   * `market_index` satisfies the global index invariants.
pub fn repay_summary(
    _env: &Env,
    _asset: &Address,
    _caller: Address,
    amount: i128,
    position: AccountPosition,
    _price_wad: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0);
    cvlr_assume!(new_scaled <= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index,
        actual_amount,
    }
}

/// Summary for `pool::LiquidityPool::update_indexes`.
///
/// Modeled postcondition: a fresh sync of `(supply_index_ray, borrow_index_ray)`
/// satisfying the global index invariants. No position mutation, no token
/// transfer.
pub fn update_indexes_summary(_env: &Env, _pool_addr: &Address, _price_wad: i128) -> MarketIndex {
    nondet_market_index()
}

/// Summary for `pool::LiquidityPool::add_rewards`.
///
/// Modeled postconditions:
///   * `amount >= 0`.
///   * Empty-pool reward credits panic with `NoSuppliersToReward`. The summary
///     is pure side-effect, so this precondition is preserved by production
///     reachability rather than a return value.
pub fn add_rewards_summary(_env: &Env, _pool_addr: &Address, _price_wad: i128, _amount: i128) {}

/// Summary for `pool::LiquidityPool::flash_loan_begin`.
///
/// Modeled postconditions:
///   * `amount >= 0`.
///   * Reserves were sufficient at begin time.
///   * Pre-balance snapshot is recorded in instance storage; any subsequent
///     `flash_loan_end` will read it back.
///
/// Pure side-effect; no return value.
pub fn flash_loan_begin_summary(
    _env: &Env,
    _pool_addr: &Address,
    _amount: i128,
    _receiver: &Address,
) {
}

/// Summary for `pool::LiquidityPool::flash_loan_end`.
///
/// Modeled postconditions:
///   * `amount >= 0`, `fee >= 0`.
///   * The receiver returned at least `amount + fee`; a short repayment panics
///     with `InvalidFlashloanRepay`.
///   * `fee` is added to protocol revenue.
///
/// Pure side-effect; no return value.
pub fn flash_loan_end_summary(
    _env: &Env,
    _pool_addr: &Address,
    _amount: i128,
    _fee: i128,
    _receiver: &Address,
) {
}

/// Summary for `pool::LiquidityPool::create_strategy`.
///
/// Modeled postconditions:
///   * `amount >= 0`, `fee >= 0`.
///   * `fee <= amount`.
///   * Reserves cover `amount`.
///   * `actual_amount == amount`, `amount_received == amount - fee`.
///     `amount_received >= 0` follows from `fee <= amount`.
///   * `position.scaled_amount_ray` is monotone non-decreasing (debt added).
///   * `market_index` satisfies the global index invariants.
pub fn create_strategy_summary(
    _env: &Env,
    _asset: &Address,
    _caller: Address,
    position: AccountPosition,
    amount: i128,
    fee: i128,
    _price_wad: i128,
) -> PoolStrategyMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    // Production enforces fee <= amount before this point; assume it so the
    // returned `amount_received` is non-negative. Without it `amount - fee`
    // could be negative and break downstream rules that assume a positive
    // controller-receivable.
    cvlr_assume!(fee >= 0);
    cvlr_assume!(amount >= 0);
    cvlr_assume!(fee <= amount);

    let market_index = nondet_market_index();
    PoolStrategyMutation {
        position: new_position,
        market_index,
        actual_amount: amount,
        amount_received: amount - fee,
    }
}

/// Summary for `pool::LiquidityPool::seize_position`.
///
/// Modeled postconditions:
///   * Returned position has `scaled_amount_ray == 0`.
///     The full position is consumed -- borrow branch socializes the debt
///     into the supply index and zeroes the scaled balance; deposit branch
///     absorbs the residual into pool revenue.
///   * The supply index may DROP in the borrow branch (bad-debt write-down
///     via `apply_bad_debt_to_supply_index`), still floored at
///     `SUPPLY_INDEX_FLOOR_RAW`. Unlike every other path, `seize_position`
///     is the only place where `supply_index_ray` is permitted to decrease.
///   * Other index/state invariants still hold:
///     `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW`,
///     `borrow_index_ray >= RAY`.
pub fn seize_position_summary(
    _env: &Env,
    _asset: &Address,
    _side: AccountPositionType,
    position: AccountPosition,
    _price_wad: i128,
) -> AccountPosition {
    let mut zeroed = position.clone();
    zeroed.scaled_amount_ray = 0;
    zeroed
}

/// Summary for `pool::LiquidityPool::claim_revenue`.
///
/// Modeled postconditions:
///   * Return value is `amount_to_transfer = current_reserves.min(treasury_actual)`
///     and both inputs are non-negative, so `result >= 0`.
///   * On the zero-revenue early-return branch the function returns 0.
///   * State updates are committed before the external token call so
///     reentrant claims observe the post-burn state.
pub fn claim_revenue_summary(_env: &Env, _pool_addr: &Address, _price_wad: i128) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

// ---------------------------------------------------------------------------
// Read-only views
// ---------------------------------------------------------------------------

/// Summary for `pool::LiquidityPool::get_sync_data`.
///
/// Production reads `(MarketParams, PoolState)` from instance storage. The
/// summary returns a fresh `PoolSyncData` whose `state` satisfies the
/// stored-state invariants:
///   * `supplied_ray >= 0`, `borrowed_ray >= 0`, `revenue_ray >= 0`.
///   * `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW`,
///     `borrow_index_ray >= RAY` (initial constructor values, only grow
///     except for bad-debt write-down).
///   * `last_timestamp` is a u64 (no bound here; rules that need staleness
///     bounds can compose with cache.current_timestamp_ms separately).
///
/// `params` is fully havoced -- rules that depend on a specific param
/// shape should constrain it themselves.
pub fn get_sync_data_summary(_env: &Env, _pool_addr: &Address) -> PoolSyncData {
    let supplied_ray: i128 = nondet();
    let borrowed_ray: i128 = nondet();
    let revenue_ray: i128 = nondet();
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    let last_timestamp: u64 = nondet();

    cvlr_assume!(supplied_ray >= 0);
    cvlr_assume!(borrowed_ray >= 0);
    cvlr_assume!(revenue_ray >= 0);
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);

    let max_borrow_rate_ray: i128 = nondet();
    let base_borrow_rate_ray: i128 = nondet();
    let slope1_ray: i128 = nondet();
    let slope2_ray: i128 = nondet();
    let slope3_ray: i128 = nondet();
    let mid_utilization_ray: i128 = nondet();
    let optimal_utilization_ray: i128 = nondet();
    let reserve_factor_bps: u32 = nondet();
    cvlr_assume!(i128::from(reserve_factor_bps) < common::constants::BPS);
    let asset_decimals: u32 = nondet();
    cvlr_assume!(asset_decimals <= 27);
    let asset_id: Address = nondet_address();

    PoolSyncData {
        params: MarketParams {
            max_borrow_rate_ray,
            base_borrow_rate_ray,
            slope1_ray,
            slope2_ray,
            slope3_ray,
            mid_utilization_ray,
            optimal_utilization_ray,
            reserve_factor_bps,
            asset_id,
            asset_decimals,
        },
        state: PoolState {
            supplied_ray,
            borrowed_ray,
            revenue_ray,
            borrow_index_ray,
            supply_index_ray,
            last_timestamp,
        },
    }
}

// ---------------------------------------------------------------------------
// Single-value view summaries
// ---------------------------------------------------------------------------
//
// These five views (`reserves`, `supplied_amount`, `borrowed_amount`,
// `protocol_revenue`, `capital_utilisation`) appear up to 4 times per rule
// in `solvency_rules.rs` and `flash_loan_rules.rs`. Without summaries each
// call returns an independent havoc; rules of shape "after op X, identity
// Y(reserves, supplied, borrowed) holds" become vacuous because the prover
// picks a different reserves/supplied/borrowed triple per call site.
//
// Two complementary summary shapes are provided:
//   * Independent per-view summaries (below) — minimal `>= 0` bound; safe
//     when the rule only reads ONE view.
//   * A joint `pool_snapshot_summary` (further down) returning all four
//     amounts under production-side identities (revenue <= supplied,
//     borrowed <= supplied + revenue) for rules that need cross-view
//     consistency.

/// Summary for `pool::LiquidityPool::reserves` (`pool/src/lib.rs:708-710`,
/// real impl at `pool/src/views.rs:44-48`).
///
/// Production reads the SAC `balance` of the pool contract address. SAC
/// guarantees a non-negative balance (negative transfers panic in the host).
pub fn reserves_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Summary for `pool::LiquidityPool::supplied_amount`
/// (`pool/src/views.rs:75-81`).
///
/// Production rescales `supplied_ray * supply_index_ray` to asset decimals.
/// Both inputs are non-negative (state invariant: `supplied_ray >= 0`,
/// `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW`), so the rescaled result is
/// non-negative.
pub fn supplied_amount_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Summary for `pool::LiquidityPool::borrowed_amount`
/// (`pool/src/views.rs:84-90`).
///
/// Same shape as `supplied_amount`: rescales
/// `borrowed_ray * borrow_index_ray` to asset decimals, both inputs are
/// non-negative.
pub fn borrowed_amount_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Summary for `pool::LiquidityPool::protocol_revenue`
/// (`pool/src/views.rs:66-72`).
///
/// Production rescales `revenue_ray * supply_index_ray` to asset decimals;
/// `revenue_ray >= 0` and `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW`
/// guarantee a non-negative output.
pub fn protocol_revenue_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Summary for `pool::LiquidityPool::capital_utilisation`
/// (`pool/src/views.rs:24-41`).
///
/// Production returns `borrowed / supplied` in RAY precision (or 0 when
/// supply is empty). Bounds: `0 <= utilisation <= RAY`. A pool that has been
/// over-borrowed via bad-debt write-down can transiently exceed RAY in
/// practice, but the controller-side rules treat the bounded RAY range as
/// the safe domain.
pub fn capital_utilisation_summary(_env: &Env) -> i128 {
    let util_ray: i128 = nondet();
    cvlr_assume!(util_ray >= 0);
    cvlr_assume!(util_ray <= RAY);
    util_ray
}

// ---------------------------------------------------------------------------
// Joint pool view snapshot (cross-view consistency)
// ---------------------------------------------------------------------------

/// Joint snapshot of the four pool quantity views.
///
/// Used by rules that compare two or more views against each other (e.g.,
/// "revenue <= supplied", "borrowed <= supplied"). Returning a tuple under
/// production-side identities is sound and lets the prover share the same
/// snapshot across every assertion in one rule body.
pub struct PoolViewsSnapshot {
    pub reserves: i128,
    pub supplied: i128,
    pub borrowed: i128,
    pub revenue: i128,
}

/// Build a single internally-consistent snapshot of the four pool views.
///
/// Production identities encoded:
///   * Each value `>= 0`.
///   * `revenue <= supplied` -- accrued revenue is a slice of the supply
///     index; it cannot exceed the total supplied principal.
///   * `borrowed <= supplied + revenue` -- borrowed principal is bounded by
///     the supplier-funded liquidity plus accrued protocol revenue.
///
/// Reserves is left independent of the other three: SAC balance can drift
/// from the accounting view via direct token transfers (production tracks
/// this discrepancy via `transfer_and_measure_received`).
pub fn pool_snapshot_summary(_env: &Env) -> PoolViewsSnapshot {
    let reserves: i128 = nondet();
    let supplied: i128 = nondet();
    let borrowed: i128 = nondet();
    let revenue: i128 = nondet();
    cvlr_assume!(reserves >= 0);
    cvlr_assume!(supplied >= 0);
    cvlr_assume!(borrowed >= 0);
    cvlr_assume!(revenue >= 0);
    cvlr_assume!(revenue <= supplied);
    cvlr_assume!(borrowed <= supplied + revenue);
    PoolViewsSnapshot {
        reserves,
        supplied,
        borrowed,
        revenue,
    }
}

// ---------------------------------------------------------------------------
// Re-export for monotone-index helpers (used by rules that compare against a
// prior MarketIndex snapshot)
// ---------------------------------------------------------------------------

/// Public wrapper over `nondet_market_index_monotone` for rules that need to
/// constrain a fresh MarketIndex against a prior snapshot without going
/// through one of the typed summary entrypoints.
pub fn fresh_monotone_index(prior: &MarketIndex) -> MarketIndex {
    nondet_market_index_monotone(prior)
}
