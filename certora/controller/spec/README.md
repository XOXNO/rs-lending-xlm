# Controller verification

The controller rules prove entrypoint gates, account isolation, position
direction, solvency, liquidation, and strategies.

## Assumptions

A controller verdict is conditional on these models.

- **Pool.** Calls resolve through `harness/external/pool.rs` to the assumed
  summaries in `shared/summaries/pool.rs`.
- **Prices and indexes.** The model uses one price per asset and one index pair
  per market per rule (`harness/ghost_prices.rs`), so a rule's health checks
  share one valuation. `price_feed_summary` prices are positive and never fail.
  `price-aggregator/spec/` proves fail-closed oracle behavior.
- **Position NFT.** Ownership is a ghost map
  (`harness/external/position_nft.rs`).
- **Other calls and storage.** A cross-contract call without a summary returns
  an unconstrained value, and position maps the fixture does not seed hold
  arbitrary values, not empty ones (see
  [Storage, context and external calls](../../README.md#storage-context-and-external-calls)).

## Rule modules

| Module | Proves |
|---|---|
| `account_isolation_rules.rs` | Supply, borrow, repay, and liquidation change only the target account |
| `boundary_rules.rs` | Bad-debt socialization threshold and the dust and force gates at the boundary |
| `consistency_rules.rs` | Supply and borrow store the position the pool returns |
| `flash_loan_rules.rs` | Flash-loan guard blocks re-entry into supply and liquidation and clears after the pool returns |
| `health_rules.rs` | Borrow and withdraw are safe or health-gated. The gate sees final totals. Repay improves an unhealthy account |
| `hf_lemma_rules.rs` | Position value is monotone in shares. Ceil value ≥ floor value |
| `index_rules.rs` | View and accrue agree on the projected index. Indexes are monotone in time |
| `liquidation_rules.rs` | Liquidation grows neither debt nor seized collateral. Bonus bounds. No estimation dust. Split partials never out-seize one close |
| `market_guard_rules.rs` | Disabled markets, collateral-less borrow, and delegate and recipient checks |
| `position_rules.rs` | Each verb moves the position in its own direction only |
| `solvency_rules.rs` | LTV borrow bound, zero-amount rejects, position limits, and one index snapshot per call |
| `spoke_rules.rs` | Spoke asset registration, deprecated spokes, and parameter bounds |
| `strategy_rules.rs` | Multiply, swap collateral, swap debt, repay with collateral, flash position, bad debt, and revenue |

These files hold no rules. `compat.rs` has single-asset shims for multi-asset
entrypoints. `health_ghost.rs` has the post-pool solvency gate record.
`fixture.rs` seeds the protocol, markets, and accounts. `harness/` is the
harness directory.

Revert-shaped rules live in `-reverts` confs with witnesses in
`-reverts-sanity` confs ([Sanity checking on WASM](../../README.md#sanity-checking-on-wasm)).

## Proof order

Prove the controller's lemmas first:

1. `common/confs/math.conf` and `rates.conf`
2. `price-aggregator/confs/tolerance-math.conf`
3. `controller/confs/solvency-*.conf` and `liquidation.conf`
4. The `heavy` profile
