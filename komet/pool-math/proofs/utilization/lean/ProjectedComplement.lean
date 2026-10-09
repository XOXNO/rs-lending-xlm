import UtilizationAnd
import Fixed64Packing

namespace UtilizationCarry

open UtilizationAnd

abbrev Mask : Nat := 18446744073709551615

/-- Infinite two's-complement XOR, independent of projection and conclusion. -/
def signedXor : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a ^^^ b)
  | .negSucc a, .ofNat b => .negSucc (a ^^^ b)
  | .ofNat a, .negSucc b => .negSucc (a ^^^ b)
  | .negSucc a, .negSucc b => .ofNat (a ^^^ b)

theorem signedXor_bits (a b : Int) (i : Nat) :
    ByteWrap.signedBit (signedXor a b) i =
      (ByteWrap.signedBit a i ^^ ByteWrap.signedBit b i) := by
  cases a <;> cases b <;>
    simp only [signedXor, ByteWrap.signedBit, Nat.testBit_xor] <;>
    cases Nat.testBit _ i <;> cases Nat.testBit _ i <;> rfl

/-- K u64Xor: primitive signed XOR followed by the low64 projection. -/
def projectedXor (a b : Int) : Int := ByteWrap.andNatMask (signedXor a b) Mask

theorem exact_mask : (M - 1 : Int) = Int.ofNat Mask ∧ Mask = 2 ^ 64 - 1 := by
  decide

theorem projectedXor_native (a b : Int) :
    projectedXor a b = signedXor a b % M := by
  rw [projectedXor, exact_mask.2]
  exact ByteWrap.and_power_mask_eq_mod (signedXor a b) 64

theorem projected_complement_bits (x : Int) (i : Nat) :
    ByteWrap.signedBit (projectedXor x (M - 1)) i =
      (!ByteWrap.signedBit x i && decide (i < 64)) := by
  rw [projectedXor, ByteWrap.andNatMask_bits, signedXor_bits, exact_mask.1]
  change ((ByteWrap.signedBit x i ^^ Nat.testBit Mask i) && Nat.testBit Mask i) =
    (!ByteWrap.signedBit x i && decide (i < 64))
  rw [exact_mask.2, Nat.testBit_two_pow_sub_one]
  cases ByteWrap.signedBit x i <;> cases decide (i < 64) <;> rfl

theorem complement_and_zero (x : Int) :
    signedAnd (projectedXor x (M - 1)) x = 0 := by
  apply Int.eq_of_testBit_eq
  intro i
  change ByteWrap.signedBit (signedAnd (projectedXor x (M - 1)) x) i =
    ByteWrap.signedBit 0 i
  rw [signedAnd_bits, projected_complement_bits]
  have hz : ByteWrap.signedBit 0 i = false := by simp [ByteWrap.signedBit]
  rw [hz]
  cases ByteWrap.signedBit x i <;> cases decide (i < 64) <;> rfl

theorem complement_and_sub (x : Int) :
    signedAnd (projectedXor x (M - 1)) x = x - x := by
  rw [complement_and_zero, Int.sub_self]

def left (x : Option Int) : Option Int :=
  strict2 signedAnd (strict2 projectedXor x (some (M - 1))) x
def right (x : Option Int) : Option Int := strict2 (· - ·) x x

/-- All signed values, including absent partial arguments; no range guard. -/
theorem strict_equivalence (x : Option Int) : left x = right x := by
  cases x with
  | none => rfl
  | some x => exact congrArg some (complement_and_sub x)

theorem strict_definedness (x : Option Int) :
    (left x ≠ none ↔ x ≠ none) ∧ (right x ≠ none ↔ x ≠ none) := by
  cases x <;> simp [left, right, strict2]

theorem erasing_partial_is_wrong : left none ≠ some 0 := by decide

end UtilizationCarry

#print axioms UtilizationCarry.signedXor_bits
#print axioms UtilizationCarry.projectedXor_native
#print axioms UtilizationCarry.complement_and_zero
#print axioms UtilizationCarry.complement_and_sub
#print axioms UtilizationCarry.strict_equivalence
#print axioms UtilizationCarry.strict_definedness
#print axioms UtilizationCarry.erasing_partial_is_wrong
