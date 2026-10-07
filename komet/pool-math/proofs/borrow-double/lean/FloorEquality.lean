import Std

namespace FloorEquality

-- K /Int truncates toward zero; modInt is Euclidean. No range/sign assumptions.
def word64 (x : Int) : Int :=
  (x - x % 18446744073709551616).tdiv 18446744073709551616

theorem word64_eq_iff (x n : Int) :
    word64 x = n ↔
      n * 18446744073709551616 ≤ x ∧
      x < (n + 1) * 18446744073709551616 := by
  have hq : word64 x * 18446744073709551616 =
      x - x % 18446744073709551616 :=
    Int.tdiv_mul_cancel Int.dvd_self_sub_emod
  have hr0 : 0 ≤ x % 18446744073709551616 :=
    Int.emod_nonneg x (by decide)
  have hrB : x % 18446744073709551616 < 18446744073709551616 :=
    Int.emod_lt_of_pos x (by decide)
  omega

def wordEq (x n : Int) : Bool := decide (word64 x = n)
def interval (x n : Int) : Bool :=
  decide (n * 18446744073709551616 ≤ x) &&
  decide (x < (n + 1) * 18446744073709551616)

theorem bool_eq (x n : Int) : wordEq x n = interval x n := by
  simp [wordEq, interval, word64_eq_iff]

-- Every arithmetic/comparison operand is lifted strictly; either missing
-- operand makes both sides undefined. Fixed divisors are nonzero.
def strictInt (f : Int → Int → Int) (a b : Option Int) : Option Int := do
  let a ← a
  let b ← b
  pure (f a b)

def strictBool (f : Bool → Bool → Bool) (a b : Option Bool) : Option Bool := do
  let a ← a
  let b ← b
  pure (f a b)

def compare (f : Int → Int → Bool) (a b : Option Int) : Option Bool := do
  let a ← a
  let b ← b
  pure (f a b)

def tdiv (a b : Option Int) : Option Int := do
  let a ← a
  let b ← b
  if b = 0 then none else some (a.tdiv b)

def emod (a b : Option Int) : Option Int := do
  let a ← a
  let b ← b
  if b = 0 then none else some (a % b)

def wordEqOption (x n : Option Int) : Option Bool :=
  compare (fun a b => decide (a = b))
    (tdiv
      (strictInt (· - ·) x (emod x (some 18446744073709551616)))
      (some 18446744073709551616)) n

def intervalOption (x n : Option Int) : Option Bool :=
  strictBool (· && ·)
    (compare (fun a b => decide (a ≤ b))
      (strictInt (· * ·) n (some 18446744073709551616)) x)
    (compare (fun a b => decide (a < b)) x
      (strictInt (· * ·) (strictInt (· + ·) n (some 1))
        (some 18446744073709551616)))

theorem option_eq (x n : Option Int) : wordEqOption x n = intervalOption x n := by
  cases x with
  | none => cases n <;> rfl
  | some a =>
    cases n with
    | none => rfl
    | some b =>
      simp only [wordEqOption, intervalOption, compare, strictInt, strictBool,
        tdiv, emod]
      exact congrArg some (bool_eq a b)

def Defined (x : Option Bool) : Prop := ∃ b : Bool, x = some b

theorem defined_iff (x n : Option Int) :
    Defined (wordEqOption x n) ↔ Defined (intervalOption x n) := by
  rw [option_eq]

theorem undefined_iff (x n : Option Int) :
    wordEqOption x n = none ↔ intervalOption x n = none := by
  rw [option_eq]

end FloorEquality

#print axioms FloorEquality.word64_eq_iff
#print axioms FloorEquality.bool_eq
#print axioms FloorEquality.option_eq
#print axioms FloorEquality.defined_iff
#print axioms FloorEquality.undefined_iff
