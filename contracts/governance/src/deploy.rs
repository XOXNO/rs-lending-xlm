//! One-time controller deployment and address lookup.

use common::errors::GenericError;

use soroban_sdk::{assert_with_error, contractimpl, Address, BytesN, Env};

use stellar_macros::only_owner;

use crate::events::{DeployControllerEvent, DeployPriceAggregatorEvent};
use crate::validate;
use crate::{storage, Governance, GovernanceArgs, GovernanceClient};

/// Deterministic salt for the one-time controller deployment; the controller
/// address derives from (governance address, salt).
const CONTROLLER_DEPLOY_SALT: [u8; 32] = [0u8; 32];

/// Deterministic salt for the one-time price-aggregator deployment.
const PRICE_AGGREGATOR_DEPLOY_SALT: [u8; 32] = [1u8; 32];

#[contractimpl]
impl Governance {
    /// Deploys the lending controller once and records its address, with the
    /// governance contract as the controller's constructor admin.
    ///
    /// # Arguments
    /// * `wasm_hash` - compiled controller Wasm (already installed).
    ///
    /// # Errors
    /// * `InvalidPoolTemplate` - `wasm_hash` is all-zero.
    /// * `PoolAlreadyDeployed` - a controller address is already stored.
    ///
    /// # Events
    /// * `DeployControllerEvent` - the deployed controller address and wasm hash.
    ///
    /// # Security Warning
    /// * Governance becomes the controller's admin, so it holds every
    ///   controller admin power. Owner-gated and one-shot; deployment tooling
    ///   must set the intended owner before calling.
    #[only_owner]
    pub fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address {
        storage::renew_governance_instance(&env);
        validate::require_nonzero_wasm_hash(&env, &wasm_hash);
        assert_with_error!(
            &env,
            !storage::has_controller(&env),
            GenericError::PoolAlreadyDeployed
        );

        let salt = BytesN::from_array(&env, &CONTROLLER_DEPLOY_SALT);
        let controller = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash.clone(), (env.current_contract_address(),));

        storage::set_controller(&env, &controller);

        DeployControllerEvent {
            controller: controller.clone(),
            wasm_hash,
        }
        .publish(&env);

        controller
    }

    pub fn controller(env: Env) -> Address {
        storage::get_controller(&env)
    }

    /// Deploys the price-aggregator once and records its address, with the
    /// governance contract as the aggregator's constructor owner.
    ///
    /// # Arguments
    /// * `wasm_hash` - compiled price-aggregator Wasm (already installed).
    ///
    /// # Errors
    /// * `InvalidPoolTemplate` - `wasm_hash` is all-zero.
    /// * `PoolAlreadyDeployed` - a price-aggregator address is already stored.
    ///
    /// # Events
    /// * `DeployPriceAggregatorEvent` - the deployed address and wasm hash.
    ///
    /// # Security Warning
    /// * Governance becomes the aggregator's owner, holding every oracle admin
    ///   power. Owner-gated and one-shot.
    #[only_owner]
    pub fn deploy_price_aggregator(env: Env, wasm_hash: BytesN<32>) -> Address {
        storage::renew_governance_instance(&env);
        validate::require_nonzero_wasm_hash(&env, &wasm_hash);
        assert_with_error!(
            &env,
            !storage::has_price_aggregator(&env),
            GenericError::PoolAlreadyDeployed
        );

        let salt = BytesN::from_array(&env, &PRICE_AGGREGATOR_DEPLOY_SALT);
        let price_aggregator = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash.clone(), (env.current_contract_address(),));

        storage::set_price_aggregator(&env, &price_aggregator);

        DeployPriceAggregatorEvent {
            price_aggregator: price_aggregator.clone(),
            wasm_hash,
        }
        .publish(&env);

        price_aggregator
    }

    pub fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
    pub fn set_controller(env: Env, addr: Address) {
        storage::set_controller(&env, &addr);
    }

    pub fn set_price_aggregator(env: Env, addr: Address) {
        storage::set_price_aggregator(&env, &addr);
    }
}
