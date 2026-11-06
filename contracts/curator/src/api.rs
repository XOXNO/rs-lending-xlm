//! Curator entrypoints. Each mutating call checks `caller`, then delegates.

use common::types::{HubAssetKey, MarketParamsRaw};
use governance_interface::AdminOperation;
use soroban_sdk::{contractimpl, Address, BytesN, Env};

use crate::storage;
use crate::{ops, CapGrant, Curator, CuratorArgs, CuratorClient};

#[contractimpl]
impl Curator {
    /// Replaces this contract's Wasm. Only the stored governance contract can
    /// authorize it, through the sensitive timelock.
    pub fn upgrade(env: Env, wasm_hash: BytesN<32>) {
        ops::upgrade(&env, &wasm_hash)
    }

    /// Stores the governance contract that owns this registry.
    pub fn __constructor(env: Env, governance: Address) {
        storage::init(&env, &governance);
    }

    /// Returns the governance contract this registry calls.
    pub fn governance(env: Env) -> Address {
        storage::governance(&env)
    }

    /// Returns the admin of `registrar`, suspended or not.
    pub fn registrar_admin(env: Env, registrar: u32) -> Option<Address> {
        storage::registrar_admin(&env, registrar)
    }

    /// Returns whether governance has suspended `registrar`.
    pub fn registrar_suspended(env: Env, registrar: u32) -> bool {
        storage::is_suspended(&env, registrar)
    }

    /// Suspends or resumes `registrar`. A suspended registrar cannot act and its
    /// cap grants stop admitting listings. Only the stored governance contract
    /// can authorize it, through the owner-proposed sensitive timelock.
    pub fn set_registrar_suspended(env: Env, registrar: u32, suspended: bool) {
        ops::set_registrar_suspended(&env, registrar, suspended)
    }

    /// Returns the stored role bits. The admin holds every bit even when this is zero.
    pub fn roles(env: Env, registrar: u32, account: Address) -> u32 {
        storage::role_bits(&env, registrar, &account)
    }

    /// Returns the registrar that owns `hub_id`.
    pub fn hub_registrar(env: Env, hub_id: u32) -> Option<u32> {
        storage::hub_registrar(&env, hub_id)
    }

    /// Returns the registrar that owns `spoke_id`.
    pub fn spoke_registrar(env: Env, spoke_id: u32) -> Option<u32> {
        storage::spoke_registrar(&env, spoke_id)
    }

    /// Returns the cap grant `grantee` holds for `asset` of `hub_id` in `spoke_id`.
    pub fn cap_grant(
        env: Env,
        grantee: u32,
        hub_id: u32,
        spoke_id: u32,
        asset: Address,
    ) -> Option<CapGrant> {
        storage::cap_grant(&env, grantee, hub_id, spoke_id, &asset).map(|(_, grant)| grant)
    }

    /// Returns the proposing registrar and the hub of a queued curator proposal.
    pub fn pending_proposal(env: Env, op_id: BytesN<32>) -> Option<(u32, u32)> {
        storage::pending(&env, &op_id)
    }

    /// Replaces the admin of `registrar`, suspended or not. Only the stored
    /// governance contract can authorize it, through the owner-proposed
    /// sensitive timelock.
    pub fn force_set_admin(env: Env, registrar: u32, admin: Address) {
        ops::force_set_admin(&env, registrar, &admin)
    }

    /// Binds `hub_id` to `registrar`, or unbinds it with `None`. Only the stored
    /// governance contract can authorize it, through the owner-proposed
    /// sensitive timelock.
    pub fn reassign_hub(env: Env, hub_id: u32, registrar: Option<u32>) {
        ops::reassign_hub(&env, hub_id, registrar)
    }

    /// Binds `spoke_id` to `registrar`, or unbinds it with `None`. Only the
    /// stored governance contract can authorize it, through the owner-proposed
    /// sensitive timelock.
    pub fn reassign_spoke(env: Env, spoke_id: u32, registrar: Option<u32>) {
        ops::reassign_spoke(&env, spoke_id, registrar)
    }

    /// Opens a registrar run by `admin` and returns its id. Only the stored
    /// governance contract can authorize it, through the owner-proposed
    /// sensitive timelock.
    pub fn register_registrar(env: Env, admin: Address) -> u32 {
        ops::register_registrar(&env, &admin)
    }

    /// Replaces the registrar admin.
    pub fn set_admin(env: Env, caller: Address, registrar: u32, new_admin: Address) {
        ops::set_admin(&env, &caller, registrar, &new_admin)
    }

    /// Replaces `account`'s role bits. Zero clears them.
    pub fn set_roles(env: Env, caller: Address, registrar: u32, account: Address, bits: u32) {
        ops::set_roles(&env, &caller, registrar, &account, bits)
    }

    /// Creates a hub and binds the returned id to `registrar`.
    pub fn create_hub(env: Env, caller: Address, registrar: u32) -> u32 {
        ops::create_hub(&env, &caller, registrar)
    }

    /// Creates a spoke and binds the returned id to `registrar`.
    pub fn add_spoke(env: Env, caller: Address, registrar: u32) -> u32 {
        ops::add_spoke(&env, &caller, registrar)
    }

    /// Registers `params.asset_id` on the existing pool, under a hub `registrar` owns.
    pub fn create_market(
        env: Env,
        caller: Address,
        registrar: u32,
        hub_id: u32,
        params: MarketParamsRaw,
    ) -> Address {
        ops::create_market(&env, &caller, registrar, hub_id, &params)
    }

    /// Decreases a cap on a spoke `registrar` owns. Every other listing field is kept.
    pub fn lower_caps(
        env: Env,
        caller: Address,
        registrar: u32,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        supply_cap: i128,
        borrow_cap: i128,
    ) {
        ops::lower_caps(
            &env, &caller, registrar, spoke_id, &hub_asset, supply_cap, borrow_cap,
        )
    }

    /// Raises halt flags on a spoke `registrar` owns.
    pub fn tighten_flags(
        env: Env,
        caller: Address,
        registrar: u32,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
        no_seize: bool,
    ) {
        ops::tighten_flags(
            &env, &caller, registrar, spoke_id, &hub_asset, paused, frozen, no_seize,
        )
    }

    /// Stores the cap `grantee` may queue in its `spoke_id` against a hub `registrar` owns.
    pub fn set_cap_grant(
        env: Env,
        caller: Address,
        registrar: u32,
        grantee: u32,
        hub_id: u32,
        spoke_id: u32,
        asset: Address,
        grant: CapGrant,
    ) {
        ops::set_cap_grant(
            &env, &caller, registrar, grantee, hub_id, spoke_id, &asset, &grant,
        )
    }

    /// Deletes the grant `grantee` holds for a hub `registrar` owns.
    pub fn revoke_cap_grant(
        env: Env,
        caller: Address,
        registrar: u32,
        grantee: u32,
        hub_id: u32,
        spoke_id: u32,
        asset: Address,
    ) {
        ops::revoke_cap_grant(&env, &caller, registrar, grantee, hub_id, spoke_id, &asset)
    }

    /// Sets both caps of a listing to zero and freezes it. `registrar` must own
    /// the spoke or the listed hub; a hub owner also drops the grant behind it.
    pub fn close_listing(
        env: Env,
        caller: Address,
        registrar: u32,
        spoke_id: u32,
        hub_asset: HubAssetKey,
    ) {
        ops::close_listing(&env, &caller, registrar, spoke_id, &hub_asset)
    }

    /// Cancels a queued proposal of a suspended registrar. Anyone can call it.
    pub fn cancel_suspended(env: Env, op_id: BytesN<32>) {
        ops::cancel_suspended(&env, &op_id)
    }

    /// Cancels another registrar's queued proposal against a hub `registrar` owns.
    pub fn cancel_grantee_proposal(env: Env, caller: Address, registrar: u32, op_id: BytesN<32>) {
        ops::cancel_grantee_proposal(&env, &caller, registrar, &op_id)
    }

    /// Queues a new listing, a listing edit, a rate change, or a flag relaxation.
    pub fn propose(
        env: Env,
        caller: Address,
        registrar: u32,
        op: AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        ops::propose(&env, &caller, registrar, op, salt)
    }
}
