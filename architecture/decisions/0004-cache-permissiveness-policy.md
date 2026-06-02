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
(`contracts/controller/src/oracle/policy.rs`) and pass it through `Cache`
(`contracts/controller/src/cache/mod.rs`):

- `OraclePolicy::RiskIncreasing` — strict pricing for paths that can add
  borrow risk or liquidate an account.
- `OraclePolicy::RiskDecreasing` — permissive pricing for paths that reduce
  risk or only move supply-side state.
- `OraclePolicy::Repay` — permissive pricing plus disabled-market pricing for
  normal repay.
- `OraclePolicy::IsolatedRepay` — disabled-market pricing with strict
  stale/deviation/TWAP gates for isolated repay.
- `OraclePolicy::Liquidation` — strict stale/deviation/TWAP gates for
  liquidation and standalone bad-debt cleanup. Its allowance table is
  byte-identical to `RiskIncreasing` (all four loosenings denied), kept a
  distinct variant for intent/auditing and guarded by
  `test_liquidation_matches_risk_increasing_allowances`. Inside the tolerance
  bands the standard selection applies (first band → safe/primary price, last
  band → midpoint); beyond the last band it reverts.
- `OraclePolicy::View` — read-only disabled-market/permissive pricing.

Per-flow assignment (`contracts/controller/src/positions/*.rs`,
`contracts/controller/src/strategies/`, `contracts/controller/src/strategies/flash_loan.rs`):

| Flow                              | Oracle policy                      |
| --------------------------------- | ---------------------------------- |
| `borrow`                          | `RiskIncreasing`                   |
| `liquidate`, `clean_bad_debt`     | `Liquidation`                      |
| `multiply`, `swap_debt`           | `RiskIncreasing`                   |
| `swap_collateral` (debt-free)     | `RiskDecreasing`                   |
| `swap_collateral` (with debt)     | `RiskIncreasing`                   |
| `repay_debt_with_collateral`      | `RiskIncreasing`                   |
| `update_account_threshold` (`has_risks`) | `RiskIncreasing`            |
| `update_account_threshold` (no risk change) | `RiskDecreasing`         |
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

- **Deviation band** (`oracle::tolerance::calculate_final_price`,
  `contracts/controller/src/oracle/tolerance.rs`): within the first band the
  safe (primary) price is used; within the last band the midpoint of the two
  prices is used; outside the last band, strict caches revert
  (`OracleError::UnsafePriceNotAllowed`) while caches whose policy
  `allows_unsafe_deviation` return the safe price. Only
  `OracleStrategy::PrimaryWithAnchor` markets reach this gate; `Single`-source
  markets return the primary directly.
- **Staleness** (`contracts/controller/src/oracle/compose.rs`,
  `validate_primary_freshness` / `anchor_is_usable`): strict policies revert
  (`OracleError::PriceFeedStale`); permissive caches accept the feed.
- **Missing anchor / degraded TWAP** (`allows_missing_twap_fallback`): two
  distinct degradations share this allowance. (a) When the anchor *source* is
  absent, unreadable, or stale-unusable, `compose::fallback_to_primary` drops
  the anchor and prices off the primary alone — strict policies instead fail
  closed with `OracleError::NoLastPrice`. (b) When a Reflector *TWAP read*
  cannot form a trusted average (empty / insufficient / stale history),
  `twap::twap_fallback_or_panic` panics with the underlying error under strict
  policy, or — under a permissive policy — emits `OracleTwapDegradedEvent` and
  returns the newest valid sample or a spot read, which then still serves as
  the anchor in tolerance selection.
- **Disabled markets** (`oracle::token_price`,
  `contracts/controller/src/oracle/price.rs`): normal policies reject;
  `Repay`, `IsolatedRepay`, and `View` allow pricing.

`token_price` additionally enforces policy-independent gates on every read:
positive price (`InvalidPrice`), the configured sanity band
(`SanityBoundViolated`), the `pending_for` self-pointer sentinel
(`OracleNotConfigured`), and `PendingOracle` markets (`PairNotActive`).

The clock-skew gate (`check_not_future_at`,
`contracts/controller/src/oracle/observation.rs`) is unconditional in every
mode: it rejects feed timestamps more than `MAX_FUTURE_SKEW_SECONDS` (a
one-sided 60s future bound) ahead of the ledger clock with
`OracleError::PriceFeedStale`.

For `repay`, isolation accounts use the strict deviation/staleness gates
because the global `IsolatedDebt(asset)` counter is updated in USD WAD
and would drift if priced under a degraded feed
(`contracts/controller/src/positions/repay.rs`).

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

- Risk-increasing flows fail closed under degraded pricing. `RiskIncreasing`
  and `Liquidation` deny all four allowances, so a missing, stale, or
  out-of-band source **reverts** rather than falling back to a single provider
  — **no single-source price can ever drive a borrow or a liquidation**, even if
  one provider is failed or removed while the other is manipulated. This is a
  load-bearing invariant: the per-provider fallback (`fallback_to_primary`,
  `allows_stale_source`, `allows_unsafe_deviation`) is reachable **only** from
  risk-decreasing/read-only policies, never from a flow that can add risk.
- Risk-decreasing flows (supply more collateral, repay debt) keep working when
  the feed is merely stale, deviating beyond a band, serving a degraded TWAP, or
  missing its anchor under a permissive policy, so users can still save their
  position. A fully missing *primary* feed still fails closed even on permissive
  paths (the primary read is `required` in `compose::resolve_components`): a
  missing Reflector primary surfaces `OracleError::NoLastPrice` (via
  `providers::read_source`), while a missing RedStone primary surfaces
  `GenericError::InvalidTicker` (`read_redstone_source` panics before the
  `NoLastPrice` fallback).
- Disabled markets remain repayable through `repay`, while normal risk
  ops are blocked.
- The allowance table is defined in one file
  (`oracle/policy.rs`, `Allowances::for_policy`) and threaded through `Cache`;
  the gates that read it are enforced across `oracle/price.rs`
  (disabled-market, in `token_price`), `oracle/compose.rs` (staleness, anchor
  fallback), and `oracle/tolerance.rs` (deviation) — a small, enumerable set of
  decision sites for formal verification.

Negative / accepted costs:

- Multiple policy variants remain part of the verification surface, but the
  risk semantics are explicit instead of encoded as boolean combinations.
- Permissive caches accept the safe price even when both sources have
  drifted in the same direction. The two-source design (ADR 0003)
  bounds this risk, and isolated debt opts out of the relaxation.

## Revisions

### 2026-05-27 — Liquidation hardens its deviation gate (reverses Codex adversarial-review finding #1)

The original posture (raised as "Codex adversarial-review finding #1" during
the 2026-05 security audit; that audit-findings tracker is external and not
part of this repository) had liquidation and standalone
bad-debt cleanup *tolerate* primary/anchor deviation: outside the last
tolerance band they resolved to the aggregator (live spot) price so
liquidations always proceeded during a flash crash. The stated reason was
that hard-blocking on deviation would DoS liquidators and leave underwater
accounts un-liquidatable, dumping bad debt on lenders.

That posture is now reversed. `OraclePolicy::Liquidation`
(`contracts/controller/src/oracle/policy.rs`) sets `unsafe_deviation: false`:
liquidation and bad-debt cleanup now **reject** with
`OracleError::UnsafePriceNotAllowed` (error #205, `common/src/errors.rs:144`)
when the primary and anchor sources diverge beyond the last tolerance band.
The protocol will not seize collateral at a price only one source
corroborates. While the sources stay inside the bands the standard selection
applies — the safe (primary) price within the first band, the midpoint of the
two prices within the last band; there is no separate aggregator-preference
flag.

Rationale (manipulation-over-availability tradeoff): the two oracle sources
are independent, so an attacker cannot sustain an out-of-band divergence
long enough to DoS liquidations — transient gaps fall inside the first two
tolerance bands, which remain tolerated (the first band resolves to the
safe/primary price, the last band to the midpoint of the two prices). Only
genuine extreme/out-of-band divergence rejects, and during a real flash
crash the block is transient (the anchor/TWAP catches up within its window),
so liquidations resume once the sources reconverge within tolerance. The
protocol accepts this narrow availability window in exchange for never
seizing collateral at a price only one source corroborates.

This refines, but does not contradict, the "Permissive caches accept the
safe price even when both sources have drifted" accepted cost above:
liquidation is no longer a permissive cache for the deviation gate.

Regression coverage:

- `test_clean_bad_debt_rejected_under_oracle_deviation`
  (`verification/test-harness/tests/keeper_tests.rs`)
- `test_unsafe_price_blocks_liquidation`
  (`verification/test-harness/tests/oracle_tolerance_tests.rs`)
- `test_liquidation_blocked_under_flash_crash`
  (`verification/test-harness/tests/oracle_tolerance_tests.rs`)

## References

- `SCF_BUILD_ARCHITECTURE.md` §9 (Oracle Pricing — `OraclePolicy` allowances).
- `contracts/controller/src/cache/mod.rs::{new, new_view}`
- `contracts/controller/src/oracle/policy.rs`
- `contracts/controller/src/oracle/{price.rs, compose.rs, tolerance.rs, observation.rs}`
- `contracts/controller/src/positions/repay.rs`
- `contracts/controller/src/positions/withdraw.rs`
