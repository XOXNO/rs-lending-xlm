//! Implements `GovernanceInterface` for `Governance`. Each method delegates
//! to a `pub(crate)` function in the module that owns the corresponding
//! behavior (`access`, `deploy`, or a `timelock::*` submodule); some methods
//! carry an `#[only_owner]` guard. Owner gating lives on these methods rather
//! than on the delegates, so the check runs at the contract entry point before
//! any state is touched.

use common::types::{AssetOracle, HubAssetKey, OracleTolerance, PriceKey};

use governance_interface::{AdminOperation, GovernanceInterface};

use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol, Val, Vec};

use stellar_governance::timelock::OperationState;
use stellar_macros::only_owner;

use crate::timelock::{immediate, lifecycle, recovery, views};
use crate::{access, deploy, storage, Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl GovernanceInterface for Governance {
    /// Deploys the controller contract from `wasm_hash` and records its
    /// address. Restricted to the owner. Panics if a controller is already
    /// deployed.
    #[only_owner]
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address {
        deploy::deploy_controller(&env, wasm_hash)
    }

    /// Returns the deployed controller's address.
    fn controller(env: Env) -> Address {
        storage::get_controller(&env)
    }

    /// Deploys the price aggregator contract from `wasm_hash`, records its
    /// address, and registers it with the controller if one is deployed.
    /// Restricted to the owner. Panics if a price aggregator is already
    /// deployed.
    #[only_owner]
    fn deploy_price_aggregator(env: Env, wasm_hash: BytesN<32>) -> Address {
        deploy::deploy_price_aggregator(&env, wasm_hash)
    }

    /// Returns the deployed price aggregator's address.
    fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    /// Executes a ready, non-expired scheduled op against `target` (not this
    /// contract). If `executor` is `Some`, requires that address to auth and
    /// hold `EXECUTOR_ROLE`; if `None`, no executor role check (anyone may
    /// drive execution of a ready op). Clears scheduled state on success.
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

    /// Cancels a pending operation. Requires the caller to hold the
    /// canceller role.
    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        lifecycle::cancel(&env, &canceller, &operation_id)
    }

    /// Returns the timelock's configured minimum delay, in ledgers.
    fn get_min_delay(env: Env) -> u32 {
        views::get_min_delay(&env)
    }

    /// Returns the current state of the operation identified by
    /// `operation_id`.
    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        views::get_operation_state(&env, &operation_id)
    }

    /// Returns the ledger at which the operation becomes ready (delay elapsed).
    /// Execution also requires the grace window and auth rules.
    fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32 {
        views::get_operation_ledger(&env, &operation_id)
    }

    /// Computes the operation id for the given target, function, arguments,
    /// predecessor, and salt.
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

    /// Validates `tolerance` and returns the resolved oracle tolerance
    /// bounds.
    fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance {
        views::resolve_oracle_tolerance(&env, tolerance)
    }

    /// Resolves `oracle` for `key`, filling in `asset_decimals` from the
    /// token contract for a `PriceKey::Token` key or `0` for `PriceKey::Ref`.
    fn resolve_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) -> AssetOracle {
        views::resolve_asset_oracle(&env, &key, &oracle)
    }

    /// Schedules `op` for later execution and returns its operation id.
    /// Requires the caller to hold the proposer role.
    fn propose(env: Env, proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32> {
        lifecycle::propose(&env, &proposer, &op, salt)
    }

    /// Pauses the controller. Requires the caller to hold the guardian
    /// role.
    fn pause(env: Env, caller: Address) {
        immediate::pause(&env, &caller)
    }

    /// Sets the paused and frozen flags for `hub_asset` in spoke
    /// `spoke_id`. Requires the caller to hold the guardian role.
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

    /// Sets the sanity-check price band for `key` on the price aggregator.
    /// Requires the caller to hold the oracle role.
    fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128) {
        immediate::set_sanity_band(&env, &caller, &key, min_wad, max_wad)
    }

    /// Creates a new hub on the controller and returns its id. Requires the
    /// caller to hold the guardian role.
    fn create_hub(env: Env, caller: Address) -> u32 {
        immediate::create_hub(&env, &caller)
    }

    /// Creates a new spoke on the controller and returns its id. Requires
    /// the caller to hold the guardian role.
    fn add_spoke(env: Env, caller: Address) -> u32 {
        immediate::add_spoke(&env, &caller)
    }

    /// Revokes `role` from `account` without going through the timelock.
    /// Restricted to the owner; only the guardian and oracle roles can be
    /// revoked this way.
    #[only_owner]
    fn revoke_role_immediate(env: Env, account: Address, role: Symbol) {
        immediate::revoke_role_immediate(&env, &account, &role)
    }

    /// Executes a ready, non-expired scheduled admin operation that targets this
    /// contract itself. If `executor` is `Some`, requires that address to auth
    /// and hold `EXECUTOR_ROLE`; if `None`, no executor role check.
    fn execute_self(env: Env, executor: Option<Address>, op: AdminOperation, salt: BytesN<32>) {
        lifecycle::execute_self(&env, executor, &op, salt)
    }

    /// Schedules a reset of the canceller role to `new_cancellers` and
    /// returns its operation id. Restricted to the owner.
    #[only_owner]
    fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        recovery::propose_canceller_reset(&env, &new_cancellers, salt)
    }

    /// Executes a ready, non-expired scheduled reset of the canceller role to
    /// `new_cancellers`. Same optional-executor auth rules as `execute`.
    fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) {
        recovery::execute_canceller_reset(&env, executor, &new_cancellers, salt)
    }

    /// Completes a pending ownership transfer to the caller.
    fn accept_ownership(env: Env) {
        access::accept_ownership(&env)
    }

    /// Returns whether `account` currently holds `role`.
    fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access::has_role(&env, &account, &role)
    }
}
