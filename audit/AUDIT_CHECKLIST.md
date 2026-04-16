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
Per `audit/CONFIG_INVARIANTS.md` summary table:
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
- [x] `ARCHITECTURE.md` — sequence diagrams + storage model
- [x] `INVARIANTS.md` — 18 protocol invariants with examples
- [x] `DEPLOYMENT.md` — operator runbook
- [x] `MATH_REVIEW.md` — rule-coverage audit
- [x] `controller/certora/SPIKES.md` — Certora toolchain ground truth

### Audit prep package (in `audit/`)
- [x] `audit/SCOPE.md` — frozen commit, file list with LOC, in/out scope
- [x] `audit/AUDIT_PREP.md` — review goals, concerns, worst-case, questions for auditors
- [x] `audit/ACTORS.md` — privilege model, trust boundaries
- [x] `audit/ENTRYPOINT_AUTH_MATRIX.md` — fn × auth × invariants × pool calls
- [x] `audit/CONFIG_INVARIANTS.md` — config field rules + gap analysis
- [x] `audit/THREAT_MODEL.md` — adversary models with risk heat-map
- [x] `audit/STELLAR_NOTES.md` — Soroban-specific assumptions and confirmation asks
- [x] `audit/AUDIT_CHECKLIST.md` — this document

## Auditor Selection Status

- **Runtime Verification**: targets full implementation review (semantic + impl).
- **Certora**: targets the formal verification track in parallel.
- See `controller/certora/SPIKES.md` for Certora toolchain status — **resolve the spike-A blocker (cvlr build) on a fresh clone before Certora hand-off**.

## Off-chain Operator Setup

Auditors who want to deploy locally need:
- `stellar` CLI, `jq`, Rust toolchain per `rust-toolchain.toml`
- a funded testnet identity (or `SIGNER=ledger`)
- `make setup-testnet` — deploys controller + pools, configures markets and e-modes
- smoke test per `DEPLOYMENT.md §Smoke-Test Runbook`

## Hunt findings shipped (see `audit/FINDINGS.md`)

### Group A — one-liners ✅
- H-03 `add_protocol_revenue` floor guard (`pool/src/interest.rs`)
- H-05 `seize_position` borrow branch `saturating_sub_ray` (`pool/src/lib.rs:439`)
- L-09 `BAD_DEBT_USD_THRESHOLD` extracted to `common/src/constants.rs`
- L-10 `seize_position` defensive `else` arm

### Group B — small fixes ✅
- H-04 `flashloan_fee_bps` bounds in `validate_asset_config` (single source of truth); constant moved to `common`
- M-05 `mul_div_floor` + `Wad::div_floor`; liquidation seizure now floors `base_amount` so protocol fee ≥ spec
- M-07 TWAP staleness uses oldest sample, not newest
- M-08 `validate_bulk_position_limits` panics on unknown position type
- M-10 excess-payment record recomputes `new_usd = new_amount * price`
- L-07 strategy `.expect(...)` → `panic_with_error!(GenericError::InternalError)`
- L-13 liquidation events re-derive `EventAccountAttributes` from post-mutation account

### Group C — ABI/structural ✅
- H-01 pool flash-loan drops `asset` arg; uses `cache.params.asset_id`
- H-02 `pool.repay` parameter order aligned with `borrow` (`(caller, amount, position, price)`)
- M-04 `claim_revenue` partial-claim single `actual_burn = min(scaled_to_burn, revenue, supplied)`
- M-06 `liquidation_threshold_bps` refreshed on supply top-up
- M-09 `dex_symbol` required field added; forced re-config migration documented in `DEPLOYMENT.md`

### Group D — operator policy ✅ (docs only)
- H-06/H-07 NO FoT / NO rebasing tokens (DEPLOYMENT, SCOPE)
- H-08 SAC issuer upgrade runbook (DEPLOYMENT, STELLAR_NOTES)
- M-12 allowlist is creation-time only (ACTORS)
- L-04 `cap = 0` means unlimited (CONFIG_INVARIANTS)
- L-12 INVARIANTS.md §4 seize-Deposit path documented
- I-03 OZ Stellar review note (SCOPE)

### Group E — centralization ✅
- M-01 document `disable_token_oracle` as single-call kill-switch (ACTORS)
- M-02 constructor grants ONLY `KEEPER`; REVENUE/ORACLE require explicit grant
- M-03 constructor pauses; operator must `unpause` post-wiring
- M-11 document `set_position_limits` immediate-effect (ACTORS)
- L-05 pool `claim_revenue` drops `caller`; pool stores accumulator at construct (ABI change)

### Group F — build/config hygiene ✅
- I-01 CVLR git revs pinned in workspace `Cargo.toml`
- I-02 `soroban-sdk` + OZ Stellar caret pins tightened to `=`

## Still Outstanding

- ⚠️ **Empirical 32+32 liquidate cost benchmark** (see `audit/THREAT_MODEL.md §3.3`). Needs a custom test-harness scenario. Not blocking pre-audit hand-off but valuable for the auditor's §3 threat-model review.
- ⚠️ **Reflector behavior spec** (see `audit/STELLAR_NOTES.md §3 Q6–Q10`). External team contact.

## Final Hand-Off

After resolving the items above:
- [ ] Push tag `audit-2026-q2`
- [ ] Send Runtime Verification: repo URL + tag + `audit/` directory link
- [ ] Send Certora: the same, plus a Certora-specific note pointing at `controller/certora/SPIKES.md` and the resolved cvlr build
- [ ] Schedule kickoff calls
- [ ] Open a Slack/Discord channel for async Q&A during the engagement

## During Audit

- Log findings in `audit/FINDINGS.md` as they arrive (severity, status, fix PR).
- Daily standup async; weekly sync call.
- Out-of-scope changes to the frozen branch require auditor sign-off.
