# Controller View ABI Alignment (get_/is_) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename 12 controller read entrypoints to a consistent `get_`/`is_` convention (matching the pool's 2026-06-26 view rename) without changing any behavior.

**Architecture:** The controller exposes thin `#[contractimpl]` wrapper methods that delegate to internal free functions. Only the wrapper (ABI) names and the `controller_interface` client-mirror trait change; internal free functions keep their names, so the internal blast radius is near-zero. The Rust compiler enumerates every stale call-site (tests use `controller::ControllerClient`, auto-generated from the wrappers); shell flows and the mirror trait are updated by hand.

**Tech Stack:** Rust / `soroban-sdk` 26.1, `stellar-cli` v27, Bash integration harness.

## Scope

**In scope (this plan): the `rs-lending-xlm` repo only.**
- 12 view-entrypoint renames (table below).
- Mirror trait, the one cross-contract consumer (`defindex-strategy`), all Rust test/fuzz call-sites, all integration shell flows, wasm rebuild.

**Explicitly OUT of scope:**
- `sdk-js` — **verified to contain zero references** to any renamed view (`lending.ts` only builds write txs: supply/borrow/withdraw/repay/liquidate/flash_loan/migrate_from_blend/multiply/swap_debt/swap_collateral/repay_debt_with_collateral). No SDK code change is required. Task 4 includes a regression grep to prove this stays true.
- `api-v2`, `az-functions`, `ui` — the real downstream view consumer is **api-v2** (3 files, listed in "Deferred — Phase 2"). Not touched here.
- `max_withdraw` / `max_supply` / `max_borrow` (left as the `max_*` idiom), admin-setter tidy-ups, and `account_exists` (does not map cleanly to `is_`). Not in scope.

## Rename Map (the only 12 entrypoints that change)

| # | Current entrypoint | New entrypoint | Kind |
|---|---|---|---|
| 1 | `health_factor` | `get_health_factor` | i128 getter |
| 2 | `can_be_liquidated` | `is_liquidatable` | bool |
| 3 | `total_collateral_in_usd` | `get_total_collateral_usd` | i128 getter |
| 4 | `total_borrow_in_usd` | `get_total_borrow_usd` | i128 getter |
| 5 | `ltv_collateral_in_usd` | `get_ltv_collateral_usd` | i128 getter |
| 6 | `liquidation_collateral_available` | `get_liquidation_collateral` | i128 getter |
| 7 | `collateral_amount_for_token` | `get_collateral_amount` | i128 getter |
| 8 | `borrow_amount_for_token` | `get_borrow_amount` | i128 getter |
| 9 | `app_version` | `get_app_version` | u32 getter |
| 10 | `get_all_markets_detailed` | `get_markets_detailed` | drops false "all_" |
| 11 | `get_all_market_indexes_detailed` | `get_market_indexes_detailed` | drops false "all_" |
| 12 | `liquidation_estimations_detailed` | `get_liquidation_estimate` | fixes pluralization |

**Internal free functions are NOT renamed** (e.g., the wrapper `get_health_factor` keeps delegating to the free fn `health_factor`). They are not ABI and renaming them would add a large, behavior-neutral blast radius for no benefit.

## Global Constraints

- Verification uses `--workspace`, **never `--all-features`** — `--all-features` enables `certora` and breaks linking (`_CVT_assert`).
- Rust bar (must pass): `cargo build --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- `stellar contract build` is **mandatory** after the rename — it regenerates the wasm contract spec so on-chain entrypoint names match; skipping it makes downstream/fixture consumers diverge.
- Format touched files with `rustfmt --edition 2021 <file>` individually. Do **not** run `cargo fmt -p controller` — it reformats `cfg`-gated `#[path]` certora/harness modules and creates unrelated churn.
- Never `cd` inside the Bash tool (the gvm hook exits 1 "GVM_ROOT not set"). Use absolute paths and `cargo --manifest-path`.
- `main` is PR-only (protected). Do all work on a feature branch and open a PR; do not push to `main`.
- This is a behavior-preserving rename: there is no new behavior to TDD. The regression gate is the existing test suite + the compiler + `stellar contract build`. Do not add new tests; do not change any assertion values.

## Surfaces (exact files)

- **ABI wrappers:** `contracts/controller/src/views/mod.rs` (entrypoints 1,3,4,7,8 at lines 40,44,48,52,56,60; entrypoints 10,11,12,6,5 at lines 96,103,107,115,119) and `contracts/controller/src/governance/access.rs:112` (`app_version`).
- **Client mirror trait:** `interfaces/controller/src/lib.rs` (entrypoints 1–8,10,11,12; `app_version` is intentionally absent from this trait).
- **Cross-contract consumer:** `contracts/defindex-strategy/src/lib.rs:96` (`collateral_amount_for_token`).
- **Rust tests/fuzz:** ~30 files (compiler-enumerated). Central harness wrappers live in `tests/test-harness/src/view.rs` and `tests/test-harness/src/assert.rs`.
- **Integration shell flows:** `tests/integration/flows/{admin,defindex,lifecycle,liquidation}.sh` and `tests/integration/lib/assert.sh` (~25 CLI invoke strings).

---

### Task 1: Rename the ABI surface (wrappers + mirror trait, in lockstep)

Rename only the `#[contractimpl]` wrapper method names and the matching mirror-trait declarations. Wrapper bodies still call the unchanged free functions. The mirror trait and the wrappers MUST change together — they are the two halves of ABI parity.

**Files:**
- Modify: `contracts/controller/src/views/mod.rs` (11 wrapper method names)
- Modify: `contracts/controller/src/governance/access.rs:112` (`app_version` wrapper)
- Modify: `interfaces/controller/src/lib.rs` (11 trait method names; not `app_version`)

**Interfaces:**
- Produces (new entrypoint + client method names consumed by Tasks 2–4): `get_health_factor`, `is_liquidatable`, `get_total_collateral_usd`, `get_total_borrow_usd`, `get_ltv_collateral_usd`, `get_liquidation_collateral`, `get_collateral_amount`, `get_borrow_amount`, `get_app_version`, `get_markets_detailed`, `get_market_indexes_detailed`, `get_liquidation_estimate`. All signatures (params, return types) are unchanged from the current entrypoints.

- [ ] **Step 1: Rename the 11 wrappers in `views/mod.rs`**

In `contracts/controller/src/views/mod.rs`, change ONLY the `pub fn <name>` on each `#[contractimpl]` wrapper (leave the delegated free-function call in the body as-is):

```rust
// line 40
pub fn is_liquidatable(env: Env, account_id: u64) -> bool {
    can_be_liquidated(&env, account_id)
}
// line 44
pub fn get_health_factor(env: Env, account_id: u64) -> i128 {
    health_factor(&env, account_id)
}
// line 48
pub fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
    total_collateral_in_usd(&env, account_id)
}
// line 52
pub fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
    total_borrow_in_usd(&env, account_id)
}
// line 56
pub fn get_collateral_amount(env: Env, account_id: u64, asset: Address) -> i128 {
    collateral_amount_for_token(&env, account_id, &asset)
}
// line 60
pub fn get_borrow_amount(env: Env, account_id: u64, asset: Address) -> i128 {
    borrow_amount_for_token(&env, account_id, &asset)
}
// line 96
pub fn get_markets_detailed(
    env: Env,
    assets: Vec<Address>,
) -> Vec<AssetExtendedConfigView> {
    get_all_markets_detailed(&env, &assets)
}
// line 103
pub fn get_market_indexes_detailed(env: Env, assets: Vec<Address>) -> Vec<MarketIndexView> {
    get_all_market_indexes_detailed(&env, &assets)
}
// line 107
pub fn get_liquidation_estimate(
    env: Env,
    account_id: u64,
    debt_payments: Vec<(Address, i128)>,
) -> LiquidationEstimate {
    liquidation_estimations_detailed(&env, account_id, &debt_payments)
}
// line 115
pub fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
    liquidation_collateral_available(&env, account_id)
}
// line 119
pub fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
    ltv_collateral_in_usd(&env, account_id)
}
```

- [ ] **Step 2: Rename `app_version` wrapper in `access.rs`**

In `contracts/controller/src/governance/access.rs:112`:

```rust
pub fn get_app_version(env: Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::AppVersion)
        .unwrap_or(INITIAL_APP_VERSION)
}
```

- [ ] **Step 3: Rename the matching trait methods in the mirror crate**

In `interfaces/controller/src/lib.rs`, rename the `fn` on each trait method (keep the doc comments and signatures), per the rename map: `can_be_liquidated`→`is_liquidatable` (line 133), `health_factor`→`get_health_factor` (136), `total_collateral_in_usd`→`get_total_collateral_usd` (139), `total_borrow_in_usd`→`get_total_borrow_usd` (142), `collateral_amount_for_token`→`get_collateral_amount` (145), `borrow_amount_for_token`→`get_borrow_amount` (148), `get_all_markets_detailed`→`get_markets_detailed` (182), `get_all_market_indexes_detailed`→`get_market_indexes_detailed` (185), `liquidation_estimations_detailed`→`get_liquidation_estimate` (188), `liquidation_collateral_available`→`get_liquidation_collateral` (195), `ltv_collateral_in_usd`→`get_ltv_collateral_usd` (198).

- [ ] **Step 4: Format the three touched files**

```bash
rustfmt --edition 2021 /Users/mihaieremia/GitHub/rs-lending-xlm/contracts/controller/src/views/mod.rs /Users/mihaieremia/GitHub/rs-lending-xlm/contracts/controller/src/governance/access.rs /Users/mihaieremia/GitHub/rs-lending-xlm/interfaces/controller/src/lib.rs
```

- [ ] **Step 5: Verify the two contracts compile (call-sites elsewhere will not yet)**

```bash
cargo build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -p controller -p controller-interface
```
Expected: PASS. `-p controller` and `-p controller-interface` have no internal callers of the old names, so they compile even though the workspace does not yet.

- [ ] **Step 6: Commit**

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/views/mod.rs contracts/controller/src/governance/access.rs interfaces/controller/src/lib.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): rename view entrypoints to get_/is_ convention"
```

---

### Task 2: Fix Rust consumers (compiler-driven)

Every Rust caller uses `controller::ControllerClient` (auto-generated from the Task 1 wrappers) or `controller_interface::ControllerClient` (the mirror trait). Both are now renamed, so `cargo build` lists every stale call-site with a precise "no method named X; did you mean Y" hint. Fix each by mechanical rename.

**Files:**
- Modify: `contracts/defindex-strategy/src/lib.rs:96` (`collateral_amount_for_token` → `get_collateral_amount`)
- Modify: `tests/test-harness/src/view.rs`, `tests/test-harness/src/assert.rs`, and every `tests/**` / `tests/fuzz/**` file the compiler flags.

**Interfaces:**
- Consumes: the renamed client methods produced by Task 1.

- [ ] **Step 1: Fix the cross-contract consumer**

In `contracts/defindex-strategy/src/lib.rs:96`, change `.collateral_amount_for_token(&account_id, &self.cfg.asset)` to `.get_collateral_amount(&account_id, &self.cfg.asset)`.

- [ ] **Step 2: Build the whole workspace to enumerate stale test call-sites**

```bash
cargo build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml --workspace --all-targets 2>&1 | grep -E "error\[|no method named|--> " | head -200
```
Expected: a list of `no method named '<old>' found` errors with `--> file:line` anchors. These are the exact call-sites to fix.

- [ ] **Step 3: Apply the rename at each flagged call-site**

For each `--> file:line` from Step 2, replace the old method token with its new name from the rename map. Caution: do NOT blind-`sed` — `health_factor` is a substring of the harness helper names `health_factor_for` / `health_factor_raw` (in `tests/test-harness/src/view.rs`), which are harness API names that must NOT change; only the internal `.health_factor(...)` / `.can_be_liquidated(...)` client calls inside those helpers change. Edit each flagged line individually.

- [ ] **Step 4: Re-build until clean**

```bash
cargo build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml --workspace --all-targets
```
Expected: PASS (zero errors). Repeat Step 3 for any remaining flagged lines.

- [ ] **Step 5: Clippy + tests**

```bash
cargo clippy --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml --workspace
```
Expected: clippy clean; all tests PASS with unchanged assertion values.

- [ ] **Step 6: Format touched files + commit**

```bash
# rustfmt only the files you edited, e.g.:
rustfmt --edition 2021 /Users/mihaieremia/GitHub/rs-lending-xlm/contracts/defindex-strategy/src/lib.rs /Users/mihaieremia/GitHub/rs-lending-xlm/tests/test-harness/src/view.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add -A
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): update Rust consumers for renamed view entrypoints"
```

---

### Task 3: Update integration shell flows (grep-driven)

Shell flows invoke entrypoints by their CLI (snake_case) name and are not compiler-checked, so they are enumerated by grep and fixed by hand.

**Files:**
- Modify: `tests/integration/flows/admin.sh`, `tests/integration/flows/defindex.sh`, `tests/integration/flows/lifecycle.sh`, `tests/integration/flows/liquidation.sh`, `tests/integration/lib/assert.sh`

- [ ] **Step 1: List every old name still present in the flows**

```bash
grep -rn -E "health_factor|total_collateral_in_usd|total_borrow_in_usd|ltv_collateral_in_usd|liquidation_collateral_available|collateral_amount_for_token|borrow_amount_for_token|app_version|can_be_liquidated|get_all_markets_detailed|get_all_market_indexes_detailed|liquidation_estimations_detailed" /Users/mihaieremia/GitHub/rs-lending-xlm/tests/integration
```
Expected: ~25 lines across the 5 files.

- [ ] **Step 2: Replace each occurrence with its new entrypoint name**

For each line, swap the old token for the new one (rename map). Note `app_version` appears in the CLI invoke as `-- app_version` and also as a local label `app_version_view`; only the `-- app_version` invoke token becomes `-- get_app_version` (leave label variable names alone — they are not ABI).

- [ ] **Step 3: Verify no old name remains**

```bash
grep -rn -E "(^|[^_a-z])(health_factor|total_collateral_in_usd|total_borrow_in_usd|ltv_collateral_in_usd|liquidation_collateral_available|collateral_amount_for_token|borrow_amount_for_token|can_be_liquidated|get_all_markets_detailed|get_all_market_indexes_detailed|liquidation_estimations_detailed)([^_a-z]|$)" /Users/mihaieremia/GitHub/rs-lending-xlm/tests/integration; echo "exit=$?"
grep -rn -- "-- app_version" /Users/mihaieremia/GitHub/rs-lending-xlm/tests/integration; echo "exit=$?"
```
Expected: no matches (grep `exit=1` on both).

- [ ] **Step 4: Commit**

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add tests/integration
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "test(integration): update shell flows for renamed view entrypoints"
```

---

### Task 4: Rebuild wasm, full verification, and SDK regression check

**Files:** none (verification only).

- [ ] **Step 1: Rebuild contract wasm so the on-chain spec reflects the new names**

```bash
stellar contract build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml
```
Expected: PASS; `target/wasm32v1-none/release/controller.wasm` regenerated.

- [ ] **Step 2: Confirm the new entrypoint names are in the built spec**

```bash
stellar contract info interface --wasm /Users/mihaieremia/GitHub/rs-lending-xlm/target/wasm32v1-none/release/controller.wasm 2>/dev/null | grep -E "get_health_factor|is_liquidatable|get_markets_detailed|get_liquidation_estimate|get_app_version"
```
Expected: each new name appears; none of the old names appear.

- [ ] **Step 3: Full Rust verification bar (re-run post-build)**

```bash
cargo clippy --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml --workspace
```
Expected: clean; all tests PASS.

- [ ] **Step 4: SDK regression check — prove the SDK is unaffected**

```bash
grep -rn -E "health_factor|can_be_liquidated|collateral_amount_for_token|get_all_markets_detailed|get_all_market_indexes_detailed|liquidation_estimations_detailed|ltv_collateral_in_usd|total_collateral_in_usd|total_borrow_in_usd|borrow_amount_for_token|liquidation_collateral_available" /Users/mihaieremia/GitHub/sdk-js/src 2>/dev/null; echo "exit=$?"
```
Expected: no matches (`exit=1`). The SDK builds only write txs and references none of these views, so no SDK change is required. If this ever returns a match, stop and add the SDK to scope.

- [ ] **Step 5: Open the PR**

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm push -u origin HEAD
gh --repo xoxno/rs-lending-xlm pr create --title "refactor(controller): align view entrypoints to get_/is_ convention" --body "Renames 12 controller read entrypoints to a consistent get_/is_ scheme (matches the pool's view rename). Behavior-preserving. SDK verified unaffected; api-v2 follow-up tracked separately (deploy-coupled)."
```

---

## Deferred — Phase 2 (NOT in this plan; separate PR + coordinated deploy)

The renamed views are read on-chain via simulate by **api-v2** (confirmed consumers):
- `xoxno-api-v2/src/endpoints/blend-data/blend-data.service.ts`
- `xoxno-api-v2/src/endpoints/lending-data/stellar-reconciler.service.ts`
- `xoxno-api-v2/src/common/stellar-oracle/stellar-oracle.service.ts`

`az-functions` and `ui`: no references to these views were found, but re-grep before deploy to be safe.

**Rollout coupling (critical):** the controller wasm in this plan must NOT be upgraded on testnet until the api-v2 PR is merged and ready to ship. The instant the controller is upgraded with renamed entrypoints, the three api-v2 simulate calls above break. Land this repo's PR, land the api-v2 PR, then upgrade the controller and deploy api-v2 together.

---

## Self-Review

- **Spec coverage:** All 12 renames from the agreed set (8 `get_` getters + `is_liquidatable` + 3 `_detailed`/`all_` fixes) are in the rename map and Task 1. ✓
- **Surfaces covered:** wrappers (Task 1), mirror trait (Task 1), defindex consumer (Task 2), Rust tests/fuzz (Task 2, compiler-driven), shell flows (Task 3), wasm spec (Task 4), SDK regression (Task 4). ✓
- **Placeholders:** none — every step has the concrete code or exact command. The test/fuzz call-sites are intentionally compiler-enumerated rather than pre-listed, because a rename's safe enumeration is the compiler's job and the set is ~30 files of identical one-token edits. ✓
- **Type/name consistency:** new names used in Tasks 2–4 match Task 1's "Produces" block exactly; signatures unchanged. ✓
- **Substring hazard noted:** `health_factor` ⊂ `health_factor_for`/`health_factor_raw` harness helpers — flagged in Task 2 Step 3 (no blind sed). ✓
