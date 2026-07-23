//! Reward, liquidation-fee, strategy, and protocol-revenue accounting.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_FLASHLOAN_FEE_BPS, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR,
    RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::Ray;
use common::math::fp_core;
use common::rates::{
    calculate_borrow_rate, compound_interest, supply_index_reward_shortfall, update_borrow_index,
    update_supply_index,
};
use common::types::PoolWithdrawEntry;
use pool_interface::LiquidityPoolInterface;

use super::fixture::{
    action, expected_protocol_fee_shares, hub, params, read_state, seed, state, ASSET_DECIMALS,
    MAX_FLOW_AMOUNT, ONE_TOKEN,
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
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= 200_000_000 * RAY);
    let reward = Ray::from_asset(reward_amount, ASSET_DECIMALS);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            0,
            0,
            RAY,
            supply_index,
            80 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let expected_index = update_supply_index(
        &e,
        Ray::from(pre.supplied),
        Ray::from(pre.supply_index),
        reward,
    );
    cvlr_assert!(expected_index.raw() >= pre.supply_index);
    let old_value = Ray::from(pre.supplied).mul(&e, Ray::from(pre.supply_index));
    let new_value = Ray::from(pre.supplied).mul(&e, expected_index);
    let distributed = new_value.checked_sub(&e, old_value);
    cvlr_assert!(distributed.raw() <= reward.raw());
    let shortfall = supply_index_reward_shortfall(
        &e,
        Ray::from(pre.supplied),
        Ray::from(pre.supply_index),
        expected_index,
        reward,
    );
    let fee_shares =
        expected_protocol_fee_shares(&e, shortfall, expected_index, Ray::from(pre.supplied));
    cvlr_assert!(fee_shares.mul_floor(&e, expected_index).raw() <= shortfall.raw());

    crate::LiquidityPool::add_rewards(e.clone(), hub(asset.clone()), reward_amount);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.supply_index == expected_index.raw());
    cvlr_assert!(post.supply_index >= pre.supply_index);
    cvlr_assert!(post.supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!(distributed.raw() + shortfall.raw() == reward.raw());
    cvlr_assert!(post.revenue - pre.revenue == fee_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == fee_shares.raw());
    cvlr_assert!(post.cash - pre.cash == reward_amount);
    cvlr_assert!(post.borrowed == pre.borrowed && post.borrow_index == pre.borrow_index);
}

/// A positive one-chunk `global_sync` grows neither index backwards, mints
/// identical protocol-fee shares into revenue and aggregate supply, leaves
/// cash fixed, and advances the synchronization timestamp to the ledger time.
#[rule]
#[allow(clippy::too_many_arguments)]
fn one_chunk_global_sync_preserves_accounting_and_advances_time(
    e: Env,
    admin: Address,
    asset: Address,
    borrowed: i128,
    borrow_index: i128,
    supply_index: i128,
    delta_ms: u64,
) {
    let supplied = 100 * RAY;
    cvlr_assume!(borrowed > 0 && borrowed <= 50 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= 10 * RAY);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    cvlr_assume!(delta_ms > 0 && delta_ms <= MILLISECONDS_PER_YEAR);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let current_timestamp = crate::utils::now_ms(&e);
    cvlr_assume!(delta_ms <= current_timestamp);
    let borrowed_value = Ray::from(borrowed).mul(&e, Ray::from(borrow_index));
    let supplied_value = Ray::from(supplied).mul(&e, Ray::from(supply_index));
    cvlr_assume!(borrowed_value.raw() <= supplied_value.raw());
    let mut initial_state = state(
        supplied,
        borrowed,
        5 * RAY,
        borrow_index,
        supply_index,
        200 * ONE_TOKEN,
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

    let mut cache = crate::cache::Cache::load(&e, &hub(asset));
    let supplied_before = cache.supplied;
    let revenue_before = cache.revenue;
    let cash_before = cache.cash;
    let old_supply_index = cache.supply_index;
    let expected_utilization = borrowed_value.div(&e, supplied_value);
    let expected_rate = calculate_borrow_rate(&e, expected_utilization, &cache.params);
    let expected_factor = compound_interest(&e, expected_rate, delta_ms);
    let expected_borrow_index = update_borrow_index(&e, Ray::from(borrow_index), expected_factor);
    let old_debt = Ray::from(borrowed).mul(&e, Ray::from(borrow_index));
    let new_debt = Ray::from(borrowed).mul(&e, expected_borrow_index);
    cvlr_assert!(new_debt.raw() >= old_debt.raw());
    let accrued = new_debt.checked_sub(&e, old_debt);
    let expected_protocol_fee = Ray::from(fp_core::mul_div_half_up(
        &e,
        accrued.raw(),
        cache.params.reserve_factor.raw(),
        BPS,
    ));
    cvlr_assert!(expected_protocol_fee.raw() <= accrued.raw());
    let expected_supplier_rewards = accrued.checked_sub(&e, expected_protocol_fee);
    let expected_supply_index = update_supply_index(
        &e,
        supplied_before,
        old_supply_index,
        expected_supplier_rewards,
    );
    cvlr_assert!(expected_supply_index.raw() >= old_supply_index.raw());
    let old_supplier_value = supplied_before.mul(&e, old_supply_index);
    let new_supplier_value = supplied_before.mul(&e, expected_supply_index);
    let distributed = new_supplier_value.checked_sub(&e, old_supplier_value);
    cvlr_assert!(distributed.raw() <= expected_supplier_rewards.raw());
    let expected_shortfall = expected_supplier_rewards.checked_sub(&e, distributed);
    let expected_protocol_reward = expected_protocol_fee.checked_add(&e, expected_shortfall);
    let expected_fee_shares = expected_protocol_fee_shares(
        &e,
        expected_protocol_reward,
        expected_supply_index,
        supplied_before,
    );
    cvlr_assert!(
        expected_fee_shares
            .mul_floor(&e, expected_supply_index)
            .raw()
            <= expected_protocol_reward.raw()
    );

    crate::interest::global_sync(&e, &mut cache);

    cvlr_assert!(cache.borrow_index == expected_borrow_index);
    cvlr_assert!(cache.borrow_index.raw() <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(cache.supply_index == expected_supply_index);
    cvlr_assert!(cache.supply_index.raw() <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!(cache.borrowed.raw() == borrowed);
    cvlr_assert!(cache.revenue.raw() - revenue_before.raw() == expected_fee_shares.raw());
    cvlr_assert!(cache.supplied.raw() - supplied_before.raw() == expected_fee_shares.raw());
    cvlr_assert!(cache.cash == cash_before);
    cvlr_assert!(cache.last_timestamp == current_timestamp);
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
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
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
    let expected_fee_shares = expected_protocol_fee_shares(
        &e,
        Ray::from_asset(protocol_fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    cvlr_assert!(
        expected_fee_shares
            .mul_floor(&e, Ray::from(supply_index))
            .raw()
            <= Ray::from_asset(protocol_fee, ASSET_DECIMALS).raw()
    );
    let expected_burn =
        Ray::from_asset(gross_amount, ASSET_DECIMALS).div_ceil(&e, Ray::from(supply_index));
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
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
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
    let rounded_fee = fp_core::mul_div_half_up(&e, amount, i128::from(flashloan_fee), BPS);
    let configured_fee = if flashloan_fee > 0 && rounded_fee == 0 {
        1
    } else {
        rounded_fee
    };
    let expected_fee = if charge_fee { configured_fee } else { 0 };
    let expected_fee_shares = expected_protocol_fee_shares(
        &e,
        Ray::from_asset(expected_fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    cvlr_assert!(
        expected_fee_shares
            .mul_floor(&e, Ray::from(supply_index))
            .raw()
            <= Ray::from_asset(expected_fee, ASSET_DECIMALS).raw()
    );
    let expected_debt =
        Ray::from_asset(amount, ASSET_DECIMALS).div_ceil(&e, Ray::from(borrow_index));

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
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
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
    let expected_burn = if expected_claim <= 0 {
        Ray::ZERO
    } else if expected_claim >= treasury_actual {
        Ray::from(revenue_before)
    } else {
        let ratio = Ray::from_fraction(&e, expected_claim, treasury_actual);
        Ray::from(revenue_before).mul(&e, ratio)
    };
    let (_, result) = crate::claim_revenue_accounting(&e, hub(asset.clone()));
    let post = read_state(&e, &asset);
    let burned_revenue = pre.revenue - post.revenue;

    cvlr_assert!(result.actual_amount == expected_claim);
    cvlr_assert!(pre.cash - post.cash == expected_claim);
    cvlr_assert!(pre.supplied - post.supplied == burned_revenue);
    cvlr_assert!(burned_revenue == expected_burn.raw());
    cvlr_assert!(burned_revenue >= 0 && burned_revenue <= revenue_before);
    cvlr_assert!(post.borrowed == pre.borrowed);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
    cvlr_assert!(treasury_actual == 0 || expected_claim != treasury_actual || post.revenue == 0);
}
