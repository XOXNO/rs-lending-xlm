//! Governance actions that execute immediately, bypassing the timelock delay.
//! Each entry point except `revoke_role_immediate` authenticates the caller and
//! checks a required role through `begin_immediate` before forwarding to the
//! controller or price aggregator.
//! `revoke_role_immediate` is instead owner-gated at the contract entry point
//! (`#[only_owner]`) and only restricts which role argument may be revoked.

use common::errors::GenericError;
use common::types::{HubAssetKey, PriceKey};

use soroban_sdk::{assert_with_error, Address, Env, Symbol};

use crate::access::{self, CANCELLER_ROLE, GUARDIAN_ROLE, ORACLE_ROLE};
use crate::storage::{self, KeyedCounter};
use crate::timelock::*;

/// Pauses the controller and advances the emergency epoch, voiding every
/// pending `Unpause`. Requires the caller to hold `GUARDIAN_ROLE`.
pub(crate) fn pause(env: &Env, caller: &Address) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).pause();
    storage::bump_emergency_epoch(env);
}

/// Tightens the paused, frozen, and no-seize flags for `hub_asset` in the given spoke.
/// Requires the caller to hold `GUARDIAN_ROLE`. The controller rejects clearing a set
/// flag with `SpokeAssetFlagRelaxation` (INV-AUTH-04). Advances the token's flag
/// epoch first, voiding every pending `AddAssetToSpoke` of the token in any spoke
/// and hub; a refused flag write reverts the advance with it.
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
    KeyedCounter::AssetFlags(hub_asset.asset.clone()).bump(env);
    controller_client(env).set_spoke_asset_flags(&spoke_id, hub_asset, &paused, &frozen, &no_seize);
}

/// Excludes one account's supply leg of `hub_asset` from its valuation.
/// Requires the caller to hold `GUARDIAN_ROLE`. The controller admits it only
/// while the asset's strict price read fails with a feed outage
/// (`SpokeAssetFeedUsable` or `SpokeAssetFeedFailureIneligible` otherwise);
/// only the timelocked `RestoreAccountAssetValuation` clears it (INV-AUTH-04).
/// It advances no governance counter: it lists nothing and relaxes nothing.
pub(crate) fn exclude_account_asset_valuation(
    env: &Env,
    caller: &Address,
    account_id: u64,
    hub_asset: &HubAssetKey,
) {
    begin_immediate(env, caller, GUARDIAN_ROLE);
    controller_client(env).exclude_account_asset_valuation(&account_id, hub_asset);
}

/// Tightens the sanity-check price band (min/max, WAD) for `key` on the price
/// aggregator. Requires the caller to hold `ORACLE_ROLE`. The aggregator rejects
/// a wider band with `SanityBandMustTighten`. Advances the key's band epoch
/// and the any-key band epoch first, voiding every pending
/// `ConfigureAssetOracle` of the key and every pending `SetPriceAggregator`;
/// a refused band write reverts both advances with it.
pub(crate) fn set_sanity_band(
    env: &Env,
    caller: &Address,
    key: &PriceKey,
    min_wad: i128,
    max_wad: i128,
) {
    begin_immediate(env, caller, ORACLE_ROLE);
    KeyedCounter::Band(key.clone()).bump(env);
    KeyedCounter::AnyBand.bump(env);
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

/// Revokes `role` from `account` without going through the timelock. The
/// revocation advances that account and role's revocation epoch
/// (`access::apply_revoke_role`), so a `GrantGovRole` of the role to the
/// account proposed earlier cannot restore it; revoking `PROPOSER` also
/// advances the account's proposer epoch, voiding every operation it
/// proposed. `GUARDIAN_ROLE`, `ORACLE_ROLE`, `PROPOSER_ROLE` and
/// `EXECUTOR_ROLE` can be revoked this way; `CANCELLER_ROLE` keeps its
/// Recovery-delay removal, which is what lets cancellers veto a compromised
/// owner key. Panics with `GenericError::InvalidRole` for `CANCELLER_ROLE`
/// or a role that is not a governance role.
pub(crate) fn revoke_role_immediate(env: &Env, account: &Address, role: &Symbol) {
    access::require_known_governance_role(env, role);
    assert_with_error!(
        env,
        role != &Symbol::new(env, CANCELLER_ROLE),
        GenericError::InvalidRole
    );
    access::apply_revoke_role(env, account, role);
}
