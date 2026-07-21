# XOXNO Oracle Adapter

Self-hosted multi-signer price oracle for assets without a native
Reflector/RedStone feed (e.g. RWA listings). Registered signer wallets push
signed price submissions; the contract stores the latest submission per signer
per feed and recomputes a median aggregate at write time. Aggregation is gated
by an N-of-M signer threshold so that reads remain O(1) regardless of the number
of signers. Submissions older than the submission-age window are excluded from
both the median and the reported observation time, preventing a lagging or
malicious signer from skewing the price or pinning the feed's freshness.

## Read ABIs

One contract exposes two drop-in read shapes for the lending controller:

- **RedStone-style**: `read_price_data` / `read_price_data_for_feed` (bulk
  price-data reads).
- **SEP-40 / Reflector-style**: `base`, `decimals`, `resolution`, `assets`,
  `lastprice`, `price`, `prices`, including TWAP reads bucketed by the
  configured `resolution`.

The lending controller lists this adapter as its own provider variant,
`OracleSourceConfig::Xoxno` (`OracleProviderKind::XoxnoPriceFeed`): the
RedStone wire shape on the read path, but a distinct provider identity, so a
Xoxno source counts as an independent second opinion next to a Reflector or
RedStone leg in `PrimaryWithAnchor` markets. At listing time governance probes
the adapter's SEP-40 `decimals()` and stores the result (the plain `RedStone`
variant assumes the fixed 8-decimal RedStone width instead). The Reflector
wire shape remains available to other consumers, but a production market pair
can never place this adapter on both legs: primary and anchor must not share a
contract address, whichever variants declare them.

## Freshness model

Three related windows:

- `MaxSubmissionAgeSeconds` bounds which per-signer submissions may enter an
  aggregate (absolute age vs ledger clock).
- `MaxRelativeSkewSeconds` (default **equal** to the inclusion window) drops
  absolute-fresh submissions that lag the freshest peer by more than the skew.
  At the default, cluster membership matches the absolute age gate; tighten only
  when bots submit in a tight synchronized wave.
- `MaxStaleSeconds` bounds how long a cached aggregate may be served on reads
  (spot and history).

Keep `MaxSubmissionAgeSeconds <=` every consumer's own staleness bound
(`max_stale_seconds` on the Xoxno source config). Governance listing probes this
against the adapter's live `max_submission_age_seconds()`.

If the number of clustered submissions for a feed drops below the configured
threshold, the cached aggregate and history are cleared. Subsequent reads for
that feed will fail with `NoDataForFeed` / `StaleData` (or return `None` on the
SEP-40 path) until enough signers submit again.

The adapter is treated as a distinct provider (`OracleProviderKind::XoxnoPriceFeed`).
In dual-source (`PrimaryWithAnchor`) markets it can serve as the independent
second opinion; a production market never places the same contract address on
both legs. Reads are strict fail-closed when used for risk.

## Layout

```text
src/
  lib.rs          Contract entrypoints and constructor
  submit.rs       Signer submissions (require_auth, threshold gating)
  reads.rs        RedStone-style + SEP-40 read surface
  admin.rs        Owner administration (signers, thresholds, feeds, upgrade)
  aggregation.rs  Median aggregation and freshness-window logic
  storage.rs      Feed, signer, and aggregate storage keys
```

Feed hygiene: `purge_feed` removes stale per-signer submission state for a
feed.

## Operational requirements

- Signer threshold should be `>= 2` in production so a single compromised
  bot cannot move the median alone. Median is a single order statistic
  (lower mid); even-sized clusters do not average.
- Feeds must be owner-registered (`register_feed` or `add_feed`) before
  signers can submit. Freeform feed ids are rejected (`FeedNotKnown`).
- `add_feed` enforces reverse uniqueness (one asset per feed id). `remove_feed`
  unmaps and wipes price state; `purge_feed` also drops any residual mapping.
- Markets priced only by this adapter under a `Single` strategy have no
  cross-provider deviation check; the market's sanity band is the remaining
  price defense and must be configured tightly.
- The owner manages the signer set, threshold, feed allowlist/mappings,
  staleness windows, and relative skew.
- `submit_price` / `submit_prices` require a registered signer + known feed +
  `require_auth`. Package timestamps must be non-decreasing per signer per feed.
