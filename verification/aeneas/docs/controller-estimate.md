# Controller liquidation estimate (Aeneas + Lean)

These Lean proofs (library `ControllerAeneas`, about its generated `Funs.lean`)
cover `estimate_liquidation_amount`, `liquidation_at_target` and
`max_hf_preserving_bonus_bps` in
`contracts/controller/src/positions/liquidation/curve.rs`.
[controller.md](controller.md) covers extraction, the trusted base and the
bonus-curve proofs. Conventions are in
[../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

* `max_hf_preserving_bonus_bps` has an exact value and a maximal cap. It never
  aborts for `hf ≥ 0`.
* `liquidation_at_target` has an exact value. It never aborts under the rate
  cap. It aborts if and only if the target-solve quotient overflows (basic
  domain).
* `estimate_liquidation_amount` has an exact value per branch and never
  aborts on the production domain. The repayment is at most the debt, leaves
  no sub-threshold dust, uses an HF-preserving bonus, and hits the curve
  target within rounding. The consistency hypothesis is necessary
  (counterexample).
* `Bps::to_wad` on a bonus is exact (`b·10^14`).

The files are plain non-`module` Lean in `lean/ControllerAeneas/Proofs/`:
`EstimateBasic.lean` (`to_wad`, `1 + bonus`, the two helpers), `Estimate.lean`
(`EstDomain`, `EstimateSpec`, spec and contract, `hf_preserved_of_rate`),
`EstimateTarget.lean` (curve target, full close, Certora generalisation) and
`EstimateAbort.lean` (aborts, counterexample). To build them, run
`lake build ControllerAeneas.Proofs.EstimateAbort`, which imports the other
three.

## Notation

`W = 10^18`, `BPS = 10^4`. Snapshot (WAD): `C` collateral, `D` debt, `Wc`
weighted collateral, `p` seizure proportion, `hf`. Curve: `T` target HF,
`hf_max`, `factor`. BPS: `base`, `max`, bonus `b`. `opb = W + b·10^14`.
`hu` = half-up. In `liquidation_at_target`: `dmax = hu(C·W/opb)`,
`δ = hu(p·opb/W)` (`denom_term`), `τ = hu(T·D/W)` (`target_debt`), target
solve `q = hu((τ − Wc)·W/(T − δ))`. `cap = ⌊hf·BPS/p⌋ − BPS`.

**EstDomain** (`Estimate.lean`), the production domain:

* curve: `0 ≤ hf_max < T`, `W < T ≤ 10 W`, `0 ≤ factor ≤ BPS`.
* bounds: `0 ≤ base ≤ max ≤ BPS·(BPS−1)`, and `(BPS + max)·p ≤ BPS·W` when `hf ≥ W`.
* snapshot: `C, Wc, p, hf ≥ 0`, `0 ≤ D ≤ 10^36`.
* consistency: `hf·D ≤ Wc·W`.

**Rate cap** (for `liquidation_at_target`): some `h` with `0 ≤ h < T`,
`(BPS + b)·p ≤ h·BPS` and `h·D ≤ Wc·W`.

## Theorems

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora |
|---|---|---|---|---|---|---|
| `bps_to_wad_spec` | `EstimateBasic` | `fp.rs::Bps::to_wad` | `b·10^14` | `0 ≤ b ≤ 10^8` | — | — |
| `one_plus_bonus_spec` | `EstimateBasic` | `Wad::ONE.checked_add(b.to_wad())` | `W + b·10^14` | `0 ≤ b ≤ 10^8` | — | — |
| `max_hf_preserving_bonus_bps_spec` | `EstimateBasic` | `curve.rs::max_hf_preserving_bonus_bps` | never aborts. `None` if `p ≤ 0 ∨ hf ≥ W`, else `Some cap`, `cap + BPS = ⌊hf·BPS/p⌋` | `hf ≥ 0` | INV-LIQ-02 | — |
| `max_hf_cap_bounds` | `EstimateBasic` | same | `(BPS + cap)·p ≤ hf·BPS < (BPS + cap + 1)·p`: largest bonus with `(1 + bonus)·p ≤ hf` | `p > 0` | INV-LIQ-02 | — |
| `liquidation_at_target_spec_gen` | `EstimateBasic` | `curve.rs::liquidation_at_target` | `r` with `LiqAtTarget` (both branches exact), `0 ≤ r ≤ min(dmax, D)` | `0 ≤ b ≤ 10^8`, `C, Wc ≥ 0`, `0 ≤ p ≤ 10^22`, `0 ≤ D ≤ 10^36`, `0 < T ≤ 10 W`, `q < 2^127` on the target-solve branch | INV-LIQ-02 | — |
| `liquidation_at_target_spec` | `EstimateBasic` | same | as above without the overflow assumption: rate cap gives `q ≤ D + W` | basic domain + rate cap | INV-LIQ-02 | — |
| `liquidation_at_target_abort` | `EstimateAbort` | same | aborts if the target-solve branch is taken (`δ < T`, `Wc < τ`) with `q ≥ 2^127`. With `liquidation_at_target_spec_gen`, **iff** on the basic domain | basic domain | — | — |
| `mul_div_half_up_overflow` | `EstimateAbort` | `fp_core.rs::mul_div_half_up` | fails if the half-up quotient is `≥ 2^127` | `x, y ≥ 0`, `d > 0` | — | — |
| `rate_branch_dead` | `EstimateTarget` | `liquidation_at_target` | `δ < T`: the `target_hf <= denom_term` early return is dead at the estimate's call | rate cap | — | — |
| `estimate_liquidation_amount_spec` | `Estimate` | `curve.rs::estimate_liquidation_amount` | never aborts. `(r, b)` = `EstimateSpec` (below), curve bonus `sb` (`calculate_linear_bonus_spec`) | EstDomain | INV-LIQ-02 | `estimate_liquidation_sanity` (generalised: every domain input) |
| `estimate_liquidation_amount_contract` | `Estimate` | same | `0 ≤ r ≤ D`. `0 ≤ b ≤ max`. Unless insolvent-with-cap, `r = D ∨ D − r ≥ 5 W` (no dust). For `0 < p`, `hf < W`, `C ≥ D`: `(BPS + b)·p ≤ hf·BPS` or (`r = D`, `b = 0`) | EstDomain | INV-LIQ-02, INV-LIQ-04 | `estimate_leaves_no_sub_threshold_dust`, `ideal_repayment_targets_curve_hf` (2nd assertion) |
| `hf_preserved_of_rate` | `Estimate` | (math) | seizing at that rate never lowers `Wc/D`: `BPS·W·Wc·(D−r) ≤ (BPS·W·Wc − p(BPS+b)r)·D`, all `r ≥ 0` | `(BPS + b)·p ≤ hf·BPS`, `hf·D ≤ Wc·W` | INV-LIQ-02 | — |
| `LiqAtTarget.bounds`, `DustAdj.no_dust` | `Estimate` | (lemmas) | target quote in `[0, D]`. Dust rule leaves `0` or `≥ 5 W` | non-negativity | — | — |
| `LiqAtTarget.target` | `EstimateTarget` | `liquidation_at_target` | every value is `D`, `Exhausts` or `HitsTarget` | none | INV-LIQ-02 | — |
| `estimate_hits_curve_target` | `EstimateTarget` | `estimate_liquidation_amount` | `r = D` ∨ `Exhausts C b r` ∨ `HitsTarget D Wc p T b r` | EstDomain | INV-LIQ-02 | `ideal_repayment_targets_curve_hf` (intent) |
| `estimate_full_close_of_low_cap` | `EstimateTarget` | same | `0 < p`, `hf < W`, `C ≥ D`, `hf·BPS < (BPS + base)·p` → `r = D`, `b = max(cap, 0)` | EstDomain | INV-LIQ-02 | — |
| `certora_estimate_rules` | `EstimateTarget` | same | returns `(D, 0)` | Certora snapshot: `C = D`, `p = hu(Wc·W/D)`, `hf = ⌊Wc·W/D⌋`, `0 < hf < W`, `0 < base ≤ max ≤ BPS`. Any validated curve. `D ≤ 10^36` | INV-LIQ-02 | `ideal_repayment_targets_curve_hf`, `estimate_leaves_no_sub_threshold_dust` (all assertions, unbounded) |
| `estimate_aborts_inconsistent` | `EstimateAbort` | same | aborts though every EstDomain field but consistency holds | input below | — | — |

Definitions: `LiqAtTarget`, `EstDomain`, `DustAdj`, `EstimateSpec`,
`HitsTarget`, `Exhausts`. Helper lemmas are documented in the Lean files.

### `EstimateSpec`: the branches of `estimate_liquidation_amount`

* `p ≤ 0` or `hf ≥ W` (no cap): `b = sb`. `r` is the dust-adjusted
  `liquidation_at_target` quote at `sb`.
* otherwise:
  * `C < D` (insolvent): `b = base`, `r = min(⌊C·W/(W + base·10^14)⌋, D)`.
  * `C ≥ D`, `cap < base`: `r = D`, `b = max(cap, 0)`.
  * `C ≥ D`, `cap ≥ base`: `b = min(sb, cap)`. `r` is the dust-adjusted quote
    at `b`.

Dust adjustment: `r = D` if `0 < D − ideal < 5 W`, else `r = ideal`.

### The curve-HF-target property (`HitsTarget`)

With `δ` the code's seizure rate, `E = W·Wc − δ·r`, `G = T·(D − r)`:

`2G − (T − δ) − W < 2E ≤ 2G + (T − δ) + W`.

`E/W` is weighted collateral left after repaying `r` and seizing `(1 + b)·r`
at proportion `δ/W`. `G/W` is the target times the remaining debt. So the post-liquidation
HF equals the target within `((T − δ) + W)/(2W) ≤ 5.5` raw WAD of weighted
collateral (`5.5·10^-18` USD) either way. `r` is the exact
`r* = (T·D − W·Wc)/(T − δ)` after three half-up roundings (`τ`, `δ`, `q`).
The rounded `δ = hu(p·opb/W)` is within `1/2` of `p·opb/W`, which adds at
most `r/2` (in `W·USD`) to the bound.

## Why the hypotheses hold in production

The only caller is `normalize_repayment_plan`, from `build_liquidation_plan`
(`positions/liquidation/plan.rs`).

* **Curve**: `LiquidationCurve::from_config` reads a `SpokeConfig` validated
  by `validate_liquidation_curve` (`common/src/validation.rs`):
  `W < T ≤ MAX_LIQUIDATION_TARGET_HF_WAD = 10 W`, `0 < hf_max < T`,
  `0 < factor ≤ BPS`.
* **Bounds**: `get_account_bonus_params` sets `max = max_bonus_for_threshold(p)`:
  `0 ≤ max ≤ BPS·(BPS−1)`, `(BPS + max)·p ≤ BPS·W` for `0 < p ≤ W`
  (`max_bonus_for_threshold_bound`, [controller.md](controller.md)), `max = 0`
  for `p ≤ 0`. `base = min(weighted bonus ≥ 0, max)`.
* **Snapshot**: The snapshot holds sums of non-negative valuations
  (`risk/totals.rs`). `p = hu(Wc·W/C)` or `0`. `D ≤ 10^36` is USD 10^18.
* **Consistency** `hf·D ≤ Wc·W`: `hf = Wc.div_floor_saturating(D) = ⌊Wc·W/D⌋`
  (no saturation, because `hf < W` is asserted, and the assertion also
  excludes `i128::MAX` at `D = 0`). The hypothesis is necessary
  (`estimate_aborts_inconsistent`).
* The asserted `hf < W` is **not** needed. `(BPS + max)·p ≤ BPS·W` covers
  `hf ≥ W`.

### Why the divisor is bounded

The curve alone allows `T = W + 1`, so `T − δ` can be `2`. The **rate cap**
(`δ ≤ hf`) and consistency (`W·(τ − Wc) ≤ D·(T − hf) + W/2`) give
`q ≤ D + (W + den)/(2·den) ≤ D + W`, with `den = T − δ ≥ T − hf`. The cap
holds in every branch that reaches `liquidation_at_target`:

* bonus `min(sb, cap)`, `cap ≥ base ≥ 0`: `h = hf`, as `(BPS + b)·p ≤ (BPS + cap)·p ≤ hf·BPS`.
* `None` with `p = 0`: `h = hf`, trivially.
* `None` with `hf ≥ W`: `h = W`, from `(BPS + max)·p ≤ BPS·W`.

## Counterexample (consistency is necessary)

The counterexample uses curve `T = W + 1`, `hf_max = W/2`, `factor = BPS`
(valid), `base = max = 0`, `C = D = 10^21` (USD 1000), `Wc = 0` and
`p = hf = W − 1`. Only `hf·D ≤ Wc·W` fails. Then `cap = 0`, bonus `0`,
`δ = W − 1`, `T − δ = 2`, `τ = 10^21 + 10^3`,
`q = (10^21 + 10^3)·5·10^17 ≈ 5·10^38 > 2^127`, so the call aborts
(`estimate_aborts_inconsistent`). Production cannot build this, because
`Wc = 0` gives `hf = p = 0`.

## Findings

No production bug. On its domain the estimate never aborts and meets its
contract. Observations:

1. **The Certora rules `ideal_repayment_targets_curve_hf` and
   `estimate_leaves_no_sub_threshold_dust` never exercise the curve.** With
   `total_collateral = total_debt`, `p = hu(Wc·W/D) ≥ hf = ⌊Wc·W/D⌋`, so
   `cap ≤ 0 < base` and the estimate always returns `(D, 0)` from full close
   (`certora_estimate_rules`). It skips `liquidation_at_target` and the dust
   check. `estimate_hits_curve_target` and
   `estimate_liquidation_amount_contract` cover the target-HF claim for every
   solvent snapshot (`C ≥ D`). To fix the spec, draw
   `total_collateral ≥ total_debt` independently.
2. **`target_hf <= denom_term` in `liquidation_at_target` is dead code** at its
   only call site (`rate_branch_dead`, plus the cases `p = 0` and `hf ≥ W`).
   Its doc comment ("If seizure's weighted rate meets the target…") describes
   an unreachable state. The dead code is harmless defence in depth.
3. **The insolvent branch (`C < D`) may leave positive debt below
   `BAD_DEBT_USD_THRESHOLD`**, by design. The remainder is bad debt for
   socialisation (INV-LIQ-04). The dust theorem excludes this branch.
4. **Full close at a reduced bonus.** If the surplus is too small for the base
   bonus (`hf·BPS < (BPS + base)·p`, roughly `(C − D)/D < base`), the estimate
   is the whole debt at bonus `max(cap, 0) < base`
   (`estimate_full_close_of_low_cap`). With `C ≥ D`, `cap < 0` (`hf < p`)
   arises only from rounding. For example, with `C = D`, `p` rounds up and `hf`
   rounds down. Then the bonus is `0` and the HF is not preserved, but the
   debt is cleared.
5. The unchecked `hf * BPS` in `max_hf_preserving_bonus_bps` aborts only for
   `hf < −2^127/BPS` with `p > 0`. Production passes `hf ≥ 0` (assumed).

## Not covered

* **The executed plan's real post-liquidation HF.** `HitsTarget` uses the
  estimate's rate `δ`. The executed seizure goes through
  `calculate_seized_collateral` (per-asset, `Context` prices) and
  `whole_unit_repayment`. Separate theorems cover those two functions
  ([controller-seizure.md](controller-seizure.md),
  [controller-repayment.md](controller-repayment.md)), but no theorem chains
  the estimate to them. The `#[cfg(test)]` mirror
  `calculate_post_liquidation_hf` is not extracted.
* **Exact abort set of `estimate_liquidation_amount` outside `EstDomain`.**
  Only non-abort on `EstDomain` and the necessity counterexample are proved
  (the exact abort condition is proved only for `liquidation_at_target`).
  Inputs with negative fields, `D > 10^36` or `max > 10^8` are not
  characterised.
* **Full-close quote above collateral backing.** For `cap < base`, rounding of
  `p` and `hf` can make `D·(1 + cap)` exceed `C` by a few units, and no theorem states this. The
  plan's `release_unbacked_repayment` handles any shortfall.
