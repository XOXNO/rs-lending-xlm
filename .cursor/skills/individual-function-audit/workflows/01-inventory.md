# Phase 1 — Inventory

## Entry criteria

- Working tree is the audit target commit (record SHA in `INVENTORY.md`)
- Skill activated; no vulnerability hunting yet

## Actions

1. Record `git rev-parse HEAD` and date at the top of
   `audit/function-context/INVENTORY.md`.
2. Prefer existing maps over rediscovery:
   - `docs/reference/endpoint-inventory.md`
   - `docs/reference/invariants.md`
   - `docs/explanation/threat-model.md`
   - `docs/reference/architecture.md`
3. Classify **state-changing** surfaces only (skip pure views unless they
   feed a money-path gate):
   - Permissionless user/keeper money paths
   - Callback / flash-loan / strategy surfaces
   - Owner / role / timelock admin paths
   - Dense internals those entrypoints call (math, sync, seize, index, cash)
4. For each candidate row, capture:
   - `crate`, `function`, `path`, approximate lines
   - access: `permissionless` | `auth_user` | `owner` | `role` | `pool_only`
   - value_move: yes/no
   - storage_touch: unknown | yes (fill in Phase 3)
   - density: `entrypoint` | `internal-dense` | `helper`
5. Do **not** deep-read implementations here. Inventory is names + roles.

## Exit criteria

- `INVENTORY.md` exists with ≥ the seed queue's functions present
- Dense internals for liquidation, indexes, cash/transfers, oracle compose,
  and timelock execute are listed even if not public ABI
- No findings section anywhere in this file
