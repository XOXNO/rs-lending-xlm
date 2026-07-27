//! Incident brakes: the entrypoints that deliberately BYPASS the timelock.
//!
//! Each one is role-gated (`GUARDIAN`, `ORACLE`, or owner). Every member except
//! `set_sanity_band` can only tighten risk — pausing, freezing, revoking an
//! operational role — or is inert on its own (`create_hub` / `add_spoke` mint an
//! empty container; the listings that give it teeth ride the delay). Anything
//! else that relaxes risk rides the delay in `lifecycle`. Adding a loosening
//! entrypoint here defeats the timelock.
//!
//! `set_sanity_band` is the deliberate exception, and it is not narrow-only: the
//! aggregator asks only that the new band overlap the old one and contain the
//! live price, so an `ORACLE` holder can widen a band as well as narrow it, with
//! no delay. Two things bound the blast radius. First, absolute caps:
//! `validate_sanity_bounds` holds `max_wad` at or under
//! `MAX_REASONABLE_PRICE_WAD`, and a `Single`-strategy band stays within
//! `MAX_SINGLE_SOURCE_SANITY_BAND_BPS` of its midpoint (anchored strategies are
//! exempt from the relative cap and keep only the absolute one). Second, the
//! containment probe resolves the price under the new config before storing it,
//! so a band that excludes what the feeds print today cannot be written at all.
//! A widened band loosens the tripwire; it does not move the price, and the
//! `ORACLE` role is itself revocable immediately.

use common::errors::GenericError;
use common::types::{HubAssetKey, PriceKey};

use soroban_sdk::{assert_with_error, contractimpl, Address, Env, Symbol};

use stellar_macros::only_owner;

use crate::access::{self, GUARDIAN_ROLE, ORACLE_ROLE};
use crate::timelock::*;
use crate::{Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    /// Halts the controller immediately. `GUARDIAN`-gated. Resume is timelocked
    /// `AdminOperation::Unpause` only.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert on pause.
    ///
    /// # Events
    /// * Controller pause event.
    pub fn pause(env: Env, caller: Address) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).pause();
    }

    /// Sets spoke listing `paused`/`frozen` immediately. `GUARDIAN`-gated.
    /// Tighten-only (`false → true` or stay); clearing rides timelocked
    /// `EditAssetInSpoke`.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`.
    /// * `AssetNotInSpoke`, `SpokeAssetFlagRelaxation` from the controller.
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

    /// Moves an asset oracle sanity band immediately. `ORACLE`-gated. Aggregator
    /// requires the new band to overlap the old one and to contain the live
    /// price; within that, the band may be widened as well as narrowed.
    ///
    /// # Errors
    /// * Access-control rejects non-`ORACLE`.
    /// * `PairNotActive`, `InvalidSanityBounds`, `SanityBandTooWideForSingleSource`,
    ///   `SanityBoundViolated`, and feed-resolution errors from the aggregator.
    ///
    /// # Events
    /// * Aggregator `UpdateAssetOracleV2Event`.
    pub fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128) {
        begin_immediate(&env, &caller, ORACLE_ROLE);
        price_aggregator_client(&env).set_sanity_band(&key, &min_wad, &max_wad);
    }

    /// Creates a hub and returns its id. `GUARDIAN`-gated. Listings still ride
    /// the timelock.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    pub fn create_hub(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).create_hub()
    }

    /// Creates a spoke and returns its id. `GUARDIAN`-gated. Listings still ride
    /// the timelock.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    pub fn add_spoke(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).add_spoke()
    }

    /// Revokes `GUARDIAN` or `ORACLE` immediately. Owner only. Other role
    /// revokes and all grants stay timelocked; canceller deadlock uses
    /// `propose_canceller_reset`.
    ///
    /// # Errors
    /// * `InvalidRole` — not `GUARDIAN`/`ORACLE`, or `account` does not hold it.
    /// * `NotAuthorized` — `account` is the owner (roles never revocable).
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
