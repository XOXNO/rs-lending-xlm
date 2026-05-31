# ADR 0009: Mainnet Launch Hardening and Operational Control

- Status: Accepted
- Date: 2026-05-06
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

Moving a lending protocol from testnet to mainnet changes the dominant risk
profile. On testnet, failures are primarily engineering feedback. On mainnet,
misconfiguration, oracle outages, liquidation edge cases, privileged-key
mistakes, or delayed incident response can convert into real user losses
quickly as TVL accumulates.

The protocol already has runtime safety controls: the controller starts
paused, owner-gated upgrades auto-pause the controller, pools are owned by
the controller, oracle pricing has strict and permissive cache modes, and
keepers can update indexes, extend TTLs, update thresholds, and clean bad
debt. The launch policy defines when those controls are sufficient for
mainnet users and how exposure grows after launch.

The policy covers four operational questions:

1. What evidence gates the launch candidate before unpause?
2. Who controls owner and role-gated actions on mainnet?
3. How much TVL and borrow exposure is allowed at launch?
4. What sustained-operation window completes the mainnet launch milestone?

## Decision

Launch mainnet through a hardening gate and capped rollout. The protocol is
not unpaused for public access until audit closure, verification evidence,
testnet soak, role-holder setup, monitoring, and pause-drill checks have all
passed for the target deployment commit and deployed contract addresses.

### Launch Gates

The launch candidate must satisfy all gates before mainnet unpause:

- External audit findings for the target branch are closed, accepted with
  documented rationale, or explicitly deferred from launch scope.
- The verification acceptance matrix in `SCF_BUILD_ARCHITECTURE.md` is run
  against the target commit, and results are recorded in launch evidence.
- The configured testnet deployment runs for 14 consecutive days with no
  unresolved P0/P1 incidents, no unexplained accounting drift, no stale TTL
  windows, and no oracle configuration drift.
- Owner, `KEEPER`, `ORACLE`, and `REVENUE` authorities are assigned according
  to the role policy below before public unpause.
- Monitoring and alerting are live for market caps, pool reserves, oracle
  freshness/deviation, health-factor distribution, liquidatable accounts,
  bad-debt events, index freshness, TTL windows, revenue claims, and privileged
  calls.
- A pause drill is completed on testnet: pause, verify user mutations reject,
  keep required operator views/checks reachable, apply a benign config or
  runbook step, and unpause.

### Initial Mainnet Caps

Initial exposure is intentionally small:

- Total protocol TVL cap: USD 250,000.
- Total protocol borrow cap: USD 100,000.
- Per-market supply cap: USD 100,000.
- Per-market borrow cap: USD 50,000.
- Flash-loan exposure is bounded by each pool's available liquidity and the
  per-market launch caps.

Caps may be raised only after each stage runs for at least 7 consecutive days
without unresolved P0/P1 incidents, unexplained accounting drift, oracle
misconfiguration, or missed keeper/TTL maintenance. Each increase requires an
operator review of liquidity, utilization, liquidatable accounts, oracle
quality, and incident history. A later governance or launch-control decision
can replace these default caps when production data justifies a different
policy.

### Role and Authority Policy

Mainnet authority is separated by responsibility:

- The controller owner must be a multisig or equivalent multi-party custody
  setup. The deployer key must not retain launch authority after ownership and
  roles are assigned.
- `KEEPER`, `ORACLE`, and `REVENUE` roles must be held by separate operational
  addresses or automation identities. A single hot key must not hold owner and
  all operational roles.
- Non-emergency owner actions that change code, templates, or material risk
  configuration receive 48 hours of off-chain notice before execution.
- Emergency pause remains immediate. The owner may pause without notice for
  oracle incidents, accounting anomalies, suspected exploit activity,
  privileged-key compromise, or severe market stress.
- No on-chain timelock is required at launch. Adding one is a future governance
  decision because it trades user-warning time against emergency response time.

### Mainnet Launch Completion

Mainnet launch completion is not defined by smoke tests alone. It is complete
when:

- the target mainnet deployment passed all launch gates,
- the capped mainnet deployment is unpaused,
- monitoring and operational runbooks are live,
- initial caps are enforced on all listed markets, and
- the protocol completes 7 consecutive days of capped mainnet operation with
  no unresolved P0/P1 incident, no unexplained accounting drift, and no missed
  keeper or TTL maintenance window.

## Alternatives Considered

- **Unpause after deployment smoke tests only.** Rejected: smoke tests prove
  basic wiring, not operational readiness under real liquidity, oracle
  variance, liquidations, keeper work, or privileged-key procedures.
- **Launch uncapped after audit.** Rejected: even a clean audit does not remove
  configuration, oracle, integration, and operational risks. Capped exposure
  creates an observation window before allowing TVL growth.
- **Require on-chain timelock at launch.** Rejected for launch: a timelock
  improves predictability for non-emergency changes but slows response during
  early incidents. The launch policy uses multisig control and off-chain notice
  for non-emergency changes, with immediate pause retained for incidents.
- **Single operator key for all roles.** Rejected: it simplifies launch
  execution but concentrates upgrade, oracle, keeper, and revenue authority in
  one compromise domain.

## Consequences

Positive:

- Mainnet launch uses measurable criteria instead of relying on deployment
  smoke tests alone.
- Initial user exposure is bounded while mainnet network, oracle, keeper,
  liquidation, and monitoring behavior are observed.
- Role separation reduces the blast radius of a single operational-key
  compromise.
- The launch-completion condition becomes observable over a sustained capped
  operation window.

Negative / accepted costs:

- Launch takes at least the 14-day testnet soak plus the 7-day capped mainnet
  observation window before exposure can grow materially.
- Low initial caps may limit early user demand and protocol revenue.
- Off-chain notice for non-emergency changes is weaker than an on-chain
  timelock and relies on operational discipline.
- More operational identities must be maintained, monitored, and rotated.

## References

- `SCF_BUILD_ARCHITECTURE.md` §13 (Access Control and Operations), §16
  (Verification Surface), §17 (Deployment and Operations), §19 (Status).
- `controller/src/access.rs`
- `controller/src/router.rs`
- `controller/src/storage/ttl.rs`
- `architecture/INVARIANTS.md`
- `verification/certora/README.md`
