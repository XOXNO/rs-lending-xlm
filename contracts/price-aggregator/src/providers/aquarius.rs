use common::constants::BPS;
use common::errors::OracleError;
use common::math::fp_core::try_mul_div_half_up;
use common::oracle::lp::{fair_lp_price_wad, isqrt_of_product, LpLeg, LpSupply};
use common::oracle::observation::try_u256_to_i128;
use common::oracle::providers::aquarius::{
    aquarius_get_tokens_call, aquarius_is_constant_product_call, aquarius_plane_of_pool_call,
    aquarius_plane_reserves_call, aquarius_pool_reserves_call, aquarius_share_id_call,
    aquarius_total_shares_call,
};
use common::types::{AquariusLpSource, AssetOracle, PriceKey};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Vec};

use crate::engine;
use crate::observation::OracleObservation;
use crate::session::Session;

/// The plane can lag a swap without changing the constant-product fair value.
/// Bound drift in sqrt(k), the exact reserve term used by LP pricing.
const MAX_LISTING_INVARIANT_DRIFT_BPS: i128 = 10;

pub(crate) fn attest(env: &Env, key: &PriceKey, oracle: &AssetOracle, lp: &AquariusLpSource) {
    let tokens = bound_tokens(env, key, lp)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::InvalidOracleBase));
    let direct_reserves = aquarius_pool_reserves_call(env, &lp.pool);
    let plane_reserves = aquarius_plane_reserves_call(env, &lp.plane, &lp.pool);
    let reserves_match = match (direct_reserves, plane_reserves) {
        (Some(direct), Some(plane)) => reserve_invariants_match(env, direct, plane),
        _ => false,
    };
    assert_with_error!(
        env,
        aquarius_is_constant_product_call(env, &lp.pool) && reserves_match,
        OracleError::UnsupportedAquariusPool
    );
    assert_with_error!(
        env,
        aquarius_total_shares_call(env, &lp.pool).is_some_and(|shares| shares > 0),
        OracleError::UnsupportedAquariusPool
    );

    let PriceKey::Token(share) = key else {
        panic_with_error!(env, OracleError::InvalidOracleBase)
    };
    assert_with_error!(
        env,
        decimals_match(env, share, &tokens, oracle.asset_decimals, lp),
        OracleError::InvalidOracleDecimals
    );
}

pub(crate) fn read(
    session: &mut Session,
    key: &PriceKey,
    lp: &AquariusLpSource,
    share_decimals: u32,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    let env = session.env().clone();
    let tokens = bound_tokens(&env, key, lp).ok_or(OracleError::NoLastPrice)?;
    if !aquarius_is_constant_product_call(&env, &lp.pool) {
        return Err(OracleError::NoLastPrice);
    }
    let PriceKey::Token(share) = key else {
        return Err(OracleError::NoLastPrice);
    };
    if !decimals_match(&env, share, &tokens, share_decimals, lp) {
        return Err(OracleError::NoLastPrice);
    }
    let price_a = engine::resolve_nested(session, &lp.key_a, depth + 1)?;
    let price_b = engine::resolve_nested(session, &lp.key_b, depth + 1)?;
    let (reserve_a, reserve_b) =
        aquarius_plane_reserves_call(&env, &lp.plane, &lp.pool).ok_or(OracleError::NoLastPrice)?;
    let total_shares =
        aquarius_total_shares_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;

    let price_wad = fair_lp_price_wad(
        &env,
        &LpLeg {
            reserve: reserve_a,
            decimals: lp.reserve_a_decimals,
            price_wad: price_a.price_wad,
        },
        &LpLeg {
            reserve: reserve_b,
            decimals: lp.reserve_b_decimals,
            price_wad: price_b.price_wad,
        },
        &LpSupply {
            total_shares,
            decimals: share_decimals,
        },
    )?;
    let share_unit = 10i128
        .checked_pow(share_decimals)
        .ok_or(OracleError::InvalidPrice)?;
    let pool_value_wad = try_mul_div_half_up(&env, price_wad, total_shares, share_unit)
        .ok_or(OracleError::InvalidPrice)?;
    if pool_value_wad < lp.min_pool_value_wad {
        return Err(OracleError::InsufficientAquariusLiquidity);
    }

    Ok(Some((
        OracleObservation {
            price_wad,
            timestamp: price_a.timestamp.min(price_b.timestamp),
        },
        false,
    )))
}

fn bound_tokens(env: &Env, key: &PriceKey, lp: &AquariusLpSource) -> Option<Vec<Address>> {
    if aquarius_plane_of_pool_call(env, &lp.pool).as_ref() != Some(&lp.plane)
        || aquarius_share_id_call(env, &lp.pool)
            .map(PriceKey::Token)
            .as_ref()
            != Some(key)
    {
        return None;
    }
    aquarius_get_tokens_call(env, &lp.pool).filter(|tokens| {
        tokens.len() == 2
            && tokens.get_unchecked(0) == lp.token_a
            && tokens.get_unchecked(1) == lp.token_b
    })
}

fn decimals_match(
    env: &Env,
    share: &Address,
    tokens: &Vec<Address>,
    share_decimals: u32,
    lp: &AquariusLpSource,
) -> bool {
    token_decimals(env, share) == Some(share_decimals)
        && token_decimals(env, &tokens.get_unchecked(0)) == Some(lp.reserve_a_decimals)
        && token_decimals(env, &tokens.get_unchecked(1)) == Some(lp.reserve_b_decimals)
}

fn token_decimals(env: &Env, token: &Address) -> Option<u32> {
    match TokenClient::new(env, token).try_decimals() {
        Ok(Ok(decimals)) => Some(decimals),
        _ => None,
    }
}

fn reserve_invariants_match(env: &Env, direct: (i128, i128), plane: (i128, i128)) -> bool {
    let root = |(a, b): (i128, i128)| {
        if a <= 0 || b <= 0 {
            return None;
        }
        let a = u128::try_from(a).ok()?;
        let b = u128::try_from(b).ok()?;
        try_u256_to_i128(&isqrt_of_product(env, a, b))
    };
    let (Some(direct_root), Some(plane_root)) = (root(direct), root(plane)) else {
        return false;
    };
    let (lower, upper) = if direct_root <= plane_root {
        (direct_root, plane_root)
    } else {
        (plane_root, direct_root)
    };
    try_mul_div_half_up(env, lower, BPS + MAX_LISTING_INVARIANT_DRIFT_BPS, BPS)
        .is_some_and(|ceiling| upper <= ceiling)
}
