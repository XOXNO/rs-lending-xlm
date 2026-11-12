-- Hand-written: the external types Aeneas leaves abstract. Trusted base.
--
-- The `I256` model is the one of `RatesAeneas/TypesExternal.lean`. The
-- collection types model the Soroban SDK's host objects by their contents:
-- * `soroban_sdk::Vec<T>` is an ordered sequence; index 0 is the front
--   (`Vec::get(0)`, the first element `iter()` yields). Its handle has value
--   semantics in the verified code: `clone` copies the handle, and every
--   mutation (`push_back`, ...) rebinds the variable to a new host object, so
--   a Lean `List T` is the observable state.
-- * `VecIter<T>` is the front-to-back iterator `Vec::iter()` returns; its state
--   is the list of elements not yet yielded.
-- * `Address` is an opaque host handle; the verified code copies and compares it.
-- * `soroban_sdk::Map<K, V>` is an association list of (key, value) pairs in
--   iteration order. What the proofs rely on is listed in `FunsExternal.lean`
--   (Map section) and in `docs/controller.md` ("Map model").
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

/-- The host's 256-bit signed integer, as a mathematical integer (see
`RatesAeneas/TypesExternal.lean`). Every value the verified code builds comes
from `from_i128` or from an operation in `FunsExternal.lean` that traps
(fails) unless its exact result lies in `[-2^255, 2^255)`. -/
@[reducible, rust_type "aeneas_soroban_shim::I256"]
def aeneas_soroban_shim.I256 : Type := Int

/-- A contract address: an opaque host handle, here a number. The verified
functions only copy addresses (clone is the identity). -/
@[reducible, rust_type "aeneas_soroban_shim::Address"]
def aeneas_soroban_shim.Address : Type := Nat

/-- `soroban_sdk::Vec<T>`: the sequence of its elements, front first. -/
@[reducible, rust_type "aeneas_soroban_shim::Vec"]
def aeneas_soroban_shim.Vec (T : Type _) : Type _ := List T

/-- The iterator `Vec::iter()` returns: the elements not yet yielded, in order. -/
@[reducible, rust_type "aeneas_soroban_shim::VecIter"]
def aeneas_soroban_shim.VecIter (T : Type _) : Type _ := List T

/-- `soroban_sdk::Map<K, V>`: its entries as (key, value) pairs, in iteration
order. Production maps iterate in ascending key order with unique keys; the
model does not record either fact, and no theorem assumes them (every theorem
about a map holds for any entry list, so in particular for the host's). -/
@[reducible, rust_type "aeneas_soroban_shim::Map"]
def aeneas_soroban_shim.Map (K : Type _) (V : Type _) : Type _ := List (K × V)

