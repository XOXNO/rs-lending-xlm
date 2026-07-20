//! `max_borrow` preview for borrowability gates, caps, and health factor.

use common::math::fp::Ray;
use common::rates::{scaled_to_original, utilization};
use common::types::{Account, DebtPositionRaw, HubAssetKey};
use common::validation::cap_is_enabled;
use soroban_sdk::Env;

use crate::context::Cache;
use crate::{spoke, storage};

use crate::views::limits::{account_gates_ok, MarketLimitCtx};

/// Largest executable borrow amount, or zero when blocked.
pub(crate) fn max_borrow(env: &Env, account_id: u64, hub_asset: &HubAssetKey) -> i128 {
    if stellar_contract_utils::pausable::paused(env) {
        return 0;
    }
    let Some(account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
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

    let mut hi = market
        .cash
        .min(spoke_borrow_cap_headroom(
            env, &mut cache, &account, hub_asset, &market,
        ))
        .max(0);
    if hi <= 0 {
        return 0;
    }

    // Feasibility only tightens as the amount grows; binary-search the largest
    // amount that clears each gate. Iterations are capped so the search stays
    // total (see `BINARY_SEARCH_MAX_STEPS`).
    let mut lo: i128 = 0;
    for _ in 0..crate::views::limits::BINARY_SEARCH_MAX_STEPS {
        if lo >= hi {
            break;
        }
        let mid = hi - (hi - lo) / 2;
        if borrow_ok(env, &mut cache, &account, hub_asset, &market, mid) {
            lo = mid;
        } else {
            hi = mid.saturating_sub(1);
        }
    }
    lo
}

/// Amount-independent borrowability of `asset` for `account`, mirroring most
/// pre-pool gates in `positions::validate_position_entry_gates` without
/// throwing (spoke deprecation, listing, paused/frozen flags, borrow
/// capability, and the position-count limit). Hub activity is not checked
/// here, so a deactivated hub can still show nonzero headroom.
fn account_can_borrow_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> bool {
    // Deprecated spoke or unlisted asset: not borrowable.
    if cache.spoke_config(account.spoke_id).is_deprecated {
        return false;
    }
    let Some(spoke_cfg) = cache.cached_spoke_asset(account.spoke_id, hub_asset) else {
        return false;
    };
    // Paused or frozen listing: zero capacity.
    if spoke_cfg.paused || spoke_cfg.frozen {
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

/// Scaled spoke borrow cap and current usage for the account's spoke.
/// `None` when the spoke does not list the asset or the cap is disabled, i.e.
/// no spoke borrow-cap constraint applies.
fn spoke_borrow_cap_scaled(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
) -> Option<(Ray, Ray)> {
    let spoke_cfg = cache.cached_spoke_asset(account.spoke_id, hub_asset)?;
    if !cap_is_enabled(spoke_cfg.borrow_cap) {
        return None;
    }
    let usage = cache.cached_spoke_usage(account.spoke_id, hub_asset);
    let cap_scaled =
        Ray::from_asset(spoke_cfg.borrow_cap, market.decimals).div_floor(env, market.borrow_index);
    let used_scaled = Ray::from(usage.borrowed_scaled_ray);
    Some((cap_scaled, used_scaled))
}

/// Spoke borrow-cap headroom for the account's spoke; `i128::MAX` when disabled.
fn spoke_borrow_cap_headroom(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
) -> i128 {
    let Some((cap_scaled, used_scaled)) =
        spoke_borrow_cap_scaled(env, cache, account, hub_asset, market)
    else {
        return i128::MAX;
    };
    if used_scaled >= cap_scaled {
        return 0;
    }
    scaled_to_original(env, cap_scaled.checked_sub(env, used_scaled), market.borrow_index)
        .to_asset(market.decimals)
}

/// Exact feasibility replica for borrowing `amount` of `asset`: pool liquidity,
/// post-borrow utilization, spoke borrow cap, then the account LTV and
/// health-factor gates with the new debt applied.
fn borrow_ok(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
    amount: i128,
) -> bool {
    if amount <= 0 {
        return true;
    }
    if amount > market.cash {
        return false;
    }

    let new_scaled = Ray::from_asset(amount, market.decimals).div(env, market.borrow_index);
    let post_borrowed = market.borrowed.checked_add(env, new_scaled);

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

    // Spoke borrow cap on post-borrow scaled usage.
    if let Some((cap_scaled, used_scaled)) =
        spoke_borrow_cap_scaled(env, cache, account, hub_asset, market)
    {
        if used_scaled.checked_add(env, new_scaled) > cap_scaled {
            return false;
        }
    }

    // Account LTV + health-factor gates with the new debt position applied.
    let mut adjusted = account.clone();
    let existing = adjusted
        .borrow_positions
        .get(hub_asset.clone())
        .map_or(Ray::ZERO, |r| Ray::from(r.scaled_amount));
    adjusted.borrow_positions.set(
        hub_asset.clone(),
        DebtPositionRaw {
            scaled_amount: existing.checked_add(env, new_scaled).raw(),
        },
    );
    account_gates_ok(env, cache, &adjusted)
}

#[cfg(test)]
#[path = "../../../tests/views/borrow.rs"]
mod tests;
