# Freeze a listing in an emergency

This runbook covers the immediate guardian action on one spoke listing: raise
`paused`, `frozen`, or `no_seize` without the timelock. Use it when one asset in
one spoke must stop, and a global pause is too wide.
[ADR-0007](../../explanation/decisions.md#adr-0007) and
[ADR-0008](../../explanation/decisions.md#adr-0008) define what each flag stops.

The guardian can only raise a flag. Clearing one needs a timelocked listing edit.

## Raise the flags

    make <network> tightenAssetFlags <config-spoke-id> <asset> <flags> SIGNER=<guardian>

`<flags>` is a comma list of `paused`, `frozen`, `no_seize`. The verb reads the
live listing, adds the requested flags to the ones already set, and calls
governance `set_spoke_asset_flags`. It cannot clear a flag that is already set.
The spoke id is the id from `configs/<network>/spokes.json`; the verb maps it to
the on-chain id.

## Cancel pending listing edits

Do this immediately after the flags are set.

A listing edit writes all three flags from the arguments it was proposed with.
An edit proposed before the freeze carries the old flags. When it is Ready, any
address can execute it, and it clears the freeze.

1. The verb prints every recorded operation with its live state. `make <network>
   listOps` prints the same list.
2. Cancel each `Waiting` or `Ready` operation that edits the frozen listing:
   `make <network> cancelOp <op-id> SIGNER=<canceller>`.
3. Do not propose a new edit of that listing until the incident is closed.
   `editAssetInSpoke` carries the live flags when the config does not name them,
   but an operation proposed by another tool does not.

## Clear the flags

1. Set the flag to `false` in `configs/<network>/spokes.json` for that listing.
2. `RELAX_SPOKE_FLAGS=1 make <network> editAssetInSpoke <config-spoke-id> <asset>`.
   Without `RELAX_SPOKE_FLAGS=1` the script refuses to turn a live flag off.
3. Execute the operation after its delay, then read the listing again with
   `make <network> getSpokeAsset <config-spoke-id> <asset>`.

## Notes

- `paused` stops repayment of that asset too, and interest continues. A borrower
  whose only debt is the paused asset cannot be liquidated during the pause and
  can be liquidated in the first transaction after it ends.
- `no_seize` on a collateral listing stops liquidation of every account in that
  spoke that holds the collateral. It does not stop new supply; add `frozen` to
  stop that.
- Insolvent accounts stay reachable through
  [force-socialize](force-socialize-bad-debt.md), which ignores `no_seize`.
