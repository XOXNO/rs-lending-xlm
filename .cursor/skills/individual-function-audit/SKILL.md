---
name: individual-function-audit
description: >-
  Orchestrates auditor-depth, one-function-at-a-time security context building
  for the Soroban lending protocol using dedicated function-analyzer subagents.
  Use when the user asks for deep audit, individual agents, per-function mental
  models, call-chain tracing, storage-slot mapping, execution-order analysis,
  or rejects broad codebase sweeps. NOT for drive-by file skims, style review,
  or shipping vulnerability reports before context artifacts exist.
---

# Individual Function Audit

Broad sweeps read functions and move on. Auditors spend sessions on a single
function: every caller, every storage slot, every reachable ordering. This
skill forces that depth via **one dedicated subagent per queued function**,
then integrates the results into a durable mental model before any bug hunt.

## When to Use

- Security review of `contracts/*`, `common/`, or `interfaces/`
- User asks for individual agents / per-function depth / call-chain tracing
- Prior review admitted it "skimmed" or "swept" the codebase
- Preparing context before a vulnerability-hunting phase

## When NOT to Use

- PR style / code-quality checks → `/code-quality`
- Static-pattern scans only → Semgrep / Scout workflows
- Writing PoCs or patches before Phase 4 context exists
- Analyzing a single trivial getter in isolation (fold into its caller's agent)

## Hard Rules (never rationalize away)

| Rationalization | Required action |
|-----------------|-----------------|
| "I'll skim the module first" | Forbidden. Inventory → queue → dispatch. No free-form reading tour. |
| "This function is simple" | Still dispatch if it writes storage, moves value, or gates risk. |
| "I can cover 20 functions myself" | Cap: orchestrator owns ≤2 functions; everything else is a subagent. |
| "Parallelize everything" | Max **4** concurrent `function-analyzer` agents. Batch the rest. |
| "Context is in my head" | Write artifacts under `audit/function-context/`. Memory decays. |
| "Found a bug mid-context" | Park it in `OPEN_QUESTIONS.md`. No severity until Phase 5. |

## Pipeline

Execute in order. Read each workflow file and follow it exactly.

| Phase | Workflow | Exit criteria |
|-------|----------|---------------|
| 1 | [workflows/01-inventory.md](workflows/01-inventory.md) | `audit/function-context/INVENTORY.md` lists state-changing entrypoints + dense internals |
| 2 | [workflows/02-queue.md](workflows/02-queue.md) | `audit/function-context/QUEUE.md` prioritized; seed merged |
| 3 | [workflows/03-dispatch-analyzers.md](workflows/03-dispatch-analyzers.md) | One markdown artifact per queued function; completeness checklist passed |
| 4 | [workflows/04-integrate-model.md](workflows/04-integrate-model.md) | `GLOBAL_MODEL.md` with invariants, storage map, workflows, trust boundaries |
| 5 | [workflows/05-vulnerability-hunt.md](workflows/05-vulnerability-hunt.md) | Findings only after Phase 4; each finding cites context artifacts |

Seed queue (always merge in Phase 2): [queues/priority-seed.yaml](queues/priority-seed.yaml)

Agent prompt template: [references/analyzer-prompt.md](references/analyzer-prompt.md)

Output schema: [references/output-schema.md](references/output-schema.md)

## Subagent Contract

Spawn via Task tool with `subagent_type: "function-analyzer"` (preferred) or
`generalPurpose` only if `function-analyzer` is unavailable.

Each agent gets:

1. Exact `path` + `function` + `line` range when known
2. The full prompt from `references/analyzer-prompt.md` with placeholders filled
3. Instruction to write **pure context** (no vulns / fixes / severity)
4. Instruction to jump into callees and list callers (do not stop at the boundary)

Orchestrator must integrate returned summaries into `GLOBAL_MODEL.md` — never
leave agent output only in chat.

## Artifact Layout

```
audit/function-context/
  INVENTORY.md
  QUEUE.md
  OPEN_QUESTIONS.md
  GLOBAL_MODEL.md
  functions/
    <crate>__<module>__<function>.md
```

Generated bodies under `audit/function-context/` are gitignored except
`.gitkeep`. The skill, seed queue, and rules are committed.

## Success Definition

You succeeded only if a later agent (or human) can reconstruct the mental
model of a queued function **from disk artifacts alone**, including callers,
storage R/W, and ordering constraints — without re-reading the whole crate.
