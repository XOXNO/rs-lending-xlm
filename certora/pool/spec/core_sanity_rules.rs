//! Concrete reachability witnesses. No universal property lives in this module.

use cvlr::cvlr_satisfy;
use cvlr::macros::rule;
use soroban_sdk::{Address, Env};

use common::constants::RAY;
use common::math::fp::Ray;
use common::rates::{calculate_borrow_rate, compound_interest, update_borrow_index};
use common::types::{
    AccountPositionType, MarketParams, PoolBorrowEntry, PoolNetSettleEntry, PoolSeizeEntry,
    PoolSupplyEntry, PoolWithdrawEntry,
};
use pool_interface::LiquidityPoolInterface;

use super::fixture::{action, hub, params, position, read_state, seed, state, ONE_TOKEN};

#[rule]
fn rate_index_domain_reachable(e: Env, asset: Address) {
    let params = MarketParams::from(&params(asset, 0, false));
    let rate = calculate_borrow_rate(&e, Ray::from(RAY / 2), &params);
    let factor = compound_interest(&e, rate, 1_000);
    let index = update_borrow_index(&e, Ray::ONE, factor);
    cvlr_satisfy!(rate.raw() > 0 && factor.raw() >= RAY && index.raw() >= RAY);
}

#[rule]
fn supply_borrow_domain_reachable(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            10 * RAY,
            0,
            RAY,
            RAY,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );
    let supply_entry = PoolSupplyEntry {
        action: action(asset.clone(), 0, ONE_TOKEN),
    };
    let (supplied, _) = crate::supply_one(&e, &supply_entry);
    let borrow_entry = PoolBorrowEntry {
        action: action(asset, 0, ONE_TOKEN),
    };
    let (_, borrowed, _) = crate::borrow_accounting(&e, &borrow_entry);
    cvlr_satisfy!(
        supplied.position.scaled_amount > 0
            && borrowed.position.scaled_amount > 0
            && supplied.actual_amount == ONE_TOKEN
            && borrowed.actual_amount == ONE_TOKEN
    );
}

#[rule]
fn withdraw_repay_domain_reachable(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            20 * RAY,
            0,
            RAY,
            RAY,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );
    let withdraw = PoolWithdrawEntry {
        action: action(asset.clone(), 10 * RAY, ONE_TOKEN),
        protocol_fee: 0,
    };
    let (_, withdrawn, _, _) = crate::withdraw_accounting(&e, false, &withdraw);
    let repay = action(asset, 10 * RAY, ONE_TOKEN);
    let (_, repaid, _, _) = crate::repay_accounting(&e, &repay);
    cvlr_satisfy!(
        withdrawn.position.scaled_amount < 10 * RAY
            && repaid.position.scaled_amount < 10 * RAY
            && withdrawn.actual_amount == ONE_TOKEN
            && repaid.actual_amount == ONE_TOKEN
    );
}

#[rule]
fn seize_settle_domain_reachable(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            20 * RAY,
            0,
            RAY,
            RAY,
            100 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );
    let before = read_state(&e, &asset);
    let seized = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Borrow,
        position: position(RAY),
    };
    crate::seize_one(&e, &seized);
    let settle = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: ONE_TOKEN,
        supply_position: position(5 * RAY),
        debt_position: position(5 * RAY),
    };
    let (settled, _) = crate::net_settle_one(&e, &settle);
    let after = read_state(&e, &asset);
    cvlr_satisfy!(
        after.borrowed < before.borrowed
            && after.supply_index < before.supply_index
            && settled.settled_amount == ONE_TOKEN
    );
}

#[rule]
fn fee_strategy_claim_domain_reachable(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 50, true),
        state(
            100 * RAY,
            10 * RAY,
            RAY,
            RAY,
            RAY,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );
    crate::LiquidityPool::add_rewards(e.clone(), hub(asset.clone()), ONE_TOKEN);
    let (_, strategy, fee) =
        crate::create_strategy_accounting(&e, action(asset.clone(), 0, ONE_TOKEN), true);
    let (_, claim) = crate::claim_revenue_accounting(&e, hub(asset));
    cvlr_satisfy!(
        fee > 0
            && strategy.position.scaled_amount > 0
            && strategy.amount_received == ONE_TOKEN - fee
            && claim.actual_amount > 0
    );
}

#[rule]
fn flash_accounting_domain_reachable(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 50, true),
        state(
            100 * RAY,
            10 * RAY,
            0,
            RAY,
            RAY,
            100 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );
    let (fee, total, after_payout, after_repayment) =
        crate::flash_repayment_terms(&e, ONE_TOKEN, 50, 100 * ONE_TOKEN);
    let mut cache = crate::cache::Cache::load(&e, &hub(asset));
    crate::book_flash_fee(&mut cache, fee);
    cvlr_satisfy!(
        fee > 0
            && total == ONE_TOKEN + fee
            && after_repayment - after_payout == total
            && cache.cash == 100 * ONE_TOKEN + fee
    );
}
