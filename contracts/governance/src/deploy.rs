//! One-time controller deployment and address lookup.

use common::errors::GenericError;
use soroban_sdk::{assert_with_error, contractimpl, Address, BytesN, Env};
use stellar_macros::only_owner;

use crate::events::DeployControllerEvent;
use crate::{storage, Governance, GovernanceArgs, GovernanceClient};

/// Deterministic salt for the one-time controller deployment; the controller
/// address derives from (governance address, salt).
const CONTROLLER_DEPLOY_SALT: [u8; 32] = [0u8; 32];

#[contractimpl]
impl Governance {
    /// One-time deployment of the controller owned by this contract; the
    /// governance address is the controller's constructor admin. Reuses
    /// `PoolAlreadyDeployed` to guard repeat deployments.
    #[only_owner]
    pub fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address {
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

    /// Returns the deployed controller; panics `PoolNotInitialized` when unset.
    pub fn controller(env: Env) -> Address {
        storage::get_controller(&env)
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
    pub fn set_controller(env: Env, addr: Address) {
        storage::set_controller(&env, &addr);
    }
}
