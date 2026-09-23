# XOXNO Oracle

Self-hosted multi-signer feed (`contracts/xoxno-oracle`). Signers submit;
contract stores per-signer latest and recomputes an N-of-M median at write
time. A cluster below `2 * (signers - threshold) + 1` entries is a quorum miss
unless `max * 10000 <= min * (10000 + max_cluster_spread_bps)`. On a quorum
miss a submission keeps the stored aggregate; `remove_signer` and
`recompute_feeds` delete it. RedStone-style reads fail closed; SEP-40 reads
soft-fail with `None`.

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Provider reference | `ProviderRef::Xoxno(MultiFeedRef)` (`common/src/types/composable_oracle.rs`) |
| Feed reference fields | `MultiFeedRef { contract, feed_id, nature }` |
| Smoothing | Never smoothed: `ProviderRef::is_smoothed` returns false for `Xoxno` |
| Consumer | price-aggregator, through a `FeedSource` holding that `ProviderRef` |

## Surface

| Call | Role |
| --- | --- |
| `submit_price` / `submit_prices` | Registered signer auth; known feed; price in `1..=MAX_SUBMITTED_PRICE`; fresh timestamp, non-decreasing per signer and feed |
| `read_price_data` / `read_price_data_for_feed` / `read_price_history` | Fail-closed RedStone ABI |
| `lastprice` / `price` / `prices` | Soft SEP-40 (`None` when unmapped/missing/stale) |
| `base` / `decimals` / `resolution` / `assets` | SEP-40 metadata |
| `feeds` / `max_stale_seconds` / `max_submission_age_seconds` / `max_relative_skew_seconds` / `max_cluster_spread_bps` | Public config views |
| `add_signer` / `remove_signer` / `set_threshold` | Owner signer set |
| `set_max_stale_seconds` / `set_max_submission_age_seconds` / `set_max_relative_skew_seconds` | Owner freshness knobs |
| `set_max_cluster_spread_bps` | Owner spread bound (1..=10000 bps, default 200) for a cluster below `2 * (signers - threshold) + 1` entries |
| `register_feed` / `add_feed` / `remove_feed` / `purge_feed` | Owner feed hygiene |
| `recompute_feeds` | Owner — re-derive aggregates after a threshold, signer, freshness or spread change, in footprint-sized batches |
| `set_resolution` / `upgrade` | Owner admin |

## Related

| Doc | Topic |
| --- | --- |
| Crate rustdoc (`//!`) | Semantics |
| `contracts/price-aggregator` | Fail-closed compose + soft status |
