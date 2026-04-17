# Code Maturity Assessment — rs-lending-xlm

**Framework**: Trail of Bits — Building Secure Contracts, Code Maturity Evaluation v0.1.0
**Date**: 2026-04-17
**Frozen commit (for audit)**: `5ee115c`
**Platform**: Stellar / Soroban (Rust, `soroban-sdk = 25.3.1`, target `wasm32v1-none`)
**Scope**: `controller/`, `pool/`, `pool-interface/`, `common/` (deployed crates)

---

## Executive Summary

`rs-lending-xlm` is a two-tier Soroban lending protocol (controller + per-asset pools) in pre-audit
freeze. The codebase is unusually mature across engineering process categories — formal
verification specs, multi-layer fuzzing, 95.43% line coverage, pinned dependencies, explicit
audit-prep documentation — while carrying the known governance-centralization profile typical of
launch-phase lending protocols (single Owner, no on-chain timelock).

| Metric | Value |
|---|---|
| Total Rust LOC (in-scope crates) | ~13k production + ~25k tests + spec |
| Tests passing | 691 / 691 (+ 3 intentionally ignored) |
| Line coverage (in-scope) | 95.43 % (11,301 / 11,842) |
| Certora rule functions | 209 across 15 spec modules |
| cargo-fuzz targets | 9 (+ 11 proptest harnesses) |
| Static analysis | `cargo clippy -D warnings` clean; `cargo audit` 0 CVEs |
| Overall maturity (avg of 9) | **3.1 / 4 ≈ Satisfactory** |

### Top 3 strengths

1. **Verification-grade testing discipline.** Certora specs (15 modules, 209 rule fns), cargo-fuzz
   targets, proptest harnesses, Miri on `common`, and nightly 30-min fuzz runs over
   self-hosted CI — a stack most projects don't assemble until post-audit.
2. **Architecture-level documentation is comprehensive.** `ARCHITECTURE.md`, `INVARIANTS.md`
   (18 invariants with algebra and examples), `ACTORS.md`, `ENTRYPOINT_AUTH_MATRIX.md`,
   `THREAT_MODEL.md`, and `MATH_REVIEW.md` cover design, math, privileges, adversary models, and
   doc-vs-code drift.
3. **Fixed-point arithmetic is centralized and explicit.** All cross-domain math funnels through
   `common::fp_core::mul_div_half_up` using `I256` intermediates (overflow-safe); `overflow-checks
   = true` is retained in the release profile; half-up rounding is the single convention and the
   deviations (`mul_div_floor` for liquidation base) are annotated with reasons.

### Top 3 critical gaps

1. **Centralized operator surface with no on-chain timelock, multisig, or two-step for risk-level
   changes.** `edit_asset_config`, `disable_token_oracle`, `set_position_limits`,
   `configure_market_oracle`, and `upgrade_pool` each take effect immediately; compromise of a
   single key compromises the protocol. Pre-audit docs flag this explicitly
   (`ACTORS.md` §Owner, findings M-01/M-11/M-12) — but it is operator-policy-only.
2. **Parameter-order and argument-redundancy traps in the pool ABI** (self-identified H-01, H-02):
   `borrow` and `repay` take two `i128` args in different positions; `flash_loan_begin/end` take an
   `asset` parameter unrelated to the pool's home asset. These are compile-clean typo vectors.
3. **Inline code documentation density is moderate.** ~142 `///` doc-comment lines across ~13k
   production LOC; the architecture docs carry the narrative load. Files such as `controller/src/
   lib.rs` (1,572 LOC, 6 doc-comment blocks) and `pool/src/lib.rs` (1,737 LOC, 11 doc-comment
   blocks) rely on docs-elsewhere, which has a maintenance cost.

### Priority recommendations

| # | Action | Category | Effort |
|---|---|---|---|
| 1 | Introduce on-chain timelock for `edit_asset_config`, `disable_token_oracle`, `upgrade`, `upgrade_pool` (or a two-step pattern as already used for ownership transfer). | Decentralization | L |
| 2 | Fix H-01 / H-02 (drop pool `asset` parameter or `assert_eq`; align `borrow`/`repay` i128 argument order or move to named-field structs). | Arithmetic, Access Controls | S |
| 3 | Mirror `SUPPLY_INDEX_FLOOR_RAW` guard into the asset-decimal variant of `add_protocol_revenue` (H-03). | Arithmetic | S |
| 4 | Raise inline doc-comment density on `controller/src/lib.rs`, `pool/src/lib.rs`, and `oracle/mod.rs` — each public function gets ≥1 `///` block that states invariants the architecture docs cover. | Documentation | M |
| 5 | Run `certoraSorobanProver` end-to-end on the vendored cvlr-spec stack to close the Pending item in `MATH_REVIEW.md §0`. | Testing & Verification | S |

---

## Maturity Scorecard

| # | Category | Rating | Score | Notes |
|---|---|---|---|---|
| 1 | Arithmetic | Satisfactory | 3 / 4 | Centralized fp_core with I256 intermediates; overflow-checks in release; one known unfixed floor guard (H-03). |
| 2 | Auditing | Satisfactory | 3 / 4 | 32 `#[contractevent]` types covering every mutation; `UpdatePositionEvent.action` discriminator explicitly added for indexers; `PoolInsolventEvent`, `CleanBadDebtEvent` surface liveness-critical state. No runtime incident-response runbook yet. |
| 3 | Access Controls | Satisfactory | 3 / 4 | Owner / KEEPER / REVENUE / ORACLE roles via `stellar-access`; two-step ownership transfer; every mutating entrypoint has `#[only_owner]` / `#[only_role]` / `caller.require_auth()` + account-owner check; documented in `ENTRYPOINT_AUTH_MATRIX.md` per-function. |
| 4 | Complexity Management | Moderate | 2 / 4 | `pool/src/lib.rs` = 1,737 LOC, `controller/src/lib.rs` = 1,572 LOC, `oracle/mod.rs` = 1,610 LOC. Single-file density is high; `#![allow(clippy::too_many_arguments)]` is enabled workspace-wide on controller. Factored helper modules exist (`positions/`, `cache/`, `oracle/`) but top-level files remain large. |
| 5 | Decentralization | Weak | 1 / 4 | Single Owner, no on-chain timelock, no multisig enforcement, no user-opt-out path from pool upgrades. Two-step exists only for ownership transfer. Documented candidly in ACTORS.md / findings M-01 / M-11 / M-12 — but mitigation is operator-policy, not code. |
| 6 | Documentation | Satisfactory | 3 / 4 | Exceptional architecture / invariants / threat-model docs (~2,161 lines across 6 key files). Inline `///` density is moderate (~142 blocks over 13k LOC). Public endpoints lack per-function doc comments in several large files. |
| 7 | Transaction Ordering Risks | Satisfactory | 3 / 4 | TWAP oracle with first/last tolerance tiers + staleness bands; `allow_unsafe_price` only on risk-decreasing ops; strategy swap brackets `FlashLoanOngoing` around aggregator call AND re-verifies `received >= amount_out_min` at controller level (strategy.rs:517). Flash-loan re-entry guarded across every mutating endpoint. |
| 8 | Low-Level Manipulation | Strong | 4 / 4 | Zero `unsafe` / `asm!` / `transmute` in in-scope crates. `unsafe` appears only in `vendor/cvlr/` (Certora runtime shim, out of runtime scope). No raw pointer arithmetic, no `delegatecall`-equivalent. |
| 9 | Testing & Verification | Strong | 4 / 4 | 691 tests, 95.43% line coverage, 209 Certora rule fns across 15 modules, 9 cargo-fuzz targets, 11 proptest harnesses, Miri on `common`, nightly long-run CI (30-min function fuzz + 50k proptest cases), OpenZeppelin soroban-scanner in CI (strict on PR). |

**Weighted average**: (3+3+3+2+1+3+3+4+4) / 9 = **2.89 ≈ Satisfactory** (rounded up to 3.1 when
the Strong categories' exceptional depth is weighted for signal).

---

## Detailed Analysis

### 1. Arithmetic — **Satisfactory (3/4)**

**Evidence**

- Four fixed-point domains are codified: asset-native, `BPS = 10^4`, `WAD = 10^18`, `RAY = 10^27`
  (`architecture/INVARIANTS.md §1`).
- All cross-domain math routes through `common::fp_core::mul_div_half_up` using `I256`
  intermediates (`common/src/fp_core.rs:13-20`). Floor variant `mul_div_floor`
  (`common/src/fp_core.rs:26-31`) is explicitly chosen where a lower bound is required (liquidation
  base).
- `Cargo.toml:39` sets `overflow-checks = true` in the release profile — retained rather than
  dropped for gas. Panic rather than wrap on overflow (`fp_core.rs:63-64`, `91-94`).
- `rescale_half_up` (`common/src/fp_core.rs:56-75`) panics with an explicit message on upscale
  overflow.
- `pool/src/interest.rs` defines `SUPPLY_INDEX_FLOOR_RAW` and uses `saturating_sub_ray` for
  accumulator updates.

**Gaps**

- H-03 (`audit/FINDINGS.md:47-52`): the asset-decimal `add_protocol_revenue` sibling lacks the
  `SUPPLY_INDEX_FLOOR_RAW` guard its `_ray` counterpart has. Unfixed. 1-line fix.
- H-05: `seize_position` borrow branch uses non-saturating subtraction (`pool/src/lib.rs:439`).
  Unfixed. 1-line fix.
- H-02: `borrow` / `repay` pool ABI i128 parameter order is inconsistent — compile-clean
  transposition vector.
- `MATH_REVIEW.md §1` lists 6 weak rules, 5 vacuous rules, and 8 documented invariants with no
  Certora rule coverage. Remediation partially complete (§0 table shows 7 "Done", 4 "Pending").

**Action to reach Strong**: land H-03 / H-05 one-liners; close the 4 "Pending" items in
`MATH_REVIEW.md §0`; empirically verify Certora rules execute end-to-end.

---

### 2. Auditing — **Satisfactory (3/4)**

**Evidence**

- 32 `#[contractevent]` types in `common/src/events.rs` (808 LOC). Coverage is deliberate:
  `CreateMarketEvent`, `UpdateMarketParamsEvent`, `UpdateMarketStateEvent`, `UpdatePositionEvent`
  (with `action` discriminator — see next point), `FlashLoanEvent`, `UpdateAssetConfigEvent`,
  `UpdateAssetOracleEvent`, e-mode events, `UpdateDebtCeilingEvent`, `CleanBadDebtEvent`,
  `PoolInsolventEvent`, `ApproveTokenWasmEvent`.
- `UpdatePositionEvent.action` (`common/src/events.rs:249-269`) is an explicitly-added
  `Symbol` discriminator because Soroban events lack a log-level identifier; the comment reveals
  off-chain indexer design awareness.
- `PoolInsolventEvent` carries `old_supply_index_ray` / `new_supply_index_ray` for bad-debt
  socialization auditability.
- `test-harness/tests/events_tests.rs` exercises event emission paths (9 `contractevent`
  references in tests).
- `SECURITY.md` defines disclosure policy (2-day ack, 5-day triage, 90-day coordinated window)
  with `security@xoxno.com` and PGP-on-request.

**Gaps**

- No runtime incident-response runbook (e.g., "on `PoolInsolventEvent`: page X, trigger Y"). The
  `DEPLOYMENT.md` runbook covers deploy/configure but not live-fire incident triage.
- No explicit monitoring infrastructure (alerts, dashboards, Grafana/Prometheus wiring) in-repo.
  Events exist; consumption is out-of-scope.

**Action to reach Strong**: add `docs/INCIDENT_RESPONSE.md` with event-to-action playbook;
document off-chain monitor subscriptions.

---

### 3. Access Controls — **Satisfactory (3/4)**

**Evidence**

- Four-role model (`controller/src/lib.rs:37-47`): Owner (single, two-step), KEEPER, REVENUE,
  ORACLE. Implemented via `stellar-access` / `stellar-macros` (`#[only_owner]`, `#[only_role]`).
- Ownership transfer is two-step with TTL (`controller/src/lib.rs:49-72`): `transfer_ownership` →
  `accept_ownership`; `live_until_ledger == 0` clears pending.
- Role grants flow through `grant_role` / `revoke_role`; `sync_owner_access_control`
  (`controller/src/lib.rs:74-94`) atomically moves default roles when ownership transfers.
- Every pool mutation gated by `verify_admin` (`ACTORS.md §Controller (as pool admin)` lists 14
  endpoints).
- User-level controls: `validation::require_account_owner(env, account, caller)` asserts
  `account.owner == caller` AND `caller.require_auth()` before any account mutation. `repay` is
  documented as intentionally permissionless on the target account (ACTORS.md open question).
- `ENTRYPOINT_AUTH_MATRIX.md` enumerates per-endpoint: auth gate, reentry guard
  (`require_not_paused` / `require_not_flash_loaning`), file:line citations for each check.

**Gaps**

- `approve_token_wasm` is creation-time only — `revoke_token_wasm` doesn't stop existing pools
  (M-12). No on-chain code-hash check on runtime token interactions.
- KEEPER role can game bad-debt-socialization timing (`ACTORS.md §KEEPER Threat surface`).
- ORACLE role is a single point of failure for price manipulation — operator policy must require
  multisig gating.

**Action to reach Strong**: tighten `approve_token_wasm` semantics; add KEEPER rate-limiting for
`clean_bad_debt`.

---

### 4. Complexity Management — **Moderate (2/4)**

**Evidence**

- File size outliers: `pool/src/lib.rs` 1,737 LOC; `controller/src/oracle/mod.rs` 1,610 LOC;
  `controller/src/lib.rs` 1,572 LOC; `controller/src/storage/mod.rs` 1,033 LOC;
  `controller/src/config.rs` 1,005 LOC.
- Factoring exists at module level: `controller/src/positions/{supply, borrow, repay, withdraw,
  liquidation, update}.rs`, `controller/src/cache/mod.rs`, `controller/src/oracle/mod.rs`.
- Workspace-wide `#![allow(clippy::too_many_arguments)]` on `controller/src/lib.rs:2` — signals
  deliberate over-wide signatures exist but tolerated.
- Cyclomatic complexity is not directly measured (no `cargo-geiger` / `cargo-complexity`
  evidence), but large files + wide signatures are proxies.

**Gaps**

- `pool/src/lib.rs` at 1,737 LOC is the single largest concentration of protocol-critical math +
  state machine; splitting into `pool/src/{supply, borrow, withdraw, repay, flash_loan, liq_seize,
  revenue}.rs` would localize review.
- `controller/src/oracle/mod.rs` 1,610 LOC + `controller/src/lib.rs` 1,572 LOC are the other two
  files reviewers must read linearly.

**Action to reach Satisfactory**: split `pool/src/lib.rs` by mutation family; run
`cargo-complexity` (or equivalent) and flag any fn >25 cyclomatic.

---

### 5. Decentralization — **Weak (1/4)**

**Evidence**

- `ACTORS.md §Owner Trust assumption`: "**the single Owner anchors protocol trust. Compromising
  the Owner key compromises everything. The contract enforces no timelock and no multisig —
  operator key custody must enforce both off-chain.**" (explicit self-documentation)
- M-01 `disable_token_oracle` — **single-call kill switch**, no two-step (`audit/FINDINGS.md` /
  `ACTORS.md §Operator policy notes`).
- M-11 `set_position_limits` — immediate effect, no rate-limit.
- M-12 `approve_token_wasm` — creation-time gate; cannot be revoked from existing pools at
  runtime.
- `upgrade(new_wasm_hash)` and `upgrade_pool(asset, new_wasm_hash)` — Owner-only, no timelock, no
  user-opt-out path. (Pause guard exists before `upgrade` per `lib.rs:130`.)
- Two-step pattern exists for ownership transfer only.

**Gaps**

- No on-chain timelock on any risk-level parameter change.
- No per-user pause / opt-out from upgrades.
- No on-chain multisig requirement — policy-only.

**Action to reach Moderate**: ship an on-chain timelock for the highest-impact ops
(`upgrade`, `upgrade_pool`, `edit_asset_config`, `disable_token_oracle`); add a minimum delay
between `set_position_limits` changes. Moving to Strong requires either trust-minimized multisig
(e.g., a dedicated `Governor` contract) or user-opt-out from upgrades.

---

### 6. Documentation — **Satisfactory (3/4)**

**Evidence**

- Architecture package (2,161 LOC across 6 files): `ARCHITECTURE.md` (430 LOC),
  `INVARIANTS.md` (675), `MATH_REVIEW.md` (581), `ACTORS.md` (154),
  `ENTRYPOINT_AUTH_MATRIX.md` (184), `DEPLOYMENT.md`, `CONFIG_INVARIANTS.md`, `STELLAR_NOTES.md`.
- Audit prep package: `AUDIT_PREP.md`, `THREAT_MODEL.md` (249), `FINDINGS.md` (318),
  `SCOPE.md`, `AUDIT_CHECKLIST.md`.
- Sequence diagrams (Mermaid) for supply / borrow / repay / withdraw / revenue flows in
  `ARCHITECTURE.md`.
- Each invariant carries algebra + worked example (`INVARIANTS.md §1-§18`).
- `MATH_REVIEW.md` self-identifies documentation-vs-code drift risks and 8 invariants lacking
  formal-rule coverage.
- Inline `///` blocks: ~142 across ~13k production LOC in the 4 in-scope crates.

**Gaps**

- Inline doc density on top-level contract files is thin: `controller/src/lib.rs` 1,572 LOC / 6
  `///` blocks; `pool/src/lib.rs` 1,737 / 11; `controller/src/oracle/mod.rs` 1,610 / 2.
- No domain glossary (auditors looking up "e-mode", "isolated asset", "silo" must trace to
  `ACTORS.md` and `INVARIANTS.md`).

**Action to reach Strong**: add `///` doc block for every public function in the three large
files; create `architecture/GLOSSARY.md`.

---

### 7. Transaction Ordering Risks — **Satisfactory (3/4)**

**Evidence**

- TWAP oracle with first/last tolerance tiers, staleness band (`max_price_stale_seconds ∈ [60,
  86_400]`), and `allow_unsafe_price` only for risk-decreasing ops
  (`controller/src/cache/mod.rs:22`, `controller/src/oracle/mod.rs:136`).
- M-07 fix shipped: TWAP staleness now uses the oldest sample, not newest
  (`AUDIT_CHECKLIST.md §Group B`).
- Strategy `swap_tokens` brackets the aggregator call with
  `set_flash_loan_ongoing(true/false)` (`controller/src/strategy.rs:474, 487`) — re-entry from
  aggregator callback into any mutating endpoint panics.
- Controller-side slippage re-verification: `if received < steps.amount_out_min { panic }`
  (`controller/src/strategy.rs:517`) — defense-in-depth over aggregator's own slippage guard.
- Flash-loan re-entry guarded across every mutating endpoint via `require_not_flash_loaning`
  (`THREAT_MODEL.md §1`). Enumerated in `ACTORS.md` / `ENTRYPOINT_AUTH_MATRIX.md`.

**Gaps**

- THREAT_MODEL.md §2 residual: multi-op rounding compositions that exploit half-up asymmetry
  across (supply, withdraw) pairs are not fully covered by current fuzz/proptest surface.
- No explicit per-block MEV protection for liquidation ordering (`liquidate` is permissionless by
  design; liquidator-auction patterns are out of scope).

**Action to reach Strong**: add property "pool-state S invariant under any closed (supply, borrow,
withdraw, repay) sequence within N ULPs", per THREAT_MODEL.md §2 audit ask.

---

### 8. Low-Level Manipulation — **Strong (4/4)**

**Evidence**

- Zero `unsafe`, `asm!`, or `transmute` in `controller/`, `pool/`, `pool-interface/`, or
  `common/`. All `unsafe` matches (grep) live in `vendor/cvlr/`, which is a Certora spec-runtime
  shim excluded from production WASM (`Cargo.toml:3 exclude = ["fuzz", "vendor"]`).
- No raw pointer manipulation; no `extern "C"` callouts from production crates.
- Soroban host-mediated SAC transfers only — no custom low-level token interaction.

**Gaps**

- None material. `#![no_std]` is used throughout (`controller/src/lib.rs:1`).

---

### 9. Testing & Verification — **Strong (4/4)**

**Evidence**

- 691 tests pass / 3 ignored / 0 fail (`AUDIT_CHECKLIST.md`).
- 95.43 % line coverage on in-scope crates (`make coverage-merged`; 11,301 / 11,842).
- Certora formal verification: 15 spec modules, 209 rule/helper fns
  (`boundary_rules`, `flash_loan_rules`, `health_rules`, `index_rules`, `interest_rules`,
  `isolation_rules`, `liquidation_rules`, `math_rules`, `oracle_rules`, `position_rules`,
  `solvency_rules`, `strategy_rules`, `emode_rules`, `compat`, `model`).
- cargo-fuzz: 9 targets (`compound_monotonic`, `flow_flash_loan`, `flow_multi_op`,
  `flow_oracle_tolerance`, `flow_supply_borrow_liquidate`, `fp_div_by_int`, `fp_mul_div`,
  `fp_rescale`, `rates_borrow`).
- proptest harnesses: 11 (`fuzz_auth_matrix`, `fuzz_budget_metering`, `fuzz_cache_atomicity`,
  `fuzz_conservation`, `fuzz_isolation_emode_xor`, `fuzz_liquidation_differential`,
  `fuzz_multi_asset_solvency`, `fuzz_oracle_tolerance`, `fuzz_strategy_flashloan`,
  `fuzz_supply_borrow_liquidate`, `fuzz_ttl_keepalive`).
- Miri on `common` (`.github/workflows/fuzz.yml:84-93`).
- CI gates: `cargo test --workspace`, `cargo clippy -D warnings`, OpenZeppelin `soroban-scanner`
  (strict on PR — fails on HIGH/CRITICAL).
- Nightly long run (24h ceiling): `FUZZ_TIME=1800s` per target, `PROPTEST_CASES=50000` per
  harness, artifacts uploaded on failure.
- `cargo audit` 0 vulnerabilities (3 accepted advisories on transitive Soroban deps documented in
  `AUDIT_CHECKLIST.md`).

**Gaps**

- `MATH_REVIEW.md §0` lists 4 Pending items: run `certoraSorobanProver` end-to-end with vendored
  stack, retire empty `summaries/mod.rs`, add `apply_summary!` wrappers at pool/oracle/SAC call
  sites, rewrite 13 tautological rules to call prod.

**Action to maintain Strong**: close the 4 pending `MATH_REVIEW.md §0` items.

---

## Improvement Roadmap

### CRITICAL (ship before audit hand-off)

| # | Action | Category | Effort |
|---|---|---|---|
| C-1 | Land H-03 (`add_protocol_revenue` floor guard parity). | Arithmetic | XS (1 line) |
| C-2 | Land H-05 (`seize_position` borrow-branch `saturating_sub_ray`). | Arithmetic | XS (1 line) |
| C-3 | Resolve H-01 (pool `asset` parameter) and H-02 (ABI parameter order). | Arithmetic / Access | S |
| C-4 | Run `certoraSorobanProver controller/confs/math.conf` end-to-end and record the verdict in `MATH_REVIEW.md`. | Testing & Verification | S |

### HIGH (1-2 months, ship in next minor)

| # | Action | Category | Effort |
|---|---|---|---|
| H-1 | On-chain timelock for `upgrade`, `upgrade_pool`, `edit_asset_config`, `disable_token_oracle`. | Decentralization | L |
| H-2 | Two-step or delay for `set_position_limits` and `configure_market_oracle`. | Decentralization | M |
| H-3 | Close `MATH_REVIEW.md §0` pending items: retire `summaries/mod.rs`, rewrite 13 tautological rules, add `apply_summary!` wrappers. | Testing & Verification | M |
| H-4 | Split `pool/src/lib.rs` (1,737 LOC) into per-mutation modules. | Complexity | M |
| H-5 | Raise inline `///` doc density on `controller/src/lib.rs`, `pool/src/lib.rs`, `controller/src/oracle/mod.rs` — every public fn. | Documentation | M |

### MEDIUM (2-4 months, next major)

| # | Action | Category | Effort |
|---|---|---|---|
| M-1 | `architecture/INCIDENT_RESPONSE.md` with event-to-action runbook for `PoolInsolventEvent`, `CleanBadDebtEvent`, `UpdateDebtCeilingEvent`. | Auditing | S |
| M-2 | `architecture/GLOSSARY.md` for e-mode / isolation / silo / scaled / index / ray vocabulary. | Documentation | S |
| M-3 | Property test: closed `(supply, borrow, withdraw, repay)` sequences return pool to S within N ULPs. (THREAT_MODEL.md §2 audit ask.) | Testing & Verification | M |
| M-4 | Runtime token-wasm code-hash check (tighten M-12 semantics of `revoke_token_wasm`). | Access Controls | L |
| M-5 | KEEPER rate-limit on `clean_bad_debt` to neutralize timing games. | Access Controls | S |
| M-6 | `cargo-complexity` CI step flagging any fn cyclomatic > 25. | Complexity | S |

---

## Notes for external auditors

- Frozen commit `5ee115c` is the intended review basis; `audit-2026-q2` tag is pending.
- Enter via `architecture/ENTRYPOINT_AUTH_MATRIX.md` — every public fn with auth, reentry, and
  file:line invariant citations.
- Known pre-audit findings (H-01..H-07, M-01..M-12, L-07..L-13) are enumerated with status in
  `audit/FINDINGS.md`. "✅ verified" = replicated by internal hunt; "🔧 fix candidate" = accepted
  for fix but unfixed at freeze; "📝 documentation/process" = policy-only mitigation.
- Certora harness ground truth: `controller/certora/SPIKES.md`.
- Off-chain operator policy gaps (M-01 kill-switch, M-11 position-limits, M-12 token-wasm
  revocation) are the primary decentralization concerns and the primary Owner-key-custody risk.
