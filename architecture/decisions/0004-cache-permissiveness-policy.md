# ADR 0004: Cache Permissiveness Policy for Oracle Failures

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

ADR 0003 sets up a two-source oracle with tolerance bands. A second
question follows: when the bands are exceeded or a feed is stale, what is
the protocol response?

A symmetric "always revert" policy halts the protocol on any oracle
hiccup, including risk-decreasing flows that the user could otherwise
use to save themselves. A symmetric "always allow" policy lets risk
increase under degraded pricing. Neither is right.

The protocol must additionally handle disabled markets: when a market is
withdrawn from active use (`MarketStatus::Disabled`), users still need a
path to repay debt and the protocol still needs read paths for
accounting.

## Decision

Centralize the policy in `OraclePolicy`
(`controller/src/oracle/policy.rs`) and pass it through `ControllerCache`
(`controller/src/cache/mod.rs`):

- `OraclePolicy::RiskIncreasing` — strict pricing for paths that can add
  borrow risk or liquidate an account.
- `OraclePolicy::RiskDecreasing` — permissive pricing for paths that reduce
  risk or only move supply-side state.
- `OraclePolicy::Repay` — permissive pricing plus disabled-market pricing for
  normal repay.
- `OraclePolicy::IsolatedRepay` — disabled-market pricing with strict
  stale/deviation/TWAP gates for isolated repay.
- `OraclePolicy::View` — read-only disabled-market/permissive pricing.

Per-flow assignment (`controller/src/positions/*.rs`,
`controller/src/strategy.rs`, `controller/src/flash_loan.rs`):

| Flow                              | Oracle policy                      |
| --------------------------------- | ---------------------------------- |
| `borrow`                          | `RiskIncreasing`                   |
| `liquidate`                       | `RiskIncreasing`                   |
| `multiply`, `swap_*`              | `RiskIncreasing`                   |
| `repay_debt_with_collateral`      | `RiskIncreasing`                   |
| `update_account_threshold`        | `RiskIncreasing`                   |
| `supply`                          | `RiskDecreasing`                   |
| `flash_loan`                      | `RiskDecreasing`                   |
| `update_indexes`                  | `RiskDecreasing`                   |
| `claim_revenue`, `add_rewards`    | `RiskDecreasing`                   |
| `withdraw` (debt-free)            | `RiskDecreasing`                   |
| `withdraw` (with debt)            | `RiskIncreasing`                   |
| `repay` (non-isolated)            | `Repay`                            |
| `repay` (isolated)                | `IsolatedRepay`                    |
| views                             | `View`                             |

`OraclePolicy` controls the gates inside the oracle module:

- **Deviation band** (`oracle::calculate_final_price`): outside the last
  band, strict caches revert (`OracleError::UnsafePriceNotAllowed`);
  permissive caches return the safe price.
- **Staleness** (`oracle::sources`): strict policies revert
  (`OracleError::PriceFeedStale`); permissive caches accept the feed.
- **Missing TWAP history** (`oracle::sources`): strict policies fail closed;
  explicitly permissive policies may fall back where the source function has
  a valid spot fallback.
- **Disabled markets** (`oracle::token_price`): normal policies reject;
  `Repay`, `IsolatedRepay`, and `View` allow pricing.

The clock-skew gate (`check_not_future`, ±60s) is unconditional and
applies in every mode.

For `repay`, isolation accounts use the strict deviation/staleness gates
because the global `IsolatedDebt(asset)` counter is updated in USD WAD
and would drift if priced under a degraded feed
(`controller/src/positions/repay.rs`).

## Alternatives Considered

- **Always strict.** Rejected: a Reflector outage halts the protocol,
  including the supply, repay, and withdraw paths users rely on to
  reduce risk. Borrowers in trouble lose their last self-help action.
- **Always permissive.** Rejected: degrades the oracle gate to advisory.
  A manipulated feed could be used to take out a borrow that the strict
  gate would have blocked.
- **Per-asset switch instead of per-flow.** Rejected: the policy is about
  what the operation does to risk, not about the asset. The same asset
  appears on both sides of the same transaction.
- **Caller-specified flag.** Rejected: makes oracle policy part of user
  input, which would either need its own validation or invite abuse.

## Consequences

Positive:

- Risk-increasing flows fail closed under degraded pricing.
- Risk-decreasing flows (supply more collateral, repay debt) keep working
  during a Reflector outage; users can still save their position.
- Disabled markets remain repayable through `repay`, while normal risk
  ops are blocked.
- The policy lives in one file (`cache/mod.rs`) and one decision site
  (`oracle::calculate_final_price`), which is the natural target for
  formal verification.

Negative / accepted costs:

- Multiple policy variants remain part of the verification surface, but the
  risk semantics are explicit instead of encoded as boolean combinations.
- Permissive caches accept the safe price even when both sources have
  drifted in the same direction. The two-source design (ADR 0003)
  bounds this risk, and isolated debt opts out of the relaxation.

## References

- `SCF_BUILD_ARCHITECTURE.md` §9 (Oracle Pricing — cache modes).
- `controller/src/cache/mod.rs::{new, new_view}`
- `controller/src/oracle/policy.rs`
- `controller/src/oracle/{sources.rs,tolerance.rs}`
- `controller/src/positions/repay.rs`
- `controller/src/positions/withdraw.rs`
