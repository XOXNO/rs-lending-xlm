-- Snapshot of RatesAeneas/Proofs/HalfUpDivInt.lean taken 2026-11-12 (namespace changed); see WRITING-PROOFS.md, last section.
import ControllerAeneas.Proofs.HalfUp

/-
`fp_core::div_by_int_half_up` (common/src/math/fp_core.rs): `a / b` rounded to
the nearest integer, exact halves away from zero, for every `i128` numerator
and positive divisor. It never aborts for `b > 0`, and always aborts for
`b ≤ 0`.
-/

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- Arithmetic core: from the truncating quotient `t` and remainder `r` of
`a / b` and `half = ⌈b/2⌉`, the production branch is the ties-away rounding. -/
theorem halfAway_of_trunc (a b t r h : Int) (hb : 0 < b) (hsum : b * t + r = a)
    (hr : -b < r ∧ r < b) (hsign1 : 0 ≤ a → 0 ≤ r) (hsign2 : a < 0 → r ≤ 0)
    (hh : h = b / 2 + b % 2) :
    IsHalfAway a b (if r ≥ h then t + 1 else if r ≤ -h then t - 1 else t) := by
  have hb2 : b = 2 * (b / 2) + b % 2 := by omega
  have hm : 0 ≤ b % 2 ∧ b % 2 ≤ 1 := by omega
  unfold IsHalfAway IsHalfUp
  by_cases ha : 0 ≤ a
  · have hr0 := hsign1 ha
    rw [if_pos ha]
    by_cases h1 : r ≥ h
    · rw [if_pos h1]; constructor <;> nlinarith
    · rw [if_neg h1]
      have h2 : ¬ r ≤ -h := by omega
      rw [if_neg h2]; constructor <;> nlinarith
  · have hr0 := hsign2 (by omega)
    rw [if_neg ha]
    have h1 : ¬ r ≥ h := by omega
    rw [if_neg h1]
    by_cases h2 : r ≤ -h
    · rw [if_pos h2]; constructor <;> nlinarith
    · rw [if_neg h2]; constructor <;> nlinarith

/-- **`div_by_int_half_up` rounds to nearest, ties away from zero, and never
aborts for a positive divisor.** For every `i128` `a` and every `b > 0` it
returns `q` with `IsHalfAway a b q`. No bound on `a`: the `±1` adjustment
cannot overflow because it only happens when `b ≥ 2`.
Production divisors: `MILLISECONDS_PER_YEAR` (`calculate_borrow_rate`) and the
power-of-ten factor in `rescale_half_up`, both positive. -/
theorem div_by_int_half_up_spec (env : Env) (a b : Std.I128) (hb : 0 < b.val) :
    math.fp_core.div_by_int_half_up env a b ⦃ q => IsHalfAway a.val b.val q.val ⦄ := by
  have hab := a.hBounds
  have hbb := b.hBounds
  simp only [IScalarTy.numBits] at hab hbb
  norm_num at hab hbb
  unfold math.fp_core.div_by_int_half_up math.fp_core.require_nonzero_divisor
  have hb0 : ¬ (b = 0#i128) := by intro h; rw [h] at hb; simp at hb
  have hbn : ¬ (b < 0#i128) := by simp [IScalar.lt_equiv]; omega
  simp only [massert, hb0, hbn, not_false_eq_true, ↓reduceIte, bind_ok]
  have hsum : b.val * Int.tdiv a.val b.val + Int.tmod a.val b.val = a.val :=
    Int.mul_tdiv_add_tmod a.val b.val
  have hlt : (Int.tmod a.val b.val).natAbs < b.val.natAbs := by
    rw [Int.natAbs_tmod]; exact Nat.mod_lt _ (Int.natAbs_pos.mpr (by omega))
  have hs1 : 0 ≤ a.val → 0 ≤ Int.tmod a.val b.val := fun h => Int.tmod_nonneg _ h
  have hs2 : a.val < 0 → Int.tmod a.val b.val ≤ 0 := fun h => by
    have := Int.tmod_nonneg b.val (Int.neg_nonneg_of_nonpos (le_of_lt h))
    rw [Int.neg_tmod] at this
    omega
  have htabs : (Int.tdiv a.val b.val).natAbs ≤ a.val.natAbs := by
    rw [Int.natAbs_tdiv]; exact Nat.div_le_self _ _
  step as ⟨t, ht⟩
  step as ⟨r, hr⟩
  step as ⟨h2, hh2⟩
  step as ⟨m2, hm2⟩
  have hh2' : Int.tdiv b.val 2 = b.val / 2 := Int.tdiv_eq_ediv_of_nonneg (by omega)
  have hm2' : Int.tmod b.val 2 = b.val % 2 := Int.tmod_eq_emod_of_nonneg (by omega)
  rw [hh2'] at hh2
  rw [hm2'] at hm2
  step as ⟨h, hh⟩
  have key := halfAway_of_trunc a.val b.val t.val r.val h.val hb (by rw [ht, hr]; exact hsum)
    (by rw [hr]; omega) (by rw [hr]; exact hs1) (by rw [hr]; exact hs2) (by omega)
  have hsum' : b.val * t.val + r.val = a.val := by rw [ht, hr]; exact hsum
  have hrb : -b.val < r.val ∧ r.val < b.val := by rw [hr]; omega
  have hs1' : 0 ≤ a.val → 0 ≤ r.val := by rw [hr]; exact hs1
  have hs2' : a.val < 0 → r.val ≤ 0 := by rw [hr]; exact hs2
  by_cases c1 : r ≥ h
  · have c1' : r.val ≥ h.val := by simpa [IScalar.le_equiv] using c1
    rw [if_pos c1'] at key
    simp only [c1, ↓reduceIte]
    have hr1 : 1 ≤ r.val := by omega
    have hb2 : 2 ≤ b.val := by omega
    have ha0 : 0 < a.val := by
      by_contra hneg
      rcases lt_or_eq_of_le (not_lt.mp hneg) with hlt | heq
      · have := hs2' hlt; omega
      · have : r.val = 0 := by rw [hr, heq]; simp
        omega
    have ht0 : 0 ≤ t.val := by nlinarith
    have : 2 * t.val ≤ b.val * t.val := by nlinarith
    step as ⟨q, hq⟩
    rw [hq]; exact key
  · have c1' : ¬ r.val ≥ h.val := by simpa [IScalar.le_equiv] using c1
    rw [if_neg c1'] at key
    simp only [c1, ↓reduceIte]
    step as ⟨nh, hnh⟩
    by_cases c2 : r ≤ nh
    · have c2' : r.val ≤ -h.val := by simpa [IScalar.le_equiv, hnh] using c2
      rw [if_pos c2'] at key
      simp only [c2, ↓reduceIte]
      have hr1 : r.val ≤ -1 := by omega
      have hb2 : 2 ≤ b.val := by omega
      have ha0 : a.val < 0 := by
        by_contra hneg
        rcases lt_or_eq_of_le (not_lt.mp hneg) with hlt | heq
        · have := hs1' (by omega); omega
        · have : r.val = 0 := by rw [hr, ← heq]; simp
          omega
      have ht0 : t.val ≤ 0 := by nlinarith
      have : b.val * t.val ≤ 2 * t.val := by nlinarith
      step as ⟨q, hq⟩
      rw [hq]; exact key
    · have c2' : ¬ r.val ≤ -h.val := by simpa [IScalar.le_equiv, hnh] using c2
      rw [if_neg c2'] at key
      simp only [c2, ↓reduceIte, WP.spec_ok]
      exact key

/-- **`div_by_int_half_up` aborts on a non-positive divisor.** For `b = 0`
(`DivisionByZero`) and `b < 0` (`MathOverflow`) the call never returns. -/
theorem div_by_int_half_up_aborts (env : Env) (a b : Std.I128) (hb : b.val ≤ 0) :
    ¬ (math.fp_core.div_by_int_half_up env a b ⦃ _ => True ⦄) := by
  unfold math.fp_core.div_by_int_half_up math.fp_core.require_nonzero_divisor
  by_cases hb0 : b = 0#i128
  · simp [massert, hb0]
  · have hbn : b < 0#i128 := by
      have : b.val ≠ 0 := fun h => hb0 (IScalar.eq_of_val_eq (by simp [h]))
      simp [IScalar.lt_equiv]; omega
    simp [massert, hb0, hbn]

end ControllerAeneas.Proofs
