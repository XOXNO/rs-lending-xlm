import Lean.Elab.Tactic.Omega
import Init.Data.Nat.Bitwise.Lemmas

namespace WordOr

theorem or_zero_iff (x y : Nat) : x ||| y = 0 ↔ x = 0 ∧ y = 0 := by
  constructor
  · intro h
    have hb (i : Nat) : x.testBit i = false ∧ y.testBit i = false := by
      have hh := congrArg (fun n : Nat => n.testBit i) h
      simpa only [Nat.testBit_or, Nat.zero_testBit, Bool.or_eq_false_iff] using hh
    exact ⟨Nat.eq_of_testBit_eq (fun i => by simpa using (hb i).1),
      Nat.eq_of_testBit_eq (fun i => by simpa using (hb i).2)⟩
  · rintro ⟨rfl, rfl⟩
    rfl

theorem projected_or_zero_iff (x y : Nat) (hx : x < 2^64) (hy : y < 2^64) :
    (x ||| y) % 2^64 = 0 ↔ x = 0 ∧ y = 0 := by
  rw [Nat.mod_eq_of_lt (Nat.or_lt_two_pow hx hy)]
  exact or_zero_iff x y

-- For nonnegative integer operands, infinite two's-complement OR agrees with
-- this natural-number OR. This does not establish the K hook implementation.
def nonnegativeOr (x y : Int) : Int := Int.ofNat (x.toNat ||| y.toNat)

theorem integer_projected_or_zero_iff (x y : Int)
    (hx : 0 ≤ x) (hxm : x < 18446744073709551616)
    (hy : 0 ≤ y) (hym : y < 18446744073709551616) :
    (nonnegativeOr x y) % 18446744073709551616 = 0 ↔ x = 0 ∧ y = 0 := by
  have hxn : x.toNat < 2^64 := by omega
  have hyn : y.toNat < 2^64 := by omega
  have h := projected_or_zero_iff x.toNat y.toNat hxn hyn
  unfold nonnegativeOr
  have hx' : x = Int.ofNat x.toNat := (Int.toNat_of_nonneg hx).symm
  have hy' : y = Int.ofNat y.toNat := (Int.toNat_of_nonneg hy).symm
  rw [hx', hy']
  change ((↑(x.toNat ||| y.toNat) : Int) % ↑(2^64 : Nat) = ↑(0 : Nat)) ↔
    (↑x.toNat : Int) = ↑(0 : Nat) ∧ (↑y.toNat : Int) = ↑(0 : Nat)
  rw [← Int.natCast_emod]
  simpa only [Int.natCast_inj] using h

#print axioms or_zero_iff
#print axioms projected_or_zero_iff
#print axioms integer_projected_or_zero_iff
end WordOr
