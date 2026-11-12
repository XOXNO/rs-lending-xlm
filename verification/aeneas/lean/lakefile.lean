import Lake
open Lake DSL

/-
The Aeneas Lean library, at the commit of the pinned Aeneas release (see
../README.md). Lake resolves its Mathlib dependency; `lake exe cache get`
downloads prebuilt Mathlib files.
-/
require aeneas from git
  "https://github.com/AeneasVerif/aeneas.git" @ "0d21a9ec7bbbb110110cacc14464a47fd71c8705" / "backends" / "lean"

package «lending-aeneas» where
  leanOptions := #[⟨`weak.linter.dupNamespace, false⟩]

@[default_target] lean_lib «RatesAeneas» {}

-- One library per extraction crate (see ../WRITING-PROOFS.md). Each root file
-- (`PoolAeneas.lean`, ...) imports its generated code and proofs, and ends
-- with `#assert_standard_axioms`, so building a library fails on any `sorry`
-- or added axiom.
lean_lib «PoolAeneas» {}
lean_lib «ControllerAeneas» {}
lean_lib «OracleAeneas» {}

-- Rounding specifications (`IsFloor`, `IsCeil`, `IsHalfUp`) shared by the
-- rates, pool and controller libraries: they import only Mathlib.
lean_lib «FixedPoint» {}

-- The `#assert_standard_axioms` command (Audit.lean).
lean_lib «Audit» {}
