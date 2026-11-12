/-
Division-free facts relating floor, half-up and ceiling quotients by a
positive divisor. Every statement is about the specification predicates
`IsFloor`, `IsCeil` (`Spec.lean`, `FloorCeilSpec.lean`) and `IsHalfUp`
(`HalfUpSpec.lean`); nothing here mentions generated code.
-/
import ControllerAeneas.Proofs.FloorCeilSpec
import ControllerAeneas.Proofs.HalfUpSpec

open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec

namespace ControllerAeneas.Proofs

theorem isFloor_pos {n d q : Int} (hd : 0 < d) :
    IsFloor n d q ↔ q * d ≤ n ∧ n < q * d + d := by
  unfold IsFloor; rw [if_pos hd]

theorem isCeil_pos {n d q : Int} (hd : 0 < d) :
    IsCeil n d q ↔ n ≤ q * d ∧ q * d < n + d := by
  unfold IsCeil; rw [if_pos hd]

/-- The floor exists (it is `Int.ediv`). -/
theorem isFloor_exists (n d : Int) (hd : 0 < d) : ∃ q, IsFloor n d q := by
  refine ⟨n / d, (isFloor_pos hd).mpr ⟨Int.ediv_mul_le n (by omega), ?_⟩⟩
  have := Int.lt_ediv_add_one_mul_self n hd
  linarith

/-- The ceiling exists (it is `-((-n) / d)`). -/
theorem isCeil_exists (n d : Int) (hd : 0 < d) : ∃ q, IsCeil n d q := by
  obtain ⟨f, hf⟩ := isFloor_exists (-n) d hd
  rw [isFloor_pos hd] at hf
  exact ⟨-f, (isCeil_pos hd).mpr ⟨by linarith [hf.1], by linarith [hf.2]⟩⟩

/-- A floor is at most any `c` with `n ≤ c*d`. -/
theorem IsFloor.le_of {n d f c : Int} (hd : 0 < d) (hf : IsFloor n d f) (h : n ≤ c * d) :
    f ≤ c := by
  rw [isFloor_pos hd] at hf
  by_contra hc
  have : (c + 1) * d ≤ f * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
  linarith [hf.1]

/-- A ceiling is at least any `f` with `f*d ≤ n`. -/
theorem IsCeil.ge_of {n d f c : Int} (hd : 0 < d) (hc : IsCeil n d c) (h : f * d ≤ n) :
    f ≤ c := by
  rw [isCeil_pos hd] at hc
  by_contra hh
  have : (c + 1) * d ≤ f * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
  linarith [hc.2]

/-- A half-up quotient is at most any `c` with `n ≤ c*d`. -/
theorem IsHalfUp.le_of {n d h c : Int} (hd : 0 < d) (hh : IsHalfUp n d h) (hn : n ≤ c * d) :
    h ≤ c := by
  unfold IsHalfUp at hh
  by_contra hc
  have : (c + 1) * d ≤ h * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
  nlinarith [hh.1]

/-- A half-up quotient is at least any `f` with `f*d ≤ n`. -/
theorem IsHalfUp.ge_of {n d h f : Int} (hd : 0 < d) (hh : IsHalfUp n d h) (hn : f * d ≤ n) :
    f ≤ h := by
  unfold IsHalfUp at hh
  by_contra hc
  have : (h + 1) * d ≤ f * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
  nlinarith [hh.2]

/-- The floor of a non-negative quotient is non-negative. -/
theorem IsFloor.nonneg_of {n d f : Int} (hd : 0 < d) (hf : IsFloor n d f) (hn : 0 ≤ n) :
    0 ≤ f := by
  rw [isFloor_pos hd] at hf
  by_contra h
  have : (f + 1) * d ≤ 0 * d := mul_le_mul_of_nonneg_right (by omega) (by omega)
  linarith [hf.2]

end ControllerAeneas.Proofs
