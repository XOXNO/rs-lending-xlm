# Threat model

## Purpose

This model identifies protected assets, actors, trust boundaries, attack
surfaces, and deliberate residual risks. It is a review guide, not a claim that
the listed controls are sufficient in every deployment.

## Protected assets

- Supplier claims and pool cash.
- Borrower collateral and account authority.
- Correct market indexes, debt totals, and revenue entitlement.
- Price integrity used for solvency and liquidation.
- Governance authority, timelock state, and emergency response.
- Protocol liveness during oracle failure, liquidation, and recovery.

## Actors

| Actor | Relevant capability |
|---|---|
| Anonymous caller | May invoke public user and maintenance actions with own authorization |
| Account owner | Controls risk-increasing actions for its account |
| Delegate | Acts only when both account-listed and governance-approved |
| Liquidator | Repays unhealthy debt for discounted collateral |
| Guardian | Immediately pauses or tightens restrictions |
| Governance roles | Schedule, execute, cancel, or recover delayed administration |
| Oracle operator | Supplies or configures external price observations |
| Keeper | Calls permissionless maintenance |
| Token contract | May have unusual transfer behavior |
| Router and venues | May be malicious, faulty, or economically poor |
| Flash receiver | Executes arbitrary callback logic |
| External vault | May present arbitrary caller and NAV behavior |

## Trust boundaries

### User to controller

User-controlled parameters, tokens, and routing intent are untrusted. The
controller authenticates callers and applies account, listing, price, and risk
rules before changing lending state.

### Controller to pool

The controller is the pool’s sole mutator. This prevents direct state changes
from skipping risk checks, but makes controller correctness and ownership
configuration critical.

### Controller to router and tokens

The router is untrusted. Token behavior is not assumed to match a requested
transfer amount. The controller measures balances, scopes pull authority, and
rechecks solvency after external work.

### Controller to price system

The controller trusts only a validated complete price snapshot. Source failure
or ambiguity halts valuation-dependent activity instead of choosing a fallback.

### Governance to protocol

Governance is trusted to set policy, but its ordinary power is delayed. The
guardian is intentionally more available but less powerful: it can tighten,
not reopen.

## Major attack surfaces

### Account operations

Attempted theft: borrow against or withdraw another account.

Controls: owner-or-delegate authorization, double-gated delegation, immutable
spoke binding, post-operation risk gates, and limits on third-party supply.

Review: test every account verb with owner, delegate, former delegate,
unrelated caller, and permissionless caller.

### Accounting and non-standard tokens

Attempted theft: claim more value than a fee-on-transfer or unusual token
delivered.

Controls: measured inbound receipt, internal cash accounting, zero-share
rejection, backing-shortfall gate, and conservative rounding.

Review: model partial delivery, direct donation, unexpected callback behavior,
dust values, and all intermediate strategy transfers.

### Oracle integrity

Attempted theft: manipulate, stale, partially fail, or race a price source.

Controls: source admission policy, freshness and sanity checks, dual-source
agreement, one snapshot per mutation, and fail-closed consumption.

Review: include one good leg, one bad leg, both stale, boundary timestamps,
large deviation, and assets with tiny positions that still require a price.

### Liquidation

Attempted theft: liquidate a healthy account, over-seize collateral, or exploit
under-delivered repayment.

Controls: health gate, self-liquidation ban, close bound, bonus and fee limits,
measured repayment, proportional seizure scaling, and explicit bad-debt gates.

Review: test close boundaries, dust, partial token delivery, paused debt,
unpriceable collateral, and residual debt.

### Flash loans and reentrancy

Attempted theft: leave the pool underpaid or call protected paths during a
callback.

Controls: contract receiver requirement, exact balance assertions,
allowance-based repayment, transaction rollback, and shared monetary
reentrancy protection.

Review: use receivers that underpay, push rather than approve, reenter every
public path, revert late, or alter token balances.

### Strategies and route execution

Attempted theft: router overspend, retain input, claim output dishonestly, or
leave the account unhealthy.

Controls: exact pull authorization, balance-delta settlement, residue return,
positive output requirement, and final solvency gate.

Review: test malicious return data, partial pull, no output, dust output,
callback reentry, and multi-leg routes.

### Governance and emergency response

Attempted theft: use a hot key to reopen risk, bypass delay, replay an
operation, or deadlock cancellation.

Controls: typed scheduling, payload identity, readiness and expiry checks,
role separation, delete-on-execute behavior, recovery handling, and the
guardian ratchet.

Review: examine deployment delay configuration separately from the code. A
timelock with a low configured delay is a live operational risk.

## Availability risks

The protocol chooses safety over availability in several cases:

- A price outage or one unusable dual-source leg blocks valuation-dependent
  actions, including liquidation.
- A paused debt listing can block its liquidation leg.
- Bounded position counts and route payload limits constrain resource use but
  may reject otherwise valid large actions.
- External token, router, oracle, and vault failures can revert a transaction.

These are intentional trade-offs that operators must monitor and govern.

## Accepted residual risks

- Governance and configured role keys remain high-value operational trust.
- Route quality is not fully optimized on-chain; a valid but poor route can
  lose value within the account’s permitted risk envelope.
- External oracle sources and token implementations are outside protocol
  control.
- Formal models and tests have explicit assumptions; they do not prove
  arbitrary cross-contract behavior or deployed configuration.

## Audit priorities

1. Verify deployment ownership, roles, delay, and active configuration.
2. Trace value through every external call to its final accounting entry.
3. Fuzz arithmetic boundaries, share rounding, and liquidation composition.
4. Treat liveness failures as security findings when they can trap collateral
   or prevent liquidation under plausible conditions.
