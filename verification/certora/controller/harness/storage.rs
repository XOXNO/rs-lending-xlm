use super::*;
use common::types::{
    AccountAttributes, AccountPosition, AssetConfig, EModeAssetConfig, MarketIndex, MarketParams,
    PositionMode, POSITION_TYPE_DEPOSIT,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Env, Map, Vec};

pub fn get_position(
    env: &Env,
    account_id: u64,
    position_type: u32,
    asset: &Address,
) -> Option<AccountPosition> {
    try_get_position(env, account_id, position_type, asset)
}

pub fn get_position_list(env: &Env, account_id: u64, position_type: u32) -> Vec<Address> {
    let map = if position_type == POSITION_TYPE_DEPOSIT {
        get_supply_positions(env, account_id)
    } else {
        get_borrow_positions(env, account_id)
    };
    let mut out = Vec::new(env);
    for asset in map.keys() {
        out.push_back(asset);
    }
    out
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

    pub fn get_asset_pool(env: &Env, asset: &Address) -> Address {
        get_market_config(env, asset).pool_address
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
        let sync = LiquidityPoolClient::new(env, &market.pool_address).get_sync_data();
        let cfg: AssetConfig = market.asset_config;
        CompatAssetConfig {
            loan_to_value_bps: cfg.loan_to_value_bps as i128,
            liquidation_threshold_bps: cfg.liquidation_threshold_bps as i128,
            liquidation_bonus_bps: cfg.liquidation_bonus_bps as i128,
            liquidation_fees_bps: cfg.liquidation_fees_bps as i128,
            is_collateralizable: cfg.is_collateralizable,
            is_borrowable: cfg.is_borrowable,
            has_emode: cfg.has_emode(),
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
        let market = get_market_config(env, asset);
        let state = LiquidityPoolClient::new(env, &market.pool_address)
            .get_sync_data()
            .state;
        MarketIndex {
            borrow_index_ray: state.borrow_index_ray,
            supply_index_ray: state.supply_index_ray,
        }
    }
}

pub mod market_params {
    use super::*;

    pub fn get_market_params(env: &Env, asset: &Address) -> MarketParams {
        let market = get_market_config(env, asset);
        LiquidityPoolClient::new(env, &market.pool_address)
            .get_sync_data()
            .params
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
        position_type: u32,
        asset: &Address,
    ) -> i128 {
        try_get_position(env, account_id, position_type, asset)
            .map(|position| position.scaled_amount_ray)
            .unwrap_or(0)
    }

    pub fn count_positions(env: &Env, account_id: u64, position_type: u32) -> u32 {
        get_position_list(env, account_id, position_type).len()
    }

    pub fn get_position_list(env: &Env, account_id: u64, position_type: u32) -> Vec<Address> {
        super::get_position_list(env, account_id, position_type)
    }
}
