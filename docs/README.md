# Documentation

Documentation map for XOXNO Lending. Code is the source of truth; these pages
orient auditors, integrators, and contributors. Public ABI semantics live in
rustdoc on the interface crates.

| Need | Start here |
|------|------------|
| Topology, storage, money flows, upgrade surface | [Reference: architecture](./reference/architecture.md) |
| Rules that must not break | [Reference: invariants](./reference/invariants.md) |
| Risk, HF, liquidation math (code-matched) | [Reference: formulas](./reference/formulas.md) |
| Actors, trust boundaries, attack surfaces | [Explanation: threat model](./explanation/threat-model.md) |
| Why a decision was made | [Explanation: ADRs](./explanation/decisions/README.md) |
| Shared protocol model for any integration | [skills/lending-protocol-fundamentals](../skills/lending-protocol-fundamentals/SKILL.md) |
| Agent integration recipes | [skills/](../skills/README.md) |
| Formal verification | [certora/](../certora/README.md) |
| Contribute a change | [CONTRIBUTING.md](../CONTRIBUTING.md) |
| Report a vulnerability | [SECURITY.md](../SECURITY.md) |

## Layout

- **reference/** — accurate description of the system as built: architecture,
  invariants, formulas.
- **explanation/** — understanding: the threat model and architecture decision
  records (ADRs).
- **research/** — point-in-time research notes; not normative.

Contract package READMEs under `contracts/*/` are indexes (entrypoint name,
role, links). Where a README and source disagree, the source wins.
