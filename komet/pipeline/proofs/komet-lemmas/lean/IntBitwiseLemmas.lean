import KBits

/-!
One theorem per rule of module INT-BITWISE-LEMMAS in the Komet fork's
`src/komet/kdist/soroban-semantics/komet-lemmas.md`, stated over the bit model
of `KBits`. Side conditions written with K helper functions are instantiated
by their definitions in the same module:
  isPowerOf2(M)  <=> 0 < M and M == 1 <<Int log2Int(M)          i.e. M = 2^k
  isFullMask(M)  <=> 0 < M and M == fullMask(log2Int(M) +Int 1)  i.e. M = 2^k - 1, k >= 1
  fullMask(n)    =   (1 <<Int n) -Int 1 for n > 0
For a nonnegative shift, `X <<Int n` is `X * 2^n` and `X >>Int n` is the
floor quotient `X divInt 2^n` (GMP mul_2exp / fdiv_q_2exp); `2 ^Int n` is `2^n`.
-/
namespace IntBitwiseLemmas

open KBits

abbrev P (k : Nat) : Int := ((2 ^ k : Nat) : Int)

-- rule C |Int S => S |Int C   [concrete(C), symbolic(S)]
theorem or_comm (C S : Int) : lor C S = lor S C :=
  ext fun i => by rw [bit_lor, bit_lor, Bool.or_comm]

-- rule X |Int 0 => X
theorem or_zero (X : Int) : lor X 0 = X :=
  ext fun i => by rw [bit_lor, bit_zero, Bool.or_false]

-- [xor-zero-right] I xorInt 0 => I
theorem xor_zero_right (I : Int) : xor I 0 = I :=
  ext fun i => by rw [bit_xor, bit_zero, Bool.xor_false]

-- [xor-zero-left] 0 xorInt I => I
theorem xor_zero_left (I : Int) : xor 0 I = I :=
  ext fun i => by rw [bit_xor, bit_zero, Bool.false_xor]

-- rule _X &Int 0 => 0   (erases X; keeps the default definedness check)
theorem and_zero (X : Int) : land X 0 = 0 :=
  ext fun i => by rw [bit_land, bit_zero, Bool.and_false]

-- [masked-int-nonnegative] 0 <=Int (_X &Int MASK) => true  requires 0 <=Int MASK
theorem masked_int_nonnegative (X MASK : Int) (h : 0 ≤ MASK) : 0 ≤ land X MASK := by
  obtain ⟨m, rfl⟩ := Int.eq_ofNat_of_zero_le h
  cases X <;> simp [land] <;> omega

theorem land_le_mask (X MASK : Int) (h : 0 ≤ MASK) : land X MASK ≤ MASK := by
  obtain ⟨m, rfl⟩ := Int.eq_ofNat_of_zero_le h
  cases X with
  | ofNat a =>
    show ((a &&& m : Nat) : Int) ≤ (m : Int)
    exact Int.ofNat_le.mpr Nat.and_le_right
  | negSucc a =>
    show ((Nat.bitwise (fun x y => !x && y) a m : Nat) : Int) ≤ (m : Int)
    apply Int.ofNat_le.mpr
    apply Nat.le_of_testBit
    intro i hi
    rw [testBit_notAnd] at hi
    cases hm : m.testBit i <;> simp_all

-- [masked-int-upper-bound] (_X &Int MASK) <Int BOUND => true
--   requires 0 <=Int MASK andBool MASK <Int BOUND
theorem masked_int_upper_bound (X MASK BOUND : Int) (h0 : 0 ≤ MASK) (h1 : MASK < BOUND) :
    land X MASK < BOUND :=
  Int.lt_of_le_of_lt (land_le_mask X MASK h0) h1

-- rule A &Int B => B &Int A   [concrete(A), symbolic(B)]
theorem and_comm (A B : Int) : land A B = land B A :=
  ext fun i => by rw [bit_land, bit_land, Bool.and_comm]

-- rule (A &Int B) &Int C => A &Int (B &Int C)   and its converse
theorem and_assoc (A B C : Int) : land (land A B) C = land A (land B C) :=
  ext fun i => by rw [bit_land, bit_land, bit_land, bit_land, Bool.and_assoc]

theorem bit_mask (k i : Nat) : bit (P k - 1) i = decide (i < k) := by
  have : P k - 1 = ((2 ^ k - 1 : Nat) : Int) := by
    have := Nat.two_pow_pos k
    simp only [P]
    omega
  rw [this, bit_natCast, Nat.testBit_two_pow_sub_one]

-- [full-mask-to-modulus] I &Int MASK => I modInt (MASK +Int 1)  requires isFullMask(MASK)
-- (MASK = 2^k - 1 with k = log2Int(MASK) + 1; the statement holds for every k.)
theorem full_mask_to_modulus (I : Int) (k : Nat) : land I (P k - 1) = I % (P k - 1 + 1) := by
  rw [Int.sub_add_cancel]
  exact ext fun i => by
    rw [bit_land, bit_mask, bit_emod_two_pow, Bool.and_comm]

-- [masked-or-zero-right] ((A |Int B) modInt M) ==Int 0
--   => (A modInt M ==Int 0) andBool (B modInt M ==Int 0)   requires 0 < M, isPowerOf2(M)
theorem masked_or_zero_right (A B : Int) (k : Nat) :
    decide (lor A B % P k = 0) = (decide (A % P k = 0) && decide (B % P k = 0)) := by
  have hab : lor A B % P k = 0 ↔ (A % P k = 0 ∧ B % P k = 0) := by
    simp only [P, emod_eq_zero_iff, bit_lor, Bool.or_eq_false_iff]
    constructor
    · intro h
      exact ⟨fun i hi => (h i hi).1, fun i hi => (h i hi).2⟩
    · intro h i hi
      exact ⟨h.1 i hi, h.2 i hi⟩
  rw [decide_eq_decide.mpr hab, Bool.decide_and]

-- [masked-or-zero-left] 0 ==Int ((A |Int B) modInt M) => same right side
theorem masked_or_zero_left (A B : Int) (k : Nat) :
    decide (0 = lor A B % P k) = (decide (A % P k = 0) && decide (B % P k = 0)) := by
  rw [← masked_or_zero_right, decide_eq_decide.mpr eq_comm]

theorem xor_false_aux (a b : Bool) :
    decide ((a ^^ b) = false) = (decide (a = false) == decide (b = false)) := by
  cases a <;> cases b <;> rfl

-- [masked-xor-sign-bit] ((X xorInt Y) modInt M) <Int B
--   => ((X modInt M) <Int B) ==Bool ((Y modInt M) <Int B)
--   requires 0 < B, isPowerOf2(B), M ==Int 2 *Int B      (B = 2^j, M = 2^(j+1))
theorem masked_xor_sign_bit (X Y : Int) (j : Nat) :
    decide (xor X Y % P (j + 1) < P j) =
      (decide (X % P (j + 1) < P j) == decide (Y % P (j + 1) < P j)) := by
  have hx := emod_lt_half_iff X j
  have hy := emod_lt_half_iff Y j
  have hxy := emod_lt_half_iff (xor X Y) j
  rw [bit_xor] at hxy
  simp only [P] at *
  rw [decide_eq_decide.mpr hxy, decide_eq_decide.mpr hx, decide_eq_decide.mpr hy]
  exact xor_false_aux _ _

-- [masked-complement-and-right] ((X xorInt MASK) modInt M) &Int X => 0
--   requires 0 < M, isPowerOf2(M), MASK ==Int M -Int 1        (M = 2^k)
theorem masked_complement_and_right (X : Int) (k : Nat) :
    land (xor X (P k - 1) % P k) X = 0 :=
  ext fun i => by
    rw [bit_land, bit_emod_two_pow, bit_xor, bit_mask, bit_zero]
    cases decide (i < k) <;> cases bit X i <;> rfl

-- [masked-complement-and-left] X &Int ((X xorInt MASK) modInt M) => 0
theorem masked_complement_and_left (X : Int) (k : Nat) :
    land X (xor X (P k - 1) % P k) = 0 := by
  rw [and_comm]
  exact masked_complement_and_right X k

-- [constant-right-shift] I >>Int SHIFT => I divInt (2 ^Int SHIFT)  requires 0 <=Int SHIFT
theorem constant_right_shift (I : Int) (s : Nat) : I >>> s = I / P s :=
  Int.shiftRight_eq_div_pow I s

/-- Packing: a shifted high word and a low word below `2^w` occupy disjoint bits. -/
theorem pack (H L : Int) (w : Nat) (h0 : 0 ≤ L) (h1 : L < P w) :
    lor (H * P w) L = H * P w + L := by
  have hp : 0 < P w := by
    have := Nat.two_pow_pos w
    simp only [P]
    omega
  have hmod : (H * P w + L) % P w = L := by
    rw [Int.add_comm, Int.add_mul_emod_self_right]
    exact Int.emod_eq_of_lt h0 h1
  have hdiv : (H * P w + L) / P w = H := by
    rw [Int.add_comm, Int.add_mul_ediv_right _ _ (Int.ne_of_gt hp), Int.ediv_eq_zero_of_lt h0 h1,
      Int.zero_add]
  obtain ⟨l, rfl⟩ := Int.eq_ofNat_of_zero_le h0
  have hl : l < 2 ^ w := by simp only [P] at h1; omega
  apply ext
  intro i
  rw [bit_lor, bit_mul_two_pow]
  by_cases hi : w ≤ i
  · have hlow : bit (l : Int) i = false := by
      rw [bit_natCast]
      exact Nat.testBit_lt_two_pow
        (Nat.lt_of_lt_of_le hl (Nat.pow_le_pow_right (by decide) hi))
    have := bit_ediv_two_pow (H * P w + l) w (i - w)
    rw [hdiv, Nat.add_sub_cancel' hi] at this
    rw [hlow, ← this]
    simp [hi]
  · have := bit_emod_two_pow (H * P w + l) w i
    rw [hmod] at this
    have hi' : i < w := by omega
    simp only [hi', decide_true, Bool.true_and] at this
    rw [← this]
    simp [hi]

-- [pack-bounded-low-word] (H <<Int WIDTH) |Int L => H *Int (2 ^Int WIDTH) +Int L
--   requires 0 <=Int WIDTH andBool 0 <=Int L andBool L <Int (2 ^Int WIDTH)
theorem pack_bounded_low_word (H L : Int) (w : Nat) (h0 : 0 ≤ L) (h1 : L < P w) :
    lor (H * P w) L = H * P w + L :=
  pack H L w h0 h1

theorem lor_mul_two_pow (x y : Int) (k : Nat) : lor (x * P k) (y * P k) = lor x y * P k :=
  ext fun i => by
    rw [bit_lor, bit_mul_two_pow, bit_mul_two_pow, bit_mul_two_pow, bit_lor]
    cases decide (k ≤ i) <;> rfl

-- [pack-four-u64-limbs]
--   (((A <<Int 192) |Int (B <<Int 128)) |Int (C <<Int 64)) |Int D
--     => A *Int 2^192 +Int B *Int 2^128 +Int C *Int 2^64 +Int D
--   requires each of A, B, C, D in [0, 2^64)
theorem pack_four_u64_limbs (A B C D : Int)
    (hB0 : 0 ≤ B) (hB1 : B < P 64) (hC0 : 0 ≤ C) (hC1 : C < P 64)
    (hD0 : 0 ≤ D) (hD1 : D < P 64) :
    lor (lor (lor (A * P 192) (B * P 128)) (C * P 64)) D =
      A * P 192 + B * P 128 + C * P 64 + D := by
  have e192 : P 192 = P 64 * P 128 := by decide
  have e128 : P 128 = P 64 * P 64 := by decide
  have s1 : lor (A * P 192) (B * P 128) = (A * P 64 + B) * P 128 := by
    rw [e192, ← Int.mul_assoc, lor_mul_two_pow, pack A B 64 hB0 hB1]
  have s2 : lor ((A * P 64 + B) * P 128) (C * P 64) = ((A * P 64 + B) * P 64 + C) * P 64 := by
    rw [e128, ← Int.mul_assoc, lor_mul_two_pow, pack _ C 64 hC0 hC1]
  rw [s1, s2, pack _ D 64 hD0 hD1, e192, e128]
  simp only [Int.add_mul, Int.mul_assoc]

#print axioms or_comm
#print axioms or_zero
#print axioms xor_zero_right
#print axioms xor_zero_left
#print axioms and_zero
#print axioms masked_int_nonnegative
#print axioms masked_int_upper_bound
#print axioms and_comm
#print axioms and_assoc
#print axioms full_mask_to_modulus
#print axioms masked_or_zero_right
#print axioms masked_or_zero_left
#print axioms masked_xor_sign_bit
#print axioms masked_complement_and_right
#print axioms masked_complement_and_left
#print axioms constant_right_shift
#print axioms pack_bounded_low_word
#print axioms pack_four_u64_limbs

/-! Side-condition helpers (definitions in the same module; `log2Int` is the
floor logarithm on positive integers, `1 <<Int n = 2^n`). -/

-- isPowerOf2(M) for 0 < M: M ==Int 1 <<Int log2Int(M)  <=>  M is a power of two.
theorem isPowerOf2_iff (m : Nat) (h : 0 < m) : m = 2 ^ m.log2 ↔ ∃ k, m = 2 ^ k := by
  constructor
  · intro e
    exact ⟨_, e⟩
  · rintro ⟨k, rfl⟩
    rw [Nat.log2_two_pow]

-- isFullMask(M) for 0 < M: M ==Int fullMask(log2Int(M) +Int 1) = 2^(log2 M + 1) - 1
--   <=>  M = 2^k - 1 for some k >= 1.
theorem isFullMask_iff (m : Nat) (h : 0 < m) :
    m = 2 ^ (m.log2 + 1) - 1 ↔ ∃ k, 1 ≤ k ∧ m = 2 ^ k - 1 := by
  constructor
  · intro e
    exact ⟨_, by omega, e⟩
  · rintro ⟨k, hk, rfl⟩
    have hne : 2 ^ k - 1 ≠ 0 := by omega
    have hlo : k - 1 ≤ (2 ^ k - 1).log2 := by
      rw [Nat.le_log2 hne]
      have : 2 ^ k = 2 * 2 ^ (k - 1) := by
        rw [← Nat.pow_succ']
        congr 1
        omega
      omega
    have hhi : (2 ^ k - 1).log2 < k := by
      rw [Nat.log2_lt hne]
      omega
    have : (2 ^ k - 1).log2 = k - 1 := by omega
    rw [this, Nat.sub_add_cancel hk]

#print axioms isPowerOf2_iff
#print axioms isFullMask_iff

end IntBitwiseLemmas
