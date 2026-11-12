import Lake
open Lake DSL

/-
The Aeneas Lean library, at the commit of the pinned Aeneas release (see
../README.md). Lake resolves its Mathlib dependency; `lake exe cache get`
downloads prebuilt Mathlib files.
-/
require aeneas from git
  "https://github.com/AeneasVerif/aeneas.git" @ "0d21a9ec7bbbb110110cacc14464a47fd71c8705" / "backends" / "lean"

package «rates-aeneas» where
  leanOptions := #[⟨`weak.linter.dupNamespace, false⟩]

@[default_target] lean_lib «RatesAeneas» {}

-- One library per extraction crate (see ../WRITING-PROOFS.md). Each root file
-- (`PoolAeneas.lean`, ...) imports its generated code and proofs.
lean_lib «PoolAeneas» {}
lean_lib «ControllerAeneas» {}
lean_lib «OracleAeneas» {}
