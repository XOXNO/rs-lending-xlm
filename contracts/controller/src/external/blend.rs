//! Blend V2 pool client used by migration flows.
//! Mirrors the Blend `submit` ABI; field names must match Blend maps.

use soroban_sdk::{contractclient, contracttype, Address, Env, Map, Vec};

/// Blend `RequestType` discriminants emitted by migration.
pub(crate) const REQ_WITHDRAW: u32 = 1; // sweep non-collateral supply
pub(crate) const REQ_WITHDRAW_COLLATERAL: u32 = 3; // sweep collateral
pub(crate) const REQ_REPAY: u32 = 5; // clear debt

/// Request against the Blend pool. Mirrors Blend `Request`.
#[contracttype]
#[derive(Clone)]
pub(crate) struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// User position returned by Blend `submit`.
#[contracttype]
#[derive(Clone)]
pub(crate) struct BlendPositions {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

#[allow(dead_code)] // Generates the Soroban client proxy.
#[contractclient(name = "BlendPoolClient")]
pub trait BlendPool {
    fn submit(
        env: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<BlendRequest>,
    ) -> BlendPositions;
}

pub(crate) fn blend_submit_call(
    env: &Env,
    blend_pool: &Address,
    from: &Address,
    spender: &Address,
    to: &Address,
    requests: &Vec<BlendRequest>,
) -> BlendPositions {
    BlendPoolClient::new(env, blend_pool).submit(from, spender, to, requests)
}
