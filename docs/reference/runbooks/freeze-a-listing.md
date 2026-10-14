# Freeze a listing in an emergency

This runbook covers the immediate guardian action on one spoke listing: raise
`paused`, `frozen`, or `no_seize` without the timelock. Use it when one asset in
one spoke must stop, and a global pause is too wide.
[ADR-0007](../../explanation/decisions.md#adr-0007) and
[ADR-0008](../../explanation/decisions.md#adr-0008) define what each flag stops.

The guardian can only raise a flag, and a listing edit cannot clear one.
Clearing needs the timelocked `RelaxSpokeAssetFlags` operation. An empty
flagged listing can be removed without clearing anything.

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
rejects an edit that turns a set flag off. A relaxation proposed before the
freeze cannot clear it either, because every flag write advances the
listing's flags epoch and the old relaxation names the old epoch. Every
`AddAssetToSpoke` of the frozen token proposed before the freeze, in any
spoke or hub, is void: the guardian call advances the token's flag epoch in
governance, and the listing reverts `AssetFlagsChangedAfterProposal`. A
removal proposed before the freeze still executes on an empty listing, which
leaves the token unlisted there. Other pending operations still execute as
proposed: a listing freeze does not advance the emergency epoch, so a pending
`Unpause` stays valid.

1. The verb prints every operation recorded in `configs/ops/<network>/` with its
   live state. `make <network> listOps` prints the same list. An operation
   proposed without a record there does not appear.
2. Cancel each `Waiting` or `Ready` operation that must not run during the
   incident: `make <network> cancelOp <op-id> SIGNER=<canceller>`. A pending
   edit of the frozen listing that names the old flags reverts at execution,
   and so does a pending listing of the frozen token; cancel them too, which
   frees their salts.

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

`remove_asset_from_spoke` needs zero scaled usage and accepts a flagged
listing, so delist a frozen listing directly; do not relax it first.

1. Keep `frozen` (or `paused`) set, so no deposit can land while the removal
   waits, and wait for the listing's supply and borrow usage to reach zero.
2. Schedule `removeAssetFromSpoke` for the listing and execute it after the
   delay. Do not schedule a `RelaxSpokeAssetFlags` with it: anyone can execute
   a Ready relaxation first, which reopens entry, and a one-unit deposit then
   blocks the removal with `SpokeAssetInUse`.
3. A re-listing of the token proposed before the latest guardian flag write
   is void. Propose a new `AddAssetToSpoke` when the asset should return.

## Notes

- `paused` stops repayment of that asset too, and interest continues. A borrower
  whose only debt is the paused asset cannot be liquidated during the pause and
  can be liquidated in the first transaction after it ends. The same holds for a
  borrower whose only collateral is a listing below 3 decimals while its unpaused
  debt backs less than one unit of it; see the whole-unit limitation under
  [ADR-0008](../../explanation/decisions.md#adr-0008).
- `no_seize` on a collateral listing stops liquidation of every account in that
  spoke that holds the collateral. It does not stop new supply; add `frozen` to
  stop that.
- Insolvent accounts stay reachable through
  [force-socialize](force-socialize-bad-debt.md), which ignores `no_seize`.

## Unblock an account shielded by a feed outage

While an asset's feed is in an outage, no account that holds a supply leg of it
can be valued, so none can be liquidated. An Aquarius LP below
`min_pool_value_wad` is one such case, and a borrower can plant a dust leg to
shield an underwater account ([DoS.1](../../explanation/threat-model.md)). The
guardian can exclude that leg from one named account's valuation
([ADR-0027](../../explanation/decisions.md#adr-0027)). The account is then
valued without it and is liquidatable on its other legs at the curve's bonus.
Other holders of the asset are not affected.

1. Freeze the listing (above) so no new legs enter.
2. Confirm the failure. Read the aggregator's `quotes` for the asset. The
   exclusion accepts only a feed outage: `PriceFeedStale`, `NoLastPrice`,
   `ReflectorHistoryEmpty`, `TwapInsufficientObservations`, `InvalidPrice`,
   `UnsafePriceNotAllowed` or `InsufficientAquariusLiquidity`. It refuses a
   `SanityBoundViolated` failure with `SpokeAssetFeedFailureIneligible`; repair
   the band through the delayed oracle path instead.
3. List the holders of the asset in that spoke from the indexer
   (`UpdatePositionBatchEvent` deposits) and value each one twice: at the last
   good price, and with the leg at zero. Record the collateral each one would
   expose to liquidation with the leg at zero. Pick only accounts that are
   underwater with the leg at zero and hold it as dust, or that are underwater
   at the last good price. Do not pick a holder that is healthy at the last
   good price: it stays frozen until the feed recovers and is then valued in
   full.
4. For each picked account, call governance `exclude_account_asset_valuation`
   as the guardian. Read the stamp back from the controller view
   `get_account_valuation_exclusions` and announce the exclusion.
5. After the feed recovers, `quotes` report the asset `valid`. For each account
   that should count the leg again, propose `RestoreAccountAssetValuation` with
   the stamp from step 4. Propose the `RelaxSpokeAssetFlags` that clears the
   freeze in the same round: the two are independent. A restore executed while
   the price is still unusable reverts with `SpokeAssetFeedUnusable`; execute it
   again after recovery, before it expires.

The exclusion covers supply legs only. An account whose unusable feed is a debt
leg stays frozen until the feed recovers. `configs/script.sh` has no verb for
these calls yet.
