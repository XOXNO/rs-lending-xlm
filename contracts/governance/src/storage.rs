//! Persistent and instance storage access for the governance contract:
//! controller and price-aggregator addresses, the owner epoch, the
//! ownership-nomination nonce, and per-operation sidecar state
//! (role-revocation target, recovery-operation marker, proposal owner epoch,
//! cancelled nomination, execution guard), and the id of the pending
//! controller upgrade.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;
use common::types::PriceKey;

use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

/// Storage keys for governance contract state. `RoleRevocationTarget`,
/// `RecoveryOp`, `ProposalOwnerEpoch`, `CancelledNomination` and
/// `ExecutionGuard` are keyed per timelock operation id.
#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    PriceAggregator,
    RoleRevocationTarget(BytesN<32>),
    RecoveryOp(BytesN<32>),
    OwnerEpoch,
    ProposalOwnerEpoch(BytesN<32>),
    NominationNonce,
    CancelledNomination(BytesN<32>),
    ExecutionGuard(BytesN<32>),
    PendingControllerUpgrade,
}

/// State an operation is bound to at proposal. Execution reverts once that
/// state has moved.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExecutionGuard {
    /// Controller pause epoch an `Unpause` was proposed under.
    PauseEpoch(u64),
    /// Price key and proposal ledger of a `ConfigureAssetOracle`.
    OracleBand(PriceKey, u32),
}

/// Records `account` as the role-revocation target for `operation_id` in
/// persistent storage and extends the entry's TTL.
pub(crate) fn mark_role_revocation_target(env: &Env, operation_id: &BytesN<32>, account: &Address) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage().persistent().set(&key, account);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Marks `operation_id` as a recovery operation in persistent storage and
/// extends the entry's TTL.
pub(crate) fn mark_recovery_op(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::RecoveryOp(operation_id.clone());
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Returns the number of completed ownership handovers, 0 before the first.
pub(crate) fn owner_epoch(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&GovernanceKey::OwnerEpoch)
        .unwrap_or(0)
}

/// Advances the owner epoch. Panics with `GenericError::MathOverflow` on
/// overflow.
pub(crate) fn bump_owner_epoch(env: &Env) {
    let next = owner_epoch(env)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&GovernanceKey::OwnerEpoch, &next);
}

/// Records the current owner epoch for `operation_id` in persistent storage
/// and extends the entry's TTL.
pub(crate) fn mark_proposal_owner_epoch(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::ProposalOwnerEpoch(operation_id.clone());
    env.storage().persistent().set(&key, &owner_epoch(env));
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Returns the owner epoch recorded for `operation_id`, or `None` if the
/// operation carries no such record.
pub(crate) fn proposal_owner_epoch(env: &Env, operation_id: &BytesN<32>) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::ProposalOwnerEpoch(operation_id.clone()))
}

/// Returns the number of ownership nominations made, 0 before the first.
pub(crate) fn nomination_nonce(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&GovernanceKey::NominationNonce)
        .unwrap_or(0)
}

/// Advances the nomination nonce. Panics with `GenericError::MathOverflow`
/// on overflow.
pub(crate) fn bump_nomination_nonce(env: &Env) {
    let next = nomination_nonce(env)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&GovernanceKey::NominationNonce, &next);
}

/// Records the current nomination nonce as the nomination that the
/// cancellation `operation_id` targets, and extends the entry's TTL.
pub(crate) fn mark_cancelled_nomination(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::CancelledNomination(operation_id.clone());
    env.storage().persistent().set(&key, &nomination_nonce(env));
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Returns the nomination nonce the cancellation `operation_id` targets, or
/// `None` if the operation carries no such record.
pub(crate) fn cancelled_nomination(env: &Env, operation_id: &BytesN<32>) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::CancelledNomination(operation_id.clone()))
}

/// Records `guard` for `operation_id` in persistent storage and extends the
/// entry's TTL.
pub(crate) fn set_execution_guard(env: &Env, operation_id: &BytesN<32>, guard: &ExecutionGuard) {
    let key = GovernanceKey::ExecutionGuard(operation_id.clone());
    env.storage().persistent().set(&key, guard);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Returns the execution guard recorded for `operation_id`, if any.
pub(crate) fn execution_guard(env: &Env, operation_id: &BytesN<32>) -> Option<ExecutionGuard> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::ExecutionGuard(operation_id.clone()))
}

/// Returns the id of the last proposed `UpgradeController`, if it has not
/// executed or been cancelled. It may have expired.
pub(crate) fn pending_controller_upgrade(env: &Env) -> Option<BytesN<32>> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::PendingControllerUpgrade)
}

/// Records `operation_id` as the pending `UpgradeController` in persistent
/// storage and extends the entry's TTL.
pub(crate) fn set_pending_controller_upgrade(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::PendingControllerUpgrade;
    env.storage().persistent().set(&key, operation_id);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Removes every sidecar entry recorded for `operation_id` from persistent
/// storage, and the pending-controller-upgrade slot when it holds
/// `operation_id`.
pub(crate) fn clear_operation_sidecars(env: &Env, operation_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RecoveryOp(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RoleRevocationTarget(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::ProposalOwnerEpoch(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::CancelledNomination(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::ExecutionGuard(operation_id.clone()));
    if pending_controller_upgrade(env).as_ref() == Some(operation_id) {
        env.storage()
            .persistent()
            .remove(&GovernanceKey::PendingControllerUpgrade);
    }
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
