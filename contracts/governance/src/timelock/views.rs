use common::types::{AssetOracle, OracleTolerance, PriceKey};

use soroban_sdk::{Address, BytesN, Env, Symbol, Val, Vec};

use stellar_governance::timelock::{self as gov_timelock, Operation, OperationState};

use crate::validate;

pub(crate) fn get_min_delay(env: &Env) -> u32 {
    gov_timelock::get_min_delay(env)
}

pub(crate) fn get_operation_state(env: &Env, operation_id: &BytesN<32>) -> OperationState {
    gov_timelock::get_operation_state(env, operation_id)
}

pub(crate) fn get_operation_ledger(env: &Env, operation_id: &BytesN<32>) -> u32 {
    gov_timelock::get_operation_ledger(env, operation_id)
}

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

pub(crate) fn resolve_oracle_tolerance(env: &Env, tolerance: u32) -> OracleTolerance {
    validate::tolerance::validate_and_calculate_tolerances(env, tolerance)
}

pub(crate) fn resolve_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) -> AssetOracle {
    crate::op::resolve_oracle(env, key, oracle)
}
