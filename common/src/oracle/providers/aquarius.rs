//! Cross-contract client traits and call helpers for Aquarius AMM pools,
//! used to read reserves, token metadata, and pool type for price derivation.

use soroban_sdk::{contractclient, Address, Env, Symbol, Vec};

/// Client interface for an individual Aquarius pool contract.
#[contractclient(name = "AquariusPoolClient")]
#[allow(dead_code)]
pub trait AquariusPool {
    /// Returns the pool's total LP share supply.
    fn get_total_shares(env: Env) -> u128;
    /// Returns the pool's token reserves.
    fn get_reserves(env: Env) -> Vec<u128>;
    /// Returns the pool's underlying token addresses.
    fn get_tokens(env: Env) -> Vec<Address>;
    /// Returns the pool's type symbol, e.g. `stable` or `constant_product`.
    fn pool_type(env: Env) -> Symbol;
    /// Returns the address of the pool's LP share token.
    fn share_id(env: Env) -> Address;

    /// Returns the pool's amplification coefficient.
    fn a(env: Env) -> u128;
}

/// Returns `pool`'s amplification coefficient. Returns `None` if the call
/// fails or the returned value is not greater than zero.
pub fn aquarius_amp_call(env: &Env, pool: &Address) -> Option<u128> {
    match AquariusPoolClient::new(env, pool).try_a() {
        Ok(Ok(amp)) if amp > 0 => Some(amp),
        _ => None,
    }
}

/// Returns whether `pool`'s `pool_type` equals `stable`. Returns `false` if
/// the call fails or the type does not match.
pub fn aquarius_is_stable_call(env: &Env, pool: &Address) -> bool {
    matches!(
        AquariusPoolClient::new(env, pool).try_pool_type(),
        Ok(Ok(kind)) if kind == Symbol::new(env, "stable")
    )
}

/// Reads `pool`'s reserves directly via `get_reserves`. Returns `None` if
/// the call fails, the reserve count is not two, or a reserve does not fit in i128.
pub fn aquarius_pool_reserves_call(env: &Env, pool: &Address) -> Option<(i128, i128)> {
    let reserves = match AquariusPoolClient::new(env, pool).try_get_reserves() {
        Ok(Ok(reserves)) => reserves,
        _ => return None,
    };
    if reserves.len() != 2 {
        return None;
    }
    Some((
        i128::try_from(reserves.get_unchecked(0)).ok()?,
        i128::try_from(reserves.get_unchecked(1)).ok()?,
    ))
}

/// Returns whether `pool`'s `pool_type` equals `constant_product`. Returns
/// `false` if the call fails or the type does not match.
pub fn aquarius_is_constant_product_call(env: &Env, pool: &Address) -> bool {
    matches!(
        AquariusPoolClient::new(env, pool).try_pool_type(),
        Ok(Ok(kind)) if kind == Symbol::new(env, "constant_product")
    )
}

/// Returns `pool`'s underlying token addresses via `get_tokens`, or `None`
/// if the call fails.
pub fn aquarius_get_tokens_call(env: &Env, pool: &Address) -> Option<Vec<Address>> {
    match AquariusPoolClient::new(env, pool).try_get_tokens() {
        Ok(Ok(tokens)) => Some(tokens),
        _ => None,
    }
}

/// Returns the address of `pool`'s LP share token via `share_id`, or `None`
/// if the call fails.
pub fn aquarius_share_id_call(env: &Env, pool: &Address) -> Option<Address> {
    match AquariusPoolClient::new(env, pool).try_share_id() {
        Ok(Ok(addr)) => Some(addr),
        _ => None,
    }
}

/// Returns `pool`'s total LP share supply as `i128`. Returns `None` if the
/// call fails or the value does not fit in i128.
pub fn aquarius_total_shares_call(env: &Env, pool: &Address) -> Option<i128> {
    match AquariusPoolClient::new(env, pool).try_get_total_shares() {
        Ok(Ok(shares)) => i128::try_from(shares).ok(),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../tests/oracle/providers/aquarius.rs"]
mod tests;
