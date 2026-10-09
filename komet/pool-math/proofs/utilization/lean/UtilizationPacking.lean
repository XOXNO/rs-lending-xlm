import Fixed64Packing

namespace UtilizationPacking

theorem packing32 (h l : Int) (hl0 : 0 ≤ l) (hl : l < 4294967296) :
    Int.lor (h * 4294967296) l = h * 4294967296 + l := by
  exact Fixed64Packing.signed_packing h l 32 hl0 hl

theorem projected_packing32 (h l : Int) (hl0 : 0 ≤ l) (hl : l < 4294967296) :
    Int.lor (h * 4294967296) l % 18446744073709551616 =
      (h * 4294967296 + l) % 18446744073709551616 := by
  rw [packing32 h l hl0 hl]

-- Both strict operands remain; adding the fixed, nonzero modulus is total.
theorem strict_projected_packing32 (h l : Option Int)
    (domain : ∀ v, l = some v → 0 ≤ v ∧ v < 4294967296) :
    Fixed64Packing.strict2
      (fun hi lo => Int.lor (hi * 4294967296) lo % 18446744073709551616) h l =
    Fixed64Packing.strict2
      (fun hi lo => (hi * 4294967296 + lo) % 18446744073709551616) h l := by
  cases h with
  | none => rfl
  | some hi =>
    cases l with
    | none => rfl
    | some lo =>
      have bounds := domain lo rfl
      exact congrArg some (projected_packing32 hi lo bounds.1 bounds.2)

#print axioms packing32
#print axioms projected_packing32
#print axioms strict_projected_packing32
end UtilizationPacking
