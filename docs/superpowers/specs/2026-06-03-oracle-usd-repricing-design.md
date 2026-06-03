# Oracle USD repricing for non-USD–quoted Reflector sources

Date: 2026-06-03
Status: Implemented (reviewed)

## Problem

The Reflector Stellar-DEX oracle (`CALI…` on mainnet) reports prices quoted in
the **USDC SAC**, not USD — its `base()` returns `Stellar(USDC)`. The controller
treats every Reflector price as USD WAD. `validate_usd_base`
(`contracts/controller/src/oracle/validation/oracle.rs`) accepts only
`Other("USD")` and panics `InvalidOracleBase` otherwise, and it runs for every
Reflector source. Result: USTRY / CETES / TESOURO (which can only be priced via
the DEX oracle — the USD-quoted CEX oracle does not list them) cannot be
oracle-configured on mainnet. Verified live 2026-06-03 via `stellar contract
invoke --send=no`.

## Goal

Allow a Reflector source whose `base()` is `Stellar(Q)` by converting its
token-per-Q price into USD at read time, using `Q`'s own market oracle to obtain
the Q→USD price. USD-quoted sources (CEX Reflector, RedStone) are unchanged.

## Design

### Config-time — `validate_usd_base` → `validate_base` (validation/oracle.rs)

- Accept `base()`:
  - `Other("USD")` → as today.
  - `Stellar(Q)` where `Q` is a configured, **Active**, **USD-quoted** market.
    "USD-quoted" = `Q`'s primary is RedStone, or Reflector with
    `base() == Other("USD")`. This **one-hop rule** forbids configuring a quote
    asset that is itself non-USD-quoted, so chains/cycles cannot be created.
  - anything else (`Other(<non-USD>)`, `String`) → panic `InvalidOracleBase`.
- All other `validate_source` checks (decimals, resolution, lastprice, TWAP
  history) are unchanged.
- Setup ordering already satisfies this: USDC is market[0]; the DEX-quoted
  markets come later.

### Read-time — per source (providers/reflector/mod.rs::read_reflector_source)

After the spot/twap `OracleObservation` is built, inspect `base()`:
- `Other("USD")` → return unchanged.
- `Stellar(Q)` → `q = resolve_usd_quote(cache, &Q)`; `price_usd =
  Wad(obs.price_wad).mul(env, q.price)`; `observed_at = min(obs.observed_at,
  q.timestamp)`.
- else → panic `InvalidOracleBase`.

`resolve_usd_quote` enforces the one-hop rule at READ time (defence in depth
over the config-time check): the quote market must be Active (rejecting a
Disabled/Pending quote regardless of the caller's `OraclePolicy`) and itself
USD-quoted (RedStone, or Reflector `base() == Other("USD")`). This holds even if
the quote market is reconfigured after the dependent market is set up — the
dependent market then reverts rather than silently chaining a second hop.

Applies to a Reflector source whether primary or anchor, and happens **before**
composition, so the tolerance band always compares USD vs USD.

### Cycle / termination safety (no persistent or Cache state)

- The one-hop rule (quote must be USD-quoted) means a Stellar-quoted market can
  never be the target of a quote edge, so 2+ cycles cannot be configured.
- Self-quote (`base == Stellar(self)`) is rejected explicitly in `validate_base`
  (`quote != asset`), because the quote check reads the asset's pre-update
  config and would otherwise let a market quote in itself.
- Ultimate backstop: the Soroban host call-depth/budget cap turns any pathological
  chain into a revert rather than an unbounded loop. No Cache field, no depth
  counter.

### Math

`obs.price_wad` (WAD, token-per-Q) `× q.price` (WAD, Q-USD) via `Wad::mul`
(`a*b/WAD`) → WAD USD. The market's existing USD sanity bounds in `token_price`
apply to the converted price unchanged.

### Error handling

Reuse `OracleError::InvalidOracleBase`. `token_price(Q)` failures (quote market
missing / disabled / stale) propagate — a token whose quote cannot be priced
must fail to price.

## Testing

- Unit: conversion multiply.
- Config: DEX market configures when its quote market is present + USD; rejects
  when the quote market is missing, disabled, or itself non-USD-quoted.
- Read: `token_price(token)` ≈ `dex_price × quote_usd`; tolerance compares the
  converted DEX primary against the RedStone USD anchor; a quote-depeg scenario
  moves the converted primary and trips the band.
- Cycle attempt (A quoted in B, B quoted in A) rejected.

## Trade-off

Every Reflector read now performs one `base()` call (including USD/CEX markets
that need no conversion) — the cost of not storing the quote asset. Accepted.

## Out of scope

No `common` type/storage changes, no Cache changes, no config-JSON changes.

## Files

- `contracts/controller/src/oracle/validation/oracle.rs`
- `contracts/controller/src/oracle/providers/reflector/mod.rs`
- test-harness oracle tests + mock oracle (`base()` returning `Stellar`).
