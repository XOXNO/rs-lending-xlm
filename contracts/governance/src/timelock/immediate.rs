//! Role-gated entrypoints that bypass the timelock.
//!
//! Tighten-only or empty-container actions (`pause`, spoke flags, hub/spoke
//! create, immediate `GUARDIAN`/`ORACLE` revoke). Risk-relaxing changes use
//! delayed ops in `lifecycle`.
//!
//! `set_sanity_band` is the exception: `ORACLE` may widen or narrow a band
//! without delay, subject to aggregator bounds (absolute max price, single-
//! source relative width, overlap with the prior band, and live-price
//! containment). The role itself is immediately revocable by the owner.

use common::errors::GenericError;
use common::types::{HubAssetKey, PriceKey};

use soroban_sdk::{assert_with_error, contractimpl, Address, Env, Symbol};

use stellar_macros::only_owner;

use crate::access::{self, GUARDIAN_ROLE, ORACLE_ROLE};
use crate::timelock::*;
use crate::{Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    /// Pauses the controller. `GUARDIAN` only. Resume is timelocked
    /// [`crate::op::AdminOperation::Unpause`].
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    ///
    /// # Events
    /// * Controller pause event.
    pub fn pause(env: Env, caller: Address) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).pause();
    }

    /// Sets spoke listing `paused` / `frozen`. `GUARDIAN` only. Tighten-only
    /// (`false → true` or unchanged); clearing uses timelocked
    /// `EditAssetInSpoke`.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`.
    /// * Controller: `AssetNotInSpoke`, `SpokeAssetFlagRelaxation`.
    ///
    /// # Events
    /// * Controller spoke-asset update event.
    pub fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    ) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).set_spoke_asset_flags(&spoke_id, &hub_asset, &paused, &frozen);
    }

    /// Updates an asset oracle sanity band. `ORACLE` only. Aggregator requires
    /// overlap with the prior band and containment of the live price.
    ///
    /// # Errors
    /// * Access-control rejects non-`ORACLE`.
    /// * Aggregator: `PairNotActive`, `InvalidSanityBounds`,
    ///   `SanityBandTooWideForSingleSource`, `SanityBoundViolated`, feed errors.
    ///
    /// # Events
    /// * Aggregator asset-oracle update event.
    pub fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128) {
        begin_immediate(&env, &caller, ORACLE_ROLE);
        price_aggregator_client(&env).set_sanity_band(&key, &min_wad, &max_wad);
    }

    /// Creates a hub and returns its id. `GUARDIAN` only. Listings remain
    /// timelocked.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    pub fn create_hub(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).create_hub()
    }

    /// Creates a spoke and returns its id. `GUARDIAN` only. Listings remain
    /// timelocked.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    pub fn add_spoke(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).add_spoke()
    }

    /// Immediately revokes `GUARDIAN` or `ORACLE`. Owner only. Other revokes
    /// and all grants stay timelocked; canceller deadlock uses
    /// `propose_canceller_reset`.
    ///
    /// # Errors
    /// * [`GenericError::InvalidRole`] — role is not `GUARDIAN`/`ORACLE`, or
    ///   `account` does not hold it.
    /// * [`GenericError::NotAuthorized`] — `account` is the owner.
    ///
    /// # Events
    /// * Access-control role-revoke event.
    #[only_owner]
    pub fn revoke_role_immediate(env: Env, account: Address, role: Symbol) {
        assert_with_error!(
            &env,
            role == Symbol::new(&env, GUARDIAN_ROLE) || role == Symbol::new(&env, ORACLE_ROLE),
            GenericError::InvalidRole
        );
        access::apply_revoke_role(&env, &account, &role);
    }
}
