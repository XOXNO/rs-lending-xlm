// Wrappers around LiquidityPool cross-contract ABI.

use common::types::{
    AccountPosition, AccountPositionType, MarketStateSnapshot, PoolAmountMutation,
    PoolPositionMutation, PoolStrategyMutation, PoolSyncData,
};
use soroban_sdk::{Address, Bytes, Env};


pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    position: AccountPosition,
    amount: i128,
    supply_cap: i128,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).supply(&position, &amount, &supply_cap)
}

pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    caller: Address,
    amount: i128,
    position: AccountPosition,
    borrow_cap: i128,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).borrow(
        &caller,
        &amount,
        &position,
        &borrow_cap,
    )
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    caller: Address,
    position: AccountPosition,
    amount: i128,
    fee: i128,
    borrow_cap: i128,
) -> PoolStrategyMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).create_strategy(
        &caller,
        &position,
        &amount,
        &fee,
        &borrow_cap,
    )
}

pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    caller: Address,
    amount: i128,
    position: AccountPosition,
    is_liquidation: bool,
    protocol_fee: i128,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).withdraw(
        &caller,
        &amount,
        &position,
        &is_liquidation,
        &protocol_fee,
    )
}

pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    caller: Address,
    amount: i128,
    position: AccountPosition,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).repay(&caller, &amount, &position)
}

pub(crate) fn pool_seize_position_call(
    env: &Env,
    pool_addr: &Address,
    side: AccountPositionType,
    position: AccountPosition,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).seize_position(&side, &position)
}


pub(crate) fn pool_flash_loan_call(
    env: &Env,
    pool_addr: &Address,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    fee: i128,
    data: &Bytes,
) -> MarketStateSnapshot {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).flash_loan(
        initiator, receiver, &amount, &fee, data,
    )
}

pub(crate) fn pool_update_indexes_call(env: &Env, pool_addr: &Address) -> MarketStateSnapshot {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_indexes()
}

pub(crate) fn pool_claim_revenue_call(env: &Env, pool_addr: &Address) -> PoolAmountMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).claim_revenue()
}

pub(crate) fn pool_add_rewards_call(
    env: &Env,
    pool_addr: &Address,
    amount: i128,
) -> MarketStateSnapshot {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).add_rewards(&amount)
}

pub(crate) fn fetch_pool_sync_data(env: &Env, pool_addr: &Address) -> PoolSyncData {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).get_sync_data()
}
