import Init.Data.Int.Bitwise.Lemmas
import Init.Omega

/-
Independent byte-wrap evidence. `andNatMask` implements signed infinite
two's-complement AND with a nonnegative mask by bits, never by modulo.
K wrap-Positive gives N &Int ((1 <<Int (WIDTH *Int 8)) -Int 1).
At WIDTH = 1 its exact mask is 255. K's INT.and hook uses GMP mpz_and.
-/
namespace ByteWrap

def signedBit : Int → Nat → Bool
  | .ofNat a, i => Nat.testBit a i
  | .negSucc a, i => !(Nat.testBit a i)

def andNatMask : Int → Nat → Int
  | .ofNat a, mask => .ofNat (a &&& mask)
  | .negSucc a, mask => .ofNat (Nat.bitwise (fun x y => !x && y) a mask)

theorem andNatMask_bits (x : Int) (mask i : Nat) :
    signedBit (andNatMask x mask) i =
      (signedBit x i && Nat.testBit mask i) := by
  cases x with
  | ofNat a => exact Nat.testBit_and a mask i
  | negSucc a =>
    exact Nat.testBit_bitwise (f := fun x y => !x && y) (by rfl) a mask i

theorem nat_complement_mask (a n : Nat) :
    Nat.bitwise (fun x y => !x && y) a (2 ^ n - 1) =
      2 ^ n - (a % 2 ^ n + 1) := by
  apply Nat.eq_of_testBit_eq
  intro i
  rw [Nat.testBit_bitwise (by rfl), Nat.testBit_two_pow_sub_one,
    Nat.testBit_two_pow_sub_succ (Nat.mod_lt a (Nat.two_pow_pos n)),
    Nat.testBit_mod_two_pow]
  cases decide (i < n) <;> cases Nat.testBit a i <;> rfl

theorem and_power_mask_eq_mod (x : Int) (n : Nat) :
    andNatMask x (2 ^ n - 1) = x % (2 ^ n : Nat) := by
  cases x with
  | ofNat a =>
    rw [andNatMask, Nat.and_two_pow_sub_one_eq_mod]
    exact (Int.natCast_emod a (2 ^ n)).symm
  | negSucc a =>
    have hp : 0 < ((2 ^ n : Nat) : Int) := by
      exact_mod_cast Nat.two_pow_pos n
    have hrem : a % 2 ^ n + 1 ≤ 2 ^ n := by
      have := Nat.mod_lt a (Nat.two_pow_pos n)
      omega
    rw [andNatMask, nat_complement_mask, Int.negSucc_emod a hp,
      ← Int.natCast_emod, Int.ofNat_eq_coe, Int.natCast_sub hrem,
      Int.natCast_add, Int.natCast_one]
    omega

def byteMask : Nat := (1 <<< (1 * 8)) - 1
def wrapOne (x : Int) : Int := andNatMask x byteMask

theorem exact_mask : byteMask = 255 ∧ byteMask = 2 ^ 8 - 1 ∧
    ((1 : Int) * 2 ^ (1 * 8 : Nat) - 1) = (byteMask : Int) := by
  decide

theorem wrapOne_eq_mod (x : Int) : wrapOne x = x % 256 := by
  rw [wrapOne, exact_mask.2.1]
  exact and_power_mask_eq_mod x 8

/-- Strict unary lifting keeps an undefined input undefined. -/
def strict (f : Int → Int) (x : Option Int) : Option Int := x.map f

theorem strict_defined (f : Int → Int) (x : Option Int) :
    strict f x ≠ none ↔ x ≠ none := by
  cases x <;> simp [strict]

theorem strict_wrapOne_eq_mod (x : Option Int) :
    strict wrapOne x = strict (fun x => x % 256) x := by
  cases x with
  | none => rfl
  | some x => exact congrArg some (wrapOne_eq_mod x)

theorem strict_wrapOne_definedness (x : Option Int) :
    (strict wrapOne x ≠ none ↔ x ≠ none) ∧
    (strict (fun x => x % 256) x ≠ none ↔ x ≠ none) := by
  exact ⟨strict_defined _ x, strict_defined _ x⟩

end ByteWrap

#print axioms ByteWrap.andNatMask_bits
#print axioms ByteWrap.nat_complement_mask
#print axioms ByteWrap.and_power_mask_eq_mod
#print axioms ByteWrap.exact_mask
#print axioms ByteWrap.wrapOne_eq_mod
#print axioms ByteWrap.strict_wrapOne_eq_mod
#print axioms ByteWrap.strict_wrapOne_definedness
