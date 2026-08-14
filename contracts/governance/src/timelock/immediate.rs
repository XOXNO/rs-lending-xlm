//! Governance actions that execute immediately, bypassing the timelock delay.
//! Each entry point authenticates the caller and checks a required role before
//! forwarding to the controller or price aggregator, or applying the change directly.

use common::errors::GenericError;
use common::types::{HubAssetKey, PriceKey};

use soroban_sdk::{assert_with_error, Address, Env, Symbol};

use crate::access::{self, GUARDIAN_ROLE, ORACLE_ROLE};
use crate::timelock::*;

/// Pauses the controller. Requires the caller to hold `GUARDIAN_ROLE`.
pub(crate) fn pause(env: &Env, caller: &Address) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).pause();
}

/// Sets the paused, frozen, and no-seize flags for `hub_asset` in the given spoke.
/// Requires the caller to hold `GUARDIAN_ROLE`.
pub(crate) fn set_spoke_asset_flags(
    env: &Env,
    caller: &Address,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).set_spoke_asset_flags(&spoke_id, hub_asset, &paused, &frozen, &no_seize);
}

/// Sets the sanity-check price band (min/max, WAD-scaled) for `key` on the price
/// aggregator. Requires the caller to hold `ORACLE_ROLE`.
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
/// hold `GUARDIAN_ROLE`.
pub(crate) fn create_hub(env: &Env, caller: &Address) -> u32 {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).create_hub()
}

/// Creates a new spoke on the controller and returns its id. Requires the caller
/// to hold `GUARDIAN_ROLE`.
pub(crate) fn add_spoke(env: &Env, caller: &Address) -> u32 {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).add_spoke()
}

/// Revokes `role` from `account` without going through the timelock. Only
/// `GUARDIAN_ROLE` and `ORACLE_ROLE` can be revoked this way; panics with
/// `GenericError::InvalidRole` for any other role.
pub(crate) fn revoke_role_immediate(env: &Env, account: &Address, role: &Symbol) {
    assert_with_error!(
        env,
        role == &Symbol::new(env, GUARDIAN_ROLE) || role == &Symbol::new(env, ORACLE_ROLE),
        GenericError::InvalidRole
    );
    access::apply_revoke_role(env, account, role);
}
