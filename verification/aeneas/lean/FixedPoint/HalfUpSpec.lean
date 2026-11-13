import FixedPoint.Spec

/-
Reference semantics for half-up and nearest-with-ties-away-from-zero
rounding, stated without division, in the style of `FixedPoint.Spec.IsFloor`.

* `IsHalfUp n d q` (for `d > 0`): `q = ⌊n/d + 1/2⌋`, i.e. `n/d` rounded to the
  nearest integer with exact halves rounded up. Checked as
  `2·q·d ≤ 2·n + d < 2·q·d + 2·d`, which is `IsFloor (2n + d) (2d) q`.
* `IsHalfAway n d q` (for `d > 0`): `n/d` rounded to the nearest integer with
  exact halves rounded away from zero: `IsHalfUp n d q` when `n ≥ 0`, and the
  mirror image `IsHalfUp (-n) d (-q)` when `n < 0`.

Neither definition divides.
-/

open FixedPoint.Spec

namespace FixedPoint.HalfUpSpec

/-- `q` is `n / d` rounded to nearest, exact halves up (`d > 0`). -/
def IsHalfUp (n d q : Int) : Prop :=
  2 * q * d ≤ 2 * n + d ∧ 2 * n + d < 2 * q * d + 2 * d

/-- `q` is `n / d` rounded to nearest, exact halves away from zero (`d > 0`). -/
def IsHalfAway (n d q : Int) : Prop :=
  if 0 ≤ n then IsHalfUp n d q else IsHalfUp (-n) d (-q)

/-- `IsHalfUp` is the floor of `(2n + d) / (2d)`. -/
theorem IsHalfUp.iff_isFloor {n d q : Int} (hd : 0 < d) :
    IsHalfUp n d q ↔ IsFloor (2 * n + d) (2 * d) q := by
  unfold IsHalfUp IsFloor
  rw [if_pos (by omega)]
  constructor <;> rintro ⟨h1, h2⟩ <;> constructor <;> linarith

/-- The half-up rounding is unique. -/
theorem IsHalfUp.unique {n d q₁ q₂ : Int} (hd : 0 < d)
    (h₁ : IsHalfUp n d q₁) (h₂ : IsHalfUp n d q₂) : q₁ = q₂ :=
  IsFloor.unique (by omega) ((IsHalfUp.iff_isFloor hd).1 h₁) ((IsHalfUp.iff_isFloor hd).1 h₂)

/-- The half-up rounding of a non-negative numerator is non-negative. -/
theorem IsHalfUp.nonneg {n d q : Int} (hd : 0 < d) (hn : 0 ≤ n) (h : IsHalfUp n d q) :
    0 ≤ q := by
  unfold IsHalfUp at h
  by_contra hq
  have : 2 * q * d + 2 * d ≤ 0 := by nlinarith
  linarith

/-- Half-up rounding is monotone in the numerator. -/
theorem IsHalfUp.mono {n₁ n₂ d q₁ q₂ : Int} (hd : 0 < d) (hn : n₁ ≤ n₂)
    (h₁ : IsHalfUp n₁ d q₁) (h₂ : IsHalfUp n₂ d q₂) : q₁ ≤ q₂ := by
  unfold IsHalfUp at h₁ h₂
  by_contra h
  have : 2 * (q₂ + 1) * d ≤ 2 * q₁ * d := by
    have : q₂ + 1 ≤ q₁ := by omega
    nlinarith
  nlinarith

/-- Half-up rounding is monotone in the divisor, decreasing: a larger divisor
never gives a larger result (non-negative numerator). -/
theorem IsHalfUp.anti_div {n d₁ d₂ q₁ q₂ : Int} (hn : 0 ≤ n) (hd : 0 < d₁) (hdd : d₁ ≤ d₂)
    (h₁ : IsHalfUp n d₁ q₁) (h₂ : IsHalfUp n d₂ q₂) : q₂ ≤ q₁ := by
  unfold IsHalfUp at h₁ h₂
  have hq2 : 0 ≤ q₂ := IsHalfUp.nonneg (by omega) hn h₂
  by_contra h
  have h' : q₁ + 1 ≤ q₂ := by omega
  -- 2n + d₂ ≥ 2 q₂ d₂ ≥ 2 (q₁+1) d₂ and 2n + d₁ < 2 (q₁+1) d₁
  have e1 : 2 * (q₁ + 1) * d₂ ≤ 2 * q₂ * d₂ := by nlinarith
  have e2 : 2 * (q₁ + 1) * (d₂ - d₁) ≥ (d₂ - d₁) := by
    have : 1 ≤ q₁ + 1 := by
      have := IsHalfUp.nonneg hd hn h₁; omega
    nlinarith
  nlinarith

/-- The value production computes on non-negative operands,
`trunc((n + trunc(d / 2)) / d)`, is the half-up rounding of `n / d`. -/
theorem isHalfUp_tdiv (n d : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    IsHalfUp n d (Int.tdiv (n + Int.tdiv d 2) d) := by
  have h2 : Int.tdiv d 2 = d / 2 := Int.tdiv_eq_ediv_of_nonneg (by omega)
  rw [h2]
  have hpos : 0 ≤ n + d / 2 := by omega
  rw [Int.tdiv_eq_ediv_of_nonneg hpos]
  have hs := Int.mul_ediv_add_emod (n + d / 2) d
  have hs0 := Int.emod_nonneg (n + d / 2) (by omega : d ≠ 0)
  have hs1 := Int.emod_lt_of_pos (n + d / 2) hd
  have hh : d = 2 * (d / 2) + d % 2 := by omega
  have hr : 0 ≤ d % 2 ∧ d % 2 ≤ 1 := by omega
  set q := (n + d / 2) / d
  set s := (n + d / 2) % d
  unfold IsHalfUp
  constructor <;> nlinarith

/-- Half-up of a non-negative numerator is also the ties-away rounding. -/
theorem IsHalfUp.isHalfAway {n d q : Int} (hn : 0 ≤ n) (h : IsHalfUp n d q) :
    IsHalfAway n d q := by
  unfold IsHalfAway; rw [if_pos hn]; exact h

/-- Every quotient with a positive divisor has a half-up rounding. (The
witness divides; the specification does not.) -/
theorem IsHalfUp.exists (n d : Int) (hd : 0 < d) : ∃ q, IsHalfUp n d q := by
  refine ⟨(2 * n + d) / (2 * d), ?_⟩
  have h1 := Int.mul_ediv_add_emod (2 * n + d) (2 * d)
  have h2 := Int.emod_nonneg (2 * n + d) (by omega : 2 * d ≠ 0)
  have h3 := Int.emod_lt_of_pos (2 * n + d) (by omega : 0 < 2 * d)
  unfold IsHalfUp
  constructor <;> nlinarith

/-- An exact quotient rounds to itself: `half_up(x*d/d) = x`. -/
theorem IsHalfUp.mul_self (x d : Int) (hd : 0 < d) : IsHalfUp (x * d) d x := by
  unfold IsHalfUp; constructor <;> nlinarith

/-- Scaling by a ratio at most one: for `0 ≤ y ≤ d`, `half_up(x*y/d) ≤ x`
(`x ≥ 0`), and it is non-negative. -/
theorem IsHalfUp.le_of_ratio_le_one {x y d q : Int} (hd : 0 < d) (hx : 0 ≤ x) (hy : 0 ≤ y)
    (hyd : y ≤ d) (h : IsHalfUp (x * y) d q) : 0 ≤ q ∧ q ≤ x :=
  ⟨IsHalfUp.nonneg hd (Int.mul_nonneg hx hy) h,
   IsHalfUp.mono hd (Int.mul_le_mul_of_nonneg_left hyd hx) h (IsHalfUp.mul_self x d hd)⟩

/-- Upper bound without division: `2·q·d ≤ 2·n + d`, so `q·d ≤ n + d/2`. -/
theorem IsHalfUp.mul_le {n d q : Int} (h : IsHalfUp n d q) : 2 * (q * d) ≤ 2 * n + d := by
  unfold IsHalfUp at h; linarith [h.1]

/-- Lower bound without division: `2·n + d < 2·q·d + 2·d`, so
`n < q·d + d/2`, in particular `n - d < q·d`. -/
theorem IsHalfUp.lt_mul {n d q : Int} (h : IsHalfUp n d q) : 2 * n < 2 * (q * d) + d := by
  unfold IsHalfUp at h; linarith [h.2]

end FixedPoint.HalfUpSpec
