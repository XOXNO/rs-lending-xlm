use cvlr::cvlr_satisfy;
use cvlr::macros::rule;
use soroban_sdk::{Address, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;
use common::rates::{calculate_borrow_rate, compound_interest, update_borrow_index};
use common::types::{
    AccountPositionType, MarketParams, PoolBorrowEntry, PoolNetSettleEntry, PoolSeizeEntry,
    PoolSupplyEntry, PoolWithdrawEntry,
};
use pool_interface::LiquidityPoolInterface;

use super::fixture::{action, hub, params, position, read_state, seed, state, ONE_TOKEN};
use crate::ops::flash::FlashTerms;
use crate::ops::strategy::StrategyOutcome;

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
    let (supplied, _) = crate::ops::supply::apply(&e, &supply_entry);
    let borrow_entry = PoolBorrowEntry {
        action: action(asset, 0, ONE_TOKEN),
    };
    let borrowed = crate::ops::borrow::accounting(&e, &borrow_entry).mutation;
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
    let withdrawn = crate::ops::withdraw::accounting(&e, false, &withdraw).mutation;
    let repay = action(asset, 10 * RAY, ONE_TOKEN);
    let repaid = crate::ops::repay::accounting(&e, &repay).mutation;
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
    crate::ops::seize::apply(&e, &seized);
    let settle = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: ONE_TOKEN,
        supply_position: position(5 * RAY),
        debt_position: position(5 * RAY),
    };
    let (settled, _) = crate::ops::net_settle::apply(&e, &settle);
    let after = read_state(&e, &asset);
    cvlr_satisfy!(
        after.borrowed < before.borrowed
            && after.supply_index < before.supply_index
            && settled.settled_amount == ONE_TOKEN
    );
}

#[rule]
fn seize_floor_residual_reachable(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(100 * RAY, 100 * RAY, 0, RAY, RAY, 0, e.ledger().timestamp()),
    );
    let seized = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Borrow,
        position: position(100 * RAY),
    };
    crate::ops::seize::apply(&e, &seized);
    let post = read_state(&e, &asset);
    let legacy_claim = Ray::from(post.supplied).mul_floor(&e, Ray::from(post.supply_index));

    cvlr_satisfy!(
        post.borrowed == 0
            && post.cash == 0
            && post.supply_index == SUPPLY_INDEX_FLOOR_RAW
            && legacy_claim.raw() > 0
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
    let StrategyOutcome {
        mutation: strategy,
        fee,
        ..
    } = crate::ops::strategy::accounting(&e, action(asset.clone(), 0, ONE_TOKEN), true);
    let claim = crate::ops::revenue::accounting(&e, hub(asset)).mutation;
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
    let FlashTerms {
        fee,
        total_repayment: total,
        balance_after_payout: after_payout,
        balance_after_repayment: after_repayment,
    } = crate::ops::flash::terms(&e, ONE_TOKEN, 50, 100 * ONE_TOKEN);
    let mut cache = crate::cache::Cache::load(&e, &hub(asset));
    crate::ops::flash::book_fee(&mut cache, fee);
    cvlr_satisfy!(
        fee > 0
            && total == ONE_TOKEN + fee
            && after_repayment - after_payout == total
            && cache.cash() == 100 * ONE_TOKEN + fee
    );
}
