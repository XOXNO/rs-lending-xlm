//! One-time deployment of the controller and price aggregator contracts
//! from this contract, using deterministic per-contract deploy salts.

use common::errors::GenericError;

use soroban_sdk::{assert_with_error, vec, Address, BytesN, Env, IntoVal, Symbol, Val};

use crate::events::{DeployControllerEvent, DeployPriceAggregatorEvent};
use crate::storage;
use crate::validate;

#[cfg(any(test, feature = "testing"))]
use crate::{Governance, GovernanceArgs, GovernanceClient};

/// Deploy salt for the controller contract, fixed so the controller always
/// deploys to the same deterministic address under this contract.
const CONTROLLER_DEPLOY_SALT: [u8; 32] = [0u8; 32];

/// Deploy salt for the price aggregator contract, fixed so the price
/// aggregator always deploys to the same deterministic address under this
/// contract.
const PRICE_AGGREGATOR_DEPLOY_SALT: [u8; 32] = [1u8; 32];

/// Deploys the controller contract from `wasm_hash`, constructed with this
/// contract's address, and records its address in storage. Publishes a
/// `DeployControllerEvent`. Panics with `GenericError::InvalidWasmHash` if
/// `wasm_hash` is all zero, and with `GenericError::PoolAlreadyDeployed` if
/// a controller is already deployed.
pub(crate) fn deploy_controller(env: &Env, wasm_hash: BytesN<32>) -> Address {
    storage::renew_governance_instance(env);
    validate::require_nonzero_wasm_hash(env, &wasm_hash);
    assert_with_error!(
        env,
        !storage::has_controller(env),
        GenericError::PoolAlreadyDeployed
    );

    let salt = BytesN::from_array(env, &CONTROLLER_DEPLOY_SALT);
    let controller = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash.clone(), (env.current_contract_address(),));

    storage::set_controller(env, &controller);

    DeployControllerEvent {
        controller: controller.clone(),
        wasm_hash,
    }
    .publish(env);

    controller
}

/// Deploys the price aggregator contract from `wasm_hash`, constructed with
/// this contract's address, and records its address in storage. If a
/// controller is already deployed, invokes `set_price_aggregator` on it to
/// register the new price aggregator. Publishes a
/// `DeployPriceAggregatorEvent`. Panics with `GenericError::InvalidWasmHash`
/// if `wasm_hash` is all zero, and with `GenericError::PoolAlreadyDeployed`
/// if a price aggregator is already deployed.
pub(crate) fn deploy_price_aggregator(env: &Env, wasm_hash: BytesN<32>) -> Address {
    storage::renew_governance_instance(env);
    validate::require_nonzero_wasm_hash(env, &wasm_hash);
    assert_with_error!(
        env,
        !storage::has_price_aggregator(env),
        GenericError::PoolAlreadyDeployed
    );

    let salt = BytesN::from_array(env, &PRICE_AGGREGATOR_DEPLOY_SALT);
    let price_aggregator = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash.clone(), (env.current_contract_address(),));

    storage::set_price_aggregator(env, &price_aggregator);

    if storage::has_controller(env) {
        env.invoke_contract::<Val>(
            &storage::get_controller(env),
            &Symbol::new(env, "set_price_aggregator"),
            vec![env, price_aggregator.clone().into_val(env)],
        );
    }

    DeployPriceAggregatorEvent {
        price_aggregator: price_aggregator.clone(),
        wasm_hash,
    }
    .publish(env);

    price_aggregator
}

/// Test-only entry points for setting the controller and price aggregator
/// addresses directly, bypassing deployment.
#[cfg(any(test, feature = "testing"))]
#[soroban_sdk::contractimpl]
impl Governance {
    /// Sets the controller address directly in storage.
    pub fn set_controller(env: Env, addr: Address) {
        storage::set_controller(&env, &addr);
    }

    /// Sets the price aggregator address directly in storage.
    pub fn set_price_aggregator(env: Env, addr: Address) {
        storage::set_price_aggregator(&env, &addr);
    }
}
