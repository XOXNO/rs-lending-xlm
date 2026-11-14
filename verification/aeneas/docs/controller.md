# Controller liquidation and risk math (Aeneas + Lean)

Machine-checked Lean 4 proofs about the production controller liquidation
math and the position valuation it relies on. Charon and Aeneas translate the
Rust; every theorem is about the generated `lean/ControllerAeneas/Funs.lean`.
Library: `ControllerAeneas`. Conventions and pitfalls: [../WRITING-PROOFS.md](../WRITING-PROOFS.md).
Extraction crate, trusted models and tool workarounds:
[extraction.md](extraction.md#controller).

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

Every theorem is **total correctness** (`f args ⦃ r => P r ⦄`: the call
returns, does not abort, and `P` holds). It holds over **unbounded `i128`
inputs**, subject only to the stated domain hypotheses. Specifications never divide:
`IsFloor`, `IsCeil` and `IsHalfUp` (`lean/FixedPoint/Spec.lean`,
`FloorCeilSpec.lean`, `HalfUpSpec.lean`, shared with the other libraries) are stated with
multiplications only.

## Extraction and trusted base

The extraction crate, its stand-ins and shim, the full list of hand-written
models, the copied proof files, the rebuild commands and the Aeneas
workarounds are in [extraction.md](extraction.md#controller).

The theorems trust three things
([details](extraction.md#controller-trusted-base)):

1. Charon and Aeneas translate faithfully (pinned release, see
   [../README.md](../README.md)).
2. The hand-written models in `lean/ControllerAeneas/TypesExternal.lean` and
   `FunsExternal.lean` are right:
   * `I256` is exact integer arithmetic with the host's 256-bit traps.
   * `Vec<T>` is a Lean list. The host's `u32::MAX` length limit is not
     modelled.
   * **Map model:** `Map<K, V>` is a list of entries in iteration order. Key
     order and key uniqueness are not modelled, and every theorem holds for
     all entry lists.
   * **Context model:** the two caches `token_prices` and `market_indexes`.
     The rest of the host state (pool index, oracle price, spoke
     configuration, transfer receipts) is uninterpreted, so a theorem holds
     for every value it could take.
3. `panic_with_error!`/`assert_with_error!` abort.

## Supporting proof files

`Model.lean` proves the Map/Context facts the liquidation proofs use:
`map_get_eq`/`mapLookup`, `cached_price_ok`, `cached_market_index_ok`,
`load_markets_cached` (no change when every key is cached),
`iter_typed_positions_spec`, `iter_debt_positions_spec`,
`i128_from_u32_val`.

* `Basic.lean`: comparison/min/conversion lemmas for `Wad`/`Ray`/`Bps`,
  `checked_add_raw`, `checked_sub_nonneg`.
* `Rounding.lean`: division-free relations between floor, half-up and
  ceiling.
* `Wrappers.lean`: the typed `fp.rs` wrappers. It holds the unfolding
  equations to the raw `mul_div_*` call, the exact-rounding specs, and the
  ratio-at-most-one lemmas `floor_ratio_fits`, `floor_ratio_le` and
  `ceil_ratio_le`.
* `Shares`, `Curve` and `Seizure` rewrite each wrapper call with its `*_eq`
  lemma and then use the raw `mul_div_*` spec.

Copied math proofs (`MulDiv`, `HalfUp`, `Scaling`, ...) are listed in
[extraction.md](extraction.md#controller-copied-proofs).

## Theorems

Notation: `R = 10^27` (RAY), `B = 10^9` (RAY→WAD factor), `W = 10^18` (WAD),
`BPS = 10^4`; `hu` = rounding to nearest, halves up.
Value-domain hypothesis **(VD)** for `scaled s`, `index i`, `price p`:
`s,i,p ≥ 0`, `s·i + R ≤ 2^127·R` and `(s·i + 2RB)·p + WRB ≤ 2^127·WRB`
(the exact value plus rounding slack fits `i128`).

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora |
|---|---|---|---|---|---|---|
| `position_value_floor_spec` | `Proofs/Value.lean` | `common/src/rates/value.rs::position_value_floor` | returns `⌊⌊⌊s·i/R⌋/B⌋·p/W⌋` | VD | INV-RISK-02 | — |
| `position_value_spec` | `Proofs/Value.lean` | `value.rs::position_value` | returns `hu(hu(hu(s·i/R)/B)·p/W)` | VD | INV-RISK-02 | — |
| `position_value_ceil_spec` | `Proofs/Value.lean` | `value.rs::position_value_ceil` | returns `⌈⌈⌈s·i/R⌉/B⌉·p/W⌉` | VD | INV-RISK-02 | — |
| `position_value_bracket` | `Proofs/Value.lean` | all three | all three return and `floor ≤ half-up ≤ ceil` | VD | INV-RISK-02 | `position_value_ceil_ge_floor` (hf_lemma_rules; bounded) |
| `position_value_floor_ceil_exact` | `Proofs/Value.lean` | floor, ceil | `floor·RBW ≤ s·i·p ≤ ceil·RBW`: floor never overstates and ceil never understates the exact value | VD | INV-RISK-02 | — |
| `apply_to_wad_floor_spec` | `Proofs/Value.lean` | `common/src/math/fp.rs::Bps::apply_to_wad_floor` | returns `⌊v·rate/BPS⌋` (single downward rounding; per-leg LTV/threshold weighting in `risk/totals.rs`) | `0 ≤ rate ≤ BPS`, `v ≥ 0` | INV-RISK-02 | — |
| `is_socializable_bad_debt_spec` | `Proofs/Curve.lean` | `liquidation/curve.rs::is_socializable_bad_debt` | never aborts; true iff `coll < debt ∧ coll ≤ 5 WAD` | none | INV-LIQ-04 | — |
| `max_bonus_for_threshold_spec` | `Proofs/Curve.lean` | `curve.rs::max_bonus_for_threshold` | `p ≤ 0` → 0; `p > 0` → `⌊BPS(BPS−e)/e⌋`, `e = clamp(⌈p·BPS/W⌉, 1, BPS)` | none (all `i128`) | INV-LIQ-02 | — |
| `max_bonus_for_threshold_bound` | `Proofs/Curve.lean` | same | `0 ≤ m ≤ BPS(BPS−1)` and `(BPS+m)·p ≤ BPS·W` (`(1+bonus)·proportion ≤ 1`) | `0 < p ≤ W` | INV-LIQ-02 | `derived_bonus_respects_threshold` |
| `max_bonus_for_threshold_antitone` | `Proofs/Curve.lean` | same | `0 < p₁ ≤ p₂` → `m(p₂) ≤ m(p₁)` | as stated | INV-LIQ-02 | — |
| `bonus_scale_spec` | `Proofs/Curve.lean` | `curve.rs::LiquidationCurve::bonus_scale` | returns `min(hu((target−hf)·W/(target−hf_max)), W)` | `0 ≤ hf_max < target ≤ 10 WAD`, `0 ≤ hf ≤ target` | — | — |
| `calculate_linear_bonus_spec` | `Proofs/Curve.lean` | `curve.rs::calculate_linear_bonus_with_target` | exact bonus `b` ([notes](#theorem-notes)); `base ≤ b ≤ max` | validated curve, `hf ≥ 0`, `0 ≤ base ≤ max` | INV-LIQ-02 | `bonus_bounded`, `bonus_is_base_at_or_above_target` |
| `calculate_linear_bonus_antitone` | `Proofs/Curve.lean` | same | `0 ≤ hf₁ ≤ hf₂` → `bonus(hf₂) ≤ bonus(hf₁)` | as above | INV-LIQ-02 | `bonus_monotone_in_hf` |
| `calculate_linear_bonus_max_below_floor` | `Proofs/Curve.lean` | same | `hf ≤ hf_max`, factor = BPS → bonus = `max` | as above | INV-LIQ-02 | `bonus_is_max_below_curve_floor` |
| `split_seized_shares_spec` | `Proofs/Shares.lean` | `liquidation/math.rs::split_seized_shares` | `fee = ⌈b·f/BPS⌉`, `fee + liq = S` exactly, both in range ([notes](#theorem-notes)) | `0 ≤ b ≤ S`, `f < BPS` | INV-LIQ-02 | none for this function ([notes](#theorem-notes)) |
| `scale_seizures_to_received_spec` | `Proofs/Seizure.lean` | `math.rs::scale_seizures_to_received` | on a short receipt, every entry scaled down by `received/planned` (`ScaledEntry`, [notes](#theorem-notes)) | `received ≥ 0`, entry domain ([notes](#theorem-notes)) | INV-LIQ-03 | none for this function ([notes](#theorem-notes)) |
| `ScaledEntry.proportional` | `Proofs/Seizure.lean` | same (per entry) | each scaled entry is proportional and no larger ([notes](#theorem-notes)) | `0 ≤ received < planned`, `shares, amount ≥ 0` | INV-LIQ-03 | — |
| `mul_wad_ratio_floor_spec` | `Proofs/Wrappers.lean` | `common/src/math/fp.rs::Ray::mul_wad_ratio_floor` | returns `⌊x·n/d⌋` (one floor; the WAD ratio is not quantised first) | `d ≠ 0`, the floor fits `i128` | INV-LIQ-03 | — |
| `mul_wad_ratio_floor_le` | `Proofs/Wrappers.lean` | same | returns `⌊x·n/d⌋ ∈ [0, x]` | `x ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | INV-LIQ-03 | — |
| `apply_ratio_to_amount_floor_spec` | `Proofs/Wrappers.lean` | `fp.rs::Wad::apply_ratio_to_amount_floor` (`self = n`) | returns `⌊a·n/d⌋` | `d ≠ 0`, the floor fits `i128` | INV-LIQ-03 | — |
| `apply_ratio_to_amount_floor_le` | `Proofs/Wrappers.lean` | same | returns `⌊a·n/d⌋ ∈ [0, a]` | `a ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | INV-LIQ-03 | — |
| `from_fraction_ceil_spec` | `Proofs/Wrappers.lean` | `fp.rs::Bps::from_fraction_ceil` | returns `⌈n·BPS/d⌉` | `d ≠ 0`, the ceiling fits `i128` | INV-LIQ-02 | — |
| `from_fraction_ceil_nonneg` | `Proofs/Wrappers.lean` | same | returns `⌈n·BPS/d⌉ ≥ 0` | `n ≥ 0`, `d > 0`, `n·BPS ≤ (2^127−1)·d` | INV-LIQ-02 | — |
| `apply_to_ray_ceil_spec` | `Proofs/Wrappers.lean` | `fp.rs::Bps::apply_to_ray_ceil` | returns `⌈v·b/BPS⌉` | the ceiling fits `i128` | INV-LIQ-02 | — |
| `apply_to_ray_ceil_le` | `Proofs/Wrappers.lean` | same | returns `⌈v·b/BPS⌉ ∈ [0, v]` | `v ≥ 0`, `0 ≤ b ≤ BPS` | INV-LIQ-02 | — |
| `mul_wad_ratio_floor_eq`, `apply_ratio_to_amount_floor_eq`, `from_fraction_ceil_eq`, `apply_to_ray_ceil_eq` | `Proofs/Wrappers.lean` | the four wrappers | each wrapper equals one raw call: `mul_div_floor(x, n, d)`, `mul_div_floor(a, n, d)`, `mul_div_ceil(n, BPS, d)`, `mul_div_ceil(v, b, BPS)` | none | — | — |
| `sum_repaid_usd_spec` | `Proofs/Sum.lean` | `math.rs::sum_repaid_usd` | returns exactly `Σ usd_wad` | every prefix sum fits `i128` | INV-LIQ-02 | — |
| `sum_repaid_usd_nonneg` | `Proofs/Sum.lean` | same | returns `Σ usd_wad ≥ 0` | all `usd_wad ≥ 0`, total `< 2^127` | INV-LIQ-02 | — |
| `calculate_account_risk_totals_spec` | `Proofs/Totals.lean` | `risk/totals.rs::calculate_account_risk_totals` | exact sums of the four totals and the health factor, context unchanged ([notes](#theorem-notes)) | TOT | INV-RISK-02 | — ([notes](#theorem-notes)) |
| `calculate_account_risk_totals_conservative` | `Proofs/Totals.lean` | same | collateral totals never overstate and debt never understates the exact value ([notes](#theorem-notes)) | TOT | INV-RISK-02 | `position_value_ceil_ge_floor` (per leg, bounded) |
| `calculate_ltv_collateral_wad_spec` | `Proofs/Totals.lean` | `totals.rs::calculate_ltv_collateral_wad` | returns `Σ ⌊fv·min(LTV,LT)/BPS⌋` (same as the totals' `ltv_collateral`), context unchanged | TOT (supply half) | INV-RISK-02 | — |
| `sum_debt_usd_spec` | `Proofs/Totals.lean` | `totals.rs::sum_debt_usd` | returns `Σ huv` (borrow index) `≤ Σ cv`, context unchanged | TOT (debt half) | INV-RISK-02 | — |
| `totals_loop_spec`, `ltv_loop_spec`, `debt_loop_spec` | `Proofs/Totals.lean` | the three generated loops | exact accumulation, context unchanged | per-leg TOT, accumulator + remaining ceiling sum `< 2^127` | INV-RISK-02 | — |
| `calculate_repayment_amounts_spec` | `Proofs/Repayment.lean` | `liquidation/math.rs::calculate_repayment_amounts` | one capped entry per merged leg, exact refunds, total `= Σ legUsd` ([notes](#theorem-notes)) | REP; `aggregate_positive_payments` returned `P` | INV-LIQ-02 | `liquidation_does_not_increase_repaid_debt` (account level, bounded) |
| `repay_entry_le_debt` | `Proofs/Repayment.lean` | same (per entry) | `amount ≤ D` and `amount ≤ offered` | an entry of the spec | INV-LIQ-02 | `liquidation_does_not_increase_repaid_debt` |
| `repay_kept_plus_refund` | `Proofs/Repayment.lean` | same (per leg) | overpaid leg: `kept + refund = offered`; otherwise `kept = offered` (excess refunded exactly) | an entry/refund of the spec | INV-LIQ-02 | — |
| `repay_loop_spec`, `usd_value_wad_spec`, `from_token_spec` | `Proofs/Repayment.lean` | loop; `PriceFeed::usd_value_wad`; `Wad::from_token` | loop invariant; `hu(a·10^(18−dec)·p/W)`; exact upscale `a·10^(18−dec)` | as stated | INV-LIQ-02 | — |
| `apply_liquidation_repayments_eq` | `Proofs/Apply.lean` | `liquidation/apply.rs::apply_liquidation_repayments` (`offered = None`) | equation: repays the *received* amounts, returns `u = Σ legRecv` ([notes](#theorem-notes)) | APP | INV-LIQ-03 | — |
| `legRecv_le_planned`, `recvLeg_bounds` | `Proofs/Apply.lean` | same | `0 ≤ u ≤ Σ usd_wad`; per leg `0 ≤ legRecv ≤ usd_wad` and `legRecv·amount ≤ usd_wad·received` on a short receipt | APP | INV-LIQ-03 | — |
| `underdelivery_scales_seizures` | `Proofs/Apply.lean` | `apply_liquidation_repayments` → `scale_seizures_to_received` | `u ≤ planned`; a short receipt floors every seizure by `u/planned` ([notes](#theorem-notes)) | APP ([notes](#theorem-notes)) | INV-LIQ-03 | `liquidation_does_not_increase_seized_collateral` (account level) |
| `credited_shares_spec` | `Proofs/Apply.lean` | `apply.rs::credited_shares` | returns `S − ⌈bonus·fee/BPS⌉ ∈ [0, S]` | `0 ≤ bonus ≤ S`, fee `< BPS` | INV-LIQ-02 | — |
| `check_bad_debt_after_liquidation_eq` | `Proofs/Apply.lean` | `apply.rs::check_bad_debt_after_liquidation` | equation: socializes exactly when `coll < debt ∧ coll ≤ 5 WAD` ([notes](#theorem-notes)) | none | INV-LIQ-04 | `bad_debt_socialization_threshold_boundary` |
| `seizure_budget_spec` | `Proofs/Apply.lean` | `math.rs::calculate_seized_collateral` (budget steps) | `total_seizure_usd = hu(repay·(BPS+bonus)/BPS)` ([notes](#theorem-notes)) | `repay ≥ 0`, `0 ≤ bonus ≤ BPS`, result fits | INV-LIQ-02 | — |
| `seizure_budget_exceeds_exact` | `Proofs/Apply.lean` | same | concrete case `repay = 10`, bonus 5%: budget 11 > exact 10.5 (rounding not in the protocol's favour, by < 1 WAD unit) | none | INV-LIQ-02 | — |
| `liq_bps_to_wad_val`, `div_floor_saturating_val` | `Proofs/Apply.lean`, `Proofs/Totals.lean` | `Bps::to_wad`, `Wad::div_floor_saturating` | `b·10^14`; `min(i128::MAX, ⌊a·W/b⌋)` for `a ≥ 0`, `b > 0` | as stated | — | — |

### Theorem notes

* **`calculate_linear_bonus_spec`**: returns `b = base` if `hf ≥ target`;
  else `b = base + hu(hu((max−base)·scale/W)·factor/BPS)`. The validated
  curve is `0 ≤ hf_max < target ≤ 10 WAD` and `0 ≤ factor ≤ BPS`.
* **`split_seized_shares_spec`**: returns `(fee, liq)` with `0 ≤ fee ≤ b` and
  `0 ≤ liq ≤ S`. The two internal re-checks are dead code. No Certora rule
  covers this function. `seizure_split_math` / `protocol_fee_bonus_math` in
  value_math_rules check the transfer-mode fee split (bounded).
* **`scale_seizures_to_received_spec`**:
  * `planned ≤ 0 ∨ received ≥ planned`: the input is returned unchanged.
  * Otherwise the output has the same length, and each entry is related by
    `ScaledEntry`. Shares, bonus and amount are floored by
    `received/planned`. The amount is capped at the floor value of the new
    shares. The fee is floored and capped at the new amount. Other fields are
    unchanged.
  * Entry domain, on a short receipt: decimals in `[3,18]`,
    `shares, index ≥ 0`, `shares·index < 2^127·R`.
  * No Certora rule covers this function.
    `liquidation_does_not_increase_seized_collateral` is an account-level
    rule.
* **`ScaledEntry.proportional`**: `x'·planned ≤ x·received` for shares and
  amount, `x' ≤ x`, `fee' ≤ amount'`, and `bonus ≤ scaled` is preserved.
* **`calculate_account_risk_totals_spec`**: returns with the context
  unchanged, and:
  * `total_collateral = Σ huv`;
  * `ltv_collateral = Σ ⌊fv·min(LTV,LT)/BPS⌋`;
  * `weighted_collateral = Σ ⌊fv·LT/BPS⌋`;
  * `total_debt = Σ cv` (borrow index);
  * `health_factor = i128::MAX` without debt, else
    `min(i128::MAX, ⌊weighted·W/debt⌋)`.

  Certora summarises this function outside the health/solvency rules.
* **`calculate_account_risk_totals_conservative`**:
  `weighted·BPS·10^54 ≤ Σ LT·s·i·p`; `ltv·BPS·10^54 ≤ Σ min(LTV,LT)·s·i·p`;
  `ltv ≤ weighted ≤ total_collateral`; `Σ fv ≤ total_collateral ≤ Σ cv`;
  `Σ s·i·p ≤ total_debt·10^54`.
* **`calculate_repayment_amounts_spec`**: for the merged legs `P`, returns
  with the context unchanged.
  * One entry per leg with `amount = min(offered, D)` and the feed/index
    snapshot.
  * `usd_wad = legUsd`: the floor value `fv` of the whole debt when the leg
    pays `D`, else `hu(offered·10^(18−dec)·p/W)`.
  * One refund `offered − D` per overpaid leg, in order.
  * Total `= Σ legUsd`.
* **`apply_liquidation_repayments_eq`**: equals one pool action per leg
  repaying with the *received* amount, then the batch repay. It returns
  `u = Σ legRecv`, where `legRecv = usd_wad` if `received ≥ amount`, else
  `⌊usd_wad·received/amount⌋`.
* **`underdelivery_scales_seizures`**: seizures are unchanged when all
  arrived. Otherwise every entry is floored by `u/planned` (`ScaledEntry`).
  Hypotheses: APP, planned `= Σ usd_wad`, and seizure entries as in
  `scale_seizures_to_received_spec`.
* **`check_bad_debt_after_liquidation_eq`**: with no debt left, only the
  empty-account cleanup runs. Otherwise socialization
  (`execute_bad_debt_cleanup` on the whole account) runs exactly when
  `coll < debt ∧ coll ≤ 5 WAD`. Else nothing happens.
* **`seizure_budget_spec`**: covers the `one_plus_bonus` /
  `total_seizure_usd` steps of `calculate_seized_collateral`.

### Hypothesis groups

Each group applies per entry of the respective map or list.

* **TOT** (`RawSupplyOK` / `RawDebtOK`):
  * the context serves the key: the price of its asset and its index are
    cached. This is true after `load_markets`, which the functions call
    first and which then changes nothing;
  * VD for (scaled, supply or borrow index, price);
  * stored LTV and liquidation threshold `≤ BPS`;
  * the ceiling values of all supply legs sum below `2^127`, and so do those
    of all debt legs.
* **REP** (`RepayLegOK`): the context serves the key; the account has that
  debt position (production aborts with `DebtPositionNotFound` otherwise);
  VD for the debt; `decimals ≤ 18`; offered amount `> 0`; for a leg offered
  below its cap, `offered·10^(18−dec)` and its USD value fit `i128`; the leg
  values sum below `2^127`.
* **APP** (`ApplyLegOK`): the spoke flags allow the exit; planned amount
  `> 0` and `usd_wad ≥ 0` (plan validation); the measured receipt is
  non-negative; the account has the debt position; and
  `Σ usd_wad < 2^127`.

### Why the hypotheses hold in production

* **VD** — scaled balances, indexes and prices are non-negative `Ray`/`Wad`
  values. The bound only asks that the exact USD value, plus at most
  `2·10^-18`-scale slack, fits `i128`. That limit is about `1.7·10^20` USD,
  far above any position.
* **Curve** — `validate_liquidation_curve` (`common/src/validation.rs`)
  enforces `WAD < target ≤ MAX_LIQUIDATION_TARGET_HF_WAD = 10 WAD`,
  `0 < hf_for_max_bonus < target` and `0 < bonus_factor ≤ BPS` on every stored
  curve; `estimate_liquidation_amount` passes `curve.target_hf` as `target`.
  `base = min(weighted bonus, max)` in `get_account_bonus_params` with both
  non-negative, so `0 ≤ base ≤ max`. Health factors are non-negative.
* **Seizure proportion** — `weighted / total` collateral with every
  liquidation threshold at most 100%, so `0 < p ≤ W` whenever collateral is
  positive.
* **Shares** — `LiquidationPlan::validate` rejects entries with
  `bonus_scaled > scaled_amount` or negative values, and stamped fee rates are
  below `BPS`.
* **Seizures** — `scale_seizures_to_received`'s own doc requires decimals
  `≥ MIN_BORROWABLE_ASSET_DECIMALS` (it aborts otherwise, as the theorem
  shows by requiring it); `MAX_ASSET_DECIMALS = 18`; received USD is
  non-negative.
* **Repayment legs** — every `usd_wad` is a floor or half-up valuation of a
  non-negative amount at a non-negative price.

## Axiom check

`ControllerAeneas.lean` ends with `#assert_standard_axioms ControllerAeneas`
(`lean/Audit.lean`). So the build fails unless every declaration of the
library depends only on `propext`, `Classical.choice` and `Quot.sound`. To
rebuild, see [extraction.md](extraction.md#controller-rebuild).

## Not covered

* **Proved in their own documents:** `estimate_liquidation_amount`,
  `liquidation_at_target`, `max_hf_preserving_bonus_bps`
  ([controller-estimate.md](controller-estimate.md));
  `calculate_seized_collateral` per leg, `get_account_bonus_params`,
  `calculate_seizure_proportions` and the post-liquidation health factor
  ([controller-seizure.md](controller-seizure.md));
  `process_excess_payment`, `release_unbacked_repayment`,
  `normalize_repayment_plan`, `whole_unit_repayment`, `one_unit_per_*`
  ([controller-repayment.md](controller-repayment.md)). Each lists what it
  still leaves out.
* **`repays_every_debt_leg`** — kept opaque. Its `Iterator::all` closure
  mutably borrows the context, and the Aeneas library's `FnMut` model cannot
  type that ([Aeneas issue 5](extraction.md#controller-aeneas-issues)). So the `repays_all_debt` flag is arbitrary in the
  model.
* **`apply_liquidation_seizures`, `apply_liquidation_share_credit`** —
  extracted, not proved. Their effects are the external batch withdraw /
  account-map updates and pool calls (uninterpreted). Share conservation of
  the credit split is proved per entry (`split_seized_shares_spec`,
  `credited_shares_spec`).
* **`apply_liquidation_repayments` with `offered = Some(..)`** (full-close
  plans) — the pull amount is looked up with `Iterator::find`. The credited
  value per leg is the same function of the receipt. The theorem is stated
  for `offered = None` only.
* **`aggregate_positive_payments`** — included verbatim and extracted. The
  repayment theorem takes its result as given (a hypothesis naming the merged
  list). It does not prove the aggregation.
* **Error codes** — every abort is just an abort; codes are checked by Kani /
  tests, not here.
* **`Vec` length limit** — not modelled ([trusted base](extraction.md#controller-trusted-base)).
* **Map key order** — not modelled; no theorem depends on it.

## Production code review notes

No economically relevant counterexample was found. One rounding direction
does not favour the protocol, by less than one WAD unit (`10^-18` USD):

* **The seizure budget of `calculate_seized_collateral` rounds half up.**
  `total_seizure_usd = repay_usd.mul(1 + bonus)` is `hu(repay·(BPS+bonus)/BPS)`
  (`seizure_budget_spec`). With `repay_usd = 10` WAD units and a 500 bps bonus
  the exact budget is 10.5 units and the code budgets 11
  (`seizure_budget_exceeds_exact`). The per-leg share (`Wad::div`), leg
  budget (`Wad::mul`) and price division (`Wad::div`) are also half up. The
  seizure is still capped by the held balance. The excess is a fraction of a
  WAD unit per rounding step. So this is a documentation point, not an
  exploitable flaw: any claim "seizure ≤ repaid × (1 + bonus)" holds only up
  to these units.

Observations made while proving:

* `split_seized_shares`: the re-checks `fee_scaled > seized_scaled` and
  `fee_scaled + liquidator_scaled != seized_scaled` can never fire once the
  entry checks pass (`fee ≤ bonus ≤ seized`; the sum is exact). They are
  harmless defence in depth.
* `max_bonus_for_threshold` accepts any `i128` proportion without aborting;
  for `p > W` the effective threshold clamps to `BPS` and the bonus is 0.
