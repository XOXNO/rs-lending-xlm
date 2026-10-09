import Lean.Elab.Tactic.Omega

/- The three compareInt/Ordering2Int cases from K HOST-OBJECT data.md. -/
namespace UtilizationOrdering

def decode : Ordering → Int
  | .lt => -1
  | .eq => 0
  | .gt => 1

def compareInt (x y : Int) : Ordering :=
  if x < y then .lt else if x = y then .eq else .gt

def normalized (x y : Int) : Int :=
  if x < y then -1 else if x > y then 1 else 0

theorem comparison (x y : Int) : decode (compareInt x y) = normalized x y := by
  by_cases hlt : x < y
  · simp [compareInt, decode, normalized, hlt]
  · by_cases heq : x = y
    · subst y
      simp [compareInt, decode, normalized]
    · have hgt : x > y := by omega
      simp [compareInt, decode, normalized, hlt, heq, hgt]

def strict2 {α β γ : Type} (f : α → β → γ)
    (a : Option α) (b : Option β) : Option γ := do
  let x ← a
  let y ← b
  pure (f x y)

theorem strict_comparison (x y : Option Int) :
    strict2 (fun a b => decode (compareInt a b)) x y = strict2 normalized x y := by
  cases x with
  | none => rfl
  | some a =>
    cases y with
    | none => rfl
    | some b => exact congrArg some (comparison a b)

#print axioms comparison
#print axioms strict_comparison
end UtilizationOrdering
