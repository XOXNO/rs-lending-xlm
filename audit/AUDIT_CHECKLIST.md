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
- [x] `controller/certora/SPIKES.md` — Certora toolchain ground truth

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
- See `controller/certora/SPIKES.md` for Certora toolchain status — **resolve the spike-A blocker (cvlr build) on a fresh clone before Certora hand-off**.

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

- ⚠️ **Empirical max-position liquidate cost benchmark** (see `audit/THREAT_MODEL.md §3.3`). Needs a custom test-harness scenario under the default `PositionLimits = 10/10`. Worst-case budget measurement is blocking-if-limits-raised.
- ⚠️ **Reflector behavior spec** (see `architecture/STELLAR_NOTES.md §3 Q6–Q10`). External team contact.
- ⚠️ **`max_borrow_rate_ray` upper cap** (MATH_REVIEW.md latent-concern). `validate_interest_rate_model` and `pool.update_params` reject `max_borrow_rate_ray < slope3` but not a high upper bound; 8-term Taylor in `compound_interest` has documented accuracy only for per-chunk `x ≤ 2 RAY`. Either cap `max_borrow_rate_ray ≤ 2 * RAY` in validation OR make `MAX_COMPOUND_DELTA_MS` adaptive.

## Final Hand-Off

After resolving the items above:
- [ ] Push tag `audit-2026-q2`
- [ ] Send Runtime Verification: repo URL + tag + `audit/` directory link
- [ ] Send Certora: the same, plus a Certora-specific note pointing at `controller/certora/SPIKES.md` and the resolved cvlr build
- [ ] Schedule kickoff calls
- [ ] Open a Slack/Discord channel for async Q&A during the engagement

## During Audit

- Create a fresh `audit/ENGAGEMENT_FINDINGS.md` at engagement start; log findings as they arrive (severity, status, fix PR). Do not re-introduce pre-remediation history here.
- Daily standup async; weekly sync call.
- Out-of-scope changes to the frozen branch require auditor sign-off.
