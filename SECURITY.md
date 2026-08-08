# Security policy

XOXNO Lending welcomes good-faith security research. Report vulnerabilities
privately so maintainers can validate, fix, and coordinate disclosure without
putting users at risk.

## Report privately

Do not open a public issue, pull request, discussion, or social-media post for
a suspected vulnerability.

Email [security@xoxno.com](mailto:security@xoxno.com). Encrypt sensitive
details when possible; a PGP key is available on request.

A useful report includes:

- a clear description of the issue and its practical impact;
- a minimal reproduction or proof of concept;
- the affected revision or release, ideally a commit SHA;
- the relevant environment, network, and contract addresses where applicable;
- observed behavior, expected behavior, and any assumptions needed to trigger
  the issue; and
- whether any part of the issue is already public.

Do not access, modify, or disclose another person's data. Stop once you have
enough evidence to demonstrate impact.

## What to expect

| Stage | Target |
|---|---|
| Acknowledgement | Within 2 business days |
| Initial triage | Within 5 business days |
| Updates while open | At least every 7 days |
| Coordinated disclosure | 90 days from report, unless we agree otherwise |

We may credit reporters in release notes with their consent.

## Scope

### In scope

- The deployed lending protocol and its supporting on-chain components.
- Shared protocol logic and public interfaces.
- Keeper and operational exporter services.
- Deployment and configuration tooling that can affect protocol behavior.

### Out of scope

- Vulnerabilities in upstream dependencies or third-party oracle providers;
  report these to their maintainers.
- Scenarios that require a compromised governance, operational-role, or keeper
  key, unless the report demonstrates that the protocol makes that compromise
  materially worse.
- Purely theoretical claims without a reproducible security impact.
- Test-only contracts, unless the issue affects a production build or the test
  harness is itself in scope.

A strong report identifies a concrete deviation from a protocol invariant,
authorization boundary, accounting rule, price guarantee, or liveness property.

## Supported versions

Security fixes target the latest tag on main. Mainnet follows that release;
testnet may run release candidates. If you are unsure whether a deployment is
affected, include its network and contract addresses in the private report.

## Safe harbor

XOXNO will not pursue legal action against good-faith researchers who follow
this policy and:

- report privately;
- avoid disruption, privacy violations, and data destruction;
- prefer local or testnet environments for active testing; and
- do not exploit an issue beyond what is necessary to demonstrate it.

This policy does not authorize activity that violates law, targets third-party
systems, or places user assets or data at risk.

## Audit status

Published audit material, when available, is linked with the relevant release.
Repository documentation describes the current design and threat model; it is
not a substitute for independent review of deployed contracts and active
configuration.
