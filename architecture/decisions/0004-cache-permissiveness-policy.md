# ADR 0004: Oracle Policy By Flow

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-06-30
- Deciders: XOXNO Lending contract team

## Context

Oracle degradation should not have one global response. Strict behavior on every
flow can trap users during an oracle outage. Permissive behavior on every flow
can allow risk to increase under bad pricing.

The current protocol has no separate market-status enum. Price activation is
represented by the presence of token-rooted `AssetOracle(asset)` configuration.
Removing that entry disables price resolution for the asset.

## Decision

Centralize oracle behavior in `OraclePolicy` and pass it through the transaction
cache:

- `RiskIncreasing`: strict pricing for flows that can increase account or system
  risk.
- `RiskDecreasing`: permissive pricing for flows that reduce account risk or do
  not need strict price safety.
- `Repay`: repay-specific path with no load-bearing oracle dependency.
- `Liquidation`: strict pricing for liquidation and bad-debt cleanup.
- `View`: read-only path that should not overstate executable behavior.

Strict policies fail closed on missing, stale, future-dated, out-of-sanity, or
out-of-tolerance prices. Permissive policies can allow degraded reads only where
the flow cannot increase risk.

## Flow Assignment

| Flow | Policy |
| --- | --- |
| `supply` | `RiskDecreasing` |
| `borrow` | `RiskIncreasing` |
| `withdraw` without debt | `RiskDecreasing` |
| `withdraw` with debt | `RiskIncreasing` |
| `repay` | `Repay` |
| `liquidate` | `Liquidation` |
| strategy flows | policy selected by the risk effect of each leg |
| price-resolving views | `View` |

## Alternatives Considered

- **Strict for every flow.** Rejected because users need de-risk paths during
  oracle outages.
- **Permissive for every flow.** Rejected because borrows and liquidations must
  not proceed under degraded pricing.
- **Per-asset switch.** Rejected because the risk effect is defined by the flow,
  not the asset.
- **Caller-specified policy.** Rejected because oracle safety must not be user
  input.

## Consequences

Positive:

- Risk-increasing flows fail closed.
- Repay and supported de-risk flows remain available when they do not rely on
  strict prices.
- Oracle behavior is explicit at each call path.

Accepted costs:

- Every new entrypoint must choose the correct policy and tests must cover that
  choice.
- Views must stay conservative and must not be treated as proof that a mutation
  will succeed.

## References

- `contracts/controller/src/oracle`
- `contracts/controller/src/context`
- `contracts/controller/src/positions`
