# Controller

User-facing lending surface: accounts, spokes, risk, liquidation, strategies,
flash loans. Prices via price-aggregator; liquidity via the pool it owns.

| Area | Entrypoints (selection) |
| --- | --- |
| Positions | `supply`, `borrow`, `withdraw`, `repay`, `liquidate`, `clean_bad_debt` |
| Strategies | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend`, `flash_loan` |
| Account | `add_delegate`, `remove_delegate`, `renew_account` |
| Views | HF, totals, positions, spoke config/usage, market indexes, liq estimates |
| Admin | hubs/spokes/assets, pool deploy/upgrade, pause, aggregators, limits |

Auth: user mutators require `caller` auth (owner or opted-in delegate + active
position manager). Admin is `#[only_owner]` (governance after deploy).

Full semantics: rustdoc on the controller `contractimpl` and
[`interfaces/controller`](../../interfaces/controller).
Protocol properties: [`docs/reference/invariants.md`](../../docs/reference/invariants.md).
Doc style: [`docs/reference/doc-style.md`](../../docs/reference/doc-style.md).

## Related

| Doc | Topic |
| --- | --- |
| [ADR 0001](../../docs/explanation/decisions/0001-controller-pool-ownership-boundary.md) | Gov / controller / pool boundary |
| [ADR 0005](../../docs/explanation/decisions/0005-strategy-aggregator-output-validated-by-balance-delta.md) | Strategy swap trust |
| [ADR 0011](../../docs/explanation/decisions/0011-pause-and-freeze-matrix.md) | Pause / freeze |
| [ADR 0012](../../docs/explanation/decisions/0012-per-spoke-liquidation-curve.md) | Liquidation curve |
| [DOC_STYLE](../../docs/reference/doc-style.md) | Public ABI comment style |
