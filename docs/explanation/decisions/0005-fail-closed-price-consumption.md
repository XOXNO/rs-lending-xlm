# 0005. Fail-closed price consumption on every mutating path

Status: Accepted

## Context

Every solvency-relevant operation — borrow, withdraw, liquidate, strategy
execution — prices assets to decide whether the operation is allowed and how
large it may be. A price the aggregator has rejected (stale, deviating
between sources, partially available, outside sanity bounds, or simply
unconfigured) is exactly the price an attacker wants the protocol to act on:
liquidating against a stale price seizes the wrong amount of collateral, and
borrowing against one mints undercollateralized debt. The tension is that the
most safety-critical consumer, liquidation, is also the one whose liveness
matters most — a liquidation that cannot run lets positions rot. The design
must pick which property yields when the oracle cannot vouch for a price.

## Decision

Price integrity wins. The controller's only mutating price path is
`contracts/controller/src/context/oracle.rs::Cache::fetch_prices`, which
calls the aggregator's fail-closed `prices()` entrypoint through
`contracts/controller/src/external/price_aggregator.rs::fetch_prices`. On the
aggregator side, `contracts/price-aggregator/src/engine.rs::force` panics
with the specific `OracleError` on the first failing key — stale, deviating,
partial, out-of-sanity, or unconfigured — and `fetch_prices` additionally
panics with `OracleError::OracleNotConfigured` if the aggregator omits a
requested key from its response.

All risk arithmetic, including liquidation repayment sizing and seizure math,
reads through `contracts/controller/src/context/oracle.rs::Cache::cached_price`,
which panics on any asset that was not pre-fetched — no code path can price
an asset the fail-closed fetch did not cover. There is no degraded mode on
mutating paths: liquidation halts whenever any asset priced in the account
fails integrity checks, exactly as borrow does.

This posture is pinned adversarially:
`tests/test-harness/tests/controller/security_audit.rs::poc_stale_oracle_blocks_liquidation`
and the stale-leg regression family in
`tests/test-harness/tests/controller/audit_liquidate_and_clean_stale_leg.rs`.

The sole fail-open consumer is the read-only
`contracts/controller/src/lib.rs::get_market_indexes_detailed` view, which
goes through
`contracts/controller/src/external/price_aggregator.rs::fetch_prices_status`
and marks failed assets `PriceStatus::unusable()` instead of panicking — a
reporting surface, never an authorization one.

## Alternatives

**Last-known-good price fallback on liquidation paths.** Caching the last
accepted price and using it when the feed fails keeps liquidations live
through outages. But a "last known good" price is, by definition, the price
the oracle now refuses to stand behind — during exactly the volatile windows
when outages and manipulation attempts cluster. Acting on it converts an
availability failure into a solvency failure.

**Per-leg price skipping.** Liquidation could process only the priceable
subset of an account's assets. This makes the liquidation partial in a way
the health-factor math cannot see: the skipped assets still back or burden
the account, so seizure sizing against a partial view can over- or
under-seize relative to the account's true state. All-or-nothing pricing
keeps every solvency computation total.

**Controller-side independent staleness and deviation re-checks.** The
controller could re-validate feeds against its own thresholds instead of
trusting the aggregator's verdict. Two threshold sets drift, disagree, and
double the audit surface; the controller deliberately consumes
`PriceFeedRaw` as validated truth, keeping every acceptance rule in the
aggregator where it is configured, validated, and proven once.

## Consequences

No mutating path can ever execute against a price the aggregator rejects,
and there is no code path to miss — the panic happens in the fetch, before
any risk arithmetic runs. See ../../reference/invariants.md §INV-ORACLE,
§INV-RISK, and §INV-LIQ.

The accepted trade-off, stated plainly: liquidations stall whenever any
asset priced in the account has its price rejected — stale, deviating,
partial, or unpriceable — for as long as the rejection lasts. Liveness
deliberately yields to price integrity. During an oracle outage,
undercollateralized positions cannot be liquidated and will sit unresolved
until the feed recovers; the protocol accepts that solvency drift in
preference to acting on distrusted prices (see ../threat-model.md for the
oracle-failure scenarios this posture is designed against).

What must stay true: mutating paths must never grow a degraded mode, the
fail-open status view must never feed an authorization decision, and the
stale-oracle PoC suite must keep failing any change that lets a liquidation
proceed past a rejected price.
