//! `max_supply` preview: spoke supply-cap headroom.

use common::math::fp::Ray;
use common::rates::scaled_to_original;
use common::types::{Account, AssetConfig, HubAssetKey, SpokeUsageRaw};
use common::validation::cap_is_enabled;
use soroban_sdk::Env;

use crate::context::Cache;
use crate::storage;

use crate::views::limits::MarketLimitCtx;

/// Largest suppliable amount of `hub_asset`; `0` while paused, on a
/// deprecated spoke or paused/frozen listing, or a non-suppliable asset.
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
    // Mutating supplies pass `require_listed_active_config`: a deprecated
    // spoke accepts no deposits, so preview zero headroom.
    if cache.spoke_config(account.spoke_id).is_deprecated {
        return 0;
    }
    // Asset must be listed on the account's spoke; collateralizability is read
    // from that listing.
    let Some(spoke_cfg) = cache.cached_spoke_asset(account.spoke_id, hub_asset) else {
        return 0;
    };
    // Mirrors `enforce_spoke_asset_flags`: paused or frozen listings reject
    // every supply, so the preview reports no capacity.
    if spoke_cfg.paused || spoke_cfg.frozen {
        return 0;
    }
    if !AssetConfig::from(&spoke_cfg).can_supply() {
        return 0;
    }
    let market = MarketLimitCtx::load(&mut cache, hub_asset);
    spoke_supply_cap_headroom(env, &mut cache, &account, hub_asset, &market)
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
    let cap_scaled =
        Ray::from_asset(spoke_cfg.supply_cap, market.decimals).div_floor(env, market.supply_index);
    let used_scaled = Ray::from(usage.supplied_scaled_ray);
    if used_scaled >= cap_scaled {
        return 0;
    }
    scaled_to_original(env, cap_scaled - used_scaled, market.supply_index).to_asset(market.decimals)
}
