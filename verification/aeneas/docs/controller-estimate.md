# Controller liquidation estimate (Aeneas + Lean)

Machine-checked Lean 4 proofs about `estimate_liquidation_amount`,
`liquidation_at_target` and `max_hf_preserving_bonus_bps` in
`contracts/controller/src/positions/liquidation/curve.rs`. Every theorem is
about the generated `lean/ControllerAeneas/Funs.lean`, library
`ControllerAeneas`. The extraction, trusted base and the bonus-curve proofs
these build on are described in [controller.md](controller.md). Conventions:
[../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

| Rust function | Status |
|---|---|
| `max_hf_preserving_bonus_bps` | exact value, maximality of the cap, never aborts for `hf ≥ 0` |
| `liquidation_at_target` | exact value; never aborts under the rate cap; aborts if and only if the target-solve quotient overflows (on the basic domain) |
| `estimate_liquidation_amount` | exact value of every branch, never aborts on the production domain, repayment ≤ debt, no sub-threshold dust, HF-preserving bonus, hits the curve target within rounding; consistency hypothesis shown necessary by a counterexample |
| `Bps::to_wad` on a bonus | exact (`b·10^14`) |

Files (all plain, non-`module` Lean files under `lean/ControllerAeneas/Proofs/`):

* `EstimateBasic.lean`: `to_wad`, `1 + bonus`, `max_hf_preserving_bonus_bps`, `liquidation_at_target`.
* `Estimate.lean`: `EstDomain`, `EstimateSpec`, the exact spec and the contract of `estimate_liquidation_amount`, `hf_preserved_of_rate`.
* `EstimateTarget.lean`: the curve-target property, the full-close branch, the Certora generalisation.
* `EstimateAbort.lean`: abort characterisation and the counterexample.

Build: `lake build ControllerAeneas.Proofs.EstimateAbort` (it imports the other three).

## Notation

`W = 10^18` (WAD), `BPS = 10^4`. Snapshot: `C` total collateral, `D` total
debt, `Wc` weighted collateral, `p` seizure proportion, `hf` health factor
(all WAD). Curve: `T` target HF, `hf_max`, `factor`. Bounds `base`, `max`, bonus
`b` (BPS). `opb = W + b·10^14` (`1 + bonus` in WAD). `hu` = half-up rounding.
Inside `liquidation_at_target`: `dmax = hu(C·W/opb)`, `δ = hu(p·opb/W)`
(`denom_term`), `τ = hu(T·D/W)` (`target_debt`), and the target solve
`q = hu((τ − Wc)·W/(T − δ))`. `cap = ⌊hf·BPS/p⌋ − BPS`.

**EstDomain** (structure in `Estimate.lean`), the production domain:

* curve: `0 ≤ hf_max < T`, `W < T ≤ 10 W`, `0 ≤ factor ≤ BPS`;
* bounds: `0 ≤ base ≤ max ≤ BPS·(BPS−1)`, and `(BPS + max)·p ≤ BPS·W` when `hf ≥ W`;
* snapshot: `C, Wc, p, hf ≥ 0`, `0 ≤ D ≤ 10^36`;
* consistency: `hf·D ≤ Wc·W`.

**Rate cap** (for `liquidation_at_target`): some `h` with `0 ≤ h < T`,
`(BPS + b)·p ≤ h·BPS` and `h·D ≤ Wc·W`.

## Theorems

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora |
|---|---|---|---|---|---|---|
| `bps_to_wad_spec` | `EstimateBasic` | `fp.rs::Bps::to_wad` | returns exactly `b·10^14` | `0 ≤ b ≤ 10^8` | — | — |
| `one_plus_bonus_spec` | `EstimateBasic` | `Wad::ONE.checked_add(b.to_wad())` | returns exactly `W + b·10^14` | `0 ≤ b ≤ 10^8` | — | — |
| `max_hf_preserving_bonus_bps_spec` | `EstimateBasic` | `curve.rs::max_hf_preserving_bonus_bps` | `p ≤ 0 ∨ hf ≥ W` → `None`; otherwise `Some cap` with `cap + BPS = ⌊hf·BPS/p⌋`; never aborts | `hf ≥ 0` | INV-LIQ-02 | — |
| `max_hf_cap_bounds` | `EstimateBasic` | same | `(BPS + cap)·p ≤ hf·BPS < (BPS + cap + 1)·p`: `cap` is the largest BPS bonus with `(1 + bonus)·p ≤ hf` | `p > 0` | INV-LIQ-02 | — |
| `liquidation_at_target_spec_gen` | `EstimateBasic` | `curve.rs::liquidation_at_target` | returns `r` with `LiqAtTarget` (exact value of both branches), `0 ≤ r ≤ min(dmax, D)` | `0 ≤ b ≤ 10^8`, `C, Wc ≥ 0`, `0 ≤ p ≤ 10^22`, `0 ≤ D ≤ 10^36`, `0 < T ≤ 10 W`, and `q < 2^127` whenever the target-solve branch is taken | INV-LIQ-02 | — |
| `liquidation_at_target_spec` | `EstimateBasic` | same | as above, with no overflow assumption: the rate cap bounds `q ≤ D + W` | basic domain + rate cap | INV-LIQ-02 | — |
| `liquidation_at_target_abort` | `EstimateAbort` | same | target-solve branch taken (`δ < T`, `Wc < τ`) and `q ≥ 2^127` → aborts. With `_spec_gen`: on the basic domain it aborts **iff** this holds | basic domain | — | — |
| `mul_div_half_up_overflow` | `EstimateAbort` | `fp_core.rs::mul_div_half_up` | on valid operands, a half-up quotient `≥ 2^127` makes the call fail | `x, y ≥ 0`, `d > 0` | — | — |
| `rate_branch_dead` | `EstimateTarget` | `liquidation_at_target` | under the rate cap `δ < T`: the `target_hf <= denom_term` early return is dead code at the estimate's call | rate cap | — | — |
| `estimate_liquidation_amount_spec` | `Estimate` | `curve.rs::estimate_liquidation_amount` | never aborts; returns `(r, b)` = `EstimateSpec` (branches below) with the curve bonus `sb` (`calculate_linear_bonus_spec`) | EstDomain | INV-LIQ-02 | `estimate_liquidation_sanity` (generalised: every domain input returns) |
| `estimate_liquidation_amount_contract` | `Estimate` | same | `0 ≤ r ≤ D`; `0 ≤ b ≤ max`; unless insolvent-with-cap, `r = D ∨ D − r ≥ 5 W` (no dust); for `0 < p`, `hf < W`, `C ≥ D`: `(BPS + b)·p ≤ hf·BPS` or (`r = D`, `b = 0`) | EstDomain | INV-LIQ-02, INV-LIQ-04 | `estimate_leaves_no_sub_threshold_dust`, `ideal_repayment_targets_curve_hf` (2nd assertion) |
| `hf_preserved_of_rate` | `Estimate` | (math) | if `(BPS + b)·p ≤ hf·BPS` and `hf·D ≤ Wc·W`, seizing at that rate never lowers `Wc/D`: `BPS·W·Wc·(D−r) ≤ (BPS·W·Wc − p(BPS+b)r)·D` for every `r ≥ 0` | as stated | INV-LIQ-02 | — |
| `LiqAtTarget.bounds`, `DustAdj.no_dust` | `Estimate` | (lemmas) | a target quote lies in `[0, D]`; the dust rule leaves `0` or `≥ 5 W` | non-negativity | — | — |
| `LiqAtTarget.target` | `EstimateTarget` | `liquidation_at_target` | every value is `D`, collateral-exhausting (`Exhausts`) or `HitsTarget` | none | INV-LIQ-02 | — |
| `estimate_hits_curve_target` | `EstimateTarget` | `estimate_liquidation_amount` | `r = D` ∨ `Exhausts C b r` ∨ `HitsTarget D Wc p T b r` | EstDomain | INV-LIQ-02 | `ideal_repayment_targets_curve_hf` (intent) |
| `estimate_full_close_of_low_cap` | `EstimateTarget` | same | `0 < p`, `hf < W`, `C ≥ D`, `hf·BPS < (BPS + base)·p` → `r = D`, `b = max(cap, 0)` | EstDomain | INV-LIQ-02 | — |
| `certora_estimate_rules` | `EstimateTarget` | same | the Certora snapshot (`C = D`, `p = hu(Wc·W/D)`, `hf = ⌊Wc·W/D⌋`, `0 < hf < W`, `0 < base ≤ max ≤ BPS`) returns exactly `(D, 0)`, for any validated curve and `D ≤ 10^36` | as stated | INV-LIQ-02 | `ideal_repayment_targets_curve_hf`, `estimate_leaves_no_sub_threshold_dust` (all assertions, unbounded) |
| `estimate_aborts_inconsistent` | `EstimateAbort` | same | counterexample: every EstDomain field but consistency holds, and the call aborts | concrete input below | — | — |

Supporting definitions: `LiqAtTarget`, `EstDomain`, `DustAdj`,
`EstimateSpec`, `HitsTarget`, `Exhausts`; helpers `est_WAD_ONE_val`,
`est_WAD_ZERO_val`, `est_wad_min_val`, `est_lift_ok`, `est_bind_eq_ok_inv`,
`dust_tail_spec`, `exhausts_of_halfUp`, `exhausts_of_floor`.

### `EstimateSpec`: the branches of `estimate_liquidation_amount`

* `p ≤ 0` or `hf ≥ W` (no HF-preserving cap): `b = sb`; `r` = the
  dust-adjusted `liquidation_at_target` quote at `sb`.
* otherwise:
  * `C < D` (insolvent): `b = base`, `r = min(⌊C·W/(W + base·10^14)⌋, D)`;
  * `C ≥ D`, `cap < base`: `r = D`, `b = max(cap, 0)`;
  * `C ≥ D`, `cap ≥ base`: `b = min(sb, cap)`; `r` = the dust-adjusted quote at `b`.

Dust adjustment: if `0 < D − ideal < 5 W` then `r = D`, else `r = ideal`.

### The curve-HF-target property (`HitsTarget`)

With `δ` the seizure rate the code computes and `E = W·Wc − δ·r`,
`G = T·(D − r)`:

`2G − (T − δ) − W < 2E ≤ 2G + (T − δ) + W`.

`E/W` is the weighted collateral left after repaying `r` and seizing
`(1 + b)·r` at proportion `δ/W`; `G/W` is target × remaining debt. So the
post-liquidation health factor equals the target up to
`((T − δ) + W)/(2W) ≤ 5.5` raw WAD units of weighted collateral
(`5.5·10^-18` USD), in both directions. The target is the exact real solution
`r* = (T·D − W·Wc)/(T − δ)`; `r` is it after three half-up roundings
(`τ`, `δ`, `q`).

The statement uses the code's rounded rate `δ = hu(p·opb/W)` rather than the
exact `p·opb/W`; the two differ by at most `1/2`, which adds at most `r/2`
(in `W·USD`) to the bound.

## Why the hypotheses hold in production

The only caller is `normalize_repayment_plan`, reached from
`build_liquidation_plan` (`positions/liquidation/plan.rs`):

* **Curve**: `LiquidationCurve::from_config` reads a `SpokeConfig` that passed
  `validate_liquidation_curve` (`common/src/validation.rs`):
  `W < T ≤ MAX_LIQUIDATION_TARGET_HF_WAD = 10 W`, `0 < hf_max < T`,
  `0 < factor ≤ BPS`.
* **Bounds**: `get_account_bonus_params` sets `max = max_bonus_for_threshold(p)`,
  so `0 ≤ max ≤ BPS·(BPS−1)` and `(BPS + max)·p ≤ BPS·W` for `0 < p ≤ W`
  (`max_bonus_for_threshold_bound`, [controller.md](controller.md)); for `p ≤ 0`,
  `max = 0`. `base = min(weighted bonus, max)` with a non-negative weighted bonus.
* **Snapshot values**: sums of non-negative valuations (`risk/totals.rs`);
  `p = hu(Wc·W/C)` or `0`. `D ≤ 10^36` is USD 10^18.
* **Consistency** `hf·D ≤ Wc·W`: `hf = Wc.div_floor_saturating(D) = ⌊Wc·W/D⌋`
  (no saturation since `hf < W` is asserted), or `i128::MAX` when `D = 0`,
  which the `hf < W` assert excludes. The field is necessary
  (`estimate_aborts_inconsistent`).
* `hf < W` (asserted before the estimate) is **not** needed: the theorems also
  cover `hf ≥ W`, using `(BPS + max)·p ≤ BPS·W`.

### Why the divisor is bounded

The divisor `T − δ` is not bounded below by the curve alone: `T` may be
`W + 1`, so `T − δ` can be `2`. What bounds the quotient is the **rate cap**
together with consistency. The cap gives `δ ≤ hf`. Consistency gives
`W·(τ − Wc) ≤ D·(T − hf) + W/2`. Together they give
`q ≤ D + (W + den)/(2·den) ≤ D + W`, using `den = T − δ ≥ T − hf`.
The cap holds in every branch that reaches `liquidation_at_target`:

* bonus `min(sb, cap)` with `cap ≥ base ≥ 0`: `(BPS + b)·p ≤ (BPS + cap)·p ≤ hf·BPS`, with `h = hf`;
* the `None` branch with `p = 0`: `h = hf` works trivially;
* the `None` branch with `hf ≥ W`: `h = W` works, from `(BPS + max)·p ≤ BPS·W`.

## Counterexample (consistency is necessary)

Curve `T = W + 1`, `hf_max = W/2`, `factor = BPS` (passes
`validate_liquidation_curve`). Bounds `base = max = 0`. Snapshot
`C = D = 10^21` (USD 1000), `Wc = 0`, `p = hf = W − 1`. Every `EstDomain`
field holds except `hf·D ≤ Wc·W`.

Then `cap = 0` and the bonus is `0`. `δ = W − 1`, so `T − δ = 2`.
`τ = 10^21 + 10^3`, and `q = (10^21 + 10^3)·5·10^17 ≈ 5·10^38 > 2^127`:
the call aborts (`estimate_aborts_inconsistent`). Production cannot build
this snapshot: with `Wc = 0` it computes `hf = 0` and `p = 0`.

## Findings

No production bug: on the production domain the estimate never aborts and
meets its documented contract. Observations:

1. **The Certora rules `ideal_repayment_targets_curve_hf` and
   `estimate_leaves_no_sub_threshold_dust` never exercise the curve.** They fix
   `total_collateral = total_debt`. Then `p = hu(Wc·W/D) ≥ hf = ⌊Wc·W/D⌋`, so
   `cap ≤ 0 < base`, and the estimate always takes the full-close branch with
   result `(D, 0)` (`certora_estimate_rules`). `liquidation_at_target` and the
   dust check are never reached, so the target-HF claim in the rule name is
   untested there. `estimate_hits_curve_target` and
   `estimate_liquidation_amount_contract` cover it for every solvent snapshot
   (`C ≥ D`). Suggested spec fix: draw `total_collateral ≥ total_debt`
   independently.
2. **`target_hf <= denom_term` in `liquidation_at_target` is dead code** at its
   only call site (`rate_branch_dead`, plus the `p = 0` / `hf ≥ W` cases). Its
   doc comment ("If seizure's weighted rate meets the target…") describes a
   state the estimate never passes in. Harmless defence in depth.
3. **The insolvent branch (`C < D`) may leave positive debt below
   `BAD_DEBT_USD_THRESHOLD`.** This is by design: the remainder is bad debt
   for socialisation (INV-LIQ-04), and the dust theorem excludes this branch
   explicitly.
4. **Full close at a reduced bonus.** For a solvent account whose surplus is
   too small for the base bonus (`hf·BPS < (BPS + base)·p`, roughly
   `(C − D)/D < base`), the estimate is the whole debt at bonus
   `max(cap, 0) < base` (`estimate_full_close_of_low_cap`). `cap < 0`
   (`hf < p`) can happen with `C ≥ D` only through rounding (for example
   `C = D`, where `p` is rounded up and `hf` down). Then the bonus is `0` and
   the HF is not preserved, but the debt is cleared.
5. `max_hf_preserving_bonus_bps` uses unchecked `hf * BPS`. It aborts only for
   `hf < −2^127/BPS` with `p > 0`, never for the non-negative HF production
   passes (the theorem assumes `hf ≥ 0`).

## Not covered

* **The real post-liquidation HF of the executed plan.** `HitsTarget` is
  stated with the estimate's own seizure rate `δ`. The executed seizure goes
  through `calculate_seized_collateral` (per-asset, `Context` prices) and
  `whole_unit_repayment`, which are not extracted (see
  [controller.md](controller.md), Not covered). The `#[cfg(test)]` mirror
  `calculate_post_liquidation_hf` is not compiled into the extraction.
* **Exact abort set of `estimate_liquidation_amount` outside `EstDomain`.**
  The exact abort condition is proved for `liquidation_at_target` on its basic
  domain (`_spec_gen` and `_abort`). For the estimate, only non-abort on
  `EstDomain` and the necessity counterexample are proved. Inputs with
  negative fields, `D > 10^36` or `max > 10^8` are not characterised.
* **Does the full-close quote exceed collateral backing?** In the `cap < base`
  branch, rounding of `p` and `hf` may make `D·(1 + cap)` exceed `C` by a few
  units. Not stated. The plan handles any shortfall in
  `release_unbacked_repayment`, which is not extracted.
* **Error codes** are not modelled (an abort is an abort).

## Axioms

`#print axioms` was run on all 27 theorems in the four files. Each depends
only on `propext`, `Classical.choice` and `Quot.sound` (or a subset). None
uses `sorry`, `admit` or a new `axiom`.
