# Security policy

XOXNO Lending welcomes good-faith security research. Report vulnerabilities
privately so maintainers can validate, fix, and coordinate disclosure without
putting users at risk.

## Report privately

Do not report a suspected vulnerability in a public issue, pull request,
discussion, or social-media post. Use one of these channels:

- To claim a bounty, submit the report through the
  [XOXNO bug bounty on Immunefi](https://immunefi.com/bug-bounty/xoxno/information/).
- For other reports, email [security@xoxno.com](mailto:security@xoxno.com).
  Encrypt sensitive details with our
  [PGP key](.github/security-pgp-key.asc), fingerprint
  `E6CD 3011 03A0 CF06 96C4 57F5 35F6 978D 1310 C877`.

Include these items in the report:

- The issue and its practical impact.
- A minimal reproduction or proof of concept.
- The affected revision or release, ideally a commit SHA.
- The environment, network, and contract addresses where applicable.
- Observed and expected behavior, and any assumptions needed to trigger it.
- Whether any part of the issue is already public.

Do not access, modify, or disclose another person's data. Stop once you have
enough evidence to show impact.

## What to expect

| Stage | Target |
|---|---|
| Acknowledgement | Within 2 business days |
| Initial triage | Within 5 business days |
| Updates while open | At least every 7 days |
| Coordinated disclosure | 90 days from report, unless we agree otherwise |

We may credit reporters in release notes with their consent.

## Bug bounty

The [Immunefi program page](https://immunefi.com/bug-bounty/xoxno/information/)
sets the assets in scope, the reward for each severity, and the submission
rules. Where this policy and the program page differ on a bounty question, the
program page applies.

## Scope

### In scope

- The deployed lending protocol and its supporting on-chain components.
- Shared protocol logic and public interfaces.
- Keeper and operational exporter services.
- Deployment and configuration tooling that can affect protocol behavior.

### Out of scope

- Vulnerabilities in upstream dependencies or third-party oracle providers.
  Report these to their maintainers.
- Scenarios that require a compromised governance, operational-role, or keeper
  key. These are in scope only when the report demonstrates that the protocol
  makes the compromise materially worse.
- Theoretical claims without a reproducible security impact.
- Test-only contracts, unless the issue affects a production build.

A strong report identifies a concrete deviation from a protocol invariant,
authorization boundary, accounting rule, price guarantee, or liveness property.

## Supported versions

Security fixes target the latest tag on main. Mainnet follows that release.
Testnet may run release candidates. If you are unsure whether a deployment is
affected, include its network and contract addresses in the report.

## Safe harbor

XOXNO will not pursue legal action against good-faith researchers who follow
this policy and who:

- Report privately.
- Avoid disruption, privacy violations, and data destruction.
- Prefer local or testnet environments for active testing.
- Do not exploit an issue beyond what is necessary to demonstrate it.

This policy does not authorize activity that violates law, targets third-party
systems, or places user assets or data at risk.

## Audits

| Auditor | Scope | Commit | Engagement | Report |
|---|---|---|---|---|
| Runtime Verification | Core contracts and `common/` | `a2486b25` (`v1.1.0`) | 2026-09-30 to 2026-11-13 | [PDF](audits/2026-11-runtime-verification.pdf) |

An audit covers only the commit in its scope. Compare it with the release you
use.
