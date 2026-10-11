//! Operation guards: governance state an operation is bound to when it is
//! proposed, so a handover, a revocation or an emergency action taken in
//! between voids the operation instead of being undone by it.

use common::errors::GenericError;
use common::types::{HubAssetKey, PriceKey, SpokeAssetArgs};
use controller_interface::ControllerClient;
use price_aggregator_interface::PriceAggregatorClient;

use soroban_sdk::{
    assert_with_error, panic_with_error, Address, BytesN, Env, Symbol, TryFromVal, Val, Vec,
};

use stellar_governance::timelock::Operation;

use crate::access::{self, CANCELLER_ROLE};
use crate::op::{AdminOperation, CONTROLLER_UPGRADE_FN};
use crate::storage::{self, KeyedCounter, OperationGuard, OracleBandGuard};

/// Value every counter held when the guards were deployed. An operation
/// scheduled before then is held to the counters its kind binds today at
/// this value, as if it had been proposed on the upgrade ledger.
const PRE_UPGRADE_EPOCH: u64 = 0;

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

/// Packs liquidation terms (BPS) into the one value a guard records: the
/// threshold in bits 64-95, the bonus in bits 32-63, the fees in bits 0-31.
fn pack_terms(threshold: u32, bonus: u32, fees: u32) -> u128 {
    (u128::from(threshold) << 64) | (u128::from(bonus) << 32) | u128::from(fees)
}

/// Returns the liquidation terms the controller at `controller` holds for the
/// listing `args` edits, packed; the controller refuses an absent listing.
fn listing_terms(env: &Env, controller: &Address, args: &SpokeAssetArgs) -> u128 {
    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    let live = ControllerClient::new(env, controller).get_spoke_asset(&args.spoke_id, &hub_asset);
    pack_terms(
        live.liquidation_threshold,
        live.liquidation_bonus,
        live.liquidation_fees,
    )
}

/// Decodes argument `index` of the scheduled `operation`, panicking with
/// `error` when it is missing or of another type. `resolve_op` built every
/// scheduled operation, so a mismatch cannot occur; the panic fails closed.
fn scheduled_arg<T: TryFromVal<Env, Val>>(
    env: &Env,
    operation: &Operation,
    index: u32,
    error: GenericError,
) -> T {
    operation
        .args
        .get(index)
        .and_then(|arg| T::try_from_val(env, &arg).ok())
        .unwrap_or_else(|| panic_with_error!(env, error))
}

/// Returns whether `operation` targets the controller.
fn targets_controller(env: &Env, operation: &Operation) -> bool {
    storage::has_controller(env) && operation.target == storage::get_controller(env)
}

/// The counter the kind of a scheduled operation is bound to. An operation
/// binds at most one.
enum KindCounter {
    /// Emergency actions: guardian pauses and executed controller upgrades
    /// (`unpause`).
    Emergency,
    /// Ownership nominations on the leg a zero-deadline `transfer_ownership`
    /// cancels: governance (`true`) or the controller (`false`).
    Nomination(bool),
    /// A persistent counter keyed by what it counts for.
    Keyed(KeyedCounter),
}

impl KindCounter {
    fn value(&self, env: &Env) -> u64 {
        match self {
            KindCounter::Emergency => storage::emergency_epoch(env),
            KindCounter::Nomination(true) => storage::nomination_epoch(env),
            KindCounter::Nomination(false) => storage::ctrl_nomination_epoch(env),
            KindCounter::Keyed(counter) => counter.value(env),
        }
    }
}

/// Returns the counter the kind of the scheduled `operation` is bound to,
/// read back from its function and arguments: the emergency epoch for
/// `unpause`; the nomination epoch of its leg for a governance or controller
/// `transfer_ownership` with a zero deadline; the token's flag epoch for
/// `add_asset_to_spoke`; the key's band epoch for `set_oracle`; the any-key
/// band epoch for a governance `set_price_aggregator`; the account
/// and role's revocation epoch for a governance `grant_role`; and the
/// upgrade epoch of the contract for an upgrade (`upgrade` on governance,
/// the controller or the price aggregator, `upgrade_pool`,
/// `upgrade_position_nft`). Each of those functions belongs to exactly one
/// `AdminOperation` arm.
fn kind_counter(env: &Env, operation: &Operation) -> Option<KindCounter> {
    let function = &operation.function;
    let on_self = operation.target == env.current_contract_address();
    Some(if *function == Symbol::new(env, "unpause") {
        KindCounter::Emergency
    } else if *function == Symbol::new(env, "transfer_ownership") {
        if scheduled_arg::<u32>(env, operation, 1, GenericError::InternalError) != 0 {
            return None;
        }
        KindCounter::Nomination(on_self)
    } else if *function == Symbol::new(env, "add_asset_to_spoke") {
        let args: SpokeAssetArgs = scheduled_arg(
            env,
            operation,
            0,
            GenericError::AssetFlagsChangedAfterProposal,
        );
        KindCounter::Keyed(KeyedCounter::AssetFlags(args.asset))
    } else if *function == Symbol::new(env, "set_oracle") {
        KindCounter::Keyed(KeyedCounter::Band(scheduled_arg(
            env,
            operation,
            0,
            GenericError::OracleBandChangedAfterProposal,
        )))
    } else if on_self && *function == Symbol::new(env, "set_price_aggregator") {
        KindCounter::Keyed(KeyedCounter::AnyBand)
    } else if on_self && *function == Symbol::new(env, "grant_role") {
        let error = GenericError::RoleRevokedAfterProposal;
        KindCounter::Keyed(KeyedCounter::RoleRevocation(
            scheduled_arg(env, operation, 0, error),
            scheduled_arg(env, operation, 1, error),
        ))
    } else if *function == Symbol::new(env, CONTROLLER_UPGRADE_FN)
        || *function == Symbol::new(env, "upgrade_pool")
        || *function == Symbol::new(env, "upgrade_position_nft")
    {
        KindCounter::Keyed(KeyedCounter::Upgrade(
            operation.target.clone(),
            function.clone(),
        ))
    } else {
        return None;
    })
}

/// Returns the guard of an operation scheduled before guards existed: the
/// owner epoch and the counter its kind binds today (`kind_counter`), both at
/// `PRE_UPGRADE_EPOCH`, the value every counter had at that upgrade. Not
/// bound, because nothing was recorded at proposal: the band value, the
/// proposer and the listing terms.
fn pre_upgrade_guard(env: &Env, operation: &Operation) -> OperationGuard {
    OperationGuard {
        kind_epoch: kind_counter(env, operation).map(|_| PRE_UPGRADE_EPOCH),
        ..OperationGuard::NONE
    }
}

/// Builds the guard `op`, resolved to `operation` and proposed by
/// `proposer`, is bound to: the proposer's epoch for every proposal (it
/// advances when the proposer loses `PROPOSER`, a handover away from an
/// owner included); the current value of the counter its kind binds
/// (`kind_counter`); the key's current sanity band, or its absence, for
/// `ConfigureAssetOracle`; and, for an `EditAssetInSpoke` a non-owner
/// proposes, the listing's live liquidation terms.
pub(crate) fn for_op(
    env: &Env,
    proposer: &Address,
    op: &AdminOperation,
    operation: &Operation,
) -> OperationGuard {
    OperationGuard {
        proposer: Some(proposer.clone()),
        authority_epoch: storage::proposer_epoch(env, proposer),
        oracle_band: match op {
            AdminOperation::ConfigureAssetOracle(args) => {
                sanity_band(env, &storage::get_price_aggregator(env), &args.key)
            }
            _ => OracleBandGuard::NotBound,
        },
        listing_terms: match op {
            AdminOperation::EditAssetInSpoke(args) if proposer != &access::owner_or_panic(env) => {
                Some(listing_terms(env, &operation.target, args))
            }
            _ => None,
        },
        kind_epoch: kind_counter(env, operation).map(|kind| kind.value(env)),
        listed_revocations: None,
    }
}

/// Panics with `GenericError::NotAuthorized` when a non-owner listing edit
/// moves the live `terms` it recorded toward the liquidator: a lower
/// liquidation threshold, a higher bonus or lower fees. That direction needs
/// the owner as proposer, like the other liquidation-terms operations
/// (INV-AUTH-09).
pub(crate) fn require_terms_kept(env: &Env, terms: Option<u128>, args: &SpokeAssetArgs) {
    if let Some(live) = terms {
        assert_with_error!(
            env,
            args.threshold >= (live >> 64) as u32
                && args.bonus <= (live >> 32) as u32
                && args.liquidation_fees >= live as u32,
            GenericError::NotAuthorized
        );
    }
}

/// Returns the guard for a canceller reset to `new_cancellers`: bound to the
/// owner epoch, and recording each listed account's `CANCELLER` revocation
/// epoch so execution leaves out an account revoked since
/// (`access::apply_canceller_reset`).
pub(crate) fn for_canceller_reset(env: &Env, new_cancellers: &Vec<Address>) -> OperationGuard {
    let role = Symbol::new(env, CANCELLER_ROLE);
    let mut epochs = Vec::new(env);
    for account in new_cancellers.iter() {
        epochs.push_back(KeyedCounter::RoleRevocation(account, role.clone()).value(env));
    }
    OperationGuard {
        authority_epoch: storage::owner_epoch(env),
        listed_revocations: Some(epochs),
        ..OperationGuard::NONE
    }
}

/// Returns the `CANCELLER` revocation epoch a canceller reset recorded for
/// its listed account at `index`: the value in `guard`, or
/// `PRE_UPGRADE_EPOCH` for a reset scheduled before resets recorded them.
pub(crate) fn listed_revocation_epoch(guard: &OperationGuard, index: u32) -> u64 {
    guard
        .listed_revocations
        .as_ref()
        .and_then(|epochs| epochs.get(index))
        .unwrap_or(PRE_UPGRADE_EPOCH)
}

/// Returns whether `guard`'s authority still holds: its proposer has not lost
/// `PROPOSER` since the proposal, or, for a guard without a proposer, no
/// ownership handover happened.
fn authority_holds(env: &Env, guard: &OperationGuard) -> bool {
    guard.authority_epoch
        == guard.proposer.as_ref().map_or_else(
            || storage::owner_epoch(env),
            |proposer| storage::proposer_epoch(env, proposer),
        )
}

/// Panics if the state the scheduled `operation`'s guard bound it to has
/// moved: `NotAuthorized` once its proposer lost `PROPOSER` (a handover away
/// from the owner that proposed it included) or, for a canceller reset or an
/// operation scheduled before guards existed, after an ownership handover;
/// the counter's error after a guardian flag write on the token
/// (`AssetFlagsChangedAfterProposal`), an `ORACLE` band write on the key, or
/// on any key for an aggregator switch (`OracleBandChangedAfterProposal`), a
/// revocation of the account and role
/// (`RoleRevokedAfterProposal`) or another executed upgrade of the same
/// contract (`UpgradeSupersededAfterProposal`); `EmergencyEpochMismatch`
/// after an emergency action, and `ControllerUpgradePending` for an
/// `Unpause` while an `UpgradeController` is pending;
/// `OracleBandChangedAfterProposal` when the aggregator does not hold the
/// recorded band, or holds one where none was recorded; and
/// `ListingTermsChangedAfterProposal` when the listing a non-owner edit
/// writes to no longer holds the liquidation terms it was proposed against.
/// A nomination cancellation's epoch is not checked here: a stale one
/// completes as a no-op (`execute_self`, `execute`). An operation without a
/// guard record was scheduled before guards existed and is held to
/// `pre_upgrade_guard`. Returns the guard for the caller's own checks.
pub(crate) fn require_holds(
    env: &Env,
    operation_id: &BytesN<32>,
    operation: &Operation,
) -> OperationGuard {
    let guard = storage::operation_guard(env, operation_id)
        .unwrap_or_else(|| pre_upgrade_guard(env, operation));
    assert_with_error!(
        env,
        authority_holds(env, &guard),
        GenericError::NotAuthorized
    );
    if let (Some(epoch), Some(kind)) = (guard.kind_epoch, kind_counter(env, operation)) {
        match kind {
            KindCounter::Nomination(_) => {}
            KindCounter::Emergency => {
                assert_with_error!(
                    env,
                    epoch == storage::emergency_epoch(env),
                    GenericError::EmergencyEpochMismatch
                );
                assert_with_error!(
                    env,
                    storage::pending_controller_upgrades(env) == 0,
                    GenericError::ControllerUpgradePending
                );
            }
            KindCounter::Keyed(counter) => {
                assert_with_error!(env, counter.value(env) == epoch, counter.error());
            }
        }
    }
    if let Some(key) = guard.oracle_band.key() {
        assert_with_error!(
            env,
            sanity_band(env, &operation.target, key) == guard.oracle_band,
            GenericError::OracleBandChangedAfterProposal
        );
    }
    if let Some(terms) = guard.listing_terms {
        let args: SpokeAssetArgs = scheduled_arg(
            env,
            operation,
            0,
            GenericError::ListingTermsChangedAfterProposal,
        );
        assert_with_error!(
            env,
            listing_terms(env, &operation.target, &args) == terms,
            GenericError::ListingTermsChangedAfterProposal
        );
    }
    guard
}

/// Returns whether the canceller reset `operation_id` can no longer execute
/// because ownership changed hands after it was proposed. A reset scheduled
/// before guards existed is bound to `PRE_UPGRADE_EPOCH`.
pub(crate) fn canceller_reset_is_stale(env: &Env, operation_id: &BytesN<32>) -> bool {
    !authority_holds(
        env,
        &storage::operation_guard(env, operation_id).unwrap_or(OperationGuard::NONE),
    )
}

/// Returns whether the scheduled `operation`, run through `execute`, is a
/// controller nomination cancellation whose recorded nomination epoch moved:
/// a controller nomination executed after it was proposed, so it completes
/// as a no-op instead of cancelling that later nomination. The governance
/// leg runs through `execute_self`.
pub(crate) fn is_stale_controller_cancel(
    env: &Env,
    operation: &Operation,
    guard: &OperationGuard,
) -> bool {
    operation.function == Symbol::new(env, "transfer_ownership")
        && guard
            .kind_epoch
            .is_some_and(|epoch| epoch != storage::ctrl_nomination_epoch(env))
}

/// Advances the counters an executed `operation` moves: the upgrade epoch of
/// the contract an upgrade replaced, the emergency epoch for a controller
/// upgrade, and the controller nomination epoch for a controller
/// `transfer_ownership` with a nonzero deadline.
pub(crate) fn after_execute(env: &Env, operation: &Operation) {
    if let Some(KindCounter::Keyed(counter @ KeyedCounter::Upgrade(_, _))) =
        kind_counter(env, operation)
    {
        counter.bump(env);
    }
    if targets_controller(env, operation) {
        if operation.function == Symbol::new(env, CONTROLLER_UPGRADE_FN) {
            storage::bump_emergency_epoch(env);
        } else if operation.function == Symbol::new(env, "transfer_ownership")
            && scheduled_arg::<u32>(env, operation, 1, GenericError::InternalError) != 0
        {
            storage::bump_ctrl_nomination_epoch(env);
        }
    }
}
