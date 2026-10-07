import Std

namespace LowWord

def encodedLow (x : Int) : Int :=
  (if x < 0 then x + 340282366920938463463374607431768211456 else x)
    % 18446744073709551616

def low (x : Int) : Int := x % 18446744073709551616

theorem integer_eq (x : Int) : encodedLow x = low x := by
  have hm : (340282366920938463463374607431768211456 : Int)
      % 18446744073709551616 = 0 := by decide
  unfold encodedLow low
  split <;> simp [Int.add_emod, hm]

-- X is the only potentially undefined dependency. All constants are defined,
-- integer arithmetic/comparison is total, and the fixed modulus is nonzero.
def encodedLowOption (x : Option Int) : Option Int := x.map encodedLow
def lowOption (x : Option Int) : Option Int := x.map low

theorem option_eq (x : Option Int) : encodedLowOption x = lowOption x := by
  cases x with
  | none => rfl
  | some v => exact congrArg some (integer_eq v)

theorem undefined_iff (x : Option Int) :
    encodedLowOption x = none ↔ x = none := by
  cases x <;> simp [encodedLowOption]

theorem definedness_eq (x : Option Int) :
    (encodedLowOption x = none) = (lowOption x = none) := by
  rw [option_eq]

end LowWord

#print axioms LowWord.integer_eq
#print axioms LowWord.option_eq
#print axioms LowWord.undefined_iff
#print axioms LowWord.definedness_eq
