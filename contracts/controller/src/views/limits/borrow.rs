//! `max_borrow` preview for borrowability gates, caps, and health factor.

use common::math::fp::Ray;
use common::rates::{scaled_to_original, utilization};
use common::validation::cap_is_enabled;
use controller_interface::types::{Account, DebtPositionRaw, HubAssetKey, SpokeUsageRaw};
use soroban_sdk::Env;

use crate::cache::Cache;
use crate::{spoke, storage};

use super::{account_gates_ok, MarketLimitCtx};

/// Largest executable `borrow` amount for `hub_asset` and `account_id`.
///
/// Returns `0` when paused, inactive, non-borrowable, structurally blocked,
/// or limited by pool liquidity, utilization, caps, LTV, or health factor.
pub fn max_borrow(env: &Env, account_id: u64, hub_asset: &HubAssetKey) -> i128 {
    if stellar_contract_utils::pausable::paused(env) {
        return 0;
    }
    let Some(account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    // Inactive: no token-rooted oracle entry.
    if storage::get_asset_oracle(env, &hub_asset.asset).is_none() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    if !account_can_borrow_asset(env, &mut cache, &account, hub_asset) {
        return 0;
    }

    let market = MarketLimitCtx::load(&mut cache, hub_asset);
    // No supplied liquidity means no borrowable cash and undefined utilization.
    if market.supplied == Ray::ZERO {
        return 0;
    }

    let hub_borrow_cap = cache.cached_pool_sync_data(hub_asset).params.borrow_cap;
    // dimensional: all headrooms in this minimum are Token(asset).
    let mut hi = market
        .cash
        .min(hub_borrow_cap_headroom(env, &market, hub_borrow_cap))
        .min(spoke_borrow_cap_headroom(
            env, &mut cache, &account, hub_asset, &market,
        ))
        .max(0);
    if hi <= 0 {
        return 0;
    }

    // Feasibility only tightens as the amount grows; binary-search the largest
    // amount that clears each gate.
    let mut lo: i128 = 0;
    while lo < hi {
        let mid = hi - (hi - lo) / 2;
        if borrow_ok(
            env,
            &mut cache,
            &account,
            hub_asset,
            &market,
            hub_borrow_cap,
            mid,
        ) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    // dimensional: max_borrow returns additional Token(asset) in asset-native units.
    lo
}

/// Amount-independent borrowability of `asset` for `account`, mirroring the
/// pre-pool gates in `validate_borrow`/`validate_asset_borrowable` without
/// throwing.
fn account_can_borrow_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> bool {
    // The account's spoke must be active and must list the asset (the spoke
    // listing is the membership signal).
    let active = matches!(cache.cached_spoke(account.spoke_id), Some(s) if !s.is_deprecated);
    if !active || cache.cached_spoke_asset(account.spoke_id, hub_asset).is_none() {
        return false;
    }

    let config = spoke::effective_asset_config(cache, account.spoke_id, hub_asset);
    if !config.can_borrow() {
        return false;
    }

    // Borrow-position limit: a new borrowed asset needs a free slot.
    if !account.borrow_positions.contains_key(hub_asset.clone())
        && account.borrow_positions.len() >= storage::get_position_limits(env).max_borrow_positions
    {
        return false;
    }

    true
}

/// Hub borrow-cap headroom in asset units; `i128::MAX` when the cap is disabled.
fn hub_borrow_cap_headroom(env: &Env, market: &MarketLimitCtx, borrow_cap: i128) -> i128 {
    if !cap_is_enabled(borrow_cap) {
        return i128::MAX;
    }
    // dimensional: borrowed Ray<Share(asset, debt)> converts to Token(asset).
    let current =
        scaled_to_original(env, market.borrowed, market.borrow_index).to_asset(market.decimals);
    (borrow_cap - current).max(0)
}

/// Spoke borrow-cap headroom for the account's spoke; `i128::MAX` when disabled.
fn spoke_borrow_cap_headroom(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
) -> i128 {
    let Some(spoke_cfg) = cache.cached_spoke_asset(account.spoke_id, hub_asset) else {
        return i128::MAX;
    };
    if !cap_is_enabled(spoke_cfg.borrow_cap) {
        return i128::MAX;
    }
    let usage = cache
        .cached_spoke_usage(account.spoke_id, hub_asset)
        .unwrap_or(SpokeUsageRaw {
            supplied_scaled_ray: 0,
            borrowed_scaled_ray: 0,
        });
    // dimensional: spoke borrow cap and usage compare as Ray<Share(asset, debt)>.
    let cap_scaled =
        Ray::from_asset(spoke_cfg.borrow_cap, market.decimals).div_floor(env, market.borrow_index);
    let used_scaled = Ray::from(usage.borrowed_scaled_ray);
    if used_scaled >= cap_scaled {
        return 0;
    }
    scaled_to_original(env, cap_scaled - used_scaled, market.borrow_index).to_asset(market.decimals)
}

/// Exact feasibility replica for borrowing `amount` of `asset`: pool liquidity,
/// post-borrow utilization, borrow cap, then the account LTV and health-factor
/// gates with the new debt applied.
#[allow(clippy::too_many_arguments)]
fn borrow_ok(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
    hub_borrow_cap: i128,
    amount: i128,
) -> bool {
    if amount <= 0 {
        return true;
    }
    if amount > market.cash {
        return false;
    }

    // dimensional: borrow amount Token(asset) -> Ray<Share(asset, debt)> at borrow index.
    let new_scaled = Ray::from_asset(amount, market.decimals).div(env, market.borrow_index);
    let post_borrowed = market.borrowed + new_scaled;

    // Pool max-utilization gate (skipped when utilization is uncapped).
    if market.max_utilization < Ray::ONE {
        let util = utilization(
            env,
            scaled_to_original(env, post_borrowed, market.borrow_index),
            scaled_to_original(env, market.supplied, market.supply_index),
        );
        if util > market.max_utilization {
            return false;
        }
    }

    // Hub borrow cap on post-borrow pool debt.
    if cap_is_enabled(hub_borrow_cap) {
        // dimensional: post_borrowed Ray<Share(asset, debt)> converts to Token(asset).
        let post_actual =
            scaled_to_original(env, post_borrowed, market.borrow_index).to_asset(market.decimals);
        if post_actual > hub_borrow_cap {
            return false;
        }
    }

    // Spoke borrow cap on post-borrow scaled usage.
    if let Some(spoke_cfg) = cache.cached_spoke_asset(account.spoke_id, hub_asset) {
        if cap_is_enabled(spoke_cfg.borrow_cap) {
            let usage = cache
                .cached_spoke_usage(account.spoke_id, hub_asset)
                .unwrap_or(SpokeUsageRaw {
                    supplied_scaled_ray: 0,
                    borrowed_scaled_ray: 0,
                });
            let cap_scaled = Ray::from_asset(spoke_cfg.borrow_cap, market.decimals)
                .div_floor(env, market.borrow_index);
            let next_scaled = Ray::from(usage.borrowed_scaled_ray) + new_scaled;
            if next_scaled > cap_scaled {
                return false;
            }
        }
    }

    // Account LTV + health-factor gates with the new debt position applied.
    let mut adjusted = account.clone();
    let existing = adjusted
        .borrow_positions
        .get(hub_asset.clone())
        .map(|r| Ray::from(r.scaled_amount))
        .unwrap_or(Ray::ZERO);
    adjusted.borrow_positions.set(
        hub_asset.clone(),
        DebtPositionRaw {
            scaled_amount: (existing + new_scaled).raw(),
        },
    );
    account_gates_ok(env, cache, &adjusted)
}
