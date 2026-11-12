-- Hand-written: the external types Aeneas leaves abstract. Trusted base.
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
type's arithmetic. (Same model as `RatesAeneas/TypesExternal.lean`.) -/
@[reducible, rust_type "aeneas_oracle_shim::I256"]
def aeneas_oracle_shim.I256 : Type := Int

/-- The host's 256-bit unsigned integer (`soroban_sdk::U256`), as a natural
number. Every value the verified code builds comes from `from_u32`,
`from_u128` or from an operation in `FunsExternal.lean` that traps (fails)
unless its exact result lies in `[0, 2^256)`, so plain `Nat` with those range
checks is the host type's arithmetic. -/
@[reducible, rust_type "aeneas_oracle_shim::U256"]
def aeneas_oracle_shim.U256 : Type := Nat
