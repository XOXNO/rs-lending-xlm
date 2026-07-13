# ADR 0009: Mainnet Launch Hardening And Operational Control

- Status: Accepted
- Date: 2026-05-06
- Revised: 2026-06-30
- Deciders: XOXNO Lending contract team

## Context

Mainnet launch changes the risk profile. Misconfiguration, oracle outages,
liquidation edge cases, privileged-key mistakes, stale TTL windows, and delayed
incident response can cause real losses once liquidity arrives.

Current runtime controls:

- controller starts paused;
- controller upgrade auto-pauses;
- governance owns controller;
- governance timelocks non-emergency protocol-admin changes;
- pause and unpause remain immediate emergency actions;
- controller owns one central pool;
- oracle policy is selected per flow;
- keeper can renew and restore TTL and optionally call `update_indexes`;
- account owners can renew their own account TTL.

The current controller does not have `KEEPER`, `REVENUE`, or `ORACLE` roles.
Governance roles are `PROPOSER`, `EXECUTOR`, `CANCELLER`, `ORACLE`, and `GUARDIAN` (for immediate per-listing incident actions).

## Decision

Launch mainnet only through a hardening gate and capped rollout. The protocol is
not publicly unpaused until launch evidence exists for the target commit and
target contract addresses.

## Launch Gates

Before public mainnet unpause:

- External audit findings are closed, accepted with rationale, or deferred with
  an explicit launch-scope decision.
- The verification acceptance matrix in `SCF_BUILD_ARCHITECTURE.md` runs against
  the target commit and the results are recorded.
- Testnet runs for 14 consecutive days without unresolved P0/P1 incidents,
  unexplained accounting drift, stale TTL windows, or oracle configuration drift.
- Governance owns controller and the central pool is deployed.
- Mainnet timelock delay is set to `TIMELOCK_MIN_DELAY_LEDGERS = 34_560`.
- Governance `PROPOSER`, `EXECUTOR`, and `CANCELLER` duties are assigned before
  public unpause. Delegated `EXECUTOR` and `CANCELLER` roles must not be held by
  the same address.
- Monitoring and alerting cover market caps, reserves, oracle freshness and
  deviation, health-factor distribution, liquidatable accounts, bad-debt events,
  index freshness, TTL windows, timelock operations, privileged calls, and
  revenue claims.
- A testnet pause drill verifies pause, rejection of gated user mutations,
  required views/runbook checks, and unpause.
- Keeper configuration enumerates every launched `HubAssetKey`.

## Initial Mainnet Caps

Initial exposure stays small:

- Total protocol TVL cap: USD 250,000.
- Total protocol borrow cap: USD 100,000.
- Per-market supply cap: USD 100,000.
- Per-market borrow cap: USD 50,000.
- Flash-loan exposure is bounded by pool `cash` and per-market launch caps.

USD figures are off-chain launch policy. On-chain, caps are enforced per spoke
asset (`SpokeAssetConfig` `supply_cap` / `borrow_cap`). Operators set asset-unit
caps that implement the USD policy.

Caps may increase only after at least 7 consecutive days without unresolved
P0/P1 incidents, unexplained accounting drift, oracle misconfiguration, or missed
keeper/TTL maintenance.

## Authority Policy

- Governance owner must be a multisig or equivalent multi-party custody setup.
- Deployer keys must not retain launch authority after ownership and roles are
  assigned.
- Direct controller owner authority is exercised through governance, not a hot
  operator key.
- Non-emergency protocol changes use typed governance proposers and wait the
  on-chain timelock delay before execution.
- Emergency pause remains immediate.

## Launch Completion

Launch is complete only when:

- all launch gates are satisfied;
- capped mainnet deployment is unpaused;
- monitoring and runbooks are live;
- initial caps are enforced on all listed markets;
- the protocol completes 7 consecutive days of capped mainnet operation without
  unresolved P0/P1 incidents, unexplained accounting drift, or missed keeper/TTL
  maintenance.

## Alternatives Considered

- **Unpause after smoke tests only.** Rejected because smoke tests do not prove
  operational readiness.
- **Launch uncapped after audit.** Rejected because audit does not remove
  configuration, oracle, integration, or operational risk.
- **Off-chain notice for admin changes.** Superseded by enforced governance
  timelock.
- **Timelock emergency pause.** Rejected because delayed halt can turn an
  incident into a loss.
- **Single operator key for all authority.** Rejected because it concentrates
  upgrade, proposal, execution, cancellation, and emergency power.

## Consequences

Positive:

- Launch readiness is evidence-based.
- Early exposure is capped while operators observe real network behavior.
- Admin changes have an on-chain warning window.

Accepted costs:

- Launch takes longer.
- Low initial caps can limit early demand.
- Routine admin changes are slower.
- More operational identities need monitoring and rotation.

## References

- [SCF_BUILD_ARCHITECTURE.md](../../SCF_BUILD_ARCHITECTURE.md)
- [ADR 0010](./0010-governance-timelock-for-controller-admin.md)
- `contracts/governance/src`
- `contracts/controller/src/governance/access.rs`
- `contracts/controller/src/storage/ttl.rs`
