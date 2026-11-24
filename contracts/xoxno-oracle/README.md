# XOXNO Oracle

Self-hosted multi-signer price feed. It keeps each registered signer's latest
submission and computes an N-of-M median at write time; the price aggregator
reads it as one provider.

| | |
| --- | --- |
| Owner | OZ `Ownable`, two-step |
| Called by | Price aggregator, through a `FeedSource` holding `ProviderRef::Xoxno(MultiFeedRef)` (`common/src/types/composable_oracle.rs`) |
| Feed reference | `MultiFeedRef { contract, feed_id, nature }` |
| Smoothing | Never: `ProviderRef::is_smoothed` is false for `Xoxno` |
| Client | `common/src/oracle/providers/xoxno.rs`, with the other providers |

Full semantics: crate rustdoc (`src/lib.rs`).

## Aggregation

Quorum, spread and freshness rules, units and entrypoints:
[endpoint reference](../../docs/reference/endpoints.md#xoxno-oracle).
RedStone reads (`read_price_data*`) fail closed; SEP-40 reads return `None`
when unmapped, missing or stale. A submission needs a known feed, a price in
`1..=MAX_SUBMITTED_PRICE` and a timestamp no older than the signer's last for
that feed. `recompute_feeds` runs in footprint-sized batches.

## History

`History(feed_id)` keeps one entry per resolution bucket, at most
`MAX_HISTORY_LEN` (12) entries. A bucket is the `resolution`-second span
`[k * resolution, (k + 1) * resolution)` that holds the write time. A zero
resolution counts as one second.

- A new aggregate in the bucket of the last entry replaces that entry.
- A new aggregate in a later bucket appends an entry. The oldest entry goes
  when the history is full.
- A quorum miss writes no entry.

`prices` and `price` serve rounds, newest first. A round is the newest entry
of a bucket at the live resolution, stamped with the bucket start in seconds.
The bucket holds the write time, not the signed package time. Round
timestamps are distinct, at least one resolution apart, and never after the
ledger time. A Reflector `Twap` read therefore measures round age and spacing
from the writes, not from the signed prices.

- `prices(asset, records)` returns up to `records` rounds.
- `price(asset, timestamp)` returns the newest round written at or before
  `timestamp`.
- `set_resolution` does not rewrite entries. The reads group them by the new
  resolution.

`lastprice`, `read_price_data*` and `read_price_history` return stored
entries. Their timestamps are the oldest clustered package time and the write
time, in milliseconds (`lastprice`: package time in seconds).

## Storage and TTL

All keys are variants of `DataKey` in [`src/storage/mod.rs`](src/storage/mod.rs).

| Key | Tier | Holds |
| --- | --- | --- |
| `Signers`, `Threshold`, `MaxStaleSeconds`, `MaxSubmissionAgeSeconds`, `MaxRelativeSkewSeconds`, `MaxClusterSpreadBps`, `Resolution` | instance | Configuration |
| `LatestSubmission(feed_id, signer)` | persistent | A signer's latest `SignerSubmission` for a feed |
| `SignerFeeds(signer)` | persistent | Feed ids a signer submitted for |
| `CurrentAggregate(feed_id)`, `History(feed_id)` | persistent | Latest median, and one entry per resolution bucket ([History](#history)) |
| `FeedMapping(asset)`, `FeedOwner(feed_id)` | persistent | SEP-40 asset to feed id, and back |
| `AssetCount`, `AssetAt`, `AssetIndex`, `FeedCount`, `FeedAt`, `FeedIndex` | persistent | Swap-remove indexed asset and feed lists |

Submissions, owner calls and `upgrade` renew the instance TTL
(`renew_instance`); reads do not. Persistent writes and aggregate reads extend
with `TTL_THRESHOLD_SHARED` / `TTL_BUMP_SHARED`.

## Events

No custom events; ownership changes publish the stock OpenZeppelin `Ownable`
events ([inherited events](../../docs/reference/events.md#inherited-events)).

## Layout

```text
src/
  lib.rs          entrypoints and crate docs
  submit.rs       signer submissions
  aggregation.rs  submission checks, median and quorum rules
  reads.rs        RedStone, SEP-40 and config reads
  admin.rs        owner configuration
  storage/        config, prices, feed registry, TTL
```

Consumer: [`contracts/price-aggregator`](../price-aggregator/README.md). Errors:
[XOXNO oracle errors](../../docs/reference/errors.md#xoxno-oracle).
