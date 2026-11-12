-- Snapshot of RatesAeneas/Proofs/FloorCeilSpec.lean taken 2026-11-12 (namespace changed); see WRITING-PROOFS.md, last section.
/-
Reference semantics for ceiling division, in the style of
`ControllerAeneas.Spec.IsFloor`: exact integer statements that never divide.

`IsCeil n d q` says `q = ⌈n / d⌉`, checked as `n ≤ q*d < n + d` for `d > 0`
(mirrored for `d < 0`).
-/
import ControllerAeneas.Proofs.Spec

namespace ControllerAeneas.Spec

/-- `q` is `⌈n / d⌉` (rounding toward positive infinity), for `d ≠ 0`.
For `d > 0`: `n ≤ q*d < n + d`. For `d < 0` (multiplying `q - 1 < n/d ≤ q`
by a negative number): `q*d ≤ n < q*d - d`. -/
def IsCeil (n d q : Int) : Prop :=
  if 0 < d then n ≤ q * d ∧ q * d < n + d else q * d ≤ n ∧ n + d < q * d

/-- The ceiling is unique. -/
theorem IsCeil.unique {n d q₁ q₂ : Int} (hd : d ≠ 0)
    (h₁ : IsCeil n d q₁) (h₂ : IsCeil n d q₂) : q₁ = q₂ := by
  unfold IsCeil at h₁ h₂
  rcases lt_or_gt_of_ne hd with hneg | hpos
  · rw [if_neg (by omega)] at h₁ h₂
    rcases lt_trichotomy q₁ q₂ with hlt | heq | hgt
    · have : q₂ * d ≤ (q₁ + 1) * d := mul_le_mul_of_nonpos_right (by omega) (by omega)
      nlinarith
    · exact heq
    · have : q₁ * d ≤ (q₂ + 1) * d := mul_le_mul_of_nonpos_right (by omega) (by omega)
      nlinarith
  · rw [if_pos hpos] at h₁ h₂
    rcases lt_trichotomy q₁ q₂ with hlt | heq | hgt
    · have : (q₁ + 1) * d ≤ q₂ * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    · exact heq
    · have : (q₂ + 1) * d ≤ q₁ * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith

/-- Floor and ceiling of a positive-divisor quotient differ by at most one,
and agree exactly when the division is exact. Stated as: the floor is at most
the ceiling. -/
theorem IsFloor.le_IsCeil {n d f c : Int} (hd : 0 < d) (hf : IsFloor n d f)
    (hc : IsCeil n d c) : f ≤ c := by
  unfold IsFloor at hf; unfold IsCeil at hc
  rw [if_pos hd] at hf hc
  by_contra h
  have : (c + 1) * d ≤ f * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
  nlinarith

/-- The candidate both production ceiling paths compute: truncating division,
plus one when the division is inexact and the operands have the same sign
(the exact quotient is positive). -/
def truncCeil (n d : Int) (inexact : Bool) : Int :=
  if inexact ∧ ((n < 0) = (d < 0)) then Int.tdiv n d + 1 else Int.tdiv n d

/-- Core lemma: the truncation-based candidate is the ceiling, provided
`inexact` says exactly whether `d` divides `n`. -/
theorem truncCeil_isCeil (n d : Int) (hd : d ≠ 0) (inexact : Bool)
    (hinex : inexact = true ↔ ¬ d ∣ n) : IsCeil n d (truncCeil n d inexact) := by
  unfold truncCeil IsCeil
  have hsum : d * Int.tdiv n d + Int.tmod n d = n := Int.mul_tdiv_add_tmod n d
  have hlt : (Int.tmod n d).natAbs < d.natAbs := by
    rw [Int.natAbs_tmod]; exact Nat.mod_lt _ (Int.natAbs_pos.mpr hd)
  have hdvd : d ∣ n ↔ Int.tmod n d = 0 := Int.dvd_iff_tmod_eq_zero
  have hsign_nonneg : 0 ≤ n → 0 ≤ Int.tmod n d := fun h => Int.tmod_nonneg d h
  have hsign_nonpos : n ≤ 0 → Int.tmod n d ≤ 0 := fun h => by
    have := Int.tmod_nonneg d (Int.neg_nonneg_of_nonpos h)
    rw [Int.neg_tmod] at this
    omega
  set t := Int.tdiv n d
  set r := Int.tmod n d
  have hrabs : r < d.natAbs ∧ -r < d.natAbs := by omega
  rcases lt_or_gt_of_ne hd with hneg | hpos
  · -- d < 0
    have hdabs : (d.natAbs : Int) = -d := by omega
    rw [if_neg (by omega)]
    by_cases hi : inexact = true <;> by_cases hn : n < 0
    · have hr : r ≠ 0 := by
        intro h0; exact (hinex.mp hi) (hdvd.mpr h0)
      rw [if_pos (by simp [hi, hn, hneg])]
      have hr' : r < 0 := lt_of_le_of_ne (hsign_nonpos (by omega)) hr
      constructor <;> nlinarith
    · rw [if_neg (by simp [hn, hneg])]
      constructor <;> nlinarith [hsign_nonneg (by omega)]
    · have hr : r = 0 := by
        by_contra h0; exact hi (hinex.mpr (fun hd' => h0 (hdvd.mp hd')))
      rw [if_neg (by simp [hi])]
      constructor <;> nlinarith
    · have hr : r = 0 := by
        by_contra h0; exact hi (hinex.mpr (fun hd' => h0 (hdvd.mp hd')))
      rw [if_neg (by simp [hi])]
      constructor <;> nlinarith
  · -- d > 0
    have hdabs : (d.natAbs : Int) = d := by omega
    rw [if_pos hpos]
    by_cases hi : inexact = true <;> by_cases hn : n < 0
    · rw [if_neg (by simp [hn, show ¬ d < 0 by omega])]
      have := hsign_nonpos (by omega)
      constructor <;> nlinarith
    · have hr : r ≠ 0 := by
        intro h0; exact (hinex.mp hi) (hdvd.mpr h0)
      rw [if_pos (by simp [hi, hn, show ¬ d < 0 by omega])]
      have hr' : 0 < r := lt_of_le_of_ne (hsign_nonneg (by omega)) (Ne.symm hr)
      constructor <;> nlinarith
    · have hr : r = 0 := by
        by_contra h0; exact hi (hinex.mpr (fun hd' => h0 (hdvd.mp hd')))
      rw [if_neg (by simp [hi])]
      constructor <;> nlinarith
    · have hr : r = 0 := by
        by_contra h0; exact hi (hinex.mpr (fun hd' => h0 (hdvd.mp hd')))
      rw [if_neg (by simp [hi])]
      constructor <;> nlinarith

end ControllerAeneas.Spec
