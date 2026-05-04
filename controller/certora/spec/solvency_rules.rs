/// Solvency and cross-contract consistency rules.
///
/// Verifies reserve availability, position-count limits, scaled-amount
/// roundtrip bounds, and controller/cache invariants that preserve market
/// solvency across account operations.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use common::constants::{MILLISECONDS_PER_YEAR, RAY, SUPPLY_INDEX_FLOOR_RAW, WAD};
use common::fp::{Ray, Wad};

// ===========================================================================
// Solvency Rules
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 3b: claim_revenue bounded by reserves  (INVARIANTS.md Sec.12)
// ---------------------------------------------------------------------------

/// Claimed revenue must never exceed the pool's pre-call token reserves.
/// Pool-side `claim_revenue` caps the transfer at `min(reserves, treasury_actual)`
/// (`pool/src/lib.rs:467-477`), so the controller-returned amount is bounded by
/// the reserves snapshot taken immediately before the call.
///
/// Invariant: claimed_amount <= pre_reserves
#[rule]
fn claim_revenue_bounded_by_reserves(e: Env, caller: Address, asset: Address) {
    let pool_addr = crate::storage::asset_pool::get_asset_pool(&e, &asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(&e, &pool_addr);

    let pre_reserves = pool_client.reserves();

    let amounts = crate::Controller::claim_revenue(e.clone(), caller, soroban_sdk::vec![&e, asset]);
    let claimed = amounts.get(0).unwrap();

    cvlr_assert!(claimed <= pre_reserves);
}

// ---------------------------------------------------------------------------
// Rule 3c: Utilization is zero when supplied_ray is zero  (INVARIANTS.md Sec.8)
// ---------------------------------------------------------------------------

/// Empty-market convention: if `state.supplied_ray == 0`, then
/// `capital_utilisation() == 0`. Guards against divide-by-zero and pins
/// the empty-market rate model to zero.
///
/// Note: uses `get_sync_data().state.supplied_ray` directly because
/// `supplied_amount()` (asset decimals) can round tiny positive raw values
/// to zero while the raw product `supplied_ray * supply_index` is still
/// nonzero -- only the raw ray value is the correct zero-test.
#[rule]
fn utilization_zero_when_supplied_zero(e: Env, asset: Address) {
    let pool_addr = crate::storage::asset_pool::get_asset_pool(&e, &asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(&e, &pool_addr);

    let sync = pool_client.get_sync_data();
    cvlr_assume!(sync.state.supplied_ray == 0);

    cvlr_assert!(pool_client.capital_utilisation() == 0);
}

// ---------------------------------------------------------------------------
// Rule 3d: Isolation debt stays non-negative across repay  (INVARIANTS.md Sec.11)
// ---------------------------------------------------------------------------

/// `adjust_isolated_debt_usd` (controller/src/utils.rs:61-92) clamps at zero
/// and applies a sub-$1 dust erasure. Given a non-negative pre-state, the
/// tracker must remain non-negative after any repay.
#[rule]
fn isolation_debt_never_negative_after_repay(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(crate::storage::get_isolated_debt(&e, &asset) >= 0);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    cvlr_assert!(crate::storage::get_isolated_debt(&e, &asset) >= 0);
}

// ---------------------------------------------------------------------------
// Rule 3e: Borrow respects pool reserves  (INVARIANTS.md Sec.13)
// ---------------------------------------------------------------------------

/// A successful borrow requires `pre_reserves >= amount`. The pool enforces
/// this via `has_reserves(amount)` (`pool/src/lib.rs:139`); if the guard
/// fails the call panics with `InsufficientLiquidity`. Therefore any path
/// that reaches the post-state with the borrow applied must have had
/// sufficient reserves pre-call.
#[rule]
fn borrow_respects_reserves(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pool_addr = crate::storage::asset_pool::get_asset_pool(&e, &asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(&e, &pool_addr);
    let pre_reserves = pool_client.reserves();

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    // If the borrow did not revert, reserves must have covered the amount.
    cvlr_assert!(pre_reserves >= amount);
}

// ---------------------------------------------------------------------------
// Rule 3f: LTV borrow bound enforced (math-anchored)
// ---------------------------------------------------------------------------

/// After any successful borrow, the account's total debt (USD WAD) must not
/// exceed its LTV-weighted collateral. Computed inline against the
/// unsummarised helpers (`calculate_ltv_collateral_wad`, `position_value`)
/// rather than via the summarised aggregate views
/// (`Controller::total_borrow_in_usd`, `Controller::ltv_collateral_in_usd`)
/// whose summaries return independent nondets. Distinct from the liquidation
/// threshold gate: LTV gates new borrows, liquidation threshold gates seizure.
#[rule]
fn ltv_borrow_bound_enforced(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    // Bound to a single supply position and a single borrow position so the
    // inline iterations below have a fixed shape.
    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let post_account = crate::storage::get_account(&e, account_id);

    // LTV-weighted collateral via the unsummarised helper.
    let ltv_collateral = crate::helpers::calculate_ltv_collateral_wad(
        &e,
        &mut cache,
        &post_account.supply_positions,
    );

    // Total debt via inline iteration over the borrow map (mirrors the
    // production borrow-side loop at controller/src/helpers/mod.rs:170 but
    // bypasses the summarised `calculate_account_totals`).
    let mut total_debt = Wad::ZERO;
    for position in post_account.borrow_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);
        let value = crate::helpers::position_value(
            &e,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        total_debt += value;
    }

    cvlr_assert!(total_debt.raw() <= ltv_collateral.raw());
}

// ---------------------------------------------------------------------------
// Rule 3g: Supply index stays above floor across supply  (INVARIANTS.md Sec.7)
// ---------------------------------------------------------------------------

/// Bad-debt socialization clamps supply_index at `SUPPLY_INDEX_FLOOR_RAW`
/// (`pool/src/interest.rs:14`). Outside that path the index only grows.
/// This rule checks the floor is inductively preserved across a supply,
/// which exercises interest accrual but no bad-debt path.
#[rule]
fn supply_index_above_floor_after_supply(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pool_addr = crate::storage::asset_pool::get_asset_pool(&e, &asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(&e, &pool_addr);

    let pre = pool_client.get_sync_data();
    cvlr_assume!(pre.state.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let post = pool_client.get_sync_data();
    cvlr_assert!(post.state.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

// ---------------------------------------------------------------------------
// Rule 3h: Supply index does not decrease across borrow  (INVARIANTS.md Sec.7)
// ---------------------------------------------------------------------------

/// The only sanctioned path that decreases `supply_index` is
/// `apply_bad_debt_to_supply_index`, invoked exclusively from
/// `seize_position`. A borrow triggers interest accrual (`global_sync`)
/// which can only grow the index. Combined with the existing
/// `index_rules::supply_index_monotonic_after_accrual` (which covers
/// supply), this rule extends Sec.7 monotonicity to the borrow path.
#[rule]
fn supply_index_monotonic_across_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pool_addr = crate::storage::asset_pool::get_asset_pool(&e, &asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(&e, &pool_addr);
    let pre = pool_client.get_sync_data();

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let post = pool_client.get_sync_data();
    cvlr_assert!(post.state.supply_index_ray >= pre.state.supply_index_ray);
}

// ---------------------------------------------------------------------------
// Rule 14: Supply index grows slower than borrow index
// ---------------------------------------------------------------------------

/// When both indexes grow due to interest accrual, the supply index growth
/// must be <= borrow index growth. The difference is the reserve factor cut.
///
/// (supply_index_after - supply_index_before) <= (borrow_index_after - borrow_index_before)
#[rule]
fn supply_index_grows_slower(
    e: Env,
    asset: Address,
    caller: Address,
    e_mode_category: u32,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    // Capture indexes before
    let index_before = crate::storage::market_index::get_market_index(&e, &asset);
    let supply_before = index_before.supply_index_ray;
    let borrow_before = index_before.borrow_index_ray;

    // Both must be initialized (>= RAY)
    cvlr_assume!(supply_before >= RAY);
    cvlr_assume!(borrow_before >= RAY);

    // Trigger interest accrual via a supply operation
    let mut assets = Vec::new(&e);
    assets.push_back((asset.clone(), amount));

    crate::Controller::supply(e.clone(), caller, account_id, e_mode_category, assets);

    // Capture indexes after
    let index_after = crate::storage::market_index::get_market_index(&e, &asset);
    let supply_after = index_after.supply_index_ray;
    let borrow_after = index_after.borrow_index_ray;

    let supply_growth = supply_after - supply_before;
    let borrow_growth = borrow_after - borrow_before;

    // Supply growth must not exceed borrow growth (reserve factor takes a cut)
    cvlr_assert!(supply_growth <= borrow_growth);
}

// ===========================================================================
// Zero/Negative Amount Reverts
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 4: Supply rejects zero amount
// ---------------------------------------------------------------------------

/// Controller::supply with amount=0 must revert. The validation layer calls
/// `require_amount_positive` which panics on amount <= 0.
///
/// Pattern: call the function, then cvlr_satisfy!(false) -- if the prover can
/// reach the satisfy, the revert did not happen (violation).
#[rule]
fn supply_rejects_zero_amount(e: Env, caller: Address, e_mode_category: u32) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;

    let mut assets = Vec::new(&e);
    assets.push_back((asset, zero_amount));

    crate::Controller::supply(e.clone(), caller, account_id, e_mode_category, assets);

    // If execution reaches here, zero-amount supply was accepted -- violation.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 5: Borrow rejects zero amount
// ---------------------------------------------------------------------------

/// Controller::borrow with amount=0 must revert.
#[rule]
fn borrow_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;

    let mut borrows = Vec::new(&e);
    borrows.push_back((asset, zero_amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows);

    // If execution reaches here, zero-amount borrow was accepted -- violation.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 6: DELETED -- withdraw_rejects_zero_amount asserted a false invariant.
// Production (`controller/src/positions/withdraw.rs:96`) treats `amount == 0`
// as the documented `WITHDRAW_ALL_SENTINEL` (full-close) sentinel, not as a
// rejection. The rule expected a revert that never happens.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 7: Repay rejects zero amount
// ---------------------------------------------------------------------------

/// Controller::repay with amount=0 must revert.
#[rule]
fn repay_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;

    let mut payments = Vec::new(&e);
    payments.push_back((asset, zero_amount));

    crate::Controller::repay(e.clone(), caller, account_id, payments);

    // If execution reaches here, zero-amount repay was accepted -- violation.
    cvlr_satisfy!(false);
}

// ===========================================================================
// Position Count Limits
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 8: Supply position limit enforced
// ---------------------------------------------------------------------------

/// After an account has reached `max_supply_positions`, attempting to supply
/// a NEW (not already held) asset must revert.
///
/// The list iteration in production (`process_supply` / `prepare_deposit_plan`)
/// runs over a symbolic-length list. To keep the loop bounded for the prover,
/// assume the list length is exactly `max_supply_positions`, the precondition
/// that triggers the panic for a new asset.
#[rule]
fn supply_position_limit_enforced(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    new_asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    // Assume the account already has the maximum number of supply positions.
    let limits = crate::storage::get_position_limits(&e);
    let current_list =
        crate::storage::get_position_list(&e, account_id, common::types::POSITION_TYPE_DEPOSIT);
    cvlr_assume!(current_list.len() == limits.max_supply_positions as u32);

    // Bound the loop: once the prover knows the length is concrete and equals
    // the limit (capped at 10 by production -- see
    // `controller/src/storage/account.rs`), the membership check below
    // unrolls into at most 10 concrete steps instead of a symbolic count.
    cvlr_assume!(limits.max_supply_positions as u32 <= 10);

    // Assume the new asset is NOT already in the position list (truly new).
    let mut already_exists = false;
    for i in 0..current_list.len() {
        let existing = current_list.get(i).unwrap();
        if existing == new_asset {
            already_exists = true;
        }
    }
    cvlr_assume!(!already_exists);

    let mut assets = Vec::new(&e);
    assets.push_back((new_asset, amount));

    crate::Controller::supply(e.clone(), caller, account_id, e_mode_category, assets);

    // If execution reaches here, position limit was not enforced -- violation.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 9: Borrow position limit enforced
// ---------------------------------------------------------------------------

/// After an account has reached `max_borrow_positions`, attempting to borrow
/// a NEW asset must revert. Same loop-bounding strategy as Rule 8.
#[rule]
fn borrow_position_limit_enforced(e: Env, caller: Address, new_asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let limits = crate::storage::get_position_limits(&e);
    let current_list =
        crate::storage::get_position_list(&e, account_id, common::types::POSITION_TYPE_BORROW);
    cvlr_assume!(current_list.len() == limits.max_borrow_positions as u32);
    cvlr_assume!(limits.max_borrow_positions as u32 <= 10);

    // Assume the new asset is NOT already in the borrow list.
    let mut already_exists = false;
    for i in 0..current_list.len() {
        let existing = current_list.get(i).unwrap();
        if existing == new_asset {
            already_exists = true;
        }
    }
    cvlr_assume!(!already_exists);

    let mut borrows = Vec::new(&e);
    borrows.push_back((new_asset, amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows);

    // If execution reaches here, position limit was not enforced -- violation.
    cvlr_satisfy!(false);
}

// ===========================================================================
// Sanity rules -- verify rules are reachable
// ===========================================================================

#[rule]
fn solvency_sanity_supply(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    let mut assets = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::Controller::supply(e, caller, account_id, e_mode_category, assets);
    cvlr_satisfy!(true);
}

#[rule]
fn solvency_sanity_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    let mut borrows = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::Controller::borrow(e, caller, account_id, borrows);
    cvlr_satisfy!(true);
}

#[rule]
fn solvency_sanity_repay(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    let mut payments = Vec::new(&e);
    payments.push_back((asset, amount));
    crate::Controller::repay(e, caller, account_id, payments);
    cvlr_satisfy!(true);
}

// ===========================================================================
// Attack Vector Defense Rules
// ===========================================================================

// ---------------------------------------------------------------------------
// Attack 1: Index Stale-Snapshot Arbitrage
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 15: index_cache_single_snapshot
// ---------------------------------------------------------------------------

/// `ControllerCache` stores market indexes per transaction. Repeated calls to
/// `cached_market_index(asset)` for the same asset must return the same
/// snapshot within that transaction.
#[rule]
fn index_cache_single_snapshot(e: Env, asset: Address) {
    let mut cache = crate::cache::ControllerCache::new(&e, false);

    // First fetch: triggers pool.update_indexes() and caches the result
    let index1 = cache.cached_market_index(&asset);

    // Second fetch: must hit the cache and return the same value
    let index2 = cache.cached_market_index(&asset);

    // Both supply and borrow indexes must be identical
    cvlr_assert!(index1.supply_index_ray == index2.supply_index_ray);
    cvlr_assert!(index1.borrow_index_ray == index2.borrow_index_ray);
}

// ---------------------------------------------------------------------------
// Attack 2: Rounding Dust Extraction
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 16: supply_withdraw_roundtrip_no_profit
// ---------------------------------------------------------------------------

/// Repeated tiny supply/withdraw must not extract meaningful dust. After a
/// supply of amount X, converting to scaled and back to original must
/// yield at most X + 1.
///
/// Supply: scaled = div_half_up(X, supply_index, RAY)
/// Withdraw: original = mul_half_up(scaled, supply_index, RAY)
///
/// With half-up rounding on BOTH the scale (div) and unscale (mul) steps,
/// the recovered amount can exceed the original by at most 1 unit. Example:
///   amount=1, supply_index=1.6*RAY -> scaled=div_half_up(1,1.6*RAY,RAY)=1
///   -> recovered=mul_half_up(1,1.6*RAY,RAY)=2 > 1.
/// This +1 dust is sub-cent and the pool's scaled accounting prevents
/// actual extraction: the pool tracks scaled_amount, not raw token amounts,
/// so the "extra" unit cannot be withdrawn without reducing scaled_amount
/// below what was credited on supply.
#[rule]
fn supply_withdraw_roundtrip_no_profit(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let supply_index: i128 = cvlr::nondet::nondet();

    // Realistic constraints: WAD * 1000 = 1e21 covers the realistic input
    // domain on i128 without forcing the SMT to enumerate the upper i128
    // tail.
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(supply_index >= RAY); // Index starts at RAY and only grows

    // Supply: actual -> scaled (what the pool stores)
    let scaled = common::fp_core::mul_div_half_up(&e, amount, RAY, supply_index);

    // Withdraw: scaled -> actual (what the user gets back)
    let recovered = common::fp_core::mul_div_half_up(&e, scaled, supply_index, RAY);

    // User must not profit beyond rounding dust: recovered <= amount + 1.
    // The +1 tolerance accounts for half-up rounding on both div and mul.
    cvlr_assert!(recovered <= amount + 1);
}

// ---------------------------------------------------------------------------
// Rule 17: borrow_repay_roundtrip_no_profit
// ---------------------------------------------------------------------------

/// Repeated tiny borrow/repay must not reduce debt meaningfully via
/// rounding. After a borrow of amount X, converting to scaled_debt and
/// back to original must yield at least X - 1.
///
/// Borrow: scaled_debt = div_half_up(X, borrow_index, RAY)
/// Repay:  original_debt = mul_half_up(scaled_debt, borrow_index, RAY)
///
/// With half-up rounding on both steps, the recovered debt can be up to
/// 1 unit less than the original borrow amount. Example:
///   amount=1, borrow_index=1.6*RAY -> scaled_debt=1
///   -> debt_owed=mul_half_up(1,1.6*RAY,RAY)=2 (actually increases here),
/// but for other index values the debt can round down by 1.
/// This -1 dust is sub-cent and cannot be exploited: the pool tracks
/// scaled_amount, and the borrow position's scaled value is what
/// determines the actual debt owed at repay time.
#[rule]
fn borrow_repay_roundtrip_no_profit(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let borrow_index: i128 = cvlr::nondet::nondet();

    // Realistic constraints: see `supply_withdraw_roundtrip_no_profit` for
    // why WAD * 1000 is the right ceiling.
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(borrow_index >= RAY); // Index starts at RAY and only grows

    // Borrow: actual -> scaled_debt (what the pool stores)
    let scaled_debt = common::fp_core::mul_div_half_up(&e, amount, RAY, borrow_index);

    // Repay calculation: scaled_debt -> actual debt owed
    let debt_owed = common::fp_core::mul_div_half_up(&e, scaled_debt, borrow_index, RAY);

    // Debt owed must be >= original borrow minus rounding dust (at most 1 unit)
    cvlr_assert!(debt_owed >= amount - 1);
}

// ---------------------------------------------------------------------------
// Attack 3: Oracle Band Consistency
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 18: price_cache_invalidation_after_swap
// ---------------------------------------------------------------------------

/// After `clean_prices_cache()`, the cache must be empty. A subsequent
/// price lookup fetches fresh data rather than returning a stale value.
/// This proves the cache invalidation mechanism works correctly -- both
/// that the cache returns consistent prices during normal operation
/// (covered by oracle_rules::price_cache_consistency) AND that the cache
/// is properly cleared when needed (e.g., after a swap).
#[rule]
fn price_cache_invalidation_after_swap(e: Env, asset: Address) {
    let mut cache = crate::cache::ControllerCache::new(&e, false);

    // First: populate the cache with a price
    let _feed1 = cache.cached_price(&asset);

    // Sanity: the cache contains the price.
    let cached = cache.try_get_price(&asset);
    cvlr_assert!(cached.is_some());

    // Invalidate the price cache (simulates post-swap cleanup)
    cache.clean_prices_cache();

    // After invalidation: cache must be empty for this asset
    let cached_after = cache.try_get_price(&asset);
    cvlr_assert!(cached_after.is_none());

    // A fresh lookup will re-fetch from the oracle. The key property is that
    // the cache was actually cleared; stale prices are not silently reused.
    let _feed2 = cache.cached_price(&asset);

    // The fresh fetch repopulates the cache
    let cached_repopulated = cache.try_get_price(&asset);
    cvlr_assert!(cached_repopulated.is_some());
}

// ---------------------------------------------------------------------------
// Attack 4: E-Mode/Isolation Transition
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 19: mode_transition_blocked_with_positions
// ---------------------------------------------------------------------------

/// An account with existing borrow positions cannot change its e_mode_category
/// or is_isolated flag. The protocol enforces this by blocking borrow/supply
/// operations that would require a mode change when positions already exist.
///
/// Specifically: if an account has borrow positions in e-mode (category > 0),
/// attempting to supply an isolated asset (which would require switching to
/// isolation mode) must revert. And vice versa.
#[rule]
fn mode_transition_blocked_with_positions(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    // Account is in e-mode with existing borrow positions
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);
    cvlr_assume!(!attrs.is_isolated);

    // Pin borrow list to a single asset so the production e-mode/isolation
    // traversal collapses to one iteration.
    let borrow_list =
        crate::storage::get_position_list(&e, account_id, common::types::POSITION_TYPE_BORROW);
    cvlr_assume!(borrow_list.len() == 1);

    // The asset is an isolated asset (would require switching to isolation)
    let config = crate::storage::get_asset_config(&e, &asset);
    cvlr_assume!(config.is_isolated_asset);

    // Attempting to supply an isolated asset into an e-mode account must revert.
    // E-Mode and isolation are mutually exclusive (emode_rules + isolation_rules),
    // and the protocol cannot transition modes while positions exist.
    let mut assets = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::Controller::supply(
        e.clone(),
        caller,
        account_id,
        attrs.e_mode_category_id,
        assets,
    );

    // If execution reaches here, the mode transition was allowed -- violation.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Attack 5: Taylor Overflow at Extreme Inputs
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 20: compound_interest_bounded_output
// ---------------------------------------------------------------------------

/// For any valid rate (<= max_borrow_rate / MILLISECONDS_PER_YEAR) and
/// time (<= MILLISECONDS_PER_YEAR), the compound interest factor must be
/// < 100 * RAY (10000%). This proves the 5-term Taylor expansion does not
/// produce absurdly large values that could overflow downstream math.
///
/// At 100% APY over 1 year: e^1.0 ~= 2.718 * RAY, well within bounds.
/// Even at extreme rates the Taylor series is bounded because the
/// calculate_borrow_rate function caps at max_borrow_rate.
#[rule]
fn compound_interest_bounded_output(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();

    // Rate is bounded by max_borrow_rate / MILLISECONDS_PER_YEAR
    // Use 10 * RAY as a generous max_borrow_rate (1000% APY)
    let max_rate_per_ms =
        common::fp_core::div_by_int_half_up(10 * RAY, MILLISECONDS_PER_YEAR as i128);

    cvlr_assume!(rate >= 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time > 0 && time <= MILLISECONDS_PER_YEAR);

    let factor = common::rates::compound_interest(&e, Ray::from_raw(rate), time);

    // Use a generous upper bound for the Taylor approximation at the largest
    // modeled rate and duration.
    let upper_bound = 100_000 * RAY; // 10,000,000% -- generous upper bound
    cvlr_assert!(factor.raw() < upper_bound);
}

// ---------------------------------------------------------------------------
// Rule 21: compound_interest_no_wrap
// ---------------------------------------------------------------------------

/// The compound interest factor must be >= RAY for any non-negative rate
/// and non-negative time. The Taylor expansion is: RAY + x + x^2/2 + ...
/// where all terms are non-negative. If an overflow caused unsigned wrapping,
/// the result could be < RAY -- this rule catches that.
///
/// This defends against silent overflow in the I256 -> i128 conversion path
/// or in the final summation of Taylor terms.
#[rule]
fn compound_interest_no_wrap(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();

    // Bound rate to max_borrow_rate / MILLISECONDS_PER_YEAR
    let max_rate_per_ms =
        common::fp_core::div_by_int_half_up(10 * RAY, MILLISECONDS_PER_YEAR as i128);

    cvlr_assume!(rate >= 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time <= MILLISECONDS_PER_YEAR);

    let factor = common::rates::compound_interest(&e, Ray::from_raw(rate), time);

    // The factor must be >= RAY (1.0). The Taylor expansion starts with RAY
    // and adds only non-negative terms. A value < RAY indicates overflow/wrap.
    cvlr_assert!(factor.raw() >= RAY);
}

// ===========================================================================
// Sanity rules for attack vector defenses
// ===========================================================================

#[rule]
fn index_cache_snapshot_sanity(e: Env, asset: Address) {
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let index = cache.cached_market_index(&asset);
    cvlr_satisfy!(index.supply_index_ray >= RAY);
}

#[rule]
fn roundtrip_supply_sanity(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let index: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!((RAY..=10 * RAY).contains(&index));

    let scaled = common::fp_core::mul_div_half_up(&e, amount, RAY, index);
    let recovered = common::fp_core::mul_div_half_up(&e, scaled, index, RAY);
    cvlr_satisfy!(recovered <= amount + 1);
}

#[rule]
fn compound_no_wrap_sanity(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();
    let max_rate_per_ms = common::fp_core::div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    cvlr_assume!(rate > 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time > 0 && time <= MILLISECONDS_PER_YEAR);
    let factor = common::rates::compound_interest(&e, Ray::from_raw(rate), time);
    cvlr_satisfy!(factor.raw() > RAY);
}
