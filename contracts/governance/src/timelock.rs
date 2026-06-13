//! Production timelock surface over the OpenZeppelin `stellar-governance`
//! storage helpers.
//!
//! Governance owns its entrypoint surface rather than exposing the OZ generic
//! `Timelock::schedule`. The only way to queue an operation is through a typed
//! `propose_*` proposer in `forward.rs`, each of which validates its inputs and
//! builds an `Operation` targeting the controller. This module supplies the
//! generic `execute`/`cancel` lifecycle, the owner-gated `update_delay`, and the
//! read-only query views — all thin wrappers over the OZ storage free
//! functions, which carry the operation state machine, hashing, and events.
//!
//! Auth model: EXECUTOR gates an explicit-executor `execute` (open execution is
//! allowed with `executor: None`); CANCELLER gates `cancel`; `update_delay` is
//! owner-gated and immediate. Scheduled operations target the controller (a
//! governance->controller cross-call authorized by governance's ownership), so
//! `execute` never re-enters governance: Soroban prohibits contract self-reentry
//! and self-authorization, which is why governance-self-targeted admin stays
//! owner-immediate (see `access.rs`).

use controller_interface::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
};
use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol, Val, Vec};
use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, get_min_delay, get_operation_ledger, get_operation_state,
    hash_operation, set_min_delay, Operation, OperationState,
};
use stellar_macros::only_owner;

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE};
use crate::storage::renew_governance_instance;
use crate::{Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    /// Executes a ready operation, invoking the controller. When `executor` is
    /// `Some`, that address must hold EXECUTOR and authorize; `None` leaves
    /// execution open so anyone may push a ready op through after the delay.
    pub fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val {
        renew_governance_instance(&env);
        if let Some(ref exec) = executor {
            exec.require_auth();
            access_control::ensure_role(&env, &Symbol::new(&env, EXECUTOR_ROLE), exec);
        }
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        execute_operation(&env, &operation)
    }

    /// Cancels a pending operation. The caller must hold CANCELLER.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        cancel_operation(&env, &operation_id);
    }

    /// Sets the minimum timelock delay. Owner-gated and immediate: the delay
    /// governs governance itself, which cannot be timelocked (self-reentry is
    /// impossible), so shortening the delay is not itself delayed.
    #[only_owner]
    pub fn update_delay(env: Env, new_delay: u32) {
        renew_governance_instance(&env);
        set_min_delay(&env, new_delay);
    }

    /// Minimum timelock delay in ledgers.
    pub fn get_min_delay(env: Env) -> u32 {
        get_min_delay(&env)
    }

    /// Current lifecycle state of an operation.
    pub fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        get_operation_state(&env, &operation_id)
    }

    /// Ledger at which an operation becomes ready (`0` unset, `1` done).
    pub fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32 {
        get_operation_ledger(&env, &operation_id)
    }

    /// Deterministic operation id for the given fields.
    pub fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        hash_operation(&env, &operation)
    }

    /// Resolves a market oracle input to the `MarketOracleConfig` that
    /// `propose_configure_market_oracle` schedules for `set_market_oracle_config`.
    /// Runs the same validation and live oracle probes as the proposer, so the
    /// output equals the scheduled args exactly — the CLI replays it verbatim at
    /// `execute` time under simulation. Read-only.
    pub fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig {
        crate::forward::resolve_market_oracle(&env, &asset, &cfg)
    }

    /// Resolves tolerance BPS inputs to the `OraclePriceFluctuation` that
    /// `propose_edit_oracle_tolerance` schedules for `set_oracle_tolerance`. Same
    /// computation as the proposer, so the CLI replays the output verbatim at
    /// `execute` time. Read-only.
    pub fn resolve_oracle_tolerance(
        env: Env,
        first_tolerance: u32,
        last_tolerance: u32,
    ) -> OraclePriceFluctuation {
        crate::validate::tolerance::validate_and_calculate_tolerances(
            &env,
            first_tolerance,
            last_tolerance,
        )
    }
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, Symbol};
    use stellar_governance::timelock::OperationState;

    use controller_interface::types::{ControllerKey, PositionLimits};

    use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, PROPOSER_ROLE};
    use crate::constants::TIMELOCK_MIN_DELAY_LEDGERS;
    use crate::{Governance, GovernanceClient};

    const ZERO_SALT: [u8; 32] = [0u8; 32];

    fn register(env: &Env, min_delay: u32) -> (Address, GovernanceClient<'_>) {
        let admin = Address::generate(env);
        let gov_id = env.register(Governance, (admin.clone(), min_delay));
        (admin, GovernanceClient::new(env, &gov_id))
    }

    // Registers governance owning a native controller, with a short test delay.
    fn register_with_controller(
        env: &Env,
        min_delay: u32,
    ) -> (Address, Address, GovernanceClient<'_>) {
        let (admin, gov) = register(env, min_delay);
        let controller_id = env.register(controller::Controller, (gov.address.clone(),));
        gov.set_controller(&controller_id);
        (admin, controller_id, gov)
    }

    fn read_position_limits(env: &Env, controller_id: &Address) -> PositionLimits {
        env.as_contract(controller_id, || {
            env.storage()
                .instance()
                .get(&ControllerKey::PositionLimits)
                .expect("position limits set")
        })
    }

    #[test]
    fn constructor_grants_timelock_roles_to_admin() {
        let env = Env::default();
        let (admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        assert!(gov.has_role(&admin, &Symbol::new(&env, PROPOSER_ROLE)));
        assert!(gov.has_role(&admin, &Symbol::new(&env, EXECUTOR_ROLE)));
        assert!(gov.has_role(&admin, &Symbol::new(&env, CANCELLER_ROLE)));
    }

    #[test]
    fn constructor_sets_min_delay() {
        let env = Env::default();
        let (_admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        assert_eq!(gov.get_min_delay(), TIMELOCK_MIN_DELAY_LEDGERS);
    }

    #[test]
    fn get_operation_state_unknown_is_unset() {
        let env = Env::default();
        let (_admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        let unknown = BytesN::<32>::from_array(&env, &[7u8; 32]);
        assert_eq!(gov.get_operation_state(&unknown), OperationState::Unset);
    }

    // (a) A proposer schedules a controller-targeted op; with a non-zero delay
    // it sits in Waiting.
    #[test]
    fn propose_schedules_waiting_operation() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, _controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);

        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    }

    // (b) Execution before the delay elapses reverts (operation not Ready, OZ
    // InvalidOperationState #4002).
    #[test]
    #[should_panic(expected = "Error(Contract, #4002)")]
    fn execute_before_delay_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let _ = gov.propose_set_position_limits(&admin, &limits, &salt);

        gov.execute(
            &Some(admin.clone()),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );
    }

    // (c) After the delay, an EXECUTOR executes and the controller reflects the
    // change. Proves the real ledger-delay path end to end.
    #[test]
    fn execute_after_delay_applies_to_controller() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 6,
            max_borrow_positions: 3,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);
        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

        env.ledger().with_mut(|l| l.sequence_number += delay);
        assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

        gov.execute(
            &Some(admin.clone()),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );

        assert_eq!(gov.get_operation_state(&id), OperationState::Done);
        let stored = read_position_limits(&env, &controller);
        assert_eq!(stored.max_supply_positions, 6);
        assert_eq!(stored.max_borrow_positions, 3);
    }

    // (d) Validation runs at PROPOSE time: a zero position limit reverts when
    // scheduling, before anything is queued (InvalidPositionLimits #36).
    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn propose_rejects_bad_input_at_schedule_time() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);

        let limits = PositionLimits {
            max_supply_positions: 0,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_set_position_limits(&admin, &limits, &salt);
    }

    // (e) A CANCELLER cancels a pending op; its state returns to Unset.
    #[test]
    fn cancel_returns_operation_to_unset() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);
        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

        gov.cancel(&admin, &id);
        assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
    }

    // (f.1) A non-PROPOSER cannot schedule (missing-role revert).
    #[test]
    #[should_panic]
    fn non_proposer_cannot_propose() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, _controller, gov) = register_with_controller(&env, 10);
        let stranger = Address::generate(&env);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_set_position_limits(&stranger, &limits, &salt);
    }

    // (f.2) A non-EXECUTOR cannot execute even after the op is Ready.
    #[test]
    #[should_panic]
    fn non_executor_cannot_execute() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);
        let stranger = Address::generate(&env);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_set_position_limits(&admin, &limits, &salt);
        env.ledger().with_mut(|l| l.sequence_number += delay);

        gov.execute(
            &Some(stranger),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );
    }

    // (f.3) A non-CANCELLER cannot cancel a pending op.
    #[test]
    #[should_panic]
    fn non_canceller_cannot_cancel() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);
        let stranger = Address::generate(&env);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);

        gov.cancel(&stranger, &id);
    }

    // (g.1) The owner may shorten the delay immediately (owner-gated, not
    // timelocked).
    #[test]
    fn update_delay_by_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        gov.update_delay(&5u32);
        assert_eq!(gov.get_min_delay(), 5u32);
    }

    // (g.2) A non-owner cannot change the delay.
    #[test]
    #[should_panic]
    fn update_delay_by_non_owner_reverts() {
        let env = Env::default();
        let (_admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);
        let stranger = Address::generate(&env);

        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &stranger,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &gov.address,
                fn_name: "update_delay",
                args: (5u32,).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        gov.update_delay(&5u32);
    }
}
