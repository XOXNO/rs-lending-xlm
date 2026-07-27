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

    /// Ledger when an operation becomes ready (`0` when unset / not pending).
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

    /// Resolves tolerance BPS to the `OracleTolerance` band `propose` would
    /// schedule. Read-only.
    ///
    /// # Errors
    /// * `BadLastTolerance` — outside allowed BPS range.
    /// * `MathOverflow` — band computation overflows.
    pub fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance {
        validate::tolerance::validate_and_calculate_tolerances(&env, tolerance)
    }

    /// Resolves an oracle proposal to the exact [`AssetOracle`] a
    /// `ConfigureAssetOracle` proposal would schedule. Read-only.
    ///
    /// This is what a CLI dry run (`--send=no`) invokes: governance overrides
    /// `asset_decimals` from the token, so the struct a proposer submits is not
    /// the struct that gets stored, and reviewing the submitted one would review
    /// the wrong thing.
    ///
    /// # Errors
    /// * `InvalidAsset` — `PriceKey::Token` whose address is not a readable
    ///   token contract.
    pub fn resolve_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) -> AssetOracle {
        crate::op::resolve_asset_oracle(&env, &key, &oracle)
    }
}
