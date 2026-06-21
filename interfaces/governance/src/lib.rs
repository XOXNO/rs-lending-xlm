#![no_std]
#![allow(clippy::too_many_arguments)]

//! Published ABI for the governance contract.
//!
//! Mirrors the governance contract's public entrypoint surface for typed
//! cross-contract and off-chain callers. Client-only: `#[contractclient]`
//! generates `GovernanceClient`; the governance contract does NOT formally
//! `impl` this trait — its entrypoints match by ABI name (its production
//! methods span the `deploy`, `timelock`, `forward`, `self_timelock`, and
//! `access` modules), the same convention `controller-interface` uses. The
//! testing-only immediate forwarders, `set_controller`, and `__constructor` are
//! excluded: constructors are not trait methods (clients call via `register`),
//! and the forwarders exist only under the contract's `testing` feature.

use common::types::{InterestRateModel, MarketParamsRaw};
use controller_interface::types::{
    AssetConfigRaw, MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
    PositionLimits,
};
use soroban_sdk::{contractclient, Address, BytesN, Env, Symbol, Val, Vec};
use stellar_governance::timelock::OperationState;

pub use stellar_governance::timelock::OperationState as GovernanceOperationState;

#[contractclient(name = "GovernanceClient")]
/// Governance contract interface: controller deployment, the timelock
/// lifecycle, the typed controller-admin proposers, and governance-self admin.
pub trait GovernanceInterface {
    // --- deploy.rs: one-time controller deployment and lookup ---

    /// One-time controller deployment; returns the deployed controller address.
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address;

    /// Returns the deployed controller address.
    fn controller(env: Env) -> Address;

    // --- timelock.rs: generic lifecycle, delay setter, and read views ---

    /// Executes a ready operation, invoking the controller; `Some(executor)`
    /// gates on EXECUTOR, `None` leaves execution open.
    fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val;

    /// Cancels a pending operation; the caller must hold CANCELLER.
    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>);

    /// Schedules a governance self-upgrade.
    fn propose_governance_upgrade(
        env: Env,
        proposer: Address,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a scheduled governance self-upgrade.
    fn execute_governance_upgrade(
        env: Env,
        executor: Option<Address>,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    );

    /// Schedules a minimum timelock delay update (monotonic in production).
    fn propose_update_delay(
        env: Env,
        proposer: Address,
        new_delay: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a scheduled minimum timelock delay update.
    fn execute_update_delay(env: Env, executor: Option<Address>, new_delay: u32, salt: BytesN<32>);

    /// Schedules a governance role grant.
    fn propose_grant_governance_role(
        env: Env,
        proposer: Address,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a scheduled governance role grant.
    fn execute_grant_governance_role(
        env: Env,
        executor: Option<Address>,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    );

    /// Schedules a governance role revocation.
    fn propose_revoke_governance_role(
        env: Env,
        proposer: Address,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a scheduled governance role revocation.
    fn execute_revoke_governance_role(
        env: Env,
        executor: Option<Address>,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    );

    /// Schedules initiation of governance ownership transfer.
    fn propose_transfer_gov_own(
        env: Env,
        proposer: Address,
        new_owner: Address,
        live_until_ledger: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a scheduled governance ownership transfer initiation.
    fn execute_transfer_gov_own(
        env: Env,
        executor: Option<Address>,
        new_owner: Address,
        live_until_ledger: u32,
        salt: BytesN<32>,
    );

    /// Minimum timelock delay in ledgers.
    fn get_min_delay(env: Env) -> u32;

    /// Current lifecycle state of an operation.
    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState;

    /// Ledger at which an operation becomes ready (`0` unset, `1` done).
    fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32;

    /// Deterministic operation id for the given fields.
    fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Resolves a market oracle input to the `MarketOracleConfig` the matching
    /// proposer schedules; read-only.
    fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig;

    /// Resolves tolerance BPS inputs to the `OraclePriceFluctuation` the
    /// matching proposer schedules; read-only.
    fn resolve_oracle_tolerance(
        env: Env,
        first_tolerance: u32,
        last_tolerance: u32,
    ) -> OraclePriceFluctuation;

    // --- forward.rs: typed controller-admin proposers ---

    /// Schedules `set_aggregator`.
    fn propose_set_aggregator(
        env: Env,
        proposer: Address,
        addr: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `set_accumulator`.
    fn propose_set_accumulator(
        env: Env,
        proposer: Address,
        addr: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `set_liquidity_pool_template`.
    fn propose_set_pool_template(
        env: Env,
        proposer: Address,
        hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `edit_asset_config`.
    fn propose_edit_asset_config(
        env: Env,
        proposer: Address,
        asset: Address,
        cfg: AssetConfigRaw,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `set_position_limits`.
    fn propose_set_position_limits(
        env: Env,
        proposer: Address,
        limits: PositionLimits,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `set_min_borrow_collateral_usd`.
    fn propose_set_min_borrow_collat(
        env: Env,
        proposer: Address,
        floor_wad: i128,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `add_e_mode_category`.
    fn propose_add_e_mode_category(
        env: Env,
        proposer: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `remove_e_mode_category`.
    fn propose_remove_e_mode_category(
        env: Env,
        proposer: Address,
        id: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `add_asset_to_e_mode_category`.
    fn propose_add_asset_to_e_mode(
        env: Env,
        proposer: Address,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        ltv: u32,
        threshold: u32,
        bonus: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `edit_asset_in_e_mode_category`.
    fn propose_edit_asset_in_e_mode(
        env: Env,
        proposer: Address,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        ltv: u32,
        threshold: u32,
        bonus: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `remove_asset_from_e_mode`.
    fn propose_remove_asset_from_e_mode(
        env: Env,
        proposer: Address,
        asset: Address,
        category_id: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `approve_token`.
    fn propose_approve_token(
        env: Env,
        proposer: Address,
        token: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `revoke_token`.
    fn propose_revoke_token(
        env: Env,
        proposer: Address,
        token: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `approve_blend_pool`.
    fn propose_approve_blend_pool(
        env: Env,
        proposer: Address,
        pool: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `revoke_blend_pool`.
    fn propose_revoke_blend_pool(
        env: Env,
        proposer: Address,
        pool: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `create_liquidity_pool`.
    fn propose_create_liquidity_pool(
        env: Env,
        proposer: Address,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `upgrade_liquidity_pool_params`.
    fn propose_upgrade_pool_params(
        env: Env,
        proposer: Address,
        asset: Address,
        params: InterestRateModel,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `deploy_pool`.
    fn propose_deploy_pool(env: Env, proposer: Address, salt: BytesN<32>) -> BytesN<32>;

    /// Schedules `upgrade_pool`.
    fn propose_upgrade_pool(
        env: Env,
        proposer: Address,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules controller `disable_token_oracle`.
    fn propose_disable_token_oracle(
        env: Env,
        proposer: Address,
        asset: Address,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules controller `upgrade`.
    fn propose_upgrade_controller(
        env: Env,
        proposer: Address,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules controller `migrate`.
    fn propose_migrate_controller(
        env: Env,
        proposer: Address,
        new_version: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules controller `transfer_ownership`.
    fn propose_transfer_ctrl_ownership(
        env: Env,
        proposer: Address,
        new_owner: Address,
        live_until_ledger: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `set_market_oracle_config` from a validated oracle input.
    fn propose_configure_market_oracle(
        env: Env,
        proposer: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Schedules `set_oracle_tolerance` from validated tolerance inputs.
    fn propose_edit_oracle_tolerance(
        env: Env,
        proposer: Address,
        asset: Address,
        first_tolerance: u32,
        last_tolerance: u32,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Emergency brake: halts the controller immediately, owner-gated.
    fn pause(env: Env);

    /// Resumes the controller, owner-gated and immediate.
    fn unpause(env: Env);

    // --- access.rs / self_timelock.rs: governance-self administration ---

    /// Accepts a pending ownership transfer of the governance contract.
    fn accept_ownership(env: Env);

    /// Returns whether `account` holds `role` on the governance contract.
    fn has_role(env: Env, account: Address, role: Symbol) -> bool;
}
