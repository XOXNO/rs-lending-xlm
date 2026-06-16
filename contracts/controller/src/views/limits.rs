//! Integrator preview views: `max_withdraw` and `max_supply`.
//!
//! `max_withdraw` first tries a full close, then caps a partial by closed-form
//! pool solvency headroom and settles with a short stroop walk
//! against the exact `partial_ok` replica of the mutating path. Indexes keep
//! accruing after the read, so callers acting later should leave a margin.

use common::constants::{RAY, WAD};
use common::math::fp::{Ray, Wad};
use common::math::fp_core;
use common::rates::{scaled_to_original, utilization};
use common::validation::cap_is_enabled;
use controller_interface::types::PriceFeed;
use controller_interface::types::{
    Account, AccountPosition, AssetConfig, DebtPositionRaw, MarketStatus,
};
use soroban_sdk::{Address, Env};

use crate::cache::Cache;
use crate::{emode, helpers, storage};

/// Stroop walks before falling back to binary search on the residual range.
const PARTIAL_SETTLE_STEPS: u32 = 24;

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

    /// Largest partial bounded by cash and post-withdraw utilization.
    fn pool_partial_cap(&self, env: &Env, full_request: i128) -> i128 {
        let cap = self.cash.min(full_request);
        if self.max_utilization >= Ray::ONE || self.borrowed == Ray::ZERO {
            return cap;
        }
        let borrowed_orig = scaled_to_original(env, self.borrowed, self.borrow_index);
        if borrowed_orig == Ray::ZERO {
            return cap;
        }
        let min_supplied = ray_div_ceil(env, borrowed_orig, self.max_utilization);
        if self.supplied <= min_supplied {
            return 0;
        }
        let max_scaled_out = self.supplied - min_supplied;
        let util_cap =
            scaled_to_original(env, max_scaled_out, self.supply_index).to_asset(self.decimals);
        cap.min(util_cap)
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

    let ceiling = full_request.saturating_sub(1).max(0);
    if ceiling == 0 {
        return 0;
    }

    let candidate = analytical_partial_cap(
        env,
        &mut cache,
        &account,
        asset,
        &position,
        &market,
        full_request,
    );
    settle_partial_max(
        env, &mut cache, &account, asset, &market, pos_scaled, candidate, ceiling,
    )
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
/// the asset is structurally not borrowable for this account (e-mode category,
/// siloed set, or borrow-position limit). Otherwise mirrors the mutating path's
/// amount-dependent gates — pool liquidity, max utilization, borrow cap, then
/// the account LTV and
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
    let mut hi = market
        .cash
        .min(borrow_cap_headroom(env, &market, &config))
        .max(0);
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
/// pre-pool gates in `validate_borrow`/`validate_asset_borrowable` without
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
/// post-borrow utilization, borrow cap, then the account LTV and health-factor
/// gates with the new debt applied.
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

fn ray_div_ceil(env: &Env, num: Ray, den: Ray) -> Ray {
    Ray::from(fp_core::mul_div_ceil(env, num.raw(), RAY, den.raw()))
}

fn wad_div_ceil(env: &Env, num: Wad, den: Wad) -> Wad {
    Wad::from(fp_core::mul_div_ceil(env, num.raw(), WAD, den.raw()))
}

/// Converts a USD WAD slack into a conservative token upper bound.
fn usd_wad_to_token_cap(env: &Env, usd: Wad, feed: PriceFeed, decimals: u32) -> i128 {
    if usd == Wad::ZERO || feed.price == Wad::ZERO {
        return 0;
    }
    Wad::from(fp_core::mul_div_floor(
        env,
        usd.raw(),
        WAD,
        feed.price.raw(),
    ))
    .to_token_floor(decimals)
}

/// Closed-form upper bound on a partial; `partial_ok` settlement tightens it.
fn analytical_partial_cap(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    position: &AccountPosition,
    market: &MarketLimitCtx,
    full_request: i128,
) -> i128 {
    let cap = market.pool_partial_cap(env, full_request);
    if account.borrow_positions.is_empty() {
        return cap;
    }
    cap.min(risk_partial_cap(
        env,
        cache,
        account,
        asset,
        position,
        market,
        full_request,
    ))
}

/// Max partial before LTV / HF gates bind on the withdrawn asset.
fn risk_partial_cap(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    position: &AccountPosition,
    market: &MarketLimitCtx,
    full_request: i128,
) -> i128 {
    let (_, debt, hf_weighted) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if debt == Wad::ZERO {
        return full_request;
    }
    let ltv_weighted = helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions);
    let ltv_slack = ltv_weighted.raw().saturating_sub(debt.raw());
    let hf_slack = hf_weighted.raw().saturating_sub(debt.raw());
    if ltv_slack == 0 && hf_slack == 0 {
        return 0;
    }

    let feed = cache.cached_price(asset);
    let ltv_ratio = position.loan_to_value.to_wad(env);
    let hf_ratio = position.liquidation_threshold.to_wad(env);
    let ltv_cap = if ltv_slack == 0 || ltv_ratio == Wad::ZERO {
        0
    } else {
        usd_wad_to_token_cap(
            env,
            wad_div_ceil(env, Wad::from(ltv_slack), ltv_ratio),
            feed,
            market.decimals,
        )
    };
    let hf_cap = if hf_slack == 0 || hf_ratio == Wad::ZERO {
        0
    } else {
        usd_wad_to_token_cap(
            env,
            wad_div_ceil(env, Wad::from(hf_slack), hf_ratio),
            feed,
            market.decimals,
        )
    };
    ltv_cap.min(hf_cap).min(full_request)
}

/// Tightens an analytical partial cap against `partial_ok`, then binary-searches
/// any remaining slack.
fn settle_partial_max(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    market: &MarketLimitCtx,
    pos_scaled: Ray,
    candidate: i128,
    ceiling: i128,
) -> i128 {
    if ceiling == 0 {
        return 0;
    }

    let mut amount = candidate.min(ceiling).max(0);
    for _ in 0..PARTIAL_SETTLE_STEPS {
        if partial_ok(env, cache, account, asset, market, pos_scaled, amount) {
            break;
        }
        if amount == 0 {
            return binary_search_partial(
                env, cache, account, asset, market, pos_scaled, 0, ceiling,
            );
        }
        amount -= 1;
    }
    if !partial_ok(env, cache, account, asset, market, pos_scaled, amount) {
        return binary_search_partial(env, cache, account, asset, market, pos_scaled, 0, ceiling);
    }

    let mut steps = 0;
    while amount < ceiling && steps < PARTIAL_SETTLE_STEPS {
        if !partial_ok(env, cache, account, asset, market, pos_scaled, amount + 1) {
            break;
        }
        amount += 1;
        steps += 1;
    }
    if amount < ceiling && partial_ok(env, cache, account, asset, market, pos_scaled, amount + 1) {
        return binary_search_partial(
            env,
            cache,
            account,
            asset,
            market,
            pos_scaled,
            amount + 1,
            ceiling,
        );
    }
    amount
}

fn binary_search_partial(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    market: &MarketLimitCtx,
    pos_scaled: Ray,
    lo: i128,
    hi: i128,
) -> i128 {
    let mut lo = lo;
    let mut hi = hi;
    while lo < hi {
        let mid = hi - (hi - lo) / 2;
        if partial_ok(env, cache, account, asset, market, pos_scaled, mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// Exact replica of a full close: pool guards on the floor payout plus the
/// account gates with the position removed.
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
    account_gates_ok(env, cache, &adjusted)
}

/// Replica of `require_post_pool_risk_gates` LTV/HF legs; HF >= 1 in
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
    if ltv_weighted.raw() < debt.raw() {
        return false;
    }
    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    floor == 0 || ltv_weighted.raw() >= floor
}
