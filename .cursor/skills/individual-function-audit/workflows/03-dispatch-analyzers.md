# Phase 3 — Dispatch function-analyzer agents

## Entry criteria

- `QUEUE.md` wave-1 defined
- Analyzer prompt template read:
  [references/analyzer-prompt.md](../references/analyzer-prompt.md)

## Actions

1. For each wave-1 item (max **4 concurrent** Task agents):
   - `subagent_type`: `function-analyzer` (fallback: `generalPurpose`)
   - Fill every placeholder in `analyzer-prompt.md`
   - Require the agent to return the full microstructure document
   - Require caller enumeration (Grep) and storage key enumeration
2. On agent return:
   - Write `audit/function-context/functions/<crate>__<module>__<fn>.md`
   - Verify against [references/output-schema.md](../references/output-schema.md)
   - If incomplete: **re-dispatch the same function** with the missing
     sections listed — do not mark the queue item done
3. Mark `QUEUE.md` checkbox only after the on-disk artifact passes the
   schema minimums (invariants ≥3, assumptions ≥5, line citations present).
4. Park contradictions and unknowns in `OPEN_QUESTIONS.md` with file:line.
5. After wave-1 completes, start wave-2 from remaining queue items.
   Prefer callees discovered in wave-1 that lack their own artifact.

## Parallelism bounds

- ≤4 live analyzer agents
- Never one agent assigned ≥2 unrelated entrypoints
- Sibling helpers of one entrypoint may share one agent **only if** the
  prompt names them and requires the continuous call-chain rule

## Exit criteria

- Every wave-1 queue item has a passing artifact on disk
- `OPEN_QUESTIONS.md` lists unresolved items (may be non-empty)
- Orchestrator did not skip to vulnerability claims
