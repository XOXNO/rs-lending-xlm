# Phase 1 — Inventory

## Entry criteria

- Working tree is the audit target commit (record SHA in `INVENTORY.md`)
- Skill activated; no vulnerability hunting yet

## Actions

1. Record `git rev-parse HEAD` and date at the top of
   `audit/function-context/INVENTORY.md`.
2. **Build inventory from source + graph** (no machine index scripts):
   - codebase-memory (`make cbm-index` if stale)
   - `interfaces/*` trait methods (ABI names)
   - thin wrappers in `contracts/*/src/lib.rs` → dense `process_*` / `apply_*`
   - priority seed: [queues/priority-seed.yaml](../queues/priority-seed.yaml)
3. Prefer graph + interfaces over free-form walking of every file.
4. Seed inventory rows from interface money-path ABIs and their dense
   callees (resolve aliases in `lib.rs` / process modules).
5. Classify **state-changing** surfaces only (skip pure views unless they
   feed a money-path gate):
   - Permissionless user/keeper money paths
   - Callback / flash-loan / strategy surfaces
   - Owner / role / timelock admin paths
   - Dense internals those entrypoints call (math, sync, seize, index, cash)
6. For each candidate row, capture:
   - `crate`, `function` (**dense** name for analysis), `abi` if different
   - `path` to dense body when known
   - access: `permissionless` | `auth_user` | `owner` | `role` | `pool_only`
   - value_move: yes/no
   - storage_touch: unknown | yes (fill in Phase 3)
   - density: `entrypoint` | `internal-dense` | `helper`
7. Do **not** deep-read implementations here. Inventory is names + roles.
8. Do **not** list only ABI Method names without dense aliases — agents
   cannot usefully trace empty lib.rs stubs.

## Exit criteria

- `INVENTORY.md` exists with ≥ the seed queue's functions present
- Every money-path ABI row has a **dense** `process_*` / `apply_*` target
  when one exists in the contract source
- Dense internals for liquidation, indexes, cash/transfers, oracle compose,
  and timelock execute are listed even if not public ABI
- No findings section anywhere in this file
