-- Hand-written: the external type Aeneas leaves abstract. Trusted base.
module
public import Aeneas
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option linter.style.whitespace false
set_option linter.style.setOption false
set_option linter.style.longLine false
set_option linter.internalConstructors false

/-- The host's 256-bit signed integer, as a mathematical integer. Every value
the verified code builds comes from `from_i128` or from an operation in
`FunsExternal.lean` that traps (fails) unless its exact result lies in
`[-2^255, 2^255)`, so plain `Int` with those range checks is the host
type's arithmetic. -/
@[reducible, rust_type "aeneas_soroban_shim::I256"]
def aeneas_soroban_shim.I256 : Type := Int
