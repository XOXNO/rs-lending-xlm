# Controller defensive-protections audit

Coordinated multi-agent review of `contracts/controller` defenses.
Agents share findings under `findings/`, wave notes under `waves/`, and
synthesis under `synthesis/`.

**Shared report:** [synthesis/FINAL.md](synthesis/FINAL.md) (A001–A110 complete).
Mid-wave [PRELIMINARY.md](synthesis/PRELIMINARY.md) is superseded.

## Themes

| ID | Theme |
|---|---|
| T1 | Defensive protections inventory (auth, pause, flags, flash guard, ratchets) |
| T2 | Low-level storage mutations on user paths via allowed tokens |
| T3 | Money movement (measured receipts, pool legs, refunds, seize modes) |
| T4 | Untrustworthy input validation |
| T5 | Spoke usage tracking after pool cross-contract calls (indexes/amounts) |
| T6 | Storage read/write simplifications and cross-contract read savings |
| T7 | In-memory `Cache` optimizations |
| T8 | Undefended gaps + impact quantification |

## Finding format

Each agent writes `findings/AXXX-<slug>.md`:

```md
# AXXX — title
- Agent: AXXX
- Theme: T*
- Severity: critical|high|medium|low|info|gap
- Status: defended|partial|undefended|optimization-note
- Paths: file:line ...
- Defense: what exists
- Gap: what is missing (or none)
- Impact: quantified blast radius (funds, accounts, markets, governance)
- Evidence: symbols, INV-* ids, tests/rules if any
- Opinion: short reasoned judgment
```

## Coordination rules

- Read-only audit: do not modify production Rust unless a fix is later scoped.
- Prefer primary sources under `contracts/controller/`, `common/`, `docs/reference/`, `docs/explanation/threat-model.md`, `STRIDE.md`.
- Cross-link peer findings by agent id when you agree/disagree.
- Never weaken gates to make a claim pass.
