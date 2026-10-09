//! Operation guards: governance state an operation is bound to when it is
//! proposed, so a handover or emergency action taken in between voids the
//! operation instead of being undone by it.

use common::errors::GenericError;
use common::types::PriceKey;
use price_aggregator_interface::PriceAggregatorClient;

use soroban_sdk::{assert_with_error, Address, BytesN, Env};

use crate::op::{requires_owner_proposer, AdminOperation};
use crate::storage::{self, OperationGuard, OracleBandGuard};

/// Owner epoch a recovery operation carrying only the bare `RecoveryOp`
/// marker is bound to: such a reset was proposed before resets recorded an
/// epoch, so it may execute only until the first ownership handover.
const LEGACY_RECOVERY_OWNER_EPOCH: u64 = 0;

/// Returns the sanity band the price aggregator holds for `key`, or
/// `Missing(key)` when the key has no oracle.
fn sanity_band(env: &Env, aggregator: &Address, key: &PriceKey) -> OracleBandGuard {
    PriceAggregatorClient::new(env, aggregator)
        .oracle(key)
        .map_or(OracleBandGuard::Missing(key.clone()), |oracle| {
            OracleBandGuard::Band(
                key.clone(),
                oracle.min_sanity_price_wad,
                oracle.max_sanity_price_wad,
            )
        })
}

/// Builds the guard `op` is bound to: the owner epoch for owner-only
/// operations, the nomination epoch for a nomination cancellation, the
/// emergency epoch for `Unpause` and `GrantGovRole`, and the current sanity
/// band, or its absence, for `ConfigureAssetOracle`.
pub(crate) fn for_op(env: &Env, op: &AdminOperation) -> OperationGuard {
    OperationGuard {
        owner_epoch: requires_owner_proposer(op).then(|| storage::owner_epoch(env)),
        nomination_epoch: match op {
            AdminOperation::TransferGovOwnership(args) if args.live_until_ledger == 0 => {
                Some(storage::nomination_epoch(env))
            }
            _ => None,
        },
        emergency_epoch: matches!(
            op,
            AdminOperation::Unpause | AdminOperation::GrantGovRole(_)
        )
        .then(|| storage::emergency_epoch(env)),
        oracle_band: match op {
            AdminOperation::ConfigureAssetOracle(args) => {
                sanity_band(env, &storage::get_price_aggregator(env), &args.key)
            }
            _ => OracleBandGuard::NotBound,
        },
    }
}

/// Returns the guard for a canceller reset: bound to the owner epoch only.
pub(crate) fn for_canceller_reset(env: &Env) -> OperationGuard {
    OperationGuard {
        owner_epoch: Some(storage::owner_epoch(env)),
        ..OperationGuard::NONE
    }
}

/// Panics if the state `operation_id`'s guard bound it to has moved:
/// `NotAuthorized` after an ownership handover, `EmergencyEpochMismatch`
/// after an emergency action, `OracleBandChangedAfterProposal` when the
/// aggregator at `target` does not hold the recorded band, or holds one where
/// none was recorded. Returns the guard for the caller's own checks.
pub(crate) fn require_holds(
    env: &Env,
    operation_id: &BytesN<32>,
    target: &Address,
) -> OperationGuard {
    let guard = storage::operation_guard(env, operation_id);
    let owner_epoch = guard.owner_epoch.or_else(|| {
        storage::is_recovery_op(env, operation_id).then_some(LEGACY_RECOVERY_OWNER_EPOCH)
    });
    if let Some(epoch) = owner_epoch {
        assert_with_error!(
            env,
            epoch == storage::owner_epoch(env),
            GenericError::NotAuthorized
        );
    }
    if let Some(epoch) = guard.emergency_epoch {
        assert_with_error!(
            env,
            epoch == storage::emergency_epoch(env),
            GenericError::EmergencyEpochMismatch
        );
    }
    if let Some(key) = guard.oracle_band.key() {
        assert_with_error!(
            env,
            sanity_band(env, target, key) == guard.oracle_band,
            GenericError::OracleBandChangedAfterProposal
        );
    }
    guard
}
