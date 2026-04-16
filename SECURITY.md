# Security Policy

The XOXNO team takes the security of `rs-lending-xlm` seriously. This document
explains how to report a vulnerability and what you can expect from us.

## Reporting a Vulnerability

**Please do NOT open a public issue, pull request, or discussion for security
problems.** Disclosure through public channels gives potential attackers a
window to exploit the issue before users can update.

Send a private report to **security@xoxno.com**.

Encrypt sensitive details if possible — request our PGP key at the same
address. We will reply with the key within one business day.

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

We follow a coordinated-disclosure model. Reporters who follow this policy will
be credited (with consent) in the release notes that ship the fix.

## Scope

In scope:

- `controller/`, `pool/`, `pool-interface/`, `common/` (the deployed
  on-chain crates).
- The Makefile + `configs/script.sh` operator path that deploys and
  configures the protocol.

Out of scope (please do NOT report):

- Vulnerabilities in third-party dependencies (Soroban SDK,
  OpenZeppelin Stellar contracts, Reflector oracle) — report those upstream.
- Issues that require already-compromised operator (Owner / KEEPER / REVENUE /
  ORACLE) keys.
- Theoretical attacks without a reproducible proof of concept.
- Findings already documented in `audit/FINDINGS.md` or `MATH_REVIEW.md`.

## Supported Versions

Only the latest tagged release on the `main` branch receives security patches.
Mainnet deployments should track the latest release; testnet may be on a
release candidate.

## Safe Harbor

Good-faith security research that follows this policy is welcome. We will not
pursue legal action against researchers who:

- Report vulnerabilities through `security@xoxno.com` rather than public
  channels.
- Avoid privacy violations, service disruption, and data destruction during
  testing.
- Use testnet or local environments for active exploitation testing.
- Do not exploit a vulnerability beyond what is necessary to demonstrate it.

## Audit Status

`audit/AUDIT_PREP.md` and `audit/FINDINGS.md` document the known findings the
internal team has identified ahead of formal review. External audits in
progress are tracked in the same directory.
