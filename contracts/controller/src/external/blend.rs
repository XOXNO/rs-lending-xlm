//! Blend V2 pool adapter used by migration flows.
//!
//! Owns the whole Blend-facing surface: the `submit` ABI, the request-type
//! discriminants, the `i128::MAX` sweep convention, and the sub-contract auth
//! entries Blend needs to pull tokens. Callers see two verbs — sweep and repay —
//! so a Blend ABI change lands here and nowhere else.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{contractclient, contracttype, symbol_short, Address, Env, IntoVal, Map, Vec};

use crate::storage;

/// Blend `RequestType` discriminants emitted by migration.
const REQ_WITHDRAW: u32 = 1; // sweep non-collateral supply
const REQ_WITHDRAW_COLLATERAL: u32 = 3; // sweep collateral
const REQ_REPAY: u32 = 5; // clear debt

/// Request against the Blend pool. Mirrors Blend `Request`.
#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// User position returned by Blend `submit`.
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

fn blend_submit_call(
    env: &Env,
    blend_pool: &Address,
    from: &Address,
    spender: &Address,
    to: &Address,
    requests: &Vec<BlendRequest>,
) -> BlendPositions {
    BlendPoolClient::new(env, blend_pool).submit(from, spender, to, requests)
}

/// Sweeps `from`'s entire Blend balance to the controller: collateral first
/// (`WithdrawCollateral`), then non-collateral supply (`Withdraw`).
///
/// Each request carries `i128::MAX`, which Blend clamps to the actual balance —
/// the controller never needs to know the exact amounts. Withdraw-only submits
/// pull nothing from the controller, so they need no pull authorization.
pub(crate) fn blend_sweep_all(
    env: &Env,
    blend_pool: &Address,
    from: &Address,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
) {
    let mut requests: Vec<BlendRequest> = Vec::new(env);
    for asset in collateral_assets.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_WITHDRAW_COLLATERAL,
            address: asset,
            amount: i128::MAX,
        });
    }
    for asset in supply_assets.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_WITHDRAW,
            address: asset,
            amount: i128::MAX,
        });
    }
    guarded_submit(env, blend_pool, from, &requests);
}

/// Repays `from`'s Blend debt, capped per asset by `debt_caps`.
///
/// Authorization and submission are bundled deliberately: Blend pulls the debt
/// tokens from the controller during `submit`, so the auth entries must be
/// installed immediately before it and must not outlive it.
pub(crate) fn blend_repay_all(
    env: &Env,
    blend_pool: &Address,
    from: &Address,
    debt_caps: &Vec<(Address, i128)>,
) {
    let mut requests: Vec<BlendRequest> = Vec::new(env);
    for (asset, max) in debt_caps.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_REPAY,
            address: asset,
            amount: max,
        });
    }
    authorize_repay_pulls(env, blend_pool, debt_caps);
    guarded_submit(env, blend_pool, from, &requests);
}

/// Invokes the Blend pool's `submit` under the flash-loan reentrancy guard.
fn guarded_submit(env: &Env, blend_pool: &Address, from: &Address, requests: &Vec<BlendRequest>) {
    storage::with_flash_guard(env, || {
        let controller = env.current_contract_address();
        let _ = blend_submit_call(env, blend_pool, from, &controller, &controller, requests);
    });
}

/// Authorizes Blend debt-token pulls from the controller, bounded per asset by
/// its cap. Withdraw-only submits have no debt caps and need no authorization.
fn authorize_repay_pulls(env: &Env, blend_pool: &Address, debt_caps: &Vec<(Address, i128)>) {
    if debt_caps.is_empty() {
        return;
    }
    let controller = env.current_contract_address();
    let mut entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for (debt_asset, max) in debt_caps.iter() {
        entries.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: debt_asset,
                fn_name: symbol_short!("transfer"),
                args: (controller.clone(), blend_pool.clone(), max).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }));
    }
    env.authorize_as_current_contract(entries);
}
