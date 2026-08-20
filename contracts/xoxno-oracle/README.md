# XOXNO Oracle

Self-hosted multi-signer feed (`contracts/xoxno-oracle`). Signers submit;
contract stores per-signer latest and recomputes an N-of-M median at write
time. RedStone-style reads fail closed; SEP-40 reads soft-fail with `None`.

| | |
| --- | --- |
| Owner | OZ `Ownable` |
| Provider reference | `ProviderRef::Xoxno(MultiFeedRef)` (`common/src/types/composable_oracle.rs`) |
| Feed reference fields | `MultiFeedRef { contract, feed_id, nature }` |
| Smoothing | Never smoothed: `ProviderRef::is_smoothed` returns false for `Xoxno` |
| Consumer | price-aggregator, through a `FeedSource` holding that `ProviderRef` |

## Surface

| Call | Role |
| --- | --- |
| `submit_price` / `submit_prices` | Signer auth; known feed; non-decreasing timestamps |
| `read_price_data` / `read_price_data_for_feed` / `read_price_history` | Fail-closed RedStone ABI |
| `lastprice` / `price` / `prices` | Soft SEP-40 (`None` when unmapped/missing/stale) |
| `base` / `decimals` / `resolution` / `assets` | SEP-40 metadata |
| `add_signer` / `remove_signer` / `set_threshold` | Owner signer set |
| `set_max_stale_seconds` / `set_max_submission_age_seconds` / `set_max_relative_skew_seconds` | Owner freshness knobs |
| `register_feed` / `add_feed` / `remove_feed` / `purge_feed` | Owner feed hygiene |
| `set_resolution` / `upgrade` | Owner admin |

## Related

| Doc | Topic |
| --- | --- |
| Crate rustdoc (`//!`) | Semantics |
| `contracts/price-aggregator` | Fail-closed compose + soft status |
