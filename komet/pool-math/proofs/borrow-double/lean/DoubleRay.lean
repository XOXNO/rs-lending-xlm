import BorrowIndex

namespace DoubleRay

theorem half_up (p : Int) (hp : 0 ≤ p) :
    (p * 2000000000000000000000000000 + 500000000000000000000000000).tdiv
      1000000000000000000000000000 = p * 2 := by
  have cast_p : (p.toNat : Int) = p := Int.toNat_of_nonneg hp
  calc
    _ = ((p.toNat : Int) * ((2 * BorrowIndex.R : Nat) : Int) +
          (BorrowIndex.R : Int).tdiv 2).tdiv (BorrowIndex.R : Int) := by
      rw [cast_p]
      rfl
    _ = (BorrowIndex.raw p.toNat (2 * BorrowIndex.R) : Int) :=
      BorrowIndex.signed_truncation_bridge p.toNat (2 * BorrowIndex.R)
    _ = ((2 * p.toNat : Nat) : Int) :=
      congrArg (fun n : Nat => (n : Int)) (BorrowIndex.doubling_raw p.toNat)
    _ = p * 2 := by
      rw [Int.natCast_mul, cast_p]
      exact Int.mul_comm 2 p

-- Strict partial operands: None stays None; Some requires a nonnegative value.
-- This is Option lifting, not verification of K hooks, a backend, or Wasm.
theorem strict_lifting (p : Option Int)
    (hp : ∀ x, p = some x → 0 ≤ x) :
    let lhs := p.map (fun x =>
      (x * 2000000000000000000000000000 + 500000000000000000000000000).tdiv
        1000000000000000000000000000)
    let rhs := p.map (fun x => x * 2)
    0 < (1000000000000000000000000000 : Int) ∧
      lhs = rhs ∧ lhs.isSome = p.isSome ∧ rhs.isSome = p.isSome := by
  constructor
  · decide
  · cases p with
    | none => exact ⟨rfl, rfl, rfl⟩
    | some x => exact ⟨congrArg some (half_up x (hp x rfl)), rfl, rfl⟩

#print axioms half_up
#print axioms strict_lifting

end DoubleRay
