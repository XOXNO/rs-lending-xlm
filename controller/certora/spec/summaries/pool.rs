//! Summaries for the `LiquidityPool` contract (`pool/src/lib.rs`).
//!
//! Cross-contract pool calls are pure havoc to the prover and the pool
//! mutating paths reach into Cache + interest accrual + I256 scaled-amount
//! math, which trips the TAC command-count budget. Every domain rule that
//! traverses a pool call becomes vacuous over havoced returns; the bounds
//! captured here are the post-conditions production guarantees so downstream
//! reasoning has something to lean on.
//!
//! Each summary's `cvlr_assume!` bounds are derived directly from the
//! production source line ranges noted on the doc comment. Bounds capture
//! only what production ENFORCES on the return value (post-conditions);
//! anything stricter would silently hide bugs.
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

/// Summary for `pool::LiquidityPool::supply` (lines 132-162).
///
/// Production guarantees:
///   * `actual_amount == amount` (line 160) -- pool reports the exact gross
///     credited; `amount` is already non-negative (line 139).
///   * `position.scaled_amount_ray` is monotone non-decreasing in the input
///     (lines 144-148: checked-add of `scaled_amount` derived from a
///     non-negative `amount`).
///   * `market_index` satisfies the global index invariants after
///     `interest::global_sync` (line 142).
pub fn supply_summary(
    _env: &Env,
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

/// Summary for `pool::LiquidityPool::borrow` (lines 164-203).
///
/// Production guarantees:
///   * `actual_amount == amount` (line 201). The pool transfers exactly
///     `amount` to the caller (line 190); the controller already validated
///     positivity and applied any flash-loan fee elsewhere.
///   * Reserves were sufficient at call time (lines 177-179: panics with
///     `InsufficientLiquidity` otherwise).
///   * `position.scaled_amount_ray` is monotone non-decreasing (lines 181-185).
///   * `market_index` satisfies the global index invariants.
pub fn borrow_summary(
    _env: &Env,
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

/// Summary for `pool::LiquidityPool::withdraw` (lines 205-285).
///
/// Production guarantees:
///   * `actual_amount` is the gross withdrawn before any liquidation fee
///     (line 283: returned as `gross_amount`). It is bounded above by the
///     position's current_supply_actual (lines 226-243: full-withdraw branch
///     caps at `current_supply_actual`; partial branch returns `amount`).
///   * `actual_amount >= 0` (input is checked non-negative at line 217).
///   * `position.scaled_amount_ray` is monotone non-increasing (lines 261-266).
///   * `market_index` satisfies the global index invariants.
pub fn withdraw_summary(
    _env: &Env,
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

/// Summary for `pool::LiquidityPool::repay` (lines 287-350).
///
/// Production guarantees:
///   * `actual_amount = amount.min(current_debt)` (line 338). With
///     `amount >= 0` (line 295) and `current_debt >= 0`, this gives
///     `0 <= actual_amount <= amount`.
///   * Overpayment beyond `current_debt` is refunded to the caller
///     (lines 333-336); the position cannot go negative because the
///     scaled-balance subtraction is checked (lines 319-322).
///   * `position.scaled_amount_ray` is monotone non-increasing (line 322).
///   * `market_index` satisfies the global index invariants.
pub fn repay_summary(
    _env: &Env,
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

/// Summary for `pool::LiquidityPool::update_indexes` (lines 352-365).
///
/// Production guarantees: a fresh sync of `(supply_index_ray, borrow_index_ray)`
/// satisfying the global index invariants. No position mutation, no token
/// transfer.
pub fn update_indexes_summary(_env: &Env, _price_wad: i128) -> MarketIndex {
    nondet_market_index()
}

/// Summary for `pool::LiquidityPool::add_rewards` (lines 367-387).
///
/// Production guarantees:
///   * `amount >= 0` is enforced at line 369.
///   * `cache.supplied != Ray::ZERO` is enforced at lines 375-377; calling
///     into an empty pool panics with `NoSuppliersToReward`. The summary is
///     pure side-effect (no return) so this precondition is preserved by
///     the prover via the production reachability check, not the summary
///     itself.
pub fn add_rewards_summary(_env: &Env, _price_wad: i128, _amount: i128) {}

/// Summary for `pool::LiquidityPool::flash_loan_begin` (lines 389-413).
///
/// Production guarantees:
///   * `amount >= 0` (line 391).
///   * Reserves were sufficient at begin time (lines 396-398).
///   * Pre-balance snapshot is recorded in instance storage; any subsequent
///     `flash_loan_end` will read it back.
///
/// Pure side-effect; no return value.
pub fn flash_loan_begin_summary(_env: &Env, _amount: i128, _receiver: Address) {}

/// Summary for `pool::LiquidityPool::flash_loan_end` (lines 415-456).
///
/// Production guarantees:
///   * `amount >= 0` (line 417), `fee >= 0` (lines 422-424).
///   * The receiver returned at least `amount + fee` (lines 444-449); a
///     short repayment panics with `InvalidFlashloanRepay`.
///   * `fee` is added to protocol revenue (line 452).
///
/// Pure side-effect; no return value.
pub fn flash_loan_end_summary(_env: &Env, _amount: i128, _fee: i128, _receiver: Address) {}

/// Summary for `pool::LiquidityPool::create_strategy` (lines 458-508).
///
/// Production guarantees:
///   * `amount >= 0`, `fee >= 0` (lines 467-468).
///   * `fee <= amount` (lines 473-475: panics with `StrategyFeeExceeds`).
///   * Reserves cover `amount` (lines 476-478).
///   * `actual_amount == amount`, `amount_received == amount - fee` (lines
///     490, 505-506). `amount_received >= 0` follows from `fee <= amount`.
///   * `position.scaled_amount_ray` is monotone non-decreasing (debt added).
///   * `market_index` satisfies the global index invariants.
pub fn create_strategy_summary(
    _env: &Env,
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

/// Summary for `pool::LiquidityPool::seize_position` (lines 510-545).
///
/// Production guarantees:
///   * Returned position has `scaled_amount_ray == 0` (lines 530, 535).
///     The full position is consumed -- borrow branch socializes the debt
///     into the supply index (line 524) and zeroes the scaled balance;
///     deposit branch absorbs the residual into pool revenue.
///   * The supply index may DROP in the borrow branch (bad-debt write-down
///     via `apply_bad_debt_to_supply_index`), still floored at
///     `SUPPLY_INDEX_FLOOR_RAW`. Unlike every other path, `seize_position`
///     is the only place where `supply_index_ray` is permitted to decrease.
///   * Other index/state invariants still hold:
///     `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW`,
///     `borrow_index_ray >= RAY`.
pub fn seize_position_summary(
    _env: &Env,
    position: AccountPosition,
    _price_wad: i128,
) -> AccountPosition {
    let mut zeroed = position.clone();
    zeroed.scaled_amount_ray = 0;
    // Production restricts mutation to {Deposit, Borrow}; an unknown variant
    // panics with `InvalidPositionType` (line 539).
    cvlr_assume!(
        position.position_type == AccountPositionType::Deposit
            || position.position_type == AccountPositionType::Borrow
    );
    zeroed
}

/// Summary for `pool::LiquidityPool::claim_revenue` (lines 547-600).
///
/// Production guarantees:
///   * Return value is `amount_to_transfer = current_reserves.min(treasury_actual)`
///     (line 566). Both inputs are non-negative, so `result >= 0`.
///   * On the zero-revenue early-return branch (lines 552-556) the function
///     returns 0 unconditionally.
///   * State updates are committed before the external token call so
///     reentrant claims observe the post-burn state (lines 584-585).
pub fn claim_revenue_summary(_env: &Env, _price_wad: i128) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

// ---------------------------------------------------------------------------
// Read-only views
// ---------------------------------------------------------------------------

/// Summary for `pool::LiquidityPool::get_sync_data` (lines 736-749).
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
pub fn get_sync_data_summary(_env: &Env) -> PoolSyncData {
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
    let reserve_factor_bps: i128 = nondet();
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
// Re-export for monotone-index helpers (used by rules that compare against a
// prior MarketIndex snapshot)
// ---------------------------------------------------------------------------

/// Public wrapper over `nondet_market_index_monotone` for rules that need to
/// constrain a fresh MarketIndex against a prior snapshot without going
/// through one of the typed summary entrypoints.
pub fn fresh_monotone_index(prior: &MarketIndex) -> MarketIndex {
    nondet_market_index_monotone(prior)
}
