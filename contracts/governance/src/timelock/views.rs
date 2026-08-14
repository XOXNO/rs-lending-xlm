//! Read-only queries over timelock state and helpers that resolve caller-supplied
//! operation parameters into their canonical, validated form without mutating
//! contract state.

use common::types::{AssetOracle, OracleTolerance, PriceKey};

use soroban_sdk::{Address, BytesN, Env, Symbol, Val, Vec};

use stellar_governance::timelock::{self as gov_timelock, Operation, OperationState};

use crate::validate;

/// Returns the timelock's current minimum delay, in ledgers.
pub(crate) fn get_min_delay(env: &Env) -> u32 {
    gov_timelock::get_min_delay(env)
}

/// Returns the current state of the operation identified by `operation_id`.
pub(crate) fn get_operation_state(env: &Env, operation_id: &BytesN<32>) -> OperationState {
    gov_timelock::get_operation_state(env, operation_id)
}

/// Returns the ledger sequence at which the operation identified by
/// `operation_id` becomes ready for execution, or 0 if it was never scheduled
/// or 1 if it has already been executed.
pub(crate) fn get_operation_ledger(env: &Env, operation_id: &BytesN<32>) -> u32 {
    gov_timelock::get_operation_ledger(env, operation_id)
}

/// Computes the operation id that would result from scheduling an operation with
/// the given target, function, args, predecessor, and salt.
pub(crate) fn hash_operation(
    env: &Env,
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
    gov_timelock::hash_operation(env, &operation)
}

/// Validates `tolerance` and derives the corresponding upper and lower oracle
/// tolerance ratios, in basis points.
pub(crate) fn resolve_oracle_tolerance(env: &Env, tolerance: u32) -> OracleTolerance {
    validate::tolerance::validate_and_calculate_tolerances(env, tolerance)
}

/// Resolves `oracle` for `key`, filling in the asset's decimals: fetched from the
/// token contract for `PriceKey::Token`, or 0 for `PriceKey::Ref`.
pub(crate) fn resolve_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) -> AssetOracle {
    crate::op::resolve_oracle(env, key, oracle)
}
