//! Client mirrors and read helpers for Aquarius AMM constant-product pools.
//!
//! Reserves are read from the pool's `plane` mirror — a compact registry the
//! pool updates on every state change — because that read is side-effect free.
//! The pool's own `get_reserves` lazily *syncs* (writes) and is deliberately
//! avoided on the pricing path.

use soroban_sdk::{contractclient, Address, Env, Symbol, Vec};

/// Pool-registry plane: `get` returns, per pool, `(kind, params, reserves)`
/// where `reserves` mirrors the pool's stored reserves in token order.
#[contractclient(name = "AquariusPlaneClient")]
#[allow(dead_code)]
pub trait AquariusPlane {
    fn get(env: Env, pools: Vec<Address>) -> Vec<(Symbol, Vec<u128>, Vec<u128>)>;
}

/// Read surface of a constant-product pool. Bindings and the read-only methods
/// are rechecked while pricing; `get_reserves` is reserved for listing-time
/// attestation because Aquarius may synchronize state during that call.
#[contractclient(name = "AquariusPoolClient")]
#[allow(dead_code)]
pub trait AquariusPool {
    fn get_total_shares(env: Env) -> u128;
    fn get_pools_plane(env: Env) -> Address;
    fn get_reserves(env: Env) -> Vec<u128>;
    fn get_tokens(env: Env) -> Vec<Address>;
    fn pool_type(env: Env) -> Symbol;
    fn share_id(env: Env) -> Address;
}

/// Reserves `(a, b)` for `pool`, read from its `plane` mirror. `None` when the
/// plane has no row, the row is malformed, or a reserve exceeds `i128`.
pub fn aquarius_plane_reserves_call(
    env: &Env,
    plane: &Address,
    pool: &Address,
) -> Option<(i128, i128)> {
    let pools = Vec::from_array(env, [pool.clone()]);
    let rows = match AquariusPlaneClient::new(env, plane).try_get(&pools) {
        Ok(Ok(rows)) => rows,
        _ => return None,
    };
    let (kind, _params, reserves) = rows.get(0)?;
    // Only "standard" (constant-product) pools expose exactly [reserve0, reserve1];
    // "stable"/"concentrated" rows carry a different or bucketed layout whose
    // first two entries are NOT the pool's total reserves. Reject anything else
    // so a mislisted (or type-changed) pool can never be priced as constant-product.
    if kind != Symbol::new(env, "standard") || reserves.len() != 2 {
        return None;
    }
    let a = i128::try_from(reserves.get_unchecked(0)).ok()?;
    let b = i128::try_from(reserves.get_unchecked(1)).ok()?;
    Some((a, b))
}

/// Reserves read directly from the pool. Aquarius may synchronize state during
/// this call, so it is deliberately used only while listing/relisting a pool,
/// never on the hot price-read path.
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

/// Direct pool-type attestation, independent of the plane's row label.
pub fn aquarius_is_constant_product_call(env: &Env, pool: &Address) -> bool {
    matches!(
        AquariusPoolClient::new(env, pool).try_pool_type(),
        Ok(Ok(kind)) if kind == Symbol::new(env, "constant_product")
    )
}

/// The pool's two reserve-token addresses, in reserve order. `None` on failure.
pub fn aquarius_get_tokens_call(env: &Env, pool: &Address) -> Option<Vec<Address>> {
    match AquariusPoolClient::new(env, pool).try_get_tokens() {
        Ok(Ok(tokens)) => Some(tokens),
        _ => None,
    }
}

/// The pool's LP share-token address. `None` on failure.
pub fn aquarius_share_id_call(env: &Env, pool: &Address) -> Option<Address> {
    match AquariusPoolClient::new(env, pool).try_share_id() {
        Ok(Ok(addr)) => Some(addr),
        _ => None,
    }
}

/// Total LP share supply for `pool`. `None` on a failed call or `i128` overflow.
pub fn aquarius_total_shares_call(env: &Env, pool: &Address) -> Option<i128> {
    match AquariusPoolClient::new(env, pool).try_get_total_shares() {
        Ok(Ok(shares)) => i128::try_from(shares).ok(),
        _ => None,
    }
}

/// The `plane` address a pool currently reports.
pub fn aquarius_plane_of_pool_call(env: &Env, pool: &Address) -> Option<Address> {
    match AquariusPoolClient::new(env, pool).try_get_pools_plane() {
        Ok(Ok(plane)) => Some(plane),
        _ => None,
    }
}
