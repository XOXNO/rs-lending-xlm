//! One-shot owner-gated deploy of controller and price-aggregator. Stores
//! addresses and wires the aggregator into the controller when both exist.

use common::errors::GenericError;

use soroban_sdk::{assert_with_error, contractimpl, vec, Address, BytesN, Env, IntoVal, Symbol, Val};

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
    /// Deploys the lending controller once and records its address. Owner only.
    /// Governance is the controller constructor admin.
    ///
    /// # Errors
    /// * `InvalidWasmHash` — `wasm_hash` is all-zero.
    /// * `PoolAlreadyDeployed` — controller address already stored.
    ///
    /// # Events
    /// * `DeployControllerEvent` — address and wasm hash.
    ///
    /// # Security Warning
    /// * Governance holds every controller admin power after deploy.
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

    /// Stored controller address.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — controller not deployed.
    pub fn controller(env: Env) -> Address {
        storage::get_controller(&env)
    }

    /// Deploys the price-aggregator once and records its address. Owner only.
    /// Governance is the aggregator constructor owner; if a controller exists,
    /// wires it immediately (Sensitive re-point still uses `SetPriceAggregator`).
    ///
    /// # Errors
    /// * `InvalidWasmHash` — `wasm_hash` is all-zero.
    /// * `PoolAlreadyDeployed` — aggregator address already stored.
    ///
    /// # Events
    /// * `DeployPriceAggregatorEvent` — address and wasm hash.
    ///
    /// # Security Warning
    /// * Governance holds every oracle admin power after deploy.
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

        // Bootstrap wiring: point the controller at the freshly deployed oracle
        // authority atomically. Both contracts are governance-owned and the
        // controller exists by this deploy step, so the one-shot owner call
        // needs no timelock. Re-pointing a LIVE aggregator still rides the
        // Sensitive-tier `SetPriceAggregator` self-op; this only covers the
        // one-time initial set (the controller has no aggregator yet).
        if storage::has_controller(&env) {
            env.invoke_contract::<Val>(
                &storage::get_controller(&env),
                &Symbol::new(&env, "set_price_aggregator"),
                vec![&env, price_aggregator.clone().into_val(&env)],
            );
        }

        DeployPriceAggregatorEvent {
            price_aggregator: price_aggregator.clone(),
            wasm_hash,
        }
        .publish(&env);

        price_aggregator
    }

    /// Stored price-aggregator address.
    ///
    /// # Errors
    /// * `AggregatorNotSet` — aggregator not deployed.
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
