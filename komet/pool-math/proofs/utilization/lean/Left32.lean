import HalfWord

-- Literal count 32 only. Reuse the signed theorem; no operand range guard.
namespace Left32Review

theorem factor32 : ((2 ^ (32 : Nat) : Nat) : Int) = 4294967296 := by decide

theorem signed_left32 (x : Int) :
    x <<< (32 : Nat) = x * 4294967296 := by
  simpa only [factor32] using Int.shiftLeft_eq_mul x 32

-- Both K operators are strict; the fixed second operands are defined values.
def left (x : Option Int) : Option Int :=
  Fixed64Packing.strict2 Int.shiftLeft x (some (32 : Nat))

def right (x : Option Int) : Option Int :=
  Fixed64Packing.strict2 (· * ·) x (some (4294967296 : Int))

theorem strict_equality (x : Option Int) : left x = right x := by
  cases x with
  | none => rfl
  | some v => exact congrArg some (signed_left32 v)

theorem strict_defined (x : Option Int) :
    (left x ≠ none ↔ x ≠ none) ∧ (right x ≠ none ↔ x ≠ none) := by
  cases x <;> simp [left, right, Fixed64Packing.strict2]

theorem strict_absence (x : Option Int) :
    (left x = none ↔ x = none) ∧ (right x = none ↔ x = none) := by
  cases x <;> simp [left, right, Fixed64Packing.strict2]

end Left32Review

#print axioms Left32Review.factor32
#print axioms Left32Review.signed_left32
#print axioms Left32Review.strict_equality
#print axioms Left32Review.strict_defined
#print axioms Left32Review.strict_absence
