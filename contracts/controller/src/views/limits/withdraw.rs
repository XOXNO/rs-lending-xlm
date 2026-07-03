//! `max_withdraw` preview: full close, then an analytical partial cap settled
//! with a short stroop walk against the exact `partial_ok` mutating-path replica.

use crate::risk;
use crate::storage;
use common::constants::WAD;
use common::math::fp::{Ray, Wad};
use common::math::fp_core;
use common::rates::scaled_to_original;
use common::types::PriceFeed;
use common::types::{Account, AccountPosition, HubAssetKey};
use soroban_sdk::Env;

use crate::context::Cache;

use super::{account_gates_ok, MarketLimitCtx};

/// Stroop walks before falling back to binary search on the residual range.
const PARTIAL_SETTLE_STEPS: u32 = 24;

pub fn max_withdraw(env: &Env, account_id: u64, hub_asset: &HubAssetKey) -> i128 {
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let Some(position_raw) = account.supply_positions.get(hub_asset.clone()) else {
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
        risk::refresh_supply_risk_params_for_asset(
            env,
            &mut cache,
            &account,
            hub_asset,
            &mut position,
        );
        account
            .supply_positions
            .set(hub_asset.clone(), (&position).into());
    }

    let market = MarketLimitCtx::load(&mut cache, hub_asset);
    let pos_scaled = position.scaled_amount;

    // Full close first: any request at or above the half-up position value
    // resolves to it, and the pool pays the floor rounding.
    // dimensional: full_request is max withdraw Token(asset) in asset-native units.
    let full_request =
        scaled_to_original(env, pos_scaled, market.supply_index).to_asset(market.decimals);
    if full_close_ok(env, &mut cache, &account, hub_asset, &market, pos_scaled) {
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
        hub_asset,
        &position,
        &market,
        full_request,
    );
    settle_partial_max(
        env, &mut cache, &account, hub_asset, &market, pos_scaled, candidate, ceiling,
    )
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
    // dimensional: output is Token(asset), floored to the asset's decimals.
    .to_token_floor(decimals)
}

/// Closed-form upper bound on a partial; `partial_ok` settlement tightens it.
fn analytical_partial_cap(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
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
        hub_asset,
        position,
        market,
        full_request,
    ))
}

/// Max partial before LTV or HF gates bind on the withdrawn asset.
fn risk_partial_cap(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
    market: &MarketLimitCtx,
    full_request: i128,
) -> i128 {
    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if totals.total_debt == Wad::ZERO {
        return full_request;
    }
    let debt = totals.total_debt.raw();
    let ltv_slack = totals.ltv_collateral.raw().saturating_sub(debt);
    let hf_slack = totals.weighted_collateral.raw().saturating_sub(debt);
    if ltv_slack == 0 && hf_slack == 0 {
        return 0;
    }

    let feed = cache.cached_price_for(account.spoke_id, hub_asset);
    let ltv_ratio = position.loan_to_value.to_wad(env);
    let hf_ratio = position.liquidation_threshold.to_wad(env);
    // dimensional: slack Wad<USD> / dimensionless risk ratio -> Token(asset) cap.
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
    hub_asset: &HubAssetKey,
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
        if partial_ok(env, cache, account, hub_asset, market, pos_scaled, amount) {
            break;
        }
        if amount == 0 {
            return binary_search_partial(
                env, cache, account, hub_asset, market, pos_scaled, 0, ceiling,
            );
        }
        amount -= 1;
    }
    if !partial_ok(env, cache, account, hub_asset, market, pos_scaled, amount) {
        return binary_search_partial(
            env, cache, account, hub_asset, market, pos_scaled, 0, ceiling,
        );
    }

    let mut steps = 0;
    while amount < ceiling && steps < PARTIAL_SETTLE_STEPS {
        if !partial_ok(
            env,
            cache,
            account,
            hub_asset,
            market,
            pos_scaled,
            amount + 1,
        ) {
            break;
        }
        amount += 1;
        steps += 1;
    }
    if amount < ceiling
        && partial_ok(
            env,
            cache,
            account,
            hub_asset,
            market,
            pos_scaled,
            amount + 1,
        )
    {
        return binary_search_partial(
            env,
            cache,
            account,
            hub_asset,
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
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
    pos_scaled: Ray,
    lo: i128,
    hi: i128,
) -> i128 {
    let mut lo = lo;
    let mut hi = hi;
    while lo < hi {
        let mid = hi - (hi - lo) / 2;
        if partial_ok(env, cache, account, hub_asset, market, pos_scaled, mid) {
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
    hub_asset: &HubAssetKey,
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
    closed.supply_positions.remove(hub_asset.clone());
    account_gates_ok(env, cache, &closed)
}

/// Exact feasibility replica for a partial withdrawal of `amount`.
#[allow(clippy::too_many_arguments)]
fn partial_ok(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    market: &MarketLimitCtx,
    pos_scaled: Ray,
    amount: i128,
) -> bool {
    // resolve_withdrawal replica: shares burnt at the half-up conversion.
    // dimensional: withdrawal amount Token(asset) -> Ray<Share(asset, supply)>.
    let scaled_w = Ray::from_asset(amount, market.decimals).div(env, market.supply_index);
    if scaled_w > pos_scaled {
        return false;
    }
    // dimensional: remaining stays Ray<Share(asset, supply)> for account gates.
    let remaining = pos_scaled - scaled_w;
    let remaining_actual =
        scaled_to_original(env, remaining, market.supply_index).to_asset(market.decimals);
    if remaining_actual == 0 {
        // The pool expands this to a full close.
        return full_close_ok(env, cache, account, hub_asset, market, pos_scaled);
    }

    if !market.pool_state_ok(env, scaled_w, amount) {
        return false;
    }

    let mut adjusted = account.clone();
    let Some(mut pos_raw) = adjusted.supply_positions.get(hub_asset.clone()) else {
        return false;
    };
    pos_raw.scaled_amount = remaining.raw();
    adjusted.supply_positions.set(hub_asset.clone(), pos_raw);
    account_gates_ok(env, cache, &adjusted)
}
