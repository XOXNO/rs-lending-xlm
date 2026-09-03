# A108 — Missing tests/rules for highest-severity gaps

- Agent: A108
- Theme: T8
- Severity: medium (evidence debt on medium residuals; not a novel Critical exploit)
- Status: partial (coverage map) — no production code change
- Paths: synthesis over A080, A048, A056, A055, A064, A110 RB-03/04/05/06/12; evidence inventory in `tests/test-harness/tests/strategy/{router,happy,adversarial,edge}/*.rs`, `tests/test-harness/src/mock_aggregator.rs`, `contracts/controller/tests/{spoke,positions/flags,config/asset_flags}.rs`, `certora/controller/spec/{strategy_rules,spoke_rules,health_rules}.rs`, peers A085/A101/A103/A106/A109
- Defense: Strong existing pins for **custody** (OverPull / UnderPull / OutputShortfall / Refund), **usage per-leg Δ** (`usage_*_tracks_scaled_delta`, `usage_exit_without_usage_row_is_a_noop`), **FreezePolicy matrix** (`flags.rs`, `no_seize_blocks_*`, `a_spoke_at_its_supply_cap_can_still_be_credited`), **FOT/extra-credit** on multiply/flash (`WeirdToken`), **flash_position controller floors** (`test_flash_position_rejects_below_minimum`)
- Gap: Highest-severity residuals lack **named** harness/Certora that (1) demonstrate dust-out sticks under spare HF / debt-free collateral swap, (2) prove or detect global `Σ positions ≈ usage`, (3) pin ADR-0008 Option C once shipped (and document today’s supply-under-`no_seize`), (4) cover non-SAC beyond FoT on strategy withdraw→swap paths
- Impact: Evidence gaps delay remediation confidence; they do **not** by themselves mint shares. Absolute loss ceilings remain A106 S1–S7 / A110 RB-01..06
- Evidence: A109 §9 (“Prefer A056 F7 dust-out vs large payload min and A080 usage↔Σ positions”); A110 RB-12; A085 §8; A056 §8/§10; A101 G-SLIP → A108; A106 §8 P3
- Opinion: Prioritize **named** tests that lock remediation acceptance criteria for RB-03 / RB-06 / RB-05. Do not invent Criticals. Do not weaken `usage_exit_without_usage_row_is_a_noop` without a product change.

---

## 0. Mission and method

**Mission:** Concrete missing **test names** and **Certora rule names** for the highest-severity controller-defense residuals owned by A080 / A048 / A056 / A055 / A064 (ordered by A110 / A106).

**Method:**

1. Read `COORDINATION.md`, `AGENT_MANIFEST` A108, `SEED`, PRELIMINARY leading residuals.
2. Read required peers A080, A048, A056, A055, A064, A110; supporting A085, A101, A103, A106, A109.
3. Inventory live harness + Certora symbols for slippage, usage, `no_seize`, lying tokens, deploy gates.
4. Propose **stable names** for missing artifacts (add after fix or as gap-documenting “today” pins where noted).
5. No git ops; write only this findings file.

**Severity filter:** Only residuals ranked medium+ in PRELIMINARY / A110 P0–P1 (plus adjacent evidence that gates those remediations). Low/info hygiene (Vec caps, docs) listed briefly in §7.

---

## 1. Ranked missing-evidence backlog (executive)

| Rank | Residual | Band | Missing class | Highest-value missing names (proposed) |
|---:|---|---|---|---|
| 1 | A048/A056 controller `min_out` (G-SLIP / S1) | P0 live code | Harness + mock mode + Certora after RB-03 | `BadMode::DustOut`; `test_swap_collateral_dust_out_settles_with_spare_hf_today`; `test_swap_collateral_rejects_measured_below_controller_min_out`; `swap_output_respects_controller_min_out` |
| 2 | A080 missing-row exit → over-admission | P1 | Impact harness + global invariant + reconcile | `test_missing_usage_row_exit_then_over_admits_to_supply_cap`; `test_spoke_usage_equals_sum_of_account_scaled`; `usage_global_equals_sum_of_positions` |
| 3 | A064 ADR-0008 Option C (`no_seize`) | P1 | Setter/entry harness (post-fix) + today’s growth pin | `test_no_seize_without_frozen_still_allows_supply_today`; `test_set_spoke_asset_flags_rejects_no_seize_without_frozen`; `test_require_can_supply_rejects_no_seize` |
| 4 | A055 non-SAC listing | P1 | Ops checklist + rebase / strategy FoT holes | Listing gate checklist; `test_rebasing_listed_token_*`; `test_swap_collateral_fee_on_transfer_credits_measured_delta` |
| 5 | A009 / router-oracle deploy gates | P0 deploy | Checklist / constant pins (not crate unit Criticals) | Sensitive-floor restore regression; ownership attestation checklist tests in deploy suite |

---

## 2. G-SLIP — Controller quantitative `min_out` (A048 / A056) ★ highest live-code evidence hole

### 2.1 What already exists

| Artifact | Pins |
|---|---|
| `tests/test-harness/tests/strategy/router.rs::test_swap_tokens_handles_zero_output_from_router` | `BadMode::OutputShortfall` → `NoSwapOutput` |
| `…::test_swap_tokens_rejects_router_pulling_more_than_allowance` | `OverPull` |
| `…::test_swap_tokens_refunds_router_underspend` / `test_swap_collateral_refunds_router_underspend_to_caller` | `UnderPull` residue → caller |
| `…::test_swap_tokens_panics_when_router_refunds_token_in` | `Refund` |
| Honest `MockAggregator` pays exactly `payload.min_out` | Assumes floor in payload; **never** models “payload min large, controller still accepts dust” |
| Certora `swap_collateral_preserves_directional_bounds`, `swap_debt_preserves_directional_bounds`, `post_gate_swap_collateral_totals_are_final` | Directional / HF finals — **not** quantitative min-out |
| Contrast: `test_flash_position_rejects_below_minimum`, `test_flash_position_rejects_all_zero_mins` | Controller-owned floor pattern strategies lack |

**Absent today:** any harness where router pays `1` (or dust) while payload claims a **large** `min_out`, and the **controller** still settles (documents gap) or rejects (after RB-03). A056 §8: “No harness asserting measured out ≥ caller min on controller.”

### 2.2 Missing harness / mock names (add now as gap pins, or with RB-03)

| Proposed name | Layer | Intent | Residual |
|---|---|---|---|
| **`BadMode::DustOut`** (alias `PayOne`) | `tests/test-harness/src/mock_aggregator.rs` | Pull full `total_in`, pay exactly `1` of `token_out`, ignore payload `min_out` | A056 F1/F7 |
| **`test_swap_collateral_dust_out_settles_with_spare_hf_today`** | harness `strategy/router.rs` or `adversarial.rs` | Debted account with spare HF; dust settles; destination credited 1; documents S1 stickiness **before** RB-03 | A048 Gap(1), A106 S1 |
| **`test_swap_collateral_debt_free_dust_out_drains_withdrawn_notional_today`** | harness | Debt-free path skips HF; nearly full withdrawn notional extractable | A048/A056 §6.2 |
| **`test_multiply_dust_out_reverts_without_spare_collateral_today`** | harness | Pins “usually HF-blocked” path so RB-03 does not mis-prioritize multiply first | A056 §6.2 |
| **`test_multiply_dust_out_settles_with_spare_hf_today`** | harness | Spare-HF multiply dust can stick | A056 |
| **`test_swap_debt_dust_out_usually_reverts_on_hf_today`** | harness | Documents often-blocked refinance path | A056 |
| **`test_repay_debt_with_collateral_cross_asset_dust_out_reverts_today`** | harness | Cross-asset RDWC dust worsens HF | A049/A056 |
| **`test_adversarial_router_pays_one_vs_large_payload_min_out_controller_accepts_today`** | harness | Payload `min_out` large; `DustOut` pays 1; controller only checks `> 0` → **succeeds** | A056 F7 |
| **`test_swap_collateral_rejects_measured_below_controller_min_out`** | harness (**after RB-03**) | Explicit controller `min_out` / decoded floor; measured Δ `< min` → reject | A110 RB-03 done-when |
| **`test_multiply_rejects_measured_below_controller_min_out`** | harness (after RB-03) | Same for multiply (+ convert_swap leg) | RB-03 |
| **`test_swap_debt_rejects_measured_below_controller_min_out`** | harness (after RB-03) | Same for swap_debt | RB-03 |
| **`test_repay_debt_with_collateral_rejects_measured_below_controller_min_out`** | harness (after RB-03) | Cross-asset RDWC | RB-03 |
| **`test_honest_aggregator_still_passes_controller_min_out`** | harness (after RB-03) | Regression: honest path + meaningful floor still works | RB-03 |
| **`test_verify_router_output_requires_received_ge_min_out`** | unit (controller tests) | Direct unit on `verify_router_output` once signature grows | A056 Opinion |

### 2.3 Missing Certora rule names

| Proposed rule | Conf hint | Asserts |
|---|---|---|
| **`swap_output_respects_controller_min_out`** | new `strategy-swap-min-out.conf` or extend strategy confs | On success of `swap_tokens` / strategy entrypoints, measured `token_out` Δ ≥ symbolic `min_out` |
| **`swap_collateral_rejects_when_out_below_min_out`** | reverts conf | Dust below floor → revert; no dest scaled increase |
| **`multiply_rejects_when_out_below_min_out`** | reverts conf | Same for multiply |
| *(optional)* **`post_gate_does_not_substitute_for_min_out`** | sanity / comment rule | Document that HF≥1 is not a price floor (may stay prose-only) |

**Do not claim** existing `swap_collateral_preserves_directional_bounds` closes slippage — A056 already notes Certora has no quantitative min.

### 2.4 Acceptance criteria (evidence)

RB-03 is **test-complete** when:

1. `BadMode::DustOut` exists and is used.
2. At least `test_swap_collateral_rejects_measured_below_controller_min_out` fails closed independent of aggregator honesty.
3. At least one Certora assert rule names the controller floor.
4. Gap-documenting `*_today` tests are deleted or flipped to reject once the floor ships.

---

## 3. A080 — Spoke usage missing-row exit / global conservation

### 3.1 What already exists

| Artifact | Pins |
|---|---|
| Unit `exit_without_usage_row_is_noop_and_does_not_persist` | Specified no-op |
| Certora **`usage_exit_without_usage_row_is_a_noop`** | Formally pins tolerance (INV-HALT-03) |
| Full `usage_*_tracks_scaled_delta` suite | **Per-leg** Δ only; seeds allow `usage ≥ account scaled` |
| Harness `spoke_caps.rs`, usage drain tests | Caps when row exists |
| `a_spoke_at_its_supply_cap_can_still_be_credited` | Credit fee-only at cap (A084 anti-regression — **keep**) |
| Composition GH-10 | Pool-share proxy loops — **not** controller usage↔Σ accounts |

**Absent:** planting missing row + exit + **over-admission to configured cap**; global `Σ positions == usage`; admin reconcile API tests (A028/A085 P-RECON).

### 3.2 Missing harness / unit / fuzz names

| Proposed name | Layer | Intent |
|---|---|---|
| **`test_missing_usage_row_exit_then_over_admits_to_supply_cap`** | harness `controller/spoke_caps.rs` or `spoke.rs` | Seed positions without usage row (or clear row); exit no-ops; subsequent supply fills from ~0 up to `supply_cap` — **impact demo** for A080 / A106 S6 |
| **`test_missing_usage_row_exit_then_over_admits_to_borrow_cap`** | harness | Same for borrow side |
| **`test_spoke_usage_equals_sum_of_account_scaled_after_ordinary_flows`** | harness / meta invariant | After N accounts supply/borrow/withdraw/repay, `get_spoke_usage` == Σ account scaled per `(spoke, hub, side)` |
| **`test_spoke_usage_equals_sum_after_strategy_and_liquidation_mix`** | harness | Include swap_collateral, Credit fee-only, Transfer seize, bad-debt cleanup |
| **`test_credit_fee_exit_noop_when_usage_row_missing`** | harness | Compound A080×A084: fee exit silent; capacity stays overstated |
| **`test_admin_reconcile_spoke_usage_from_positions`** | harness (**after RB-06**) | Permissioned rewrite heals \(U \ll P\) / \(U \gg P\) |
| **`test_reconcile_rejects_unauthorized_caller`** | harness (after RB-06) | Auth on reconcile |
| **`prop_spoke_usage_conserves_across_accounts`** | fuzz / proptest | Random multi-account ops; usage == Σ scaled (or documented slack) |
| **`test_persist_spoke_usage_only_after_pool_success_named_regression`** | unit/integration (optional P3) | Named A078 anti-regression (A085 P3) — do not reorder persist |

### 3.3 Missing Certora rule names

| Proposed rule | Notes |
|---|---|
| **`usage_global_equals_sum_of_positions`** | Assert `SpokeUsage(spoke,hub).supply == Σ_accounts supply_scaled` (and debt side). Requires multi-account ghost or summarized ledger — **beyond** current `assume_usage_seeds` |
| **`usage_under_count_allows_entry_up_to_cap`** | Reachability/impact: missing row + entry succeeds while true occupancy would have blocked — only if product wants a formal blast-radius witness |
| **`usage_cap_breach_reverts_supply`** / **`usage_cap_breach_reverts_borrow`** | Optional formal P-CAP (fixtures today force `UNCONSTRAINED_CAP` — A085) |
| **`usage_multi_asset_keys_track_independent_deltas`** | A079 adjacency: two hubs in one `supply` move two usage keys |

**Keep:** `usage_exit_without_usage_row_is_a_noop` until RB-06 intentionally changes semantics.

### 3.4 Acceptance criteria (evidence)

RB-06 is **test-complete** when over-admission impact is harnessed, a global conservation check exists (harness and/or Certora), and reconcile (if shipped) has auth + heal tests. Delta-only `usage_*` rules alone are **not** sufficient (A085/A103).

---

## 4. A064 — `no_seize` ↔ `frozen` / supply growth (ADR-0008 Option C)

### 4.1 What already exists

| Artifact | Pins |
|---|---|
| Unit `no_seize_does_not_block_entry_or_exit` | **Documents** BlockOnEntry ignores `no_seize` |
| Unit `no_seize_blocks_seizure`; harness `no_seize_blocks_the_seizure_leg_in_both_modes` | SeizureLeg |
| `no_seize_does_not_block_ordinary_withdrawal` | Exit liveness |
| `set_spoke_asset_flags_tightens_no_seize_independently` | Guardian can set `no_seize` without freeze |
| `set_spoke_asset_flags_rejects_clearing_no_seize` | Ratchet |
| Pause/freeze entry harnesses | Orthogonal matrix |

**Absent:** e2e “supply **grows** unliquidatable set under `no_seize` without `frozen`”; post-Option-C setter/entry rejects; Certora named `enforce_spoke_asset_flags` (A064: acceptable for discrete flags — not the primary hole).

### 4.2 Missing harness / unit names

| Proposed name | When | Intent |
|---|---|---|
| **`test_no_seize_without_frozen_still_allows_supply_today`** | **now** (gap pin) | `no_seize=true`, `frozen=false`; `supply` succeeds; liquidation of holder reverts `#318` | 
| **`test_no_seize_allows_second_user_to_grow_unliquidatable_collateral_set_today`** | now | Two accounts supply under flag; both unliquidatable via seize | 
| **`test_set_spoke_asset_flags_rejects_no_seize_without_frozen`** | **after Option C** | Setter coupling | 
| **`test_edit_asset_in_spoke_rejects_no_seize_without_frozen`** | after Option C | Owner edit path if applicable | 
| **`test_require_can_supply_rejects_no_seize`** | after Option C (if entry-side choice) | Entry rejects `#?` when `no_seize` | 
| **`test_no_seize_with_frozen_blocks_new_supply_and_seizure`** | after Option C | Coupled happy path | 
| **`test_force_socialize_bad_debt_still_clears_no_seize_stranded_account`** | harness | Hatch remains after Option C | 

### 4.3 Missing Certora (optional)

| Proposed rule | Notes |
|---|---|
| **`no_seize_implies_frozen_on_flag_write`** | After Option C setter |
| **`supply_rejects_when_no_seize`** | If entry-side enforcement chosen |

Not required to close discrete flag logic today; **harness pins are the P1 evidence**.

---

## 5. A055 — Non-SAC / lying / FoT listing trust

### 5.1 What already exists

| Artifact | Pins |
|---|---|
| `WeirdToken` FoT + extra-credit + transfer hooks | Multiply / flash_position / flash_loan / outbound measurement |
| `test_multiply_fee_on_transfer_*`, `test_flash_position_fee_on_transfer_*`, `test_flash_loan_fee_on_transfer_fails_closed` | Measured / fail-closed patterns |
| `test_multiply_extra_credit_is_not_pool_theft`, flash counterparts | Credit-on-transfer |

**Absent / thin:**

- No **rebasing** token suite (mid-tx balance mutates without transfer).
- No **`swap_collateral` / `swap_debt` / RDWC** FoT harness (strategy withdraw→swap→deposit legs).
- No automated **listing gate** that rejects non-SAC metadata (ops RB-04).
- No “balance-lying view” token where `balance` ≠ transferable truth beyond FoT/extra-credit.

### 5.2 Missing names

| Proposed name | Layer | Intent |
|---|---|---|
| **`test_swap_collateral_fee_on_transfer_credits_measured_delta`** | harness | Withdraw/swap/deposit books follow measured Δ; no share mint from FoT | 
| **`test_swap_debt_fee_on_transfer_fails_closed_or_credits_net`** | harness | Borrow/repay equality path under FoT debt asset | 
| **`test_repay_debt_with_collateral_fee_on_transfer_measured`** | harness | Cross-asset RDWC | 
| **`test_rebasing_listed_token_desyncs_or_is_rejected_at_list_time`** | harness / listing | Documents market-TVL risk **or** enforces pre-list reject once policy ships | 
| **`test_never_flashloanable_on_fee_on_transfer_market`** | harness / admin | RB-04: `is_flashloanable` false on non-exact | 
| **`listing_checklist_sac_only`** | ops runbook + optional CI doc test | A110 RB-04 done-when (not a Certora rule) | 

Certora: **no** productive rule for arbitrary lying tokens — listing policy is the control (A055 Opinion).

---

## 6. A009 / trust-root deploy gates (P0 absolute ceiling — evidence shape)

These are **release blockers**, not missing `#[only_owner]` unit tests.

| Proposed evidence artifact | Intent |
|---|---|
| **`test_timelock_sensitive_min_delay_is_production_floor`** (gov constants / upgrade checklist) | After Sensitive restore: assert `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS` ≥ production target (not 12) | 
| **Deploy attestation checklist** (integration / runbook, cite A009 §12) | Controller owner == governance; no pending hot `TransferCtrlOwnership`; aggregator + XOXNO owners intended multisig | 
| **`test_controller_owner_is_governance_on_harness_mainnet_config`** | Config-level pin where env exists | 

Do **not** invent a controller unit test that “proves” mainnet ownership — that is on-chain attestation (A110 RB-01/RB-02).

---

## 7. Adjacent P2/P3 evidence (brief — not highest severity)

From A110 RB-12 / A102 / A085 — list for completeness; **do not** outrank §§2–4:

| Proposed name | Residual |
|---|---|
| `test_keeper_asset_vec_rejects_over_max_inputs` / `test_mutator_payments_reject_over_max_inputs` | A062 / A015 |
| `test_liquidate_raw_debt_vec_aligns_with_estimate_256` | A062 symmetry |
| `test_refund_assets_rejects_over_length` | A070 |
| Static/lint: every pool merge → `put_market_index` + `apply_leg_usage` | A094 |
| `usage_cap_breach_reverts_*` Certora (optional) | A085 P-CAP formal |
| `test_supply_two_assets_updates_two_usage_keys` | A079 |

---

## 8. Evidence matrix (highest residuals)

| Residual | Existing strongest pin | Critical missing pin | Fix band |
|---|---|---|---|
| A048/A056 slippage | `OutputShortfall` / OverPull / UnderPull | **`DustOut` + spare-HF / debt-free settle today; reject-under-`min_out` after RB-03; `swap_output_respects_controller_min_out`** | P0 |
| A080 usage | `usage_exit_without_usage_row_is_a_noop` + per-leg Δ | **`test_missing_usage_row_exit_then_over_admits_to_supply_cap`; `usage_global_equals_sum_of_positions`; reconcile tests** | P1 |
| A064 Option C | `no_seize_does_not_block_entry_or_exit` (unit) | **E2E supply-growth under `no_seize`; post-fix setter/entry rejects** | P1 |
| A055 listing | FoT multiply/flash | **Rebase / swap_collateral FoT / listing checklist** | P1 |
| A009 deploy | `only_owner` + gov timelock tests | **Sensitive floor + ownership attestation** | P0 deploy |

---

## 9. Must-not-add / anti-patterns

1. Do **not** remove or “fix” `usage_exit_without_usage_row_is_a_noop` via a test that expects decrement on missing rows unless product changes A080.
2. Do **not** treat GH-10 or per-leg `usage_*` as closing A080 global conservation.
3. Do **not** claim `MockAggregator` paying `min_out` proves controller slippage defense.
4. Do **not** gate seizure on `paused` in new tests as the Option C solution (A064 anti-fix).
5. Do **not** invent Critical fund-theft tests for measured custody paths already closed (A101 L6–L10).

---

## 10. Cross-links

| Peer | Relation |
|---|---|
| A048 / A056 | Own G-SLIP; this file names the missing dust-vs-min evidence A056 F7 / A101→A108 |
| A080 / A085 / A103 | Own capacity residual; A085 §8 P1 rows expanded here with concrete names |
| A064 / A102 | Option C harness names for RB-05 |
| A055 / A101 G-LIST | Listing + strategy FoT holes |
| A009 / A110 RB-01/02 | Deploy attestation, not crate Critical invention |
| A106 §8 / A109 §9 | Ranking inputs for this backlog |
| A110 RB-12 | Consumes this file; was peer-derived while A108 unfiled |

---

## 11. Verdict

**Highest-severity evidence debt is concentrated in three named holes:**

1. **No controller dust-out suite** — missing `BadMode::DustOut`, spare-HF / debt-free `swap_collateral` settle pins, and post-RB-03 `*_rejects_measured_below_controller_min_out` + Certora `swap_output_respects_controller_min_out`.
2. **No A080 impact / global conservation suite** — missing over-admission-after-missing-row harness and `usage_global_equals_sum_of_positions` (Certora or keeper).
3. **No Option C acceptance harness** — missing today’s supply-growth e2e under `no_seize` and post-fix setter/entry rejects.

Ship those names with the corresponding A110 work packages (WP-B, WP-D, WP-C). Secondary: A055 rebase/strategy-FoT and A009 Sensitive/ownership attestation. No novel Critical beyond documented trust-boundary classes.
