import Std
import Init.Data.Nat.Bitwise.Lemmas

namespace IndependentFixed64Xor

def modulusNat : Nat := 18446744073709551616

def halfNat : Nat := 9223372036854775808

def modulus : Int := 18446744073709551616

def half : Int := 9223372036854775808

-- Lean4.22 stdlib has Nat.xor but no Int.xor. These are the four
-- infinite two's-complement constructor cases; negSucc a is complement(a).
def intXor : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a ^^^ b)
  | .negSucc a, .ofNat b => .negSucc (a ^^^ b)
  | .ofNat a, .negSucc b => .negSucc (a ^^^ b)
  | .negSucc a, .negSucc b => .ofNat (a ^^^ b)

def intBit : Int → Nat → Bool
  | .ofNat a, i => Nat.testBit a i
  | .negSucc a, i => !Nat.testBit a i

theorem intBit_xor (x y : Int) (i : Nat) :
    intBit (intXor x y) i = (intBit x i ^^ intBit y i) := by
  cases x <;> cases y <;> simp only [intXor, intBit, Nat.testBit_xor]
  all_goals
    rename_i a b
    cases (Nat.testBit a i) <;> cases (Nat.testBit b i) <;> rfl

theorem nat_mod64_sign (n : Nat) :
    decide (n % modulusNat < halfNat) = !Nat.testBit n 63 := by
  have modulus_power : modulusNat = 2 ^ 64 := by decide
  have half_power : halfNat = 2 ^ 63 := by decide
  have bits : Nat.testBit (n % modulusNat) 63 = Nat.testBit n 63 := by
    rw [modulus_power]
    simpa only [show decide (63 < 64) = true from rfl, Bool.true_and]
      using (Nat.testBit_mod_two_pow n 64 63)
  have bit_div : Nat.testBit (n % modulusNat) 63 =
      decide ((n % modulusNat) / halfNat % 2 = 1) := by
    rw [half_power]
    exact Nat.testBit_eq_decide_div_mod_eq
  have bound : n % modulusNat < modulusNat := Nat.mod_lt n (by decide)
  by_cases lower : n % modulusNat < halfNat
  · have zero : (n % modulusNat) / halfNat = 0 := Nat.div_eq_of_lt lower
    rw [← bits, bit_div, zero]
    simp [lower]
  · have one : (n % modulusNat) / halfNat = 1 := by
      unfold modulusNat halfNat at *
      omega
    rw [← bits, bit_div, one]
    simp [lower]

def wordSign (x : Int) : Bool := decide (x % modulus < half)

theorem wordSign_eq_not_intBit (x : Int) : wordSign x = !intBit x 63 := by
  cases x with
  | ofNat n =>
      change decide ((n : Int) % (modulusNat : Int) < (halfNat : Int)) = !Nat.testBit n 63
      rw [← Int.natCast_emod]
      simp only [Int.ofNat_lt]
      exact nat_mod64_sign n
  | negSucc n =>
      change decide (Int.negSucc n % (modulusNat : Int) < (halfNat : Int)) =
        !(!Nat.testBit n 63)
      rw [Int.negSucc_emod n (by decide), ← Int.natCast_emod]
      have complement_comparison :
          (modulusNat : Int) - 1 - (n % modulusNat : Nat) < (halfNat : Int) ↔
            ¬ (n % modulusNat < halfNat) := by
        unfold modulusNat halfNat
        omega
      simp only [complement_comparison, decide_not, nat_mod64_sign]

theorem fixed64_xor_sign (x y : Int) :
    wordSign (intXor x y) = (wordSign x == wordSign y) := by
  rw [wordSign_eq_not_intBit, intBit_xor,
    wordSign_eq_not_intBit, wordSign_eq_not_intBit]
  cases (intBit x 63) <;> cases (intBit y 63) <;> rfl

def u64Xor (x y : Int) : Int := intXor x y % modulus

theorem fixed64_mod_idempotence (x : Int) :
    (x % modulus) % modulus = x % modulus := Int.emod_emod x modulus

theorem fixed64_helper_sign (x y : Int) :
    wordSign (u64Xor x y) = (wordSign x == wordSign y) := by
  change decide ((intXor x y % modulus) % modulus < half) = (wordSign x == wordSign y)
  rw [fixed64_mod_idempotence]
  exact fixed64_xor_sign x y

-- None denotes an undefined operand. Both input ceilings stay strict.
def primitiveBefore (x y : Option Int) : Option Bool :=
  x.bind fun a => y.map fun b => wordSign (intXor a b)

def helperBefore (x y : Option Int) : Option Bool :=
  x.bind fun a => y.map fun b => wordSign (u64Xor a b)

def parityAfter (x y : Option Int) : Option Bool :=
  x.bind fun a => y.map fun b => (wordSign a == wordSign b)

def partialMod (x : Option Int) : Option Int := x.map fun a => a % modulus

theorem primitive_strict_lifting (x y : Option Int) :
    primitiveBefore x y = parityAfter x y := by
  cases x <;> cases y <;> simp [primitiveBefore, parityAfter, fixed64_xor_sign]

theorem helper_strict_lifting (x y : Option Int) :
    helperBefore x y = parityAfter x y := by
  cases x <;> cases y <;> simp [helperBefore, parityAfter, fixed64_helper_sign]

theorem primitive_definedness (x y : Option Int) :
    primitiveBefore x y = none ↔ x = none ∨ y = none := by
  cases x <;> cases y <;> simp [primitiveBefore]

theorem helper_definedness (x y : Option Int) :
    helperBefore x y = none ↔ x = none ∨ y = none := by
  cases x <;> cases y <;> simp [helperBefore]

theorem parity_definedness (x y : Option Int) :
    parityAfter x y = none ↔ x = none ∨ y = none := by
  cases x <;> cases y <;> simp [parityAfter]

theorem modulo_strict_lifting (x : Option Int) :
    partialMod (partialMod x) = partialMod x := by
  cases x <;> simp [partialMod]

theorem modulo_definedness (x : Option Int) :
    partialMod x = none ↔ x = none := by
  cases x <;> simp [partialMod]

theorem fixed64_modulus_positive : 0 < modulus := by decide

#print axioms intBit_xor
#print axioms nat_mod64_sign
#print axioms wordSign_eq_not_intBit
#print axioms fixed64_xor_sign
#print axioms fixed64_helper_sign
#print axioms fixed64_mod_idempotence
#print axioms primitive_strict_lifting
#print axioms helper_strict_lifting
#print axioms primitive_definedness
#print axioms helper_definedness
#print axioms parity_definedness
#print axioms modulo_strict_lifting
#print axioms modulo_definedness

end IndependentFixed64Xor
