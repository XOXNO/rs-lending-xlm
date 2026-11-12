/-
`#assert_standard_axioms Lib` makes the build fail unless every declaration
in the modules `Lib.*` (generated code, hand-written external models and all
proofs) depends only on Lean's three standard axioms: `propext`,
`Classical.choice` and `Quot.sound`.

`sorry` is the axiom `sorryAx`, `native_decide` uses `Lean.ofReduceBool`, and
an `axiom` declaration is an axiom, so one command rules out all three. Lean
itself only warns about `sorry`; each library's root file ends with this
command so that `lake build <Lib>` (and `run.sh`) cannot pass with one.

The command also fails if a `.lean` file under `Lib/` is not imported from
the root, since such a file would be neither built nor audited.
-/
import Lean

open Lean Elab Command

namespace Audit

/-- The axioms a proof may depend on. -/
def allowed : List Name := [``propext, ``Classical.choice, ``Quot.sound]

structure State where
  visited : NameSet := {}
  axioms : Array Name := #[]

/-- Collects the axioms `c` depends on, sharing the visited set across calls
(the same traversal as `Lean.collectAxioms`). -/
partial def collect (env : Environment) (c : Name) : StateM State Unit := do
  if (← get).visited.contains c then return
  modify fun s => { s with visited := s.visited.insert c }
  let go (e : Expr) : StateM State Unit := e.getUsedConstants.forM (collect env)
  match env.find? c with
  | some (.axiomInfo _) => modify fun s => { s with axioms := s.axioms.push c }
  | some (.defnInfo v) => go v.type *> go v.value
  | some (.thmInfo v) => go v.type *> go v.value
  | some (.opaqueInfo v) => go v.type *> go v.value
  | some (.ctorInfo v) => go v.type
  | some (.recInfo v) => go v.type
  | some (.inductInfo v) => go v.type *> v.ctors.forM (collect env)
  | some (.quotInfo _) | none => pure ()

/-- Fails unless every declaration of the modules `Lib.*` depends only on the
standard axioms and every source file under `Lib/` is imported. Reports the
number of declarations checked. -/
elab "#assert_standard_axioms " lib:ident : command => do
  let env ← getEnv
  let pfx := lib.getId
  let mods := env.header.moduleNames
  let mut decls : Array Name := #[]
  for (n, _) in env.constants.map₁.toList do
    let some idx := env.getModuleIdxFor? n | continue
    unless pfx.isPrefixOf mods[idx.toNat]! do continue
    decls := decls.push n
  if decls.isEmpty then
    throwError "#assert_standard_axioms: no declaration found in modules {pfx}.*"
  -- A source file that the root does not import is neither built nor audited.
  let root : System.FilePath := (← getFileName)
  let dir := (root.parent.getD ".") / pfx.toString
  let mut orphans : Array Name := #[]
  if ← dir.isDir then
    for f in ← dir.walkDir do
      if f.extension == some "lean" then
        let rel := (f.withExtension "").components.drop (dir.components.length - 1)
        let m := rel.foldl Name.mkStr Name.anonymous
        unless mods.contains m do orphans := orphans.push m
  unless orphans.isEmpty do
    throwError "{pfx}: source files not imported from the library root: {orphans}"
  let (_, s) := (decls.forM (collect env)).run {}
  let extra := s.axioms.filter (fun a => !allowed.contains a)
  unless extra.isEmpty do
    -- Slow path, only on failure: name the offending declarations.
    let mut bad : Array MessageData := #[]
    for n in decls do
      let (_, t) := (collect env n).run {}
      let ex := t.axioms.filter (fun a => !allowed.contains a)
      unless ex.isEmpty do bad := bad.push m!"{n} uses {ex}"
    throwError "{pfx}: declarations depend on non-standard axioms {extra}:\n{MessageData.joinSep bad.toList "\n"}"
  logInfo m!"{pfx}: {decls.size} declarations audited; axioms used: {s.axioms}"

end Audit
