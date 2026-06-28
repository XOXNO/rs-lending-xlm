//! `max_supply` preview: supply-cap headroom across hub and e-mode spoke caps.

use common::math::fp::Ray;
use common::rates::scaled_to_original;
use common::validation::cap_is_enabled;
use controller_interface::types::{
    Account, AssetConfig, EModeSpokeUsageRaw, HubAssetKey, MarketStatus,
};
use soroban_sdk::{Address, Env};

use crate::cache::Cache;
use crate::storage;

use super::MarketLimitCtx;

pub fn max_supply(env: &Env, account_id: u64, asset: &Address) -> i128 {
    if stellar_contract_utils::pausable::paused(env) {
        return 0;
    }
    let market_config = storage::get_market_config(env, asset);
    if market_config.status != MarketStatus::Active {
        return 0;
    }
    let config: AssetConfig = (&market_config.asset_config).into();
    if !config.can_supply() {
        return 0;
    }
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = Cache::new_view(env);
    if account.e_mode_category_id > 0
        && (!market_config
            .asset_config
            .e_mode_categories
            .contains(account.e_mode_category_id)
            || cache
                .cached_emode_asset(account.e_mode_category_id, asset)
                .is_none())
    {
        return 0;
    }
    let hub_supply_cap = cache.cached_pool_sync_data(asset).params.supply_cap;
    let market = MarketLimitCtx::load(&mut cache, asset);
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let hub_headroom = hub_supply_cap_headroom(env, &market, hub_supply_cap);
    let spoke_headroom = spoke_supply_cap_headroom(env, &mut cache, &account, &hub_asset, &market);
    // dimensional: max_supply returns additional Token(asset) in asset-native units.
    hub_headroom.min(spoke_headroom)
}

/// Hub supply-cap headroom in asset units; `i128::MAX` when the cap is disabled.
fn hub_supply_cap_headroom(env: &Env, market: &MarketLimitCtx, supply_cap: i128) -> i128 {
    if !cap_is_enabled(supply_cap) {
        return i128::MAX;
    }
    // dimensional: supply_cap is Token(asset); cap_ray/current are Ray<Token(asset)>.
    let cap_ray = Ray::from_asset(supply_cap, market.decimals);
    let current = market.supplied.mul(env, market.supply_index);
    if current >= cap_ray {
        return 0;
    }

    // dimensional: candidate Token(asset) round-trips through Ray<Share(asset, supply)>.
    let mut candidate = (cap_ray - current).to_asset_floor(market.decimals);
    for _ in 0..8 {
        if candidate <= 0 {
            return 0;
        }
        let scaled = Ray::from_asset(candidate, market.decimals).div(env, market.supply_index);
        if (market.supplied + scaled).mul(env, market.supply_index) <= cap_ray {
            return candidate;
        }
        candidate -= 1;
    }
    0
}

/// Spoke supply-cap headroom for an e-mode account; `i128::MAX` when disabled.
fn spoke_supply_cap_headroom(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
) -> i128 {
    if account.e_mode_category_id == 0 {
        return i128::MAX;
    }
    let Some(emode_cfg) = cache.cached_emode_asset(account.e_mode_category_id, &hub_asset.asset)
    else {
        return i128::MAX;
    };
    if !cap_is_enabled(emode_cfg.supply_cap) {
        return i128::MAX;
    }
    let usage = cache
        .cached_emode_spoke_usage(account.e_mode_category_id, hub_asset)
        .unwrap_or(EModeSpokeUsageRaw {
            supplied_scaled_ray: 0,
            borrowed_scaled_ray: 0,
        });
    // dimensional: spoke supply cap and usage compare as Ray<Share(asset, supply)>.
    let cap_scaled =
        Ray::from_asset(emode_cfg.supply_cap, market.decimals).div_floor(env, market.supply_index);
    let used_scaled = Ray::from(usage.supplied_scaled_ray);
    if used_scaled >= cap_scaled {
        return 0;
    }
    scaled_to_original(env, cap_scaled - used_scaled, market.supply_index).to_asset(market.decimals)
}
