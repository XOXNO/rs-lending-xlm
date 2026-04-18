# Audit Preparation Package

**Project**: Stellar Lending Protocol (`rs-lending-xlm`)
**Auditors**: Runtime Verification + Certora (formal verification track in parallel)
**Frozen commit**: `5ee115c` (see [SCOPE.md](./SCOPE.md))
**Prep date**: 2026-04-16

## Security Objectives

1. **No user fund theft** — under any sequence of permitted operations, no actor can withdraw collateral or principal belonging to someone else.
2. **Solvency under adversarial math** — no attacker can drive fixed-point rounding, index updates, or rate-model evaluation into a state where `Σ debt > Σ collateral × LT` while the protocol still believes itself solvent.
3. **Flash loans are atomic and self-balancing** — `flash_loan_begin` and `flash_loan_end` always execute as a pair; reserve invariants hold before the user transaction returns; no nested flash loan or recursive entry escapes the boolean guard.
4. **Liquidations preserve progress** — every successful liquidation strictly reduces or maintains the debt ratio (`new_hf >= old_hf` in fallback) and never seizes more collateral than `bonus + fee` warrants.
5. **Configuration cannot brick or unsafely relax the protocol** — on-chain invariants reject operator misconfiguration where possible; the system *defends itself* against operator error.

## Areas Of Concern (Author-Identified)

### High-priority unfamiliar territory (Stellar/Soroban-specific)

- **Flash loan re-entrancy semantics on Soroban.** **Every** mutating controller endpoint — `supply`/`borrow`/`repay`/`withdraw` included — checks the single-bool `FlashLoanOngoing` (Instance storage). The receiver callback can only reach external contracts (aggregator, tokens). Auditors should confirm Soroban's panic-rollback semantics clear the flag under sub-call failures. See `THREAT_MODEL.md §1` (revised).
- **Stellar protocol limits** constrain bulk batches: 400M instructions per tx, 200 disk-read entries, 200 write-ledger entries, 132 KB tx size, 16 KB events. Bulk endpoints `supply`, `borrow`, `withdraw`, `repay` accept `Vec<(Address, i128)>`; `liquidate` accepts `Vec<(Address, i128)>` debt payments and seizes *all* of an account's collateral assets. **Worst-case footprint per asset count remains undocumented.** See `THREAT_MODEL.md §3`.
- **Token transfer semantics on Soroban.** SAC `transfer` checks balance but returns no bool — failure panics the entire host call. Confirm: does any code path infer transfer success from "no panic"?
- **Reflector oracle behavior.** Differs from MultiversX price aggregator: TWAP record availability, decimal handling per asset kind (`Stellar` vs `Other`), staleness vs not-yet-published. See `architecture/STELLAR_NOTES.md §3`.
- **Inner contract auth propagation.** When controller calls pool and pool transfers tokens, confirm the auth tree the host enforces matches our model (`pool/src/lib.rs:1596`).

### Auth and missing-check surface

- Confirm every public fn carries `#[only_owner]`, `#[only_role(...)]`, `caller.require_auth()`, or is a pure view. See `architecture/ENTRYPOINT_AUTH_MATRIX.md`.
- Confirm `verify_admin` covers every pool mutator and that construction sets the admin to exactly the controller. See `architecture/STELLAR_NOTES.md §4`.
- Confirm role-gated functions that take a `caller: Address` param actually use it (some are `let _ = caller;` shims — watch for log/audit gaps).

### Liquidation complexity

- Three-tier target HF cascade (`1.02` → `1.01` → fallback `d_max = total_coll / (1+base_bonus)`) guarded against `new_hf < old_hf` regression.
- Bad-debt socialization with a $5 per-account threshold and supply-index floor `10^18` raw (see `architecture/INVARIANTS.md §7`).
- Per-asset seizure splits into `base + bonus + protocol_fee_on_bonus` — confirm the rounding direction across the three slices preserves `Σ slices ≤ collateral_seized`.
- **Dust-erasure asymmetry on isolated debt** (decrement-only) — known issue per `architecture/INVARIANTS.md §11` and `architecture/MATH_REVIEW.md §5.1`.

### Bulk / batch threat models

- Same-asset deduplication in batches (`validate_bulk_position_limits` dedupes via `Map`, but does *amount* aggregation behave consistently? See `THREAT_MODEL.md §3`).
- Multi-account bulk liquidation **has no endpoint** — `liquidate` runs per-account. Confirm this reflects design (per-account atomicity), not a planned future feature.
- `update_indexes`, `claim_revenue`, `add_rewards`, `keepalive_*` accept `Vec<Address>` / `Vec<u64>` — bound the worst-case footprint.

### Misconfiguration self-defense

- `validate_interest_rate_model` enforces monotone slopes, util ordering, RF range. **Confirm completeness** — see `architecture/CONFIG_INVARIANTS.md` for gap analysis.
- `validate_asset_config` checks LT > LTV, bonus ≤ MAX_LIQUIDATION_BONUS, fees ≤ 100%, non-negative caps, but skips `isolation_debt_ceiling_usd_wad >= 0` and `flashloan_fee_bps >= 0`. See `architecture/CONFIG_INVARIANTS.md §3`.
- Position limits clamp to `[1, 32]` (`config::set_position_limits`). Cross-check against worst-case liquidation gas.

## Worst-Case Scenarios

| Scenario | Loss | Likelihood vector |
|---|---|---|
| Flash-loan-driven oracle manipulation triggers undeserved liquidation | Per-account collateral seized at attacker price | Reflector tolerance bypass; same-tx oracle staleness |
| Math attack inflates scaled supply or shrinks scaled debt | Pool insolvency | Rounding direction asymmetry; index update ordering |
| Bad-debt socialization underflows supply index past floor | Permanent supplier loss | Liquidation cascade fallback math + bad-debt size |
| Re-entry via flash receiver completes a borrow that bypasses HF check | Drained pool | Missing `require_not_flash_loaning` in user paths |
| Misconfigured market (operator) accepted by controller | Liquidatable-on-create accounts; exceedable caps | Gap in `validate_asset_config` |
| Reflector oracle returns stale or wrong-decimal price unhandled | Mispriced liquidation / borrow | `max_price_stale_seconds` mishandling, decimal confusion |

## Questions For Auditors

### For Runtime Verification (semantic / implementation)

1. Does the absent `require_not_flash_loaning` in `supply`/`borrow`/`repay`/`withdraw` open an exploitable hole, or does in-flash-loan deposit/borrow belong to the multi-step strategy contract by design?
2. Does the per-asset seizure split (`base + bonus + protocol_fee_on_bonus`) preserve `Σ slices ≤ collateral_seized` under all rounding paths?
3. Does the `KEEPER`-callable `clean_bad_debt_standalone` path match the in-liquidation socialization path exactly, or does it open a separate adversary-controlled path to mutate `supply_index`?
4. What is the worst-case storage footprint for `liquidate` when an account holds the maximum 32 supply + 32 borrow positions across distinct pools? Does it exceed the 200 r/w entry tx limit?
5. Can any token-balance reconciliation path (for example, an attacker's direct token transfer to a pool address) desync `reserves` from accounting?

### For Certora (formal / spec)

6. Confirm the rule-coverage gaps `architecture/MATH_REVIEW.md` documents, and produce a verdict for each.
7. Spec-level: does `apply_bad_debt_to_supply_index` always preserve `revenue_ray ≤ supplied_ray` after the floor clamp?
8. Spec-level: prove `accrued_interest = supplier_rewards + protocol_fee` under the actual half-up rounding implementation (not idealized math).
9. Soroban toolchain blocker (CVLR build) — does the vendored fix at `vendor/cvlr/` suffice for end-to-end runs? See `controller/certora/SPIKES.md`.

### For both

10. Threat model for multi-account liquidator gas griefing — should we add batch liquidation across accounts as a primitive, and what attack surface follows?
11. Token allowlist (`approve_token_wasm`/`revoke_token_wasm`) — confirm address-keyed approval suffices given Soroban's lack of runtime Wasm-hash lookup.

## Static Analysis Status

- `cargo clippy --workspace --lib --bins -- -D warnings`: **CLEAN on production code** (controller, pool, pool-interface, common). Prep fixed 3 trivial style errors in production (`common/src/events.rs` ×2, `common/src/rates.rs` ×1, `pool/src/lib.rs` ×3 redundant-field/range-contains). 8 pre-existing lints in `test-harness/` and pool unit tests (digits-grouped, redundant-field, doc-list-overindent, loop-variable, useless-conversion, min-or-max) never reach deployed bytecode; we track them for cleanup but they do not block the audit.
- `cargo audit`: **CLEAN** (0 vulnerabilities). We accept 3 advisories on transitive deps inherited via `soroban-sdk`: `derivative` (RUSTSEC-2024-0388, unmaintained), `paste` (RUSTSEC-2024-0436, unmaintained), `rand` (RUSTSEC-2026-0097, unsound with custom logger — our paths never exercise it).
- `cargo +nightly udeps`: not run (optional).
- `make coverage-merged`: **95.43% line coverage** (11,301/11,842 in-scope lines, post-hardening). Production files ≥ 90%, most ≥ 99%; flash_loan, utils, withdraw, account, cache, pool/views, pool/interest at **100%**. Lowest: `controller/src/oracle/reflector.rs` (0/2, only real network calls exercise it) and `controller/src/helpers/testutils.rs` (39%, test infrastructure). Every position/strategy/liquidation path ≥ 99.7%. Report at `target/coverage/merged-report.md` and `merged.lcov.info`.

## Verified Facts vs Inferred (Verification Pass Result)

6 parallel verification agents read each handler end-to-end and produced file:line citations, then we updated the audit docs. Notable findings:

**Corrections applied to docs**:
- `borrow` does NOT recompute HF after the batch — pre-borrow it checks only LTV. Removed the earlier ENTRYPOINT_AUTH_MATRIX claim; added an auditor ask about e-mode threshold overrides under that constraint.
- `withdraw` HAS an `amount == 0` → `i128::MAX` sentinel (withdraw.rs:84). **DOC DRIFT FIXED**: updated `architecture/ARCHITECTURE.md:246` and the `architecture/MATH_REVIEW.md` drift table — both controller and pool sentinels now appear.
- Pool's `flash_loan_end` calls plain `tok.transfer(receiver→pool, ...)` (pool/lib.rs:353), which requires Soroban-native `from.require_auth()` — NOT ERC-20 `transfer_from`/`approve`. The receiver must call `env.authorize_as_current_contract` in its callback. Corrected architecture/ACTORS.md.

**Code hardening shipped during prep**:
- `strategy.rs::swap_tokens` brackets the aggregator router call with `set_flash_loan_ongoing(true/false)` to block aggregator-callback re-entry into mutating controller endpoints.
- `strategy.rs::swap_tokens` enforces `received >= steps.amount_out_min` controller-side after the swap (panic `InternalError`) — defense-in-depth against a router that ignores its own slippage parameter.

**Confirmed correct**:
- All auth gates (file:line in each handler).
- Bad-debt threshold `5 * WAD` strict `>` (liquidation.rs:429-430).
- Three-tier HF cascade values 1.02 → 1.01 → fallback (helpers/mod.rs:231, 261, 284).
- Supply-index floor `10^18 raw` (pool/interest.rs:14).
- `clean_bad_debt_standalone` shares the `execute_bad_debt_cleanup` math path with in-liquidation (liquidation.rs:463).
- All tolerance band logic and Reflector decimals discovery (oracle/mod.rs, config.rs).
- `repay` stays permissionless (no account-owner check) by design; the refund targets the actual repayer (pool/lib.rs:267).

## Documentation Inventory

| Doc | Purpose | Status |
|---|---|---|
| `README.md` | Enterprise overview | up-to-date |
| `architecture/ARCHITECTURE.md` | Component boundaries, sequence diagrams, storage model | up-to-date |
| `architecture/INVARIANTS.md` | 18 protocol invariants with worked examples | up-to-date |
| `architecture/DEPLOYMENT.md` | Build/deploy/configure runbook | up-to-date |
| `architecture/MATH_REVIEW.md` | Rule-coverage audit, drift between docs and code | active |
| `architecture/ACTORS.md` | Actor and privilege model | up-to-date |
| `architecture/ENTRYPOINT_AUTH_MATRIX.md` | Per-fn auth × invariant × pool-call matrix | up-to-date |
| `architecture/CONFIG_INVARIANTS.md` | All config fields × valid range × enforcement site | up-to-date |
| `architecture/STELLAR_NOTES.md` | Soroban-specific assumptions and uncertainties | up-to-date |
| `controller/certora/SPIKES.md` | Toolchain ground truth for Certora | active |
| `audit/SCOPE.md` | This audit's frozen scope and file list | this prep |
| `audit/THREAT_MODEL.md` | Adversary models for each concern area | this prep |
| `audit/AUDIT_CHECKLIST.md` | Hand-off checklist | this prep |
| `audit/CODE_MATURITY_ASSESSMENT.md` | Trail-of-Bits 9-category scorecard | this prep |
