import FloorWordComparison

/- Integer and strict partial arithmetic only; no Wasm/backend refinement. -/
namespace SignedWordNegative
open FloorWordComparison

theorem comparison (b : Bool) (w : Int) :
    ((if b then w else w + -word) < 0) ↔
    (if b then w < 0 else w < word) := by
  cases b <;> simp [word] <;> omega

def partialLhs (b : Option Bool) (w : Option Int) : Option Bool := do
  let b ← b
  let w ← w
  pure (decide ((if b then w else w + -word) < 0))

def partialRhs (b : Option Bool) (w : Option Int) : Option Bool := do
  let b ← b
  let w ← w
  pure (decide (if b then w < 0 else w < word))

theorem partial_equivalence (b : Option Bool) (w : Option Int) :
    partialLhs b w = partialRhs b w := by
  cases b <;> cases w <;> simp [partialLhs, partialRhs, comparison]

theorem lhs_definedness (b : Option Bool) (w : Option Int) :
    (partialLhs b w).isSome = (b.isSome && w.isSome) := by
  cases b <;> cases w <;> rfl

theorem rhs_definedness (b : Option Bool) (w : Option Int) :
    (partialRhs b w).isSome = (b.isSome && w.isSome) := by
  cases b <;> cases w <;> rfl

#print axioms comparison
#print axioms partial_equivalence
#print axioms lhs_definedness
#print axioms rhs_definedness
end SignedWordNegative
