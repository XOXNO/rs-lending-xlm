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
| `upgrade` | Replace contract Wasm |
| Views | Checkpoint util, cash, rates, amounts; `get_bulk_indexes` simulates live |

Full semantics: rustdoc on the `LiquidityPoolInterface` impl in `src/lib.rs`.
Protocol properties: [`architecture/INVARIANTS.md`](../../architecture/INVARIANTS.md).
Ownership boundary: [ADR 0001](../../architecture/decisions/0001-controller-pool-ownership-boundary.md).
