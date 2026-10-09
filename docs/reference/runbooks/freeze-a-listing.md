# Freeze a listing in an emergency

This runbook covers the immediate guardian action on one spoke listing: raise
`paused`, `frozen`, or `no_seize` without the timelock. Use it when one asset in
one spoke must stop, and a global pause is too wide.
[ADR-0007](../../explanation/decisions.md#adr-0007) and
[ADR-0008](../../explanation/decisions.md#adr-0008) define what each flag stops.

The guardian can only raise a flag; a listing edit cannot clear one, and a
flagged listing cannot be removed. Clearing needs the timelocked
`RelaxSpokeAssetFlags` operation.

## Raise the flags

    make <network> tightenAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<guardian>

`<flags>` is a comma list of `paused`, `frozen`, `no_seize`. The verb reads the
live listing, adds the requested flags to the ones already set, and calls
governance `set_spoke_asset_flags`. It cannot clear a flag that is already set.
The spoke id is the id from `configs/<network>/spokes.json`; the verb maps it to
the on-chain id.

## Review pending operations

Do this immediately after the flags are set.

A listing edit proposed before the freeze cannot clear it: the controller
rejects an edit that turns a set flag off. A removal proposed before the
freeze reverts while a flag is set. A relaxation proposed before the freeze
cannot clear it either, because every flag write advances the listing's flags
epoch and the old relaxation names the old epoch. Other pending operations
still execute as proposed: a listing freeze does not advance the emergency
epoch, so a pending `Unpause` stays valid.

1. The verb prints every operation recorded in `configs/ops/<network>/` with its
   live state. `make <network> listOps` prints the same list. An operation
   proposed without a record there does not appear.
2. Cancel each `Waiting` or `Ready` operation that must not run during the
   incident: `make <network> cancelOp <op-id> SIGNER=<canceller>`. A pending
   edit of the frozen listing that names the old flags reverts at execution;
   cancel it too.

## Clear the flags

    make <network> relaxAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<proposer>

1. `<flags>` is a comma list of the flags to clear. The verb reads the live
   listing and its flags epoch, keeps every flag not named, and schedules
   `RelaxSpokeAssetFlags` bound to that epoch. It refuses a flag that is not set.
2. The verb waits for the delay, then executes the operation. With
   `AUTO_EXECUTE=0` it only schedules; run `make <network> executeOp <op-id>`
   after the delay. If any flag write lands first, the operation reverts with
   `SpokeFlagsEpochMismatch`; run the verb again.
3. Read the listing again with
   `make <network> getSpokeAsset <config-spoke-id> <asset>`.

`editAssetInSpoke` refuses a config that sets a live flag to `false` and names
this verb instead.

## Remove a flagged listing

`remove_asset_from_spoke` reverts with `SpokeAssetFlagRelaxation` while any
flag is set, so a frozen listing cannot be delisted in one step. The
relaxation reopens entry, and the removal still needs zero usage, so a deposit
that lands between the two executions blocks the removal.

1. Schedule `RelaxSpokeAssetFlags` for every live flag and
   `removeAssetFromSpoke` for the same listing with `AUTO_EXECUTE=0`.
   `RemoveAssetFromSpoke` has no proposal-time flag check, so both can wait
   together.
2. After the delay, execute the two operations in one transaction, relaxation
   first. No deposit can land between them.

## Notes

- `paused` stops repayment of that asset too, and interest continues. A borrower
  whose only debt is the paused asset cannot be liquidated during the pause and
  can be liquidated in the first transaction after it ends.
- `no_seize` on a collateral listing stops liquidation of every account in that
  spoke that holds the collateral. It does not stop new supply; add `frozen` to
  stop that.
- Insolvent accounts stay reachable through
  [force-socialize](force-socialize-bad-debt.md), which ignores `no_seize`.
