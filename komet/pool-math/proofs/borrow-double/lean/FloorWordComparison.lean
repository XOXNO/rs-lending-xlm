import Lean.Elab.Tactic.Omega

/-
Integer arithmetic model only, not a K definedness or Wasm-refinement proof.
K `/Int` maps to Int.tdiv (truncation toward zero); `modInt` maps to `%`
(Int.emod, Euclidean remainder). For a positive divisor these operations are
defined on all integer operands. A K simplification must retain X, B, N and
their default definedness obligations, and require 0 < B. Lean's division
and modulo by zero are totalized, so they do not model K's partial zero case.
-/
namespace FloorWordComparison

def word : Int := 18446744073709551616

-- Removing the Euclidean remainder gives an exact multiple, even for X < 0.
theorem cancelled_quotient (x b : Int) (hb : b ≠ 0) :
    (x - x % b).tdiv b = x / b := by
  apply Int.tdiv_eq_of_eq_mul_right hb
  have := Int.ediv_add_emod x b
  omega

-- Both directions hold, with no sign restriction on X or N.
theorem cancelled_lt_iff (x b n : Int) (hb : 0 < b) :
    (x - x % b).tdiv b < n ↔ x < n * b := by
  rw [cancelled_quotient x b (Int.ne_of_gt hb)]
  exact Int.ediv_lt_iff_lt_mul hb

theorem word_eq_pow64 : word = (2 : Int)^64 := by decide

theorem word_lt_iff (x n : Int) :
    (x - x % word).tdiv word < n ↔ x < n * word :=
  cancelled_lt_iff x word n (by decide)

theorem word_lt_implication (x n : Int)
    (h : (x - x % word).tdiv word < n) : x < n * word :=
  (word_lt_iff x n).mp h

-- Negative nonmultiples distinguish the cancelled quotient from raw tdiv.
theorem negative_boundary_witnesses :
    ((-1 : Int) - (-1 : Int) % word).tdiv word = -1 ∧
    (-1 : Int).tdiv word = 0 ∧
    ((-word - 1) - (-word - 1) % word).tdiv word = -2 ∧
    ((-word) - (-word) % word).tdiv word = -1 ∧
    ((-word + 1) - (-word + 1) % word).tdiv word = -1 := by decide

-- At a negative threshold N = -1, the exact boundary must remain false.
theorem negative_threshold_witnesses :
    ((-word - 1) - (-word - 1) % word).tdiv word < -1 ∧
    ¬ (((-word) - (-word) % word).tdiv word < -1) ∧
    ¬ (((-word + 1) - (-word + 1) % word).tdiv word < -1) := by decide

theorem negative_divisor_counterexample :
    ((0 : Int) - (0 : Int) % (-1)).tdiv (-1) < 1 ∧
    ¬ ((0 : Int) < 1 * (-1)) := by decide

#print axioms cancelled_quotient
#print axioms cancelled_lt_iff
#print axioms word_eq_pow64
#print axioms word_lt_iff
#print axioms word_lt_implication
#print axioms negative_boundary_witnesses
#print axioms negative_threshold_witnesses
#print axioms negative_divisor_counterexample

end FloorWordComparison
