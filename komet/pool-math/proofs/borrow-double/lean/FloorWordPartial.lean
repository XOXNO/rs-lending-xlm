import FloorWordComparison

/-
Strict partial model only: None is undefined; Some contains a defined Int.
The fixed positive word divisor makes both inner operations total on Int.
This proves lifting the integer equality through strict partial operands.
It does not prove K hook strictness, backend refinement, or a Wasm/APR claim.
-/
namespace FloorWordComparison

def partialLhs (x n : Option Int) : Option Bool := do
  let a ← x
  let b ← n
  pure (decide ((a - a % word).tdiv word < b))

def partialRhs (x n : Option Int) : Option Bool := do
  let a ← x
  let b ← n
  pure (decide (a < b * word))

theorem partial_equivalence (x n : Option Int) :
    partialLhs x n = partialRhs x n := by
  cases x <;> cases n <;> simp [partialLhs, partialRhs, word_lt_iff]

theorem lhs_definedness (x n : Option Int) :
    (partialLhs x n).isSome = (x.isSome && n.isSome) := by
  cases x <;> cases n <;> rfl

theorem rhs_definedness (x n : Option Int) :
    (partialRhs x n).isSome = (x.isSome && n.isSome) := by
  cases x <;> cases n <;> rfl

#print axioms partial_equivalence
#print axioms lhs_definedness
#print axioms rhs_definedness

end FloorWordComparison
