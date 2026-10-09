import UtilizationWords

/- Generic truncating-division simplifications. Every operator remains strict;
   a true guard requires defined inputs and a positive divisor. No Wasm claim
   or financial invariant is assumed. -/
namespace UtilizationQuotient

def strict2 {α β γ : Type} (f : α → β → γ)
    (a : Option α) (b : Option β) : Option γ := do
  let x ← a
  let y ← b
  pure (f x y)

def strict3 {α β γ δ : Type} (f : α → β → γ → δ)
    (a : Option α) (b : Option β) (c : Option γ) : Option δ := do
  let x ← a
  let y ← b
  let z ← c
  pure (f x y z)

def domain (n d : Int) : Bool := decide (0 ≤ n) && decide (0 < d)

theorem lower (n d : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    decide (n.tdiv d * d ≤ n) = domain n d := by
  simp [domain, hn, hd, Utilization.quotient_lower n d hn hd]

theorem upper (n d : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    decide (n < (n.tdiv d + 1) * d) = domain n d := by
  simp [domain, hn, hd, Utilization.quotient_upper n d hn hd]

theorem bound (n d q : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    decide (n.tdiv d ≤ q) = decide (n < (q + 1) * d) := by
  apply Bool.eq_iff_iff.mpr
  simpa only [decide_eq_true_eq] using Utilization.quotient_le_iff n d q hn hd

theorem strict_lower (n d : Option Int)
    (guard : strict2 domain n d = some true) :
    strict2 (fun x y => decide (x.tdiv y * y ≤ x)) n d =
      strict2 domain n d := by
  cases n with
  | none => rfl
  | some x =>
    cases d with
    | none => rfl
    | some y =>
      have h : 0 ≤ x ∧ 0 < y := by simpa [strict2, domain] using guard
      exact congrArg some (lower x y h.1 h.2)

theorem strict_upper (n d : Option Int)
    (guard : strict2 domain n d = some true) :
    strict2 (fun x y => decide (x < (x.tdiv y + 1) * y)) n d =
      strict2 domain n d := by
  cases n with
  | none => rfl
  | some x =>
    cases d with
    | none => rfl
    | some y =>
      have h : 0 ≤ x ∧ 0 < y := by simpa [strict2, domain] using guard
      exact congrArg some (upper x y h.1 h.2)

theorem strict_bound (n d q : Option Int)
    (guard : strict2 domain n d = some true) :
    strict3 (fun x y z => decide (x.tdiv y ≤ z)) n d q =
      strict3 (fun x y z => decide (x < (z + 1) * y)) n d q := by
  cases n with
  | none => rfl
  | some x =>
    cases d with
    | none => rfl
    | some y =>
      cases q with
      | none => rfl
      | some z =>
        have h : 0 ≤ x ∧ 0 < y := by simpa [strict2, domain] using guard
        exact congrArg some (bound x y z h.1 h.2)

#print axioms lower
#print axioms upper
#print axioms bound
#print axioms strict_lower
#print axioms strict_upper
#print axioms strict_bound
end UtilizationQuotient
