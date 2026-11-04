import KBits
import IntBitwiseLemmas

/-!
Rules of module HOST-OBJECT-LEMMAS (Komet fork `komet-lemmas.md`), over:
* `KBits` for K's bit operations;
* `minWidth`, the KWasm `#minWidth` on nonnegative arguments (numeric.md:
  `#minWidth(0) = 0`, `#minWidth(N) = 1 + #minWidth(N >>Int 1)` for `N > 0`);
  `minWidth_recursion` proves this definition satisfies those equations;
* a byte-function model of KWasm linear memory (`SparseBytes`): unmapped
  bytes read as zero; `#getRange(BM, A, W)` is the little-endian unsigned
  value of `W` bytes at `A` (0 unless `0 < W` and `0 <= A`); `#setRange(BM, A,
  V, W)` writes the low `W` bytes of `V` little-endian (`Int2Bytes(W, V, LE)`)
  and is the identity unless `0 < W`, `0 <= V` and `0 <= A`.
-/
namespace HostObjectLemmas

open KBits IntBitwiseLemmas

/-! ### `#minWidth` -/

def minWidth (n : Nat) : Nat := if n = 0 then 0 else n.log2 + 1

theorem lt_minWidth_iff (n t : Nat) : t < minWidth n ↔ 2 ^ t ≤ n := by
  unfold minWidth
  by_cases h : n = 0
  · subst h
    have := Nat.two_pow_pos t
    rw [if_pos rfl]
    omega
  · simp only [h, if_false]
    rw [← Nat.le_log2 h]
    omega

theorem eq_of_lt_iff {a b : Nat} (h : ∀ t, t < a ↔ t < b) : a = b := by
  have h1 := h b
  have h2 := h a
  omega

theorem minWidth_recursion (n : Nat) :
    (n = 0 → minWidth n = 0) ∧ (0 < n → minWidth n = 1 + minWidth (n / 2)) := by
  refine ⟨fun h => by simp [minWidth, h], fun hn => ?_⟩
  apply eq_of_lt_iff
  intro t
  rw [lt_minWidth_iff]
  cases t with
  | zero => simp; omega
  | succ s =>
    rw [show s + 1 < 1 + minWidth (n / 2) ↔ s < minWidth (n / 2) by omega,
      lt_minWidth_iff, Nat.le_div_iff_mul_le (by decide), Nat.pow_succ]

theorem minWidth_le (n k : Nat) (h : n < 2 ^ k) : minWidth n ≤ k := by
  by_cases hc : k < minWidth n
  · have := (lt_minWidth_iff n k).mp hc
    omega
  · omega

-- [reverse-min-width-comparison]
--   ((C -Int #minWidth(N)) modInt M) >Int B => notBool ((B +Int 1) >Int ((C -Int #minWidth(N)) modInt M))
theorem reverse_min_width_comparison (x B : Int) : decide (x > B) = !decide (B + 1 > x) := by
  by_cases h : x > B
  · have : ¬ B + 1 > x := by omega
    simp [h, this]
  · have : B + 1 > x := by omega
    simp [h, this]

/-- Under the rules' guard `0 <= N < 2^(C modInt M)`, the reversed width is exact. -/
theorem width_mod (C M : Int) (N : Nat) (hM : 0 < M) (hN : N < 2 ^ (C % M).toNat) :
    (C - minWidth N) % M = (C % M) - minWidth N ∧ (minWidth N : Int) ≤ C % M := by
  have hk0 : 0 ≤ C % M := Int.emod_nonneg C (Int.ne_of_gt hM)
  have hk1 : C % M < M := Int.emod_lt_of_pos C hM
  have hle : (minWidth N : Int) ≤ C % M := by
    have := minWidth_le N (C % M).toNat hN
    omega
  refine ⟨?_, hle⟩
  rw [Int.sub_emod]
  have : (minWidth N : Int) % M = minWidth N := Int.emod_eq_of_lt (by omega) (by omega)
  rw [this]
  exact Int.emod_eq_of_lt (by omega) (by omega)

-- [bounded-min-width-comparison-nonpositive]  B >Int ((C -Int #minWidth(N)) modInt M) => false
--   requires 0 <Int M andBool 0 <=Int N andBool N <Int (2 ^Int (C modInt M)) andBool B <=Int 0
theorem bounded_min_width_nonpositive (C M B : Int) (N : Nat) (hM : 0 < M)
    (hN : N < 2 ^ (C % M).toNat) (hB : B ≤ 0) : ¬ B > (C - minWidth N) % M := by
  obtain ⟨h1, h2⟩ := width_mod C M N hM hN
  rw [h1]
  omega

-- [bounded-min-width-comparison-above]  ... => true   requires ... andBool B >Int (C modInt M)
theorem bounded_min_width_above (C M B : Int) (N : Nat) (hM : 0 < M)
    (hN : N < 2 ^ (C % M).toNat) (hB : B > C % M) : B > (C - minWidth N) % M := by
  obtain ⟨h1, h2⟩ := width_mod C M N hM hN
  rw [h1]
  omega

-- [bounded-min-width-comparison-threshold]
--   B >Int ((C -Int #minWidth(N)) modInt M) => N >=Int (2 ^Int ((C modInt M) -Int B))
--   requires ... andBool 0 <Int B andBool B <=Int (C modInt M)
theorem bounded_min_width_threshold (C M B : Int) (N : Nat) (hM : 0 < M)
    (hN : N < 2 ^ (C % M).toNat) (_hB0 : 0 < B) (hB1 : B ≤ C % M) :
    B > (C - minWidth N) % M ↔ (N : Int) ≥ ((2 ^ (C % M - B).toNat : Nat) : Int) := by
  obtain ⟨h1, h2⟩ := width_mod C M N hM hN
  rw [h1]
  have key := lt_minWidth_iff N (C % M - B).toNat
  have ht : ((C % M - B).toNat : Int) = C % M - B := Int.toNat_of_nonneg (by omega)
  constructor
  · intro h
    have : (C % M - B).toNat < minWidth N := by omega
    have := key.mp this
    omega
  · intro h
    have : 2 ^ (C % M - B).toNat ≤ N := by omega
    have := key.mpr this
    omega

-- [boolean-word-value] #bool(B) => #if B #then 1 #else 0 #fi
-- KWasm defines #bool by `#bool(B) => 1 requires B` and `#bool(B) => 0 requires notBool B`.
def boolToInt (b : Bool) : Int := match b with | true => 1 | false => 0
theorem boolean_word_value (b : Bool) : boolToInt b = if b then 1 else 0 := by
  cases b <;> rfl

-- [bitwise-mk-hostval-then-mask] (_I <<Int SHIFT |Int T) &Int MASK => T &Int MASK
--   requires isFullMask(MASK) andBool SHIFT >=Int log2Int(MASK +Int 1)   (MASK = 2^k - 1, k <= SHIFT)
theorem bitwise_mk_hostval_then_mask (I T : Int) (k s : Nat) (hks : k ≤ s) :
    land (lor (I * P s) T) (P k - 1) = land T (P k - 1) :=
  ext fun i => by
    rw [bit_land, bit_land, bit_lor, bit_mul_two_pow, bit_mask]
    by_cases hi : i < k
    · have : ¬ s ≤ i := by omega
      simp [hi, this]
    · simp [hi]

-- [wrap-in-range] #wrap(WIDTH, N) => N
--   requires 0 <Int WIDTH andBool 0 <=Int N andBool N <Int (1 <<Int (8 *Int WIDTH))
-- (#wrap(WIDTH, N) = N &Int ((1 <<Int (WIDTH *Int 8)) -Int 1) for 0 < WIDTH)
theorem wrap_in_range (N : Int) (w : Nat) (h0 : 0 ≤ N) (h1 : N < P (8 * w)) :
    land N (P (w * 8) - 1) = N := by
  rw [Nat.mul_comm, full_mask_to_modulus, Int.sub_add_cancel]
  exact Int.emod_eq_of_lt h0 h1

/-! ### `#signed`, `#signedTotal`, `#unsigned` (KWasm wasm-data.md)

`#signed(T, N)` is `N` on `[0, 2^(w-1))` and `N - 2^w` on `[2^(w-1), 2^w)`, and
undefined elsewhere; `#signedTotal(T, N)` equals `#signed(T, N)` where the latter is
defined. `#unsigned(T, N)` is `N + 2^w` for `N < 0` and `N` for `0 <= N`. -/

def signedPartial (pow1 pow : Int) (n : Int) : Option Int :=
  if 0 ≤ n ∧ n < pow1 then some n
  else if pow1 ≤ n ∧ n < pow then some (n - pow) else none

-- [signed-in-range] / [signed-total-in-range]
--   #signed(W, N) => #if N <Int #pow1(W) #then N #else N -Int #pow(W) #fi
--   requires 0 <=Int N andBool N <Int #pow(W)
theorem signed_in_range (pow1 pow N : Int) (h0 : 0 ≤ N) (h1 : N < pow) :
    signedPartial pow1 pow N = some (if N < pow1 then N else N - pow) := by
  unfold signedPartial
  by_cases h : N < pow1
  · simp [h, h0]
  · have : pow1 ≤ N := by omega
    simp [h, this, h1]

def unsignedDef (pow N : Int) : Int := if N < 0 then N + pow else N

-- [unsigned-in-range] #unsigned(W, N) => #if N <Int 0 #then N +Int #pow(W) #else N #fi
--   requires 0 -Int #pow1(W) <=Int N andBool N <Int #pow(W)   (inside definedUnsigned)
theorem unsigned_in_range (pow N : Int) : unsignedDef pow N = if N < 0 then N + pow else N := rfl

/-! ### Linear memory -/

abbrev Mem := Nat → Nat

def readLE (m : Mem) (a : Nat) : Nat → Nat
  | 0 => 0
  | w + 1 => readLE m a w + 256 ^ w * (m (a + w) % 256)

def getRange (m : Mem) (addr width : Int) : Int :=
  if 0 < width ∧ 0 ≤ addr then (readLE m addr.toNat width.toNat : Int) else 0

def setRange (m : Mem) (addr val width : Int) : Mem :=
  if 0 < width ∧ 0 ≤ val ∧ 0 ≤ addr then
    fun j => if addr.toNat ≤ j ∧ j < addr.toNat + width.toNat
      then val.toNat / 256 ^ (j - addr.toNat) % 256 else m j
  else m

def wrap (width N : Int) : Int := if 0 < width then land N (P (width.toNat * 8) - 1) else 0

theorem readLE_lt (m : Mem) (a w : Nat) : readLE m a w < 256 ^ w := by
  induction w with
  | zero => simp [readLE]
  | succ w ih =>
    simp only [readLE, Nat.pow_succ]
    have : m (a + w) % 256 < 256 := Nat.mod_lt _ (by decide)
    have h2 : 256 ^ w * (m (a + w) % 256) ≤ 256 ^ w * 255 := Nat.mul_le_mul_left _ (by omega)
    omega

theorem readLE_congr (m m' : Mem) (a w : Nat) (h : ∀ k, k < w → m (a + k) = m' (a + k)) :
    readLE m a w = readLE m' a w := by
  induction w with
  | zero => rfl
  | succ w ih =>
    simp only [readLE]
    rw [ih (fun k hk => h k (by omega)), h w (by omega)]

theorem readLE_digits (v a w : Nat) (m : Mem)
    (h : ∀ k, k < w → m (a + k) = v / 256 ^ k % 256) : readLE m a w = v % 256 ^ w := by
  induction w with
  | zero => simp [readLE, Nat.mod_one]
  | succ w ih =>
    simp only [readLE]
    rw [ih (fun k hk => h k (by omega)), h w (by omega), Nat.mod_mod, Nat.mod_pow_succ]

theorem pow256 (w : Nat) : (256 : Nat) ^ w = 2 ^ (w * 8) := by
  rw [Nat.mul_comm, Nat.pow_mul]

-- rule #getRange(#setRange(_BM, ADDR, VAL, WIDTH), ADDR', WIDTH') => #wrap(WIDTH', VAL)
--   requires 0 <=Int ADDR andBool 0 <Int WIDTH andBool 0 <=Int VAL
--    andBool VAL <Int 2 ^Int (8 *Int WIDTH) andBool ADDR' ==Int ADDR andBool WIDTH' <=Int WIDTH
theorem getRange_setRange_same (m : Mem) (A V W W' : Int) (hA : 0 ≤ A) (hW : 0 < W)
    (hV : 0 ≤ V) (hW' : W' ≤ W) :
    getRange (setRange m A V W) A W' = wrap W' V := by
  unfold getRange wrap
  by_cases hp : 0 < W'
  · simp only [hp, hA, and_self, if_true]
    rw [full_mask_to_modulus, Int.sub_add_cancel]
    have hrd : readLE (setRange m A V W) A.toNat W'.toNat = V.toNat % 256 ^ W'.toNat := by
      apply readLE_digits
      intro k hk
      simp only [setRange, hW, hV, hA, and_self, if_true]
      have : A.toNat ≤ A.toNat + k ∧ A.toNat + k < A.toNat + W.toNat := by omega
      simp only [this, and_self, if_true, Nat.add_sub_cancel_left]
    rw [hrd, pow256]
    have hv : (V.toNat : Int) = V := Int.toNat_of_nonneg hV
    rw [Int.natCast_emod, hv]
  · simp [hp]

-- [getRange-before-setRange] #getRange(#setRange(BM, ADDR, _VAL, WIDTH), READ_ADDR, READ_WIDTH)
--   => #getRange(BM, READ_ADDR, READ_WIDTH)
--   requires 0 <=Int ADDR andBool 0 <=Int WIDTH andBool 0 <=Int READ_ADDR
--    andBool 0 <=Int READ_WIDTH andBool READ_ADDR +Int READ_WIDTH <=Int ADDR
theorem getRange_before_setRange (m : Mem) (A V W RA RW : Int) (hA : 0 ≤ A) (hRA : 0 ≤ RA)
    (_hRW : 0 ≤ RW) (hd : RA + RW ≤ A) :
    getRange (setRange m A V W) RA RW = getRange m RA RW := by
  unfold getRange
  by_cases hp : 0 < RW ∧ 0 ≤ RA
  · simp only [hp, and_self, if_true]
    congr 1
    apply readLE_congr
    intro k hk
    unfold setRange
    split
    · have : ¬ (A.toNat ≤ RA.toNat + k ∧ RA.toNat + k < A.toNat + W.toNat) := by omega
      exact if_neg this
    · rfl
  · simp [hp]

-- [getRange-after-setRange] ... requires ... andBool ADDR +Int WIDTH <=Int READ_ADDR
theorem getRange_after_setRange (m : Mem) (A V W RA RW : Int) (hA : 0 ≤ A) (_hW : 0 ≤ W)
    (hRA : 0 ≤ RA) (hd : A + W ≤ RA) :
    getRange (setRange m A V W) RA RW = getRange m RA RW := by
  unfold getRange
  by_cases hp : 0 < RW ∧ 0 ≤ RA
  · simp only [hp, and_self, if_true]
    congr 1
    apply readLE_congr
    intro k hk
    unfold setRange
    split
    · have : ¬ (A.toNat ≤ RA.toNat + k ∧ RA.toNat + k < A.toNat + W.toNat) := by omega
      exact if_neg this
    · rfl
  · simp [hp]

-- rule #getRange(SB, ADDR, 8) &Int 18446744073709551615 => #getRange(SB, ADDR, 8)
theorem getRange8_mask (m : Mem) (A : Int) :
    land (getRange m A 8) 18446744073709551615 = getRange m A 8 := by
  have hmask : (18446744073709551615 : Int) = P 64 - 1 := by decide
  rw [hmask, full_mask_to_modulus, Int.sub_add_cancel]
  unfold getRange
  split
  · apply Int.emod_eq_of_lt (by omega)
    have := readLE_lt m A.toNat (8 : Int).toNat
    have h8 : ((256 : Nat) ^ (8 : Int).toNat : Nat) = 2 ^ 64 := by decide
    simp only [P]
    omega
  · simp

#print axioms lt_minWidth_iff
#print axioms minWidth_recursion
#print axioms reverse_min_width_comparison
#print axioms bounded_min_width_nonpositive
#print axioms bounded_min_width_above
#print axioms bounded_min_width_threshold
#print axioms boolean_word_value
#print axioms bitwise_mk_hostval_then_mask
#print axioms wrap_in_range
#print axioms signed_in_range
#print axioms unsigned_in_range
#print axioms getRange_setRange_same
#print axioms getRange_before_setRange
#print axioms getRange_after_setRange
#print axioms getRange8_mask

end HostObjectLemmas
