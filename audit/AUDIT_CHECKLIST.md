# Audit Hand-Off Checklist

## Pre-Hand-Off

### Code freeze
- [x] Frozen commit identified: `5ee115cfa5097670add106c348b189f01bf3d62b` (`5ee115c`)
- [ ] Tag `audit-2026-q2` created and pushed
- [ ] Branch `audit/2026-q2` created from frozen commit

### Build verification
- [x] `make build` succeeds
- [x] `make optimize` succeeds
- [x] `cargo test --workspace` passes (688 passed, 0 failed, 3 ignored)
- [x] `make coverage-merged` — 95.43% line coverage (11,301/11,842); per-file detail in `AUDIT_PREP.md`
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean (3 trivial issues fixed during prep)
- [x] `cargo audit` — 0 vulnerabilities; 3 accepted advisories on transitive Soroban deps: `derivative` unmaintained (RUSTSEC-2024-0388), `paste` unmaintained (RUSTSEC-2024-0436), `rand` unsound-with-custom-logger (RUSTSEC-2026-0097). All inherited via `soroban-sdk`.
- [ ] Optional: `cargo +nightly udeps --workspace` for dead-dep detection

### Configuration self-defense gaps — resolved during prep
Per `architecture/CONFIG_INVARIANTS.md` summary table:
- [x] Gap #1: ✅ already enforced (`router::validate_market_creation`)
- [x] Gap #2/#8: 🔧 fixed (LT ≤ 10_000 added to `validate_asset_config` and e-mode validation)
- [x] Gap #3: 🔧 fixed (`isolation_debt_ceiling_usd_wad >= 0` added)
- [x] Gap #4: 🔧 fixed (`flashloan_fee_bps >= 0` added with `NegativeFlashLoanFee` error)
- [x] Gap #5: ✅ already enforced (`max_price_stale_seconds ∈ [60, 86_400]`)
- [x] Gap #6: ✅ already enforced (cex_symbol probed via `lastprice` at config time)
- [x] Gap #7: 📝 intentional (`twap_records == 0` is intentional spot-fallback path)
- [x] Gap #9: 📝 leave as runtime check (e_mode_enabled is preserved by `edit_asset_config`; only `add_asset_to_e_mode_category` flips it)

## Documentation Package

### Existing docs (in repo root)
- [x] `README.md` — system overview
- [x] `architecture/ARCHITECTURE.md` — sequence diagrams + storage model
- [x] `architecture/INVARIANTS.md` — 18 protocol invariants with examples
- [x] `architecture/DEPLOYMENT.md` — operator runbook
- [x] `architecture/MATH_REVIEW.md` — rule-coverage audit
- [x] `architecture/ACTORS.md` — privilege model, trust boundaries
- [x] `architecture/ENTRYPOINT_AUTH_MATRIX.md` — fn × auth × invariants × pool calls
- [x] `architecture/CONFIG_INVARIANTS.md` — config field rules + gap analysis
- [x] `architecture/STELLAR_NOTES.md` — Soroban-specific assumptions and confirmation asks
- [x] `controller/certora/HANDOFF.md` — Certora toolchain ground truth

### Audit prep package (in `audit/`)
- [x] `audit/SCOPE.md` — frozen commit, file list with LOC, in/out scope
- [x] `audit/AUDIT_PREP.md` — review goals, concerns, worst-case, questions for auditors
- [x] `audit/THREAT_MODEL.md` — adversary models with risk heat-map
- [x] `audit/CODE_MATURITY_ASSESSMENT.md` — Trail-of-Bits 9-category maturity scorecard
- [x] `audit/AUDIT_CHECKLIST.md` — this document

Historical pre-remediation findings and adversarial-loop notes were removed from
this directory; every item is either shipped in code (regression-gated in
`test-harness/tests/fuzz_*.rs`) or tracked in `architecture/MATH_REVIEW.md`.

## Auditor Selection Status

- **Runtime Verification**: targets full implementation review (semantic + impl).
- **Certora**: targets the formal verification track in parallel.
- See `controller/certora/HANDOFF.md` for Certora toolchain status — **resolve the spike-A blocker (cvlr build) on a fresh clone before Certora hand-off**.

## Off-chain Operator Setup

Auditors who want to deploy locally need:
- `stellar` CLI, `jq`, Rust toolchain per `rust-toolchain.toml`
- a funded testnet identity (or `SIGNER=ledger`)
- `make setup-testnet` — deploys controller + pools, configures markets and e-modes
- smoke test per `architecture/DEPLOYMENT.md §Smoke-Test Runbook`

## Pre-audit remediation status

All pre-audit hunt findings (H-01–H-08, M-01–M-12, L-01–L-13, I-01–I-03) and all
adversarial-loop findings (N-01–N-13) that warranted code changes have shipped
and are verified present in current prod code. Regression gates exist in
`test-harness/tests/fuzz_auth_matrix.rs`, `fuzz_strategy_flashloan.rs`,
`fuzz_ttl_keepalive.rs`, and `fuzz_conservation.rs` for the specific classes
(C-01, M-03, M-08, H-03, H-04, L-05, M-09, M-10, M-11, M-14, N-02, NEW-01).

Operator-policy items (FoT / rebasing token bans, SAC issuer upgrade runbook,
`cap = 0` semantics, allowlist creation-time scope) are documented in
`architecture/DEPLOYMENT.md`, `architecture/ACTORS.md`, and
`architecture/CONFIG_INVARIANTS.md`.

## Still Outstanding

- ⚠️ **Production-tx budget benchmark for `liquidate`** at the contract cap `PositionLimits = 32/32`. The harness-only bench in `test-harness/tests/bench_liquidate_max_positions.rs` validates that Soroban's cost model surfaces budget exhaustion cleanly at 5/5 × 5 markets (no opaque panic) — see `audit/THREAT_MODEL.md §3.3 Empirical bench`. The remaining gate is testnet measurement under real signed-tx auth (mock-auth in the harness inflates the auth-tree budget). Operator-policy: keep `PositionLimits = 10/10` until measured.
- ⚠️ **Reflector behavior spec** (see `architecture/STELLAR_NOTES.md §3 Q6–Q10`). External team contact.

## Resolved during 2026-Q2 audit prep

- ✅ **`max_borrow_rate_ray` upper cap**. `validate_interest_rate_model`
  (`controller/src/validation.rs:90-118`) and `pool::update_params`
  (`pool/src/lib.rs:597-606`) now reject `max_borrow_rate_ray > 2 * RAY`.
  Constant `MAX_BORROW_RATE_RAY = 2 * RAY` lives in
  `common/src/constants.rs`. Regression tests:
  `test_upgrade_pool_params_rejects_max_borrow_rate_above_cap` and
  `test_upgrade_pool_params_accepts_max_borrow_rate_at_cap` in
  `test-harness/tests/admin_config_tests.rs`.

## Final Hand-Off

After resolving the items above:
- [ ] Push tag `audit-2026-q2`
- [ ] Send Runtime Verification: repo URL + tag + `audit/` directory link
- [ ] Send Certora: the same, plus a Certora-specific note pointing at `controller/certora/HANDOFF.md` and the resolved cvlr build
- [ ] Schedule kickoff calls
- [ ] Open a Slack/Discord channel for async Q&A during the engagement

## During Audit

- Create a fresh `audit/ENGAGEMENT_FINDINGS.md` at engagement start; log findings as they arrive (severity, status, fix PR). Do not re-introduce pre-remediation history here.
- Daily standup async; weekly sync call.
- Out-of-scope changes to the frozen branch require auditor sign-off.
