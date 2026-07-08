# Security Policy

This policy defines the private vulnerability reporting process for
`rs-lending-xlm`.

## Reporting a Vulnerability

Do not open a public issue, pull request, or discussion for security problems.
Public disclosure can give attackers a window to exploit an issue before users
can update.

Send a private report to **security@xoxno.com**.

Encrypt sensitive details when possible. A PGP key is available by request at
the same address within one business day.

### What to include

- A clear description of the vulnerability and its impact.
- Step-by-step instructions to reproduce, or a proof-of-concept that
  demonstrates the issue. Cite specific commit / file / line numbers.
- The protocol behavior you observed and what you expected.
- Your environment (Stellar network, controller / pool addresses if relevant,
  toolchain versions).
- Whether the issue is already public knowledge.

## Response Timeline

| Stage | Target |
|---|---|
| Acknowledgement of receipt | within **2 business days** |
| Initial triage (severity, scope) | within **5 business days** |
| Status update cadence during fix | every **7 days** until resolved |
| Coordinated disclosure window | **90 days** from report, negotiable |

The protocol follows a coordinated-disclosure model. Reporters who follow this
policy are credited, with consent, in the release notes that ship the fix.

## Scope

In scope:

- The on-chain crates: `contracts/controller`, `contracts/pool`,
  `contracts/governance`, `contracts/aggregator`,
  `contracts/xoxno-oracle-adapter`, `contracts/defindex-strategy`,
  `contracts/flash-loan-receiver`, plus shared `common/` and `interfaces/`.
- The Makefile + `configs/script.sh` operator path that deploys and
  configures the protocol.

Not in scope:

- Vulnerabilities in third-party dependencies (Soroban SDK,
  OpenZeppelin Stellar contracts, Reflector oracle) — report those upstream.
- Issues that require already-compromised operator keys (the governance owner
  or governance role holders).
- Theoretical attacks without a reproducible proof of concept.

## Supported Versions

Only the latest tagged release on the `main` branch receives security patches.
Mainnet deployments track the latest release; testnet deployments can run a
release candidate.

## Safe Harbor

Good-faith security research that follows this policy is welcome. XOXNO will
not pursue legal action against researchers who:

- Report vulnerabilities through `security@xoxno.com` rather than public
  channels.
- Avoid privacy violations, service disruption, and data destruction during
  testing.
- Use testnet or local environments for active exploitation testing.
- Do not exploit a vulnerability beyond what is necessary to demonstrate it.

## Audit Status

External audit artifacts will be published with the corresponding release once
available.
