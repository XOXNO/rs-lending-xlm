//! Governance actions that execute immediately, bypassing the timelock delay.
//! Each entry point except `revoke_role_immediate` authenticates the caller and
//! checks a required role through `begin_immediate` before forwarding to the
//! controller or price aggregator. `set_spoke_asset_flags` accepts the guardian
//! or curator role directly.
//! `revoke_role_immediate` is instead owner-gated at the contract entry point
//! (`#[only_owner]`) and only restricts which role argument may be revoked.

use common::errors::GenericError;
use common::ttl::renew_instance;
use common::types::{HubAssetKey, MarketParamsRaw, PriceKey};

use soroban_sdk::{assert_with_error, vec, Address, Env, IntoVal, Symbol};

use stellar_access::access_control;

use crate::access::{self, CURATOR_ROLE, GUARDIAN_ROLE, ORACLE_ROLE};
use crate::timelock::*;
use crate::validate;

/// Pauses the controller. Requires the caller to hold `GUARDIAN_ROLE`.
pub(crate) fn pause(env: &Env, caller: &Address) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).pause();
}

/// Tightens the paused, frozen, and no-seize flags for `hub_asset` in the given spoke.
/// Requires `GUARDIAN_ROLE` or `CURATOR_ROLE`. The controller rejects clearing a set
/// flag with `SpokeAssetFlagRelaxation` (INV-AUTH-04).
pub(crate) fn set_spoke_asset_flags(
    env: &Env,
    caller: &Address,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    renew_instance(env);
    caller.require_auth();
    if access_control::has_role(env, caller, &Symbol::new(env, CURATOR_ROLE)).is_none() {
        access_control::ensure_role(env, &Symbol::new(env, GUARDIAN_ROLE), caller);
    }
    controller_client(env).set_spoke_asset_flags(&spoke_id, hub_asset, &paused, &frozen, &no_seize);
}

/// Tightens the sanity-check price band (min/max, WAD) for `key` on the price
/// aggregator. Requires the caller to hold `ORACLE_ROLE`. The aggregator rejects
/// a wider band with `SanityBandMustTighten`.
pub(crate) fn set_sanity_band(
    env: &Env,
    caller: &Address,
    key: &PriceKey,
    min_wad: i128,
    max_wad: i128,
) {
    begin_immediate(env, caller, ORACLE_ROLE);
    price_aggregator_client(env).set_sanity_band(key, &min_wad, &max_wad);
}

/// Creates a new hub on the controller and returns its id. Requires the caller to
/// hold `CURATOR_ROLE`.
pub(crate) fn create_hub(env: &Env, caller: &Address) -> u32 {
    begin_immediate(env, caller, CURATOR_ROLE);
    controller_client(env).create_hub()
}

/// Creates a new spoke on the controller and returns its id. Requires the caller
/// to hold `CURATOR_ROLE`.
pub(crate) fn add_spoke(env: &Env, caller: &Address) -> u32 {
    begin_immediate(env, caller, CURATOR_ROLE);
    controller_client(env).add_spoke()
}

/// Registers an asset market on the deployed pool under `hub_id`. Returns that
/// pool's address. Requires `CURATOR_ROLE`.
pub(crate) fn create_liquidity_pool(
    env: &Env,
    caller: &Address,
    hub_id: u32,
    params: &MarketParamsRaw,
) -> Address {
    begin_immediate(env, caller, CURATOR_ROLE);
    validate::asset::validate_new_market(env, &params.asset_id, params);
    controller_client(env).create_liquidity_pool(&hub_id, &params.asset_id, params)
}

/// Suspends `registrar` on `curator`. Requires `GUARDIAN_ROLE`, and `curator`
/// must hold `CURATOR_ROLE`.
pub(crate) fn suspend_curator_registrar(
    env: &Env,
    caller: &Address,
    curator: &Address,
    registrar: u32,
) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    assert_with_error!(
        env,
        access_control::has_role(env, curator, &Symbol::new(env, CURATOR_ROLE)).is_some(),
        GenericError::NotAuthorized
    );
    env.invoke_contract::<()>(
        curator,
        &Symbol::new(env, "set_registrar_suspended"),
        vec![env, registrar.into_val(env), true.into_val(env)],
    );
}

/// Deactivates `manager` on the controller. Requires the caller to hold
/// `GUARDIAN_ROLE`. Activation stays a timelocked, owner-proposed operation.
pub(crate) fn deactivate_position_manager(env: &Env, caller: &Address, manager: &Address) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).set_position_manager(manager, &false);
}

/// Revokes `pool` as a Blend migration source on the controller. Requires the
/// caller to hold `GUARDIAN_ROLE`. Approval stays a timelocked operation.
pub(crate) fn revoke_blend_pool(env: &Env, caller: &Address, pool: &Address) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).revoke_blend_pool(pool);
}

/// Revokes `role` from `account` without going through the timelock. Only
/// `GUARDIAN_ROLE` and `ORACLE_ROLE` can be revoked this way; panics with
/// `GenericError::InvalidRole` for any other role. The role-revoked event names
/// the owner, who calls this entrypoint, as the caller.
pub(crate) fn revoke_role_immediate(env: &Env, account: &Address, role: &Symbol) {
    assert_with_error!(
        env,
        role == &Symbol::new(env, GUARDIAN_ROLE) || role == &Symbol::new(env, ORACLE_ROLE),
        GenericError::InvalidRole
    );
    access::apply_revoke_role(env, account, role, &access::owner_or_panic(env));
}
