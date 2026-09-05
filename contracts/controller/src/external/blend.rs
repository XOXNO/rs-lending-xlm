use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{contractclient, contracttype, symbol_short, Address, Env, IntoVal, Map, Vec};

use crate::storage;

const REQ_WITHDRAW: u32 = 1;
const REQ_WITHDRAW_COLLATERAL: u32 = 3;
const REQ_REPAY: u32 = 5;

/// Request record matching Blend's `submit` ABI.
#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// Position response matching Blend's `submit` ABI.
#[contracttype]
#[derive(Clone)]
pub struct BlendPositions {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

#[allow(dead_code)]
#[contractclient(name = "BlendPoolClient")]
pub trait BlendPool {
    /// Submits requests against `from`'s position, pulling repayments from `spender`
    /// and sending withdrawals to `to`.
    fn submit(
        env: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<BlendRequest>,
    ) -> BlendPositions;
}

/// Fully withdraws the specified collateral and supply assets to the controller.
/// Uses Blend's `i128::MAX` full-withdrawal sentinel for each requested asset.
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

/// Repays the specified debts up to their caps using controller funds.
/// Authorizes the corresponding token pulls before submitting requests.
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

/// Submits with the controller as spender and recipient while holding the
/// flash-loan guard against reentry into guarded controller operations.
fn guarded_submit(env: &Env, blend_pool: &Address, from: &Address, requests: &Vec<BlendRequest>) {
    storage::with_flash_guard(env, || {
        let controller = env.current_contract_address();
        let _ =
            BlendPoolClient::new(env, blend_pool).submit(from, &controller, &controller, requests);
    });
}

/// Authorizes exact cap-sized token transfers from the controller to Blend
/// for nested repayment calls; no-op when no debt caps are supplied.
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

#[cfg(test)]
#[path = "../../tests/external/blend.rs"]
mod tests;
