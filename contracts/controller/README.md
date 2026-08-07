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

Listing halt flags (`paused` / `frozen`): `set_spoke_asset_flags` ratchets
(immediate GUARDIAN may only tighten). Clearing flags is intentional via
owner-only `edit_asset_in_spoke` (governance timelocks that op). See
[`contracts/governance/README.md`](../governance/README.md).

Full semantics: rustdoc on the controller `contractimpl` and
[`interfaces/controller`](../../interfaces/controller).
Shared model: [`skills/lending-protocol-fundamentals`](../../skills/lending-protocol-fundamentals/SKILL.md).
Protocol math (HF, bonus curve, close size, seize, bad debt):
[`docs/reference/formulas.md`](../../docs/reference/formulas.md).
