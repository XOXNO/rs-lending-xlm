//! Blend V2 pool client for one-click migration.
//!
//! Mirrors only the Blend ABI surface the migration uses: `submit`. The
//! `BlendRequest` and `BlendPositions` field NAMES must match Blend exactly
//! (`#[contracttype]` structs encode as field-name maps); see Blend
//! `pool/src/pool/actions.rs` (`Request`) and `pool/src/pool/user.rs`
//! (`Positions`). The migration never reads Blend positions or reserves: it
//! repays with the caller's debt cap and reconciles Blend's over-repay refund.

use soroban_sdk::{contractclient, contracttype, Address, Env, Map, Vec};

/// Blend `RequestType` discriminants. Only these three are emitted by migration.
pub const REQ_WITHDRAW: u32 = 1; // sweep non-collateral supply
pub const REQ_WITHDRAW_COLLATERAL: u32 = 3; // sweep collateral
pub const REQ_REPAY: u32 = 5; // clear debt

/// A request against the Blend pool. Mirror of Blend `Request`.
#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// A user's per-pool position on Blend. Mirror of Blend `Positions`. Returned by
/// `submit`; decoded for type fidelity then discarded (migration measures
/// controller balance deltas instead of trusting this value).
#[contracttype]
#[derive(Clone)]
pub struct BlendPositions {
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

/// Calls Blend `submit`.
///
/// The caller MUST have emitted `authorize_as_current_contract` for the
/// controller's `spender` legs immediately before this call (no intervening
/// cross-call); the user authorizes the `from` leg through the transaction's
/// auth tree.
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
