# Liquidity Pool

Owner-gated market engine: interest, scaled shares, tracked cash per
`(hub_id, asset)`. Controller owns risk; this contract moves liquidity under
its own math.

| Entrypoint | Role |
| --- | --- |
| `create_market` | Init params + zeroed state |
| `supply` / `borrow` / `withdraw` / `repay` | Mint/burn scaled shares; cash ± |
| `net_settle` | Same-market supply vs debt, no transfer |
| `seize_positions` | Bad-debt index write-down or deposit → revenue |
| `add_rewards` / `claim_revenue` | Supply-index rewards; burn revenue shares |
| `flash_loan` / `create_strategy` | Callback lend / strategy borrow + fee |
| `update_indexes` / `update_params` | Accrue; optional IRM replace |
| `reconcile_reserves` | Realign tracked cash after an issuer clawback |
| `upgrade` | Replace contract Wasm |
| Views | Checkpoint util, cash, rates, amounts; `get_bulk_indexes` simulates live |

## Source map

| Path | Owns |
| --- | --- |
| `src/lib.rs` | Module declarations and the ABI; every entrypoint delegates |
| `src/ops/` | One module per entrypoint, end to end |
| `src/cache.rs` | `Cache`: load a market, mutate by named transition, commit |
| `src/interest.rs` | Every index movement: accrual, revenue, rewards, bad debt |
| `src/guards.rs` | Utilization and solvency checks before a mutation persists |
| `src/storage.rs` | The only place `PoolKey` is constructed, read, written, renewed |
| `src/views.rs` | Checkpoint reads behind the view ABI |
| `src/events.rs` | Batched market-state and params events |
| `src/time.rs` | Ledger clock in milliseconds |

Full semantics: rustdoc on the `LiquidityPoolInterface` impl in `src/lib.rs`.
Protocol properties: [`docs/reference/invariants.md`](../../docs/reference/invariants.md).
Ownership boundary: [ADR 0001](../../docs/explanation/decisions/0001-controller-pool-ownership-boundary.md).
