//! Integration with an external Blend Pool contract.
//!
//! Builds `BlendRequest` batches for sweeping all collateral/supply out of the
//! pool and for repaying capped debt amounts, authorizes the token transfers
//! the pool needs to pull during repayment, and submits requests to the pool
//! inside the flash-loan reentrancy guard.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{contractclient, contracttype, symbol_short, Address, Env, IntoVal, Map, Vec};

use crate::storage;

const REQ_WITHDRAW: u32 = 1;
const REQ_WITHDRAW_COLLATERAL: u32 = 3;
const REQ_REPAY: u32 = 5;

/// A single instruction in a Blend Pool `submit` batch: a request type code
/// (withdraw, withdraw-collateral, or repay, per the `REQ_*` constants in
/// this module), the reserve asset it applies to, and the amount.
#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// Positions the Blend Pool returns from a `submit` call: liabilities,
/// collateral, and supply balances keyed by reserve index.
#[contracttype]
#[derive(Clone)]
pub struct BlendPositions {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

/// Client interface for the external Blend Pool contract's `submit` entry point.
#[allow(dead_code)]
#[contractclient(name = "BlendPoolClient")]
pub trait BlendPool {
    /// Submits `requests` against the pool on behalf of `from`, with `spender`
    /// providing any tokens the requests need and `to` receiving any tokens
    /// the requests release. Returns the resulting positions.
    fn submit(
        env: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<BlendRequest>,
    ) -> BlendPositions;
}

/// Withdraws all of `from`'s collateral and supply positions from the pool,
/// requesting the maximum representable amount for each asset in
/// `collateral_assets` and `supply_assets` so the pool withdraws the full
/// balance it holds.
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

/// Repays each debt asset in `debt_caps` up to its capped amount. Authorizes
/// the pool to pull the corresponding token amounts from the controller
/// before submitting the repay requests.
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

/// Submits `requests` to the pool with the controller acting as both spender
/// and recipient, running inside the flash-loan reentrancy guard.
fn guarded_submit(env: &Env, blend_pool: &Address, from: &Address, requests: &Vec<BlendRequest>) {
    storage::with_flash_guard(env, || {
        let controller = env.current_contract_address();
        let _ =
            BlendPoolClient::new(env, blend_pool).submit(from, &controller, &controller, requests);
    });
}

/// Authorizes a `transfer` call from the controller to `blend_pool` for each
/// debt asset in `debt_caps`, up to its capped amount. Returns immediately
/// without authorizing anything if `debt_caps` is empty.
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
