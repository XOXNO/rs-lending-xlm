//! Conservative plain-pool valuation using Curve's own invariant/share view.

use common::constants::WAD;
use common::errors::OracleError;
use common::oracle::providers::curve::CurvePoolClient;
use common::types::{AssetOracle, CurveStableLpSource, PriceKey, MAX_CURVE_COINS};
use soroban_sdk::{panic_with_error, token, Env, Executable, U256};

use crate::{engine, observation::OracleObservation, session::Session};

pub(crate) fn shape(lp: &CurveStableLpSource) -> bool {
    (2..=MAX_CURVE_COINS).contains(&lp.coins.len())
        && lp.min_pool_value_wad > 0
        && lp
            .coins
            .iter()
            .enumerate()
            .all(|(i, coin)| coin != lp.pool && lp.coins.first_index_of(coin) == Some(i as u32))
}

fn bound_pool<'a>(
    env: &'a Env,
    key: &PriceKey,
    lp: &'a CurveStableLpSource,
    decimals: u32,
) -> Option<CurvePoolClient<'a>> {
    if !shape(lp)
        || *key != PriceKey::Token(lp.pool.clone())
        || decimals != 18
        || lp.pool.executable() != Some(Executable::Wasm(lp.wasm_hash.clone()))
    {
        return None;
    }
    let client = CurvePoolClient::new(env, &lp.pool);
    let info = client.try_pool_info().ok()?.ok()?;
    if client.try_decimals().ok()?.ok()? != 18
        || info.coin_count != lp.coins.len()
        || info.coins != lp.coins
        || info.coin_decimals.len() != info.coin_count
        || info.rates.len() != info.coin_count
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

pub(crate) fn attest(env: &Env, key: &PriceKey, oracle: &AssetOracle, lp: &CurveStableLpSource) {
    bound_pool(env, key, lp, oracle.asset_decimals)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::UnsupportedCurvePool));
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
    if virtual_price <= 0 || minimum <= 0 || supply <= 0 {
        return Err(OracleError::InvalidPrice);
    }
    let price_wad = floor_product(&env, virtual_price, minimum)?;
    if floor_product(&env, price_wad, supply)? < lp.min_pool_value_wad {
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

fn floor_product(env: &Env, a: i128, b: i128) -> Result<i128, OracleError> {
    U256::from_u128(env, a as u128)
        .mul(&U256::from_u128(env, b as u128))
        .div(&U256::from_u128(env, WAD as u128))
        .to_u128()
        .and_then(|value| i128::try_from(value).ok())
        .ok_or(OracleError::InvalidPrice)
}

#[cfg(test)]
#[path = "../../tests/oracle/curve_provider.rs"]
mod tests;
