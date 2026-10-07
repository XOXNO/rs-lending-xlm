import Std

namespace SignedDecoder

-- K /Int is truncation toward zero (Int.tdiv); modInt is Euclidean (Int.emod).
-- This proves integer arithmetic and strict Option lifting, not Wasm refinement.
theorem fixed_divisor_nonzero : (18446744073709551616 : Int) ≠ 0 := by
  decide

theorem shifted_remainder (x : Int) :
    (x + 340282366920938463463374607431768211456) % 18446744073709551616 =
      x % 18446744073709551616 := by
  have h : (340282366920938463463374607431768211456 : Int) %
      18446744073709551616 = 0 := by decide
  rw [Int.add_emod, h, Int.add_zero, Int.emod_emod]

-- Exact identity, universally quantified over Int, including negative inputs.
theorem signed_decoder_int (x : Int) :
    ((x - x % 18446744073709551616).tdiv 18446744073709551616) *
        18446744073709551616 +
      ((if x < 0 then x + 340282366920938463463374607431768211456 else x) %
        18446744073709551616) = x := by
  have h : ((x - x % 18446744073709551616).tdiv 18446744073709551616) *
      18446744073709551616 = x - x % 18446744073709551616 :=
    Int.tdiv_mul_cancel Int.dvd_self_sub_emod
  rw [h]
  by_cases hx : x < 0
  · rw [if_pos hx, shifted_remainder, Int.sub_add_cancel]
  · rw [if_neg hx, Int.sub_add_cancel]

-- Strict operands; division/modulo additionally fail when the divisor is zero.
def strictBinary (f : Int → Int → Int) (a b : Option Int) : Option Int := do
  let a ← a
  let b ← b
  pure (f a b)

def strictTdiv (a b : Option Int) : Option Int := do
  let a ← a
  let b ← b
  if b = 0 then none else some (a.tdiv b)

def strictEmod (a b : Option Int) : Option Int := do
  let a ← a
  let b ← b
  if b = 0 then none else some (a % b)

-- Same expression with every arithmetic operator lifted strictly.
-- The condition and selected branch are strict in the sole input via Option.map.
def signed_decoder_option (x : Option Int) : Option Int :=
  strictBinary (· + ·)
    (strictBinary (· * ·)
      (strictTdiv
        (strictBinary (· - ·) x (strictEmod x (some 18446744073709551616)))
        (some 18446744073709551616))
      (some 18446744073709551616))
    (strictEmod
      (x.map (fun v => if v < 0 then v + 340282366920938463463374607431768211456 else v))
      (some 18446744073709551616))

theorem signed_decoder_some (x : Int) : signed_decoder_option (some x) = some x := by
  simp only [signed_decoder_option, strictBinary, strictTdiv, strictEmod,
    Option.map_some]
  exact congrArg some (signed_decoder_int x)

theorem signed_decoder_none : signed_decoder_option none = none := by
  rfl

theorem signed_decoder_option_eq (x : Option Int) : signed_decoder_option x = x := by
  cases x with
  | none => exact signed_decoder_none
  | some v => exact signed_decoder_some v

def Defined (x : Option Int) : Prop := ∃ v : Int, x = some v

theorem signed_decoder_defined_iff (x : Option Int) :
    Defined (signed_decoder_option x) ↔ Defined x := by
  rw [signed_decoder_option_eq]

theorem signed_decoder_undefined_iff (x : Option Int) :
    signed_decoder_option x = none ↔ x = none := by
  rw [signed_decoder_option_eq]

#print axioms fixed_divisor_nonzero
#print axioms shifted_remainder
#print axioms signed_decoder_int
#print axioms signed_decoder_some
#print axioms signed_decoder_none
#print axioms signed_decoder_option_eq
#print axioms signed_decoder_defined_iff
#print axioms signed_decoder_undefined_iff

end SignedDecoder
