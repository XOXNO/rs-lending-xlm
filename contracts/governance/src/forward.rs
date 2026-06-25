//! Timelocked controller-admin proposals and immediate pause forwarders.
//!
//! Controller-admin operations are queued through the generic `propose`
//! entrypoint. The proposer must hold PROPOSER, inputs are validated before
//! scheduling, and the queued `Operation` targets a controller owner-gated
//! setter.
//!
//! `pause` and `unpause` are owner-gated immediate calls for emergency response.
//!
//! Testing builds include an `execute_immediate` forwarder for single-frame
//! harness setup.

use controller_interface::types::MarketOracleConfigInput;
use controller_interface::ControllerAdminClient;
use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol};
#[cfg(any(test, feature = "testing"))]
use soroban_sdk::{IntoVal, Val};
use stellar_access::access_control;
use stellar_governance::timelock::{schedule_operation, Operation};
use stellar_macros::only_owner;

use crate::access::PROPOSER_ROLE;
use crate::timelock::operation_delay;
use crate::{storage, validate, Governance, GovernanceArgs, GovernanceClient};

fn controller_client(env: &Env) -> ControllerAdminClient<'_> {
    ControllerAdminClient::new(env, &storage::get_controller(env))
}

/// Shared proposal preamble: renew instance TTL, authorize the proposer, and
/// require PROPOSER.
fn begin_proposal(env: &Env, proposer: &Address) {
    storage::renew_governance_instance(env);
    proposer.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, PROPOSER_ROLE), proposer);
}

/// Resolves the `MarketOracleConfig` persisted by `set_market_oracle_config`.
/// Shared by the proposer and view so simulation can replay the scheduled args.
pub(crate) fn resolve_market_oracle(
    env: &Env,
    asset: &Address,
    cfg: &MarketOracleConfigInput,
) -> controller_interface::types::MarketOracleConfig {
    let tolerance = validate::tolerance::validate_and_calculate_tolerances(
        env,
        cfg.first_tolerance_bps,
        cfg.last_tolerance_bps,
    );
    let controller = storage::get_controller(env);
    validate::oracle_probe::validate_market_oracle_sources(env, &controller, asset, cfg, tolerance)
}

#[contractimpl]
impl Governance {
    pub fn propose(
        env: Env,
        proposer: Address,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        let (target, function, args, delay_tier) = crate::op::resolve_op(&env, &op);
        let delay = operation_delay(&env, delay_tier);
        let operation = Operation {
            target,
            function,
            args,
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        schedule_operation(&env, &operation, delay)
    }
}

#[contractimpl]
impl Governance {
    /// Halt the controller immediately; owner-gated and not timelocked.
    #[only_owner]
    pub fn pause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).pause();
    }

    /// Resume the controller immediately; owner-gated.
    #[only_owner]
    pub fn unpause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).unpause();
    }
}

/// Forwarder compiled for tests and the `testing` feature. Excluded from
/// production wasm; production configuration uses timelocked `propose`
/// entrypoint.
#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
    pub fn execute_immediate(env: Env, caller: Address, op: crate::op::AdminOperation) -> Val {
        storage::renew_governance_instance(&env);
        match &op {
            crate::op::AdminOperation::ConfigureMarketOracle(_)
            | crate::op::AdminOperation::EditOracleTolerance(_) => {
                caller.require_auth();
                stellar_access::access_control::ensure_role(
                    &env,
                    &Symbol::new(&env, crate::access::ORACLE_ROLE),
                    &caller,
                );
            }
            _ => {
                caller.require_auth();
                let owner = stellar_access::ownable::get_owner(&env)
                    .unwrap_or_else(|| panic!("Owner not set"));
                assert_eq!(caller, owner, "not owner");
            }
        }
        let (target, function, args, _) = crate::op::resolve_op(&env, &op);
        if target == env.current_contract_address() {
            crate::op::apply_self_op(&env, &op);
            ().into_val(&env)
        } else {
            env.invoke_contract(&target, &function, args)
        }
    }
}
