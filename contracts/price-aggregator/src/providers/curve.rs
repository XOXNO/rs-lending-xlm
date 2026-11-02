//! Conservative plain-pool valuation using Curve's own invariant/share view.

use common::constants::WAD;
use common::errors::OracleError;
use common::math::fp_core::mul_div_floor_saturating;
use common::oracle::providers::curve::CurvePoolClient;
use common::types::{CurveStableLpSource, PriceKey};
use soroban_sdk::{token, Env, Executable};

use crate::{engine, observation::OracleObservation, session::Session};

fn bound_pool<'a>(
    env: &'a Env,
    key: &PriceKey,
    lp: &'a CurveStableLpSource,
    decimals: u32,
) -> Option<CurvePoolClient<'a>> {
    if *key != PriceKey::Token(lp.pool.clone())
        || decimals != 18
        || lp.pool.executable() != Some(Executable::Wasm(lp.wasm_hash.clone()))
    {
        return None;
    }
    let client = CurvePoolClient::new(env, &lp.pool);
    let info = client.try_pool_info().ok()?.ok()?;
    if client.try_decimals().ok()?.ok()? != 18
        || info.coins != lp.coins
        || info.scales.precision != WAD
    {
        return None;
    }
    // shortcut: fixed decimal rates only; rate-bearing pools need prices per normalized unit.
    for (i, coin) in lp.coins.iter().enumerate() {
        let decimals = info.coin_decimals.get(i as u32)?;
        if decimals > 18
            || token::Client::new(env, &coin).try_decimals().ok()?.ok()? != decimals
            || info.rates.get(i as u32)? != 10i128.pow(36 - decimals)
        {
            return None;
        }
    }
    Some(client)
}

pub(crate) fn read(
    session: &mut Session,
    key: &PriceKey,
    lp: &CurveStableLpSource,
    decimals: u32,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    let env = session.env().clone();
    let client = bound_pool(&env, key, lp, decimals).ok_or(OracleError::UnsupportedCurvePool)?;
    let mut minimum = i128::MAX;
    let mut timestamp = session.now_secs();
    for coin in lp.coins.iter() {
        let price = engine::resolve_nested(session, &PriceKey::Token(coin), depth + 1)?;
        minimum = minimum.min(price.price_wad);
        timestamp = timestamp.min(price.timestamp);
    }
    let virtual_price = client
        .try_get_virtual_price()
        .ok()
        .and_then(Result::ok)
        .ok_or(OracleError::NoLastPrice)?;
    let supply = client
        .try_total_supply()
        .ok()
        .and_then(Result::ok)
        .ok_or(OracleError::NoLastPrice)?;
    let price_wad = mul_div_floor_saturating(&env, virtual_price, minimum, WAD);
    if mul_div_floor_saturating(&env, price_wad, supply, WAD) < lp.min_pool_value_wad {
        return Err(OracleError::InsufficientCurveLiquidity);
    }
    Ok(Some((
        OracleObservation {
            price_wad,
            timestamp,
        },
        false,
    )))
}

#[cfg(test)]
#[path = "../../tests/oracle/curve_provider.rs"]
mod tests;
