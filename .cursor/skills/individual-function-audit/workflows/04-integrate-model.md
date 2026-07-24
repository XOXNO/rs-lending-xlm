# Phase 4 — Integrate global mental model

## Entry criteria

- Wave-1 (and any required callees) have passing function artifacts

## Actions

1. Create/update `audit/function-context/GLOBAL_MODEL.md` with sections:

   ### Storage map
   For each key/struct: who reads, who writes, TTL/renewal notes, coupling.

   ### Invariants
   Cross-function only. Cite the function artifacts that established each.

   ### Workflows
   Reconstruct end-to-end:
   - supply → borrow → repay / withdraw
   - liquidate → seize → bad-debt cleanup
   - flash loan / strategy finalize
   - governance propose → cancel/execute → controller admin sink
   - oracle config → hard price read on money paths

   ### Trust boundaries
   Actor → entrypoint → auth → what they can mutate.

   ### Fragility clusters
   Functions with many assumptions, multi-module state coupling, or
   ordering sensitivity. These feed Phase 5 prioritization.

2. Resolve `OPEN_QUESTIONS.md` items that artifacts now answer; leave the
   rest explicitly open.
3. If two artifacts disagree, re-read the code (orchestrator) and correct
   the wrong artifact in place. Record the correction.

## Exit criteria

- `GLOBAL_MODEL.md` has all five sections populated with file:line citations
- No section is a restatement of README marketing — only evidence from
  function artifacts + code
- Still **no** severity-rated findings in this file
