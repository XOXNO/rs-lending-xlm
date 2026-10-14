//! Persistent and instance storage access for the governance contract:
//! controller and price-aggregator addresses, the owner, nomination,
//! controller-nomination and emergency epochs, the pending controller
//! upgrade count, the per-token flag, per-key and any-key band,
//! per-(account, role) revocation, per-upgrade-target and per-proposer
//! epochs, and per-operation
//! sidecar state (role-revocation target, recovery-operation marker,
//! operation guard).

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;
use common::types::PriceKey;

use soroban_sdk::{
    contracttype, panic_with_error, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};

/// Storage keys for governance contract state. `RoleRevocationTarget`,
/// `RecoveryOp` and `OperationGuard` are keyed per timelock operation id.
/// `AssetFlagsEpoch`, `BandEpoch`, `RoleRevocationEpoch`, `UpgradeEpoch` and
/// `ProposerEpoch` are persistent counters keyed by the token, price key,
/// (account, role), (upgraded contract, upgrade function) or proposer they
/// count for; `AnyBandEpoch` is the persistent counter of band writes on any
/// key.
#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    PriceAggregator,
    RoleRevocationTarget(BytesN<32>),
    RecoveryOp(BytesN<32>),
    OwnerEpoch,
    NominationEpoch,
    EmergencyEpoch,
    OperationGuard(BytesN<32>),
    AssetFlagsEpoch(Address),
    BandEpoch(PriceKey),
    RoleRevocationEpoch(Address, Symbol),
    CtrlNominationEpoch,
    PendingControllerUpgrades,
    PendingControllerUpgrade(BytesN<32>),
    UpgradeEpoch(Address, Symbol),
    ProposerEpoch(Address),
    AnyBandEpoch,
}

/// Instance counters an operation can be bound to at proposal.
#[derive(Clone, Copy)]
enum Epoch {
    /// Completed ownership handovers.
    Owner,
    /// Ownership nominations made.
    Nomination,
    /// Emergency actions taken: guardian pauses and executed controller
    /// upgrades.
    Emergency,
    /// Controller ownership nominations executed.
    CtrlNomination,
}

impl Epoch {
    fn key(self) -> GovernanceKey {
        match self {
            Epoch::Owner => GovernanceKey::OwnerEpoch,
            Epoch::Nomination => GovernanceKey::NominationEpoch,
            Epoch::Emergency => GovernanceKey::EmergencyEpoch,
            Epoch::CtrlNomination => GovernanceKey::CtrlNominationEpoch,
        }
    }
}

/// The sanity band (min WAD, max WAD) of a price key as a
/// `ConfigureAssetOracle` found it at proposal: `Band` when the key had an
/// oracle, `Missing` when it had none yet. `NotBound` is for operations that
/// bind no band.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OracleBandGuard {
    NotBound,
    Missing(PriceKey),
    Band(PriceKey, i128, i128),
}

impl OracleBandGuard {
    /// Returns the bound price key, if the guard binds one.
    pub(crate) fn key(&self) -> Option<&PriceKey> {
        match self {
            OracleBandGuard::NotBound => None,
            OracleBandGuard::Missing(key) | OracleBandGuard::Band(key, _, _) => Some(key),
        }
    }
}

/// A persistent counter an operation's kind is bound to, keyed by what it
/// counts for. `guard` derives it from the scheduled function and arguments,
/// at proposal and again at execution, so the guard records only its value.
pub(crate) enum KeyedCounter {
    /// Guardian flag writes on a token, in any spoke and hub
    /// (`AddAssetToSpoke`).
    AssetFlags(Address),
    /// `ORACLE` band writes on a price key (`ConfigureAssetOracle`).
    Band(PriceKey),
    /// `ORACLE` band writes on any price key (`SetPriceAggregator`, whose
    /// switch would otherwise leave every narrowing behind on the old
    /// aggregator).
    AnyBand,
    /// Revocations of a role from an account (`GrantGovRole`, and each
    /// account a canceller reset lists).
    RoleRevocation(Address, Symbol),
    /// Executed upgrades of a contract, keyed by the contract and its upgrade
    /// function (every upgrade operation).
    Upgrade(Address, Symbol),
}

impl KeyedCounter {
    fn key(&self) -> GovernanceKey {
        match self {
            KeyedCounter::AssetFlags(token) => GovernanceKey::AssetFlagsEpoch(token.clone()),
            KeyedCounter::Band(key) => GovernanceKey::BandEpoch(key.clone()),
            KeyedCounter::AnyBand => GovernanceKey::AnyBandEpoch,
            KeyedCounter::RoleRevocation(account, role) => {
                GovernanceKey::RoleRevocationEpoch(account.clone(), role.clone())
            }
            KeyedCounter::Upgrade(target, function) => {
                GovernanceKey::UpgradeEpoch(target.clone(), function.clone())
            }
        }
    }

    /// Returns the counter's value, 0 before its first advance.
    pub(crate) fn value(&self, env: &Env) -> u64 {
        persistent_epoch(env, &self.key())
    }

    /// Advances the counter, voiding every pending operation bound to it.
    pub(crate) fn bump(&self, env: &Env) {
        bump_persistent_epoch(env, &self.key());
    }

    /// Returns the error an operation reports once the counter moved.
    pub(crate) fn error(&self) -> GenericError {
        match self {
            KeyedCounter::AssetFlags(_) => GenericError::AssetFlagsChangedAfterProposal,
            KeyedCounter::Band(_) | KeyedCounter::AnyBand => {
                GenericError::OracleBandChangedAfterProposal
            }
            KeyedCounter::RoleRevocation(_, _) => GenericError::RoleRevokedAfterProposal,
            KeyedCounter::Upgrade(_, _) => GenericError::UpgradeSupersededAfterProposal,
        }
    }
}

/// Governance state an operation is bound to at proposal. Each `Some` field
/// and a bound `oracle_band` must still hold when the operation executes;
/// the counter of `kind_epoch` and the listing of `listing_terms` are read
/// back from the scheduled operation. Every operation proposed since guards
/// exist has a record, even one that binds nothing, so a missing record
/// identifies an operation scheduled before them (`guard::require_holds`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationGuard {
    /// The proposer whose epoch `authority_epoch` records; `None` for a
    /// canceller reset and an operation scheduled before guards existed,
    /// whose `authority_epoch` is the owner epoch.
    pub proposer: Option<Address>,
    pub authority_epoch: u64,
    /// The value of the counter the operation's kind binds, if any
    /// (`guard::kind_counter`): the emergency epoch for `Unpause`, the
    /// nomination epoch of its leg for a zero-deadline ownership transfer,
    /// or a `KeyedCounter`.
    pub kind_epoch: Option<u64>,
    pub oracle_band: OracleBandGuard,
    /// The liquidation terms of the listing a non-owner `EditAssetInSpoke`
    /// found at proposal and must find again at execution, packed by
    /// `guard::pack_terms`.
    pub listing_terms: Option<u128>,
    /// For a canceller reset, the `CANCELLER` revocation epoch of each listed
    /// account at proposal, index for index. Execution leaves out a listed
    /// account whose epoch has moved since, so the reset cannot re-seat an
    /// account revoked after it was proposed.
    pub listed_revocations: Option<Vec<u64>>,
}

impl OperationGuard {
    pub(crate) const NONE: OperationGuard = OperationGuard {
        proposer: None,
        authority_epoch: 0,
        kind_epoch: None,
        oracle_band: OracleBandGuard::NotBound,
        listing_terms: None,
        listed_revocations: None,
    };
}

/// Writes `value` under `key` in persistent storage and extends its TTL.
fn set_with_ttl<V: IntoVal<Env, Val>>(env: &Env, key: &GovernanceKey, value: &V) {
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Returns the persistent counter under `key`, 0 before its first bump.
fn persistent_epoch(env: &Env, key: &GovernanceKey) -> u64 {
    env.storage().persistent().get(key).unwrap_or(0)
}

/// Advances the persistent counter under `key` and extends its TTL; panics
/// with `GenericError::MathOverflow` on overflow.
fn bump_persistent_epoch(env: &Env, key: &GovernanceKey) {
    let next = persistent_epoch(env, key)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    set_with_ttl(env, key, &next);
}

/// Returns `proposer`'s epoch: the number of times it lost `PROPOSER`.
pub(crate) fn proposer_epoch(env: &Env, proposer: &Address) -> u64 {
    persistent_epoch(env, &GovernanceKey::ProposerEpoch(proposer.clone()))
}

/// Advances `proposer`'s epoch, voiding every operation it proposed.
pub(crate) fn bump_proposer_epoch(env: &Env, proposer: &Address) {
    bump_persistent_epoch(env, &GovernanceKey::ProposerEpoch(proposer.clone()));
}

/// Returns the number of pending `UpgradeController` operations proposed
/// since this count exists.
pub(crate) fn pending_controller_upgrades(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&GovernanceKey::PendingControllerUpgrades)
        .unwrap_or(0)
}

/// Adds `delta` (+1 at proposal, -1 when the operation leaves the queue) to
/// the pending `UpgradeController` count. Saturates at 0: only a marked
/// operation decrements it, and every mark was counted when written.
fn add_pending_controller_upgrades(env: &Env, delta: i32) {
    let count = pending_controller_upgrades(env).saturating_add_signed(delta);
    env.storage()
        .instance()
        .set(&GovernanceKey::PendingControllerUpgrades, &count);
}

fn get_epoch(env: &Env, epoch: Epoch) -> u64 {
    env.storage().instance().get(&epoch.key()).unwrap_or(0)
}

/// Advances `epoch`; panics with `GenericError::MathOverflow` on overflow.
fn bump_epoch(env: &Env, epoch: Epoch) {
    let next = get_epoch(env, epoch)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&epoch.key(), &next);
}

/// Returns the number of completed ownership handovers, 0 before the first.
pub(crate) fn owner_epoch(env: &Env) -> u64 {
    get_epoch(env, Epoch::Owner)
}

pub(crate) fn bump_owner_epoch(env: &Env) {
    bump_epoch(env, Epoch::Owner);
}

/// Returns the number of ownership nominations made, 0 before the first.
pub(crate) fn nomination_epoch(env: &Env) -> u64 {
    get_epoch(env, Epoch::Nomination)
}

pub(crate) fn bump_nomination_epoch(env: &Env) {
    bump_epoch(env, Epoch::Nomination);
}

/// Returns the number of emergency actions taken, 0 before the first.
pub(crate) fn emergency_epoch(env: &Env) -> u64 {
    get_epoch(env, Epoch::Emergency)
}

pub(crate) fn bump_emergency_epoch(env: &Env) {
    bump_epoch(env, Epoch::Emergency);
}

/// Returns the number of controller ownership nominations executed, 0 before
/// the first.
pub(crate) fn ctrl_nomination_epoch(env: &Env) -> u64 {
    get_epoch(env, Epoch::CtrlNomination)
}

pub(crate) fn bump_ctrl_nomination_epoch(env: &Env) {
    bump_epoch(env, Epoch::CtrlNomination);
}

/// Records `account` as the role-revocation target for `operation_id`.
pub(crate) fn mark_role_revocation_target(env: &Env, operation_id: &BytesN<32>, account: &Address) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    set_with_ttl(env, &key, account);
}

/// Marks `operation_id` as a recovery operation.
pub(crate) fn mark_recovery_op(env: &Env, operation_id: &BytesN<32>) {
    set_with_ttl(env, &GovernanceKey::RecoveryOp(operation_id.clone()), &true);
}

/// Marks `operation_id` as a pending `UpgradeController`, counted by
/// `pending_controller_upgrades` until it leaves the queue.
pub(crate) fn mark_pending_controller_upgrade(env: &Env, operation_id: &BytesN<32>) {
    set_with_ttl(
        env,
        &GovernanceKey::PendingControllerUpgrade(operation_id.clone()),
        &true,
    );
    add_pending_controller_upgrades(env, 1);
}

/// Records `guard` for `operation_id`. Written even when it binds nothing:
/// the record is what tells the operation from one scheduled before guards
/// existed.
pub(crate) fn set_operation_guard(env: &Env, operation_id: &BytesN<32>, guard: &OperationGuard) {
    set_with_ttl(
        env,
        &GovernanceKey::OperationGuard(operation_id.clone()),
        guard,
    );
}

/// Returns the guard recorded for `operation_id`, or `None` for an operation
/// scheduled before guards existed (or an unknown id).
pub(crate) fn operation_guard(env: &Env, operation_id: &BytesN<32>) -> Option<OperationGuard> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::OperationGuard(operation_id.clone()))
}

/// Removes every sidecar entry recorded for `operation_id`. Every path that
/// takes an operation out of the queue (execution, cancellation, clearing an
/// expired operation) comes through here, so a removed record that binds a
/// controller upgrade also leaves the pending `UpgradeController` count.
pub(crate) fn clear_operation_sidecars(env: &Env, operation_id: &BytesN<32>) {
    let storage = env.storage().persistent();
    let pending_upgrade = GovernanceKey::PendingControllerUpgrade(operation_id.clone());
    if storage.has(&pending_upgrade) {
        storage.remove(&pending_upgrade);
        add_pending_controller_upgrades(env, -1);
    }
    storage.remove(&GovernanceKey::RecoveryOp(operation_id.clone()));
    storage.remove(&GovernanceKey::RoleRevocationTarget(operation_id.clone()));
    storage.remove(&GovernanceKey::OperationGuard(operation_id.clone()));
}

/// Returns the account recorded as the role-revocation target for
/// `operation_id`, or `None` if no such entry exists.
pub(crate) fn role_revocation_target(env: &Env, operation_id: &BytesN<32>) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RoleRevocationTarget(operation_id.clone()))
}

/// Returns whether `operation_id` is marked as a recovery operation.
/// Returns `false` if no marker entry exists.
pub(crate) fn is_recovery_op(env: &Env, operation_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RecoveryOp(operation_id.clone()))
        .unwrap_or(false)
}

/// Returns whether the controller address is set in instance storage.
pub(crate) fn has_controller(env: &Env) -> bool {
    env.storage().instance().has(&GovernanceKey::Controller)
}

/// Returns the controller address from instance storage. Panics with
/// `GenericError::PoolNotInitialized` if it is not set.
pub(crate) fn get_controller(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::Controller)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Stores `addr` as the controller address in instance storage.
pub(crate) fn set_controller(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::Controller, addr);
}

/// Returns whether the price aggregator address is set in instance storage.
pub(crate) fn has_price_aggregator(env: &Env) -> bool {
    env.storage()
        .instance()
        .has(&GovernanceKey::PriceAggregator)
}

/// Returns the price aggregator address from instance storage. Panics with
/// `GenericError::AggregatorNotSet` if it is not set.
pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Stores `addr` as the price aggregator address in instance storage.
pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::PriceAggregator, addr);
}

#[cfg(test)]
#[path = "../tests/storage.rs"]
mod tests;
