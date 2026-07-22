# Controller

User-facing lending surface. Owns accounts, spokes, risk (health factor, caps,
pause/freeze), liquidation, strategies, and flash loans. Prices via the
price-aggregator; liquidity via the pool it owns.

| | |
| --- | --- |
| Mutators (user) | auth on `caller` / account owner or delegate |
| Admin | `#[only_owner]` → governance after deploy |
| Interfaces | `interfaces/controller` |
| Math / types | `common` |

## Role

```text
User / strategies ──► Controller ──► Pool (only_owner)
                         │
                         ├──► PriceAggregator (prices)
                         └──► SwapAggregator (strategy routes; untrusted)
```

Risk, HF, spokes, and account maps live here. The pool only moves cash and
indexes under controller-gated mutators
([ADR 0001](../../architecture/decisions/0001-controller-pool-ownership-boundary.md)).

## Surface

| Area | Entrypoints (selection) |
| --- | --- |
| Positions | `supply`, `borrow`, `withdraw`, `repay`, `liquidate` |
| Strategies | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend`, `flash_loan` |
| Account | `add_delegate`, `remove_delegate`, `renew_account` |
| Views | HF, totals, positions, spoke config/usage, market indexes, liq estimates |
| Admin | hubs/spokes/assets, pool deploy/upgrade, pause, aggregators, limits |

## Layout

```text
src/
  positions/    Supply, borrow, withdraw, repay, liquidation
  strategies/   Flash loan, multiply, swaps, blend migrate
  risk/         HF params, totals, post-action validation
  spoke/        Caps and spoke usage
  config/       Hub/spoke/asset/registry admin
  context/      Tx cache (pool, oracle, spoke, indexes)
  storage/      Account, hub, spoke, protocol, session, TTL
  views/        Limits and aggregates
  external/     Pool, SAC, price-aggregator, blend clients
  governance/   Owner-gated access helpers
```

## Related

| Doc | Topic |
| --- | --- |
| [ADR 0001](../../architecture/decisions/0001-controller-pool-ownership-boundary.md) | Gov / controller / pool boundary |
| [ADR 0005](../../architecture/decisions/0005-strategy-aggregator-output-validated-by-balance-delta.md) | Strategy swap trust |
| [ADR 0011](../../architecture/decisions/0011-pause-and-freeze-matrix.md) | Pause / freeze |
| [ADR 0012](../../architecture/decisions/0012-per-spoke-liquidation-curve.md) | Liquidation curve |
| `contracts/pool` | Liquidity engine |
| `contracts/governance` | Timelocked owner |
