# Certora Verification Repair — Phase 1–3 (get the build green)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to execute. Steps use checkbox (`- [ ]`).

**Goal:** Restore the existing Certora/CVLR spec + harness so the certora-feature build compiles and the certora WASM builds clean — i.e. green `verification/certora/compile_all.sh --wasm`. This re-establishes the "specs can't silently drift" gate. **Out of scope:** running the hosted prover (needs CERTORAKEY/funded Certora — Phase 4) and adding rules for governance/timelock (Phase 5).

**Why it's broken:** the specs were last green ~2026-05-05 (the `.certora_internal` snapshot dates). Since then three waves of refactor landed without spec updates: the **single-pool/asset-keyed migration** (pool methods take `asset`, `MarketConfig.pool_address` removed), the **type-crate move** (`controller` types → `controller-interface`, `external` module rename), and the **governance split** (admin moved out of the controller). The certora-WASM build surfaces ~87 controller errors; common/pool are lighter.

**Branch:** do this on `feat/governance-split` (or after it merges) so specs target the *final* architecture (single pool + gov split + timelock), not an intermediate state.

**The gate (`verification/certora/compile_all.sh`):**
```
cargo check -p common     --features certora
cargo check -p pool       --features certora --no-default-features
cargo check -p controller --features certora --no-default-features
python3 check_orphans.py            # every conf-referenced rule has a #[rule] fn
python3 check_invariant_coverage.py
python3 scripts/sync_wasm_conf.py
# --wasm: make certora-wasm + check_wasm_artifacts.py
```

**Repair surface:**
- `common`: `spec/{harness,math_rules,rates_rules,mod}.rs` + 2 confs — small (common types stable).
- `pool`: `spec/{additivity_rules,integrity_rules,summary_contract_rules,mod}.rs` + 5 confs — moderate (asset-keyed + `pool_address`).
- `controller`: `harness/{external/pool,external/sac,oracle_price,oracle_tolerance,storage,summarized,views/aggregates}.rs` (7) + `spec/*.rs` (19 rule files + `compat.rs` + `mod.rs`) + 28 confs — the bulk.

**Dominant error classes (controller):** 51× `cannot find module/crate controller` (path/type-move drift, likely cascading from `spec/compat.rs` + harness), 13× asset-keyed arg-count, 4× `no field pool_address on MarketConfig`, plus renamed/moved APIs (`pool_create_market_call`, `common::types::PriceFeedRaw`, `PoolPositionMutation::get`).

---

### Task 1: Triage — authoritative error map + mechanical-vs-semantic split
**Files:** read-only; output a fix-map note under `verification/certora/REPAIR_NOTES.md`.
- [ ] Run the build path certora actually uses, per crate, and capture the FULL error set: `cargo check -p <crate> --features certora [--no-default-features]` AND `make certora-wasm` (the wasm32v1-none build surfaces target-only issues the host check misses — this is why earlier host-check counts disagreed with the ~87 wasm errors).
- [ ] Categorize every error → (file, class, fix-kind). Fix-kinds: **MECH** (repoint path / thread `&asset` / arg-count) vs **SEM** (needs a decision: e.g. rules over `MarketConfig.pool_address` must be rethought for the asset-keyed central pool; admin-config rules whose target moved to the governance contract must be **removed or repointed to the surviving thin setter**).
- [ ] Flag the SEM set explicitly — those need owner/CVLR-aware judgment, not blind edits. Commit the note.

### Task 2: Fix the shared layer (compat + harness) first
**Files:** `controller/spec/compat.rs`, `controller/harness/{storage.rs,summarized.rs,external/pool.rs,external/sac.rs,oracle_price.rs,oracle_tolerance.rs,views/aggregates.rs}`
- [ ] `compat.rs` is the type/alias shim the spec layer imports through; the 51× `cannot find module/crate controller` very likely cascade from here + the harness. Repoint to `controller_interface::types::*` (type-crate move) and the `external`-module rename; update the asset-keyed pool harness (`external/pool.rs`: methods now take `asset`; no `pool_address`).
- [ ] Re-run `cargo check -p controller --features certora --no-default-features`; confirm the error count collapses (most of the 51 should clear). Commit `fix(certora): repoint controller harness/compat to current types + asset-keyed pool`.

### Task 3: Fix `common` specs
**Files:** `common/spec/{harness,math_rules,rates_rules}.rs`
- [ ] Resolve the remaining errors (likely minor — common types are stable). `cargo check -p common --features certora` green. Commit.

### Task 4: Fix `pool` specs
**Files:** `pool/spec/{additivity_rules,integrity_rules,summary_contract_rules}.rs`
- [ ] Thread `&asset` through asset-keyed pool calls; rework any rule referencing the removed `pool_address` / per-market pool to the central asset-keyed model (SEM items from Task 1). `cargo check -p pool --features certora --no-default-features` green. Commit.

### Task 5: Fix `controller` rule files (19) by error class
**Files:** `controller/spec/{account_isolation,boundary,consistency,emode,flash_loan,health,index,interest,isolation,liquidation,market_guard,math,oracle_compose,oracle,position,solvency,strategy,tolerance_math}_rules.rs`
- [ ] Sweep by class (MECH first): module paths, asset-keyed sigs, arg-counts, renamed APIs. Then the SEM set: admin/config rules whose targets moved to governance — decide per Task 1 (repoint to surviving thin setter, or delete the rule and record the coverage gap in `REPAIR_NOTES.md`).
- [ ] `cargo check -p controller --features certora --no-default-features` fully green. Commit (may split MECH vs SEM into two commits).

### Task 6: Orphan + invariant-coverage green
- [ ] `python3 verification/certora/check_orphans.py` — every conf-referenced rule still has a `#[rule]` fn; reconcile any rule renamed/removed in Task 5 against its `.conf` (drop the conf entry or restore the rule). `python3 verification/certora/check_invariant_coverage.py` green. `python3 verification/certora/scripts/sync_wasm_conf.py`. Commit `chore(certora): reconcile confs ↔ rules after repair`.

### Task 7: Build the certora WASM (Phase 3) + full gate
- [ ] `make certora-wasm` (unoptimized per-crate; note the optimizer-crashes-GC-checker constraint) builds clean for common/pool/controller; `python3 verification/certora/scripts/check_wasm_artifacts.py` passes.
- [ ] `./verification/certora/compile_all.sh --wasm` green end-to-end (this is exactly the `certora-verification` CI compile job). Commit `chore(certora): certora WASM builds green`.

### Task 8 (handoff): confirm CI compile job + document the Phase-4 boundary
- [ ] Confirm the `certora-verification.yml` *compile* matrix would pass on this branch (the build half). The prover steps still need `CERTORAKEY` (Phase 4) — do NOT attempt here.
- [ ] In `REPAIR_NOTES.md`: list deleted/parked rules (coverage gaps), the SEM decisions made, and the explicit Phase-4/5 follow-ups (run prover; add governance/timelock + single-pool rules). Commit.

## Self-review / risks
- **Not purely mechanical:** the gov split means some controller admin rules verify code that no longer lives in the controller — those are delete-or-repoint *decisions*, not edits. Task 1 must surface them; don't let an implementer silently delete coverage.
- **Prereq met:** the certora feature pulls cvlr/cvlr-soroban, now on sdk-26 (this branch). If `make certora-wasm` fails on cvlr, that's the vendoring/sdk path, not the specs.
- **Green ≠ proven:** this restores *compilation* + the orphan/coverage gates. Whether the rules still *hold* is Phase 4 (hosted prover) — explicitly out of scope.
- **Make CI required** (the durability win) is a tiny follow-up once green: mark the `certora-verification` compile job a required check so specs can't drift again. Note it for the owner; not done here.
