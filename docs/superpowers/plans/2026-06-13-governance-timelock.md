# Governance Timelock (OpenZeppelin) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Embed the OpenZeppelin `stellar_governance::timelock` module into the governance contract so every state-changing admin operation (except the emergency pause/unpause) must be scheduled, wait a 48h minimum delay, then be executed — closing threat-model residual #1 (governance without timelock).

**Architecture:** Governance embeds the OZ `Timelock` `#[contracttrait]`. Scheduled `Operation`s target governance *itself* (`target = current_contract`, `function = forwarder name`, `args`), so at execute time the existing typed forwarders re-run their full validation and forward to the controller. Forwarders change from `#[only_owner]` to **self-only** (reachable only via the timelock `execute` path). `pause`/`unpause` and the one-time `deploy_controller` bootstrap stay immediate. Timelock roles (PROPOSER/EXECUTOR/CANCELLER) live in governance's existing access_control; admin = governance self, so changing the delay or roles is itself timelocked.

**Tech Stack:** Rust / soroban-sdk =26.0.0; `stellar-governance` 0.7.1 (vendored, `[patch.crates-io]`); stellar-access/stellar-contract-utils 0.7.1; GNU make + stellar CLI 26.1.

**Branch:** continue on `feat/governance-split` (timelock depends on the unmerged governance contract; same logical feature).

**Owner decisions (locked — see memory `governance-timelock-decisions`):** scope = all admin except pause; min_delay = 48h (≈34,560 ledgers at ~5s/ledger), config-parameterized so testnet can use a short value for live e2e; admin = self; initial PROPOSER+EXECUTOR+CANCELLER = deploying owner.

**Verification bar (every task):** `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` (known pre-existing failure: meta `budget_withdraw_5_collateral_double_pass`); `cargo test -p governance`; `make -C <repo> build` + `wasm-size-check`. No `cd` in Bash (use absolute paths / `--manifest-path` / `make -C` / `git -C`); never `--all-features`; no workspace-wide `cargo fmt` (rustfmt touched files only); present-tense comments.

---

### Task 1: Vendor `stellar-governance` 0.7.1 + design spike (REVIEW CHECKPOINT)

**Files:** `vendor/openzeppelin/stellar-governance/**` (new), `Cargo.toml` (workspace dep + patch), `docs/superpowers/specs/timelock-integration.md` (new design note)

- [ ] **Step 1:** Obtain `stellar-governance` 0.7.1 source matching the vendored stellar-access/stellar-contract-utils 0.7.1. Use `cargo vendor` against a scratch crate that depends on `stellar-governance = "=0.7.1"`, or fetch the crates.io `.crate` tarball, and place it at `vendor/openzeppelin/stellar-governance/` mirroring the existing two vendored crates' layout. Confirm its `Cargo.toml` deps resolve against soroban-sdk =26.0.0 and stellar-access/stellar-contract-utils 0.7.1 (same workspace pins).
- [ ] **Step 2:** Add `stellar-governance = "=0.7.1"` to root `[workspace.dependencies]` and `stellar-governance = { path = "vendor/openzeppelin/stellar-governance" }` under `[patch.crates-io]` (mirror the stellar-access entry at Cargo.toml:55).
- [ ] **Step 3 (SPIKE — read the real vendored source, do not guess):** Read `vendor/openzeppelin/stellar-governance/src/timelock/{mod.rs,storage.rs}` and any roles module. Write `docs/superpowers/specs/timelock-integration.md` capturing VERBATIM with file:line: (a) the full `Timelock` trait — which methods have defaults, which the host implements; (b) the `Operation` type and `OperationState` enum; (c) **how `execute_operation` invokes the target** — does it call `e.authorize_as_current_contract(...)` / `e.invoke_contract(...)`? This determines whether a self-targeted op can satisfy a forwarder's auth — the linchpin of the whole design; (d) role constants (PROPOSER/EXECUTOR/CANCELLER) and the storage helpers the host calls (`schedule_operation`/`execute_operation`/`cancel_operation`/`set_min_delay`); (e) the delay unit (ledgers) and how min_delay is stored/initialized; (f) any constructor/init helper. Then state the LOCKED integration design: forwarder auth change mechanism (self `require_auth` vs transient executing-flag), exact min_delay constant (48h→ledgers with the ledger-close assumption named), and the public entrypoint surface. IF the source shows execute does NOT authorize-as-current-contract (so self-targeted forwarders can't be reached), STOP and report BLOCKED with the alternative (schedule targets controller directly + validation moves to typed schedulers) for re-decision.
- [ ] **Step 4:** `cargo check --workspace` (the vendored crate compiles in the tree, even before use). Commit `chore(vendor): add OpenZeppelin stellar-governance 0.7.1` + the spec note.

### Task 2: Wire the Timelock trait into governance (storage + roles + constructor)

**Files:** `contracts/governance/Cargo.toml` (dep), `contracts/governance/src/lib.rs`, `contracts/governance/src/access.rs` (constructor + roles), `contracts/governance/src/timelock.rs` (new), `contracts/governance/src/storage.rs` (role/key additions if needed)

- [ ] **Step 1 (TDD):** Write failing tests in `governance/src/timelock.rs` `#[cfg(test)]`: constructor grants PROPOSER/EXECUTOR/CANCELLER to admin and sets min_delay; `get_min_delay` returns the configured value; `get_operation_state(unknown)` == Unset. Run, confirm fail.
- [ ] **Step 2:** Add `stellar-governance = { workspace = true }` to governance Cargo.toml. Implement the `Timelock` trait for `Governance` in `timelock.rs` (its own `#[contractimpl]`/trait impl block): `schedule`/`execute`/`cancel`/`update_delay` each do `renew_governance_instance`, the role `require_auth` + role check per the spec note, then delegate to the OZ storage helper; query methods use the trait defaults. Use a named `TIMELOCK_MIN_DELAY_LEDGERS` constant (value per spec note; document the ledger-close assumption) in `governance/src/constants.rs` (create if absent) — but make the constructor accept `min_delay: u32` so deploys parameterize it.
- [ ] **Step 3:** Extend `__constructor` (access.rs) to also take `min_delay: u32` (and optionally proposer/executor sets — default: grant all three timelock roles to `admin`); call the OZ min_delay initializer; admin of the timelock = governance self. Keep ownable owner + ORACLE role as today. Update the scaffold test + any harness `env.register(Governance, (admin,))` call sites to the new constructor arity (search workspace).
- [ ] **Step 4:** Verify bar green; `cargo test -p governance`. Commit `feat(governance): embed OpenZeppelin timelock trait + roles`.

### Task 3: Route forwarders through the timelock (self-only + execute path)

**Files:** `contracts/governance/src/forward.rs`, `contracts/governance/src/deploy.rs`, `contracts/governance/src/timelock.rs`

- [ ] **Step 1 (TDD):** Write failing integration tests (governance tests.rs): (a) a direct call to a forwarder (e.g. `set_aggregator`) by the owner now REVERTS (no longer directly callable — must go through timelock); (b) `schedule`(op targeting self `set_aggregator`) by a proposer → `get_operation_state` == Waiting; (c) advancing the ledger past min_delay then `execute` → the controller reflects the change (validation ran at execute); (d) `execute` before delay reverts; (e) `cancel` by canceller drops the op; (f) bad input scheduled → `execute` reverts with the validation error (e.g. InvalidPositionLimits) — proving execute-time validation. Use `env.ledger().set_sequence_number(...)`/timestamp to cross the delay. Run, confirm fail.
- [ ] **Step 2:** Change every state-changing forwarder in forward.rs from `#[only_owner]`/`#[only_role(...)]` to **self-only** per the spec note's mechanism (the forwarder asserts it is being invoked by the governance contract itself during execute). This includes the 2 previously-ORACLE-gated oracle forwarders. Validation bodies are UNCHANGED — they still validate then call the controller client. `pause`/`unpause` KEEP `#[only_owner]` (immediate). `deploy_controller`/`set_controller` stay as-is (genesis bootstrap, immediate).
- [ ] **Step 3:** Confirm the schedule→execute round-trip: a proposer schedules `Operation{target: env.current_contract_address(), function: symbol_short!("set_aggr…")/Symbol::new(forwarder), args}`; execute invokes it; the forwarder's self-auth check passes because OZ execute authorizes-as-current-contract. (If the spec note chose the transient-flag mechanism instead, set/clear the flag in execute around the invoke.)
- [ ] **Step 4:** Verify bar green; `make build` + record governance.wasm size (expect growth — confirm still under the 50,000 budget, bump budget if needed with justification). Commit `feat(governance)!: timelock all admin forwarders except pause`.

### Task 4: Test suite — timelock coverage + migrate harness admin path

**Files:** `verification/test-harness/src/setup/builder.rs`, `verification/test-harness/tests/governance/timelock.rs` (new), existing governance tests

- [ ] **Step 1:** The harness builder calls governance admin forwarders directly for setup (Task-6 of the split). Those now revert (self-only). Update the builder to drive setup through schedule→advance-ledger→execute, OR add a testing-gated immediate-admin path. DECISION: add a `#[cfg(any(test, feature = "testing"))]` helper on governance that schedules+executes in one call with `min_delay` bypassed (e.g. `test_execute_now(operation)`), used only by the harness, so the 400+ existing tests don't each have to advance ledgers. Document it as testing-only (same pattern as `set_controller`). Wire the builder through it.
- [ ] **Step 2:** New `tests/governance/timelock.rs` integration suite: full lifecycle per Task 3 Step 1, plus: non-proposer cannot schedule; non-executor cannot execute; non-canceller cannot cancel; `update_delay` is itself timelocked (self-admin); predecessor ordering (op B blocked until op A executed) if the OZ module supports it; `get_operation_state` transitions Unset→Waiting→Ready→Done.
- [ ] **Step 3:** Keep every existing suite green (`cargo test --workspace`; `-p governance --lib`; `-p controller --lib`). Update the auth fuzz proptest (`tests/fuzz/privileged_auth_rejects.rs`) — the forwarders are no longer owner-gated; assert instead they reject direct calls (self-only). Commit `test(governance): timelock lifecycle + harness execute-now path`.

### Task 5: Deploy tooling — schedule/execute/cancel verbs + min_delay config

**Files:** `Makefile`, `configs/script.sh`, `configs/networks.json`, `configs/*` as needed

- [ ] **Step 1:** networks.json: add `"timelock_min_delay_ledgers"` per network (testnet: a short value e.g. 12 for live e2e; mainnet: 34560). Governance deploy (`_deploy`) passes `--min_delay` to the constructor.
- [ ] **Step 2:** script.sh: the admin verbs (create_market, edit_asset_config, oracle config, e-mode, setters, role grants, upgrades) now produce a two-step flow. Add `schedule`/`execute`/`cancel`/`opState` helpers that build the `Operation` (target=governance self, function + args) and call governance `schedule`/`execute`/`cancel`. Existing admin functions become "schedule the op" + print the operation id and the ready-ledger; add a companion `execute <op-id>` path. pause/unpause stay direct. Views unchanged.
- [ ] **Step 3:** Makefile setup flow: `setup-testnet` must schedule each market/e-mode/oracle op then execute after the (short testnet) delay — add a `_await-timelock` helper that polls `get_operation_state` until Ready (or sleeps the known short delay) then executes. Keep it robust (fail loudly if an op never reaches Ready). `bash -n` + `make -n` dry-run sanity.
- [ ] **Step 4:** Commit `feat(deploy): timelock schedule/execute tooling`.

### Task 6: Testnet end-to-end redeploy through the timelock

- [ ] **Step 1:** `make deploy-testnet` (governance with short min_delay → deploy_controller → pool). Then drive market/e-mode/oracle setup via schedule→await→execute. Record addresses.
- [ ] **Step 2:** Prove the lifecycle on-chain: schedule one op, show `get_operation_state` == Waiting before delay and Ready after, execute it, confirm the controller reflects the change; show a `cancel` on a second op; confirm `pause` is still immediate. Confirm a direct forwarder call reverts (self-only).
- [ ] **Step 3:** Commit `chore(deploy): record timelocked governance testnet deployment`.

### Task 7: Docs, memory, final review

- [ ] **Step 1:** Update the architecture doc/ADR: timelock topology, scope (all-admin-except-pause), 48h/ledgers, role model (PROPOSER/EXECUTOR/CANCELLER, self-admin), schedule→execute→done lifecycle, validation-at-execute. Note SDK/types follow-up (admin tx builders now build Operations + schedule/execute).
- [ ] **Step 2:** Update memory (`governance-timelock-decisions` with realized sizes/addresses; threat-model cross-map residual #1 → CLOSED).
- [ ] **Step 3:** Final whole-implementation reviewer pass; then fold in the deferred governance-split Task-10 cleanups (review-nit batch) and run `superpowers:finishing-a-development-branch`.

## Self-review notes
- Linchpin risk: the self-targeted-execute design depends on OZ `execute_operation` authorizing-as-current-contract — Task 1 spike confirms before any forwarder auth changes; BLOCKED path documented.
- pause/unpause immediate (safety invariant); deploy_controller immediate (genesis); everything else timelocked (owner decision).
- Delay in LEDGERS not seconds — every delay value carries the ledger-close assumption.
- Harness `test_execute_now` testing-gated bypass keeps the 400+ existing tests fast without each advancing ledgers (same precedent as `set_controller`).
- Known traps: workspace feature-unification (governance production-only tests gate on `-p governance --lib`); wasm budget bump for governance; no `--all-features`; rustfmt touched-files-only.
