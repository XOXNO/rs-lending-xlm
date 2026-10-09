import Fixed64Packing

/-
Generic signed-integer arithmetic only. No utilization path assumptions.
Lean core arithmetic right shift is floor division by a positive power of two.
K /Int is represented by truncating Int.tdiv; positive modInt by Euclidean %.
The left-shift definition below is copied from the frozen CarryWord model:
shift the natural magnitude, preserving the sign. Its multiplication theorem
is kernel checked for every Int and natural shift count. No K/Wasm refinement
is asserted here; the external hook arithmetic must be checked separately.
-/
namespace Int

protected def shiftLeft : Int → Nat → Int
  | .ofNat a, n => .ofNat (a <<< n)
  | .negSucc a, n => -(.ofNat ((a + 1) <<< n))

instance : HShiftLeft Int Nat Int := ⟨.shiftLeft⟩

theorem shiftLeft_eq_mul (x : Int) (n : Nat) :
    x <<< n = x * (2 ^ n : Nat) := by
  cases x with
  | ofNat a =>
    change ((a <<< n : Nat) : Int) = (a : Int) * (2 ^ n : Nat)
    rw [Nat.shiftLeft_eq, Int.natCast_mul]
  | negSucc a =>
    change -(((a + 1) <<< n : Nat) : Int) = Int.negSucc a * (2 ^ n : Nat)
    rw [Nat.shiftLeft_eq, Int.natCast_mul, Int.negSucc_eq, Int.neg_mul]
    simp only [Int.natCast_add, Int.natCast_one]

end Int

namespace HalfWord

abbrev M : Int := 18446744073709551616
abbrev C : Int := 9223372036854775808

def u64Or (a b : Int) : Int := Int.lor a b % M

theorem fixed_constants :
    M = 2 ^ (64 : Nat) ∧ C = 2 ^ (63 : Nat) ∧
    M ≠ 0 ∧ C ≠ 0 ∧ (2 : Int) ≠ 0 ∧
    (1 : Int) ≥ 0 ∧ (63 : Int) ≥ 0 := by decide

theorem shiftRight_one_eq_exact_tdiv (x : Int) :
    (x >>> (1 : Nat)) = (x - x % 2).tdiv 2 := by
  rw [Int.shiftRight_eq_div_pow]
  change x / 2 = (x - x % 2).tdiv 2
  symm
  apply Int.tdiv_eq_of_eq_mul_right (by decide : (2 : Int) ≠ 0)
  have := Int.ediv_add_emod x 2
  omega

theorem right_one_low (x : Int) :
    (x >>> (1 : Nat)) % M = ((x - x % 2).tdiv 2) % M := by
  rw [shiftRight_one_eq_exact_tdiv]

theorem left_63_low (x : Int) :
    (x <<< (63 : Nat)) % M = (x % 2) * C := by
  rw [Int.shiftLeft_eq_mul]
  change (x * C) % (C * 2) = (x % 2) * C
  rw [Int.mul_comm x C,
    Int.mul_emod_mul_of_pos x 2 (by decide : 0 < C), Int.mul_comm]

theorem pack_63_low (H L : Int)
    (low_nonnegative : 0 ≤ L) (low_below_63 : L < C) :
    u64Or (H * C) L = (H * C + L) % M := by
  unfold u64Or
  have h := Fixed64Packing.signed_packing H L 63 low_nonnegative
    (by simpa using low_below_63)
  change Int.lor (H * (2 ^ (63 : Nat) : Nat)) L % M =
    (H * (2 ^ (63 : Nat) : Nat) + L) % M
  rw [h]

-- One partial-result model: undefined inputs and zero divisors produce none.
-- Literal natural shift counts 1 and 63 are valid and defined.
abbrev strictBinary := @Fixed64Packing.strict2

def strictTdiv (a b : Option Int) : Option Int :=
  a.bind fun x => b.bind fun y => if y = 0 then none else some (x.tdiv y)

def strictEmod (a b : Option Int) : Option Int :=
  a.bind fun x => b.bind fun y => if y = 0 then none else some (x % y)

def strictRightOne (x : Option Int) : Option Int :=
  x.map fun (v : Int) => v >>> (1 : Nat)

def strictLeft63 (x : Option Int) : Option Int :=
  x.map fun (v : Int) => v <<< (63 : Nat)

def strictU64Or (a b : Option Int) : Option Int :=
  strictEmod (strictBinary Int.lor a b) (some M)

def rightOneLeft (x : Option Int) : Option Int :=
  strictEmod (strictRightOne x) (some M)

def rightOneRight (x : Option Int) : Option Int :=
  strictEmod
    (strictTdiv (strictBinary (· - ·) x (strictEmod x (some 2))) (some 2))
    (some M)

def left63Left (x : Option Int) : Option Int :=
  strictEmod (strictLeft63 x) (some M)

def left63Right (x : Option Int) : Option Int :=
  strictBinary (· * ·) (strictEmod x (some 2)) (some C)

def pack63Left (H L : Option Int) : Option Int :=
  strictU64Or (strictBinary (· * ·) H (some C)) L

def pack63Right (H L : Option Int) : Option Int :=
  strictEmod (strictBinary (· + ·) (strictBinary (· * ·) H (some C)) L) (some M)

theorem strict_right_one_low (x : Option Int) : rightOneLeft x = rightOneRight x := by
  cases x with
  | none => rfl
  | some v =>
    change some ((v >>> (1 : Nat)) % M) = some (((v - v % 2).tdiv 2) % M)
    exact congrArg some (right_one_low v)

theorem strict_left_63_low (x : Option Int) : left63Left x = left63Right x := by
  cases x with
  | none => rfl
  | some v =>
    change some ((v <<< (63 : Nat)) % M) = some ((v % 2) * C)
    exact congrArg some (left_63_low v)

theorem strict_pack_63_low (H L : Option Int)
    (low_domain : ∀ l, L = some l → 0 ≤ l ∧ l < C) :
    pack63Left H L = pack63Right H L := by
  cases H with
  | none => rfl
  | some h =>
    cases L with
    | none => rfl
    | some l =>
      obtain ⟨h0, hlt⟩ := low_domain l rfl
      change some (u64Or (h * C) l) = some ((h * C + l) % M)
      exact congrArg some (pack_63_low h l h0 hlt)

theorem strict_right_one_defined (x : Option Int) :
    (rightOneLeft x ≠ none ↔ x ≠ none) ∧
    (rightOneRight x ≠ none ↔ x ≠ none) := by
  cases x <;> simp [rightOneLeft, rightOneRight, strictRightOne,
    strictEmod, strictTdiv, strictBinary, Fixed64Packing.strict2, M]

theorem strict_left_63_defined (x : Option Int) :
    (left63Left x ≠ none ↔ x ≠ none) ∧
    (left63Right x ≠ none ↔ x ≠ none) := by
  cases x <;> simp [left63Left, left63Right, strictLeft63,
    strictEmod, strictBinary, Fixed64Packing.strict2, M]

theorem strict_pack_63_defined (H L : Option Int) :
    (pack63Left H L ≠ none ↔ H ≠ none ∧ L ≠ none) ∧
    (pack63Right H L ≠ none ↔ H ≠ none ∧ L ≠ none) := by
  cases H <;> cases L <;> simp [pack63Left, pack63Right, strictU64Or,
    strictEmod, strictBinary, Fixed64Packing.strict2, M]

theorem strict_right_one_undefined (x : Option Int) :
    (rightOneLeft x = none ↔ x = none) ∧
    (rightOneRight x = none ↔ x = none) := by
  cases x <;> simp [rightOneLeft, rightOneRight, strictRightOne,
    strictEmod, strictTdiv, strictBinary, Fixed64Packing.strict2, M]

theorem strict_left_63_undefined (x : Option Int) :
    (left63Left x = none ↔ x = none) ∧
    (left63Right x = none ↔ x = none) := by
  cases x <;> simp [left63Left, left63Right, strictLeft63,
    strictEmod, strictBinary, Fixed64Packing.strict2, M]

theorem strict_pack_63_undefined (H L : Option Int) :
    (pack63Left H L = none ↔ H = none ∨ L = none) ∧
    (pack63Right H L = none ↔ H = none ∨ L = none) := by
  cases H <;> cases L <;> simp [pack63Left, pack63Right, strictU64Or,
    strictEmod, strictBinary, Fixed64Packing.strict2, M]

end HalfWord

#print axioms Int.shiftLeft_eq_mul
#print axioms HalfWord.fixed_constants
#print axioms HalfWord.shiftRight_one_eq_exact_tdiv
#print axioms HalfWord.right_one_low
#print axioms HalfWord.left_63_low
#print axioms HalfWord.pack_63_low
#print axioms HalfWord.strict_right_one_low
#print axioms HalfWord.strict_left_63_low
#print axioms HalfWord.strict_pack_63_low
#print axioms HalfWord.strict_right_one_defined
#print axioms HalfWord.strict_left_63_defined
#print axioms HalfWord.strict_pack_63_defined
#print axioms HalfWord.strict_right_one_undefined
#print axioms HalfWord.strict_left_63_undefined
#print axioms HalfWord.strict_pack_63_undefined
