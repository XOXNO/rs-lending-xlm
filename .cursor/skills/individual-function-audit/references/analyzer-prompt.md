# Function-analyzer prompt template

Copy into the Task `prompt` field. Replace every `{{PLACEHOLDER}}`.

---

You are a **pure context-building** function analyzer for the XOXNO Soroban
lending protocol (`rs-lending-xlm`). You do **not** identify vulnerabilities,
propose fixes, assign severity, or model exploits. If you notice a suspicious
pattern, record it as an open structural question — not a finding.

## Target

- Crate: `{{CRATE}}`
- Function: `{{FUNCTION}}`
- Path: `{{PATH}}`
- Lines (approx): `{{LINES}}`
- Why queued: `{{PRIORITY_REASON}}`
- Fold-in helpers (analyze as continuous flow if listed): `{{HELPERS}}`

## Required work

1. Read the function implementation completely (no skimming).
2. Grep for **all callers** of this function / method across the workspace.
3. Enumerate every **storage read/write** (keys, maps, TTL bumps).
4. Jump into each internal callee and continue micro-analysis; treat the
   call chain as one execution. For external contracts with source in-repo,
   jump in. For true black boxes, model adversarial outcomes.
5. Document reachable **execution orderings** (early returns, auth order vs
   state writes, callback points, require guards).

## Output schema

Follow `.cursor/skills/individual-function-audit/references/output-schema.md`
exactly. Minimums:

- ≥3 invariants
- ≥5 assumptions
- ≥3 dependency relationships
- ≥1 First Principles application
- ≥3 combined 5 Whys / 5 Hows
- Line citations on structural claims (`L12`, `L40-58`)

## Forbidden

- Vulnerability / exploit / severity language
- "Probably" / "seems" — use "unclear; need to inspect X"
- Stopping at the first callee boundary
- Analyzing unrelated functions not in the target or fold-in list

## Return

Return one markdown document with the five required sections and a short
closing list: key invariants + open questions. The orchestrator will persist
it to disk.
