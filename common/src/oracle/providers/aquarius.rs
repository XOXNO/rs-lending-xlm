//! Cross-contract client traits and call helpers for Aquarius AMM pool and
//! plane contracts, used to read pool reserves, token metadata, and pool
//! type for price derivation.

use soroban_sdk::{contractclient, Address, Env, Symbol, Vec};

/// Client interface for an Aquarius plane contract, which aggregates pool
/// kind, parameters, and reserves for a set of pools.
#[contractclient(name = "AquariusPlaneClient")]
#[allow(dead_code)]
pub trait AquariusPlane {
    /// Returns, for each pool in `pools`, its kind symbol, parameters, and reserves.
    fn get(env: Env, pools: Vec<Address>) -> Vec<(Symbol, Vec<u128>, Vec<u128>)>;
}

/// Client interface for an individual Aquarius pool contract.
#[contractclient(name = "AquariusPoolClient")]
#[allow(dead_code)]
pub trait AquariusPool {
    /// Returns the pool's total LP share supply.
    fn get_total_shares(env: Env) -> u128;
    /// Returns the address of the plane contract that aggregates this pool.
    fn get_pools_plane(env: Env) -> Address;
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

/// Reads `pool`'s two-asset reserves from `plane`, requiring the pool's row
/// to be of kind `standard`. Returns `None` if the call fails, the kind
/// does not match, the reserve count is not two, or a reserve does not fit in i128.
pub fn aquarius_plane_reserves_call(
    env: &Env,
    plane: &Address,
    pool: &Address,
) -> Option<(i128, i128)> {
    plane_reserves_of_kind(env, plane, pool, "standard")
}

/// Reads `pool`'s two-asset reserves from `plane`, requiring the pool's row
/// to be of kind `stable`. Returns `None` if the call fails, the kind does
/// not match, the reserve count is not two, or a reserve does not fit in i128.
pub fn aquarius_stable_plane_reserves_call(
    env: &Env,
    plane: &Address,
    pool: &Address,
) -> Option<(i128, i128)> {
    plane_reserves_of_kind(env, plane, pool, "stable")
}

/// Queries `plane` for `pool`'s row and returns its reserves as `(i128, i128)`
/// if the row's kind symbol equals `kind` and it contains exactly two
/// reserves that fit in i128. Returns `None` on any mismatch or call failure.
fn plane_reserves_of_kind(
    env: &Env,
    plane: &Address,
    pool: &Address,
    kind: &str,
) -> Option<(i128, i128)> {
    let pools = Vec::from_array(env, [pool.clone()]);
    let rows = match AquariusPlaneClient::new(env, plane).try_get(&pools) {
        Ok(Ok(rows)) => rows,
        _ => return None,
    };
    let (row_kind, _params, reserves) = rows.get(0)?;
    if row_kind != Symbol::new(env, kind) || reserves.len() != 2 {
        return None;
    }
    let a = i128::try_from(reserves.get_unchecked(0)).ok()?;
    let b = i128::try_from(reserves.get_unchecked(1)).ok()?;
    Some((a, b))
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

/// Returns the address of the plane contract associated with `pool` via
/// `get_pools_plane`, or `None` if the call fails.
pub fn aquarius_plane_of_pool_call(env: &Env, pool: &Address) -> Option<Address> {
    match AquariusPoolClient::new(env, pool).try_get_pools_plane() {
        Ok(Ok(plane)) => Some(plane),
        _ => None,
    }
}
