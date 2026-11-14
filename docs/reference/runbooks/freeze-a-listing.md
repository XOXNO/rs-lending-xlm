# Freeze a listing in an emergency

Raising flags needs the governance `GUARDIAN` role; clearing needs `PROPOSER`.

Use this when one asset in one spoke must stop and a global pause is too wide.
The guardian raises `paused`, `frozen` or `no_seize` without the timelock. [ADR-0007](../../explanation/decisions.md#adr-0007) and
[ADR-0008](../../explanation/decisions.md#adr-0008) define what each flag
stops. Neither the guardian nor a listing edit can clear a flag; only the
timelocked `RelaxSpokeAssetFlags` operation can.

## Raise the flags

1. If a curator registrar owns the spoke, suspend it first:

       make <network> suspendCuratorRegistrar <registrar-id> SIGNER=<guardian>

2. Raise the flags:

       make <network> tightenAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<guardian>

`<flags>` is a comma list of `paused`, `frozen`, `no_seize`. The verb reads the
live listing, adds the requested flags, and calls governance
`set_spoke_asset_flags`; it never clears one. The spoke id comes from
`configs/<network>/spokes.json`; the verb maps it on-chain.

An unsuspended registrar can still queue a relaxation at the new epoch; it
executes after its delay.

## Review pending operations

Do these steps immediately after the flags are set.

1. Read the operation list the verb prints (operations in
   `configs/ops/<network>/` with live state; also `make <network> listOps`).
2. Cancel each `Waiting` or `Ready` operation that must not run during the
   incident:

       make <network> cancelOp <op-id> SIGNER=<canceller>

3. Cancel each pending edit of the frozen listing that names the old flags.
   That edit reverts at execution.
4. If a curator registrar owns the spoke, read the curator's
   `["curator", "proposed"]` events. Curator-queued operations are not
   recorded in `configs/ops/`.
5. Cancel each operation those events name with `cancelOp`.

Pending operations cannot clear the new flags. The controller rejects a
pre-freeze listing edit that turns a set flag off. A pre-freeze relaxation
names the old flags epoch, which every flag change advances. Other pending
operations, such as `Unpause`, still execute as proposed.

Naming only held flags reverts with `SpokeAssetFlagsUnchanged`, epoch
unchanged; `tightenAssetFlags` skips such calls. A new flag advances the epoch,
invalidating pending relaxations.

## Clear the flags

1. Schedule the relaxation:

       make <network> relaxAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<proposer>

   `<flags>` lists the flags to clear. The verb reads the live listing and its
   flags epoch, keeps every flag not named, and schedules
   `RelaxSpokeAssetFlags` bound to that epoch. It refuses a flag that is not
   set.
2. Wait for the delay. The verb then executes the operation.
3. If you set `AUTO_EXECUTE=0`, run `make <network> executeOp <op-id>` after the
   delay.
4. If the operation reverts with `SpokeFlagsEpochMismatch`, another flag write
   landed first. Run the verb again.
5. Read the listing again:

       make <network> getSpokeAsset <config-spoke-id> <asset>

`editAssetInSpoke` refuses a config that sets a live flag to `false` and names
this verb instead.

## Notes

- `paused` also stops repayment of that asset while interest continues. A
  borrower whose only debt is the paused asset cannot be liquidated during the
  pause, but can be in the first transaction after it ends.
- `no_seize` on a collateral listing stops liquidation of every account in
  that spoke holding the collateral. It does not stop new supply; add `frozen`
  for that.
- Insolvent accounts stay reachable through
  [force-socialize](force-socialize-bad-debt.md), which ignores `no_seize`.
