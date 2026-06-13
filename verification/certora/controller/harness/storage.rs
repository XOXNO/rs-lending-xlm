//! Certora storage accessors for controller rules.
//!
//! Provides rule-friendly reads for account state, market config, positions,
//! and e-mode data under the `certora` feature.

use super::*;
use crate::types::{
    AccountAttributes, AccountPositionRaw, AccountPositionType, EModeAssetConfig, MarketIndex,
    MarketParamsRaw, PositionMode,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Env, Map, Vec};

pub fn get_position(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
    asset: &Address,
) -> Option<AccountPositionRaw> {
    match position_type {
        AccountPositionType::Deposit => get_supply_positions(env, account_id).get(asset.clone()),
        // Debt positions carry only the scaled share; risk params stay
        // supply-side, so the collateral fields read as zero for debt.
        AccountPositionType::Borrow => {
            get_debt_positions(env, account_id)
                .get(asset.clone())
                .map(|debt| AccountPositionRaw {
                    scaled_amount_ray: debt.scaled_amount_ray,
                    liquidation_threshold_bps: 0,
                    liquidation_bonus_bps: 0,
                    loan_to_value_bps: 0,
                })
        }
    }
}

pub fn get_position_list(
    env: &Env,
    account_id: u64,
    position_type: AccountPositionType,
) -> Vec<Address> {
    match position_type {
        AccountPositionType::Deposit => get_supply_positions(env, account_id).keys(),
        AccountPositionType::Borrow => get_debt_positions(env, account_id).keys(),
    }
}

pub fn get_account_attrs(env: &Env, account_id: u64) -> AccountAttributes {
    try_get_account_meta(env, account_id)
        .map(|meta| AccountAttributes::from(&meta))
        .unwrap_or(AccountAttributes {
            is_isolated: false,
            e_mode_category_id: 0,
            mode: PositionMode::Normal,
        })
}

pub fn get_asset_config(env: &Env, asset: &Address) -> asset_config::CompatAssetConfig {
    asset_config::get_asset_config(env, asset)
}

pub fn get_asset_emodes(env: &Env, asset: &Address) -> Vec<u32> {
    get_market_config(env, asset).asset_config.e_mode_categories
}

pub fn get_emode_assets(env: &Env, category_id: u32) -> Map<Address, EModeAssetConfig> {
    try_get_emode_category(env, category_id)
        .map(|category| category.assets)
        .unwrap_or_else(|| Map::new(env))
}

pub mod asset_pool {
    use super::*;

    pub fn get_asset_pool(env: &Env, _asset: &Address) -> Address {
        // Central-pool architecture: every market routes through one pool.
        super::get_pool(env)
    }
}

pub mod asset_config {
    use super::*;

    #[allow(dead_code)]
    #[derive(Clone, Debug)]
    pub struct CompatAssetConfig {
        pub loan_to_value_bps: i128,
        pub liquidation_threshold_bps: i128,
        pub liquidation_bonus_bps: i128,
        pub liquidation_fees_bps: i128,
        pub is_collateralizable: bool,
        pub is_borrowable: bool,
        pub has_emode: bool,
        pub is_isolated_asset: bool,
        pub is_siloed_borrowing: bool,
        pub is_flashloanable: bool,
        pub isolation_borrow_enabled: bool,
        pub isolation_debt_ceiling_usd_wad: i128,
        pub flashloan_fee_bps: i128,
        pub borrow_cap: i128,
        pub supply_cap: i128,
        pub reserve_factor_bps: i128,
    }

    pub fn get_asset_config(env: &Env, asset: &Address) -> CompatAssetConfig {
        let market = get_market_config(env, asset);
        let pool = super::get_pool(env);
        let sync = LiquidityPoolClient::new(env, &pool).get_sync_data(asset);
        let cfg = market.asset_config;
        CompatAssetConfig {
            loan_to_value_bps: cfg.loan_to_value_bps as i128,
            liquidation_threshold_bps: cfg.liquidation_threshold_bps as i128,
            liquidation_bonus_bps: cfg.liquidation_bonus_bps as i128,
            liquidation_fees_bps: cfg.liquidation_fees_bps as i128,
            is_collateralizable: cfg.is_collateralizable,
            is_borrowable: cfg.is_borrowable,
            has_emode: !cfg.e_mode_categories.is_empty(),
            is_isolated_asset: cfg.is_isolated_asset,
            is_siloed_borrowing: cfg.is_siloed_borrowing,
            is_flashloanable: cfg.is_flashloanable,
            isolation_borrow_enabled: cfg.isolation_borrow_enabled,
            isolation_debt_ceiling_usd_wad: cfg.isolation_debt_ceiling_usd_wad,
            flashloan_fee_bps: cfg.flashloan_fee_bps as i128,
            borrow_cap: cfg.borrow_cap,
            supply_cap: cfg.supply_cap,
            reserve_factor_bps: sync.params.reserve_factor_bps as i128,
        }
    }
}

pub mod market_index {
    use super::*;

    pub fn get_market_index(env: &Env, asset: &Address) -> MarketIndex {
        use common::math::fp::Ray;
        let pool = super::get_pool(env);
        let state = LiquidityPoolClient::new(env, &pool).get_sync_data(asset).state;
        MarketIndex {
            borrow_index: Ray::from(state.borrow_index_ray),
            supply_index: Ray::from(state.supply_index_ray),
        }
    }
}

pub mod market_params {
    use super::*;

    pub fn get_market_params(env: &Env, asset: &Address) -> MarketParamsRaw {
        let pool = super::get_pool(env);
        LiquidityPoolClient::new(env, &pool).get_sync_data(asset).params
    }
}

pub mod isolation {
    use super::*;

    #[allow(dead_code)]
    pub fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
        super::get_isolated_debt(env, asset)
    }
}

pub mod accounts {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct AccountData {
        pub is_isolated: bool,
        pub e_mode_category: u32,
        pub isolated_asset: Address,
    }

    pub fn get_account_data(env: &Env, account_id: u64) -> AccountData {
        let meta = get_account_meta(env, account_id);
        let isolated_asset = meta.isolated_asset.unwrap_or_else(|| meta.owner.clone());
        AccountData {
            is_isolated: meta.is_isolated,
            e_mode_category: meta.e_mode_category_id,
            isolated_asset,
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
            .map(|position| position.scaled_amount_ray)
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
