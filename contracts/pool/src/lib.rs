#![no_std]

mod cache;
mod events;
mod guards;
mod interest;
mod ops;
mod storage;
mod time;
mod views;

#[cfg(test)]
#[path = "../tests/test_support.rs"]
mod test_support;

#[cfg(feature = "certora")]
#[path = "../../../certora/pool/spec/mod.rs"]
pub mod spec;

use common::rates::simulate_update_indexes;
use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};

use pool_interface::LiquidityPoolInterface;

use soroban_sdk::{contract, contractimpl, contractmeta, Address, Bytes, BytesN, Env, Vec};

use stellar_access::ownable;
use stellar_macros::only_owner;

contractmeta!(key = "name", val = "Liquidity Pool");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        ops::market::create(&env, hub_id, params);
    }

    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        ops::market::replace_rate_model(&env, hub_asset, model);
    }

    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, ops::supply::apply)
    }

    #[only_owner]
    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, |env, entry| {
            ops::borrow::apply(env, &receiver, entry)
        })
    }

    #[only_owner]
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, |env, entry| {
            ops::withdraw::apply(env, &receiver, is_liquidation, entry)
        })
    }

    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, actions, |env, action| {
            ops::repay::apply(env, &payer, action)
        })
    }

    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        ops::market::accrue(&env, hub_asset);
    }

    #[only_owner]
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128) {
        ops::rewards::apply(&env, hub_asset, amount);
    }

    #[only_owner]
    fn recapitalize(
        env: Env,
        hub_asset: HubAssetKey,
        payer: Address,
        amount: i128,
    ) -> PoolAmountMutation {
        ops::recapitalize::apply(&env, hub_asset, payer, amount)
    }

    #[only_owner]
    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        data: Bytes,
    ) -> i128 {
        ops::flash::apply(&env, hub_asset, initiator, receiver, amount, data)
    }

    #[only_owner]
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation {
        ops::strategy::apply(&env, &receiver, action, charge_fee)
    }

    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        ops::run_batch_without_result(&env, entries, ops::seize::apply);
    }

    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        storage::renew_instance(&env);
        let (result, snapshot) = ops::net_settle::apply(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    #[only_owner]
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        ops::revenue::apply(&env, hub_asset)
    }

    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::utilization(&env, &hub_asset)
    }

    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        storage::load_sync_data(&env, &hub_asset)
    }

    fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw> {
        let now = time::now_ms(&env);
        let mut indexes = Vec::new(&env);
        for hub_asset in hub_assets.iter() {
            let sync = storage::load_sync_data(&env, &hub_asset);
            indexes.push_back(MarketIndexRaw::from(&simulate_update_indexes(
                &env, now, &sync,
            )));
        }
        indexes
    }
}

#[cfg(test)]
#[path = "../tests/lib_orchestration.rs"]
mod lib_orchestration_tests;

#[cfg(test)]
#[path = "../tests/flows.rs"]
mod tests;
