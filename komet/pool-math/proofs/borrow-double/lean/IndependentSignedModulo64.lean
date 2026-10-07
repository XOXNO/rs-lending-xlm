import Std

-- Exact mathematical model of cached wasm-data.md:452-453,458-464.
-- None represents an undefined operand or the partial #signed domain failure.
namespace IndependentSignedModulo64

def modulus64 : Int := 18446744073709551616

def half64 : Int := 9223372036854775808

def signedValue (modulus half n : Int) : Int :=
  if n < half then n else n - modulus

def signedPartial (modulus half n : Int) : Option Int :=
  if 0 <= n ∧ n < modulus then some (signedValue modulus half n) else none

theorem modulo_unsigned_domain (modulus x : Int) (positive : 0 < modulus) :
    0 <= x % modulus ∧ x % modulus < modulus :=
  ⟨Int.emod_nonneg x (by omega), Int.emod_lt_of_pos x positive⟩

theorem partial_signed_domain (modulus half n : Int) :
    signedPartial modulus half n = none ↔ ¬ (0 <= n ∧ n < modulus) := by
  by_cases domain : 0 <= n ∧ n < modulus <;> simp [signedPartial, domain]

theorem partial_signed_modulo (modulus half x : Int) (positive : 0 < modulus) :
    signedPartial modulus half (x % modulus) =
      some (signedValue modulus half (x % modulus)) := by
  have domain := modulo_unsigned_domain modulus x positive
  simp [signedPartial, domain]

-- Works for every integer x, including negative and arbitrarily large x.
-- The split point is arbitrary: the unsigned domain alone proves both cases.
theorem modulo_sign_iff (modulus half x : Int) (positive : 0 < modulus) :
    (0 <= signedValue modulus half (x % modulus)) ↔ x % modulus < half := by
  have domain := modulo_unsigned_domain modulus x positive
  by_cases lower : x % modulus < half
  · simp only [signedValue, if_pos lower]
    omega
  · simp only [signedValue, if_neg lower]
    omega

theorem modulo_sign_boolean (modulus half x : Int) (positive : 0 < modulus) :
    decide (0 <= signedValue modulus half (x % modulus)) =
      decide (x % modulus < half) := by
  have equivalence := modulo_sign_iff modulus half x positive
  by_cases lower : x % modulus < half
  · have nonnegative := equivalence.mpr lower
    simp [lower, nonnegative]
  · have negative : ¬ (0 <= signedValue modulus half (x % modulus)) := by
      intro nonnegative
      exact lower (equivalence.mp nonnegative)
    simp [lower, negative]

def before (modulus half : Int) (operand : Option Int) : Option Bool :=
  operand.bind fun x =>
    (signedPartial modulus half (x % modulus)).map fun signed => decide (0 <= signed)

def after (modulus half : Int) (operand : Option Int) : Option Bool :=
  operand.map fun x => decide (x % modulus < half)

-- Strict lifting preserves an undefined operand; no operand is erased.
theorem strict_partial_lifting (modulus half : Int) (positive : 0 < modulus)
    (operand : Option Int) : before modulus half operand = after modulus half operand := by
  cases operand with
  | none => rfl
  | some x =>
      simp [before, after, partial_signed_modulo modulus half x positive,
        modulo_sign_boolean modulus half x positive]

theorem before_definedness (modulus half : Int) (positive : 0 < modulus)
    (operand : Option Int) : before modulus half operand = none ↔ operand = none := by
  rw [strict_partial_lifting modulus half positive operand]
  cases operand <;> simp [after]

theorem after_definedness (modulus half : Int) (operand : Option Int) :
    after modulus half operand = none ↔ operand = none := by
  cases operand <;> simp [after]

theorem signed64_modulo_identity (x : Int) :
    (0 <= signedValue modulus64 half64 (x % modulus64)) ↔ x % modulus64 < half64 :=
  modulo_sign_iff modulus64 half64 x (by decide)

theorem signed64_modulo_strict_lifting (operand : Option Int) :
    before modulus64 half64 operand = after modulus64 half64 operand :=
  strict_partial_lifting modulus64 half64 (by decide) operand

theorem signed64_modulo_unsigned_domain (x : Int) :
    0 <= x % modulus64 ∧ x % modulus64 < modulus64 :=
  modulo_unsigned_domain modulus64 x (by decide)

#print axioms modulo_unsigned_domain
#print axioms partial_signed_domain
#print axioms partial_signed_modulo
#print axioms modulo_sign_iff
#print axioms modulo_sign_boolean
#print axioms strict_partial_lifting
#print axioms before_definedness
#print axioms after_definedness
#print axioms signed64_modulo_identity
#print axioms signed64_modulo_strict_lifting
#print axioms signed64_modulo_unsigned_domain

end IndependentSignedModulo64
