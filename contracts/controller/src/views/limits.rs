//! Integrator preview views: `max_withdraw` and `max_supply`.
//!
//! `max_withdraw` evaluates feasibility with replicas of the exact
//! fixed-point gates the mutating path runs (pool cash, max-utilization,
//! solvency, the account LTV/HF gates, the dust floor) at view-simulated
//! indexes, then binary-searches the largest passing amount — feasibility is
//! monotone in the amount, so the result never overstates what the next
//! transaction allows. Indexes keep accruing after the read, so callers
//! acting later should leave a margin.

use common::math::fp::{Ray, Wad};
use common::rates::{scaled_to_original, utilization};
use common::types::{Account, AccountPosition, AssetConfig, DebtPositionRaw, MarketStatus};
use common::validation::cap_is_enabled;
use soroban_sdk::{Address, Env};

use crate::cache::Cache;
use crate::{emode, helpers, storage};

/// Pool-side market state at view-simulated indexes.
struct MarketLimitCtx {
    supplied: Ray,
    borrowed: Ray,
    cash: i128,
    max_utilization: Ray,
    supply_index: Ray,
    decimals: u32,
    borrow_index: Ray,
}

impl MarketLimitCtx {
    fn load(cache: &mut Cache, asset: &Address) -> Self {
        let index = cache.cached_market_index(asset);
        let sync = cache.cached_pool_sync_data(asset);
        Self {
            supplied: Ray::from(sync.state.supplied_ray),
            borrowed: Ray::from(sync.state.borrowed_ray),
            cash: sync.state.cash,
            max_utilization: Ray::from(sync.params.max_utilization_ray),
            supply_index: index.supply_index,
            decimals: sync.params.asset_decimals,
            borrow_index: index.borrow_index,
        }
    }

    /// Mirrors the pool's post-withdraw reserve, utilization, and solvency
    /// guards for an outflow of `transfer_out` units burning `scaled_out`.
    fn pool_state_ok(&self, env: &Env, scaled_out: Ray, transfer_out: i128) -> bool {
        if transfer_out > self.cash || scaled_out > self.supplied {
            return false;
        }
        let post_supplied = self.supplied - scaled_out;
        if post_supplied == Ray::ZERO {
            return self.borrowed == Ray::ZERO;
        }
        if self.max_utilization >= Ray::ONE {
            return true;
        }
        let util = utilization(
            env,
            scaled_to_original(env, self.borrowed, self.borrow_index),
            scaled_to_original(env, post_supplied, self.supply_index),
        );
        util <= self.max_utilization
    }
}

pub fn max_withdraw(env: &Env, account_id: u64, asset: &Address) -> i128 {
    if stellar_contract_utils::pausable::paused(env) {
        return 0;
    }
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let Some(position_raw) = account.supply_positions.get(asset.clone()) else {
        return 0;
    };
    let mut position: AccountPosition = (&position_raw).into();
    if position.scaled_amount == Ray::ZERO {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    // The mutating path refreshes the withdrawn asset's risk params before
    // its LTV/HF gates; mirror that on the in-memory account.
    if !account.borrow_positions.is_empty() {
        helpers::refresh_supply_risk_params_for_asset(
            env,
            &mut cache,
            &account,
            asset,
            &mut position,
        );
        account
            .supply_positions
            .set(asset.clone(), (&position).into());
    }

    let market = MarketLimitCtx::load(&mut cache, asset);
    let pos_scaled = position.scaled_amount;

    // Full close first: any request at or above the half-up position value
    // resolves to it, and the pool pays the floor rounding.
    let full_request =
        scaled_to_original(env, pos_scaled, market.supply_index).to_asset(market.decimals);
    if full_close_ok(env, &mut cache, &account, asset, &market, pos_scaled) {
        return full_request;
    }

    let floor_wad = cache.cached_asset_config(asset).min_collat_floor_usd.raw();

    // Feasibility is monotone below the full request: every gate only
    // tightens as the amount grows. Binary-search the largest passing
    // partial.
    let mut lo: i128 = 0;
    let mut hi = market.cash.min(full_request.saturating_sub(1)).max(0);
    while lo < hi {
        let mid = hi - (hi - lo) / 2;
        if partial_ok(
            env, &mut cache, &account, asset, &market, pos_scaled, floor_wad, mid,
        ) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

pub fn max_supply(env: &Env, asset: &Address) -> i128 {
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
    if !cap_is_enabled(config.supply_cap) {
        return i128::MAX;
    }

    let mut cache = Cache::new_view(env);
    let market = MarketLimitCtx::load(&mut cache, asset);
    let cap_ray = Ray::from_asset(config.supply_cap, market.decimals);
    let current = market.supplied.mul(env, market.supply_index);
    if current >= cap_ray {
        return 0;
    }

    // The floor-converted headroom sits within a few stroops of the true
    // bound; walk down against the pool's exact cap gate at the same index.
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

/// Largest currently executable `borrow` amount of `asset` for `account_id`.
///
/// Returns `0` while paused, on an inactive or non-borrowable market, or when
/// the asset is structurally not borrowable for this account (isolation,
/// e-mode category, siloed set, or borrow-position limit). Otherwise mirrors
/// the mutating path's amount-dependent gates — pool liquidity, max
/// utilization, borrow cap, isolation debt ceiling, then the account LTV and
/// health-factor gates — and binary-searches the largest passing amount.
/// Feasibility is monotone in the amount, so the result never overstates what
/// the next transaction allows; indexes keep accruing after the read, so
/// callers acting later should leave a margin.
pub fn max_borrow(env: &Env, account_id: u64, asset: &Address) -> i128 {
    if stellar_contract_utils::pausable::paused(env) {
        return 0;
    }
    let Some(account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    if storage::get_market_config(env, asset).status != MarketStatus::Active {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    if !account_can_borrow_asset(env, &mut cache, &account, asset) {
        return 0;
    }

    let market = MarketLimitCtx::load(&mut cache, asset);
    // No supplied liquidity means no borrowable cash and an undefined
    // utilization; nothing can be borrowed.
    if market.supplied == Ray::ZERO {
        return 0;
    }

    let config = cache.cached_asset_config(asset);
    let mut hi = market.cash.min(borrow_cap_headroom(env, &market, &config)).max(0);
    if hi <= 0 {
        return 0;
    }

    // Feasibility only tightens as the amount grows; binary-search the largest
    // amount that clears every gate.
    let mut lo: i128 = 0;
    while lo < hi {
        let mid = hi - (hi - lo) / 2;
        if borrow_ok(env, &mut cache, &account, asset, &market, &config, mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// Amount-independent borrowability of `asset` for `account`, mirroring the
/// pre-pool gates in `prepare_borrow_plan`/`validate_asset_borrowable` without
/// throwing.
fn account_can_borrow_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
) -> bool {
    let category = emode::e_mode_category(env, account.e_mode_category_id);
    if let Some(cat) = &category {
        // A deprecated category reverts the borrow in the mutating path.
        if cat.is_deprecated {
            return false;
        }
    }

    let config = emode::effective_asset_config(env, account, asset, cache, &category);
    if !config.can_borrow() {
        return false;
    }
    if account.is_isolated && !config.can_borrow_in_isolation() {
        return false;
    }
    if account.e_mode_category_id > 0 {
        let market = cache.cached_market_config(asset);
        if !market
            .asset_config
            .e_mode_categories
            .contains(account.e_mode_category_id)
            || cache
                .cached_emode_asset(account.e_mode_category_id, asset)
                .is_none()
        {
            return false;
        }
    }

    // Borrow-position limit: an asset not already borrowed needs a free slot.
    if !account.borrow_positions.contains_key(asset.clone())
        && account.borrow_positions.len() >= storage::get_position_limits(env).max_borrow_positions
    {
        return false;
    }

    siloed_borrow_ok(cache, account, asset)
}

/// Replica of `validate_siloed_borrow_set`: a siloed asset must be the
/// account's only borrow across the union of existing debt and this asset.
fn siloed_borrow_ok(cache: &mut Cache, account: &Account, asset: &Address) -> bool {
    let mut distinct = account.borrow_positions.len();
    if !account.borrow_positions.contains_key(asset.clone()) {
        distinct += 1;
    }
    if distinct <= 1 {
        return true;
    }
    for existing in account.borrow_positions.keys() {
        if cache.cached_asset_config(&existing).is_siloed_borrowing {
            return false;
        }
    }
    !cache.cached_asset_config(asset).is_siloed_borrowing
}

/// Borrow-cap headroom in asset units; `i128::MAX` when the cap is disabled.
fn borrow_cap_headroom(env: &Env, market: &MarketLimitCtx, config: &AssetConfig) -> i128 {
    if !cap_is_enabled(config.borrow_cap) {
        return i128::MAX;
    }
    let current =
        scaled_to_original(env, market.borrowed, market.borrow_index).to_asset(market.decimals);
    (config.borrow_cap - current).max(0)
}

/// Exact feasibility replica for borrowing `amount` of `asset`: pool liquidity,
/// post-borrow utilization, borrow cap, isolation ceiling, then the account
/// LTV and health-factor gates with the new debt applied.
#[allow(clippy::too_many_arguments)]
fn borrow_ok(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    market: &MarketLimitCtx,
    config: &AssetConfig,
    amount: i128,
) -> bool {
    if amount <= 0 {
        return true;
    }
    if amount > market.cash {
        return false;
    }

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

    // Borrow cap on post-borrow debt.
    if cap_is_enabled(config.borrow_cap) {
        let post_actual =
            scaled_to_original(env, post_borrowed, market.borrow_index).to_asset(market.decimals);
        if post_actual > config.borrow_cap {
            return false;
        }
    }

    // Isolation debt ceiling lives on the isolated collateral's config and is
    // tracked in USD WAD at the borrow price.
    if account.is_isolated {
        if let Some(isolated) = account.try_isolated_token() {
            let feed = cache.cached_price(asset);
            let added = feed.usd_value_wad(env, amount).raw();
            let current = storage::get_isolated_debt(env, &isolated);
            let ceiling = cache
                .cached_asset_config(&isolated)
                .isolation_debt_ceiling_usd
                .raw();
            if current.saturating_add(added) > ceiling {
                return false;
            }
        }
    }

    // Account LTV + health-factor gates with the new debt position applied.
    let mut adjusted = account.clone();
    let existing = adjusted
        .borrow_positions
        .get(asset.clone())
        .map(|r| Ray::from(r.scaled_amount_ray))
        .unwrap_or(Ray::ZERO);
    adjusted.borrow_positions.set(
        asset.clone(),
        DebtPositionRaw {
            scaled_amount_ray: (existing + new_scaled).raw(),
        },
    );
    account_gates_ok(env, cache, &adjusted)
}

/// Exact replica of a full close: pool guards on the floor payout plus the
/// account gates with the position removed (dust never applies).
fn full_close_ok(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    market: &MarketLimitCtx,
    pos_scaled: Ray,
) -> bool {
    let payout = pos_scaled
        .mul_floor(env, market.supply_index)
        .to_asset_floor(market.decimals);
    if !market.pool_state_ok(env, pos_scaled, payout) {
        return false;
    }
    let mut closed = account.clone();
    closed.supply_positions.remove(asset.clone());
    account_gates_ok(env, cache, &closed)
}

/// Exact feasibility replica for a partial withdrawal of `amount`.
#[allow(clippy::too_many_arguments)]
fn partial_ok(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    market: &MarketLimitCtx,
    pos_scaled: Ray,
    floor_wad: i128,
    amount: i128,
) -> bool {
    // resolve_withdrawal replica: shares burnt at the half-up conversion.
    let scaled_w = Ray::from_asset(amount, market.decimals).div(env, market.supply_index);
    if scaled_w > pos_scaled {
        return false;
    }
    let remaining = pos_scaled - scaled_w;
    let remaining_actual =
        scaled_to_original(env, remaining, market.supply_index).to_asset(market.decimals);
    if remaining_actual == 0 {
        // The pool expands this to a full close.
        return full_close_ok(env, cache, account, asset, market, pos_scaled);
    }

    if !market.pool_state_ok(env, scaled_w, amount) {
        return false;
    }

    let mut adjusted = account.clone();
    let Some(mut pos_raw) = adjusted.supply_positions.get(asset.clone()) else {
        return false;
    };
    pos_raw.scaled_amount_ray = remaining.raw();
    adjusted.supply_positions.set(asset.clone(), pos_raw);
    if !account_gates_ok(env, cache, &adjusted) {
        return false;
    }

    // Replica of the touched-asset dust gate on the residue.
    if floor_wad == 0 {
        return true;
    }
    let feed = cache.cached_price(asset);
    let value = helpers::position_value(env, remaining, market.supply_index, feed.price);
    !(value > Wad::ZERO && value.raw() < floor_wad)
}

/// Replica of `require_within_ltv` + `require_healthy_account`; HF >= 1 in
/// floor division is equivalent to `weighted >= debt`.
fn account_gates_ok(env: &Env, cache: &mut Cache, account: &Account) -> bool {
    if account.borrow_positions.is_empty() {
        return true;
    }
    let (_, debt, hf_weighted) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if debt == Wad::ZERO {
        return true;
    }
    if hf_weighted.raw() < debt.raw() {
        return false;
    }
    let ltv_weighted = helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions);
    ltv_weighted.raw() >= debt.raw()
}
