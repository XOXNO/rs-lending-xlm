/-
Reference semantics for the oracle proofs: exact integer statements that
never divide, in the style of `RatesAeneas/Proofs/Spec.lean`.
-/
module
public import Mathlib.Tactic
@[expose] public section

namespace OracleAeneas.Spec

/-- `q` is `n / d` rounded half up (`⌊(n + ⌊d/2⌋)/d⌋`), for `d > 0`, stated
without division: `2qd ≤ 2n + d < 2qd + 2d`. For odd `d` the two forms agree
because `2qd` is even and `2n + d` odd. -/
def IsHalfUp (n d q : Int) : Prop :=
  2 * q * d ≤ 2 * n + d ∧ 2 * n + d < 2 * q * d + 2 * d

/-- `q` is `⌊n / d⌋` for `d > 0`: `q*d ≤ n < q*d + d`. -/
def IsFloorPos (n d q : Int) : Prop :=
  q * d ≤ n ∧ n < q * d + d

/-- The half-up quotient is unique. -/
theorem IsHalfUp.unique {n d q₁ q₂ : Int} (hd : 0 < d)
    (h₁ : IsHalfUp n d q₁) (h₂ : IsHalfUp n d q₂) : q₁ = q₂ := by
  unfold IsHalfUp at h₁ h₂
  rcases lt_trichotomy q₁ q₂ with hlt | heq | hgt
  · have : (q₁ + 1) * d ≤ q₂ * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith
  · exact heq
  · have : (q₂ + 1) * d ≤ q₁ * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith

/-- The floor is unique. -/
theorem IsFloorPos.unique {n d q₁ q₂ : Int} (hd : 0 < d)
    (h₁ : IsFloorPos n d q₁) (h₂ : IsFloorPos n d q₂) : q₁ = q₂ := by
  unfold IsFloorPos at h₁ h₂
  rcases lt_trichotomy q₁ q₂ with hlt | heq | hgt
  · have : (q₁ + 1) * d ≤ q₂ * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith
  · exact heq
  · have : (q₂ + 1) * d ≤ q₁ * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith

/-- `(n + ⌊d/2⌋) / d` (Euclidean division, here equal to truncation since the
numerator is not negative) is the half-up quotient. -/
theorem halfUp_of_ediv (n d : Int) (hd : 0 < d) :
    IsHalfUp n d ((n + d / 2) / d) := by
  have h1 := Int.emod_add_mul_ediv (n + d / 2) d
  have h2 := Int.emod_nonneg (n + d / 2) (show d ≠ 0 by omega)
  have h3 := Int.emod_lt_of_pos (n + d / 2) hd
  have h4 : 2 * (d / 2) ≤ d ∧ d ≤ 2 * (d / 2) + 1 := by omega
  generalize (n + d / 2) / d = q at *
  generalize (n + d / 2) % d = r at *
  generalize d / 2 = h at *
  unfold IsHalfUp
  constructor <;> nlinarith

/-- With `d = 1` the half-up quotient is the number itself. -/
theorem IsHalfUp.one {n q : Int} (h : IsHalfUp n 1 q) : q = n := by
  unfold IsHalfUp at h; omega

/-- `n / d` (Euclidean) is the floor for `d > 0`. -/
theorem floorPos_of_ediv (n d : Int) (hd : 0 < d) : IsFloorPos n d (n / d) := by
  have h1 := Int.emod_add_mul_ediv n d
  have h2 := Int.emod_nonneg n (show d ≠ 0 by omega)
  have h3 := Int.emod_lt_of_pos n hd
  generalize n / d = q at *
  generalize n % d = r at *
  unfold IsFloorPos
  constructor <;> nlinarith

end OracleAeneas.Spec
