# XOXNO Lending documentation

This documentation is written for reviewers first. It explains what the
protocol is meant to protect, why its design choices exist, and what should be
true across every execution path. Implementation navigation belongs in the
codebase; these documents are the map for understanding it.

## Start here

| Question | Read |
|---|---|
| What system is this? | [Architecture](reference/architecture.md) |
| What must never break? | [Runtime invariants](reference/invariants.md) |
| Who can attack or operate it? | [Threat model](explanation/threat-model.md) |
| How do values and risk calculations work? | [Formulas](reference/formulas.md) |
| Why was a design chosen? | [Decision records](explanation/decisions/README.md) |

## Reading order for an audit

1. Architecture: components, custody, authority, and money flow.
2. Threat model: actors, trust boundaries, attack surfaces, and residual risk.
3. Invariants: the properties to test or prove.
4. Formulas: unit conventions, rounding, interest, health, and liquidation.
5. Decision records: the rationale behind consequential choices.

## Documentation conventions

- “Must” describes a safety property, not an aspiration.
- “Fails closed” means the transaction reverts rather than guessing.
- “Pool” is one custody system with isolated per-market accounting.
- “Hub asset” identifies a market; “spoke” identifies an account’s risk regime.
- RAY is 10^27, WAD is 10^18, and BPS is 10,000.

The documents describe the intended current protocol behavior. Configuration,
deployment state, and external dependencies still require independent audit.
