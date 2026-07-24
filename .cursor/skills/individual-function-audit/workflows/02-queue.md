# Phase 2 — Queue

## Entry criteria

- `audit/function-context/INVENTORY.md` complete

## Actions

1. Copy [queues/priority-seed.yaml](../queues/priority-seed.yaml) into the
   working queue as the **mandatory baseline**.
2. Merge inventory rows that are missing from the seed:
   - Promote any permissionless value-moving entrypoint not already seeded
   - Promote any function touched by open Certora/fuzz/PoC suites
3. Order by priority score (higher first):

   | Signal | Weight |
   |--------|--------|
   | Permissionless + moves value | +5 |
   | Writes shared indexes / cash / positions | +4 |
   | Cross-contract call or callback | +3 |
   | Prior audit / PoC / held finding nearby | +3 |
   | Complex branching / math | +2 |
   | Owner/timelock only, no value move | +1 |

4. Write `audit/function-context/QUEUE.md` as a checklist:

   ```markdown
   # Function audit queue
   Commit: <sha>

   - [ ] P0 controller::liquidate → functions/controller__liquidation__liquidate.md
   - [ ] P0 ...
   ```

5. Cap the first dispatch wave at **8** P0/P1 items. Remaining stay queued
   for later waves — do not expand scope mid-wave.

## Exit criteria

- Every seed `id` appears in `QUEUE.md`
- Wave-1 ≤ 8 unchecked items marked for immediate dispatch
- Helpers that only exist to serve one entrypoint are noted as
  `fold-into: <parent>` instead of separate agents
