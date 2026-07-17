//! Certora storage accessors for controller rules.
//! Spec models hub 0: asset-keyed reads use `HubAssetKey { hub_id: 0, asset }`.

#![allow(dead_code)]
use super::*;
use crate::types::{
    AccountAttributes, AccountPositionRaw, AccountPositionType, HubAssetKey, MarketIndex,
    MarketParamsRaw, PositionMode,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Env, Vec};

/// Hub-0 coordinate for `asset`.
pub fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

pub fn get_position(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
    asset: &Address,
) -> Option<AccountPositionRaw> {
    let hub_asset = hub0(asset);
    match position_type {
        AccountPositionType::Deposit => get_supply_positions(env, account_id).get(hub_asset),
        // Debt has scaled share only; collateral risk fields are zero.
        AccountPositionType::Borrow => {
            get_debt_positions(env, account_id)
                .get(hub_asset)
                .map(|debt| AccountPositionRaw {
                    scaled_amount: debt.scaled_amount,
                    liquidation_threshold: 0,
                    liquidation_bonus: 0,
                    loan_to_value: 0,
                    liquidation_fees: 0,
                })
        }
    }
}

pub fn get_position_list(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
) -> Vec<Address> {
    // Project hub-0 keys back to asset for asset-keyed rule callers.
    let keys: Vec<HubAssetKey> = match position_type {
        AccountPositionType::Deposit => get_supply_positions(env, account_id).keys(),
        AccountPositionType::Borrow => get_debt_positions(env, account_id).keys(),
    };
    let mut assets = Vec::new(env);
    for key in keys.iter() {
        assets.push_back(key.asset);
    }
    assets
}

pub fn get_account_attrs(env: &Env, account_id: u64) -> AccountAttributes {
    try_get_account_meta(env, account_id)
        .map(|meta| AccountAttributes::from(&meta))
        .unwrap_or(AccountAttributes {
            spoke_id: 0,
            mode: PositionMode::Normal,
        })
}

pub mod asset_pool {
    use super::*;

    /// Central pool from instance storage; `_asset` kept for asset-keyed callers.
    pub fn get_asset_pool(env: &Env, _asset: &Address) -> Address {
        crate::storage::get_pool(env)
    }
}

pub mod market_index {
    use super::*;

    pub fn get_market_index(env: &Env, asset: &Address) -> MarketIndex {
        use common::math::fp::Ray;
        let pool = crate::storage::get_pool(env);
        let state = LiquidityPoolClient::new(env, &pool)
            .get_sync_data(&hub0(asset))
            .state;
        MarketIndex {
            borrow_index: Ray::from(state.borrow_index),
            supply_index: Ray::from(state.supply_index),
        }
    }
}

pub mod market_params {
    use super::*;

    pub fn get_market_params(env: &Env, asset: &Address) -> MarketParamsRaw {
        let pool = crate::storage::get_pool(env);
        LiquidityPoolClient::new(env, &pool)
            .get_sync_data(&hub0(asset))
            .params
    }
}

pub mod accounts {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct AccountData {
        pub spoke_id: u32,
    }

    pub fn get_account_data(env: &Env, account_id: u64) -> AccountData {
        let meta = get_account_meta(env, account_id);
        AccountData {
            spoke_id: meta.spoke_id,
        }
    }
}

pub mod positions {
    use super::*;

    pub fn get_scaled_amount(
        env: &Env,
        account_id: u64,
        position_type: AccountPositionType,
        asset: &Address,
    ) -> i128 {
        super::get_position(env, account_id, position_type, asset)
            .map(|position| position.scaled_amount)
            .unwrap_or(0)
    }

    pub fn count_positions(env: &Env, account_id: u64, position_type: AccountPositionType) -> u32 {
        get_position_list(env, account_id, position_type).len()
    }

    pub fn get_position_list(
        env: &Env,
        account_id: u64,
        position_type: AccountPositionType,
    ) -> Vec<Address> {
        super::get_position_list(env, account_id, position_type)
    }
}
