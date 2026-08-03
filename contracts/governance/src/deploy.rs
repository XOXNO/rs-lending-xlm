use common::errors::GenericError;

use soroban_sdk::{
    assert_with_error, contractimpl, vec, Address, BytesN, Env, IntoVal, Symbol, Val,
};

use stellar_macros::only_owner;

use crate::events::{DeployControllerEvent, DeployPriceAggregatorEvent};
use crate::validate;
use crate::{storage, Governance, GovernanceArgs, GovernanceClient};

const CONTROLLER_DEPLOY_SALT: [u8; 32] = [0u8; 32];

const PRICE_AGGREGATOR_DEPLOY_SALT: [u8; 32] = [1u8; 32];

#[contractimpl]
impl Governance {
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
