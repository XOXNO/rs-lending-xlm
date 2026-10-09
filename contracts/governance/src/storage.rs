//! Persistent and instance storage access for the governance contract:
//! controller and price-aggregator addresses, the owner, nomination and
//! emergency epochs, and per-operation sidecar state (role-revocation
//! target, recovery-operation marker, operation guard).

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;
use common::types::PriceKey;

use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env, IntoVal, Val};

/// Storage keys for governance contract state. `RoleRevocationTarget`,
/// `RecoveryOp` and `OperationGuard` are keyed per timelock operation id.
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
}

/// Instance counters an operation can be bound to at proposal.
#[derive(Clone, Copy)]
enum Epoch {
    /// Completed ownership handovers.
    Owner,
    /// Ownership nominations made.
    Nomination,
    /// Emergency actions taken: guardian pauses, controller upgrades and
    /// immediate role revocations.
    Emergency,
}

impl Epoch {
    fn key(self) -> GovernanceKey {
        match self {
            Epoch::Owner => GovernanceKey::OwnerEpoch,
            Epoch::Nomination => GovernanceKey::NominationEpoch,
            Epoch::Emergency => GovernanceKey::EmergencyEpoch,
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

/// Governance state an operation is bound to at proposal. Each `Some` field
/// must still hold when the operation executes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationGuard {
    pub owner_epoch: Option<u64>,
    pub nomination_epoch: Option<u64>,
    pub emergency_epoch: Option<u64>,
    pub oracle_band: OracleBandGuard,
}

impl OperationGuard {
    pub(crate) const NONE: OperationGuard = OperationGuard {
        owner_epoch: None,
        nomination_epoch: None,
        emergency_epoch: None,
        oracle_band: OracleBandGuard::NotBound,
    };

    fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

/// Writes `value` under `key` in persistent storage and extends its TTL.
fn set_with_ttl<V: IntoVal<Env, Val>>(env: &Env, key: &GovernanceKey, value: &V) {
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
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

/// Records `account` as the role-revocation target for `operation_id`.
pub(crate) fn mark_role_revocation_target(env: &Env, operation_id: &BytesN<32>, account: &Address) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    set_with_ttl(env, &key, account);
}

/// Marks `operation_id` as a recovery operation.
pub(crate) fn mark_recovery_op(env: &Env, operation_id: &BytesN<32>) {
    set_with_ttl(env, &GovernanceKey::RecoveryOp(operation_id.clone()), &true);
}

/// Records `guard` for `operation_id` unless it binds nothing.
pub(crate) fn set_operation_guard(env: &Env, operation_id: &BytesN<32>, guard: &OperationGuard) {
    if !guard.is_none() {
        set_with_ttl(
            env,
            &GovernanceKey::OperationGuard(operation_id.clone()),
            guard,
        );
    }
}

/// Returns the guard recorded for `operation_id`, or a guard binding nothing.
pub(crate) fn operation_guard(env: &Env, operation_id: &BytesN<32>) -> OperationGuard {
    env.storage()
        .persistent()
        .get(&GovernanceKey::OperationGuard(operation_id.clone()))
        .unwrap_or(OperationGuard::NONE)
}

/// Removes every sidecar entry recorded for `operation_id`.
pub(crate) fn clear_operation_sidecars(env: &Env, operation_id: &BytesN<32>) {
    let storage = env.storage().persistent();
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
