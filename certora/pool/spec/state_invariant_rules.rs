//! Inductive invariant over the pool's persisted market state.
//!
//! Every other pool rule *assumes* a well-formed start state and proves one
//! operation correct from it. That leaves a gap: nothing shows the assumed
//! states are the reachable ones. This module closes it.
//!
//! `invariant_holds_after_market_create` is the base case — the only way a
//! market comes into existence. Every other rule is an induction step: assume
//! the invariant on a fully symbolic pre-state, run one production transition,
//! assert the invariant on the persisted post-state. Together they cover every
//! writer of `PoolKey::State`, so the invariant holds on all reachable states
//! and the assumptions made by the other rule modules are discharged.
//!
//! The invariant (see `assume_invariant` / `assert_invariant`):
//!
//! ```text
//! supplied >= 0, borrowed >= 0, revenue >= 0
//! revenue <= supplied
//! SUPPLY_INDEX_FLOOR_RAW <= supply_index <= MAX_SUPPLY_INDEX_RAY
//! RAY                    <= borrow_index <= MAX_BORROW_INDEX_RAY
//! cash >= 0
//! ```
//!
//! # What is deliberately excluded
//!
//! **Magnitude ceilings are not invariant conjuncts.** The `<= 100 * RAY` and
//! `<= 1_000 * ONE_TOKEN` bounds in `assume_invariant` are solver-tractability
//! bounds on the symbolic domain. They are assumed but *not* asserted, because
//! a supply legitimately grows the totals past any fixed ceiling. The induction
//! is therefore closed over the invariant's *shape* — signs, the revenue
//! ordering, and both index bands — but not over magnitude. Overflow safety is
//! carried by `checked_add` / `checked_sub_nonneg` in `common::math::fp`, not
//! by this module.
//!
//! **`borrowed <= supplied` is not an invariant.** Several rules in the other
//! modules assume it over *shares*. It is deliberately absent here because it is
//! false on reachable states, and a counterexample is pinned by
//! `test_borrowed_shares_exceed_supplied_shares_after_add_rewards` in
//! `contracts/pool/tests/flows.rs`.
//!
//! The lever is `add_rewards`. `interest::distribute_reward` raises
//! `supply_index` and never touches `borrow_index`, and `ops::rewards::apply`
//! credits the whole reward to `cash`. After that a borrowed token still mints
//! ~1 debt share while a supplied token mints only `1 / supply_index` supply
//! shares, and the reward cash is what satisfies `require_reserves`. From a
//! fresh market with a *strict* 80% cap: supply 100 tokens, `add_rewards(900)`
//! (`supply_index` -> 9.9109 RAY, `borrow_index` stays RAY, `cash` -> 1_000),
//! then borrow 500. The persisted state is `supplied = 1.00899e29`,
//! `borrowed = 5e29` — 4.96x inverted — at 50% utilization, with every
//! production guard satisfied rather than bypassed.
//!
//! The cap cannot rescue it: `require_utilization_below_max` compares *values*,
//! so the share ordering flips exactly when
//! `utilization > borrow_index / supply_index`, and `add_rewards` drives that
//! ratio arbitrarily far below 1 (up to `SUPPLY_INDEX_REWARD_CEILING_RAY`,
//! `1e5 * RAY`). `max_utilization == RAY` is not required; it only widens the
//! room. Adding the conjunct here would therefore assert something production
//! does not maintain.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR, RAY, RAY_DECIMALS,
    SUPPLY_INDEX_FLOOR_RAW,
};
use common::types::{
    AccountPositionType, PoolBorrowEntry, PoolNetSettleEntry, PoolSeizeEntry, PoolStateRaw,
    PoolSupplyEntry, PoolWithdrawEntry,
};

use super::fixture::{
    action, hub, params, params_with_decimals, position, read_state, seed, state, MAX_FLOW_AMOUNT,
    ONE_TOKEN,
};

/// Domain ceiling on share totals. Tractability bound, not an invariant.
const MAX_SHARES: i128 = 100 * RAY;

/// Domain ceiling on tracked cash. Tractability bound, not an invariant.
const MAX_CASH: i128 = 1_000 * ONE_TOKEN;

/// Constrains a symbolic pre-state to the invariant.
fn assume_invariant(
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= MAX_SHARES);
    cvlr_assume!(borrowed >= 0 && borrowed <= MAX_SHARES);
    cvlr_assume!(revenue >= 0 && revenue <= supplied);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(cash >= 0 && cash <= MAX_CASH);
}

/// Asserts the invariant on a persisted post-state.
fn assert_invariant(post: &PoolStateRaw) {
    cvlr_assert!(post.supplied >= 0);
    cvlr_assert!(post.borrowed >= 0);
    cvlr_assert!(post.revenue >= 0);
    cvlr_assert!(post.revenue <= post.supplied);
    cvlr_assert!(post.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assert!(post.supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!(post.borrow_index >= RAY);
    cvlr_assert!(post.borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(post.cash >= 0);
}

/// Seeds a market whose state is symbolic but invariant-respecting.
#[allow(clippy::too_many_arguments)]
fn seed_invariant_market(
    e: &Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    assume_invariant(
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    seed(
        e,
        admin,
        asset.clone(),
        params(asset, 0, false),
        state(
            supplied,
            borrowed,
            revenue,
            borrow_index,
            supply_index,
            cash,
            e.ledger().timestamp(),
        ),
    );
}

// ---------------------------------------------------------------------------
// Base case
// ---------------------------------------------------------------------------

#[rule]
fn invariant_holds_after_market_create(e: Env, asset: Address, asset_decimals: u32) {
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    crate::ops::market::create(
        &e,
        0,
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
    );

    assert_invariant(&read_state(&e, &asset));
}

// ---------------------------------------------------------------------------
// Induction steps — position flows
// ---------------------------------------------------------------------------

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_supply(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolSupplyEntry {
        action: action(asset.clone(), position_before, amount),
    };
    crate::ops::supply::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_borrow(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolBorrowEntry {
        action: action(asset.clone(), position_before, amount),
    };
    crate::ops::borrow::accounting(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_withdraw(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    protocol_fee: i128,
    position_before: i128,
    is_liquidation: bool,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(protocol_fee >= 0 && protocol_fee <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee,
    };
    crate::ops::withdraw::accounting(&e, is_liquidation, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_repay(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let repay_action = action(asset.clone(), position_before, amount);
    crate::ops::repay::accounting(&e, &repay_action);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_net_settle(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supply_position: i128,
    debt_position: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_position >= 0 && supply_position <= MAX_SHARES);
    cvlr_assume!(debt_position >= 0 && debt_position <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount,
        supply_position: position(supply_position),
        debt_position: position(debt_position),
    };
    crate::ops::net_settle::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

// ---------------------------------------------------------------------------
// Induction steps — loss and revenue
// ---------------------------------------------------------------------------

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_seize_borrow(
    e: Env,
    admin: Address,
    asset: Address,
    seized: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(seized >= 0 && seized <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Borrow,
        position: position(seized),
    };
    crate::ops::seize::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_seize_deposit(
    e: Env,
    admin: Address,
    asset: Address,
    seized: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(seized >= 0 && seized <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Deposit,
        position: position(seized),
    };
    crate::ops::seize::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_add_rewards(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    crate::ops::rewards::apply(&e, hub(asset.clone()), amount);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_claim_revenue(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    crate::ops::revenue::accounting(&e, hub(asset.clone()));

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_recapitalize(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    crate::ops::recapitalize::accounting(&e, hub(asset.clone()), amount);

    assert_invariant(&read_state(&e, &asset));
}

// ---------------------------------------------------------------------------
// Induction steps — debt-minting side channels
// ---------------------------------------------------------------------------

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_create_strategy(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    charge_fee: bool,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let strategy_action = action(asset.clone(), position_before, amount);
    crate::ops::strategy::accounting(&e, strategy_action, charge_fee);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_flash_fee_booking(
    e: Env,
    admin: Address,
    asset: Address,
    fee: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(fee >= 0 && fee <= MAX_FLOW_AMOUNT);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let mut cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    crate::ops::flash::book_fee(&mut cache, fee);
    cache.commit();

    assert_invariant(&read_state(&e, &asset));
}

// ---------------------------------------------------------------------------
// Induction step — interest accrual
//
// Bounded to one production chunk, matching
// `one_chunk_global_sync_preserves_accounting_and_advances_time`. Arbitrary
// multi-year loop completeness stays out of scope.
// ---------------------------------------------------------------------------

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_one_chunk_accrual(
    e: Env,
    admin: Address,
    asset: Address,
    delta_ms: u64,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    assume_invariant(
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );
    cvlr_assume!(delta_ms > 0 && delta_ms <= MILLISECONDS_PER_YEAR);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let current_timestamp = crate::time::now_ms(&e);
    cvlr_assume!(delta_ms <= current_timestamp);

    let mut initial_state = state(
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        cash,
        e.ledger().timestamp(),
    );
    initial_state.last_timestamp = current_timestamp - delta_ms;
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        initial_state,
    );

    crate::ops::market::accrue(&e, hub(asset.clone()));

    assert_invariant(&read_state(&e, &asset));
}
