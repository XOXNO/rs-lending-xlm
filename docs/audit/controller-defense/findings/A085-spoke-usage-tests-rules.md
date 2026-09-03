# A085 — Tests and Certora rules covering spoke usage

- Agent: A085
- Theme: T5
- Severity: info
- Status: partial
- Paths:
  - `contracts/controller/src/spoke_usage.rs` (production semantics under test)
  - `contracts/controller/tests/spoke.rs` (unit: entry/exit/cap/no-op)
  - `contracts/controller/tests/storage/spoke.rs` (prune-on-zero)
  - `common/tests/rates/scaling.rs` (`calculate_scaled_cap`)
  - `common/tests/validation.rs` (`max_cap_for_decimals` / domain)
  - `tests/test-harness/tests/controller/spoke_caps.rs`
  - `tests/test-harness/tests/controller/spoke.rs` (cap + usage drain / ratchet)
  - `tests/test-harness/tests/controller/liquidation_seize_modes.rs` (Credit fee + cap-at-limit)
  - `tests/test-harness/tests/composition/{helpers,repeated_loops_never_extract_value,atomic_revert_all_legs}.rs`
  - `certora/controller/spec/spoke_rules.rs` (V-5 `usage_*` / `usage_liq_*`)
  - `certora/controller/spec/fixture.rs` (`seed_spoke_usage`, `spoke_usage`, `UNCONSTRAINED_CAP`)
  - `certora/controller/confs/spoke-usage{,-sanity,-liquidation,-liq-sanity}.conf`
- Defense: Unit + harness tightly pin ADR-0015 / INV-HALT-03 cap behavior (zero, exact, +1, domain ceiling, exit-safe closed markets, interest-tightening). Certora V-5 proves **per-leg** `Δusage == Δscaled` (both sides) across ordinary verbs, strategy legs, Transfer seize, Credit fee-sum, and bad-debt cleanup, with an exhaustive `PositionAction` coverage guard and paired reachability witnesses. Composition GH-10 empirically checks spoke usage against pool share totals after long loops; GH-09 rolls usage back with failed scripts.
- Gap: (1) Certora seeds `usage >= account scaled` and proves **delta tracking**, not global `Σ positions ≈ usage` — does **not** close A080. (2) Cap breach / `SpokeSupplyCapReached` / `SpokeBorrowCapReached` have **no** Certora assert rules (fixtures force `UNCONSTRAINED_CAP`). (3) No dedicated multi-asset two-key usage end-to-end rule (A079 residual). (4) A080 missing-row exit is **pinned as specified** (unit + `usage_exit_without_usage_row_is_a_noop`), not tested as an over-admission scenario. (5) No admin reconcile tool/tests; no fuzz target for caps/usage desync. (6) Persist-before-pool ordering is structural, not a named regression test.
- Impact: Coverage gaps leave soft-governance capacity integrity (A080) formally open and multi-key batch usage only compositionally evidenced. They do **not** by themselves demonstrate theft, HF bypass, or a second medium beyond A080. Cap enforcement itself is well covered at unit/harness level.
- Evidence: INV-HALT-03; ADR-0015; ADR-0019 (Credit fee exemption); peers A076–A080, A079, A082, A084, A103 §7.4; Certora suite review table of `usage_*` rules; `certora/profiles.json` spoke-usage confs.
- Opinion: Spoke-usage **test/proof surface is among the densest T5 defenses** for what it claims (leg deltas + caps in harness). Treat “VERIFIED” language on INV-HALT-03 carefully: formal rules verify exit no-op and withdraw delta, while zero/exact cap are harness-owned. Highest-value follow-ups are a global reconcile invariant/keeper (A080/A103 P1) and an explicit multi-asset usage harness (A079), not more single-leg delta clones.

---

## Scope and method

1. Read `shared/COORDINATION.md`, `SEED.md`, `AGENT_MANIFEST.md` (A085), A103 §7.4 gap brief, and peers A076–A080 / A079 / A084.
2. Inventory every unit, harness, composition, common, fuzz, and Certora surface that names `SpokeUsage` / `apply_entry|exit` / caps / `usage_*` rules.
3. Map each property class → what is proved / tested → what remains unproven.
4. Explicitly separate **delta tracking** from **global reconciliation** (A080) and **cap enforcement** (INV-HALT-03 / ADR-0015).
5. No production Rust edits; no git.

Out of scope as primary claims: inventing new semantic bugs in `spoke_usage.rs` (A076); index choice (A077/A081); persist ordering correctness (A078 — covered here only as regression-test debt); Credit fee intent (A084).

---

## 1. Property classes (what “covering spoke usage” means)

| ID | Property | Why it matters |
|---|---|---|
| P-DELTA | After a wired verb, `Δ SpokeUsage == Δ Σ scaled positions` (per hub, both sides) | Catches a merge that skips `apply_leg_usage` |
| P-COV | Every `PositionAction` / merge leg is classified and exercised | Tomorrow’s verb cannot silently escape V-5 |
| P-CAP | Entry rejects when `usage + Δ > calculate_scaled_cap(cap, decimals, index)`; zero cap admits nothing | INV-HALT-03 / ADR-0015 |
| P-EXIT | Exits never consume caps; underflow / negative panics; missing row no-ops | Exit safety + A080 specified behavior |
| P-PERSIST | Durable usage writes only after pool success (ordinary + bad-debt) | A078 must-not-regress |
| P-LIQ | Transfer full seize exit; Credit fee-only exit; bad-debt sheds wiped positions | A084 / INV-LIQ |
| P-GLOBAL | `SpokeUsage(spoke, hub) == Σ_accounts scaled(side)` (or documented tolerance) | Closes A080 under-count |
| P-MULTI | Multi-asset batch updates distinct usage keys by each leg’s delta | A079 residual |
| P-RECON | Admin/keeper can rewrite usage from positions | A028 / A103 P1 |

---

## 2. Certora inventory (V-5)

### 2.1 Confs and profiles

| Conf | Role | Rules (assert / satisfy) |
|---|---|---|
| `spoke-usage.conf` | Core delta + coverage + A080 pin | 13 assert rules |
| `spoke-usage-sanity.conf` | Reachability witnesses for ordinary/strategy | 9 satisfy rules |
| `spoke-usage-liquidation.conf` | Credit sum + bad-debt | 2 assert rules |
| `spoke-usage-liq-sanity.conf` | Liq reachability + `Credit(0)` deprecated | 6 satisfy rules |
| `spoke-usage-reverts{,-sanity}.conf` | **Not usage/caps** — add-asset listing reverts only | listed-asset fixture |

Profiles in `certora/profiles.json` wire the four usage confs above. Artifact: `controller-spoke-rules.wasm` (`certora-spoke-rules` feature).

`spoke-usage-reverts*` is named for the spoke-rules wasm, not for `SpokeSupplyCapReached`. **No conf proves a cap revert.**

### 2.2 Assert rules (`usage_*` / `usage_liq_*`)

| Rule | Level driven | Property |
|---|---|---|
| `usage_supply_tracks_scaled_delta` | `process_supply` entrypoint | P-DELTA supply entry |
| `usage_withdraw_tracks_scaled_delta` | `process_withdraw` | P-DELTA supply exit |
| `usage_borrow_tracks_scaled_delta` | `process_borrow` | P-DELTA debt entry |
| `usage_repay_tracks_scaled_delta` | `process_repay` | P-DELTA debt exit |
| `usage_strategy_borrow_leg_tracks_scaled_delta` | `borrow_into_controller` + persist | P-DELTA strategy debt entry |
| `usage_strategy_withdraw_leg_tracks_scaled_delta` | strategy withdraw merge + persist | P-DELTA |
| `usage_strategy_repay_leg_tracks_scaled_delta` | strategy repay merge + persist | P-DELTA |
| `usage_strategy_net_settle_tracks_scaled_delta` | `net_settle` both sides | P-DELTA dual-side |
| `usage_liq_repay_leg_tracks_scaled_delta` | `apply_repay_batch` / LiqRepay | P-DELTA + debt falls |
| `usage_liq_transfer_seize_leg_tracks_scaled_delta` | `apply_withdraw_batch` / Liquidation | P-DELTA + supply falls |
| `usage_liq_credit_seize_sums_over_two_accounts` | `process_liquidation` Credit | P-LIQ fee = Σ debit−credit |
| `usage_liq_bad_debt_cleanup_sheds_every_wiped_position` | `clean_bad_debt_standalone` | P-LIQ residual = extras |
| `usage_coverage_no_unwired_verb` | nondet `PositionAction` × Wave0 leg | P-COV compile+semantic |
| `usage_param_refresh_moves_neither` | ParamUpd | no scaled/usage move |
| `usage_exit_without_usage_row_is_a_noop` | withdraw with no usage row | **pins A080** (P-EXIT as specified) |

Shared helper `assert_usage_tracks_scaled` asserts both supply and debt sides’ deltas and `usage_after >= 0`.

### 2.3 Reachability witnesses

Ordinary: `usage_{supply,withdraw,borrow,repay}_reachable`, four strategy `*_reachable`, `usage_coverage_dispatch_reachable`.

Liquidation: `usage_liq_{repay,transfer_seize}_leg_reachable`, `usage_liq_credit_seize_reachable`, `usage_liq_credit_fee_exits_usage_reachable` (fee exit must fire), `usage_liq_bad_debt_cleanup_reachable`, plus `credit_zero_liquidation_creates_receiver_in_deprecated_spoke` (account mint / deprecation — adjacent to usage seeding).

Reachability is load-bearing: a `satisfy(true)` would not catch a leg that never called `apply_leg_usage`; these witnesses require usage to move in the expected direction.

### 2.4 Coverage guard (compile-time)

`usage_coverage_class` matches **exhaustively** over `events::PositionAction` (no `_`). Wave0 / NoScaledMove / CrossAccount buckets force authors to name legs or point at an existing rule. Adding a verb without classifying it fails `--features certora-spoke-rules` compile. This is the strongest structural defense against “forgot `apply_leg_usage`” regressions.

### 2.5 What Certora deliberately does **not** prove

| Unproven | Mechanism |
|---|---|
| P-GLOBAL equality | `assume_usage_seeds`: `usage_supply >= supply_scaled` (and debt). Seeds allow usage **above** the exercised account; never assert `usage == Σ all accounts`. |
| P-CAP breach | Fixture `UNCONSTRAINED_CAP`; comment at `USAGE_SEED_MAX`: keep far inside cap so a cap revert cannot vacate a rule. |
| P-MULTI two-key | Rules are single-`asset` (Credit uses collateral+debt but asserts collateral usage vs pair sum). |
| Initial consistency | Arbitrary ghost storage can start desynced; proofs only that wired verbs preserve delta coupling from that seed. |
| Persist-before-pool | Strategy/liq leg rules call `cache.persist_spoke_usage()` after the leg; they do not model a failed pool then durable write. |

This matches A103 §7.4 and A080’s recommended follow-up: delta proofs ≠ global reconcile.

---

## 3. Unit tests

### 3.1 `contracts/controller/tests/spoke.rs` (21 tests; usage-focused subset)

| Test | Property |
|---|---|
| `usage_supply_decrement_below_zero_panics` / `usage_borrow_*` | P-EXIT underflow |
| `zero_supply_cap_rejects_entry` / `zero_borrow_cap_rejects_entry` | P-CAP zero |
| `apply_entry_stores_single_add_not_dual_add` | no double-add bug |
| `apply_entry_at_exact_cap_succeeds` / `apply_entry_one_over_cap_reverts_with_supply_cap` | P-CAP boundary |
| `apply_entry_overflow_on_usage_plus_delta_panics` | overflow before/at cap |
| `ceiling_cap_saturates_instead_of_panicking_at_the_index_floor` | scaled-cap saturation |
| `exit_without_usage_row_is_noop_and_does_not_persist` | **A080 pin** — no invented zero row |
| `entry_without_usage_row_default_inserts_and_persists` | entry creates row |
| `exit_sees_entry_cached_row_in_same_context` | RAM map before persist |
| `usage_side_cap_reads_matching_field` | Supply↔supply_cap / Borrow↔borrow_cap |
| `full_exit_after_entry_prunes_storage` | prune-on-zero via persist |
| listing panics / LTV helpers | adjacent listing, not usage math |

### 3.2 Storage + common

| File | Coverage |
|---|---|
| `contracts/controller/tests/storage/spoke.rs::spoke_usage_prunes_zero_entry` | `set_spoke_usage` removes zero row |
| `common/tests/rates/scaling.rs::calculate_scaled_cap_floors_and_saturates` (+ related) | P-CAP math primitive |
| `common/tests/validation.rs` max-cap domain | governance cap ceiling |

Unit layer owns **direct** `SpokeUsageContext` semantics that Certora mostly exercises through position APIs.

---

## 4. Integration / harness tests

### 4.1 `spoke_caps.rs` (11 tests) — INV-HALT-03 end-to-end

Zero supply/borrow reject with **no usage write**; closed market (caps 0) still allows full repay/withdraw and drains usage; closed market still allows bad-debt cleanup and drains both sides; domain ceiling accept / over-ceiling reject / `i128::MAX` reject on edit and add; exact cap then +1 for supply and borrow.

This is the primary **P-CAP** evidence. Certora does not duplicate it.

### 4.2 `controller/spoke.rs` usage/cap suite (~12 relevant)

| Test | Notes |
|---|---|
| `test_spoke_supply_cap_enforced` / `test_spoke_borrow_cap_enforced` | cumulative breach |
| `test_removed_spoke_asset_withdraw_decrements_usage` | delisted exit still tracks |
| `test_deprecated_spoke_repay_decrements_usage` | deprecated exit still tracks |
| `test_edit_spoke_{supply,borrow}_cap_below_usage_ratchets_down` | ratchet + resume after drain |
| `test_spoke_supply_cap_bounds_cumulative_supply` | fill to ceiling |
| `test_spoke_spoke_supply_cap_headroom_restored_after_withdraw` | exit restores headroom |
| `test_spoke_spoke_borrow_cap_tightens_as_interest_accrues` | live index tightens effective headroom (A077 adjacency) |
| `test_remove_asset_with_live_{supply,borrow}_usage_reverts_until_drained` | usage gates delisting |
| domain overflow reject | admin validation |

### 4.3 Liquidation seize modes

| Test | Property |
|---|---|
| `credit_mode_moves_spoke_usage_by_exactly_the_protocol_fee` | P-LIQ / A084 — usage ↓ by fee only |
| `a_spoke_at_its_supply_cap_can_still_be_credited` | Credit exempt from supply entry cap (ADR-0019) |

These are the load-bearing harness twins of `usage_liq_credit_*` rules.

### 4.4 Composition

| Test | Property |
|---|---|
| `repeated_loops_never_extract_value` (GH-10) | After 50 supply/borrow/repay/withdraw cycles, `SpokeUsage.supplied == pool.supplied − revenue` and `borrowed == pool.borrowed` per asset — **empirical P-GLOBAL proxy vs pool shares** (single harness spoke) |
| `atomic_revert_all_legs` (GH-09) | Failed script leaves Snapshot (includes usage) unchanged — atomicity / P-PERSIST adjacency |

GH-10 is the closest existing check to A080’s recommended invariant, but it compares to **pool** totals (minus revenue), not a scanned sum of controller account maps, and assumes a healthy single-spoke market without planted missing usage rows.

### 4.5 Gaps in harness

- No test that **plants** a missing usage row, exits, then **over-admits** to cap (A080 impact demo).
- No `supply([(A,x),(B,y)])` asserting both usage keys independently (A079 → A085 residual).
- No cross-spoke usage isolation scenario (A083 still unfiled).
- No dedicated “persist called before pool” negative test (would need a test-only hook).
- Fuzz (`tests/fuzz`, harness fuzz) does not target spoke usage / caps; migrate “reconcile” means Blend leftover debt, not usage rows.

---

## 5. Property → evidence matrix

| Property | Unit | Harness | Composition | Certora | Verdict |
|---|---|---|---|---|---|
| P-DELTA ordinary | partial (context) | usage drain asserts | — | **strong** (4 entrypoints) | **covered** |
| P-DELTA strategy / net settle | — | indirect | — | **strong** | **covered** |
| P-DELTA / P-LIQ Transfer | — | — | — | **strong** | **covered** |
| P-LIQ Credit fee | — | **strong** | — | **strong** | **covered** |
| P-LIQ bad debt | — | closed-market cleanup | — | **strong** | **covered** |
| P-COV new verb | — | — | — | **strong** (exhaustive match) | **covered** |
| P-CAP zero / exact / +1 | **strong** | **strong** | — | **absent** | **covered** (tests own) |
| P-CAP interest / ratchet / headroom | — | **strong** | — | absent | **covered** (tests) |
| P-EXIT underflow / prune | **strong** | — | — | via delta ≥0 | **covered** |
| P-EXIT missing row | **pins** | — | — | **pins** | **specified**, not impact-tested |
| P-PERSIST after pool | — | GH-09 snapshot | GH-09 | structural call order | **defended in A078**; thin named regression |
| P-GLOBAL Σ accounts | — | — | GH-10 pool proxy | **not proved** | **open (A080)** |
| P-MULTI two-key | dual-add unit only | multi-asset verbs exist; no usage assert | — | bulk positions ≠ usage | **thin (A079)** |
| P-RECON admin | — | — | — | — | **absent** |

---

## 6. Interaction with peer findings

| Peer | Relation |
|---|---|
| A076 | Semantics unit-tested in `tests/spoke.rs`; A085 maps those tests |
| A077 | Borrow-cap-tightens-with-interest harness; no Certora cap×index rule |
| A078 | Persist ordering defended in code; A085 notes lack of a named anti-regression test |
| A079 | Explicitly assigns multi-asset usage end-to-end proof to A085 — still open |
| A080 | Formally **pinned** as no-op; **not** closed by P-DELTA; medium residual remains |
| A082 | Pool-truth deltas are what `usage_*_tracks_scaled_delta` encode |
| A084 | Credit fee harness + Certora twins |
| A103 §7.4 | This file discharges the “A085 absent” coverage debt; confirms provisional gap list |

No disagreement file needed: agrees with A103 that delta proofs leave A080 open; agrees with A079 on multi-key residual.

---

## 7. INV / ADR claim hygiene

INV-HALT-03 lists VERIFIED rules `usage_exit_without_usage_row_is_a_noop` and `usage_withdraw_tracks_scaled_delta` plus `spoke_caps.rs`. That is accurate for **exit no-op + withdraw delta + harness caps**, but:

- Zero-cap / exact-cap are **not** Certora-verified (fixtures unconstrained).
- `usage_exit_without_usage_row_is_a_noop` verifies the **tolerance**, which is exactly A080’s residual when positions exist without a row — do not read that rule as “usage always equals positions.”

ADR-0015 auditor focus (zero, boundary, index-change, multi-leg, exit) is largely met by unit+harness; multi-leg **usage** remains the weak formal/harness corner.

---

## 8. Prioritized coverage backlog (tests/rules only)

| Priority | Add | Closes |
|---|---|---|
| P1 | Keeper/harness: for each `(spoke, hub)`, assert usage vs Σ account scaled (or document intentional slack) | P-GLOBAL / A080 |
| P1 | Harness: missing usage row + exit + subsequent supply fills to configured cap (documents blast radius) | A080 impact |
| P2 | Harness or Certora: `supply` two distinct assets → each `SpokeUsage` key moves by its own scaled Δ | P-MULTI / A079 |
| P2 | Optional Certora: constrained cap where `usage + Δ > cap_scaled` forces revert (one supply + one borrow) | P-CAP formal |
| P3 | Named unit/integration guard: `persist_spoke_usage` call sites after pool success (static checklist already in A078) | P-PERSIST regression |
| P3 | Fuzz/proptest: random cap + multi-account usage conservation | residual search |
| P3 | Admin reconcile API + tests when/if productized | P-RECON |

Do **not** weaken `usage_exit_without_usage_row_is_a_noop` without an intentional product change — it documents current contract behavior.

---

## 9. Verdict

**Partial — strong where scoped, open where A080/A079 need global or multi-key evidence.**

Spoke usage is one of the better-tested controller surfaces:

1. Cap policy (ADR-0015 / INV-HALT-03) is thoroughly owned by unit + `spoke_caps` + spoke harness.
2. Certora V-5 is a serious delta-tracking and coverage-guard suite across ordinary, strategy, and liquidation legs, with fee-aware Credit accounting.
3. Composition GH-10 gives a practical conservation check against pool share books.

The coverage does **not** prove that usage starts or stays equal to the sum of live positions, does **not** formally prove cap reverts, and does **not** end-to-end prove multi-asset key isolation. Those holes are **evidence gaps**, aligned with A080 (medium semantic residual) and A079 (info residual), not a newly discovered critical defect in production code.

---

## 10. Sources read

- `docs/audit/controller-defense/shared/{COORDINATION,SEED,AGENT_MANIFEST}.md`
- Findings: A076, A077, A078, A079, A080, A084, A103 (§7.4), adjacency A028 / A102 / A104
- `contracts/controller/src/spoke_usage.rs`
- `contracts/controller/tests/spoke.rs`, `storage/spoke.rs`
- `tests/test-harness/tests/controller/{spoke_caps,spoke,liquidation_seize_modes}.rs`
- `tests/test-harness/tests/composition/{helpers,repeated_loops_never_extract_value,atomic_revert_all_legs}.rs`
- `certora/controller/spec/{spoke_rules.rs,fixture.rs}`
- `certora/controller/confs/spoke-usage*.conf`, `certora/profiles.json`
- `docs/reference/invariants.md` (INV-HALT-03), `docs/explanation/decisions/0015-*.md`
- `docs/explanation/certora-suite-review-2026-09-03.md` (`usage_*` table)
