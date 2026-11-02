import Init.Data.Int.Bitwise.Lemmas
import Init.Data.Nat.Bitwise.Lemmas
import Init.Omega

/-!
Model of K's arbitrary-precision integer bit operations (`&Int`, `|Int`,
`xorInt`, `<<Int`, `>>Int`), which K implements with GMP on infinite two's
complement integers. Lean 4.22 core has no signed `land`/`lor`/`xor` on `Int`,
so they are defined here by sign constructors (`negSucc a` is the complement
of `a`) and characterised bit by bit. Euclidean `x % m` models `modInt`, `x / m`
(`Int.ediv`) models `divInt`, `Int.tdiv` models `/Int`.
-/
namespace KBits

def bit : Int → Nat → Bool
  | .ofNat a, i => a.testBit i
  | .negSucc a, i => !a.testBit i

def land : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a &&& b)
  | .ofNat a, .negSucc b => .ofNat (Nat.bitwise (fun x y => x && !y) a b)
  | .negSucc a, .ofNat b => .ofNat (Nat.bitwise (fun x y => !x && y) a b)
  | .negSucc a, .negSucc b => .negSucc (a ||| b)

def lor : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a ||| b)
  | .negSucc a, .ofNat b => .negSucc (Nat.bitwise (fun x y => x && !y) a b)
  | .ofNat a, .negSucc b => .negSucc (Nat.bitwise (fun x y => x && !y) b a)
  | .negSucc a, .negSucc b => .negSucc (a &&& b)

def xor : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a ^^^ b)
  | .negSucc a, .ofNat b => .negSucc (a ^^^ b)
  | .ofNat a, .negSucc b => .negSucc (a ^^^ b)
  | .negSucc a, .negSucc b => .ofNat (a ^^^ b)

theorem bit_natCast (n i : Nat) : bit (n : Int) i = n.testBit i := rfl

theorem bit_zero (i : Nat) : bit 0 i = false := Nat.zero_testBit i

theorem testBit_andNot (x y i : Nat) :
    (Nat.bitwise (fun x y => x && !y) x y).testBit i = (x.testBit i && !y.testBit i) :=
  Nat.testBit_bitwise (f := fun x y => x && !y) rfl x y i

theorem testBit_notAnd (x y i : Nat) :
    (Nat.bitwise (fun x y => !x && y) x y).testBit i = (!x.testBit i && y.testBit i) :=
  Nat.testBit_bitwise (f := fun x y => !x && y) rfl x y i

theorem bit_land (a b : Int) (i : Nat) : bit (land a b) i = (bit a i && bit b i) := by
  cases a <;> cases b <;>
    simp only [land, bit, Nat.testBit_and, Nat.testBit_or, testBit_andNot, testBit_notAnd] <;>
    rename_i x y <;> cases x.testBit i <;> cases y.testBit i <;> rfl

theorem bit_lor (a b : Int) (i : Nat) : bit (lor a b) i = (bit a i || bit b i) := by
  cases a <;> cases b <;>
    simp only [lor, bit, Nat.testBit_and, Nat.testBit_or, testBit_andNot] <;>
    rename_i x y <;> cases x.testBit i <;> cases y.testBit i <;> rfl

theorem bit_xor (a b : Int) (i : Nat) : bit (xor a b) i = (bit a i ^^ bit b i) := by
  cases a <;> cases b <;>
    simp only [xor, bit, Nat.testBit_xor] <;>
    rename_i x y <;> cases x.testBit i <;> cases y.testBit i <;> rfl

theorem testBit_large (a b : Nat) : a.testBit (a + b) = false :=
  Nat.testBit_lt_two_pow (Nat.lt_of_lt_of_le Nat.lt_two_pow_self
    (Nat.pow_le_pow_right (by decide) (Nat.le_add_right a b)))

/-- Extensionality: two integers with the same bits are equal. -/
theorem ext {a b : Int} (h : ∀ i, bit a i = bit b i) : a = b := by
  cases a with
  | ofNat a =>
    cases b with
    | ofNat b => exact congrArg Int.ofNat (Nat.eq_of_testBit_eq h)
    | negSucc b =>
      have := h (a + b)
      simp only [bit, testBit_large, Nat.add_comm a b] at this
      rw [Nat.add_comm b a, testBit_large] at this
      contradiction
  | negSucc a =>
    cases b with
    | ofNat b =>
      have := h (a + b)
      simp only [bit, testBit_large] at this
      rw [Nat.add_comm a b, testBit_large] at this
      contradiction
    | negSucc b =>
      have : a = b := Nat.eq_of_testBit_eq fun i => by
        have := h i
        simp only [bit] at this
        cases ha : a.testBit i <;> cases hb : b.testBit i <;> simp_all
      rw [this]

/-! Bits of Euclidean remainders, products and quotients by powers of two. -/

theorem bit_emod_two_pow (x : Int) (k i : Nat) :
    bit (x % ((2 ^ k : Nat) : Int)) i = (decide (i < k) && bit x i) := by
  cases x with
  | ofNat a =>
    rw [Int.ofNat_eq_coe, ← Int.natCast_emod, bit_natCast, Nat.testBit_mod_two_pow]
    rfl
  | negSucc a =>
    have hpos : (0 : Int) < ((2 ^ k : Nat) : Int) := by
      have := Nat.two_pow_pos k
      omega
    have hlt : a % 2 ^ k < 2 ^ k := Nat.mod_lt a (Nat.two_pow_pos k)
    have heq : Int.negSucc a % ((2 ^ k : Nat) : Int) =
        ((2 ^ k - (a % 2 ^ k + 1) : Nat) : Int) := by
      rw [Int.negSucc_emod a hpos, ← Int.natCast_emod]
      omega
    rw [heq, bit_natCast, Nat.testBit_two_pow_sub_succ hlt, Nat.testBit_mod_two_pow]
    simp only [bit]
    cases decide (i < k) <;> simp

theorem bit_mul_two_pow (x : Int) (k i : Nat) :
    bit (x * ((2 ^ k : Nat) : Int)) i = (decide (k ≤ i) && bit x (i - k)) := by
  cases x with
  | ofNat a =>
    rw [Int.ofNat_eq_coe, ← Int.natCast_mul, bit_natCast, Nat.testBit_mul_two_pow]
    rfl
  | negSucc a =>
    have hp : 0 < 2 ^ k := Nat.two_pow_pos k
    have hmul : Int.negSucc a * ((2 ^ k : Nat) : Int) =
        Int.negSucc (2 ^ k * a + (2 ^ k - 1)) := by
      rw [Int.negSucc_eq, Int.negSucc_eq, Int.neg_mul, Int.natCast_add,
        Int.natCast_mul, Int.natCast_sub (by omega : 1 ≤ 2 ^ k)]
      simp only [Int.natCast_one]
      rw [Int.add_mul, Int.one_mul, Int.mul_comm (a : Int)]
      omega
    rw [hmul, Nat.two_pow_add_eq_or_of_lt (by omega : 2 ^ k - 1 < 2 ^ k)]
    simp only [bit, Nat.testBit_or, Nat.testBit_two_pow_sub_one]
    rw [Nat.mul_comm, Nat.testBit_mul_two_pow]
    by_cases h : k ≤ i
    · have : ¬ i < k := by omega
      simp [h, this]
    · have : i < k := by omega
      simp [h, this]

theorem bit_ediv_two_pow (x : Int) (k j : Nat) :
    bit (x / ((2 ^ k : Nat) : Int)) j = bit x (k + j) := by
  rw [← Int.shiftRight_eq_div_pow]
  cases x with
  | ofNat a =>
    show bit ((a >>> k : Nat) : Int) j = a.testBit (k + j)
    rw [bit_natCast, Nat.testBit_shiftRight]
  | negSucc a =>
    rw [Int.negSucc_shiftRight]
    simp only [bit, Nat.testBit_shiftRight]

/-- A remainder modulo `2^k` is a natural number below `2^k`. -/
theorem emod_two_pow_nat (x : Int) (k : Nat) :
    ∃ n : Nat, x % ((2 ^ k : Nat) : Int) = (n : Int) ∧ n < 2 ^ k := by
  have hpos : (0 : Int) < ((2 ^ k : Nat) : Int) := by
    have := Nat.two_pow_pos k
    omega
  have h0 := Int.emod_nonneg x (Int.ne_of_gt hpos)
  have h1 := Int.emod_lt_of_pos x hpos
  refine ⟨(x % ((2 ^ k : Nat) : Int)).toNat, (Int.toNat_of_nonneg h0).symm, ?_⟩
  omega

/-- For `n < 2^(j+1)`, `n < 2^j` exactly when bit `j` is clear. -/
theorem nat_lt_half_iff (n j : Nat) (h : n < 2 ^ (j + 1)) :
    n < 2 ^ j ↔ n.testBit j = false := by
  rw [Nat.testBit_eq_decide_div_mod_eq]
  have hp : 0 < 2 ^ j := Nat.two_pow_pos j
  rw [Nat.pow_succ] at h
  by_cases hl : n < 2 ^ j
  · rw [Nat.div_eq_of_lt hl]
    simp [hl]
  · have : n / 2 ^ j = 1 := Nat.div_eq_of_lt_le (by omega) (by omega)
    rw [this]
    simp [hl]

/-- Sign test of the low `j+1` bits: below `2^j` iff bit `j` is clear. -/
theorem emod_lt_half_iff (x : Int) (j : Nat) :
    x % ((2 ^ (j + 1) : Nat) : Int) < ((2 ^ j : Nat) : Int) ↔ bit x j = false := by
  obtain ⟨n, hn, hlt⟩ := emod_two_pow_nat x (j + 1)
  have hb : bit x j = n.testBit j := by
    have := bit_emod_two_pow x (j + 1) j
    rw [hn, bit_natCast] at this
    simp at this
    exact this.symm
  rw [hn, hb, ← nat_lt_half_iff n j hlt]
  omega

/-- A remainder modulo `2^k` is zero iff the low `k` bits are clear. -/
theorem emod_eq_zero_iff (x : Int) (k : Nat) :
    x % ((2 ^ k : Nat) : Int) = 0 ↔ ∀ i, i < k → bit x i = false := by
  constructor
  · intro h i hi
    have := bit_emod_two_pow x k i
    rw [h, bit_zero] at this
    simp [hi] at this
    exact this
  · intro h
    apply ext
    intro i
    rw [bit_emod_two_pow, bit_zero]
    by_cases hi : i < k
    · simp [hi, h i hi]
    · simp [hi]

theorem natCast_two_pow (k : Nat) : (((2 ^ k : Nat) : Int)) = (2 : Int) ^ k := by
  rw [Int.natCast_pow]
  rfl

end KBits
