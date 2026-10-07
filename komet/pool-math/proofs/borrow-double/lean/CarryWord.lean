import Fixed64Packing
import SignedDecoder

/-
K /Int is truncation toward zero; positive modInt is Euclidean remainder.
Only the literal shift count 1 is modeled here; variable counts are out of scope.
The signed shift uses Nat.shiftLeft on magnitude, preserving the input sign.
This proves integer/strict Option identities, not a refinement of Wasm or K hooks.
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

theorem shiftLeft_one (x : Int) : x <<< (1 : Nat) = x * 2 := by
  simpa using shiftLeft_eq_mul x 1

end Int

namespace CarryWord

abbrev M : Int := 18446744073709551616
abbrev C : Int := 9223372036854775808

def u64Or (a b : Int) : Int := Int.lor a b % M

theorem fixed_constants : M ≠ 0 ∧ C ≠ 0 ∧ (1 : Int) ≥ 0 := by decide

theorem projected_shift_one (H : Int) :
    (H <<< (1 : Nat)) % M = (H % C) * 2 := by
  rw [Int.shiftLeft_one]
  change (H * 2) % (2 * C) = (H % C) * 2
  rw [Int.mul_comm H 2, Int.mul_emod_mul_of_pos H C (by decide), Int.mul_comm]

theorem projected_carry_range (H L : Int) (low_nonnegative : 0 ≤ L)
    (low_below_two : L < 2) :
    0 ≤ (H % C) * 2 + L ∧ (H % C) * 2 + L < M := by
  have h0 := Int.emod_nonneg H (by decide : C ≠ 0)
  have hlt := Int.emod_lt_of_pos H (by decide : 0 < C)
  constructor <;> (simp only [M, C] at *; omega)

-- The generic K simplification: arbitrary signed H; only the existing low-bit guard.
theorem fixed64_projected_shift_one_carry (H L : Int)
    (low_nonnegative : 0 ≤ L) (low_below_two : L < 2) :
    u64Or ((H <<< (1 : Nat)) % M) L = (H % C) * 2 + L := by
  unfold u64Or
  rw [projected_shift_one]
  have packing : Int.lor ((H % C) * 2) L = (H % C) * 2 + L :=
    Fixed64Packing.signed_packing (H % C) L 1 low_nonnegative low_below_two
  rw [packing]
  obtain ⟨h0, hlt⟩ := projected_carry_range H L low_nonnegative low_below_two
  exact Int.emod_eq_of_lt h0 hlt

theorem fixed64_projected_shift_one_carry_mod (H L : Int)
    (low_nonnegative : 0 ≤ L) (low_below_two : L < 2) :
    u64Or ((H <<< (1 : Nat)) % M) L = (H * 2 + L) % M := by
  rw [fixed64_projected_shift_one_carry H L low_nonnegative low_below_two]
  obtain ⟨h0, hlt⟩ := projected_carry_range H L low_nonnegative low_below_two
  calc
    (H % C) * 2 + L = ((H % C) * 2 + L) % M := (Int.emod_eq_of_lt h0 hlt).symm
    _ = ((H <<< (1 : Nat)) % M + L) % M := by rw [projected_shift_one]
    _ = ((H <<< (1 : Nat)) + L) % M := Int.emod_add_emod _ _ _
    _ = (H * 2 + L) % M := by rw [Int.shiftLeft_one]

def high (x : Int) : Int := (x - x % M).tdiv M
def carry (x : Int) : Int := (x % M - (x % M) % C).tdiv C
def doubledHigh (x : Int) : Int := (x * 2 - (x * 2) % M).tdiv M
def encoded (x : Int) : Int := u64Or ((high x <<< (1 : Nat)) % M) (carry x)

theorem high_mul (x : Int) : high x * M = x - x % M :=
  Int.tdiv_mul_cancel Int.dvd_self_sub_emod

theorem carry_mul (x : Int) : carry x * C = x % M - (x % M) % C :=
  Int.tdiv_mul_cancel Int.dvd_self_sub_emod

theorem carry_bounds (x : Int) : 0 ≤ carry x ∧ carry x < 2 := by
  have hm0 := Int.emod_nonneg x (by decide : M ≠ 0)
  have hmlt := Int.emod_lt_of_pos x (by decide : 0 < M)
  have hc0 := Int.emod_nonneg (x % M) (by decide : C ≠ 0)
  have hclt := Int.emod_lt_of_pos (x % M) (by decide : 0 < C)
  have h := carry_mul x
  constructor <;> (simp only [M, C] at *; omega)

theorem doubledHigh_eq (x : Int) : doubledHigh x = high x * 2 + carry x := by
  have hq := high_mul x
  have hc := carry_mul x
  have hs0 := Int.emod_nonneg (x % M) (by decide : C ≠ 0)
  have hslt := Int.emod_lt_of_pos (x % M) (by decide : 0 < C)
  have hexp : x * 2 = (high x * 2 + carry x) * M + ((x % M) % C) * 2 := by
    simp only [M, C] at *
    omega
  have hrem : (x * 2) % M = ((x % M) % C) * 2 := by
    rw [hexp, Int.mul_add_emod_self_right]
    apply Int.emod_eq_of_lt <;> (simp only [M, C] at *; omega)
  have hn : x * 2 - (x * 2) % M = (high x * 2 + carry x) * M := by
    rw [hrem]
    simp only [M, C] at *
    omega
  unfold doubledHigh
  rw [hn, Int.mul_tdiv_cancel _ (by decide : M ≠ 0)]

-- Universal, including negative x: the output is the unsigned 64-bit projection.
theorem encoded_eq_doubledHigh_mod (x : Int) : encoded x = doubledHigh x % M := by
  obtain ⟨h0, hlt⟩ := carry_bounds x
  unfold encoded
  rw [fixed64_projected_shift_one_carry_mod _ _ h0 hlt, doubledHigh_eq]

theorem doubledHigh_i128_range (x : Int) (nonnegative : 0 ≤ x)
    (signed_i128_upper : x ≤ 170141183460469231731687303715884105727) :
    0 ≤ doubledHigh x ∧ doubledHigh x < M := by
  have h0 := Int.emod_nonneg (x * 2) (by decide : M ≠ 0)
  have hlt := Int.emod_lt_of_pos (x * 2) (by decide : 0 < M)
  have h : doubledHigh x * M = x * 2 - (x * 2) % M :=
    Int.tdiv_mul_cancel Int.dvd_self_sub_emod
  constructor <;> (simp only [M] at *; omega)

-- Direct equality uses exactly the existing nonnegative signed-i128 path bounds.
theorem encoded_eq_doubledHigh_i128 (x : Int) (nonnegative : 0 ≤ x)
    (signed_i128_upper : x ≤ 170141183460469231731687303715884105727) :
    encoded x = doubledHigh x := by
  rw [encoded_eq_doubledHigh_mod]
  obtain ⟨h0, hlt⟩ := doubledHigh_i128_range x nonnegative signed_i128_upper
  exact Int.emod_eq_of_lt h0 hlt

open SignedDecoder

-- Shift count is the fixed, defined literal 1. Both OR operands remain strict.
def strictShiftOne (x : Option Int) : Option Int := x.map (fun (v : Int) => v <<< (1 : Nat))
def strictU64Or (a b : Option Int) : Option Int :=
  strictEmod (strictBinary Int.lor a b) (some M)

def projectedCarryOption (H L : Option Int) : Option Int :=
  strictU64Or (strictEmod (strictShiftOne H) (some M)) L

def projectedCarryResultOption (H L : Option Int) : Option Int :=
  strictBinary (· + ·) (strictBinary (· * ·) (strictEmod H (some C)) (some 2)) L

theorem strict_projected_shift_one_carry (H L : Option Int)
    (low_domain : ∀ l, L = some l → 0 ≤ l ∧ l < 2) :
    projectedCarryOption H L = projectedCarryResultOption H L := by
  cases H with
  | none => rfl
  | some h =>
    cases L with
    | none => rfl
    | some l =>
      obtain ⟨h0, hlt⟩ := low_domain l rfl
      change some (u64Or ((h <<< (1 : Nat)) % M) l) = some ((h % C) * 2 + l)
      exact congrArg some (fixed64_projected_shift_one_carry h l h0 hlt)

theorem strict_projected_shift_one_carry_defined (H L : Option Int) :
    (Defined (projectedCarryOption H L) ↔ Defined H ∧ Defined L) ∧
    (Defined (projectedCarryResultOption H L) ↔ Defined H ∧ Defined L) := by
  cases H <;> cases L <;>
    simp [Defined, projectedCarryOption, projectedCarryResultOption,
      strictU64Or, strictShiftOne, strictEmod, strictBinary, M, C]

theorem strict_projected_shift_one_carry_undefined (H L : Option Int) :
    (projectedCarryOption H L = none ↔ H = none ∨ L = none) ∧
    (projectedCarryResultOption H L = none ↔ H = none ∨ L = none) := by
  cases H <;> cases L <;>
    simp [projectedCarryOption, projectedCarryResultOption,
      strictU64Or, strictShiftOne, strictEmod, strictBinary, M, C]

-- Every operator in the original encoding and the replacement is lifted strictly.
def highOption (x : Option Int) : Option Int :=
  strictTdiv (strictBinary (· - ·) x (strictEmod x (some M))) (some M)

def carryOption (x : Option Int) : Option Int :=
  strictTdiv
    (strictBinary (· - ·) (strictEmod x (some M))
      (strictEmod (strictEmod x (some M)) (some C))) (some C)

def encodedOption (x : Option Int) : Option Int :=
  strictU64Or (strictEmod (strictShiftOne (highOption x)) (some M)) (carryOption x)

def doubledHighOption (x : Option Int) : Option Int :=
  strictTdiv
    (strictBinary (· - ·) (strictBinary (· * ·) x (some 2))
      (strictEmod (strictBinary (· * ·) x (some 2)) (some M))) (some M)

def doubledHighModOption (x : Option Int) : Option Int :=
  strictEmod (doubledHighOption x) (some M)

theorem encodedOption_eq_map (x : Option Int) : encodedOption x = x.map encoded := by
  cases x <;> rfl

theorem doubledHighOption_eq_map (x : Option Int) :
    doubledHighOption x = x.map doubledHigh := by
  cases x <;> rfl

theorem doubledHighModOption_eq_map (x : Option Int) :
    doubledHighModOption x = x.map (fun v => doubledHigh v % M) := by
  cases x <;> rfl

theorem strict_encoded_eq_doubledHigh_mod (x : Option Int) :
    encodedOption x = doubledHighModOption x := by
  rw [encodedOption_eq_map, doubledHighModOption_eq_map]
  cases x with
  | none => rfl
  | some v => exact congrArg some (encoded_eq_doubledHigh_mod v)

theorem strict_encoded_eq_doubledHigh_i128 (x : Option Int)
    (input_domain : ∀ v, x = some v →
      0 ≤ v ∧ v ≤ 170141183460469231731687303715884105727) :
    encodedOption x = doubledHighOption x := by
  rw [encodedOption_eq_map, doubledHighOption_eq_map]
  cases x with
  | none => rfl
  | some v =>
    obtain ⟨h0, hlt⟩ := input_domain v rfl
    exact congrArg some (encoded_eq_doubledHigh_i128 v h0 hlt)

theorem strict_encoded_defined (x : Option Int) :
    (Defined (encodedOption x) ↔ Defined x) ∧
    (Defined (doubledHighOption x) ↔ Defined x) ∧
    (Defined (doubledHighModOption x) ↔ Defined x) := by
  rw [encodedOption_eq_map, doubledHighOption_eq_map, doubledHighModOption_eq_map]
  cases x <;> simp [Defined]

theorem strict_encoded_undefined (x : Option Int) :
    (encodedOption x = none ↔ x = none) ∧
    (doubledHighOption x = none ↔ x = none) ∧
    (doubledHighModOption x = none ↔ x = none) := by
  rw [encodedOption_eq_map, doubledHighOption_eq_map, doubledHighModOption_eq_map]
  cases x <;> simp

end CarryWord

#print axioms Int.shiftLeft_eq_mul
#print axioms Int.shiftLeft_one
#print axioms CarryWord.fixed_constants
#print axioms CarryWord.projected_shift_one
#print axioms CarryWord.projected_carry_range
#print axioms CarryWord.fixed64_projected_shift_one_carry
#print axioms CarryWord.fixed64_projected_shift_one_carry_mod
#print axioms CarryWord.high_mul
#print axioms CarryWord.carry_mul
#print axioms CarryWord.carry_bounds
#print axioms CarryWord.doubledHigh_eq
#print axioms CarryWord.encoded_eq_doubledHigh_mod
#print axioms CarryWord.doubledHigh_i128_range
#print axioms CarryWord.encoded_eq_doubledHigh_i128
#print axioms CarryWord.strict_projected_shift_one_carry
#print axioms CarryWord.strict_projected_shift_one_carry_defined
#print axioms CarryWord.strict_projected_shift_one_carry_undefined
#print axioms CarryWord.encodedOption_eq_map
#print axioms CarryWord.doubledHighOption_eq_map
#print axioms CarryWord.doubledHighModOption_eq_map
#print axioms CarryWord.strict_encoded_eq_doubledHigh_mod
#print axioms CarryWord.strict_encoded_eq_doubledHigh_i128
#print axioms CarryWord.strict_encoded_defined
#print axioms CarryWord.strict_encoded_undefined
