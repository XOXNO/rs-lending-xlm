import ByteWrap

/- Literal utilization simplifications only. Every signed integer is accepted
by the two unary rules. The two AND range rules have explicit operand guards.
The bitwise definition is independent of the arithmetic conclusions. -/
namespace UtilizationAnd

abbrev W : Int := 4294967296
abbrev Mask : Nat := 4294967295
abbrev M : Int := 18446744073709551616

/-- Infinite two's-complement AND, by sign constructors and natural bits. -/
def signedAnd : Int → Int → Int
  | .ofNat a, .ofNat b => .ofNat (a &&& b)
  | .negSucc a, .ofNat b => .ofNat (Nat.bitwise (fun x y => !x && y) a b)
  | .ofNat a, .negSucc b => .ofNat (Nat.bitwise (fun x y => x && !y) a b)
  | .negSucc a, .negSucc b => .negSucc (a ||| b)

theorem signedAnd_bits (a b : Int) (i : Nat) :
    ByteWrap.signedBit (signedAnd a b) i =
      (ByteWrap.signedBit a i && ByteWrap.signedBit b i) := by
  cases a with
  | ofNat a =>
    cases b with
    | ofNat b => exact Nat.testBit_and a b i
    | negSucc b =>
      exact Nat.testBit_bitwise (f := fun x y => x && !y) (by rfl) a b i
  | negSucc a =>
    cases b with
    | ofNat b =>
      exact Nat.testBit_bitwise (f := fun x y => !x && y) (by rfl) a b i
    | negSucc b =>
      simp only [signedAnd, ByteWrap.signedBit, Nat.testBit_or]
      cases Nat.testBit a i <;> cases Nat.testBit b i <;> rfl

theorem signedAnd_natMask (x : Int) (mask : Nat) :
    signedAnd x (.ofNat mask) = ByteWrap.andNatMask x mask := by
  cases x <;> rfl

theorem exact_constants : Mask = 2 ^ 32 - 1 ∧ W = (2 ^ 32 : Nat) ∧
    M = (2 ^ 64 : Nat) ∧ W ≠ 0 := by decide

theorem low32_mask (x : Int) : signedAnd x Mask = x % W := by
  change signedAnd x (Int.ofNat Mask) = x % W
  rw [signedAnd_natMask, exact_constants.1]
  exact ByteWrap.and_power_mask_eq_mod x 32

theorem mask32_literal (x : Int) : signedAnd x 4294967295 = x % 4294967296 :=
  low32_mask x

theorem right32_exact_tdiv (x : Int) :
    (x >>> (32 : Nat)) = (x - x % W).tdiv W := by
  rw [Int.shiftRight_eq_div_pow]
  change x / W = (x - x % W).tdiv W
  symm
  apply Int.tdiv_eq_of_eq_mul_right exact_constants.2.2.2
  have := Int.ediv_add_emod x W
  omega

theorem right32_literal (x : Int) :
    (x >>> (32 : Nat)) = (x - x % 4294967296).tdiv 4294967296 :=
  right32_exact_tdiv x

theorem and_nonnegative (a b : Int) (ha : 0 ≤ a) (hb : 0 ≤ b) :
    0 ≤ signedAnd a b := by
  cases a with
  | negSucc a => omega
  | ofNat a =>
    cases b with
    | negSucc b => omega
    | ofNat b => exact Int.natCast_nonneg _

theorem and_le_left (a b : Int) (ha : 0 ≤ a) (hb : 0 ≤ b) :
    signedAnd a b ≤ a := by
  cases a with
  | negSucc a => omega
  | ofNat a =>
    cases b with
    | negSucc b => omega
    | ofNat b =>
      change ((a &&& b : Nat) : Int) ≤ (a : Int)
      exact_mod_cast (Nat.and_le_left (n := a) (m := b))

theorem lower_range (a b : Int) (ha : 0 ≤ a) (hb : 0 ≤ b) :
    decide (0 ≤ signedAnd a b) = (decide (0 ≤ a) && decide (0 ≤ b)) := by
  have hab := and_nonnegative a b ha hb
  simp [ha, hb, hab]

theorem upper_range (a b : Int)
    (ha : 0 ≤ a) (halt : a < M) (hb : 0 ≤ b) (hblt : b < M) :
    decide (signedAnd a b < M) = (decide (a < M) && decide (b < M)) := by
  have hab : signedAnd a b < M := Int.lt_of_le_of_lt (and_le_left a b ha hb) halt
  simp [halt, hblt, hab]

/-- Generic strict operators: absent arguments stay absent. -/
def strict1 {α β : Type} (f : α → β) (a : Option α) : Option β := a.map f
def strict2 {α β γ : Type} (f : α → β → γ) (a : Option α) (b : Option β) : Option γ :=
  a.bind fun x => b.map (f x)
def partialEmod (a b : Option Int) : Option Int :=
  a.bind fun x => b.bind fun y => if y = 0 then none else some (x % y)
def partialTdiv (a b : Option Int) : Option Int :=
  a.bind fun x => b.bind fun y => if y = 0 then none else some (x.tdiv y)

def maskLeft (x : Option Int) := strict2 signedAnd x (some Mask)
def maskRight (x : Option Int) := partialEmod x (some W)
def shiftLeft (x : Option Int) := strict1 (fun (v : Int) => v >>> (32 : Nat)) x
def shiftRight (x : Option Int) :=
  partialTdiv (strict2 (· - ·) x (partialEmod x (some W))) (some W)
def nonnegative (a : Option Int) := strict1 (fun v => decide (0 ≤ v)) a
def belowM (a : Option Int) := strict1 (fun v => decide (v < M)) a
def lowerLeft (a b : Option Int) := nonnegative (strict2 signedAnd a b)
def lowerRight (a b : Option Int) := strict2 (· && ·) (nonnegative a) (nonnegative b)
def upperLeft (a b : Option Int) := belowM (strict2 signedAnd a b)
def upperRight (a b : Option Int) := strict2 (· && ·) (belowM a) (belowM b)

theorem strict_mask (x : Option Int) : maskLeft x = maskRight x := by
  cases x with
  | none => rfl
  | some x => exact congrArg some (low32_mask x)

theorem strict_shift (x : Option Int) : shiftLeft x = shiftRight x := by
  cases x with
  | none => rfl
  | some x => exact congrArg some (right32_exact_tdiv x)

theorem strict_lower (a b : Option Int)
    (ha : ∀ v, a = some v → 0 ≤ v) (hb : ∀ v, b = some v → 0 ≤ v) :
    lowerLeft a b = lowerRight a b := by
  cases a with
  | none => rfl
  | some a =>
    cases b with
    | none => rfl
    | some b => exact congrArg some (lower_range a b (ha a rfl) (hb b rfl))

theorem strict_upper (a b : Option Int)
    (ha : ∀ v, a = some v → 0 ≤ v ∧ v < M)
    (hb : ∀ v, b = some v → 0 ≤ v ∧ v < M) :
    upperLeft a b = upperRight a b := by
  cases a with
  | none => rfl
  | some a =>
    cases b with
    | none => rfl
    | some b =>
      obtain ⟨ha0, halt⟩ := ha a rfl
      obtain ⟨hb0, hblt⟩ := hb b rfl
      exact congrArg some (upper_range a b ha0 halt hb0 hblt)

theorem mask_definedness (x : Option Int) :
    (maskLeft x ≠ none ↔ x ≠ none) ∧ (maskRight x ≠ none ↔ x ≠ none) := by
  cases x <;> simp [maskLeft, maskRight, strict2, partialEmod, W]

theorem shift_definedness (x : Option Int) :
    (shiftLeft x ≠ none ↔ x ≠ none) ∧ (shiftRight x ≠ none ↔ x ≠ none) := by
  cases x <;> simp [shiftLeft, shiftRight, strict1, strict2, partialEmod, partialTdiv, W]

theorem lower_definedness (a b : Option Int) :
    (lowerLeft a b ≠ none ↔ a ≠ none ∧ b ≠ none) ∧
    (lowerRight a b ≠ none ↔ a ≠ none ∧ b ≠ none) := by
  cases a <;> cases b <;> simp [lowerLeft, lowerRight, strict1, strict2, nonnegative]

theorem upper_definedness (a b : Option Int) :
    (upperLeft a b ≠ none ↔ a ≠ none ∧ b ≠ none) ∧
    (upperRight a b ≠ none ↔ a ≠ none ∧ b ≠ none) := by
  cases a <;> cases b <;> simp [upperLeft, upperRight, strict1, strict2, belowM]

theorem unary_absence (x : Option Int) :
    (maskLeft x = none ↔ x = none) ∧ (maskRight x = none ↔ x = none) ∧
    (shiftLeft x = none ↔ x = none) ∧ (shiftRight x = none ↔ x = none) := by
  cases x <;> simp [maskLeft, maskRight, shiftLeft, shiftRight,
    strict1, strict2, partialEmod, partialTdiv, W]

theorem binary_absence (a b : Option Int) :
    (lowerLeft a b = none ↔ a = none ∨ b = none) ∧
    (lowerRight a b = none ↔ a = none ∨ b = none) ∧
    (upperLeft a b = none ↔ a = none ∨ b = none) ∧
    (upperRight a b = none ↔ a = none ∨ b = none) := by
  cases a <;> cases b <;> simp [lowerLeft, lowerRight, upperLeft, upperRight,
    strict1, strict2, nonnegative, belowM]

/-- Concrete LLVM short-circuits BOOL.and. Under the guards this agrees. -/
def shortAnd (a b : Option Bool) : Option Bool :=
  a.bind fun v => if v then b else some false

theorem guarded_lower_shortAnd (a b : Option Int)
    (ha : ∀ v, a = some v → 0 ≤ v) (hb : ∀ v, b = some v → 0 ≤ v) :
    lowerLeft a b = shortAnd (nonnegative a) (nonnegative b) := by
  rw [strict_lower a b ha hb]
  cases a with
  | none => rfl
  | some a =>
    have ha0 := ha a rfl
    cases b <;> simp [lowerRight, nonnegative, strict1, strict2, shortAnd, ha0]

theorem guarded_upper_shortAnd (a b : Option Int)
    (ha : ∀ v, a = some v → 0 ≤ v ∧ v < M)
    (hb : ∀ v, b = some v → 0 ≤ v ∧ v < M) :
    upperLeft a b = shortAnd (belowM a) (belowM b) := by
  rw [strict_upper a b ha hb]
  cases a with
  | none => rfl
  | some a =>
    have halt := (ha a rfl).2
    cases b <;> simp [upperRight, belowM, strict1, strict2, shortAnd, halt]

-- Removing the range guards is false, even on total operands.
theorem lower_guard_needed :
    decide (0 ≤ signedAnd 0 (-1)) ≠ (decide (0 ≤ (0 : Int)) && decide (0 ≤ (-1 : Int))) := by
  decide

theorem upper_guard_needed :
    decide (signedAnd 0 M < M) ≠ (decide ((0 : Int) < M) && decide (M < M)) := by
  decide

-- Dropping X's remainder is false for negative inputs, even at literal 32.
theorem truncating_division_wrong : ((-1 : Int) >>> (32 : Nat)) ≠ (-1 : Int).tdiv W := by
  decide

end UtilizationAnd

#print axioms UtilizationAnd.signedAnd_bits
#print axioms UtilizationAnd.low32_mask
#print axioms UtilizationAnd.mask32_literal
#print axioms UtilizationAnd.right32_exact_tdiv
#print axioms UtilizationAnd.right32_literal
#print axioms UtilizationAnd.lower_range
#print axioms UtilizationAnd.upper_range
#print axioms UtilizationAnd.strict_mask
#print axioms UtilizationAnd.strict_shift
#print axioms UtilizationAnd.strict_lower
#print axioms UtilizationAnd.strict_upper
#print axioms UtilizationAnd.mask_definedness
#print axioms UtilizationAnd.shift_definedness
#print axioms UtilizationAnd.lower_definedness
#print axioms UtilizationAnd.upper_definedness
#print axioms UtilizationAnd.unary_absence
#print axioms UtilizationAnd.binary_absence
#print axioms UtilizationAnd.guarded_lower_shortAnd
#print axioms UtilizationAnd.guarded_upper_shortAnd
#print axioms UtilizationAnd.lower_guard_needed
#print axioms UtilizationAnd.upper_guard_needed
#print axioms UtilizationAnd.truncating_division_wrong
