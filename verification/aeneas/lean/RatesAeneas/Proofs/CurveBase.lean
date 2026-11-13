import RatesAeneas.Proofs.HalfUpWrappers

/-
Building blocks shared by the curve and index proofs: the generated `Ray`
comparisons, `Ray::checked_add`/`checked_sub`, a bounded `Ray::mul`, and
`rates::curve::utilization`.
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open rates_aeneas

set_option maxRecDepth 20000

namespace RatesAeneas.Proofs

/-- The integer value of a `Ray`/`Bps` (both are `i128`). -/
local notation "rv " r:max => IScalar.val (ty := IScalarTy.I128) r

/-- `<` on `Ray` through the derived `PartialOrd`: integer comparison. -/
theorem ray_lt_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.lt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (rv a < rv b)) := by
  simp [core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_lt_iff_lt]

/-- `>` on `Ray` through the derived `PartialOrd`: integer comparison. -/
theorem ray_gt_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.gt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (rv b < rv a)) := by
  simp [core.cmp.PartialOrd.gt.trait_default, core.cmp.PartialOrd.gt.default,
    core.cmp.PartialOrd.gt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_gt_iff_gt]

/-- `>=` on `Ray` through the derived `PartialOrd`: integer comparison. -/
theorem ray_ge_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.ge.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (rv b ≤ rv a)) := by
  simp [core.cmp.PartialOrd.ge.trait_default, core.cmp.PartialOrd.ge.default,
    core.cmp.PartialOrd.ge_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_gt_iff_gt]
  by_cases h1 : rv b < rv a <;> by_cases h2 : rv a = rv b <;> simp [h1, h2] <;> omega

/-- `==` on `Ray` through the derived `PartialEq`: integer equality. -/
theorem ray_eq_eq (a b : math.fp.Ray) :
    math.fp.Ray.Insts.CoreCmpPartialEqRay.eq a b = ok (decide (rv a = rv b)) := by
  simp only [math.fp.Ray.Insts.CoreCmpPartialEqRay.eq]
  congr 1
  simp only [decide_eq_decide]
  exact ⟨fun h => by rw [h], fun h => IScalar.eq_of_val_eq h⟩

/-- `Ray::ONE` is `10^27`. -/
theorem ray_one_val : rv math.fp.Ray.ONE = 10^27 := by
  unfold math.fp.Ray.ONE constants.production.shared.RAY; simp

/-- `Ray::ZERO` is `0`. -/
theorem ray_zero_val_hu : rv math.fp.Ray.ZERO = 0 := by
  unfold math.fp.Ray.ZERO; simp

/-- `Ray::checked_add` returns the exact sum when it fits `i128`. -/
theorem ray_checked_add_spec (env : Env) (a b : math.fp.Ray)
    (h : -2^127 ≤ rv a + rv b ∧ rv a + rv b < 2^127) :
    math.fp.Ray.checked_add a env b ⦃ c => rv c = rv a + rv b ⦄ := by
  unfold math.fp.Ray.checked_add math.fp.checked_add_raw
  have hadd := core.num.checked_add_IScalar_bv_spec a b
  rw [show I128.checked_add a b = core.num.checked_add_IScalar a b from rfl]
  cases hc : core.num.checked_add_IScalar a b with
  | none =>
    rw [hc] at hadd; exfalso; apply hadd
    simp only [IScalar.min, IScalar.max, IScalarTy.numBits]; omega
  | some c =>
    rw [hc] at hadd
    obtain ⟨_, _, hcv, _⟩ := hadd
    simp [lift, core.option.Option.unwrap_or_else, hcv]

/-- `Ray::checked_sub` returns `a - b` for `0 ≤ b ≤ a`. -/
theorem ray_checked_sub_spec (env : Env) (a b : math.fp.Ray)
    (hb : 0 ≤ rv b) (hba : rv b ≤ rv a) :
    math.fp.Ray.checked_sub a env b ⦃ c => rv c = rv a - rv b ⦄ := by
  unfold math.fp.Ray.checked_sub math.fp.checked_sub_nonneg
  have h1 : ¬ (a < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have h2 : ¬ (b < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have h3 : ¬ (b > a) := by simp [IScalar.lt_equiv]; omega
  simp only [massert, h1, h2, h3, not_false_eq_true, ↓reduceIte, bind_ok]
  have hab := a.hBounds
  have hbb := b.hBounds
  simp only [IScalarTy.numBits] at hab hbb
  step as ⟨c, hc⟩
  simpa using hc

/-- `Ray::checked_sub` aborts when the result would be negative (`b > a`) or
an operand is negative. -/
theorem ray_checked_sub_aborts (env : Env) (a b : math.fp.Ray)
    (h : rv a < 0 ∨ rv b < 0 ∨ rv a < rv b) :
    ¬ (math.fp.Ray.checked_sub a env b ⦃ _ => True ⦄) := by
  unfold math.fp.Ray.checked_sub math.fp.checked_sub_nonneg
  by_cases h1 : a < 0#i128
  · simp [massert, h1]
  by_cases h2 : b < 0#i128
  · simp [massert, h1, h2]
  have h3 : b > a := by
    simp [IScalar.lt_equiv] at h1 h2 ⊢; omega
  simp [massert, h1, h2, h3]

/-- `Ray::mul` by a factor in `[0, RAY]`: never aborts, exact half-up, and the
result lies in `[0, b]`. -/
theorem ray_mul_frac_spec (env : Env) (a b : math.fp.Ray)
    (ha : 0 ≤ rv a) (haR : rv a ≤ 10^27) (hb : 0 ≤ rv b) :
    math.fp.Ray.mul a env b ⦃ q =>
      IsHalfUp (rv a * rv b) (10^27) (rv q) ∧ 0 ≤ rv q ∧ rv q ≤ rv b ⦄ := by
  have hbb := b.hBounds
  simp only [IScalarTy.numBits] at hbb; norm_num at hbb
  have hfit := halfUp_ratio_fits (rv b) (rv a) (10^27) (by norm_num) hb (by omega) ha haR
  rw [mul_comm] at hfit
  apply WP.spec_mono (ray_mul_spec env a b ha hb hfit)
  intro q hq
  refine ⟨hq, ?_⟩
  rw [mul_comm] at hq
  exact IsHalfUp.le_of_ratio_le_one (by norm_num) hb ha haR hq

/-! ## `rates::curve::utilization` -/

/-- **`utilization` is total and returns a value in `[0, RAY]`.** For
non-negative `borrowed` and `supplied` it never aborts, and:

* `supplied = 0` gives 0;
* `borrowed ≥ supplied > 0` gives `RAY`, by comparison, with no division;
* `borrowed < supplied` gives `half_up(borrowed * RAY / supplied)`, which is at
  most `RAY`, so the division cannot overflow.

Production feeds pool totals (`borrowed`, `supplied` are sums of non-negative
scaled amounts times indexes), so the hypotheses hold. This closes the
remaining part of audit finding S-03: no quotient above `RAY` and no overflow
fallback. -/
theorem utilization_spec (env : Env) (b s : math.fp.Ray) (hb : 0 ≤ rv b) (hs : 0 ≤ rv s) :
    rates.curve.utilization env b s ⦃ u =>
      0 ≤ rv u ∧ rv u ≤ 10^27 ∧
      (rv s = 0 → rv u = 0) ∧
      (0 < rv s → rv s ≤ rv b → rv u = 10^27) ∧
      (rv b < rv s → IsHalfUp (rv b * 10^27) (rv s) (rv u)) ⦄ := by
  unfold rates.curve.utilization
  simp only [ray_eq_eq, ray_lt_eq, ray_ge_eq, bind_ok, ray_zero_val_hu]
  by_cases hs0 : rv s = 0
  · simp only [hs0, decide_true, ↓reduceIte, WP.spec_ok]
    rw [ray_zero_val_hu]
    exact ⟨le_refl _, by norm_num, fun _ => rfl, fun h => absurd h (by omega), fun h => by omega⟩
  · have hb' : ¬ rv b < 0 := by omega
    have hs' : ¬ rv s < 0 := by omega
    simp only [hs0, hb', hs', decide_false, Bool.false_eq_true, ↓reduceIte, massert,
      not_false_eq_true, bind_ok]
    by_cases hge : rv s ≤ rv b
    · simp only [hge, decide_true, ↓reduceIte, WP.spec_ok]
      rw [ray_one_val]
      exact ⟨by norm_num, le_refl _, fun h => by first | exact h.elim | exact absurd h hs0,
        fun _ _ => rfl, fun h => by first | exact h.elim | omega⟩
    · simp only [hge, decide_false, Bool.false_eq_true, ↓reduceIte]
      have hsp : 0 < rv s := by omega
      have hfit := halfUp_ratio_fits (10^27) (rv b) (rv s) hsp (by norm_num) (by norm_num) hb
        (by omega)
      rw [mul_comm] at hfit
      apply WP.spec_mono (ray_div_spec env b s hb hsp hfit)
      intro u hu
      have hu' := hu
      rw [mul_comm] at hu'
      have hbound := IsHalfUp.le_of_ratio_le_one hsp (by norm_num) hb (by omega) hu'
      exact ⟨hbound.1, hbound.2, fun h => by first | exact h.elim | exact absurd h hs0,
        fun _ h => by first | exact h.elim | omega, fun _ => hu⟩

end RatesAeneas.Proofs
