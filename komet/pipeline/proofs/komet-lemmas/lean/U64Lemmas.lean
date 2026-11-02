import KBits
import IntBitwiseLemmas

/-!
Module U64-BITWISE-SMT and the U64 rules of INT-BITWISE-LEMMAS (Komet fork
commit 0593a68, `komet-lemmas.md`).

`#u64Or(X, Y)` / `#u64Xor(X, Y)` (symbols `u64Or` / `u64Xor`) are total functions
whose concrete evaluation rules are `(X |Int Y) modInt 2^64` and
`(X xorInt Y) modInt 2^64`, and whose SMT translation is
`bv2int (bvor/bvxor ((_ int2bv 64) X) ((_ int2bv 64) Y))`. SMT-LIB `int2bv 64`
maps an integer to its residue modulo 2^64 and `bv2int` reads a vector as an
unsigned integer, so the hook denotes the unsigned OR/XOR of the two residues.
`smt_hook_or` and `smt_hook_xor` prove that this equals the K rule, so the
solver and the rewriter agree on every integer input, negative ones included.
-/
namespace U64Lemmas

open KBits IntBitwiseLemmas

abbrev W : Int := P 64          -- 2^64
abbrev Half : Int := P 63       -- 2^63

def u64Or (a b : Int) : Int := lor a b % W
def u64Xor (a b : Int) : Int := xor a b % W

/-- The SMT hook's value: the natural-number OR/XOR of the two 64-bit residues. -/
def hookOr (a b : Int) : Int := (((a % W).toNat ||| (b % W).toNat : Nat) : Int)
def hookXor (a b : Int) : Int := (((a % W).toNat ^^^ (b % W).toNat : Nat) : Int)

theorem emod_W_natCast (a : Int) : ((a % W).toNat : Int) = a % W :=
  Int.toNat_of_nonneg (Int.emod_nonneg a (by decide))

theorem smt_hook_or (a b : Int) : hookOr a b = u64Or a b := by
  have h : lor (a % W) (b % W) = hookOr a b := by
    unfold hookOr
    rw [← emod_W_natCast a, ← emod_W_natCast b]
    rfl
  rw [← h]
  exact ext fun i => by
    simp only [u64Or, W, bit_lor, bit_emod_two_pow]
    cases decide (i < 64) <;> simp

theorem smt_hook_xor (a b : Int) : hookXor a b = u64Xor a b := by
  have h : xor (a % W) (b % W) = hookXor a b := by
    unfold hookXor
    rw [← emod_W_natCast a, ← emod_W_natCast b]
    rfl
  rw [← h]
  exact ext fun i => by
    simp only [u64Xor, W, bit_xor, bit_emod_two_pow]
    cases decide (i < 64) <;> simp

-- [u64-or-native] / [project-u64-or]: (A |Int B) modInt 2^64 <=> #u64Or(A, B)
theorem project_u64_or (A B : Int) : lor A B % W = u64Or A B := rfl
-- [u64-xor-native] / [project-u64-xor]
theorem project_u64_xor (A B : Int) : xor A B % W = u64Xor A B := rfl

-- [u64-or-zero-right] #u64Or(A, B) ==Int 0 => (A modInt 2^64 ==Int 0) andBool (B modInt 2^64 ==Int 0)
theorem u64_or_zero_right (A B : Int) :
    decide (u64Or A B = 0) = (decide (A % W = 0) && decide (B % W = 0)) :=
  masked_or_zero_right A B 64

-- [u64-or-zero-left] 0 ==Int #u64Or(A, B) => same
theorem u64_or_zero_left (A B : Int) :
    decide (0 = u64Or A B) = (decide (A % W = 0) && decide (B % W = 0)) :=
  masked_or_zero_left A B 64

-- [u64-xor-sign-bit] #u64Xor(X, Y) <Int 2^63
--   => ((X modInt 2^64) <Int 2^63) ==Bool ((Y modInt 2^64) <Int 2^63)
theorem u64_xor_sign_bit (X Y : Int) :
    decide (u64Xor X Y < Half) = (decide (X % W < Half) == decide (Y % W < Half)) :=
  masked_xor_sign_bit X Y 63

-- [u64-xor-sign-bit-complement] 2^63 <=Int #u64Xor(X, Y) => notBool (#u64Xor(X, Y) <Int 2^63)
theorem u64_xor_sign_bit_complement (X Y : Int) :
    decide (Half ≤ u64Xor X Y) = !decide (u64Xor X Y < Half) := by
  have : Half ≤ u64Xor X Y ↔ ¬ u64Xor X Y < Half := by omega
  rw [decide_eq_decide.mpr this, decide_not]

-- [u64-xor-nonnegative] #u64Xor(_X, _Y) <Int 0 => false
theorem u64_xor_nonnegative (X Y : Int) : ¬ u64Xor X Y < 0 := by
  have := Int.emod_nonneg (xor X Y) (by decide : W ≠ 0)
  simp only [u64Xor]
  omega

-- [u64-xor-upper-bound] #u64Xor(_X, _Y) <Int 2^64 => true
theorem u64_xor_upper_bound (X Y : Int) : u64Xor X Y < W :=
  Int.emod_lt_of_pos _ (by decide)

-- [project-u64-shift32] (A <<Int 32) modInt 2^64 => (A modInt 2^32) *Int 2^32
-- (A <<Int 32 = A * 2^32 for the literal nonnegative shift.)
theorem project_u64_shift32 (A : Int) : (A * P 32) % W = (A % P 32) * P 32 := by
  have hW : W = P 32 * P 32 := by decide
  rw [hW, Int.mul_comm A, Int.mul_emod_mul_of_pos _ _ (by decide), Int.mul_comm]

-- [u64-pack-bounded-low32] #u64Or(H *Int 2^32, L) => (H modInt 2^32) *Int 2^32 +Int L
--   requires 0 <=Int L andBool L <Int 2^32
theorem u64_pack_bounded_low32 (H L : Int) (h0 : 0 ≤ L) (h1 : L < P 32) :
    u64Or (H * P 32) L = (H % P 32) * P 32 + L := by
  unfold u64Or
  have hLW : L < W := Int.lt_trans h1 (by decide)
  rw [pack H L 32 h0 h1, Int.add_emod, project_u64_shift32, Int.emod_eq_of_lt h0 hLW]
  have hq0 : 0 ≤ H % P 32 := Int.emod_nonneg H (by decide)
  have hq1 : H % P 32 < P 32 := Int.emod_lt_of_pos H (by decide)
  apply Int.emod_eq_of_lt
  · have : 0 ≤ H % P 32 * P 32 := Int.mul_nonneg hq0 (by decide)
    omega
  · have : H % P 32 * P 32 ≤ (P 32 - 1) * P 32 := Int.mul_le_mul_of_nonneg_right (by omega) (by decide)
    simp only [P, W] at *
    omega

#print axioms smt_hook_or
#print axioms smt_hook_xor
#print axioms u64_or_zero_right
#print axioms u64_or_zero_left
#print axioms u64_xor_sign_bit
#print axioms u64_xor_sign_bit_complement
#print axioms u64_xor_nonnegative
#print axioms u64_xor_upper_bound
#print axioms project_u64_shift32
#print axioms u64_pack_bounded_low32

end U64Lemmas
