import KBits
import IntBitwiseLemmas
import U64Lemmas

/-!
Rules added by `komet/pipeline/proofs/pool-math-lemmas.k` on top of the 39
rules of `pool-math/proofs/utilization/arithmetic.k` (whose Lean coverage is
`DoubleCliRules.lean` and `UtilizationRules.lean`). `u64Xor` / `u64Or` are the
Komet symbols `#u64Xor` / `#u64Or`, modelled as in `U64Lemmas`.
-/
namespace PoolMathRules

open KBits IntBitwiseLemmas U64Lemmas

-- [u64-xor-zero-right]  u64Xor(A, B) ==Int 0 => (A modInt 2^64) ==Int (B modInt 2^64)
theorem u64_xor_zero_right (A B : Int) :
    decide (u64Xor A B = 0) = decide (A % W = B % W) := by
  apply decide_eq_decide.mpr
  constructor
  · intro h
    apply ext
    intro i
    have := congrArg (fun z => bit z i) h
    simp only [u64Xor, W, bit_emod_two_pow, bit_xor, bit_zero] at this
    rw [bit_emod_two_pow, bit_emod_two_pow]
    revert this
    cases decide (i < 64) <;> cases bit A i <;> cases bit B i <;> simp
  · intro h
    apply ext
    intro i
    have := congrArg (fun z => bit z i) h
    simp only [W, bit_emod_two_pow] at this
    simp only [u64Xor, W, bit_emod_two_pow, bit_xor, bit_zero]
    revert this
    cases decide (i < 64) <;> cases bit A i <;> cases bit B i <;> simp

-- [u64-xor-zero-left]  0 ==Int u64Xor(A, B) => (A modInt 2^64) ==Int (B modInt 2^64)
theorem u64_xor_zero_left (A B : Int) :
    decide (0 = u64Xor A B) = decide (A % W = B % W) := by
  rw [← u64_xor_zero_right, decide_eq_decide.mpr eq_comm]

theorem or_false_aux (a b : Bool) :
    decide ((a || b) = false) = (decide (a = false) && decide (b = false)) := by
  cases a <;> cases b <;> rfl

-- [u64-or-sign-bit]  u64Or(A, B) <Int 2^63
--   => (A modInt 2^64 <Int 2^63) andBool (B modInt 2^64 <Int 2^63)
theorem u64_or_sign_bit (A B : Int) :
    decide (u64Or A B < Half) = (decide (A % W < Half) && decide (B % W < Half)) := by
  have ha := emod_lt_half_iff A 63
  have hb := emod_lt_half_iff B 63
  have hab := emod_lt_half_iff (lor A B) 63
  rw [bit_lor] at hab
  simp only [u64Or, W, Half, P] at *
  rw [decide_eq_decide.mpr hab, decide_eq_decide.mpr ha, decide_eq_decide.mpr hb]
  exact or_false_aux _ _

-- [ray-identity-half-up]  (X *Int 10^27 +Int 5*10^26) /Int 10^27 => X  requires 0 <=Int X
theorem ray_identity_half_up (X : Int) (h : 0 ≤ X) :
    (X * 1000000000000000000000000000 + 500000000000000000000000000).tdiv
      1000000000000000000000000000 = X := by
  rw [Int.tdiv_eq_ediv_of_nonneg (by omega)]
  omega

#print axioms u64_xor_zero_right
#print axioms u64_xor_zero_left
#print axioms or_false_aux
#print axioms u64_or_sign_bit
#print axioms ray_identity_half_up

def bw (b : Bool) : Int := if b then 1 else 0

-- [bool-word-and] / [bool-word-or] / [bool-word-xor]
theorem bool_word_and (b1 b2 : Bool) : land (bw b1) (bw b2) = bw (b1 && b2) := by
  cases b1 <;> cases b2 <;> rfl
theorem bool_word_or (b1 b2 : Bool) : lor (bw b1) (bw b2) = bw (b1 || b2) := by
  cases b1 <;> cases b2 <;> rfl
theorem bool_word_xor (b1 b2 : Bool) : xor (bw b1) (bw b2) = bw (b1 ^^ b2) := by
  cases b1 <;> cases b2 <;> rfl

-- [bool-word-mod]  requires 2 <=Int M
theorem bool_word_mod (b : Bool) (M : Int) (h : 2 ≤ M) : bw b % M = bw b := by
  cases b <;> simp only [bw] <;> apply Int.emod_eq_of_lt <;> omega

#print axioms bool_word_and
#print axioms bool_word_or
#print axioms bool_word_xor
#print axioms bool_word_mod

theorem and_false_aux (a b : Bool) :
    decide ((a && b) = false) = (decide (a = false) || decide (b = false)) := by
  cases a <;> cases b <;> rfl

theorem land_sign (X Y : Int) (hx0 : 0 ≤ X) (hx1 : X < W) (hy0 : 0 ≤ Y) (hy1 : Y < W) :
    decide (land X Y < Half) = (decide (X < Half) || decide (Y < Half)) := by
  have hxy0 : 0 ≤ land X Y := masked_int_nonnegative X Y hy0
  have hxy1 : land X Y < W := Int.lt_of_le_of_lt (land_le_mask X Y hy0) hy1
  have ex : X % W = X := Int.emod_eq_of_lt hx0 hx1
  have ey : Y % W = Y := Int.emod_eq_of_lt hy0 hy1
  have exy : land X Y % W = land X Y := Int.emod_eq_of_lt hxy0 hxy1
  have ha := emod_lt_half_iff X 63
  have hb := emod_lt_half_iff Y 63
  have hab := emod_lt_half_iff (land X Y) 63
  rw [bit_land] at hab
  simp only [W, Half, P] at *
  rw [ex] at ha; rw [ey] at hb; rw [exy] at hab
  rw [decide_eq_decide.mpr hab, decide_eq_decide.mpr ha, decide_eq_decide.mpr hb]
  exact and_false_aux _ _

-- [u64-xor-and-sign]: both operands are u64Xor values, hence in [0, 2^64).
theorem u64_xor_and_sign (A B C D : Int) :
    decide (land (u64Xor A B) (u64Xor C D) < Half) =
      (decide (u64Xor A B < Half) || decide (u64Xor C D < Half)) :=
  land_sign _ _ (Int.le_of_lt_add_one (by have := u64_xor_nonnegative A B; omega))
    (u64_xor_upper_bound A B)
    (Int.le_of_lt_add_one (by have := u64_xor_nonnegative C D; omega)) (u64_xor_upper_bound C D)

-- [u64-xor-and-sign-ge]
theorem u64_xor_and_sign_ge (A B C D : Int) :
    decide (Half ≤ land (u64Xor A B) (u64Xor C D)) =
      !(decide (u64Xor A B < Half) || decide (u64Xor C D < Half)) := by
  rw [← u64_xor_and_sign]
  have : Half ≤ land (u64Xor A B) (u64Xor C D) ↔ ¬ land (u64Xor A B) (u64Xor C D) < Half := by omega
  rw [decide_eq_decide.mpr this, decide_not]

#print axioms land_sign
#print axioms u64_xor_and_sign
#print axioms u64_xor_and_sign_ge

-- [mul-div-cancel]  (X *Int C) /Int C => X  requires C =/=Int 0
theorem mul_div_cancel (X C : Int) (h : C ≠ 0) : (X * C).tdiv C = X :=
  Int.mul_tdiv_cancel X h

#print axioms mul_div_cancel

-- [i256-shr1-limbs]  the 128-bit arithmetic shift right by one, recombined
-- from its two u64 limbs (hi = (X - X mod 2^64) / 2^64, lo = X mod 2^64),
-- is X /Int 2 for every X >= 0.
theorem i256_shr1_limbs (X : Int) (h : 0 ≤ X) :
    ((((X - X % 18446744073709551616).tdiv 18446744073709551616
        - ((X - X % 18446744073709551616).tdiv 18446744073709551616) % 2).tdiv 2)
        * 18446744073709551616)
      + (((((X - X % 18446744073709551616).tdiv 18446744073709551616) % 2)
          * 9223372036854775808)
        + ((X % 18446744073709551616 - (X % 18446744073709551616) % 2).tdiv 2))
      = X.tdiv 2 := by
  have hlo0 : 0 ≤ X % 18446744073709551616 := Int.emod_nonneg _ (by decide)
  have hlo1 : X % 18446744073709551616 < 18446744073709551616 := Int.emod_lt_of_pos _ (by decide)
  have hhi : (X - X % 18446744073709551616).tdiv 18446744073709551616 = X / 18446744073709551616 := by
    rw [Int.tdiv_eq_ediv_of_nonneg (by omega)]; omega
  rw [hhi]
  have hq0 : 0 ≤ X / 18446744073709551616 := Int.ediv_nonneg h (by decide)
  rw [Int.tdiv_eq_ediv_of_nonneg (by omega), Int.tdiv_eq_ediv_of_nonneg (by omega),
    Int.tdiv_eq_ediv_of_nonneg h]
  omega

#print axioms i256_shr1_limbs

end PoolMathRules
