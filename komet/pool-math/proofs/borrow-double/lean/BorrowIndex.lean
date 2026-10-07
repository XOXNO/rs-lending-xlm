import Lean.Elab.Tactic.Omega

/-
Production mapping at rs-lending-xlm commit
a2486b255b974ef8432ce44de014931f51552f4b:
  common/src/rates/index.rs:13-19, update_borrow_index
  common/src/math/fp.rs:50-52, Ray.mul
  common/src/math/fp_core.rs:122-143, try_mul_div_half_up

For nonnegative operands and positive denominator, Rust/Soroban signed
division truncating toward zero equals Nat's floor division. The wide path
computes (old * factor + R / 2) / R before conversion to i128 and clamping.
This is a mathematical model of that expression, not compiled-Wasm refinement.
-/
namespace BorrowIndex

-- Independent total-integer fact; says nothing about partial K expressions.
theorem integer_self_subtraction (i : Int) : i - i = 0 := Int.sub_self i

def R : Nat := 1000000000000000000000000000 -- 10^27
def C : Nat := 1000000000000000000000000000000000000 -- 10^36
def i128Max : Nat := 170141183460469231731687303715884105727
def u128Max : Nat := 340282366920938463463374607431768211455
def i256Max : Nat :=
  57896044618658097711785492504343953926634992332820282019728792003956564819967

def numerator (old factor : Nat) : Nat := old * factor + R / 2
def raw (old factor : Nat) : Nat := numerator old factor / R
def update (old factor : Nat) : Nat :=
  if raw old factor > C then C else raw old factor

theorem constants :
    R = 10^27 ∧ C = 10^36 ∧ i128Max = 2^127 - 1 ∧
    u128Max = 2^128 - 1 ∧ i256Max = 2^255 - 1 := by decide

-- No negative operands are hidden: the Nat casts establish the signed domain.
-- Int.tdiv is truncation toward zero, matching Rust's `/` on signed integers.
theorem signed_truncation_bridge (old factor : Nat) :
    ((old : Int) * (factor : Int) + (R : Int).tdiv 2).tdiv (R : Int) =
      (raw old factor : Int) := by rfl

-- The bias is present in the theorem; exact doubling survives half-up rounding.
theorem doubling_raw (old : Nat) : raw old (2 * R) = 2 * old := by
  unfold raw numerator R
  omega

-- A real half-way case: 1.5 raw units of growth rounds to 2, not 1.
theorem exact_half_rounds_up :
    raw (3 * R / 2) (R + 1) = 3 * R / 2 + 2 := by
  unfold raw numerator R
  decide

theorem doubling_capped (old : Nat) :
    update old (2 * R) = min (2 * old) C := by
  unfold update
  rw [doubling_raw]
  by_cases h : C < 2 * old
  · simp [h, Nat.min_eq_right (Nat.le_of_lt h)]
  · have hle : 2 * old ≤ C := by omega
    simp [h, Nat.min_eq_left hle]

-- All Rust numeric preconditions and both sides of the machine-width boundary.
-- checked_mul fails for EVERY input in this domain, so I256 handles the product.
theorem doubling_machine_bounds (old : Nat) (hlo : R ≤ old) (hhi : old ≤ C) :
    0 < R ∧ old ≤ i128Max ∧ 2 * R ≤ i128Max ∧
    i128Max < old * (2 * R) ∧
    numerator old (2 * R) ≤
      2000000000000000000000000000000000000500000000000000000000000000 ∧
    numerator old (2 * R) ≤ i256Max ∧
    raw old (2 * R) ≤ 2 * C ∧
    raw old (2 * R) ≤ i128Max ∧ raw old (2 * R) ≤ u128Max := by
  rw [doubling_raw]
  unfold numerator R C i128Max u128Max i256Max at *
  omega

theorem doubling_invariant (old : Nat) (hlo : R ≤ old) (hhi : old ≤ C) :
    raw old (2 * R) = 2 * old ∧
    update old (2 * R) = min (2 * old) C ∧
    old ≤ update old (2 * R) ∧
    R ≤ update old (2 * R) ∧ update old (2 * R) ≤ C := by
  rw [doubling_raw, doubling_capped]
  omega

-- Two witnesses ensure the requested domain is inhabited, including cap activation.
theorem domain_and_cap_witnesses :
    R ≤ C ∧ update R (2 * R) = 2 * R ∧ update C (2 * R) = C := by
  rw [doubling_capped, doubling_capped]
  unfold R C
  decide

#print axioms constants
#print axioms integer_self_subtraction
#print axioms signed_truncation_bridge
#print axioms doubling_raw
#print axioms exact_half_rounds_up
#print axioms doubling_capped
#print axioms doubling_machine_bounds
#print axioms doubling_invariant
#print axioms domain_and_cap_witnesses

end BorrowIndex
