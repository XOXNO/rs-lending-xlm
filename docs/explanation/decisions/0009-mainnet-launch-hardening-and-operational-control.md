# ADR 0009: Mainnet Launch Hardening And Operational Control

- Status: Accepted
- Date: 2026-05-06
- Deciders: XOXNO Lending contract team

## Context

Mainnet changes the loss profile. Protocol controls for pause, timelock, roles,
and per-spoke caps already exist (ADR 0010, ADR 0011, spoke cap enforcement).
What remains is **when** to open public liquidity and **how much** exposure to
allow at first open.

## Decision

Public mainnet unpause only after readiness evidence for a **target commit** and
**target contract addresses**, and only under a **capped** first exposure.

### Protocol preconditions (enforced on-chain)

Documented fully in ADR 0010 / 0011 and the contracts:

- Controller starts paused; upgrades re-pause.
- Governance owns the controller; non-emergency admin is timelocked.
- GUARDIAN pauses immediately; resume is `AdminOperation::Unpause` only.
- Per-spoke `supply_cap` / `borrow_cap` (asset units) bound listing exposure.
- Governance roles: `PROPOSER`, `EXECUTOR`, `CANCELLER`, `ORACLE`, `GUARDIAN`.
  Delegated `EXECUTOR` and `CANCELLER` are not the same address.

### Go-live policy (ops)

Before public unpause:

1. Audit items for launch scope are closed, accepted with rationale, or deferred
   in writing.
2. Verification matrix in [docs/reference/architecture.md](../../reference/architecture.md)
   has been run on the target commit; results recorded.
3. Testnet soak: 14 consecutive days without unresolved P0/P1, unexplained
   accounting drift, stale TTL, or oracle config drift.
4. Live governance `min_delay` is at the production floor
   (`TIMELOCK_MIN_DELAY_LEDGERS = 34_560`). Constructor allows a shorter
   bootstrap delay; operators raise delay before go-live (see
   [DEPLOYMENT.md](../../how-to/deploy-and-operate.md)).
5. Production roles and incident keys (including GUARDIAN) are assigned;
   deployer no longer holds residual authority.
6. Monitoring covers caps, reserves, oracle freshness/deviation, health-factor
   distribution, liquidations, bad debt, indexes, TTL, timelock ops, and revenue.
7. Pause drill: GUARDIAN pause, gated mutations rejected, then
   `propose(Unpause)` → await → execute.
8. Keeper config lists every launched `HubAssetKey`.

### Initial exposure policy

USD figures are **off-chain launch policy**. On-chain enforcement is only
per-spoke `supply_cap` / `borrow_cap` in asset units. Operators must set those
caps to implement the budget; there is no protocol-wide TVL or total-borrow
aggregate.

| Budget | USD policy |
|--------|------------|
| Total TVL | 250,000 |
| Total borrow | 100,000 |
| Per-market supply | 100,000 |
| Per-market borrow | 50,000 |

Raise caps only after at least 7 consecutive days without unresolved P0/P1,
accounting drift, oracle misconfig, or missed keeper/TTL work.

### Launch complete when

All go-live items above are satisfied, mainnet is unpaused via timelocked
`Unpause`, initial spoke caps match the launch budget, and the protocol has run
7 consecutive days capped without unresolved P0/P1, accounting drift, or missed
keeper/TTL work.

## Alternatives considered

- Unpause after smoke tests only.
- Launch uncapped after audit.
- Off-chain notice for admin changes.
- Timelocked emergency pause.
- Immediate unpause.
- Single operator key for all authority.

## Consequences

**Positive:** go-live is evidence-gated; early exposure is intentionally small;
resume cannot skip the timelock.

**Costs:** launch takes longer; low caps limit early demand; operators must keep
spoke caps aligned with the USD budget (no aggregate on-chain check).

## References

- [ADR 0010](./0010-governance-timelock-for-controller-admin.md)
- [ADR 0011](./0011-pause-and-freeze-matrix.md)
- [DEPLOYMENT.md](../../how-to/deploy-and-operate.md)
- [docs/reference/architecture.md](../../reference/architecture.md)
