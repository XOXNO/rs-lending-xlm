# A108 — Missing tests/rules for highest-severity gaps

- Agent: A108 (synthesis)
- Theme: T8
- Severity: medium (evidence debt on the leading live residuals; no novel Critical)
- Status: partial (corpus tests/rules are dense on *defended* stacks; thin on *residual* blast-radius and on post-fix asserts)
- Paths: synthesis over findings A080, A048, A056, A055, A064, A015, A062, A101–A106 (plus A085, A107, A110 adjacency); live pins in `certora/controller/spec/{spoke_rules,strategy_rules,health_rules,market_guard_rules}.rs`, `tests/test-harness/tests/strategy/{router,happy,adversarial,flash_position*}.rs`, `contracts/controller/tests/{spoke,positions/flags,config/asset_flags,views}.rs`
- Defense: See §3 — what is already pinned (do not clone)
- Gap: See §4–§7 — named missing harness / unit / Certora / fuzz / static-gate artifacts. **Names only; no production code in this file.**
- Impact: Missing evidence does not create theft. It leaves (1) residual blast radius un-demonstrated (A080 over-admission, A048/A056 dust-out stickiness, A064 supply-under-`no_seize`), (2) STRIDE Tamper.4 / INV-STRAT-02 overclaiming “minimums” without a controller floor pin, (3) post-fix regressions unguarded once A110 RB-03/RB-05/RB-06/RB-07 land
- Evidence: Peer “A108 backlog” pointers in A056 §7/§10, A101 §10, A103 §7.4, A085 §8, A105 §9, A106 §8, A110 RB-12; live inventories in §3
- Opinion: Highest-value *new* artifacts are **not** more `usage_*_tracks_scaled_delta` clones or another `RouterOverspend` case. They are: (a) **PIN** tests that today’s controller *accepts* dust-out / missing-row over-admission / `no_seize`+supply; (b) **CLOSE** twins that fail until controller `min_out`, ADR-0008 Option C, usage reconcile, and Vec caps ship; (c) Certora rules that prove *floors and global equality*, which current suites deliberately do not. Keep `usage_exit_without_usage_row_is_a_noop` as a specification pin until product changes it.

---

## 0. Mission, method, and naming rules

### 0.1 Mission

Inventory **missing tests and formal rules** for the audit’s **highest-severity residual classes**, and propose **concrete names** (harness `test_*`, unit `fn`, Certora `#[rule]`, fuzz `prop_*`, optional static gates). Do **not** write implementation code.

**In-scope residual owners (user + PRELIMINARY / A106):**

| Rank | Residual | IDs | Sev | Kind (A106) |
|---:|---|---|---|---|
| 1 | Controller positivity-only swap out | A048, A056, A101 G-SLIP | medium | D — account |
| 2 | `apply_exit` missing-row no-op | A080, A103, A085 | medium | C — market cap |
| 3 | `no_seize` ̸⇒ `frozen` / still allows supply | A064 G1, A102 G-VAL-1 | medium | A→C — liq halt |
| 4 | Listed non-SAC / lying / rebasing tokens | A055, A101 G-LIST | medium | C — ≤ TVL_m |
| 5 | Uncapped mutator / keeper Vecs | A062, A015, A102 G-VAL-2 | low | A — fees |
| — | Syntheses that *name* this file | A101–A106 | — | evidence routing |

**Adjacent high-leverage evidence holes** (not the user’s primary list, but peers already assigned them to A108): INV-AUTH-02 Certora skew (A003), `put_market_index` footgun (A094/A104), Sensitive-floor constant pin (A009/A110 RB-01), A060 unfiled dust↔bad-debt.

### 0.2 Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST.md` (A108).
2. Read required primaries A080, A048, A056, A055, A064, A015, A062 and syntheses A101–A106.
3. Read coverage-owner peers **A085** (spoke-usage tests/rules — do not duplicate its P1 list without expanding) and **A110 RB-12** (inferred test rows while this file was unfiled).
4. Spot-check live names in Certora `usage_*` / `swap_*` / `post_gate_*`, harness `strategy/router.rs` `BadMode`, unit `spoke.rs` / `flags.rs` / `views.rs`.
5. Split every proposed artifact into **PIN** (documents current residual; must pass *today*) vs **CLOSE** (fails until the matching A110 remediation) vs **REGRESS** (defended stack still unpinned).
6. Cross-link; no `disagreements/` — A085/A110 inferred names are **superseded and expanded**, not contradicted.

No production Rust. No git operations.

### 0.3 Artifact classes

| Tag | Meaning | CI expectation *today* |
|---|---|---|
| **PIN** | Locks residual *behavior* so a silent “fix” cannot land without an intentional product change | **Must pass** on current tree |
| **CLOSE** | Asserts the remediated world (A110 RB-03/05/06/07) | **Must fail** (or be `#[ignore]`/`xfail`) until the fix; flip on the same PR as the fix |
| **REGRESS** | Defended control with no named pin | Should pass today; add to prevent silent weaken |

### 0.4 Naming conventions (match the tree)

| Layer | Pattern | Home |
|---|---|---|
| Harness | `test_<verb>_<property>` / `regression_*` | `tests/test-harness/tests/{strategy,controller,composition}/` |
| Unit | snake, no `test_` prefix in controller crate tests | `contracts/controller/tests/` |
| Certora | `snake_case` `#[rule]` + optional `*_reachable` | `certora/controller/spec/` + new/extended `.conf` |
| Fuzz | `prop_*` | `tests/test-harness/tests/fuzz/` |
| Static | `make` / script name | `scripts/` or `make access-control-check`-class gates |
| Integration | `inv_*` / flow assert | `tests/integration/flows/` |

**Do not** invent a second name for an existing pin (§3). Proposed names below are **absent** from `rg 'fn <name>'` at write time unless marked *extend*.

---

## 1. Executive verdict

**The leading residuals are under-evidenced at the blast-radius layer, over-evidenced at the custody/delta layer.**

| Residual | What *is* pinned | What is *missing* |
|---|---|---|
| G-SLIP (A048/A056) | Over-pull, underspend refund, **zero** out (`NoSwapOutput`), directional scaled bounds, post-gate HF, honest-router exact Δ fuzz | Router that pays **1** vs large payload `min_out`; debt-free / spare-HF **successful** dust-out; Certora `measured_out ≥ min_out`; CLOSE twins after controller floor |
| A080 | Missing-row exit is a **no-op** (unit + Certora); per-leg `Δusage == Δscaled`; GH-10 pool-share proxy on a *healthy* book | Planted missing row → exit → **fill to cap**; global `Σ positions ≈ usage`; reconcile API tests; missing-row on Credit-fee / bad-debt |
| A064 G1 | `no_seize` blocks seizure; does **not** block entry/exit; guardian can set it **independently** of `frozen` | Supply still allowed under `no_seize`; unliquidatable set **grows**; Option C setter CLOSE; Certora `no_seize ⇒ frozen` after ship |
| A055 | SAC FoT: strategy borrow fail-closed; flash min miss; measured inbound; revenue dust intact | Rebase / balance-liar **desync demos**; FoT on `swap_collateral` withdraw budget; Transfer-seize liquidator haircut; no rebasing mock |
| A062/A015 | Views cap 256; payment **sum** of dupes; flash/migrate-debt **reject** dupes; keeper *money* destination | Keeper/mutator **length** reject; `liquidate` vs estimate 256 symmetry; empty-Vec policy pin |

**No missing test, by itself, is a Critical hole.** The gap is that A110 remediations can ship without a failing-then-green evidence pair, and that STRIDE Tamper.4 / INV-STRAT-02 can keep claiming “minimums” while CI only proves positivity.

---

## 2. What this file does *not* ask for

Clone-prevention (already owned; adding more is noise):

| Existing pin | Owner | Why not clone |
|---|---|---|
| `test_swap_tokens_handles_zero_output_from_router` | A056 | Zero ≠ dust-one |
| `BadMode::{OverPull,UnderPull,Refund,OutputShortfall}` | `strategy/router.rs` | No **PayDust** mode that delivers `1` |
| `swap_collateral_preserves_directional_bounds`, `swap_*_rejects_same_token`, `post_gate_swap_*_totals_are_final` | Certora strategy/health | Directional / HF, not min-out |
| `prop_swap_collateral_conserves_position_delta` | harness fuzz | Honest router; `min_out_raw` paid exactly |
| `usage_*_tracks_scaled_delta` (ordinary/strategy/liq) | Certora V-5 / A085 | Per-leg delta, not P-GLOBAL |
| `usage_exit_without_usage_row_is_a_noop` | Certora + unit | **Keep** as PIN of current spec |
| `exit_without_usage_row_is_noop_and_does_not_persist` | unit `spoke.rs` | Same |
| `repeated_loops_never_extract_value` (GH-10) | composition | Pool-share proxy, healthy rows |
| `no_seize_blocks_the_seizure_leg_in_both_modes`, `no_seize_does_not_block_ordinary_withdrawal` | harness | Seizure halt, not supply growth |
| `no_seize_does_not_block_entry_or_exit` | unit `flags.rs` | Policy matrix; not e2e supply-then-stuck-liq |
| `set_spoke_asset_flags_tightens_no_seize_independently` | unit `asset_flags.rs` | **Pins the footgun**; CLOSE replaces it after Option C |
| `a_spoke_at_its_supply_cap_can_still_be_credited` | A084 | Anti-regression for fee-only; keep |
| `view_input_bound_rejects_oversized_asset_vectors` | A008 | Views only |
| `test_*_fee_on_transfer_*` (multiply debt, flash loan/position) | A055/A043/A045 | SAC FoT, not rebase/liar |
| `test_flash_position_rejects_below_minimum` | A045 | Pattern to **mirror**, not duplicate |
| `audit_supply_stale_shield` / `audit_liquidate_and_clean_bricked_by_unpriceable_dust_leg` | A065 | Plant-stale already evidenced |
| `supply_new_slot_requires_owner_or_delegate` | Certora market-guard | Supply-slot only (A003 skew) |
| `iso_update_indexes_writes_no_controller_state` | Certora index | Effect, not Vec length |
| `threshold_update_min_hf_is_one_point_zero_five_wad` | A015 | Floor, not Vec |

---

## 3. Existing coverage (so proposed names are deltas)

### 3.1 G-SLIP — A048 / A056 / A101 L1 / A106 S1

| Layer | Present | Bound |
|---|---|---|
| Harness | OverPull / Refund / UnderPull / OutputShortfall (zero out); underspend refunds to caller on multiply / swap_collateral / RDWC | Input + zero-out |
| Harness happy | `test_swap_collateral_replaces_supply`, `test_swap_collateral_no_borrows`, `test_swap_collateral_no_borrows_skip_hf` | Honest route; debt-free **skip HF** (the stickiness precondition) |
| Mock | `MockAggregator` pays **exactly** `payload.min_out`; `BadMode::OutputShortfall` pays **nothing** | No “pay 1 vs large min” |
| Honest aggregator | Unit `SlippageExceeded` on `total_out < total_min_out` | Untrusted Wasm; controller never sees it |
| Certora | Directional scaled bounds; same-token revert; post-gate finals | **No** `received ≥ min_out` |
| Fuzz | Exact position Δ under honest swap | Does not attack the floor |

**A056 explicit miss:** “No harness asserting measured out ≥ caller min on controller (mock pays min_out; BadMode has no ‘pay 1 vs large min’).”

### 3.2 A080 — missing-row exit (A103 / A085)

| Layer | Present | Bound |
|---|---|---|
| Unit | `exit_without_usage_row_is_noop_and_does_not_persist`; entry creates row | Spec of no-op |
| Certora | `usage_exit_without_usage_row_is_a_noop`; `assume_usage_seeds` allows `usage ≥ position` | Tolerance pin; **not** Σ equality |
| Harness caps | `spoke_caps.rs` / `spoke.rs` fill-to-cap on **healthy** usage rows | Never plants a missing row then re-fills |
| Liq | Credit fee usage; cap-at-limit Credit still works | Healthy rows |
| Composition | GH-10 vs **pool** supplied/borrowed | No planted hole |

A085 P1 already named the impact demo and global assert; §4.2 **gives those names** and adds Credit/bad-debt/borrow-side variants A085 left generic.

### 3.3 A064 G1 — `no_seize` coupling

| Layer | Present | Bound |
|---|---|---|
| Unit flags | Full FreezePolicy matrix including `no_seize_does_not_block_entry_or_exit` | Helper, not e2e supply |
| Unit ratchet | `set_spoke_asset_flags_tightens_no_seize_independently` | Guardian **can** set without freeze — **this is the gap pin** |
| Harness | Seizure blocked both modes; ordinary withdraw OK; paused collateral still seizable | No “supply then liquidate fails then supply more” |
| Integration | `liquidation.sh` sets `no_seize` | Halt seizure; not growth |
| Certora | Fixtures force `no_seize: false` | **No** `enforce_spoke_asset_flags` rule (A064 §8) |

### 3.4 A055 — lying / non-SAC

| Layer | Present | Bound |
|---|---|---|
| Harness FoT | Multiply debt fail-closed; flash position min-miss / net credit; flash loan fail-closed; outbound measurement | Tax tokens, SAC-shaped |
| Builder | `with_fee_on_transfer_market` | No with_rebasing_market / with_lying_balance_market |
| A042 residual | Strategy withdraw measures Δ, no `Δ == pool.actual_amount` | No named FoT-on-swap_collateral-withdraw test |

### 3.5 A015 / A062 — Vec hygiene

| Layer | Present | Bound |
|---|---|---|
| Views | `view_input_bound_rejects_oversized_asset_vectors`; `MAX_VIEW_INPUTS = 256` | Views only |
| Payments | Unit aggregate sum + overflow; harness duplicate supply/borrow/repay/withdraw/liquidate | Intentional **sum**, not length |
| Flash / migrate debt | Hard reject dupes + flash length ≤ `max_supply_positions` | Strongest list hygiene |
| Keepers | Auth, pause, flash, revenue→accumulator, 1.05 HF floor | **No** len/empty tests |
| Fuzz | `privileged_auth_rejects` empty keeper Vecs **succeed** as no-ops | Documents current empty-OK |

### 3.6 Syntheses A101–A106 → evidence asks

| Source | Ask of A108 |
|---|---|
| A056 / A101 | Controller dust-out vs large payload `min_out` |
| A103 / A085 | Global usage ↔ Σ positions; planted missing-row over-admission |
| A102 / A105 | Option C setter pins; (A102 also Vec caps, plant-stale — latter **already** has `audit_supply_stale_shield`) |
| A106 | Dust-out stickiness by path (swap_collateral > multiply spare HF ≫ swap_debt / RDWC) |
| A104 | `put_market_index` checklist (static more than runtime) |
| A110 RB-12 | Same four rows this file now owns with names |
| A003 | Certora owner-or-delegate on the eight mutators, not only supply-slot |
| A101 §8.3 | A060 still unfiled — do not treat G-DUST tests as closing it |

---

## 4. Named catalog — harness / unit (primary)

Each row is one **proposed** function or test name. `When` = PIN / CLOSE / REGRESS. `Fix` = A110 item if CLOSE.

### 4.1 G-SLIP — strategy dust-out (highest money residual)

Need a new mock arm (name only): **`BadMode::PayDust`** — pull full `amount_in`, transfer **1** unit of `token_out`, ignore `payload.min_out`, return a lie. Distinct from `OutputShortfall` (pays 0 → already `#502`).

#### 4.1.1 PIN — current controller *accepts* dust-out

| Name | Layer | Property |
|---|---|---|
| test_swap_collateral_pay_dust_succeeds_when_debt_free | harness `strategy/router.rs` | Debt-free account: PayDust spends full withdrawn `token_in`, credits 1 `token_out`, **succeeds**; post-gate skipped (`test_swap_collateral_no_borrows_skip_hf` precondition). Documents A106 S1 debt-free bound. |
| test_swap_collateral_pay_dust_succeeds_with_spare_hf | harness | Indebted account with spare HF: PayDust **succeeds**; HF ≥ 1 after; source supply down, dest dust up. Stickiest levered path (A048/A056 §6.2). |
| test_verify_router_output_accepts_unit_dust_despite_large_payload_min_out | harness | Same PayDust; payload `min_out` ≫ 1; controller still settles. Proves aggregator floor is **not** a controller defense. |
| test_multiply_pay_dust_succeeds_with_large_initial_collateral | harness | Spare HF / large payment: dust debt→collateral swap **sticks** (A056 table). |
| test_swap_collateral_payload_min_out_one_is_self_authorized_max_slippage | harness | Honest `MockAggregator` + caller `min_out = 1` + large `amount_in` → success. Distinct adversary: user/quote, not router owner. |

#### 4.1.2 PIN — current controller *rejects* dust-out via HF (path ranking)

| Name | Layer | Property |
|---|---|---|
| test_multiply_pay_dust_on_bare_new_debt_reverts_health_factor | harness | New account, no spare collateral → `HealthFactorTooLow` / insufficient collateral. Pins “usually reverts” (A056). |
| test_swap_debt_pay_dust_reverts_health_factor | harness | Full new debt + dust repay of old → revert. |
| test_repay_debt_with_collateral_pay_dust_reverts_health_factor | harness | Cross-asset RDWC: withdraw valuable coll, dust repay → revert. |

#### 4.1.3 CLOSE — after controller `min_out` (A110 RB-03)

Ship on the same PR as `verify_router_output` / entrypoint floor. Prefer explicit arg names matching `flash_position`’s `min_amount`.

| Name | Layer | Property |
|---|---|---|
| test_swap_collateral_rejects_pay_dust_below_controller_min_out | harness | Controller `min_out` ≫ 1; PayDust → new error (not `#502` zero). Independent of payload. |
| test_multiply_rejects_pay_dust_below_controller_min_out | harness | Same on multiply (+ `convert_swap` if distinct). |
| test_swap_debt_rejects_pay_dust_below_controller_min_out | harness | Floor fires **before** HF (dust would have reverted HF anyway; still pin the new code path). |
| test_repay_debt_with_collateral_rejects_pay_dust_below_controller_min_out | harness | Cross-asset only. |
| test_swap_tokens_controller_min_out_inclusive_boundary | harness | Measured Δ == `min_out` accepts; Δ == `min_out - 1` rejects. Mirror flash `test_flash_position_rejects_below_minimum`. |
| test_same_asset_passthrough_ignores_min_out_or_requires_empty_swap | harness | Cross-hub passthrough: no router; `min_out` either N/A or must equal `amount_in`. Do not apply aggregator semantics. |
| test_honest_aggregator_still_passes_when_measured_meets_controller_min_out | harness | Regression: RB-03 must not break happy `test_swap_collateral_replaces_supply`. |

#### 4.1.4 Unit / mock

| Name | Layer | Property |
|---|---|---|
| verify_router_output_rejects_non_positive | unit (if extracted) | Today’s `#502` pin next to the new ≥ `min_out` assert — avoid losing positivity. |
| bad_mode_pay_dust_delivers_one_and_spends_full_in | harness mock unit | PayDust contract behavior, isolated from strategies. |

### 4.2 A080 — missing-row over-admission (highest T5 residual)

A085 already asked for “plant missing row + exit + fill to cap” and “usage vs Σ scaled”. Names below are the concrete catalog.

#### 4.2.1 PIN — blast radius on current no-op

| Name | Layer | Property |
|---|---|---|
| test_missing_usage_row_withdraw_then_supply_fills_to_configured_cap | harness `spoke_caps.rs` or new `spoke_usage_desync.rs` | Seed live supply **without** `SpokeUsage` row (or delete row via test-only storage poke). Withdraw no-ops usage. Second user supplies up to **full** `supply_cap` as if occupancy were 0. Asserts A103 §5.1 over-admission. |
| test_missing_usage_row_repay_then_borrow_fills_to_configured_borrow_cap | harness | Same on borrow side. |
| test_missing_usage_row_partial_withdraw_does_not_create_usage_row | unit/harness | Non-zero exit + missing row → storage still `None` (extends unit no-op to e2e). |
| test_missing_usage_row_credit_fee_exit_does_not_heal_headroom | harness | Credit liq with missing usage: fee `apply_spoke_exit` no-ops; subsequent supply still sees full cap (A084 compounding). |
| test_missing_usage_row_bad_debt_cleanup_does_not_heal_headroom | harness | `clean_bad_debt` / force-socialize exits no-op; cap headroom remains overstated (A027 residual). |
| test_orphaned_usage_above_positions_false_cap_hit | harness | Dual: `U > P` → entry `#SpokeSupplyCapReached` while true occupancy lower (A103 §5.2). PIN of over-count, not just under-count. |

#### 4.2.2 CLOSE — after reconcile / invariant (A110 RB-06)

| Name | Layer | Property |
|---|---|---|
| test_reconcile_spoke_usage_rewrites_from_account_scaled_maps | harness | Permissioned reconcile_spoke_usage (or whatever ships): after plant-desync, usage == Σ account scaled per `(spoke, hub, side)`. |
| test_reconcile_spoke_usage_is_owner_or_keeper_gated | harness | Stranger cannot rewrite caps. |
| test_reconcile_spoke_usage_no_op_when_already_matched | harness | Idempotent. |
| test_usage_keeper_assert_reverts_on_undercount | harness | If product is fail-closed monitor rather than silent rewrite. |

#### 4.2.3 REGRESS — persist / multi-key (A078 / A079; A085 P2–P3)

| Name | Layer | Property |
|---|---|---|
| test_bulk_supply_two_assets_moves_distinct_usage_keys | harness | `supply([(A,x),(B,y)])` → each `SpokeUsage` key Δ matches that asset’s scaled Δ (A079 / A085 P-MULTI). |
| test_bulk_borrow_two_assets_moves_distinct_usage_keys | harness | Borrow twin. |
| persist_spoke_usage_call_sites_follow_pool_success | unit comment-test or `scripts/` grep gate | Named anti-regression for A078; A085 noted this as structural-only today. |

Do **not** add a test that “fixes” missing-row by inventing a zero row on exit unless product changes — that would fight `usage_exit_without_usage_row_is_a_noop`.

### 4.3 A064 G1 — `no_seize` without freeze / supply still open

#### 4.3.1 PIN — today’s footgun (e2e)

| Name | Layer | Property |
|---|---|---|
| test_no_seize_without_frozen_still_allows_supply | harness `liquidation_seize_modes.rs` or `spoke.rs` | After `set_spoke_asset_flags(..., no_seize=true, frozen=false)`, `supply` of that asset **succeeds**. Unit helper already implies this; e2e does not. |
| test_no_seize_new_supply_grows_unliquidatable_set | harness | Account A underwater on `no_seize` coll; account B **newly supplies** same asset; both liquidations revert `#318`; B added *after* the flag. Documents growth (A102 §4.1). |
| test_no_seize_blocks_whole_liquidation_if_any_seize_leg_halted | harness | Multi-coll account; one `no_seize` leg → entire tx reverts (may exist as `no_seize_blocks_the_seizure_leg_in_both_modes` — if that suite is single-asset only, add this **multi-leg** name). |
| test_force_socialize_bad_debt_recovers_no_seize_stranded_insolvent | harness | Hatch path: seize blocked, owner socialize still works (A064 operator hatch). |

Keep `set_spoke_asset_flags_tightens_no_seize_independently` until Option C; then replace (do not leave both green).

#### 4.3.2 CLOSE — ADR-0008 Option C (A110 RB-05)

Pick names matching the shipped predicate (`no_seize ⇒ frozen` and/or `require_can_supply` rejects `no_seize`).

| Name | Layer | Property |
|---|---|---|
| test_set_spoke_asset_flags_rejects_no_seize_unless_frozen | unit `asset_flags.rs` | Setter coupling. Replaces `…_tightens_no_seize_independently`. |
| test_require_can_supply_rejects_no_seize | unit `flags.rs` | If entry policy changes. |
| test_no_seize_cannot_grow_supply_after_option_c | harness | Inverse of PIN growth test. |
| test_option_c_still_allows_seizure_halt_on_already_frozen_listing | harness | Frozen + `no_seize` remains a valid seize halt; exits still `AllowOnExit` for freeze. |
| test_paused_still_does_not_block_seizure_after_option_c | harness | **Anti-fix:** Option C must not reintroduce Aave CS-AAVE4-002 (`paused` on seize). |

#### 4.3.3 REGRESS — Credit delist / helper pairing (A064 G2–G3, low)

| Name | Layer | Property |
|---|---|---|
| test_credit_seize_delisted_asset_onto_empty_receiver_reverts_listing | harness | `#307` on new Credit slot (A052); Transfer still works. |
| enforce_spoke_asset_flags_block_on_entry_without_listing_stays_noop | unit | Keep G3 pin: future callers must not drop `require_listed_*`. |

### 4.4 A055 — non-SAC / lying / rebasing (listing residual)

FoT-on-SAC is already covered. Missing is the **listing-trust failure demo** and strategy-withdraw FoT (A042 residual (2)).

Need builder mocks (names only): **with_rebasing_market**, **with_balance_lying_market** (balanceOf ≠ sum of transfers).

#### 4.4.1 PIN — impact demos (may be `#[ignore]` if too hostile for default CI)

| Name | Layer | Property |
|---|---|---|
| test_listed_rebasing_collateral_desyncs_pool_cash_versus_shares | harness | Rebase mid-hold → supplier claims vs cash diverge; bound discussion ≤ TVL_m (A101 L2). Documents why listing is a security boundary. |
| test_balance_lying_token_can_inflate_measured_delta | harness | Token reports larger `balance` than transferred; show where Δ oracle breaks (A058 Gap). |
| test_swap_collateral_fot_withdraw_shrinks_swap_budget_after_gross_burn | harness | FoT on strategy withdraw-to-controller: shares burned gross, swap `amount_in` = measured Δ < gross (A042/A048). |
| test_liquidation_transfer_fot_haircuts_liquidator_not_share_mint | harness | Transfer seize FOT: liquidator short; books already debited net (A051). |
| test_credit_on_transfer_donation_does_not_mint_supply_shares | harness | Unexpected inbound during measured path does not credit shares (A058 dust hygiene). |

#### 4.4.2 CLOSE / ops — listing policy (A110 RB-04)

Mostly runbook, not Rust. Optional code gates:

| Name | Layer | Property |
|---|---|---|
| test_is_flashloanable_rejected_or_documented_for_fot_market | harness / ops | A044: never flashloanable on non-exact. If no code gate, this is a **checklist item**, not a test. |
| listing_checklist_sac_only | docs/runbook assert in `/deploy-preflight` | Not a `#[test]`; A108 records the evidence hole. |

#### 4.4.3 CLOSE optional — strategy withdraw equality (A110 RB-09)

| Name | Layer | Property |
|---|---|---|
| test_withdraw_collateral_to_controller_rejects_when_measured_exceeds_or_ne_pool_gross | harness | Only if equality/`measured ≤ gross` ships. |

### 4.5 A015 / A062 — Vec length (low, high fixability)

#### 4.5.1 CLOSE — after MAX_KEEPER_INPUTS / mutator cap (A110 RB-07)

| Name | Layer | Property |
|---|---|---|
| require_keeper_inputs_bound_rejects_oversized_vectors | unit (twin of `view_input_bound_rejects_oversized_asset_vectors`) | Shared helper. |
| test_update_indexes_rejects_len_above_max_keeper_inputs | harness `keeper.rs` | Before pool loop. |
| test_claim_revenue_rejects_len_above_max_keeper_inputs | harness | Same; revenue still cannot redirect (A015). |
| test_update_account_threshold_rejects_len_above_max_account_ids | harness | Same. |
| test_supply_rejects_len_above_max_payment_inputs | harness | Optional mutator cap **before** aggregate. |
| test_borrow_rejects_len_above_max_payment_inputs | harness | Twin. |
| test_liquidate_rejects_len_above_max_view_inputs | harness | Align with `get_liquidation_estimate` 256 (A062 G-VAL-3). |
| test_update_indexes_empty_vec_policy | harness | Product: keep no-op **or** fail-loud; pin whichever ships. |
| test_claim_revenue_duplicate_hubs_second_returns_zero | harness | Hygiene; money already accumulator-only. |

Do **not** add test_supply_rejects_duplicate_hub_keys — that would fight documented aggregate-and-sum (`aggregate_payments_dedups_and_preserves_order`).

#### 4.5.2 REGRESS — lists already strong

| Name | Layer | Property |
|---|---|---|
| test_flash_position_rejects_refund_assets_over_max_supply_positions | harness | A070 over-length (A102 G-VAL-7; A110 RB-12 optional). |
| test_flash_position_refund_listing_keyed_by_debt_hub | harness | Multi-hub keying nit (A070). |
| test_migrate_rejects_duplicate_collateral_or_supply_assets | harness | Only if product switches soft-dedup → hard reject (A062 G-VAL-4). |

### 4.6 Adjacent pins peers already routed to A108

#### 4.6.1 A003 — INV-AUTH-02 Certora skew (info; runtime defended)

Harness already has `test_*_wrong_account_owner`. Missing are **prover** rules (see §5.4). Optional harness completeness:

| Name | Layer | Property |
|---|---|---|
| test_flash_position_wrong_account_owner | harness | If not already present under another name. |
| test_migrate_from_blend_wrong_account_owner | harness | Same. |

#### 4.6.2 A009 — Sensitive floor (deploy P0; not a controller unit)

| Name | Layer | Property |
|---|---|---|
| sensitive_delay_floor_is_production_sized | unit `contracts/governance/tests/` | **CLOSE/xfail today:** `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS == 120_960`. Today it is `12` with TEMPORARY comment. Flips green on UpgradeGov restore (A110 RB-01). |
| timelock_sensitive_floor_is_temporary_twelve | unit PIN | Inverse pin **until** restore, so the TEMPORARY value cannot silently change to something else. Replace with the production pin on the same PR. |

#### 4.6.3 A094 / A104 — forgotten `put_market_index`

Runtime tests cannot easily omit a call that current merges always make. Prefer static:

| Name | Layer | Property |
|---|---|---|
| merge_helpers_call_put_market_index | `make` / `scripts/` static gate | Every `merge_*_leg` / new pool-mutation merge contains `put_market_index` (+ `apply_leg_usage`). |
| forgotten_put_market_index_stales_post_gate_hf | Certora/harness **only if** a test hook can skip the put | Documents A106 S10; skip unless hook exists — do not weaken production to test it. |

#### 4.6.4 A067 — Certora floor witness (A102 / A110 RB-10)

| Name | Layer | Property |
|---|---|---|
| min_borrow_collateral_floor_nonzero_blocks_origination | Certora | Fixtures today force floor `0`; add a **non-zero** witness rule (A067). |

#### 4.6.5 A060 — still unfiled (do not pretend G-DUST closes it)

If/when A060 files, expected names (placeholders):

| Name | Layer | Property |
|---|---|---|
| test_cross_asset_dust_does_not_open_bad_debt_gate | harness | Multi-asset dust vs `BAD_DEBT_USD_THRESHOLD` (A101 §8.3). `dust_threshold_and_decimal_floor.rs` is **single-market** probes — not this. |

---

## 5. Named catalog — Certora rules

New rules need conf/profile budget (CLAUDE.md: renaming/adding rules invalidates tuned CI). Propose **few, high-leverage** assert rules plus reachability twins only where V-5 already requires them.

### 5.1 G-SLIP (only meaningful after RB-03, except one PIN)

| Rule name | After? | Property |
|---|---|---|
| verify_router_output_requires_strictly_positive | PIN now | Current `#502` as a rule on a harness-level summary, if `verify_router_output` is in-spec. Optional — harness already owns it. |
| swap_measured_out_meets_controller_min_out | **CLOSE** RB-03 | Success ⇒ `token_out` Δ ≥ symbolic `min_out`. A056: “no rule that measured out ≥ symbolic min”. |
| swap_collateral_rejects_when_measured_out_below_min_out | CLOSE | Revert path. |
| multiply_rejects_when_measured_out_below_min_out | CLOSE | Same. |
| swap_passthrough_same_asset_does_not_require_router_min_out | CLOSE | Empty swap / equal Address. |

Do **not** encode “dust-out succeeds” as a Certora **assert** of theft — that would freeze the residual. PIN dust-out in **harness** (§4.1.1) instead.

### 5.2 A080 / P-GLOBAL (A085 highest formal miss)

| Rule name | After? | Property |
|---|---|---|
| usage_equals_sum_of_seeded_account_scaled | CLOSE or new fixture | For the **exercised** accounts in-spec (not all chain accounts): `SpokeUsage.supplied == Σ supply_scaled` (debt twin). Requires dropping or tightening `assume_usage_seeds` (`usage ≥ position`). |
| usage_undercount_admits_entry_up_to_configured_cap | PIN (hard) | If cap fixtures stay `UNCONSTRAINED_CAP`, this cannot fire — needs a **constrained-cap** conf (A085 P2). Alternative: leave to harness §4.2.1. |
| usage_cap_revert_when_entry_exceeds_scaled_cap | REGRESS | A085: **no** Certora cap-revert rule. One supply + one borrow. New conf, not stuffed into `spoke-usage.conf`. |
| `usage_exit_without_usage_row_is_a_noop` | **keep** | Do not weaken until RB-06 changes semantics. If exit starts creating rows, **replace** this rule in the same PR. |

Suggested conf names: `spoke-usage-global.conf`, `spoke-usage-caps.conf` (A085 implied; name them here).

### 5.3 A064 FreezePolicy / Option C

A064: “No Certora rules call `enforce_spoke_asset_flags` by name; coverage is unit + harness — acceptable for discrete flags.” Still useful **after** Option C:

| Rule name | After? | Property |
|---|---|---|
| no_seize_implies_frozen_in_stored_spoke_asset | CLOSE Option C | Storage invariant on setter success. |
| seize_halted_listing_rejects_new_supply | CLOSE if entry policy changes | `require_can_supply` fail. |
| paused_listing_does_not_block_seizure_leg | REGRESS | Anti-Aave-halt; encode `SeizureLeg` ignores `paused`. Discrete; optional. |

### 5.4 A003 — owner-or-delegate on eight mutators

| Rule name | After? | Property |
|---|---|---|
| borrow_requires_owner_or_delegate | REGRESS | Stranger caller, existing account → revert. Parallel to `supply_new_slot_requires_owner_or_delegate`. |
| withdraw_requires_owner_or_delegate | REGRESS | Same. |
| multiply_requires_owner_or_delegate | REGRESS | Same. |
| swap_debt_requires_owner_or_delegate | REGRESS | Same. |
| swap_collateral_requires_owner_or_delegate | REGRESS | Same. |
| repay_with_collateral_requires_owner_or_delegate | REGRESS | Same. |
| flash_position_requires_owner_or_delegate | REGRESS | Same. |
| migrate_from_blend_requires_owner_or_delegate | REGRESS | Same. |

Runtime harness already covers several wrong_account_owner cases; this is **prover coverage skew**, not a live auth hole (A003 Opinion).

### 5.5 A015 keeper effects (already partly proved)

| Rule name | After? | Property |
|---|---|---|
| claim_revenue_never_pays_caller | REGRESS optional | If keepers enter spec: payout address == accumulator. Harness already strong. |
| full_tuple_threshold_update_preserves_hf_ge_1_05 | REGRESS optional | Dual gate; harness-owned today. |

Vec length is a poor Certora target; keep it in unit/harness §4.5.

### 5.6 A055

Do **not** add Certora rules that assume SAC-lying tokens unless the harness models them. Listing trust is ops + PIN demos.

---

## 6. Named catalog — fuzz / proptest / integration / static

### 6.1 Fuzz / proptest

| Name | After? | Property |
|---|---|---|
| prop_swap_collateral_pay_dust_bounded_by_hf_or_full_leg_if_debt_free | PIN | Random spare HF vs debt-free; PayDust; assert loss ≤ A106 formulas (no share mint). |
| prop_swap_rejects_below_controller_min_out | CLOSE RB-03 | Measured Δ < min → always Err. |
| prop_spoke_usage_matches_sum_of_scaled_positions | CLOSE RB-06 or PIN slack=0 markets | Multi-account random supply/borrow/repay/withdraw; usage == Σ maps. **Not** in `tests/fuzz` today (A085). |
| prop_missing_usage_row_allows_cap_refill | PIN | Occasional planted hole → fill-to-cap possible. |
| prop_keeper_vec_above_max_always_rejects | CLOSE RB-07 | Random oversized `assets` / `account_ids`. |
| prop_payment_vec_above_max_always_rejects | CLOSE if mutator cap ships | Same. |
| prop_no_seize_supply_rejected_after_option_c | CLOSE RB-05 | Flag matrix fuzz. |

Do not add a fuzz target that **requires** aggregate-and-sum to become reject-duplicates.

### 6.2 Integration (`tests/integration/flows/`)

| Name | After? | Property |
|---|---|---|
| `strategy.sh` / new inv_swap_collateral_pay_dust | PIN | WASM-level PayDust if mock router installable. |
| inv_no_seize_supply_still_works | PIN | Complement `sf_set_no_seize` in `liquidation.sh`. |
| inv_no_seize_requires_frozen | CLOSE Option C | Inverse. |
| inv_keeper_oversized_vec_rejects | CLOSE RB-07 | CLI/resource. |

### 6.3 Static / make gates

| Name | After? | Property |
|---|---|---|
| `make merge-index-overwrite-check` | REGRESS A094 | `put_market_index` on every merge helper. |
| `make sensitive-delay-production-check` | CLOSE RB-01 | Fails while floor == 12; used as release blocker, not PR-to-`main` until restore. |
| `scripts/permissionless_entrypoints.txt` | n/a | Unchanged; keepers stay declared. Vec cap is body validation, not a new ungated verb. |

---

## 7. Priority matrix (tests/rules only)

Aligned with A106 / A110 **evidence** order, not deploy ops (ownership attestation is not a `#[test]`).

| P | Band | Artifacts (short) | Closes evidence for |
|---|---|---|---|
| **P0** | PIN + CLOSE pair | §4.1 PayDust PIN on `swap_collateral` debt-free/spare-HF; CLOSE `*_rejects_pay_dust_below_controller_min_out`; Certora swap_measured_out_meets_controller_min_out | A048/A056/A101 G-SLIP / A106 S1 / STRIDE Tamper.4 |
| **P1** | PIN blast + CLOSE reconcile | §4.2 missing-row fill-to-cap (supply **and** borrow); Credit-fee / bad-debt no-heal; Certora global equality **or** harness Σ; reconcile tests when API exists | A080/A103/A085 P-GLOBAL / A106 S6 |
| **P1** | PIN growth + CLOSE Option C | §4.3 test_no_seize_without_frozen_still_allows_supply + growth; setter CLOSE; paused-does-not-block-seize REGRESS | A064/A102 G-VAL-1 / A106 S7 |
| **P2** | PIN listing demos | §4.4 rebase/liar/FoT-on-swap_collateral-withdraw | A055/A101 G-LIST / A042 residual |
| **P2** | CLOSE Vec caps | §4.5 keeper three verbs + optional mutator/`liquidate` 256 | A062/A015 / A106 S8 |
| **P3** | Certora auth skew | §5.4 eight `*_requires_owner_or_delegate` | A003 → A108 tracking |
| **P3** | Static A094; Sensitive constant PIN/CLOSE | §4.6.2–4.6.3 | A009/A094/A110 RB-01/RB-08 |
| **P3** | Caps Certora; bulk two-key usage; A067 floor witness; A070 refund length | §4.2.3, §5.2, §4.5.2, §4.6.4 | A079/A085/A067/A070 |
| **P4** | A060 placeholder; listing runbook | §4.6.5, §4.4.2 | Unfiled / ops |

**P0 test work is blocked on a PayDust mock** (small harness-only change) for PIN; CLOSE P0 is blocked on RB-03 product.

---

## 8. Cross-links and corpus hygiene

### 8.1 Agreement with peers

| Peer | A108 stance |
|---|---|
| A056 F1 / §10.6 | Owns the dust-vs-min miss — **named** here |
| A048 | Highest stickiness on `swap_collateral` — PIN tests start there |
| A080 / A103 | Capacity residual; Certora no-op is spec, not a close |
| A085 | P1/P2 backlog **adopted and named**; multi-key + cap-revert Certora kept P3 |
| A064 / A102 | Option C tests; do not gate seize on `paused` |
| A055 / A101 G-LIST | FoT covered; rebase/liar not |
| A015 / A062 | Length CLOSE; do not reject payment dupes |
| A101 §7.2 | Framing: custody defended ≠ slippage evidenced |
| A105 / A107 Tamper.4 | RAISE at controller — PIN PayDust is the evidence that rating needs |
| A106 S1 path table | PIN success vs PIN HF-revert split matches S1 |
| A110 RB-12 | This file **is** the missing A108 those rows deferred to |

### 8.2 A110 snapshot note

A110 §0.4 listed A108 as **absent** and inferred RB-12. This file **fills that debt**. A110 need not be rewritten here (COORDINATION: do not edit other agents’ files). A110 rank 11 “Tests for dust-out…” is the same P0/P1 catalog.

### 8.3 No disagreement file

No peer claims these tests already exist. A085’s generic “harness: missing usage row + exit + subsequent supply fills to cap” is the same artifact as test_missing_usage_row_withdraw_then_supply_fills_to_configured_cap.

---

## 9. Anti-catalog (do not add)

| Tempting test/rule | Why not |
|---|---|
| test_supply_rejects_duplicate_legs | Breaks endpoints.md §6 aggregate-and-sum |
| test_paused_collateral_cannot_be_seized | Inverse of INV-HALT-02 / A064; would encode the Aave-class halt |
| Weaken `usage_exit_without_usage_row_is_a_noop` without RB-06 | Loses the specification pin |
| Certora assert that dust-out **succeeds** forever | Freezes G-SLIP as intended behavior |
| Mid-tx oracle refresh test “to catch stale prices” | Violates ADR-0005 (A104 anti-remediation) |
| Persist-usage-before-pool “optimization” test | Violates A078 |
| Recipient-Δ on ordinary withdraw | Violates ADR-0013 / A042 |
| More `usage_*_tracks_scaled_delta` clones | V-5 already exhaustive via `usage_coverage_no_unwired_verb` |
| Cap-revert Certora inside unconstrained `spoke-usage.conf` | Would vacate existing rules (fixture comment) |

---

## 10. Mapping: residual → first test to write

If only **one** artifact per residual is affordable:

| Residual | Write first | Why that one |
|---|---|---|
| A048/A056 | test_swap_collateral_pay_dust_succeeds_when_debt_free | Largest A106 account bound; no HF clip; Tamper.4 counterexample |
| A080 | test_missing_usage_row_withdraw_then_supply_fills_to_configured_cap | Converts A080 prose into a numeric cap-refill |
| A064 | test_no_seize_without_frozen_still_allows_supply | e2e of G1; setter unit already independent |
| A055 | test_swap_collateral_fot_withdraw_shrinks_swap_budget_after_gross_burn | Uses existing FoT builder; no new rebase mock |
| A015/A062 | test_update_indexes_rejects_len_above_max_keeper_inputs | Smallest CLOSE once RB-07 lands; three keepers share a helper |
| A003 | borrow_requires_owner_or_delegate | Highest-risk mutator not in current Certora auth rule |
| A009 | timelock_sensitive_floor_is_temporary_twelve then flip to production pin | Deploy P0 is otherwise untested as a constant |

After RB-03, the **first CLOSE** is test_swap_collateral_rejects_pay_dust_below_controller_min_out — same scenario as the P0 PIN, inverted.

---

## 11. Suggested file placement (for implementers; not created here)

| New tests live in | Why |
|---|---|
| `tests/test-harness/tests/strategy/router.rs` | PayDust next to `BadMode` / `OutputShortfall` |
| `tests/test-harness/src/mock_aggregator.rs` | `BadMode::PayDust` |
| `tests/test-harness/tests/controller/spoke_usage_desync.rs` | A080 plant-and-refill (keep `spoke_caps.rs` for healthy caps) |
| `tests/test-harness/tests/controller/liquidation_seize_modes.rs` | A064 growth / Option C |
| `tests/test-harness/tests/controller/keeper.rs` | Vec CLOSE |
| `contracts/controller/tests/config/asset_flags.rs` | Option C setter |
| `certora/controller/spec/strategy_rules.rs` + new conf | swap_measured_out_meets_controller_min_out |
| `certora/controller/spec/spoke_rules.rs` + `spoke-usage-global.conf` / `spoke-usage-caps.conf` | P-GLOBAL / P-CAP |
| `certora/controller/spec/market_guard_rules.rs` | eight owner-or-delegate rules |
| `contracts/governance/tests/timelock.rs` | Sensitive floor PIN/CLOSE |
| `tests/test-harness/tests/fuzz/strategy_router_invariants.rs` | PayDust / min_out props next to `prop_swap_collateral_conserves_position_delta` |
| `tests/test-harness/tests/fuzz/` new `spoke_usage_conservation.rs` | prop_spoke_usage_matches_sum_of_scaled_positions |

---

## 12. Verdict

**Status: partial** at the evidence layer — the controller’s *defended* money, cap-delta, and freeze-matrix stacks are well pinned; the *leading residuals* are not demonstrated as blast-radius tests and will not be locked after A110 fixes unless CLOSE twins ship on the same PRs.

1. **Highest missing test:** test_swap_collateral_pay_dust_succeeds_when_debt_free (+ `BadMode::PayDust`), then its CLOSE inverse under controller `min_out`.
2. **Highest missing usage evidence:** planted missing-row **cap refill** (A080), not another delta rule; keep `usage_exit_without_usage_row_is_a_noop` until reconcile changes the spec.
3. **Highest missing validation evidence:** e2e `no_seize` still allows **supply** and **grows** the unliquidatable set; Option C setter CLOSE; never pin `paused` as a seize halt.
4. **Highest missing listing evidence:** FoT-on-strategy-withdraw (existing mock) plus rebase/liar demos; SAC FoT fail-closed tests already exist.
5. **Vec / Certora auth / Sensitive floor / `put_market_index`:** hygiene and prover-skew, ranked below the four mediums.

**No novel Critical** is implied by missing coverage. The cost of not writing these names is that STRIDE Tamper.4, INV-STRAT-02, and INV-HALT-03 “VERIFIED” lines remain easy to over-read, and remediations can land without a red-to-green pair.

**Sources read:** `shared/{COORDINATION,SEED,AGENT_MANIFEST}.md`; README format; PRELIMINARY; findings A080, A048, A056, A055, A064, A015, A062, A101–A107, A085, A110 (adjacency); A003 Certora note; A042 residual (2); live `BadMode`, `usage_*`, `strategy_rules`, `flags.rs`, `asset_flags.rs`, `spoke.rs` unit, `keeper.rs`, `router.rs`.
