use soroban_sdk::{contractclient, Address, Env, Symbol, Vec};

#[contractclient(name = "AquariusPlaneClient")]
#[allow(dead_code)]
pub trait AquariusPlane {
    fn get(env: Env, pools: Vec<Address>) -> Vec<(Symbol, Vec<u128>, Vec<u128>)>;
}

#[contractclient(name = "AquariusPoolClient")]
#[allow(dead_code)]
pub trait AquariusPool {
    fn get_total_shares(env: Env) -> u128;
    fn get_pools_plane(env: Env) -> Address;
    fn get_reserves(env: Env) -> Vec<u128>;
    fn get_tokens(env: Env) -> Vec<Address>;
    fn pool_type(env: Env) -> Symbol;
    fn share_id(env: Env) -> Address;

    fn a(env: Env) -> u128;
}

pub fn aquarius_plane_reserves_call(
    env: &Env,
    plane: &Address,
    pool: &Address,
) -> Option<(i128, i128)> {
    plane_reserves_of_kind(env, plane, pool, "standard")
}

pub fn aquarius_stable_plane_reserves_call(
    env: &Env,
    plane: &Address,
    pool: &Address,
) -> Option<(i128, i128)> {
    plane_reserves_of_kind(env, plane, pool, "stable")
}

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

pub fn aquarius_amp_call(env: &Env, pool: &Address) -> Option<u128> {
    match AquariusPoolClient::new(env, pool).try_a() {
        Ok(Ok(amp)) if amp > 0 => Some(amp),
        _ => None,
    }
}

pub fn aquarius_is_stable_call(env: &Env, pool: &Address) -> bool {
    matches!(
        AquariusPoolClient::new(env, pool).try_pool_type(),
        Ok(Ok(kind)) if kind == Symbol::new(env, "stable")
    )
}

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

pub fn aquarius_is_constant_product_call(env: &Env, pool: &Address) -> bool {
    matches!(
        AquariusPoolClient::new(env, pool).try_pool_type(),
        Ok(Ok(kind)) if kind == Symbol::new(env, "constant_product")
    )
}

pub fn aquarius_get_tokens_call(env: &Env, pool: &Address) -> Option<Vec<Address>> {
    match AquariusPoolClient::new(env, pool).try_get_tokens() {
        Ok(Ok(tokens)) => Some(tokens),
        _ => None,
    }
}

pub fn aquarius_share_id_call(env: &Env, pool: &Address) -> Option<Address> {
    match AquariusPoolClient::new(env, pool).try_share_id() {
        Ok(Ok(addr)) => Some(addr),
        _ => None,
    }
}

pub fn aquarius_total_shares_call(env: &Env, pool: &Address) -> Option<i128> {
    match AquariusPoolClient::new(env, pool).try_get_total_shares() {
        Ok(Ok(shares)) => i128::try_from(shares).ok(),
        _ => None,
    }
}

pub fn aquarius_plane_of_pool_call(env: &Env, pool: &Address) -> Option<Address> {
    match AquariusPoolClient::new(env, pool).try_get_pools_plane() {
        Ok(Ok(plane)) => Some(plane),
        _ => None,
    }
}
