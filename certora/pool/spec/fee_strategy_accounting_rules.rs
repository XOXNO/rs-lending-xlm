//! Reward, liquidation-fee, strategy, and protocol-revenue accounting.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{MAX_FLASHLOAN_FEE_BPS, RAY};
use common::math::fp::{Bps, Ray};
use common::rates::{protocol_fee_shares, supply_index_reward_shortfall, update_supply_index};
use common::types::PoolWithdrawEntry;
use pool_interface::LiquidityPoolInterface;

use super::fixture::{
    action, hub, params, read_state, seed, state, ASSET_DECIMALS, MAX_FLOW_AMOUNT, ONE_TOKEN,
};

/// Reward cash is split between supplier index growth and shortfall revenue;
/// the shortfall mints identical deltas into revenue and aggregate supply.
#[rule]
fn add_rewards_accounts_cash_index_and_shortfall(
    e: Env,
    admin: Address,
    asset: Address,
    reward_amount: i128,
    supply_index: i128,
) {
    cvlr_assume!(reward_amount >= 0 && reward_amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            RAY,
            supply_index,
            80 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let reward = Ray::from_asset(reward_amount, ASSET_DECIMALS);
    let expected_index = update_supply_index(
        &e,
        Ray::from(pre.supplied),
        Ray::from(pre.supply_index),
        reward,
    );
    let shortfall = supply_index_reward_shortfall(
        &e,
        Ray::from(pre.supplied),
        Ray::from(pre.supply_index),
        expected_index,
        reward,
    );
    let fee_shares = protocol_fee_shares(&e, shortfall, expected_index, Ray::from(pre.supplied));

    crate::LiquidityPool::add_rewards(e.clone(), hub(asset.clone()), reward_amount);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.supply_index == expected_index.raw());
    cvlr_assert!(post.supply_index >= pre.supply_index);
    cvlr_assert!(post.revenue - pre.revenue == fee_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == fee_shares.raw());
    cvlr_assert!(post.cash - pre.cash == reward_amount);
    cvlr_assert!(post.borrowed == pre.borrowed && post.borrow_index == pre.borrow_index);
}

/// Liquidation withdrawal retains the fee in cash and books exactly the same
/// fee shares into revenue and aggregate supply before burning user shares.
#[rule]
fn liquidation_withdraw_books_protocol_fee(
    e: Env,
    admin: Address,
    asset: Address,
    gross_amount: i128,
    protocol_fee: i128,
    position_before: i128,
    supply_index: i128,
) {
    cvlr_assume!(gross_amount > 0 && gross_amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(protocol_fee >= 0 && protocol_fee <= gross_amount);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    let current_actual = Ray::from(position_before)
        .mul(&e, Ray::from(supply_index))
        .to_asset(ASSET_DECIMALS);
    cvlr_assume!(gross_amount < current_actual);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            RAY,
            supply_index,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let expected_fee_shares = protocol_fee_shares(
        &e,
        Ray::from_asset(protocol_fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    let expected_burn =
        Ray::from_asset(gross_amount, ASSET_DECIMALS).div(&e, Ray::from(supply_index));
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, gross_amount),
        protocol_fee,
    };
    let (_, result, _, net) = crate::withdraw_accounting(&e, true, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(result.actual_amount == gross_amount);
    cvlr_assert!(net == gross_amount - protocol_fee);
    cvlr_assert!(post.revenue - pre.revenue == expected_fee_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == expected_fee_shares.raw() - expected_burn.raw());
    cvlr_assert!(position_before - result.position.scaled_amount == expected_burn.raw());
    cvlr_assert!(pre.cash - post.cash == gross_amount - protocol_fee);
    cvlr_assert!(post.borrowed == pre.borrowed);
}

/// Strategy borrow records gross debt, sends the net amount, and books the
/// configured optional fee into revenue and aggregate supply in lockstep.
#[rule]
#[allow(clippy::too_many_arguments)]
fn create_strategy_accounts_debt_cash_and_fee(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    debt_before: i128,
    borrow_index: i128,
    supply_index: i128,
    flashloan_fee: u32,
    charge_fee: bool,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before >= 0 && debt_before <= 10 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= 10 * RAY);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    cvlr_assume!(i128::from(flashloan_fee) <= MAX_FLASHLOAN_FEE_BPS);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), flashloan_fee, true),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            borrow_index,
            supply_index,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let expected_fee = if charge_fee {
        Bps::from(i128::from(flashloan_fee)).flash_loan_fee_on(&e, amount)
    } else {
        0
    };
    let expected_fee_shares = protocol_fee_shares(
        &e,
        Ray::from_asset(expected_fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    let expected_debt = Ray::from_asset(amount, ASSET_DECIMALS).div(&e, Ray::from(borrow_index));

    let (_, result, fee) = crate::create_strategy_accounting(
        &e,
        action(asset.clone(), debt_before, amount),
        charge_fee,
    );
    let post = read_state(&e, &asset);

    cvlr_assert!(fee == expected_fee);
    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.amount_received == amount - expected_fee);
    cvlr_assert!(result.position.scaled_amount - debt_before == expected_debt.raw());
    cvlr_assert!(post.borrowed - pre.borrowed == expected_debt.raw());
    cvlr_assert!(post.revenue - pre.revenue == expected_fee_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == expected_fee_shares.raw());
    cvlr_assert!(pre.cash - post.cash == amount - expected_fee);
}

/// Revenue claim burns identical revenue/supply shares and debits exactly the
/// returned claim, bounded by both cash and the floor-valued treasury claim.
#[rule]
fn claim_revenue_burns_equal_shares_and_cash(
    e: Env,
    admin: Address,
    asset: Address,
    revenue_before: i128,
    cash_before: i128,
    supply_index: i128,
) {
    cvlr_assume!(revenue_before >= 0 && revenue_before <= 20 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 200 * ONE_TOKEN);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            0,
            revenue_before,
            RAY,
            supply_index,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let treasury_actual = Ray::from(revenue_before)
        .mul_floor(&e, Ray::from(supply_index))
        .to_asset_floor(ASSET_DECIMALS);
    let expected_claim = cash_before.min(treasury_actual);
    let (_, result) = crate::claim_revenue_accounting(&e, hub(asset.clone()));
    let post = read_state(&e, &asset);
    let burned_revenue = pre.revenue - post.revenue;

    cvlr_assert!(result.actual_amount == expected_claim);
    cvlr_assert!(pre.cash - post.cash == expected_claim);
    cvlr_assert!(pre.supplied - post.supplied == burned_revenue);
    cvlr_assert!(burned_revenue >= 0 && burned_revenue <= revenue_before);
    cvlr_assert!(post.borrowed == pre.borrowed);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
    cvlr_assert!(treasury_actual == 0 || expected_claim != treasury_actual || post.revenue == 0);
}
