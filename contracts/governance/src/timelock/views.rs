//! Read-only timelock and oracle-config views. No auth, no state change.

use common::types::{AssetOracle, OracleTolerance, PriceKey};

use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol, Val, Vec};

use stellar_governance::timelock::{
    get_min_delay, get_operation_ledger, get_operation_state, hash_operation, Operation,
    OperationState,
};

use crate::{validate, Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    /// Minimum timelock delay in ledgers.
    pub fn get_min_delay(env: Env) -> u32 {
        get_min_delay(&env)
    }

    /// Lifecycle state of a scheduled operation.
    pub fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        get_operation_state(&env, &operation_id)
    }

    /// Ledger when an operation becomes ready (`0` when unset or not pending).
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

    /// Builds the [`OracleTolerance`] band that `propose` would schedule for
    /// the given BPS value.
    ///
    /// # Errors
    /// * [`common::errors::OracleError::BadLastTolerance`] — outside allowed BPS.
    /// * [`common::errors::GenericError::MathOverflow`] — band math overflows.
    pub fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance {
        validate::tolerance::validate_and_calculate_tolerances(&env, tolerance)
    }

    /// Builds the [`AssetOracle`] that a `ConfigureAssetOracle` proposal would
    /// schedule, including governance-derived `asset_decimals`.
    ///
    /// # Errors
    /// * [`common::errors::GenericError::InvalidAsset`] — token key whose
    ///   address is not a readable SAC.
    pub fn resolve_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) -> AssetOracle {
        crate::op::resolve_asset_oracle(&env, &key, &oracle)
    }
}
