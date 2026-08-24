//! Prices Aquarius AMM LP share tokens for both constant-product and stable
//! pools from the pool contract's own reserves, share supply, and metadata.

use common::errors::OracleError;
use common::math::fp_core::try_mul_div_half_up;
use common::oracle::lp::{fair_lp_price_wad, LpLeg, LpSupply};
use common::oracle::lp_stable::fair_stable_lp_price_wad;
use common::oracle::providers::aquarius::{
    aquarius_amp_call, aquarius_get_tokens_call, aquarius_is_constant_product_call,
    aquarius_is_stable_call, aquarius_pool_reserves_call, aquarius_share_id_call,
    aquarius_total_shares_call,
};
use common::types::{AquariusLpSource, AssetOracle, PriceKey};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Vec};

use crate::engine;
use crate::observation::OracleObservation;
use crate::session::Session;

/// Validates that `lp` describes an Aquarius pool matching `key` and
/// `oracle`: the share token and token pair bind to `lp`, the pool reports
/// itself as the expected kind — stable with a positive amplification
/// coefficient when `stable`, constant-product otherwise — with positive
/// reserves and total shares, and token decimals match
/// `oracle.asset_decimals` and `lp`. Panics if any check fails.
pub(crate) fn attest(
    env: &Env,
    key: &PriceKey,
    oracle: &AssetOracle,
    lp: &AquariusLpSource,
    stable: bool,
) {
    let tokens = bound_tokens(env, key, lp)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::InvalidOracleBase));
    assert_with_error!(
        env,
        pool_kind_matches(env, lp, stable)
            && (!stable || aquarius_amp_call(env, &lp.pool).is_some())
            && aquarius_pool_reserves_call(env, &lp.pool).is_some_and(|(a, b)| a > 0 && b > 0),
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

/// Derives the fair price of an Aquarius LP share for `key` by re-validating
/// the pool binding and decimals, resolving both leg prices through
/// `engine::resolve_nested`, and computing the price from the pool's
/// reserves and total shares — plus its amplification coefficient when
/// `stable`. Returns an error if validation fails, the pool cannot be read,
/// or the pool's value falls below `lp.min_pool_value_wad`. On success, the
/// observation carries the earlier of the two leg timestamps.
pub(crate) fn read(
    session: &mut Session,
    key: &PriceKey,
    lp: &AquariusLpSource,
    share_decimals: u32,
    depth: u32,
    stable: bool,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    let env = session.env().clone();
    let tokens = bound_tokens(&env, key, lp).ok_or(OracleError::NoLastPrice)?;
    if !pool_kind_matches(&env, lp, stable) {
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
        aquarius_pool_reserves_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;
    let total_shares =
        aquarius_total_shares_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;

    let leg_a = LpLeg {
        reserve: reserve_a,
        decimals: lp.reserve_a_decimals,
        price_wad: price_a.price_wad,
    };
    let leg_b = LpLeg {
        reserve: reserve_b,
        decimals: lp.reserve_b_decimals,
        price_wad: price_b.price_wad,
    };
    let supply = LpSupply {
        total_shares,
        decimals: share_decimals,
    };
    let price_wad = if stable {
        let amp = aquarius_amp_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;
        fair_stable_lp_price_wad(&env, &leg_a, &leg_b, &supply, amp)?
    } else {
        fair_lp_price_wad(&env, &leg_a, &leg_b, &supply)?
    };
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

/// Reports whether `lp.pool` still describes itself as the kind the source
/// was configured for: a stable pool when `stable`, a constant-product pool
/// otherwise.
fn pool_kind_matches(env: &Env, lp: &AquariusLpSource, stable: bool) -> bool {
    if stable {
        aquarius_is_stable_call(env, &lp.pool)
    } else {
        aquarius_is_constant_product_call(env, &lp.pool)
    }
}

/// Confirms that `lp.pool`'s share token matches `key`, then returns the
/// pool's two underlying token addresses if they equal `lp.token_a` and
/// `lp.token_b` in order. Returns `None` if the share token or token pair
/// do not match.
fn bound_tokens(env: &Env, key: &PriceKey, lp: &AquariusLpSource) -> Option<Vec<Address>> {
    let PriceKey::Token(share) = key else {
        return None;
    };
    if aquarius_share_id_call(env, &lp.pool).as_ref() != Some(share) {
        return None;
    }
    aquarius_get_tokens_call(env, &lp.pool).filter(|tokens| {
        tokens.len() == 2
            && tokens.get_unchecked(0) == lp.token_a
            && tokens.get_unchecked(1) == lp.token_b
    })
}

/// Reports whether the on-chain decimals of the share token and both
/// reserve tokens equal `share_decimals` and `lp.reserve_a_decimals` /
/// `lp.reserve_b_decimals` respectively.
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

/// Fetches `token`'s decimals via the token contract's `decimals` entry
/// point. Returns `None` if the call fails.
fn token_decimals(env: &Env, token: &Address) -> Option<u32> {
    match TokenClient::new(env, token).try_decimals() {
        Ok(Ok(decimals)) => Some(decimals),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../tests/oracle/aquarius_provider.rs"]
mod tests;
