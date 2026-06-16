# ADR 0009: Mainnet Launch Hardening and Operational Control

- Status: Accepted
- Date: 2026-05-06
- Revised: 2026-06-16
- Deciders: XOXNO Lending contract team
- Supersedes: original launch policy that used off-chain notice instead of an
  enforced governance timelock

## Context

Moving a lending protocol from testnet to mainnet changes the risk profile.
Misconfiguration, oracle outages, liquidation edge cases, privileged-key
mistakes, stale TTL windows, or delayed incident response can create real user
losses once liquidity arrives.

The current protocol has runtime controls:

- the controller starts paused,
- controller upgrade auto-pauses,
- governance owns the controller,
- governance timelocks protocol-affecting admin changes,
- pause and unpause remain immediate emergency actions,
- the controller owns one central pool,
- oracle reads use per-flow strict and permissive policies,
- keepers can update indexes, propagate thresholds, and clean bad debt,
- account owners can renew their own account TTL,
- the keeper service can extend ledger-entry TTL permissionlessly off-chain.

The launch policy defines when those controls are sufficient for mainnet users
and how exposure grows after launch.

## Decision

Launch mainnet through a hardening gate and capped rollout. The protocol is not
unpaused for public access until audit closure, verification evidence, testnet
soak, governance setup, monitoring, and pause-drill checks have all passed for
the target deployment commit and deployed contract addresses.

### Launch Gates

The launch candidate must satisfy all gates before mainnet unpause:

- External audit findings for the target branch are closed, accepted with
  documented rationale, or explicitly deferred from launch scope.
- The verification acceptance matrix in `SCF_BUILD_ARCHITECTURE.md` is run
  against the target commit, and results are recorded in launch evidence.
- The configured testnet deployment runs for 14 consecutive days with no
  unresolved P0/P1 incidents, no unexplained accounting drift, no stale TTL
  windows, and no oracle configuration drift.
- Governance is deployed, owns the controller, and has the mainnet delay set to
  `TIMELOCK_MIN_DELAY_LEDGERS = 34_560`.
- Governance `PROPOSER`, `EXECUTOR`, and `CANCELLER` duties are assigned before
  public unpause. Delegated `EXECUTOR` and `CANCELLER` roles must not be held by
  the same address; the contract enforces this for delegated grants.
- Controller `KEEPER`, `REVENUE`, and emergency `ORACLE` roles are assigned to
  the intended operational addresses.
- Monitoring and alerting are live for market caps, central-pool reserves,
  oracle freshness/deviation, health-factor distribution, liquidatable
  accounts, bad-debt events, index freshness, TTL windows, revenue claims,
  timelock operations, and privileged calls.
- A pause drill is completed on testnet: pause through governance, verify user
  mutations reject, keep required operator views/checks reachable, apply a
  benign config or runbook step if needed, and unpause.

### Initial Mainnet Caps

Initial exposure is intentionally small:

- Total protocol TVL cap: USD 250,000.
- Total protocol borrow cap: USD 100,000.
- Per-market supply cap: USD 100,000.
- Per-market borrow cap: USD 50,000.
- Flash-loan exposure is bounded by the central pool's available `cash` for
  that asset and by per-market launch caps.

These USD figures are off-chain launch policy. On-chain, each market enforces
per-asset `supply_cap` and `borrow_cap` denominated in asset units via
`enforce_supply_cap` and `enforce_borrow_cap` (`contracts/pool/src/utils.rs`).
Operators set those unit caps to realize the per-market USD policy. There is no
protocol-wide TVL or borrow USD cap in contract code.

Caps may be raised only after each stage runs for at least 7 consecutive days
without unresolved P0/P1 incidents, unexplained accounting drift, oracle
misconfiguration, or missed keeper/TTL maintenance. Each increase requires an
operator review of liquidity, utilization, liquidatable accounts, oracle
quality, timelock queue state, and incident history.

### Role and Authority Policy

Mainnet authority is separated by responsibility:

- Governance owner must be a multisig or equivalent multi-party custody setup.
  The deployer key must not retain launch authority after ownership and roles
  are assigned.
- Governance owns the controller in production. Direct controller owner
  authority is therefore exercised by governance, not by a hot operator key.
- Controller `KEEPER`, `REVENUE`, and emergency `ORACLE` roles must be held by
  separate operational addresses or automation identities where practical. A
  single hot key must not hold every controller role.
- Governance `PROPOSER`, `EXECUTOR`, and `CANCELLER` roles should be separated
  operationally. The owner may retain full recovery authority, but delegated
  executor and canceller accounts must be distinct.
- Non-emergency protocol changes are scheduled through governance typed
  proposers and wait the on-chain timelock delay before execution.
- Emergency pause remains immediate. Governance owner may pause without delay
  for oracle incidents, accounting anomalies, suspected exploit activity,
  privileged-key compromise, or severe market stress.

### Mainnet Launch Completion

Mainnet launch completion is not defined by smoke tests alone. It is complete
when:

- the target mainnet deployment passed all launch gates,
- governance owns the controller and the central pool is deployed,
- the capped mainnet deployment is unpaused,
- monitoring and operational runbooks are live,
- initial caps are enforced on all listed markets, and
- the protocol completes 7 consecutive days of capped mainnet operation with no
  unresolved P0/P1 incident, no unexplained accounting drift, and no missed
  keeper or TTL maintenance window.

## Alternatives Considered

- **Unpause after deployment smoke tests only.** Rejected: smoke tests prove
  basic wiring, not operational readiness under real liquidity, oracle
  variance, liquidations, keeper work, or privileged-key procedures.
- **Launch uncapped after audit.** Rejected: even a clean audit does not remove
  configuration, oracle, integration, and operational risks. Capped exposure
  creates an observation window before allowing TVL growth.
- **Off-chain notice for non-emergency admin changes.** Superseded: notice is
  useful, but it is not an enforcement mechanism. Governance timelock is now
  the load-bearing delay.
- **Timelock emergency pause.** Rejected: delaying a halt of a compromised
  market converts a containable incident into a loss.
- **Single operator key for all roles.** Rejected: it concentrates upgrade,
  oracle, keeper, revenue, proposal, execution, and cancellation authority in
  one compromise domain.

## Consequences

Positive:

- Mainnet launch uses measurable criteria instead of relying on deployment
  smoke tests alone.
- Initial user exposure is bounded while mainnet network, oracle, keeper,
  liquidation, central-pool, and monitoring behavior are observed.
- Governance timelock gives users and integrators an enforced observation
  period for protocol-affecting changes.
- Immediate pause remains available for incidents.
- Role separation reduces the blast radius of a single operational-key
  compromise.

Negative / accepted costs:

- Launch takes at least the 14-day testnet soak plus the 7-day capped mainnet
  observation window before exposure can grow materially.
- Low initial caps may limit early user demand and protocol revenue.
- Timelocked admin changes slow routine non-emergency operations.
- More operational identities must be maintained, monitored, and rotated.

## References

- `SCF_BUILD_ARCHITECTURE.md` §13 (Access Control and Operations), §16
  (Verification Surface), §17 (Deployment and Operations), §19 (Status).
- `contracts/governance/src/{access.rs,forward.rs,timelock.rs,self_timelock.rs}`
- `contracts/controller/src/governance/access.rs`
- `contracts/controller/src/router.rs`
- `contracts/controller/src/storage/ttl.rs`
- `architecture/INVARIANTS.md`
- ADR 0010 (Governance Timelock for Protocol Admin)
- `certora/README.md`
