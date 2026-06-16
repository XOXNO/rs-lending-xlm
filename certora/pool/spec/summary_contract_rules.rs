use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    AccountPositionType, MarketParamsRaw, PoolAction, PoolKey, PoolStateRaw, ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;

fn params(asset: Address) -> MarketParamsRaw {
    MarketParamsRaw {
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY / 2,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_borrow_rate_ray: 2 * RAY,
        max_utilization_ray: RAY,
        reserve_factor_bps: 1_000,
        asset_id: asset,
        asset_decimals: 7,
    }
}

fn state(supplied: i128, borrowed: i128, revenue: i128, timestamp: u64) -> PoolStateRaw {
    PoolStateRaw {
        supplied_ray: supplied,
        borrowed_ray: borrowed,
        revenue_ray: revenue,
        borrow_index_ray: RAY,
        supply_index_ray: RAY,
        last_timestamp: timestamp * 1000,
        cash: supplied.saturating_sub(borrowed),
    }
}

fn seed(env: &Env, admin: Address, asset: Address, state: PoolStateRaw) {
    crate::LiquidityPool::__constructor(env.clone(), admin);
    env.storage()
        .persistent()
        .set(&PoolKey::Params(asset.clone()), &params(asset.clone()));
    env.storage()
        .persistent()
        .set(&PoolKey::State(asset), &state);
}

fn position(scaled: i128) -> ScaledPositionRaw {
    ScaledPositionRaw {
        scaled_amount_ray: scaled,
    }
}

fn action(position: ScaledPositionRaw, amount: i128, asset: Address) -> PoolAction {
    PoolAction {
        position,
        amount,
        asset,
    }
}

// Bulk-of-one wrappers mirroring integrity_rules: rules verify per-entry
// semantics; bulk endpoints are input-ordered loops of that body.
fn supply_first(e: &Env, act: PoolAction, cap: i128) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolSupplyEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolSupplyEntry {
        action: act,
        supply_cap: cap,
    });
    crate::LiquidityPool::supply(e.clone(), entries).get_unchecked(0)
}

fn borrow_first(
    e: &Env,
    receiver: Address,
    act: PoolAction,
    cap: i128,
) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolBorrowEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolBorrowEntry {
        action: act,
        borrow_cap: cap,
    });
    crate::LiquidityPool::borrow(e.clone(), receiver, entries).get_unchecked(0)
}

fn withdraw_first(
    e: &Env,
    receiver: Address,
    act: PoolAction,
    is_liquidation: bool,
    protocol_fee: i128,
) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolWithdrawEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolWithdrawEntry {
        action: act,
        protocol_fee,
    });
    crate::LiquidityPool::withdraw(e.clone(), receiver, is_liquidation, entries).get_unchecked(0)
}

fn repay_first(e: &Env, payer: Address, act: PoolAction) -> common::types::PoolPositionMutation {
    let mut actions: soroban_sdk::Vec<PoolAction> = soroban_sdk::Vec::new(e);
    actions.push_back(act);
    crate::LiquidityPool::repay(e.clone(), payer, actions).get_unchecked(0)
}

#[rule]
fn supply_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(RAY);
    let result = supply_first(&e, action(before.clone(), amount, asset), i128::MAX);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn borrow_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = borrow_first(&e, caller, action(before.clone(), amount, asset), i128::MAX);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn withdraw_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    scaled: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!((1..=100 * RAY).contains(&scaled));
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(scaled);
    let result = withdraw_first(&e, caller, action(before.clone(), amount, asset), false, 0);

    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount || result.position.scaled_amount_ray == 0);
    cvlr_assert!(result.position.scaled_amount_ray <= before.scaled_amount_ray);
    cvlr_assert!(result.position.scaled_amount_ray >= 0);
}

#[rule]
fn repay_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    scaled: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!((1..=100 * RAY).contains(&scaled));
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, scaled, 0, e.ledger().timestamp()),
    );

    let before = position(scaled);
    let result = repay_first(&e, caller, action(before.clone(), amount, asset));

    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount);
    cvlr_assert!(result.position.scaled_amount_ray <= before.scaled_amount_ray);
    cvlr_assert!(result.position.scaled_amount_ray >= 0);
}

#[rule]
fn create_strategy_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    fee: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!(fee >= 0 && fee <= amount);
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = crate::LiquidityPool::create_strategy(
        e,
        caller,
        action(before.clone(), amount, asset),
        fee,
        i128::MAX,
    );

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.amount_received == amount - fee);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn seize_position_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    scaled: i128,
) {
    cvlr_assume!((1..=100 * RAY).contains(&scaled));
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, scaled, 0, e.ledger().timestamp()),
    );

    let result = crate::LiquidityPool::seize_position(
        e,
        asset,
        AccountPositionType::Borrow,
        position(scaled),
    );
    cvlr_assert!(result.position.scaled_amount_ray == 0);
}

#[rule]
fn claim_revenue_satisfies_controller_summary_contract(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, 0, RAY, e.ledger().timestamp()),
    );

    let amount = crate::LiquidityPool::claim_revenue(e, asset).actual_amount;
    cvlr_assert!(amount >= 0);
}

#[rule]
fn flash_loan_satisfies_fee_domain(
    e: Env,
    admin: Address,
    asset: Address,
    receiver: Address,
    amount: i128,
    fee: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!(fee >= 0 && fee <= amount);
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let revenue_before = crate::LiquidityPool::protocol_revenue(e.clone(), asset.clone());
    crate::LiquidityPool::flash_loan(
        e.clone(),
        asset.clone(),
        admin,
        receiver,
        amount,
        fee,
        Bytes::new(&e),
    );
    let revenue_after = crate::LiquidityPool::protocol_revenue(e, asset);

    cvlr_assert!(revenue_after == revenue_before + fee);
    cvlr_satisfy!(true);
}

// View-summary soundness
//
// The pool view summaries in `certora/shared/summaries/pool.rs`
// (`reserves_summary`, `supplied_amount_summary`, `borrowed_amount_summary`,
// `protocol_revenue_summary`, `capital_utilisation_summary`) assume bounds on
// the real views. Unlike the mutating-op summaries above, those view summaries
// had no soundness proof. These rules run the REAL `LiquidityPool` views on a
// seeded state satisfying the documented state invariants and assert the
// claimed bounds.
//
// Bounds mirror `integrity_rules::revenue_le_supplied_after_add_rewards`
// (amounts <= 1_000_000 * RAY) so the unscale math stays inside the same
// widening domain the existing pool proofs use.
//
// NOTE: `capital_utilisation <= RAY` is deliberately NOT asserted. The real
// view returns `borrowed/supplied` in RAY, which a bad-debt write-down can push
// above RAY (see the `capital_utilisation_summary` doc). Only `>= 0` is sound.

#[allow(clippy::too_many_arguments)]
fn view_state(
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
    timestamp: u64,
) -> PoolStateRaw {
    PoolStateRaw {
        supplied_ray: supplied,
        borrowed_ray: borrowed,
        revenue_ray: revenue,
        borrow_index_ray: borrow_index,
        supply_index_ray: supply_index,
        last_timestamp: timestamp * 1000,
        cash,
    }
}

/// `reserves` returns the accounted `cash` field unchanged; with the state
/// invariant `cash >= 0` it is non-negative.
#[rule]
fn reserves_view_nonneg(e: Env, admin: Address, asset: Address, cash: i128) {
    cvlr_assume!(cash >= 0 && cash <= 1_000_000_000_000i128);
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(10 * RAY, 0, 0, RAY, RAY, cash, e.ledger().timestamp()),
    );
    cvlr_assert!(crate::LiquidityPool::reserves(e, asset) >= 0);
}

/// `supplied_amount = unscale_supply(supplied_ray)`; non-negative inputs and a
/// floored supply index keep the rescaled asset amount non-negative.
#[rule]
fn supplied_amount_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    supply_index: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= 1_000_000 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            0,
            0,
            supply_index,
            RAY,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::supplied_amount(e, asset) >= 0);
}

/// `borrowed_amount = unscale_borrow(borrowed_ray)`; non-negative inputs and a
/// `>= RAY` borrow index keep the rescaled asset amount non-negative.
#[rule]
fn borrowed_amount_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    borrowed: i128,
    borrow_index: i128,
) {
    cvlr_assume!(borrowed >= 0 && borrowed <= 1_000_000 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            borrowed,
            borrowed,
            0,
            RAY,
            borrow_index,
            borrowed,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::borrowed_amount(e, asset) >= 0);
}

/// `protocol_revenue = unscale_supply(revenue_ray)`; non-negative revenue and a
/// floored supply index keep it non-negative.
#[rule]
fn protocol_revenue_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    revenue: i128,
    supply_index: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= 1_000_000 * RAY);
    cvlr_assume!(revenue >= 0 && revenue <= supplied);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            0,
            revenue,
            supply_index,
            RAY,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::protocol_revenue(e, asset) >= 0);
}

/// The cross-view identity `revenue <= supplied`. Both views unscale by the
/// SAME supply index (a monotone non-decreasing map), so the ray-level
/// invariant `revenue_ray <= supplied_ray` (proven preserved in
/// `integrity_rules`) carries to asset units.
#[rule]
fn protocol_revenue_le_supplied_view(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    revenue: i128,
    supply_index: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= 1_000_000 * RAY);
    cvlr_assume!(revenue >= 0 && revenue <= supplied);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            0,
            revenue,
            supply_index,
            RAY,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    let revenue_units = crate::LiquidityPool::protocol_revenue(e.clone(), asset.clone());
    let supplied_units = crate::LiquidityPool::supplied_amount(e, asset);
    cvlr_assert!(revenue_units <= supplied_units);
}

/// `capital_utilisation` returns `borrowed/supplied` in RAY (0 when supply is
/// empty); it is always non-negative. The `<= RAY` upper bound is intentionally
/// unproven — a bad-debt write-down can transiently exceed it.
#[rule]
fn capital_utilisation_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    supply_index: i128,
    borrow_index: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= 1_000_000 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= 1_000_000 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= 10 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            borrowed,
            0,
            supply_index,
            borrow_index,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::capital_utilisation(e, asset) >= 0);
}
