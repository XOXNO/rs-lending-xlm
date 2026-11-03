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

end PoolMathRules
