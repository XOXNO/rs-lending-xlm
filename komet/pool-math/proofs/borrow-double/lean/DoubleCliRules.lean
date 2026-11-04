import CarryWord
import DoubleWord
import DoubleRay
import FloorEquality
import FloorWordComparison
import IndependentFixed64XorV2
import IndependentSignedModulo64
import LowWord
import SignedWordNegative

/-
One theorem per rule of ../double-cli.k, stated in the rule's own shape.
Operator model (K 7.1.337 INT, KWasm/Komet helpers):
  A /Int B   = Int.tdiv A B        (truncation toward zero)
  A modInt B = A % B               (Euclidean remainder)
  A xorInt B = IndependentFixed64Xor.intXor A B   (infinite two's complement)
  A |Int B   = Int.lor A B         (Fixed64Packing, infinite two's complement)
  A <<Int 1  = A <<< (1 : Nat)     (CarryWord)
  u64Xor(A, B) = intXor A B % 2^64, u64Or(A, B) = Int.lor A B % 2^64
  #signed(i64, N) = if N < 2^63 then N else N - 2^64, for 0 <= N < 2^64
  #if B #then X #else Y #fi = if B then X else Y;  ==Bool = Bool equality.
Each Boolean rule is stated as an equation between Bool values (decide).
Every rule except integer-self-subtraction keeps every variable of its left
side on its right side, and every division/modulus has a literal nonzero
divisor, so the right side is defined whenever the left side is. The single
erasing rule, integer-self-subtraction, carries no preserves-definedness
attribute; the backend must establish definedness of I before applying it.
-/
namespace DoubleCliRules

open IndependentFixed64Xor (intXor)

abbrev W : Int := 18446744073709551616  -- 2^64
abbrev H : Int := 9223372036854775808   -- 2^63

def signed64 (n : Int) : Int := if n < H then n else n - W
def u64Xor (a b : Int) : Int := intXor a b % W
def u64Or (a b : Int) : Int := Int.lor a b % W

theorem decide_not_of_iff {P Q : Prop} [Decidable P] [Decidable Q] (h : P ↔ ¬ Q) :
    decide P = !decide Q := by
  by_cases hq : Q <;> simp [hq, h]

-- [distribute-signed-word-comparison]
theorem distribute_signed_word_comparison (B : Bool) (X : Int) :
    decide ((if B then X else X + -18446744073709551616) ≥ 0) =
      (if B then !decide (X < 0) else !decide (X < 18446744073709551616)) := by
  cases B
  · simp only [Bool.false_eq_true, if_false]
    exact decide_not_of_iff (by omega)
  · simp only [if_true]
    exact decide_not_of_iff (by omega)

-- [xor-zero-left-nonnegative-complement]
theorem xor_zero_left_nonnegative_complement (X Y : Int) :
    decide (0 ≤ u64Xor X Y) = !decide (u64Xor X Y < 0) :=
  decide_not_of_iff (by omega)

-- [integer-self-subtraction]  (erases I: requires I to be defined, see above)
theorem integer_self_subtraction (I : Int) : I - I = 0 := Int.sub_self I

-- [floor-word64-less-than]
theorem floor_word64_less_than (X N : Int) :
    decide ((X - X % W).tdiv W < N) = decide (X < N * W) := by
  have := FloorWordComparison.word_lt_iff X N
  simp only [FloorWordComparison.word] at this
  simp [this]

-- [distribute-signed-negative-word-comparison]
theorem distribute_signed_negative_word_comparison (B : Bool) (X : Int) :
    decide ((if B then X else X + -18446744073709551616) < 0) =
      (if B then decide (X < 0) else decide (X < 18446744073709551616)) := by
  cases B <;> simp <;> omega

-- [pack-fixed64-product-low-word]   requires 0 <= L < 2^64
theorem pack_fixed64_product_low_word (Hi L : Int) (h0 : 0 ≤ L) (h1 : L < W) :
    Int.lor (Hi * W) L = Hi * W + L :=
  Fixed64Packing.fixed64_packing Hi L h0 (by simpa using h1)

-- [double-ray-half-up-nonnegative]   requires 0 <= X
theorem double_ray_half_up_nonnegative (X : Int) (h : 0 ≤ X) :
    (X * 2000000000000000000000000000 + 500000000000000000000000000).tdiv
      1000000000000000000000000000 = X * 2 :=
  DoubleRay.half_up X h

-- [signed-fixed64-decoder-roundtrip]
theorem signed_fixed64_decoder_roundtrip (X : Int) :
    ((X - X % W).tdiv W) * W + X % W = X := by
  rw [Int.tdiv_mul_cancel Int.dvd_self_sub_emod, Int.sub_add_cancel]

-- [floor-fixed64-word-equality]
theorem floor_fixed64_word_equality (X N : Int) :
    decide ((X - X % W).tdiv W = N) =
      (decide (N * W ≤ X) && decide (X < (N + 1) * W)) := by
  have := FloorEquality.word64_eq_iff X N
  simp only [FloorEquality.word64] at this
  by_cases h : (X - X % W).tdiv W = N
  · have := this.mp h; simp [h, this.1, this.2]
  · have hn : ¬ (N * W ≤ X ∧ X < (N + 1) * W) := fun hh => h (this.mpr hh)
    by_cases h1 : N * W ≤ X
    · have h2 : ¬ X < (N + 1) * W := fun h2 => hn ⟨h1, h2⟩
      simp [h, h1, h2]
    · simp [h, h1]

-- [signed-fixed64-low-word]
theorem signed_fixed64_low_word (X : Int) :
    (if X < 0 then X + 340282366920938463463374607431768211456 else X) % W = X % W :=
  LowWord.integer_eq X

-- [signed64-masked-nonnegative]   (#signed is defined: 0 <= X mod 2^64 < 2^64)
theorem signed64_masked_nonnegative (X : Int) :
    (0 ≤ X % W ∧ X % W < W) ∧
      decide (signed64 (X % W) ≥ 0) = decide (X % W < H) := by
  refine ⟨⟨Int.emod_nonneg X (by decide), Int.emod_lt_of_pos X (by decide)⟩, ?_⟩
  have h0 : 0 ≤ X % W := Int.emod_nonneg X (by decide)
  have h1 : X % W < W := Int.emod_lt_of_pos X (by decide)
  simp only [signed64, ge_iff_le]
  by_cases h : X % W < H
  · simp only [h, if_true, decide_true]
    simp [h0]
  · simp only [h, if_false, decide_false]
    simp only [decide_eq_false_iff_not]
    omega

-- [fixed64-masked-xor-sign-bit]
theorem fixed64_masked_xor_sign_bit (X Y : Int) :
    decide (intXor X Y % W < H) = (decide (X % W < H) == decide (Y % W < H)) :=
  IndependentFixed64Xor.fixed64_xor_sign X Y

-- [fixed64-projected-xor-masked-sign-bit]
theorem fixed64_projected_xor_masked_sign_bit (X Y : Int) :
    decide (u64Xor X Y % W < H) = (decide (X % W < H) == decide (Y % W < H)) :=
  IndependentFixed64Xor.fixed64_helper_sign X Y

-- [fixed64-mod-idempotence]
theorem fixed64_mod_idempotence (X : Int) : (X % W) % W = X % W := Int.emod_emod X W

-- [fixed64-projected-shift-one-carry]   requires 0 <= L < 2
theorem fixed64_projected_shift_one_carry (Hi L : Int) (h0 : 0 ≤ L) (h1 : L < 2) :
    u64Or ((Hi <<< (1 : Nat)) % W) L = (Hi % H) * 2 + L :=
  CarryWord.fixed64_projected_shift_one_carry Hi L h0 h1

-- [fixed64-shift-one-low-projection]
theorem fixed64_shift_one_low_projection (X : Int) :
    ((X % W) <<< (1 : Nat)) % W = (X * 2) % W :=
  DoubleWord.doubled_low_int X

-- [fixed64-double-high-recomposition]
theorem fixed64_double_high_recomposition (X : Int) :
    ((X - X % W).tdiv W) * 2 + ((X % W - (X % W) % H).tdiv H) =
      (X * 2 - (X * 2) % W).tdiv W :=
  DoubleWord.doubled_high_int X

-- Guard checks: the literal constants are the intended powers of two.
theorem constants : W = 2 ^ 64 ∧ H = 2 ^ 63 ∧
    (340282366920938463463374607431768211456 : Int) = 2 ^ 128 := by decide

#print axioms distribute_signed_word_comparison
#print axioms xor_zero_left_nonnegative_complement
#print axioms integer_self_subtraction
#print axioms floor_word64_less_than
#print axioms distribute_signed_negative_word_comparison
#print axioms pack_fixed64_product_low_word
#print axioms double_ray_half_up_nonnegative
#print axioms signed_fixed64_decoder_roundtrip
#print axioms floor_fixed64_word_equality
#print axioms signed_fixed64_low_word
#print axioms signed64_masked_nonnegative
#print axioms fixed64_masked_xor_sign_bit
#print axioms fixed64_projected_xor_masked_sign_bit
#print axioms fixed64_mod_idempotence
#print axioms fixed64_projected_shift_one_carry
#print axioms fixed64_shift_one_low_projection
#print axioms fixed64_double_high_recomposition
#print axioms constants

end DoubleCliRules
