# Output schema (per function)

Each `audit/function-context/functions/<crate>__<module>__<fn>.md` must contain:

```markdown
# {{crate}}::{{function}}

- Path: `...`
- Lines: Lx–Ly
- Commit: <sha>
- Access: ...
- Callers: (bulleted file:line)

## 1. Purpose
(2–3 sentences minimum)

## 2. Inputs and Assumptions
| Input | Type / source | Trust |
|-------|---------------|-------|
| ... | ... | untrusted / auth / owner / protocol |

Assumptions (minimum 5):
1. ...

## 3. Outputs and Effects
- Returns: ...
- Storage writes: ...
- Events: ...
- External calls: ...
- Postconditions: ...

## 4. Block-by-Block Analysis
### Block A — Lx–Ly — <title>
- What:
- Why here:
- Assumptions:
- Depends on:
- First Principles / 5 Whys / 5 Hows:

(repeat for every logical block: auth, load state, compute, external call,
 persist, event)

## 5. Cross-Function Dependencies
- Callees: ...
- Callers: ...
- Shared state couplings: ...
- Ordering constraints: ...

## Invariants (minimum 3)
1. ...

## Open questions
- ...
```

## Completeness gate (orchestrator)

Mark the queue item done only if all are true:

- [ ] Sections 1–5 present
- [ ] ≥5 assumptions
- [ ] ≥3 invariants
- [ ] ≥1 caller listed **or** explicit "no in-repo callers (entrypoint only)"
- [ ] Storage R/W enumerated (or explicit "none")
- [ ] Every external/cross-contract call has Case A (in-repo jump) or Case B
      (adversarial outcomes)
- [ ] No vulnerability/severity wording
