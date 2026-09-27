# XOXNO Oracle

Self-hosted multi-signer price feed. Registered signers submit prices; the
contract keeps each signer's latest submission and computes an N-of-M median
at write time. The price aggregator reads it as one provider.

| | |
| --- | --- |
| Owner | OZ `Ownable`, two-step |
| Called by | Price aggregator, through a `FeedSource` holding `ProviderRef::Xoxno(MultiFeedRef)` (`common/src/types/composable_oracle.rs`) |
| Feed reference | `MultiFeedRef { contract, feed_id, nature }` |
| Smoothing | Never: `ProviderRef::is_smoothed` is false for `Xoxno` |
| Client | In `common/src/oracle/providers/xoxno.rs`, with the other providers |

Full semantics are in the crate rustdoc (`src/lib.rs`).

## Aggregation

- A median forms once at least `threshold` fresh submissions fall within the
  timestamp-skew bound of each other.
- A cluster with fewer than `2 * (signers - threshold) + 1` entries is a
  quorum miss unless it fits the spread bound:
  `max * 10000 <= min * (10000 + max_cluster_spread_bps)`.
- On a quorum miss a submission keeps the stored aggregate. `remove_signer`
  and `recompute_feeds` delete it instead.
- RedStone-style reads fail closed. SEP-40 reads return `None`.

## Entrypoints

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `submit_price`, `submit_prices` | registered signer | Known feed; price in `1..=MAX_SUBMITTED_PRICE`; fresh timestamp, not older than the signer's last for that feed |
| `read_price_data`, `read_price_data_for_feed`, `read_price_history` | anyone | RedStone ABI, fail-closed |
| `lastprice`, `price`, `prices` | anyone | SEP-40; `None` when unmapped, missing or stale |
| `base`, `decimals`, `resolution`, `assets` | anyone | SEP-40 metadata |
| `feeds`, `max_stale_seconds`, `max_submission_age_seconds`, `max_relative_skew_seconds`, `max_cluster_spread_bps` | anyone | Config |
| `add_signer`, `remove_signer`, `set_threshold` | owner | Signer set |
| `set_max_stale_seconds`, `set_max_submission_age_seconds`, `set_max_relative_skew_seconds` | owner | Freshness bounds |
| `set_max_cluster_spread_bps` | owner | Spread bound, `1..=10000`, default 200 |
| `register_feed`, `add_feed`, `remove_feed`, `purge_feed` | owner | Feed registry |
| `recompute_feeds` | owner | Rebuilds aggregates after a signer, threshold, freshness or spread change, in footprint-sized batches |
| `set_resolution`, `upgrade` | owner | Admin |

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

## References

- Consumer: [`contracts/price-aggregator`](../price-aggregator/README.md)
