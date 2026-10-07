import CarryWord

/-
All identities quantify over every signed Int; no path or range guard is added.
K /Int is modeled by truncating Int.tdiv, positive modInt by Euclidean remainder.
Reuse the frozen signed-magnitude shift model, restricted here to literal count 1.
This proves arithmetic and strict Option semantics, not native K/Wasm refinement.
-/
namespace DoubleWord

open CarryWord SignedDecoder

theorem fixed_divisors_and_shift : M ≠ 0 ∧ C ≠ 0 ∧ (1 : Int) ≥ 0 :=
  CarryWord.fixed_constants

theorem doubled_low_int (x : Int) :
    ((x % M) <<< (1 : Nat)) % M = (x * 2) % M := by
  rw [Int.shiftLeft_one, Int.mul_emod (x % M) 2 M, Int.emod_emod,
    ← Int.mul_emod x 2 M]

-- Exact expanded truncation quotients, including negative x.
theorem doubled_high_int (x : Int) :
    ((x - x % M).tdiv M) * 2 + ((x % M - (x % M) % C).tdiv C) =
      (x * 2 - (x * 2) % M).tdiv M :=
  (CarryWord.doubledHigh_eq x).symm

def doubledLowLeftOption (x : Option Int) : Option Int :=
  strictEmod (strictShiftOne (strictEmod x (some M))) (some M)

def doubledLowRightOption (x : Option Int) : Option Int :=
  strictEmod (strictBinary (· * ·) x (some 2)) (some M)

def doubledHighLeftOption (x : Option Int) : Option Int :=
  strictBinary (· + ·) (strictBinary (· * ·) (highOption x) (some 2)) (carryOption x)

-- Every arithmetic operator is strict; every side retains the sole input x.
theorem strict_doubled_low (x : Option Int) :
    doubledLowLeftOption x = doubledLowRightOption x := by
  cases x with
  | none => rfl
  | some v =>
    change some (((v % M) <<< (1 : Nat)) % M) = some ((v * 2) % M)
    exact congrArg some (doubled_low_int v)

theorem strict_doubled_high (x : Option Int) :
    doubledHighLeftOption x = doubledHighOption x := by
  cases x with
  | none => rfl
  | some v =>
    change some (((v - v % M).tdiv M) * 2 + ((v % M - (v % M) % C).tdiv C)) =
      some ((v * 2 - (v * 2) % M).tdiv M)
    exact congrArg some (doubled_high_int v)

theorem strict_doubled_low_defined (x : Option Int) :
    (Defined (doubledLowLeftOption x) ↔ Defined x) ∧
    (Defined (doubledLowRightOption x) ↔ Defined x) := by
  cases x <;> simp [Defined, doubledLowLeftOption, doubledLowRightOption,
    strictShiftOne, strictEmod, strictBinary, M]

theorem strict_doubled_low_undefined (x : Option Int) :
    (doubledLowLeftOption x = none ↔ x = none) ∧
    (doubledLowRightOption x = none ↔ x = none) := by
  cases x <;> simp [doubledLowLeftOption, doubledLowRightOption,
    strictShiftOne, strictEmod, strictBinary, M]

theorem strict_doubled_high_defined (x : Option Int) :
    (Defined (doubledHighLeftOption x) ↔ Defined x) ∧
    (Defined (doubledHighOption x) ↔ Defined x) := by
  cases x <;> simp [Defined, doubledHighLeftOption, highOption, carryOption,
    doubledHighOption, strictTdiv, strictEmod, strictBinary, M, C]

theorem strict_doubled_high_undefined (x : Option Int) :
    (doubledHighLeftOption x = none ↔ x = none) ∧
    (doubledHighOption x = none ↔ x = none) := by
  cases x <;> simp [doubledHighLeftOption, highOption, carryOption,
    doubledHighOption, strictTdiv, strictEmod, strictBinary, M, C]

end DoubleWord

#print axioms DoubleWord.fixed_divisors_and_shift
#print axioms DoubleWord.doubled_low_int
#print axioms DoubleWord.doubled_high_int
#print axioms DoubleWord.strict_doubled_low
#print axioms DoubleWord.strict_doubled_high
#print axioms DoubleWord.strict_doubled_low_defined
#print axioms DoubleWord.strict_doubled_low_undefined
#print axioms DoubleWord.strict_doubled_high_defined
#print axioms DoubleWord.strict_doubled_high_undefined
