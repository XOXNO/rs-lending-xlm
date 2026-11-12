-- Hand-written: the external types Aeneas leaves abstract. Trusted base.
-- Copied from RatesAeneas/TypesExternal.lean (I256) and extended for the pool
-- extraction (Address, array iterator).
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
@[reducible, rust_type "aeneas_pool_shim::I256"]
def aeneas_pool_shim.I256 : Type := Int

/-- `soroban_sdk::Address`, an opaque host handle, here a number (the model
of `ControllerAeneas/TypesExternal.lean`). The extracted code only stores and
clones it inside `Cache` (market key and asset id); distinct addresses stay
distinct, so two markets of one hub are different storage keys. -/
@[reducible, rust_type "aeneas_pool_shim::Address"]
def aeneas_pool_shim.Address : Type := Nat

/-- `core::array::IntoIter<T, N>`: the by-value iterator over an array.
Modelled as the list of elements not yet yielded, in order. -/
@[reducible, rust_type "core::array::iter::IntoIter"]
def core.array.iter.IntoIter (T : Type) (N : Std.Usize) : Type := List T

/-- `core::num::niche_types::NonZeroU64Inner`, the niche-carrying `u64`
wrapper inside `NonZeroU64`. Modelled as the `u64` it wraps. Reached only
through the `ZeroablePrimitive` impl for `u64` (`NonZeroU64::new`/`get` in
`interest::global_sync`); no generated code inspects it. -/
@[reducible, rust_type "core::num::niche_types::NonZeroU64Inner"]
def core.num.niche_types.NonZeroU64Inner : Type := Std.U64

/-- `core::num::NonZero<T>`: a `T` that is not zero. Modelled as the bare
`T`; the non-zero invariant is not carried by the type, but the only
constructor in the model, `NonZero::new` (FunsExternal.lean), returns `Some`
only for a non-zero value, as in Rust. -/
@[reducible, rust_type "core::num::nonzero::NonZero"]
def core.num.nonzero.NonZero (T : Type) (Clause0_NonZeroInner : Type) : Type := T

/-- `soroban_sdk::storage::Storage`, the host storage handle `Env::storage`
returns. Only `common::ttl::renew_instance` uses it, to extend the instance
TTL, which the model does not represent: a single value. -/
@[reducible, rust_type "aeneas_pool_shim::Storage"]
def aeneas_pool_shim.Storage : Type := Unit

/-- `soroban_sdk::storage::Instance`, instance storage (`Storage::instance`).
Only its TTL extension is called: a single value. -/
@[reducible, rust_type "aeneas_pool_shim::Instance"]
def aeneas_pool_shim.Instance : Type := Unit
