# XOXNO Oracle Adapter

Self-hosted multi-signer price oracle for assets without a native
Reflector/RedStone feed (e.g. RWA listings). Registered signer wallets push
signed price submissions; the contract aggregates them into a median at write
time, gated by an N-of-M signer threshold, so reads stay O(1).

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

Two decoupled staleness windows:

- `MaxSubmissionAgeSeconds` bounds which per-signer submissions may enter an
  aggregate, so one lagging or malicious signer cannot pin a feed's reported
  freshness.
- `MaxStaleSeconds` bounds how long a cached aggregate keeps serving reads.

Keep `MaxSubmissionAgeSeconds <=` every consumer's own staleness bound
(`max_price_stale_seconds` on the lending market config).

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
  bot cannot move the median alone.
- Markets priced only by this adapter under a `Single` strategy have no
  cross-provider deviation check; the market's sanity band is the remaining
  price defense and must be configured tightly.
