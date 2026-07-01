//! `max_supply` preview: supply-cap headroom across hub and spoke caps.

use common::math::fp::Ray;
use common::rates::scaled_to_original;
use common::types::{Account, AssetConfig, HubAssetKey, SpokeUsageRaw};
use common::validation::cap_is_enabled;
use soroban_sdk::Env;

use crate::context::Cache;
use crate::storage;

use super::MarketLimitCtx;

pub fn max_supply(env: &Env, account_id: u64, hub_asset: &HubAssetKey) -> i128 {
    if stellar_contract_utils::pausable::paused(env) {
        return 0;
    }
    // Inactive: no token-rooted oracle entry.
    if storage::get_asset_oracle(env, &hub_asset.asset).is_none() {
        return 0;
    }
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = Cache::new_view(env);
    // Asset must be listed on the account's spoke; collateralizability is read
    // from that listing.
    let Some(spoke_cfg) = cache.cached_spoke_asset(account.spoke_id, hub_asset) else {
        return 0;
    };
    if !AssetConfig::from(&spoke_cfg).can_supply() {
        return 0;
    }
    let hub_supply_cap = cache.cached_pool_sync_data(hub_asset).params.supply_cap;
    let market = MarketLimitCtx::load(&mut cache, hub_asset);
    let hub_headroom = hub_supply_cap_headroom(env, &market, hub_supply_cap);
    let spoke_headroom = spoke_supply_cap_headroom(env, &mut cache, &account, hub_asset, &market);
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

/// Spoke supply-cap headroom for the account's spoke; `i128::MAX` when disabled.
fn spoke_supply_cap_headroom(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
) -> i128 {
    let Some(spoke_cfg) = cache.cached_spoke_asset(account.spoke_id, hub_asset) else {
        return i128::MAX;
    };
    if !cap_is_enabled(spoke_cfg.supply_cap) {
        return i128::MAX;
    }
    let usage = cache
        .cached_spoke_usage(account.spoke_id, hub_asset)
        .unwrap_or(SpokeUsageRaw {
            supplied_scaled_ray: 0,
            borrowed_scaled_ray: 0,
        });
    // dimensional: spoke supply cap and usage compare as Ray<Share(asset, supply)>.
    let cap_scaled =
        Ray::from_asset(spoke_cfg.supply_cap, market.decimals).div_floor(env, market.supply_index);
    let used_scaled = Ray::from(usage.supplied_scaled_ray);
    if used_scaled >= cap_scaled {
        return 0;
    }
    scaled_to_original(env, cap_scaled - used_scaled, market.supply_index).to_asset(market.decimals)
}
