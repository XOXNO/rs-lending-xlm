//! Prices Aquarius AMM LP share tokens for both constant-product and stable
//! pools. Cross-checks pool metadata reported directly by the pool contract
//! against the values reported by the shared plane contract before deriving a
//! fair LP price from the reserves and the underlying leg prices.

use common::constants::BPS;
use common::errors::OracleError;
use common::math::fp_core::try_mul_div_half_up;
use common::oracle::lp::{fair_lp_price_wad, isqrt_of_product, LpLeg, LpSupply};
use common::oracle::lp_stable::{fair_stable_lp_price_wad, stable_invariant_d_wad};
use common::oracle::observation::try_u256_to_i128;
use common::oracle::providers::aquarius::{
    aquarius_amp_call, aquarius_get_tokens_call, aquarius_is_constant_product_call,
    aquarius_is_stable_call, aquarius_plane_of_pool_call, aquarius_plane_reserves_call,
    aquarius_pool_reserves_call, aquarius_share_id_call, aquarius_stable_plane_reserves_call,
    aquarius_total_shares_call,
};
use common::types::{AquariusLpSource, AssetOracle, PriceKey};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Vec};

use crate::engine;
use crate::observation::OracleObservation;
use crate::session::Session;

/// Maximum allowed drift, in basis points, between the invariant computed
/// from a pool's direct reserves and the invariant computed from the plane
/// contract's reserves.
const MAX_LISTING_INVARIANT_DRIFT_BPS: i128 = 10;

/// Validates that `lp` describes a constant-product Aquarius pool consistent
/// with `key` and `oracle`. Checks that the pool's plane, share token, and
/// token pair match `lp`, that direct and plane-reported reserves agree
/// within `MAX_LISTING_INVARIANT_DRIFT_BPS`, that the pool reports itself as
/// constant-product with nonzero total shares, and that on-chain token
/// decimals match `oracle.asset_decimals` and `lp`. Panics with
/// `OracleError::InvalidOracleBase`, `OracleError::UnsupportedAquariusPool`,
/// or `OracleError::InvalidOracleDecimals` if any check fails.
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

/// Derives the fair price of a constant-product Aquarius LP share for `key`.
/// Re-validates the pool binding and token decimals, resolves both leg
/// prices recursively through `engine::resolve_nested`, and computes the LP
/// price from the plane-reported reserves and total shares. Returns
/// `Err(OracleError::NoLastPrice)` if the pool binding is invalid, the pool
/// is not constant-product, decimals mismatch, or reserves/shares cannot be
/// read. Returns `Err(OracleError::InsufficientAquariusLiquidity)` if the
/// computed pool value is below `lp.min_pool_value_wad`. On success, returns
/// the observation with the price and the earlier of the two leg timestamps.
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
    // Reserves come from the plane, share supply from the pool. Reject the read
    // when the two disagree, or a lagging plane prices a live share supply.
    let direct = aquarius_pool_reserves_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;
    if !reserve_invariants_match(&env, direct, (reserve_a, reserve_b)) {
        return Err(OracleError::UnsupportedAquariusPool);
    }
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

/// Validates that `lp` describes a stable Aquarius pool consistent with
/// `key` and `oracle`. Checks that the pool's plane, share token, and token
/// pair match `lp`, that the direct and plane-reported stable invariants
/// agree within `MAX_LISTING_INVARIANT_DRIFT_BPS`, that the pool reports
/// itself as stable with nonzero total shares, and that on-chain token
/// decimals match `oracle.asset_decimals` and `lp`. Panics with
/// `OracleError::InvalidOracleBase`, `OracleError::UnsupportedAquariusPool`,
/// or `OracleError::InvalidOracleDecimals` if any check fails.
pub(crate) fn attest_stable(
    env: &Env,
    key: &PriceKey,
    oracle: &AssetOracle,
    lp: &AquariusLpSource,
) {
    let tokens = bound_tokens(env, key, lp)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::InvalidOracleBase));
    let amp = aquarius_amp_call(env, &lp.pool)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::UnsupportedAquariusPool));
    let direct_reserves = aquarius_pool_reserves_call(env, &lp.pool);
    let plane_reserves = aquarius_stable_plane_reserves_call(env, &lp.plane, &lp.pool);
    let reserves_match = match (direct_reserves, plane_reserves) {
        (Some(direct), Some(plane)) => stable_invariants_match(env, direct, plane, amp, lp),
        _ => false,
    };
    assert_with_error!(
        env,
        aquarius_is_stable_call(env, &lp.pool) && reserves_match,
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

/// Derives the fair price of a stable Aquarius LP share for `key`.
/// Re-validates the pool binding and token decimals, resolves both leg
/// prices recursively through `engine::resolve_nested`, and computes the LP
/// price from the plane-reported reserves, total shares, and amplification
/// coefficient. Returns `Err(OracleError::NoLastPrice)` if the pool binding
/// is invalid, the pool is not stable, decimals mismatch, or
/// reserves/shares/amp cannot be read. Returns
/// `Err(OracleError::InsufficientAquariusLiquidity)` if the computed pool
/// value is below `lp.min_pool_value_wad`. On success, returns the
/// observation with the price and the earlier of the two leg timestamps.
pub(crate) fn read_stable(
    session: &mut Session,
    key: &PriceKey,
    lp: &AquariusLpSource,
    share_decimals: u32,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    let env = session.env().clone();
    let tokens = bound_tokens(&env, key, lp).ok_or(OracleError::NoLastPrice)?;
    if !aquarius_is_stable_call(&env, &lp.pool) {
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
    let (reserve_a, reserve_b) = aquarius_stable_plane_reserves_call(&env, &lp.plane, &lp.pool)
        .ok_or(OracleError::NoLastPrice)?;
    let total_shares =
        aquarius_total_shares_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;
    let amp = aquarius_amp_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;
    // Reserves come from the plane, share supply and amp from the pool. Reject
    // the read when the two disagree on the invariant.
    let direct = aquarius_pool_reserves_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;
    if !stable_invariants_match(&env, direct, (reserve_a, reserve_b), amp, lp) {
        return Err(OracleError::UnsupportedAquariusPool);
    }

    let price_wad = fair_stable_lp_price_wad(
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
        amp,
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

/// Computes the stable-swap invariant D from `direct` and from `plane`
/// reserves and reports whether the larger value stays within
/// `MAX_LISTING_INVARIANT_DRIFT_BPS` of the smaller. Returns `false` if D
/// cannot be computed for either reserve pair.
fn stable_invariants_match(
    env: &Env,
    direct: (i128, i128),
    plane: (i128, i128),
    amp: u128,
    lp: &AquariusLpSource,
) -> bool {
    let d_of = |(a, b): (i128, i128)| {
        stable_invariant_d_wad(env, a, lp.reserve_a_decimals, b, lp.reserve_b_decimals, amp).ok()
    };
    let (Some(direct_d), Some(plane_d)) = (d_of(direct), d_of(plane)) else {
        return false;
    };
    let (lower, upper) = if direct_d <= plane_d {
        (direct_d, plane_d)
    } else {
        (plane_d, direct_d)
    };
    try_mul_div_half_up(env, lower, BPS + MAX_LISTING_INVARIANT_DRIFT_BPS, BPS)
        .is_some_and(|ceiling| upper <= ceiling)
}

/// Confirms that `lp.pool` is registered under `lp.plane` and that its share
/// token matches `key`, then returns the pool's two underlying token
/// addresses if they equal `lp.token_a` and `lp.token_b` in order. Returns
/// `None` if the plane binding, share token, or token pair do not match.
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

/// Computes the constant-product invariant (the integer square root of the
/// reserve product) from `direct` and from `plane` reserves and reports
/// whether the larger value stays within `MAX_LISTING_INVARIANT_DRIFT_BPS`
/// of the smaller. Returns `false` if either reserve pair has a
/// non-positive value or the root cannot be computed.
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
