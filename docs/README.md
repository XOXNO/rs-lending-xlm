# Documentation

Diátaxis layout for XOXNO Lending. Code is the source of truth; these pages
orient humans and auditors. Public ABI semantics live in rustdoc — see
[doc-style.md](./reference/doc-style.md).

| Need | Start here |
|------|------------|
| First successful build/test | [Tutorial: build and test](./tutorials/01-build-and-test.md) |
| Deploy / day-2 ops | [How-to: deploy and operate](./how-to/deploy-and-operate.md) |
| Contribute a change | [CONTRIBUTING.md](../CONTRIBUTING.md) |
| Topology / storage / verification surface | [Reference: architecture](./reference/architecture.md) |
| Rules that must not break | [Reference: invariants](./reference/invariants.md) |
| Why a decision was made | [Explanation: ADRs](./explanation/decisions/README.md) |
| Threat model | [Explanation: threat model](./explanation/threat-model.md) |
| Agent integration recipes | [skills/](../skills/README.md) |
| Report a vulnerability | [SECURITY.md](../SECURITY.md) |

## Quadrants

- **Tutorials** — learning-oriented lessons to a working outcome.
- **How-to** — recipes for a specific job (deploy, operate).
- **Reference** — accurate description of the system (architecture, invariants,
  inventories, rustdoc style).
- **Explanation** — understanding (ADRs, threat model).

Contract package READMEs under `contracts/*/` are **indexes** only (entrypoint
name + role + link here / rustdoc). Do not duplicate full semantics there.
