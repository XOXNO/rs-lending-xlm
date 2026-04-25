# Remediation Plan

Canonical summary of internal-finding remediation, derived from `git log` so
the SDF audit-bank submission has a single source of truth. Replaces the
removed `audit/FINDINGS.md` / `audit/HANDOFF_SUMMARY.md` / `audit/new-findings.md`
files (per `cb14a72 docs(audit): drop pre-remediation finding logs, refresh
live prep docs`) without re-introducing the historical detail noise the
external audit team's own engagement log will absorb.

For the underlying invariant proofs, see
[`architecture/INVARIANTS.md`](../architecture/INVARIANTS.md). For static-tool
verdicts, see [`audit/TOOLING_SCAN.md`](./TOOLING_SCAN.md). For the
threat-modelling input that drove these findings, see
[`audit/STRIDE.md`](./STRIDE.md) and
[`audit/THREAT_MODEL.md`](./THREAT_MODEL.md).

## How to read this file

Each finding row contains:

- **ID** — the historical tag (H-/M-/L-/N-/I-/C-/NEW- series).
- **Severity** — `C` Critical, `H` High, `M` Medium, `L` Low, `I`
  Informational, `N` Adversarial-loop note.
- **Class** — Spoofing / Tampering / Repudiation / Information Disclosure /
  DoS / Elevation of Privilege / Misconfig / Doc / Build, mapped against
  STRIDE.
- **Root cause** — one-line summary of the underlying issue.
- **Fix** — file:line of the production code change.
- **Fix commit** — short SHA of the merging commit.
- **Regression gate** — file:line of the regression test (when applicable).
- **Status** — `Shipped` (in code), `Documented` (operator policy),
  `Outstanding` (still open).

Frozen-commit reference for the audit: tag `audit-2026-q2` on the
`audit/2026-q2` branch (cut from current `main` HEAD per
[`audit/AUDIT_CHECKLIST.md`](./AUDIT_CHECKLIST.md)).

## 1. Critical / High

| ID | Sev | Class | Root cause | Fix | Fix commit | Regression gate | Status |
|---|---|---|---|---|---|---|---|
| C-01 | C | Elevation | `edit_e_mode_category` was missing `#[only_owner]` — any address could mutate the e-mode risk schedule. | `controller/src/lib.rs:451-627` (single-line auth gate) | d59afe1 | `test-harness/tests/fuzz_auth_matrix.rs:189-198` (`expect_rejected`) | Shipped |
| H-01 | H | Tampering | `pool.flash_loan_begin/end` accepted an `asset` arg that could disagree with the pool's own `cache.params.asset_id`, allowing cross-asset mismatch. | Pool drops the `asset` arg; uses `cache.params.asset_id` directly. `pool/src/lib.rs:317-353` | d59afe1 | `test-harness/tests/flash_loan_tests.rs` + `fuzz_strategy_flashloan.rs` | Shipped |
| H-02 | H | Tampering | `pool.repay` parameter order differed from `borrow`, encouraging caller-side argument swaps and silent value misrouting. | Aligned signature: `repay(caller, amount, position, price_wad)`. `pool/src/lib.rs:237-268` | d59afe1 | `test-harness/tests/repay_tests.rs` | Shipped |
| H-03 | H | Tampering | `add_protocol_revenue` had a floor guard on the Ray path (`pool/src/interest.rs:63-75`) but **not** the asset-decimal path — under post-bad-debt conditions, the asset-path could drive the supply index below the floor. | Floor guard mirrored at `pool/src/interest.rs:299` (asset-decimal variant). | d59afe1 | `test-harness/tests/bad_debt_index_tests.rs` | Shipped |
| H-04 | H | Misconfig | `flashloan_fee_bps` was bounded only at the pool level — controller-side `validate_asset_config` accepted out-of-band values. | `validate_asset_config` enforces `flashloan_fee_bps ∈ [0, MAX_FLASHLOAN_FEE_BPS=500]` (`controller/src/validation.rs:150-155`); negative emits `NegativeFlashLoanFee`. | d59afe1 | `test-harness/tests/admin_config_tests.rs` | Shipped |
| H-05 | H | Tampering | `seize_position` borrow branch used a non-saturating subtraction; in pathological splits this could underflow. | Switched to `saturating_sub_ray` on the borrow branch (`pool/src/lib.rs::seize_position`). | d59afe1 | `test-harness/tests/liquidation_math_tests.rs`, `fuzz_liquidation_differential.rs` | Shipped |
| H-06 | H | Tampering / DoS | Fee-on-transfer SAC tokens silently break the protocol's accounting (the pool reads pre-fee and post-fee balances). | Operator-policy: `approve_token_wasm` MUST exclude FoT tokens. Documented in `architecture/DEPLOYMENT.md "Token allowlist policy"` and `audit/SCOPE.md "Operator policy preconditions"`. | d59afe1 | n/a (operator policy) | Documented |
| H-07 | H | Tampering / DoS | Rebasing tokens (balance-rebase outside transfers) similarly break scaled-supply accounting. | Operator-policy: same allowlist exclusion. | d59afe1 | n/a (operator policy) | Documented |
| H-08 | H | Tampering | A SAC issuer upgrade can change token semantics post-listing. | Operator runbook: `architecture/DEPLOYMENT.md` and `architecture/STELLAR_NOTES.md §Tokens` describe the response (pause, migrate to a new market). | d59afe1 | n/a (operator policy) | Documented |

## 2. Medium

| ID | Sev | Class | Root cause | Fix | Fix commit | Regression gate | Status |
|---|---|---|---|---|---|---|---|
| M-01 | M | Decentralization | `edit_asset_config` takes effect immediately with no two-step or timelock. | Documented in `architecture/ACTORS.md §Owner` as a known operator-policy gap; on-chain remediation tracked as Maturity H-1. | d59afe1 | n/a | Documented |
| M-02 | M | Elevation | The constructor previously granted KEEPER + REVENUE + ORACLE roles by default, widening the initial attack surface. | Constructor grants ONLY KEEPER (`controller/src/lib.rs:862`); REVENUE and ORACLE require explicit `grant_role` post-deploy. | d59afe1 | `controller/src/lib.rs:1276, 1314, 1426` (test-mode hardening); `fuzz_auth_matrix.rs` | Shipped |
| M-03 | M | DoS / Misconfig | The constructor left the contract unpaused, so a partially-wired controller (no markets, no oracle) exposed user endpoints. | Constructor pauses the contract; operator must explicitly `unpause` post-wiring (`controller/src/lib.rs:879`; tests at `:1350, :1394`). | d59afe1 | `controller/src/positions/repay.rs:387` (test-mode unpause), `fuzz_auth_matrix.rs` paused-state checks | Shipped |
| M-04 | M | Tampering | `claim_revenue` partial-claim path computed two separate `actual_burn` values, with `revenue_burn > supplied_burn` reachable on dimensional drift. | Single `actual_burn` so `revenue_ray ≤ supplied_ray` always holds (`pool/src/lib.rs:478-496`). | d59afe1 | `test-harness/tests/revenue_tests.rs` | Shipped |
| M-05 | M | Tampering | `mul_div_half_up` rounded the liquidation-base seizure up, occasionally overshooting collateral. | New `mul_div_floor` + `Wad::div_floor`; liquidation seizure base uses the floored variant (`common/src/fp_core.rs`, `controller/src/positions/liquidation.rs`). | d59afe1 | `fuzz_liquidation_differential.rs` (BigRational diff) | Shipped |
| M-06 | M | Tampering | Re-supplying into an existing position kept the prior `liquidation_threshold_bps` snapshot, ignoring an asset-config refresh in between. | `liquidation_threshold_bps` refreshed on every supply top-up (`controller/src/positions/supply.rs`). | d59afe1 | `test-harness/tests/supply_tests.rs` | Shipped |
| M-07 | M | Tampering | TWAP staleness was checked against the **newest** sample's timestamp, leaving a narrow window where the oldest sample violated `max_price_stale_seconds`. | Now uses the oldest sample (`controller/src/oracle/mod.rs:252`). | d59afe1 | `test-harness/tests/oracle_tolerance_tests.rs` | Shipped |
| M-08 | M | Misconfig | `validate_bulk_position_limits` previously silently no-op'd for unknown `position_type`. | Now panics with `GenericError::UnknownPositionType` (`controller/src/validation.rs:291`). | d59afe1 | `fuzz_auth_matrix.rs`, `fuzz_multi_asset_solvency.rs` | Shipped |
| M-09 | M | Misconfig | `dex_symbol` was missing from `OracleProviderConfig`, blocking a rich `DualOracle` configuration. | Field added; storage-layout migration required (`common/src/types.rs`). | d59afe1 | `architecture/CONFIG_INVARIANTS.md` enumerates field-by-field validation | Shipped |
| M-10 | M | Tampering | Strategy `swap_*` accepted `amount_out_min <= 0`, allowing operator misconfig to bypass slippage. | Strategy entry points reject `amount_out_min <= 0` with `AmountMustBePositive` (`controller/src/strategy.rs`). | d59afe1 | `fuzz_strategy_flashloan.rs:319-325` (`assert!` panic message), `strategy_edge_tests.rs:1293` | Shipped |
| M-11 | M | Tampering | `swap_collateral` previously used the requested withdrawal amount instead of the actual withdrawn delta, mispricing the swap by the dust-lock residual. | Reads actual delta from pool return value. | d59afe1 | `fuzz_strategy_flashloan.rs:328-334` | Shipped |
| M-12 | M | Elevation / DoS | `approve_token_wasm` is creation-time only; `revoke_token_wasm` doesn't stop existing pools at runtime. | Documented in `architecture/ACTORS.md`; runtime token-WASM hash check tracked as Maturity M-4. | d59afe1 | n/a | Documented |
| M-14 | M | DoS | Full withdraw could leave an orphan `SupplyPosition` key in persistent storage if the asset-list shrink ran in the wrong order. | Pool-side fix at the `seize_position` and full-withdraw paths; controller-side `account.supply_assets` updated atomically. | d59afe1 | `fuzz_ttl_keepalive.rs:228-292` (`prop_account_orphan_positions_not_stuck`) | Shipped |

## 3. Low

| ID | Sev | Class | Root cause | Fix | Fix commit | Regression gate | Status |
|---|---|---|---|---|---|---|---|
| L-04 | L | Doc | `cap = 0` semantics ambiguous (off vs unlimited). | `cap = 0` documented as "unlimited" in `architecture/CONFIG_INVARIANTS.md` and tests (`admin_config_tests.rs`). | d59afe1 | n/a | Documented |
| L-05 | L | Tampering | `pool.claim_revenue` previously took a `caller` arg; an attacker could spoof a caller value across a sub-call. | Pool drops the `caller` arg; reads `Accumulator` from its own instance storage written at construct (`pool/src/lib.rs:760`). | d59afe1 | `test-harness/tests/revenue_tests.rs` | Shipped |
| L-07 | L | Tampering | Strategy code used `expect()` for invariant violations, panicking with a generic message. | Replaced with `panic_with_error!(InternalError)` so off-chain indexers see a stable error code. | d59afe1 | `strategy_panic_coverage_tests.rs` | Shipped |
| L-09 | L | Doc | `BAD_DEBT_USD_THRESHOLD = 5 * WAD` was an inline magic value. | Constant moved to `common/src/constants.rs`; consumed at `controller/src/positions/liquidation.rs:413`. | d59afe1 | n/a | Shipped |
| L-10 | L | Tampering | `seize_position` lacked a defensive `else` branch on the rounding asymmetry path. | Defensive `else` arm added; emits invariant-failure on impossible state. | d59afe1 | `liquidation_math_tests.rs` | Shipped |
| L-12 | L | Doc | `INVARIANTS.md §4` did not document the seize→Deposit path explicitly. | Added to `INVARIANTS.md §4`. | d59afe1 | n/a | Documented |
| L-13 | L | Repudiation | Liquidation events read pre-mutation values for some attributes, so off-chain indexers could see inconsistent post-state vs event payload. | Re-derive event attributes from post-mutation account snapshot. | d59afe1 | `events_tests.rs` | Shipped |

## 4. Adversarial-loop notes (N-series)

| ID | Sev | Class | Root cause | Fix | Fix commit | Regression gate | Status |
|---|---|---|---|---|---|---|---|
| N-02 | N | Tampering | Rate-model edge case at the slope boundary surfaced under proptest. | Validation hardened in `validate_interest_rate_model`. | d59afe1 | `fuzz_conservation.rs`, `math_rates_tests.rs` | Shipped |
| NEW-01 | H | Tampering | `strategy::swap_tokens` left a non-zero token allowance to the aggregator after the call, enabling later drain. | Allowance zeroed after the swap regardless of Ok/Err (`controller/src/strategy.rs`). | d59afe1 | `fuzz_strategy_flashloan.rs:248, 336`, `strategy_panic_coverage_tests.rs:400` | Shipped |

(Other N-series notes — N-01, N-03 through N-13 — were either confirmed
non-issues, subsumed by the H/M/L items above, or absorbed into Maturity-roadmap
items in `audit/CODE_MATURITY_ASSESSMENT.md`. The historical narrative was
intentionally removed in `cb14a72` to avoid confusing the external audit team.)

## 5. Informational

| ID | Sev | Class | Root cause | Resolution | Status |
|---|---|---|---|---|---|
| I-01 | I | Build | CVLR git revs were unpinned in workspace `Cargo.toml`, risking spec-build drift. | Workspace `Cargo.toml` uses `[patch]` redirecting to `vendor/cvlr/`; vendored copy is the single source of truth. | Shipped |
| I-02 | I | Build | `soroban-sdk` and OpenZeppelin Stellar deps used caret pins (`^0.7.x`) that allow patch-version drift. | Tightened to `=` pins in `Cargo.toml`. | Shipped |
| I-03 | I | Doc | OpenZeppelin Stellar contracts (v0.7.0) are young; the repo bears recommend a targeted manual review of access-control / ownable / pausable / upgradeable. | Documented in `audit/SCOPE.md "Trusted Dependencies"`. | Documented |

## 6. Pre-audit findings additionally hardened during prep

These shipped between commit `d59afe1` and the current `audit-2026-q2` HEAD;
they did not have an "X-NN" tag at finding time but were surfaced by the
verification pass documented in `audit/AUDIT_PREP.md "Verified Facts vs Inferred"`.

| Class | Fix | Fix commit | Regression gate |
|---|---|---|---|
| Aggregator-callback re-entry | `strategy::swap_tokens` brackets the aggregator call with `set_flash_loan_ongoing(true/false)` (`controller/src/strategy.rs:467, 487`); controller-side `received >= amount_out_min` postcheck at `:517`. | shipped pre-`d59afe1`; verified in `audit/AUDIT_PREP.md` | `fuzz_strategy_flashloan.rs` |
| `withdraw amount==0` sentinel doc drift | Updated `architecture/ARCHITECTURE.md:246` and `architecture/MATH_REVIEW.md` drift table (controller maps `amount==0 → i128::MAX`; pool takes the full-withdraw branch via `amount ≥ current_supply_actual`). | post-`d59afe1` doc-fix | n/a |
| `flash_loan_end` SAC auth pattern doc drift | Corrected `architecture/ACTORS.md` to describe Soroban-native auth (`env.authorize_as_current_contract` in receiver) — not ERC-20 `transfer_from`/`approve`. | post-`d59afe1` doc-fix | n/a |
| Action discriminator on `UpdatePositionEvent` | Added `action: Symbol` to `UpdatePositionEvent` so off-chain indexers can identify supply / borrow / withdraw / repay without parsing payload shape (`common/src/events.rs:249-269`). | a4d2afe + c0ced1a | `events_tests.rs` |

## 7. Outstanding (still open) items

These items are tracked in
[`audit/AUDIT_CHECKLIST.md "Still Outstanding"`](./AUDIT_CHECKLIST.md#still-outstanding)
and form the audit-team handoff list:

| Item | Class | Severity | Status | Plan |
|---|---|---|---|---|
| Empirical max-position liquidate cost benchmark at `PositionLimits = 32/32` | DoS | M | Outstanding | New `test-harness/tests/bench_liquidate_max_positions.rs`; record instructions / r/w entries / write bytes against the 400M / 200 / 200 / 286KB ceilings. Operator-policy fallback: keep `PositionLimits = 10/10` until measured. (This audit-prep cycle: P1-7.) |
| `max_borrow_rate_ray` upper-bound cap (or adaptive `MAX_COMPOUND_DELTA_MS`) | Misconfig | L | Outstanding | Cap `max_borrow_rate_ray ≤ 2 * RAY` in `validate_interest_rate_model` (`controller/src/validation.rs`) and `pool.update_params` (`pool/src/lib.rs::update_params`). Resolves the documented Taylor-accuracy envelope concern. (This cycle: P1-6.) |
| Reflector behaviour spec (TWAP availability post-redeploy, `Stellar` vs `Other` asset-kind dispatch, decimals upgrade) | Tampering | M | Outstanding | External Reflector-team contact. Open asks recorded in `architecture/STELLAR_NOTES.md §Reflector` (Q6–Q10) and routed to auditors at engagement kickoff if unanswered. |
| Empirical Certora `certoraSorobanProver` end-to-end run | Verification | — | Outstanding | Resolve the `cvlr` build per `controller/certora/HANDOFF.md` on a fresh clone, run `certoraSorobanProver controller/confs/math.conf`, record verdicts in `MATH_REVIEW.md §0`. (This cycle: P1-8.) |
| Tautological / weak / vacuous Certora rule rewrite | Verification | — | Outstanding | 13 tautological rules to rewrite to call prod, 6 weak rules to tighten bounds, 5 vacuous rules to repair. Plan in `architecture/MATH_REVIEW.md §3.2 / §3.3 / §3.4`. |

## 8. Maturity-roadmap items

The Trail-of-Bits 9-category maturity scorecard
(`audit/CODE_MATURITY_ASSESSMENT.md`) flags additional non-finding hardening
work, not blocking the audit submission:

- **C-1, C-2, C-3** (Critical, ship before audit hand-off): empirical Certora
  run, `max_borrow_rate_ray` cap, `add_rewards` balance-delta or vanilla-SAC
  restriction.
- **H-1, H-2, H-3, H-4, H-5** (High, ship in next minor): on-chain timelock
  for highest-impact ops; two-step `set_position_limits` / `configure_market_oracle`;
  close `MATH_REVIEW.md §0` items; split `pool/src/lib.rs`; raise inline `///`
  density.
- **M-1 through M-6** (Medium, next major): incident-response runbook,
  glossary, closed-sequence ULP property test, runtime token-WASM check,
  KEEPER rate-limit on `clean_bad_debt`, `cargo-complexity` CI step.

## 9. Test-evidence index

The regression suite that gates every shipped finding lives under
`test-harness/tests/`. The audit team can replay every gate with:

```bash
make build                                       # produces wasm artifacts
cargo test --workspace                           # 685 passed (currently)
make proptest PROPTEST_CASES=50000               # nightly-cadence campaign
make fuzz FUZZ_TIME=1800                         # nightly-cadence campaign
```

CI re-runs every gate on every push (`.github/workflows/ci.yml` and
`fuzz.yml`).

## 10. Reading order for auditors

1. Start with this file for the finding history.
2. Read `architecture/ENTRYPOINT_AUTH_MATRIX.md` for per-fn auth/invariant
   citations.
3. Read `architecture/INVARIANTS.md` for the algebraic claims each finding
   protects.
4. Read `audit/STRIDE.md` for the threat-model framing each finding
   addresses.
5. Read `audit/THREAT_MODEL.md` for adversary-capability depth per concern
   class.
6. Use `audit/TOOLING_SCAN.md` to reproduce the static analysis verdict.
