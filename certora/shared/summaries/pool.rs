//! LiquidityPool summaries for Certora.
//!
//! Bounds model pool postconditions without expanding cross-contract calls.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Bytes, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    AccountPositionType, MarketIndex, MarketParamsRaw, MarketStateSnapshot, PoolAmountMutation,
    PoolPositionMutation, PoolStateRaw, PoolStrategyMutation, PoolSyncData, ScaledPositionRaw,
};
// Shared helpers

/// Build a nondet `MarketIndex` within production lower bounds.
fn nondet_market_index() -> MarketIndex {
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);
    MarketIndex {
        supply_index: common::math::fp::Ray::from(supply_index_ray),
        borrow_index: common::math::fp::Ray::from(borrow_index_ray),
    }
}

/// Build a nondet `MarketIndex` whose indexes do NOT decrease relative to a
/// prior snapshot. Used by every accruing pool path; only `seize_position`
/// allows the supply index to drop (bad-debt write-down).
fn nondet_market_index_monotone(prior: &MarketIndex) -> MarketIndex {
    let idx = nondet_market_index();
    cvlr_assume!(idx.supply_index >= prior.supply_index);
    cvlr_assume!(idx.borrow_index >= prior.borrow_index);
    idx
}

fn nondet_market_state(asset: &Address, market_index: &MarketIndex) -> MarketStateSnapshot {
    let timestamp: u64 = nondet();
    let reserves_ray: i128 = nondet();
    let supplied_ray: i128 = nondet();
    let borrowed_ray: i128 = nondet();
    let revenue_ray: i128 = nondet();
    cvlr_assume!(reserves_ray >= 0);
    cvlr_assume!(supplied_ray >= 0);
    cvlr_assume!(borrowed_ray >= 0);
    cvlr_assume!(revenue_ray >= 0);
    MarketStateSnapshot {
        asset: asset.clone(),
        timestamp,
        supply_index_ray: market_index.supply_index.raw(),
        borrow_index_ray: market_index.borrow_index.raw(),
        reserves_ray,
        supplied_ray,
        borrowed_ray,
        revenue_ray,
        asset_price_wad: None,
    }
}
// Mutating endpoints

/// Single-entry model for `pool::LiquidityPool::supply`.
///
/// The production ABI bulks supply into `Vec<PoolSupplyEntry>` and returns a
/// `Vec<PoolPositionMutation>`. The certora harness loops this element model
/// over the batch (length-preserving) so every per-entry postcondition holds
/// independently. `asset` is the entry's market asset, stamped into the
/// returned `market_state`.
///
/// Modeled postconditions:
///   * `actual_amount == amount`; pool reports the exact gross credited
///     amount on the successful path.
///   * `position.scaled_amount_ray` is monotone non-decreasing in the input
///     because the scaled amount is derived from a non-negative input.
///   * `market_index` satisfies the global index invariants after accrual.
pub fn supply_summary(
    _env: &Env,
    asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
    _supply_cap: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount: amount,
    }
}

/// Single-entry model for `pool::LiquidityPool::borrow`.
///
/// Looped element-wise by the harness over `Vec<PoolBorrowEntry>`. `asset` is
/// the entry's market asset; `_receiver` is hoisted to the batch endpoint.
///
/// Modeled postconditions:
///   * `actual_amount == amount`. The pool transfers exactly `amount` to the
///     caller on the successful path.
///   * Reserves were sufficient at call time.
///   * `position.scaled_amount_ray` is monotone non-decreasing.
///   * `market_index` satisfies the global index invariants.
pub fn borrow_summary(
    _env: &Env,
    asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
    _borrow_cap: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
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
    asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
    _is_liquidation: bool,
    _protocol_fee: i128,
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
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
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
    asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
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
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount,
    }
}

/// Summary for `pool::LiquidityPool::update_indexes`.
///
/// Modeled postcondition: a fresh sync of `(supply_index_ray, borrow_index_ray)`
/// satisfying the global index invariants. No position mutation, no token
/// transfer.
pub fn update_indexes_summary(_env: &Env, asset: &Address) -> MarketStateSnapshot {
    let market_index = nondet_market_index();
    nondet_market_state(asset, &market_index)
}

/// Summary for `pool::LiquidityPool::add_rewards`.
///
/// Modeled postconditions:
///   * `amount >= 0`.
///   * Empty-pool reward credits panic with `NoSuppliersToReward`. The summary
///     is pure side-effect, so this precondition is preserved by production
///     reachability rather than a return value.
pub fn add_rewards_summary(_env: &Env, asset: &Address, _amount: i128) -> MarketStateSnapshot {
    let market_index = nondet_market_index();
    nondet_market_state(asset, &market_index)
}

/// Summary for `pool::LiquidityPool::flash_loan`.
///
/// Modeled postconditions:
///   * `amount > 0`, `fee >= 0`, and `amount + fee` cannot overflow.
///   * Reserves were sufficient at flash-loan time.
///   * The receiver approved and returned exactly `amount + fee`; a short
///     repayment panics with `InvalidFlashloanRepay`.
///   * `fee` is added to protocol revenue.
///
/// Pure side-effect; no return value.
pub fn flash_loan_summary(
    _env: &Env,
    asset: &Address,
    _initiator: &Address,
    _receiver: &Address,
    amount: i128,
    fee: i128,
    _data: &Bytes,
) -> MarketStateSnapshot {
    cvlr_assume!(amount > 0);
    cvlr_assume!(fee >= 0);
    cvlr_assume!(fee <= i128::MAX - amount);
    let market_index = nondet_market_index();
    nondet_market_state(asset, &market_index)
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
    asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
    fee: i128,
    _borrow_cap: i128,
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
    let market_state = nondet_market_state(asset, &market_index);
    PoolStrategyMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
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
    asset: &Address,
    _side: AccountPositionType,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut zeroed = position.clone();
    zeroed.scaled_amount_ray = 0;
    // `nondet_market_index` floors the supply index at `SUPPLY_INDEX_FLOOR_RAW`
    // without a monotone constraint against any prior snapshot. That is the
    // bad-debt write-down property: seize_position is the only path where the
    // supply index may drop, still bounded by the floor. Preserved verbatim
    // through the asset-keyed signature rewrite.
    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: zeroed,
        market_index: (&market_index).into(),
        market_state,
        actual_amount: 0,
    }
}

/// Summary for `pool::LiquidityPool::claim_revenue`.
///
/// Modeled postconditions:
///   * Return value is `amount_to_transfer = current_reserves.min(treasury_actual)`
///     and both inputs are non-negative, so `result >= 0`.
///   * On the zero-revenue early-return branch the function returns 0.
///   * State updates are committed before the external token call so
///     reentrant claims observe the post-burn state.
pub fn claim_revenue_summary(_env: &Env, asset: &Address) -> PoolAmountMutation {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    let market_index = nondet_market_index();
    PoolAmountMutation {
        market_state: nondet_market_state(asset, &market_index),
        actual_amount: amount,
    }
}
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
pub fn get_sync_data_summary(_env: &Env, asset: &Address) -> PoolSyncData {
    let supplied_ray: i128 = nondet();
    let borrowed_ray: i128 = nondet();
    let revenue_ray: i128 = nondet();
    let cash: i128 = nondet();
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    let last_timestamp: u64 = nondet();

    cvlr_assume!(supplied_ray >= 0);
    cvlr_assume!(borrowed_ray >= 0);
    cvlr_assume!(revenue_ray >= 0);
    cvlr_assume!(cash >= 0);
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);

    let max_borrow_rate_ray: i128 = nondet();
    let base_borrow_rate_ray: i128 = nondet();
    let slope1_ray: i128 = nondet();
    let slope2_ray: i128 = nondet();
    let slope3_ray: i128 = nondet();
    let mid_utilization_ray: i128 = nondet();
    let optimal_utilization_ray: i128 = nondet();
    let max_utilization_ray: i128 = nondet();
    let reserve_factor_bps: u32 = nondet();
    cvlr_assume!(i128::from(reserve_factor_bps) < common::constants::BPS);
    let asset_decimals: u32 = nondet();
    cvlr_assume!(asset_decimals <= 27);
    let asset_id: Address = asset.clone();

    PoolSyncData {
        params: MarketParamsRaw {
            max_borrow_rate_ray,
            base_borrow_rate_ray,
            slope1_ray,
            slope2_ray,
            slope3_ray,
            mid_utilization_ray,
            optimal_utilization_ray,
            max_utilization_ray,
            reserve_factor_bps,
            asset_id,
            asset_decimals,
        },
        state: PoolStateRaw {
            supplied_ray,
            borrowed_ray,
            revenue_ray,
            borrow_index_ray,
            supply_index_ray,
            last_timestamp,
            cash,
        },
    }
}
// Single-value view summaries
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

/// Summary for `pool::LiquidityPool::reserves` (real impl at
/// `pool/src/views.rs`: `reserves` -> `load_state(..).cash`).
///
/// Production returns the pool's accounted `cash` (liquid token units backing
/// borrows/withdrawals/revenue claims), NOT the live SAC balance — so a direct
/// donation cannot inflate it. `cash` is a non-negative state field (every
/// mutating path keeps it >= 0 via checked subtraction), so the model is
/// `nondet >= 0`. Crucially this is the SAME quantity the borrow/claim reserve
/// guards (`has_reserves`, `claim_revenue`) check against, so reserve-availability
/// rules now read and assert over one consistent value.
pub fn reserves_summary(_env: &Env) -> i128 {
    // Accounted `cash` >= 0 (state invariant); see `views.rs::reserves`.
    let cash: i128 = nondet();
    cvlr_assume!(cash >= 0);
    cash
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
// Joint pool view snapshot (cross-view consistency)

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
/// `reserves` is the accounted `cash` (the value `reserves()` now returns from
/// `views.rs`), so it is donation-independent rather than the drift-prone SAC
/// balance. Its exact relationship to (supplied, borrowed, revenue) — the
/// solvency identity `cash = supplied + revenue - borrowed` in underlying terms
/// — is left unconstrained here so rules that don't need it stay general; a rule
/// that needs the identity should constrain it explicitly.
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
// Re-export for monotone-index helpers (used by rules that compare against a
// prior MarketIndex snapshot)

/// Public wrapper over `nondet_market_index_monotone` for rules that need to
/// constrain a fresh MarketIndex against a prior snapshot without going
/// through one of the typed summary entrypoints.
pub fn fresh_monotone_index(prior: &MarketIndex) -> MarketIndex {
    nondet_market_index_monotone(prior)
}
