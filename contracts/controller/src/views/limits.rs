//! Integrator preview views: `max_withdraw` and `max_supply`.
//!
//! Both mirror the enforcement math the next transaction runs — pool cash,
//! the max-utilization cap, supply caps, the account LTV/HF gates, and the
//! dust floor — at view-simulated indexes. Closed-form candidates are then
//! verified against replicas built from the same fixed-point operations the
//! mutating path uses, walking down until they pass, so a returned amount
//! never overstates what is currently executable. Indexes keep accruing after
//! the read, so callers acting later should leave a margin.

use common::constants::BPS;
use common::math::fp::{Ray, Wad};
use common::rates::{scaled_to_original, utilization};
use common::types::{Account, AccountPosition, AssetConfig, MarketStatus};
use common::validation::cap_is_enabled;
use soroban_sdk::{Address, Env};

use crate::cache::Cache;
use crate::{helpers, storage};

/// Closed-form candidates sit within a few stroops of the true bound, so a
/// handful of decrements always converges; exhausting the budget returns the
/// safe understatement `0`.
const REFINE_STEPS: u32 = 8;

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
    let full_payout = pos_scaled
        .mul_floor(env, market.supply_index)
        .to_asset_floor(market.decimals);
    if market.pool_state_ok(env, pos_scaled, full_payout) {
        let mut closed = account.clone();
        closed.supply_positions.remove(asset.clone());
        if account_gates_ok(env, &mut cache, &closed) {
            return full_request;
        }
    }

    let floor_wad = cache.cached_asset_config(asset).min_collat_floor_usd.raw();
    let mut candidate = partial_bound(
        env,
        &mut cache,
        &account,
        &position,
        &market,
        asset,
        floor_wad,
        full_request,
    );

    for _ in 0..REFINE_STEPS {
        if candidate <= 0 {
            return 0;
        }
        if partial_ok(
            env, &mut cache, &account, asset, &market, pos_scaled, floor_wad, candidate,
        ) {
            return candidate;
        }
        candidate -= 1;
    }
    0
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

    let mut candidate = (cap_ray - current).to_asset_floor(market.decimals);
    for _ in 0..REFINE_STEPS {
        if candidate <= 0 {
            return 0;
        }
        // Replica of the pool's supply-cap gate at the same index.
        let scaled = Ray::from_asset(candidate, market.decimals).div(env, market.supply_index);
        if (market.supplied + scaled).mul(env, market.supply_index) <= cap_ray {
            return candidate;
        }
        candidate -= 1;
    }
    0
}

/// Tightest closed-form upper bound for a partial withdrawal; every term is
/// floor-biased and the caller verifies it against the exact gate replicas.
#[allow(clippy::too_many_arguments)]
fn partial_bound(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    position: &AccountPosition,
    market: &MarketLimitCtx,
    asset: &Address,
    floor_wad: i128,
    full_request: i128,
) -> i128 {
    let mut bound = market.cash.min(full_request.saturating_sub(1));

    if market.borrowed != Ray::ZERO && market.max_utilization < Ray::ONE {
        let supplied_u = scaled_to_original(env, market.supplied, market.supply_index);
        let borrowed_u = scaled_to_original(env, market.borrowed, market.borrow_index);
        let required = borrowed_u.div_floor(env, market.max_utilization);
        let headroom = if supplied_u > required {
            (supplied_u - required).to_asset_floor(market.decimals)
        } else {
            0
        };
        bound = bound.min(headroom);
    }

    if !account.borrow_positions.is_empty() {
        bound = bound.min(risk_headroom(env, cache, account, position, asset, market));
    }

    if floor_wad > 0 {
        // A partial withdrawal must leave the residue at or above the USD
        // floor; reserve one extra unit against price rounding.
        let feed = cache.cached_price(asset);
        let full_floor = position
            .scaled_amount
            .mul_floor(env, market.supply_index)
            .to_asset_floor(market.decimals);
        let min_residue = Wad::from(floor_wad)
            .div(env, feed.price)
            .to_token(market.decimals)
            + 1;
        bound = bound.min(full_floor.saturating_sub(min_residue));
    }

    bound.max(0)
}

/// Largest withdrawable USD value converted to asset units that keeps the
/// LTV gate (`ltv_weighted >= debt`) and HF gate (`weighted >= debt`) intact.
fn risk_headroom(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    position: &AccountPosition,
    asset: &Address,
    market: &MarketLimitCtx,
) -> i128 {
    let debt = helpers::calculate_total_debt_wad(env, cache, &account.borrow_positions);
    let ltv_weighted = helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions);
    let (_, _, hf_weighted) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let excess_ltv = (ltv_weighted.raw() - debt.raw()).max(0);
    let excess_hf = (hf_weighted.raw() - debt.raw()).max(0);

    let by_ltv = removable_value(env, excess_ltv, position.loan_to_value.raw());
    let by_hf = removable_value(env, excess_hf, position.liquidation_threshold.raw());
    let removable_wad = by_ltv.min(by_hf);
    if removable_wad == i128::MAX {
        return i128::MAX;
    }

    let feed = cache.cached_price(asset);
    Wad::from(removable_wad)
        .div_floor(env, feed.price)
        .to_token(market.decimals)
}

/// USD value whose removal consumes exactly `excess` at `weight_bps`; zero
/// weight means the asset never tightens that gate.
fn removable_value(env: &Env, excess_wad: i128, weight_bps: i128) -> i128 {
    if weight_bps == 0 {
        return i128::MAX;
    }
    let scaled = excess_wad.checked_mul(BPS).unwrap_or_else(|| {
        soroban_sdk::panic_with_error!(env, common::errors::GenericError::MathOverflow)
    });
    scaled / weight_bps
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
        let payout = pos_scaled
            .mul_floor(env, market.supply_index)
            .to_asset_floor(market.decimals);
        if !market.pool_state_ok(env, pos_scaled, payout) {
            return false;
        }
        let mut closed = account.clone();
        closed.supply_positions.remove(asset.clone());
        return account_gates_ok(env, cache, &closed);
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

    dust_ok(env, cache, asset, remaining, floor_wad)
}

/// Replica of `require_within_ltv` + `require_healthy_account`; HF >= 1 in
/// floor division is equivalent to `weighted >= debt`.
fn account_gates_ok(env: &Env, cache: &mut Cache, account: &Account) -> bool {
    if account.borrow_positions.is_empty() {
        return true;
    }
    let debt = helpers::calculate_total_debt_wad(env, cache, &account.borrow_positions);
    let ltv_weighted = helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions);
    if ltv_weighted.raw() < debt.raw() {
        return false;
    }
    let (_, total_debt, hf_weighted) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    total_debt == Wad::ZERO || hf_weighted.raw() >= total_debt.raw()
}

/// Replica of the touched-asset dust gate on the residue.
fn dust_ok(env: &Env, cache: &mut Cache, asset: &Address, remaining: Ray, floor_wad: i128) -> bool {
    if remaining == Ray::ZERO || floor_wad == 0 {
        return true;
    }
    let index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);
    let value = helpers::position_value(env, remaining, index.supply_index, feed.price);
    !(value > Wad::ZERO && value.raw() < floor_wad)
}
