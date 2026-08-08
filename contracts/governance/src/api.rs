//! The contract's exported surface, as a single `GovernanceInterface` impl.
//!
//! Every method here is a one-line delegation to a `pub(crate)` function in the
//! module that owns the behaviour (`access`, `deploy`, `timelock::*`). The
//! indirection buys compile-time enforcement: a Soroban trait impl must be one
//! block, so binding the published interface to the contract requires a single
//! block, while the implementation stays split by concern.
//!
//! Owner gating lives on these methods rather than on the delegates, so the
//! check runs at the entrypoint before any state is touched.

use common::types::{AssetOracle, HubAssetKey, OracleTolerance, PriceKey};

use governance_interface::{AdminOperation, GovernanceInterface};

use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol, Val, Vec};

use stellar_governance::timelock::OperationState;
use stellar_macros::only_owner;

use crate::timelock::{immediate, lifecycle, recovery, views};
use crate::{access, deploy, storage, Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl GovernanceInterface for Governance {
    #[only_owner]
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address {
        deploy::deploy_controller(&env, wasm_hash)
    }

    fn controller(env: Env) -> Address {
        storage::get_controller(&env)
    }

    #[only_owner]
    fn deploy_price_aggregator(env: Env, wasm_hash: BytesN<32>) -> Address {
        deploy::deploy_price_aggregator(&env, wasm_hash)
    }

    fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val {
        lifecycle::execute(&env, executor, target, function, args, predecessor, salt)
    }

    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        lifecycle::cancel(&env, &canceller, &operation_id)
    }

    fn get_min_delay(env: Env) -> u32 {
        views::get_min_delay(&env)
    }

    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        views::get_operation_state(&env, &operation_id)
    }

    fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32 {
        views::get_operation_ledger(&env, &operation_id)
    }

    fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        views::hash_operation(&env, target, function, args, predecessor, salt)
    }

    fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance {
        views::resolve_oracle_tolerance(&env, tolerance)
    }

    fn resolve_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) -> AssetOracle {
        views::resolve_asset_oracle(&env, &key, &oracle)
    }

    fn propose(env: Env, proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32> {
        lifecycle::propose(&env, &proposer, &op, salt)
    }

    fn pause(env: Env, caller: Address) {
        immediate::pause(&env, &caller)
    }

    fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    ) {
        immediate::set_spoke_asset_flags(&env, &caller, spoke_id, &hub_asset, paused, frozen)
    }

    fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128) {
        immediate::set_sanity_band(&env, &caller, &key, min_wad, max_wad)
    }

    fn create_hub(env: Env, caller: Address) -> u32 {
        immediate::create_hub(&env, &caller)
    }

    fn add_spoke(env: Env, caller: Address) -> u32 {
        immediate::add_spoke(&env, &caller)
    }

    #[only_owner]
    fn revoke_role_immediate(env: Env, account: Address, role: Symbol) {
        immediate::revoke_role_immediate(&env, &account, &role)
    }

    fn execute_self(env: Env, executor: Option<Address>, op: AdminOperation, salt: BytesN<32>) {
        lifecycle::execute_self(&env, executor, &op, salt)
    }

    #[only_owner]
    fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        recovery::propose_canceller_reset(&env, &new_cancellers, salt)
    }

    fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) {
        recovery::execute_canceller_reset(&env, executor, &new_cancellers, salt)
    }

    fn accept_ownership(env: Env) {
        access::accept_ownership(&env)
    }

    fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access::has_role(&env, &account, &role)
    }
}
