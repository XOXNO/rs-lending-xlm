# Freeze a listing in an emergency

For protocol operators. You need the governance `GUARDIAN` role to raise flags.
Clearing them needs the `PROPOSER` role.

This runbook covers the immediate guardian action on one spoke listing. It
raises `paused`, `frozen`, or `no_seize` without the timelock. Use it when one
asset in one spoke must stop and a global pause is too wide.
[ADR-0007](../../explanation/decisions.md#adr-0007) and
[ADR-0008](../../explanation/decisions.md#adr-0008) define what each flag stops.

The guardian can only raise a flag. A listing edit cannot clear a flag either.
Clearing needs the timelocked `RelaxSpokeAssetFlags` operation.

## Raise the flags

1. If a curator registrar owns the spoke, suspend the registrar first:

       make <network> suspendCuratorRegistrar <registrar-id> SIGNER=<guardian>

2. Raise the flags:

       make <network> tightenAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<guardian>

`<flags>` is a comma list of `paused`, `frozen`, `no_seize`. The verb reads the
live listing and adds the requested flags to the ones already set. It then
calls governance `set_spoke_asset_flags`. It cannot clear a flag that is
already set. The spoke id is the id from `configs/<network>/spokes.json`. The
verb maps it to the on-chain id.

A registrar that is not suspended can queue a relaxation at the new flags
epoch. That relaxation executes once its delay has passed.

## Review pending operations

Do these steps immediately after the flags are set.

1. Read the operation list that the verb prints. It shows every operation
   recorded in `configs/ops/<network>/` with its live state.
   `make <network> listOps` prints the same list.
2. Cancel each `Waiting` or `Ready` operation that must not run during the
   incident:

       make <network> cancelOp <op-id> SIGNER=<canceller>

3. Cancel each pending edit of the frozen listing that names the old flags.
   That edit reverts at execution.
4. If a curator registrar owns the spoke, read the curator's
   `["curator", "proposed"]` events. Operations queued through the curator are
   not recorded in `configs/ops/`.
5. Cancel each operation those events name with `cancelOp`.

These pending operations cannot clear the new flags:

- A listing edit proposed before the freeze. The controller rejects an edit
  that turns a set flag off.
- A relaxation proposed before the freeze. Every flag change advances the
  listing's flags epoch, and the old relaxation names the old epoch.

Other pending operations, such as `Unpause`, still execute as proposed.

A guardian call that names only flags the listing already holds reverts with
`SpokeAssetFlagsUnchanged`. It leaves the epoch unchanged. The
`tightenAssetFlags` verb skips the call in that case. Setting a flag that is
not yet set advances the epoch and invalidates a pending relaxation.

## Clear the flags

1. Schedule the relaxation:

       make <network> relaxAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<proposer>

   `<flags>` is a comma list of the flags to clear. The verb reads the live
   listing and its flags epoch. It keeps every flag not named and schedules
   `RelaxSpokeAssetFlags` bound to that epoch. It refuses a flag that is not set.
2. Wait for the delay. The verb then executes the operation.
3. If you set `AUTO_EXECUTE=0`, run `make <network> executeOp <op-id>` after the
   delay.
4. If the operation reverts with `SpokeFlagsEpochMismatch`, run the verb again.
   Another flag write landed first.
5. Read the listing again:

       make <network> getSpokeAsset <config-spoke-id> <asset>

`editAssetInSpoke` refuses a config that sets a live flag to `false`. It names
this verb instead.

## Notes

- `paused` also stops repayment of that asset, and interest continues. A
  borrower whose only debt is the paused asset cannot be liquidated during the
  pause. That borrower can be liquidated in the first transaction after it ends.
- `no_seize` on a collateral listing stops liquidation of every account in that
  spoke that holds the collateral. It does not stop new supply. Add `frozen` to
  stop new supply.
- Insolvent accounts stay reachable through
  [force-socialize](force-socialize-bad-debt.md), which ignores `no_seize`.
