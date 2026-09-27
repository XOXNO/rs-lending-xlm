# Controller verification

The controller rules prove entrypoint gates, account isolation, position
direction, solvency, liquidation and strategy properties.

## Assumptions

A controller verdict is conditional on these models:

- **Pool.** Calls resolve to the summaries in `shared/summaries/pool.rs`
  through `harness/external/pool.rs`. The summaries are assumed, not proved.
- **Prices and indexes.** Each asset has one price and each market one index
  pair per rule (`harness/ghost_prices.rs`), so all health checks in a rule use
  one valuation. Prices come from `price_feed_summary`: positive and never
  failing. The fail-closed oracle behavior is proved in
  `price-aggregator/spec/`.
- **Position NFT.** Ownership is a ghost map
  (`harness/external/position_nft.rs`).
- **Other calls.** The prover does not implement a cross-contract `call`; it
  returns an unconstrained value. A rule that skips a summary sees any result.
- **Storage.** Sunbeam havocs storage. Position maps that the fixture does not
  seed hold arbitrary values, not empty ones.

## Rule modules

| Module | Proves |
|---|---|
| `account_isolation_rules.rs` | Supply, borrow, repay and liquidation change only the target account |
| `boundary_rules.rs` | Bad-debt socialization threshold and the dust and force gates at the boundary |
| `consistency_rules.rs` | Supply and borrow store the position the pool returns |
| `flash_loan_rules.rs` | The flash-loan guard blocks re-entry into supply and liquidation, and clears after the pool returns |
| `health_rules.rs` | Borrow and withdraw are safe or health-gated; the gate sees final totals; repay on an unhealthy account improves health |
| `hf_lemma_rules.rs` | Position value is monotone in shares; ceil value is at least floor value |
| `index_rules.rs` | View and accrue return the same projected index; indexes are monotone in time |
| `liquidation_rules.rs` | Liquidation does not grow debt or seized collateral; bonus bounds; estimation leaves no dust; split partials never out-seize one close |
| `market_guard_rules.rs` | Disabled markets, collateral-less borrow, delegate and recipient checks |
| `position_rules.rs` | Each verb moves the position in its own direction only |
| `solvency_rules.rs` | LTV borrow bound, zero-amount rejects, position limits, one index snapshot per call |
| `spoke_rules.rs` | Spoke asset registration, deprecated spokes, parameter bounds |
| `strategy_rules.rs` | Multiply, swap collateral, swap debt, repay with collateral, flash position, bad debt, revenue |

Support modules without rules: `compat.rs` (single-asset shims for
multi-asset entrypoints), `health_ghost.rs` (record of the post-pool solvency
gate), `fixture.rs` (protocol, market and account seeding), and `harness/`.

To find the confs of a module: `grep -l <rule_name> certora/controller/confs/*.conf`.
Revert-shaped rules live in `-reverts` confs with witnesses in
`-reverts-sanity` confs; see "Sanity checking on WASM" in
[certora/README.md](../../README.md#sanity-checking-on-wasm).

## Proof order

Prove the lemmas the controller depends on first:

1. `common/confs/math.conf` and `rates.conf`
2. `price-aggregator/confs/tolerance-math.conf`
3. `controller/confs/solvency-*.conf` and `liquidation.conf`
4. the `heavy` profile
