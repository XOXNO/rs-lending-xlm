# Controller liquidation and risk math (Aeneas + Lean)

Lean 4 proofs about the controller liquidation math and position valuation,
against the generated `lean/ControllerAeneas/Funs.lean`. Conventions:
[../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

| Rust file | Functions extracted | Status |
|---|---|---|
| `common/src/rates/value.rs` | `position_value`, `position_value_floor`, `position_value_ceil` | exact rounding + bracket proved |
| `common/src/math/fp.rs` | `Bps::apply_to_wad_floor` (and everything the functions below call) | exact floor proved |
| same (typed wrappers the liquidation math calls) | `Ray::mul_wad_ratio_floor`, `Wad::apply_ratio_to_amount_floor`, `Bps::from_fraction_ceil`, `Bps::apply_to_ray_ceil` | exact rounding proved |
| `contracts/controller/src/positions/liquidation/curve.rs` | `is_socializable_bad_debt`, `max_bonus_for_threshold`, `LiquidationCurve::bonus_scale`, `calculate_linear_bonus_with_target` | proved |
| same | `max_hf_preserving_bonus_bps`, `liquidation_at_target`, `estimate_liquidation_amount` | proved in [controller-estimate.md](controller-estimate.md) |
| `contracts/controller/src/positions/liquidation/math.rs` | `split_seized_shares`, `scale_seizures_to_received`, `sum_repaid_usd` | proved |
| same | `calculate_repayment_amounts` | proved (caps, exact refunds, exact USD total) |
| same | `calculate_seized_collateral` | seizure budget (first three steps) proved here; per-leg loop proved in [controller-seizure.md](controller-seizure.md) |
| same | `get_account_bonus_params`, `calculate_seizure_proportions` | proved in [controller-seizure.md](controller-seizure.md) |
| same | `normalize_repayment_plan`, `whole_unit_repayment`, `release_unbacked_repayment`, `process_excess_payment`, `one_unit_per_*` | proved in [controller-repayment.md](controller-repayment.md) |
| same | `repays_every_debt_leg` | kept opaque (closure with a mutable capture, [Aeneas issue 5](extraction.md#controller-aeneas-issues)) |
| `contracts/controller/src/positions/liquidation/apply.rs` | `apply_liquidation_repayments`, `credited_shares`, `check_bad_debt_after_liquidation` | proved |
| same | `apply_liquidation_seizures`, `apply_liquidation_share_credit`, `credit_supply_shares` | extracted, not proved (their observable effects are external calls) |
| `contracts/controller/src/risk/totals.rs` | `calculate_account_risk_totals`, `calculate_ltv_collateral_wad`, `sum_debt_usd` (and `sum_debt_usd_loaded`, `portfolio_hub_keys`) | proved (exact sums, INV-RISK-02 conservativeness) |

Every theorem is **total correctness** (`f args ⦃ r => P r ⦄`) over
unbounded `i128` inputs, subject only to its stated hypotheses. The
specifications `IsFloor`, `IsCeil` and `IsHalfUp` (`lean/FixedPoint/Spec.lean`,
`FloorCeilSpec.lean`, `HalfUpSpec.lean`) use multiplication only.

## Extraction and trusted base

Crate, models (`I256`, `Vec<T>`, Map, Context), copied proofs, rebuild and
Aeneas workarounds: [extraction.md](extraction.md#controller)
([trusted base](extraction.md#controller-trusted-base)).

## Supporting proof files

* `Model.lean`: Map/Context facts: `map_get_eq`/`mapLookup`,
  `cached_price_ok`, `cached_market_index_ok`, `load_markets_cached` (no
  change when every key is cached), `iter_typed_positions_spec`,
  `iter_debt_positions_spec`, `i128_from_u32_val`.
* `Basic.lean`: comparison/min/conversion lemmas for `Wad`/`Ray`/`Bps`,
  `checked_add_raw`, `checked_sub_nonneg`.
* `Rounding.lean`: division-free floor/half-up/ceiling relations.
* `Wrappers.lean`: typed `fp.rs` wrappers: unfolding equations to the raw
  `mul_div_*` call, exact-rounding specs, ratio-at-most-one lemmas
  `floor_ratio_fits`, `floor_ratio_le`, `ceil_ratio_le`.
* `Shares`, `Curve`, `Seizure` rewrite each wrapper call with its `*_eq`
  lemma, then use the raw `mul_div_*` spec.

Copied math proofs (`MulDiv`, `HalfUp`, `Scaling`, ...):
[extraction.md](extraction.md#controller-copied-proofs).

## Theorems

Notation: `R = 10^27` (RAY), `B = 10^9` (RAY→WAD factor), `W = 10^18` (WAD),
`BPS = 10^4`; `hu` = rounding to nearest, halves up.
Value-domain hypothesis **(VD)** for `scaled s`, `index i`, `price p`:
`s,i,p ≥ 0`, `s·i + R ≤ 2^127·R` and `(s·i + 2RB)·p + WRB ≤ 2^127·WRB`
(the exact value plus rounding slack fits `i128`).

| Lean name | File | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|---|
| `position_value_floor_spec` | `Proofs/Value.lean` | `common/src/rates/value.rs::position_value_floor` | returns `⌊⌊⌊s·i/R⌋/B⌋·p/W⌋` | VD | INV-RISK-02 |
| `position_value_spec` | `Proofs/Value.lean` | `value.rs::position_value` | returns `hu(hu(hu(s·i/R)/B)·p/W)` | VD | INV-RISK-02 |
| `position_value_ceil_spec` | `Proofs/Value.lean` | `value.rs::position_value_ceil` | returns `⌈⌈⌈s·i/R⌉/B⌉·p/W⌉` | VD | INV-RISK-02 |
| `position_value_bracket` | `Proofs/Value.lean` | all three | all three return and `floor ≤ half-up ≤ ceil` | VD | INV-RISK-02 |
| `position_value_floor_ceil_exact` | `Proofs/Value.lean` | floor, ceil | `floor·RBW ≤ s·i·p ≤ ceil·RBW`: floor never overstates and ceil never understates the exact value | VD | INV-RISK-02 |
| `apply_to_wad_floor_spec` | `Proofs/Value.lean` | `common/src/math/fp.rs::Bps::apply_to_wad_floor` | returns `⌊v·rate/BPS⌋` (single downward rounding; per-leg LTV/threshold weighting in `risk/totals.rs`) | `0 ≤ rate ≤ BPS`, `v ≥ 0` | INV-RISK-02 |
| `is_socializable_bad_debt_spec` | `Proofs/Curve.lean` | `liquidation/curve.rs::is_socializable_bad_debt` | never aborts; true iff `coll < debt ∧ coll ≤ 5 WAD` | none | INV-LIQ-04 |
| `max_bonus_for_threshold_spec` | `Proofs/Curve.lean` | `curve.rs::max_bonus_for_threshold` | `p ≤ 0` → 0; `p > 0` → `⌊BPS(BPS−e)/e⌋`, `e = clamp(⌈p·BPS/W⌉, 1, BPS)` | none (all `i128`) | INV-LIQ-02 |
| `max_bonus_for_threshold_bound` | `Proofs/Curve.lean` | same | `0 ≤ m ≤ BPS(BPS−1)` and `(BPS+m)·p ≤ BPS·W` (`(1+bonus)·proportion ≤ 1`) | `0 < p ≤ W` | INV-LIQ-02 |
| `max_bonus_for_threshold_antitone` | `Proofs/Curve.lean` | same | `0 < p₁ ≤ p₂` → `m(p₂) ≤ m(p₁)` | as stated | INV-LIQ-02 |
| `bonus_scale_spec` | `Proofs/Curve.lean` | `curve.rs::LiquidationCurve::bonus_scale` | returns `min(hu((target−hf)·W/(target−hf_max)), W)` | `0 ≤ hf_max < target ≤ 10 WAD`, `0 ≤ hf ≤ target` | — |
| `calculate_linear_bonus_spec` | `Proofs/Curve.lean` | `curve.rs::calculate_linear_bonus_with_target` | exact bonus `b` ([notes](#theorem-notes)); `base ≤ b ≤ max` | validated curve, `hf ≥ 0`, `0 ≤ base ≤ max` | INV-LIQ-02 |
| `calculate_linear_bonus_antitone` | `Proofs/Curve.lean` | same | `0 ≤ hf₁ ≤ hf₂` → `bonus(hf₂) ≤ bonus(hf₁)` | as above | INV-LIQ-02 |
| `calculate_linear_bonus_max_below_floor` | `Proofs/Curve.lean` | same | `hf ≤ hf_max`, factor = BPS → bonus = `max` | as above | INV-LIQ-02 |
| `split_seized_shares_spec` | `Proofs/Shares.lean` | `liquidation/math.rs::split_seized_shares` | `fee = ⌈b·f/BPS⌉`, `fee + liq = S` exactly, both in range ([notes](#theorem-notes)) | `0 ≤ b ≤ S`, `f < BPS` | INV-LIQ-02 |
| `scale_seizures_to_received_spec` | `Proofs/Seizure.lean` | `math.rs::scale_seizures_to_received` | on a short receipt, every entry scaled down by `received/planned` (`ScaledEntry`, [notes](#theorem-notes)) | `received ≥ 0`, entry domain ([notes](#theorem-notes)) | INV-LIQ-03 |
| `ScaledEntry.proportional` | `Proofs/Seizure.lean` | same (per entry) | each scaled entry is proportional and no larger ([notes](#theorem-notes)) | `0 ≤ received < planned`, `shares, amount ≥ 0` | INV-LIQ-03 |
| `mul_wad_ratio_floor_spec` | `Proofs/Wrappers.lean` | `common/src/math/fp.rs::Ray::mul_wad_ratio_floor` | returns `⌊x·n/d⌋` (one floor; the WAD ratio is not quantised first) | `d ≠ 0`, the floor fits `i128` | INV-LIQ-03 |
| `mul_wad_ratio_floor_le` | `Proofs/Wrappers.lean` | same | returns `⌊x·n/d⌋ ∈ [0, x]` | `x ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | INV-LIQ-03 |
| `apply_ratio_to_amount_floor_spec` | `Proofs/Wrappers.lean` | `fp.rs::Wad::apply_ratio_to_amount_floor` (`self = n`) | returns `⌊a·n/d⌋` | `d ≠ 0`, the floor fits `i128` | INV-LIQ-03 |
| `apply_ratio_to_amount_floor_le` | `Proofs/Wrappers.lean` | same | returns `⌊a·n/d⌋ ∈ [0, a]` | `a ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | INV-LIQ-03 |
| `from_fraction_ceil_spec` | `Proofs/Wrappers.lean` | `fp.rs::Bps::from_fraction_ceil` | returns `⌈n·BPS/d⌉` | `d ≠ 0`, the ceiling fits `i128` | INV-LIQ-02 |
| `from_fraction_ceil_nonneg` | `Proofs/Wrappers.lean` | same | returns `⌈n·BPS/d⌉ ≥ 0` | `n ≥ 0`, `d > 0`, `n·BPS ≤ (2^127−1)·d` | INV-LIQ-02 |
| `apply_to_ray_ceil_spec` | `Proofs/Wrappers.lean` | `fp.rs::Bps::apply_to_ray_ceil` | returns `⌈v·b/BPS⌉` | the ceiling fits `i128` | INV-LIQ-02 |
| `apply_to_ray_ceil_le` | `Proofs/Wrappers.lean` | same | returns `⌈v·b/BPS⌉ ∈ [0, v]` | `v ≥ 0`, `0 ≤ b ≤ BPS` | INV-LIQ-02 |
| `mul_wad_ratio_floor_eq`, `apply_ratio_to_amount_floor_eq`, `from_fraction_ceil_eq`, `apply_to_ray_ceil_eq` | `Proofs/Wrappers.lean` | the four wrappers | each wrapper equals one raw call: `mul_div_floor(x, n, d)`, `mul_div_floor(a, n, d)`, `mul_div_ceil(n, BPS, d)`, `mul_div_ceil(v, b, BPS)` | none | — |
| `sum_repaid_usd_spec` | `Proofs/Sum.lean` | `math.rs::sum_repaid_usd` | returns exactly `Σ usd_wad` | every prefix sum fits `i128` | INV-LIQ-02 |
| `sum_repaid_usd_nonneg` | `Proofs/Sum.lean` | same | returns `Σ usd_wad ≥ 0` | all `usd_wad ≥ 0`, total `< 2^127` | INV-LIQ-02 |
| `calculate_account_risk_totals_spec` | `Proofs/Totals.lean` | `risk/totals.rs::calculate_account_risk_totals` | exact sums of the four totals and the health factor, context unchanged ([notes](#theorem-notes)) | TOT | INV-RISK-02 |
| `calculate_account_risk_totals_conservative` | `Proofs/Totals.lean` | same | collateral totals never overstate and debt never understates the exact value ([notes](#theorem-notes)) | TOT | INV-RISK-02 |
| `calculate_ltv_collateral_wad_spec` | `Proofs/Totals.lean` | `totals.rs::calculate_ltv_collateral_wad` | returns `Σ ⌊fv·min(LTV,LT)/BPS⌋` (same as the totals' `ltv_collateral`), context unchanged | TOT (supply half) | INV-RISK-02 |
| `sum_debt_usd_spec` | `Proofs/Totals.lean` | `totals.rs::sum_debt_usd` | returns `Σ huv` (borrow index) `≤ Σ cv`, context unchanged | TOT (debt half) | INV-RISK-02 |
| `totals_loop_spec`, `ltv_loop_spec`, `debt_loop_spec` | `Proofs/Totals.lean` | the three generated loops | exact accumulation, context unchanged | per-leg TOT, accumulator + remaining ceiling sum `< 2^127` | INV-RISK-02 |
| `calculate_repayment_amounts_spec` | `Proofs/Repayment.lean` | `liquidation/math.rs::calculate_repayment_amounts` | one capped entry per merged leg, exact refunds, total `= Σ legUsd` ([notes](#theorem-notes)) | REP; `aggregate_positive_payments` returned `P` | INV-LIQ-02 |
| `repay_entry_le_debt` | `Proofs/Repayment.lean` | same (per entry) | `amount ≤ D` and `amount ≤ offered` | an entry of the spec | INV-LIQ-02 |
| `repay_kept_plus_refund` | `Proofs/Repayment.lean` | same (per leg) | overpaid leg: `kept + refund = offered`; otherwise `kept = offered` (excess refunded exactly) | an entry/refund of the spec | INV-LIQ-02 |
| `repay_loop_spec`, `usd_value_wad_spec`, `from_token_spec` | `Proofs/Repayment.lean` | loop; `PriceFeed::usd_value_wad`; `Wad::from_token` | loop invariant; `hu(a·10^(18−dec)·p/W)`; exact upscale `a·10^(18−dec)` | as stated | INV-LIQ-02 |
| `apply_liquidation_repayments_eq` | `Proofs/Apply.lean` | `liquidation/apply.rs::apply_liquidation_repayments` (`offered = None`) | equation: repays the *received* amounts, returns `u = Σ legRecv` ([notes](#theorem-notes)) | APP | INV-LIQ-03 |
| `legRecv_le_planned`, `recvLeg_bounds` | `Proofs/Apply.lean` | same | `0 ≤ u ≤ Σ usd_wad`; per leg `0 ≤ legRecv ≤ usd_wad` and `legRecv·amount ≤ usd_wad·received` on a short receipt | APP | INV-LIQ-03 |
| `underdelivery_scales_seizures` | `Proofs/Apply.lean` | `apply_liquidation_repayments` → `scale_seizures_to_received` | `u ≤ planned`; a short receipt floors every seizure by `u/planned` ([notes](#theorem-notes)) | APP ([notes](#theorem-notes)) | INV-LIQ-03 |
| `credited_shares_spec` | `Proofs/Apply.lean` | `apply.rs::credited_shares` | returns `S − ⌈bonus·fee/BPS⌉ ∈ [0, S]` | `0 ≤ bonus ≤ S`, fee `< BPS` | INV-LIQ-02 |
| `check_bad_debt_after_liquidation_eq` | `Proofs/Apply.lean` | `apply.rs::check_bad_debt_after_liquidation` | equation: socializes exactly when `coll < debt ∧ coll ≤ 5 WAD` ([notes](#theorem-notes)) | none | INV-LIQ-04 |
| `seizure_budget_spec` | `Proofs/Apply.lean` | `math.rs::calculate_seized_collateral` (budget steps) | `total_seizure_usd = hu(repay·(BPS+bonus)/BPS)` ([notes](#theorem-notes)) | `repay ≥ 0`, `0 ≤ bonus ≤ BPS`, result fits | INV-LIQ-02 |
| `seizure_budget_exceeds_exact` | `Proofs/Apply.lean` | same | concrete case `repay = 10`, bonus 5%: budget 11 > exact 10.5 (rounding not in the protocol's favour, by < 1 WAD unit) | none | INV-LIQ-02 |
| `liq_bps_to_wad_val`, `div_floor_saturating_val` | `Proofs/Apply.lean`, `Proofs/Totals.lean` | `Bps::to_wad`, `Wad::div_floor_saturating` | `b·10^14`; `min(i128::MAX, ⌊a·W/b⌋)` for `a ≥ 0`, `b > 0` | as stated | — |

Certora counterparts (others have none): `position_value_bracket`: `position_value_ceil_ge_floor` (hf_lemma_rules; bounded); `max_bonus_for_threshold_bound`: `derived_bonus_respects_threshold`; `calculate_linear_bonus_spec`: `bonus_bounded`, `bonus_is_base_at_or_above_target`; `calculate_linear_bonus_antitone`: `bonus_monotone_in_hf`; `calculate_linear_bonus_max_below_floor`: `bonus_is_max_below_curve_floor`; `calculate_account_risk_totals_conservative`: `position_value_ceil_ge_floor` (per leg, bounded); `calculate_repayment_amounts_spec`: `liquidation_does_not_increase_repaid_debt` (account level, bounded); `repay_entry_le_debt`: `liquidation_does_not_increase_repaid_debt`; `underdelivery_scales_seizures`: `liquidation_does_not_increase_seized_collateral` (account level); `check_bad_debt_after_liquidation_eq`: `bad_debt_socialization_threshold_boundary`.

### Theorem notes

* **`calculate_linear_bonus_spec`**: `b = base` if `hf ≥ target`, else
  `b = base + hu(hu((max−base)·scale/W)·factor/BPS)`. Validated curve:
  `0 ≤ hf_max < target ≤ 10 WAD`, `0 ≤ factor ≤ BPS`.
* **`split_seized_shares_spec`**: `(fee, liq)`, `0 ≤ fee ≤ b`, `0 ≤ liq ≤ S`;
  the two re-checks are dead ([below](#production-code-review-notes)). No
  Certora rule; `seizure_split_math` / `protocol_fee_bonus_math`
  (value_math_rules) check the transfer-mode split (bounded).
* **`scale_seizures_to_received_spec`**:
  * `planned ≤ 0 ∨ received ≥ planned`: input returned unchanged.
  * Else same length, entries related by `ScaledEntry`: shares, bonus,
    amount floored by `received/planned`; amount capped at the new shares'
    floor value; fee floored and capped at the new amount; other fields
    unchanged.
  * Entry domain on a short receipt: decimals in `[3,18]`,
    `shares, index ≥ 0`, `shares·index < 2^127·R`.
  * No Certora rule (`liquidation_does_not_increase_seized_collateral` is
    account-level).
* **`ScaledEntry.proportional`**: `x'·planned ≤ x·received` for shares and
  amount, `x' ≤ x`, `fee' ≤ amount'`, and `bonus ≤ scaled` is preserved.
* **`calculate_account_risk_totals_spec`**: context unchanged, and:
  * `total_collateral = Σ huv`;
  * `ltv_collateral = Σ ⌊fv·min(LTV,LT)/BPS⌋`;
  * `weighted_collateral = Σ ⌊fv·LT/BPS⌋`;
  * `total_debt = Σ cv` (borrow index);
  * `health_factor = i128::MAX` without debt, else
    `min(i128::MAX, ⌊weighted·W/debt⌋)`.

  Certora summarises it outside the health/solvency rules.
* **`calculate_account_risk_totals_conservative`**:
  `weighted·BPS·10^54 ≤ Σ LT·s·i·p`; `ltv·BPS·10^54 ≤ Σ min(LTV,LT)·s·i·p`;
  `ltv ≤ weighted ≤ total_collateral`; `Σ fv ≤ total_collateral ≤ Σ cv`;
  `Σ s·i·p ≤ total_debt·10^54`.
* **`calculate_repayment_amounts_spec`** (merged legs `P`, context
  unchanged): one entry per leg with `amount = min(offered, D)` and the
  feed/index snapshot; `usd_wad = legUsd` (floor value `fv` of the whole
  debt when the leg pays `D`, else `hu(offered·10^(18−dec)·p/W)`); one refund
  `offered − D` per overpaid leg, in order; total `= Σ legUsd`.
* **`apply_liquidation_repayments_eq`**: one pool action per leg repaying the
  *received* amount, then the batch repay; returns `u = Σ legRecv`,
  `legRecv = usd_wad` if `received ≥ amount`, else
  `⌊usd_wad·received/amount⌋`.
* **`underdelivery_scales_seizures`**: seizures unchanged when all arrived;
  else every entry floored by `u/planned` (`ScaledEntry`). Hypotheses: APP,
  planned `= Σ usd_wad`, seizure entries as in
  `scale_seizures_to_received_spec`.
* **`check_bad_debt_after_liquidation_eq`**: no debt left → only the
  empty-account cleanup; else socialization (`execute_bad_debt_cleanup` on
  the whole account) exactly when `coll < debt ∧ coll ≤ 5 WAD`; else nothing.
* **`seizure_budget_spec`**: the `one_plus_bonus` / `total_seizure_usd` steps
  of `calculate_seized_collateral`.

### Hypothesis groups

Per entry of the respective map or list:

* **TOT** (`RawSupplyOK` / `RawDebtOK`): context serves the key (price and
  index cached, as after `load_markets`, which runs first); VD for (scaled,
  supply or borrow index, price); stored LTV and liquidation threshold
  `≤ BPS`; supply-leg ceiling values sum below `2^127`, debt-leg ones too.
* **REP** (`RepayLegOK`): context serves the key; the account has that debt
  position (else production aborts with `DebtPositionNotFound`); VD for the
  debt; `decimals ≤ 18`; offered `> 0`; for a leg offered below its cap,
  `offered·10^(18−dec)` and its USD value fit `i128`; leg values sum below
  `2^127`.
* **APP** (`ApplyLegOK`): spoke flags allow the exit; planned amount `> 0`,
  `usd_wad ≥ 0` (plan validation); measured receipt `≥ 0`; the account has
  the debt position; `Σ usd_wad < 2^127`.

### Why the hypotheses hold in production

* **VD**: balances, indexes, prices are non-negative `Ray`/`Wad`; the bound
  asks only that the exact USD value plus `2·10^-18`-scale slack fit `i128`
  (about `1.7·10^20` USD).
* **Curve**: `validate_liquidation_curve` (`common/src/validation.rs`)
  enforces `WAD < target ≤ MAX_LIQUIDATION_TARGET_HF_WAD = 10 WAD`,
  `0 < hf_for_max_bonus < target`, `0 < bonus_factor ≤ BPS` on every stored
  curve; `estimate_liquidation_amount` passes `curve.target_hf` as `target`.
  `get_account_bonus_params` sets `base = min(weighted bonus, max)`, both
  non-negative. Health factors are non-negative.
* **Seizure proportion**: `weighted / total` collateral with thresholds
  `≤ 100%`, so `0 < p ≤ W` for positive collateral.
* **Shares**: `LiquidationPlan::validate` rejects `bonus_scaled >
  scaled_amount` and negative values; stamped fee rates are below `BPS`.
* **Seizures**: `scale_seizures_to_received`'s doc requires decimals
  `≥ MIN_BORROWABLE_ASSET_DECIMALS` (it aborts otherwise);
  `MAX_ASSET_DECIMALS = 18`; received USD `≥ 0`.
* **Repayment legs**: every `usd_wad` is a floor or half-up valuation of a
  non-negative amount at a non-negative price.

## Axiom check

`ControllerAeneas.lean` ends with `#assert_standard_axioms ControllerAeneas`
([what it enforces](../README.md#trusted-base)).

## Not covered

* **`repays_every_debt_leg`** — opaque
  ([Aeneas issue 5](extraction.md#controller-aeneas-issues)), so the
  `repays_all_debt` flag is arbitrary in the model.
* **`apply_liquidation_seizures`, `apply_liquidation_share_credit`** —
  extracted, unproved: their effects are uninterpreted external batch
  withdraw / account-map updates and pool calls. Credit-split share
  conservation is proved per entry (`split_seized_shares_spec`,
  `credited_shares_spec`).
* **`apply_liquidation_repayments` with `offered = Some(..)`** (full-close
  plans): the pull amount comes from `Iterator::find`; the credited value is
  the same function of the receipt, but the theorem covers `offered = None`
  only.
* **`aggregate_positive_payments`** — extracted verbatim; the repayment
  theorem takes its merged list as a hypothesis.

Other documents list their own gaps. Error codes and `Vec`/`Map` model
limits: [README](../README.md#not-covered),
[trusted base](extraction.md#controller-trusted-base).

## Production code review notes

No economically relevant counterexample. One rounding direction disfavours
the protocol, by under one WAD unit (`10^-18` USD) per step; a liquidation
with `n` collateral legs has `1 + 3n` such steps, and the share and price
steps scale with the budget and the price (about `5·10^-13` USD for a $1M
liquidation against BTC):

* **`calculate_seized_collateral`'s seizure budget rounds half up.**
  `total_seizure_usd = repay_usd.mul(1 + bonus)` is `hu(repay·(BPS+bonus)/BPS)`
  (`seizure_budget_spec`): `repay_usd = 10` WAD units at 500 bps budgets 11,
  not 10.5 (`seizure_budget_exceeds_exact`). Per-leg steps also round half up
  ([seizure finding 2](controller-seizure.md#findings)). Seizure stays capped
  by the held balance, so "seizure ≤ repaid × (1 + bonus)" holds only up to
  these units; not exploitable.

Observations:

* `split_seized_shares`: the re-checks `fee_scaled > seized_scaled` and
  `fee_scaled + liquidator_scaled != seized_scaled` never fire after the
  entry checks (`fee ≤ bonus ≤ seized`; exact sum): harmless defence in
  depth.
* `max_bonus_for_threshold` never aborts on any `i128` proportion; for
  `p > W` the threshold clamps to `BPS` and the bonus is 0.
