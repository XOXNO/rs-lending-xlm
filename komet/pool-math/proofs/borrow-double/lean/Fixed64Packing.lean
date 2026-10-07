import Init.Data.Int.Bitwise.Lemmas
import Init.Omega

/-
Lean 4.22 core has no signed Int.lor. This file defines infinite two's-
complement OR by sign constructors and Nat.bitwise, then checks every bit.
No definition contains the packing guard or arithmetic conclusion.
-/
namespace Int

def testBit : Int → Nat → Bool
  | .ofNat a, i => Nat.testBit a i
  | .negSucc a, i => !(Nat.testBit a i)

def lor : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a ||| b)
  | .negSucc a, .ofNat b => .negSucc (Nat.bitwise (fun x y => x && !y) a b)
  | .ofNat a, .negSucc b => .negSucc (Nat.bitwise (fun x y => x && !y) b a)
  | .negSucc a, .negSucc b => .negSucc (a &&& b)

theorem testBit_eq_decide_shiftRight_mod (a : Int) (i : Nat) :
    testBit a i = decide ((a >>> i) % 2 = 1) := by
  cases a with
  | ofNat a =>
    change Nat.testBit a i = decide ((((a >>> i : Nat) : Int) % (2 : Nat)) = (1 : Nat))
    rw [← Int.natCast_emod]
    simp only [Int.natCast_inj, Nat.testBit_eq_decide_div_mod_eq, Nat.shiftRight_eq_div_pow]
  | negSucc a =>
    simp only [testBit, Int.negSucc_shiftRight, Nat.testBit_eq_decide_div_mod_eq,
      ← Nat.shiftRight_eq_div_pow]
    change (!decide ((a >>> i) % 2 = 1)) =
      decide (Int.subNatNat 2 ((a >>> i) % 2 + 1) = 1)
    have hrem := Nat.mod_lt (a >>> i) (by decide : 0 < 2)
    have hs : Int.subNatNat 2 ((a >>> i) % 2 + 1) =
        2 - (((a >>> i) % 2 + 1 : Nat) : Int) := by
      exact Int.subNatNat_eq_coe
    rw [hs]
    apply Bool.eq_iff_iff.mpr
    simp only [Bool.not_eq_true', decide_eq_false_iff_not, decide_eq_true_eq,
      Int.natCast_add, Int.natCast_one]
    omega

theorem testBit_eq_decide_div_pow_mod (a : Int) (i : Nat) :
    testBit a i = decide ((a / (2 ^ i : Nat)) % 2 = 1) := by
  rw [← Int.shiftRight_eq_div_pow]
  exact testBit_eq_decide_shiftRight_mod a i

theorem eq_of_testBit_eq {a b : Int}
    (bits_equal : ∀ i, testBit a i = testBit b i) : a = b := by
  cases a with
  | ofNat a =>
    cases b with
    | ofNat b => exact congrArg Int.ofNat (Nat.eq_of_testBit_eq bits_equal)
    | negSucc b =>
      have ha : a < 2 ^ (a + b + 1) := Nat.lt_of_le_of_lt
        (by omega : a ≤ a + b + 1) Nat.lt_two_pow_self
      have hb : b < 2 ^ (a + b + 1) := Nat.lt_of_le_of_lt
        (by omega : b ≤ a + b + 1) Nat.lt_two_pow_self
      have h := bits_equal (a + b + 1)
      simp only [testBit, Nat.testBit_lt_two_pow ha, Nat.testBit_lt_two_pow hb] at h
      cases h
  | negSucc a =>
    cases b with
    | ofNat b =>
      have ha : a < 2 ^ (a + b + 1) := Nat.lt_of_le_of_lt
        (by omega : a ≤ a + b + 1) Nat.lt_two_pow_self
      have hb : b < 2 ^ (a + b + 1) := Nat.lt_of_le_of_lt
        (by omega : b ≤ a + b + 1) Nat.lt_two_pow_self
      have h := bits_equal (a + b + 1)
      simp only [testBit, Nat.testBit_lt_two_pow ha, Nat.testBit_lt_two_pow hb] at h
      cases h
    | negSucc b =>
      apply congrArg Int.negSucc
      exact Nat.eq_of_testBit_eq (fun i => Bool.not_inj (bits_equal i))

theorem testBit_lor (a b : Int) (i : Nat) :
    testBit (lor a b) i = (testBit a i || testBit b i) := by
  cases a with
  | ofNat a =>
    cases b with
    | ofNat b => exact Nat.testBit_or a b i
    | negSucc b =>
      simp only [lor, testBit, Nat.testBit_bitwise (f := fun x y => x && !y) (by rfl)]
      cases Nat.testBit a i <;> cases Nat.testBit b i <;> rfl
  | negSucc a =>
    cases b with
    | ofNat b =>
      simp only [lor, testBit, Nat.testBit_bitwise (f := fun x y => x && !y) (by rfl)]
      cases Nat.testBit a i <;> cases Nat.testBit b i <;> rfl
    | negSucc b =>
      simp only [lor, testBit, Nat.testBit_and]
      cases Nat.testBit a i <;> cases Nat.testBit b i <;> rfl

theorem lor_unique (a b result : Int)
    (bits_or : ∀ i, testBit result i = (testBit a i || testBit b i)) :
    lor a b = result := by
  apply eq_of_testBit_eq
  intro i
  exact (testBit_lor a b i).trans (bits_or i).symm

end Int

namespace Fixed64Packing

private theorem nat_clear_low (h l n : Nat) (hl : l < 2 ^ n) :
    Nat.bitwise (fun x y => x && !y) (2 ^ n * h + (2 ^ n - 1)) l =
      2 ^ n * h + (2 ^ n - (l + 1)) := by
  apply Nat.eq_of_testBit_eq
  intro i
  have hp : 0 < 2 ^ n := Nat.two_pow_pos n
  have hm : 2 ^ n - 1 < 2 ^ n := by omega
  have hc : 2 ^ n - (l + 1) < 2 ^ n := by omega
  rw [Nat.testBit_bitwise (by rfl), Nat.testBit_two_pow_mul_add h hm,
    Nat.testBit_two_pow_mul_add h hc, Nat.testBit_two_pow_sub_one,
    Nat.testBit_two_pow_sub_succ hl]
  by_cases hi : i < n
  · simp [hi]
  · have hn : n ≤ i := by omega
    have hli : l < 2 ^ i := Nat.lt_of_lt_of_le hl
      (Nat.pow_le_pow_right Nat.zero_lt_two hn)
    simp [hi, Nat.testBit_lt_two_pow hli]

theorem signed_packing (H L : Int) (n : Nat)
    (low_nonnegative : 0 ≤ L) (low_below_word : L < (2 ^ n : Nat)) :
    Int.lor (H * (2 ^ n : Nat)) L = H * (2 ^ n : Nat) + L := by
  cases L with
  | negSucc l => omega
  | ofNat l =>
    have hl : l < 2 ^ n := by
      simpa only [Int.ofNat_eq_coe, Int.ofNat_lt] using low_below_word
    cases H with
    | ofNat h =>
      have hor := Nat.two_pow_add_eq_or_of_lt hl h
      simpa only [Int.natCast_mul, Int.natCast_add, Int.lor, Nat.mul_comm]
        using congrArg Int.ofNat hor.symm
    | negSucc h =>
      have hp : 0 < 2 ^ n := Nat.two_pow_pos n
      have hmul : Int.negSucc h * (2 ^ n : Nat) =
          Int.negSucc (2 ^ n * h + (2 ^ n - 1)) := by
        rw [Int.negSucc_eq, Int.negSucc_eq, Int.neg_mul,
          Int.natCast_add, Int.natCast_mul, Int.natCast_sub (by omega : 1 ≤ 2 ^ n)]
        simp only [Int.natCast_one]
        rw [Int.add_mul, Int.one_mul, Int.mul_comm (h : Int)]
        omega
      rw [hmul, Int.lor, nat_clear_low h l n hl, Int.negSucc_eq,
        Int.negSucc_eq, Int.natCast_add, Int.natCast_add,
        Int.natCast_sub (by omega : l + 1 ≤ 2 ^ n),
        Int.natCast_sub (by omega : 1 ≤ 2 ^ n),
        Int.natCast_add, Int.natCast_one, Int.ofNat_eq_coe]
      omega

theorem fixed64_packing (H L : Int)
    (low_nonnegative : 0 ≤ L) (low_below_word : L < 2 ^ (64 : Nat)) :
    Int.lor (H * 2 ^ (64 : Nat)) L = H * 2 ^ (64 : Nat) + L := by
  exact signed_packing H L 64 low_nonnegative (by simpa using low_below_word)

/-- Strict binary lifting: both operands must produce a value. -/
def strict2 {α β γ : Type} (f : α → β → γ) (a : Option α) (b : Option β) : Option γ :=
  a.bind fun x => b.map (f x)

theorem strict2_defined {α β γ : Type} (f : α → β → γ) (a : Option α) (b : Option β) :
    strict2 f a b ≠ none ↔ a ≠ none ∧ b ≠ none := by
  cases a <;> cases b <;> simp [strict2]

theorem strict_fixed64_defined (H L : Option Int) :
    (strict2 (fun h l => Int.lor (h * 2 ^ (64 : Nat)) l) H L ≠ none ↔
      H ≠ none ∧ L ≠ none) ∧
    (strict2 (fun h l => h * 2 ^ (64 : Nat) + l) H L ≠ none ↔
      H ≠ none ∧ L ≠ none) := by
  exact ⟨strict2_defined _ H L, strict2_defined _ H L⟩

theorem strict_fixed64_packing (H L : Option Int)
    (low_domain : ∀ l, L = some l → 0 ≤ l ∧ l < 2 ^ (64 : Nat)) :
    strict2 (fun h l => Int.lor (h * 2 ^ (64 : Nat)) l) H L =
      strict2 (fun h l => h * 2 ^ (64 : Nat) + l) H L := by
  cases H with
  | none => rfl
  | some h =>
    cases L with
    | none => rfl
    | some l =>
      obtain ⟨h0, hlt⟩ := low_domain l rfl
      exact congrArg some (fixed64_packing h l h0 hlt)

end Fixed64Packing

#print axioms Int.testBit_lor
#print axioms Int.testBit_eq_decide_shiftRight_mod
#print axioms Int.testBit_eq_decide_div_pow_mod
#print axioms Int.eq_of_testBit_eq
#print axioms Int.lor_unique
#print axioms Fixed64Packing.signed_packing
#print axioms Fixed64Packing.fixed64_packing
#print axioms Fixed64Packing.strict2_defined
#print axioms Fixed64Packing.strict_fixed64_defined
#print axioms Fixed64Packing.strict_fixed64_packing
