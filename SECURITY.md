# Security Policy

Private vulnerability reporting for `rs-lending-xlm`.

## Reporting

Do **not** open a public issue, pull request, or discussion for security
problems.

Email **security@xoxno.com**.

Encrypt sensitive details when possible. A PGP key is available on request at
the same address (within one business day).

### Include

- Description and impact  
- Repro steps or PoC (prefer commit SHA + file:line)  
- Observed vs expected behavior  
- Environment (network, contract addresses if relevant, toolchain)  
- Whether the issue is already public  

Report against a **specific commit SHA** when you can.

## Response targets

| Stage | Target |
|-------|--------|
| Ack | 2 business days |
| Initial triage | 5 business days |
| Updates while open | every 7 days |
| Coordinated disclosure | 90 days from report (negotiable) |

Reporters who follow this policy may be credited in release notes (with consent).

## Scope

**In scope**

- On-chain crates: `contracts/controller`, `pool`, `governance`, `aggregator`,
  `xoxno-oracle-adapter`, `defindex-strategy`, plus `common/` and `interfaces/`
- `services/keeper` (TTL / restore)
- `services/lending-exporter` (ops metrics service)
- Makefile / `configs/` operator tooling that deploys or configures the protocol

**Out of scope**

- Upstream dependencies (Soroban SDK, OZ Stellar contracts, third-party oracles) —
  report upstream
- Issues that require already-compromised operator keys (governance owner, role
  holders including GUARDIAN, keeper keys)
- Theoretical issues without a reproducible PoC
- `contracts/flash-loan-receiver` as a production surface — it is **test-only**
  unless you are attacking the test harness itself

Technical properties: [architecture/INVARIANTS.md](./architecture/INVARIANTS.md),
[STRIDE.md](./STRIDE.md), [SCF_BUILD_ARCHITECTURE.md](./SCF_BUILD_ARCHITECTURE.md).
Report concrete deviations from those.

## Supported versions

Security patches target the **latest tag on `main`**. Mainnet tracks that
release; testnet may run release candidates.

## Safe harbor

Good-faith research under this policy is welcome. XOXNO will not pursue legal
action against researchers who:

- Report via **security@xoxno.com** (not public channels)
- Avoid privacy violations, disruption, and data destruction
- Prefer testnet / local environments for active testing
- Do not exploit beyond what is needed to demonstrate the issue

## Audit status

The design includes hardening ADRs (including 0009–0012: launch controls,
timelock, pause/freeze matrix, per-spoke liquidation curve). External audit
artifacts, when published for a release, will ship with that release or be
linked from the repo. Current design and threat posture: STRIDE, INVARIANTS,
and the contracts.
