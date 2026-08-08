# 0017. Test-only powers must not reach release artifacts

**Status:** Accepted

## Decision

Testing and verification helpers are feature-gated. Release builds include an
artifact-level check for forbidden exported testing entrypoints, rather than
trusting feature selection alone.

Formal harnesses compile against production-shaped interfaces while declaring
their assumptions about external calls and prices.

## Guarantees

- Convenience test controls cannot silently become deployable public ABI.
- The release process checks the built artifact, not only source intent.
- Proof claims remain scoped to their harness assumptions.

## Auditor focus

Inspect feature combinations, release build inputs, exported ABI checks, and
the gap between formal models and live external integrations.
