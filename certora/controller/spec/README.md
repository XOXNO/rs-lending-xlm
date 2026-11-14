# Controller verification

The controller rules prove entrypoint gates, account isolation, position
direction, solvency, liquidation and strategies.

## Assumptions

A controller verdict is conditional on these models.

- **Pool.** Calls resolve through `harness/external/pool.rs` to the assumed
  summaries in `shared/summaries/pool.rs`.
- **Prices and indexes.** One price per asset and one index pair per market
  per rule (`harness/ghost_prices.rs`), so a rule's health checks share one
  valuation. `price_feed_summary` prices are positive and never fail;
  fail-closed oracle behavior is proved in `price-aggregator/spec/`.
- **Position NFT.** Ownership is a ghost map
  (`harness/external/position_nft.rs`).
- **Other calls and storage.** A cross-contract call without a summary returns
  an unconstrained value, and position maps the fixture does not seed hold
  arbitrary values, not empty ones (see
  [Storage, context and external calls](../../README.md#storage-context-and-external-calls)).

## Rule modules

| Module | Proves |
|---|---|
| `account_isolation_rules.rs` | Supply, borrow, repay, liquidation change only the target account |
| `boundary_rules.rs` | Bad-debt socialization threshold and the dust and force gates at the boundary |
| `consistency_rules.rs` | Supply and borrow store the position the pool returns |
| `flash_loan_rules.rs` | Flash-loan guard blocks re-entry into supply and liquidation, clears after the pool returns |
| `health_rules.rs` | Borrow and withdraw safe or health-gated; gate sees final totals; repay improves an unhealthy account |
| `hf_lemma_rules.rs` | Position value monotone in shares; ceil value ≥ floor value |
| `index_rules.rs` | View and accrue agree on the projected index; indexes monotone in time |
| `liquidation_rules.rs` | Liquidation grows neither debt nor seized collateral; bonus bounds; no estimation dust; split partials never out-seize one close |
| `market_guard_rules.rs` | Disabled markets, collateral-less borrow, delegate and recipient checks |
| `position_rules.rs` | Each verb moves the position in its own direction only |
| `solvency_rules.rs` | LTV borrow bound, zero-amount rejects, position limits, one index snapshot per call |
| `spoke_rules.rs` | Spoke asset registration, deprecated spokes, parameter bounds |
| `strategy_rules.rs` | Multiply, swap collateral, swap debt, repay with collateral, flash position, bad debt, revenue |

Rule-free support: `compat.rs` (single-asset shims for multi-asset
entrypoints), `health_ghost.rs` (post-pool solvency gate record), `fixture.rs`
(protocol, market, account seeding), `harness/`.

Revert-shaped rules live in `-reverts` confs with witnesses in
`-reverts-sanity` confs ([Sanity checking on WASM](../../README.md#sanity-checking-on-wasm)).

## Proof order

Prove the controller's lemmas first:

1. `common/confs/math.conf` and `rates.conf`
2. `price-aggregator/confs/tolerance-math.conf`
3. `controller/confs/solvency-*.conf` and `liquidation.conf`
4. the `heavy` profile
