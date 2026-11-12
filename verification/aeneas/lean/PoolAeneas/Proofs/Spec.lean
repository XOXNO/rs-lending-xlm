-- Copied from RatesAeneas/Proofs/Spec.lean (2026-11-12 snapshot) for the pool
-- extraction, with `RatesAeneas`/`rates_aeneas`/`aeneas_soroban_shim` renamed to
-- `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim` and `constants.production.` to
-- `constants.` (WRITING-PROOFS.md, last section). Proof text otherwise unchanged.
/-
Reference semantics for the proofs: exact integer statements that never
divide. `IsFloor n d q` says `q = ⌊n / d⌋`, checked as `q*d ≤ n < q*d + d`
(for `d > 0`; mirrored for `d < 0`), the same formulation the Kani harnesses
use.
-/
module
public import Mathlib.Tactic
@[expose] public section

namespace PoolAeneas.Spec

/-- `q` is `⌊n / d⌋` (rounding toward negative infinity), for `d ≠ 0`. -/
def IsFloor (n d q : Int) : Prop :=
  if 0 < d then q * d ≤ n ∧ n < q * d + d else n ≤ q * d ∧ q * d + d < n

/-- The floor is unique. -/
theorem IsFloor.unique {n d q₁ q₂ : Int} (hd : d ≠ 0)
    (h₁ : IsFloor n d q₁) (h₂ : IsFloor n d q₂) : q₁ = q₂ := by
  unfold IsFloor at h₁ h₂
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

/-- The candidate both production paths compute: truncating division, minus
one when the division is inexact and the operands have opposite signs. -/
def truncFloor (n d : Int) (inexact : Bool) : Int :=
  if inexact ∧ ((n < 0) ≠ (d < 0)) then Int.tdiv n d - 1 else Int.tdiv n d

/-- Core lemma: the truncation-based candidate is the floor, provided
`inexact` says exactly whether `d` divides `n`. -/
theorem truncFloor_isFloor (n d : Int) (hd : d ≠ 0) (inexact : Bool)
    (hinex : inexact = true ↔ ¬ d ∣ n) : IsFloor n d (truncFloor n d inexact) := by
  unfold truncFloor IsFloor
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
    · rw [if_neg (by simp [hn, hneg])]
      constructor <;> nlinarith [hsign_nonpos (by omega)]
    · have hr : r ≠ 0 := by
        intro h0; exact (hinex.mp hi) (hdvd.mpr h0)
      rw [if_pos (by simp [hi, hn, hneg])]
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
  · -- d > 0
    have hdabs : (d.natAbs : Int) = d := by omega
    rw [if_pos hpos]
    by_cases hi : inexact = true <;> by_cases hn : n < 0
    · have hr : r ≠ 0 := by
        intro h0; exact (hinex.mp hi) (hdvd.mpr h0)
      rw [if_pos (by simp [hi, hn, show ¬ d < 0 by omega])]
      have hr' : r < 0 := lt_of_le_of_ne (hsign_nonpos (by omega)) hr
      constructor <;> nlinarith
    · rw [if_neg (by simp [hn, show ¬ d < 0 by omega])]
      have := hsign_nonneg (by omega)
      constructor <;> nlinarith
    · have hr : r = 0 := by
        by_contra h0; exact hi (hinex.mpr (fun hd' => h0 (hdvd.mp hd')))
      rw [if_neg (by simp [hi])]
      constructor <;> nlinarith
    · have hr : r = 0 := by
        by_contra h0; exact hi (hinex.mpr (fun hd' => h0 (hdvd.mp hd')))
      rw [if_neg (by simp [hi])]
      constructor <;> nlinarith

end PoolAeneas.Spec
