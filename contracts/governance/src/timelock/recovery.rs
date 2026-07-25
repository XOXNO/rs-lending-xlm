//! Recovery tier: the non-vetoable canceller reset.
//!
//! Longest delay of any path and cancellers cannot veto it, so this is the
//! escape hatch when the canceller set itself is captured. See ADR 0010.

use soroban_sdk::{contractimpl, Address, BytesN, Env, Vec};

use stellar_governance::timelock::{schedule_operation, set_execute_operation};
use stellar_macros::only_owner;

use crate::access::{self};
use crate::timelock::*;
use crate::{storage, Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    /// Schedules a non-vetoable canceller-council reset at Recovery delay. Owner only.
    ///
    /// # Errors
    /// * Owner gate via `#[only_owner]`.
    /// * OZ timelock rejects duplicate schedule.
    ///
    /// # Events
    /// * OZ timelock schedule event.
    #[only_owner]
    pub fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let operation = canceller_reset_operation(&env, &new_cancellers, salt);
        let delay = operation_delay(&env, DelayTier::Recovery);
        let id = schedule_operation(&env, &operation, delay);
        storage::mark_recovery_op(&env, &id);
        id
    }

    /// Executes a matured canceller-council reset. `Some(executor)` requires
    /// `EXECUTOR`; `None` leaves execution open.
    ///
    /// # Errors
    /// * `TimelockOperationExpired` — past grace window.
    /// * `InvalidRole` — EXECUTOR/CANCELLER overlap on a non-owner grant.
    /// * OZ not-scheduled / not-ready; `EXECUTOR` gate when set.
    ///
    /// # Events
    /// * OZ timelock execute event; access-control role grant/revoke events.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready reset.
    pub fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) {
        let operation = canceller_reset_operation(&env, &new_cancellers, salt);
        let operation_id = prepare_execute(&env, executor.as_ref(), &operation);
        set_execute_operation(&env, &operation);
        access::apply_canceller_reset(&env, &new_cancellers);
        finish_execute(&env, &operation_id);
    }
}
