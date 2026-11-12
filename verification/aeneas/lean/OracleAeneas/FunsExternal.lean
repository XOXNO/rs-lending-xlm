-- Hand-written: meanings for the functions Aeneas leaves abstract. Trusted base.
--
-- Three groups:
-- * Rust core library functions the Aeneas standard library does not model
--   yet. Each follows the Rust documentation. The first seven are copied from
--   `RatesAeneas/FunsExternal.lean`; `bool::then_some`, `u128::isqrt`,
--   `u128::pow` and `RangeInclusive::contains` are new.
-- * The Soroban host's I256 functions, copied from `RatesAeneas/FunsExternal.lean`:
--   exact integer arithmetic with the host's traps (add/sub/mul/div trap when
--   the exact result leaves the signed 256-bit range, div and rem_euclid trap on
--   a zero divisor, division truncates, rem_euclid is `Int.emod`).
-- * The Soroban host's U256 functions (soroban-sdk 28.0.0, `src/num.rs`, which
--   calls the host's `u256_*` functions), as exact natural-number arithmetic
--   with the host's traps: `add` and `mul` trap when the exact result exceeds
--   `2^256 - 1`, `sub` traps when the result would be negative, `div` traps on
--   a zero divisor and otherwise rounds down; `checked_mul` returns `None`
--   instead of trapping; `to_u128` is `Some` exactly when the value is below
--   `2^128`; comparisons are the order of the naturals.
module
public import Aeneas
public import OracleAeneas.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
open oracle_aeneas
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option linter.style.whitespace false
set_option linter.style.setOption false
set_option linter.style.longLine false
set_option linter.internalConstructors false

/-! ## Rust core library -/

/-- `bool::then_some`: `Some(t)` if the flag is true, else `None`. -/
def core.bool.Bool.then_some {T : Type _} (b : Bool) (t : T) : Result (Option T) :=
  ok (if b then some t else none)

/-- `i128::checked_pow`: the exact power when it fits `i128`, else `None`. -/
def core.num.I128.checked_pow (x : Std.I128) (e : Std.U32) : Result (Option Std.I128) :=
  ok (IScalar.tryMkOpt .I128 (x.val ^ e.val))

/-- `u32::abs_diff`: `|a - b|`, which always fits. -/
def core.num.U32.abs_diff (a b : Std.U32) : Result Std.U32 :=
  UScalar.tryMk .U32 (if b.val ≤ a.val then a.val - b.val else b.val - a.val)

/-- `u128::isqrt`: "the square root of the number, rounded down" (Rust docs),
which is `Nat.sqrt` and always fits `u128`. -/
def core.num.U128.isqrt (a : Std.U128) : Result Std.U128 :=
  UScalar.tryMk .U128 (Nat.sqrt a.val)

/-- `u128::pow`: the exact power; overflow panics (debug builds and constant
evaluation, the only use here being the constant `10u128.pow(34)`). -/
def core.num.U128.pow (x : Std.U128) (e : Std.U32) : Result Std.U128 :=
  UScalar.tryMk .U128 (x.val ^ e.val)

/-- `RangeInclusive::contains`: `start <= item` and `item <= end`
(short-circuiting); once the range is exhausted the end bound is exclusive,
as `RangeInclusive::end_bound` documents. -/
def core.ops.range.RangeInclusive.contains {Idx : Type _} {U : Type _}
    (_ : core.cmp.PartialOrd Idx Idx) (idxU : core.cmp.PartialOrd Idx U)
    (uIdx : core.cmp.PartialOrd U Idx) (r : core.ops.range.RangeInclusive Idx) (item : U) :
    Result Bool := do
  let lo ← idxU.le r.start item
  if lo then
    if r.exhausted then uIdx.lt item r.«end» else uIdx.le item r.«end»
  else ok false

def core.option.Option.unwrap_or_else {T : Type _} {F : Type _}
    (inst : core.ops.function.FnOnce F Unit T) (o : Option T) (f : F) : Result T :=
  match o with
  | some x => ok x
  | none => inst.call_once f ()

def core.option.Option.and_then {T : Type _} {U : Type _} {F : Type _}
    (inst : core.ops.function.FnOnce F T (Option U)) (o : Option T) (f : F) :
    Result (Option U) :=
  match o with
  | some x => inst.call_once f x
  | none => ok none

/-- The `?` operator on `Option`: continue with the value, or break with `None`. -/
def core.option.Option.Insts.CoreOpsTry_traitTry.branch {T : Type _} (o : Option T) :
    Result (core.ops.control_flow.ControlFlow (Option Never) T) :=
  match o with
  | some x => ok (.Continue x)
  | none => ok (.Break none)

def core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual
    (T : Type _) (_ : Option Never) : Result (Option T) :=
  ok none

/-! ## Soroban host -/

def aeneas_oracle_shim.Error.from_contract_error (code : Std.U32) :
    Result aeneas_oracle_shim.Error :=
  ok { code }

namespace aeneas_oracle_shim.I256

/-- Smallest and largest host I256 values. -/
def MIN : Int := -2^255
def MAX : Int := 2^255 - 1

/-- A host arithmetic result: the exact value, or a trap if it leaves the range. -/
def checked (v : Int) : Result aeneas_oracle_shim.I256 :=
  if MIN ≤ v ∧ v ≤ MAX then ok v else fail .integerOverflow

def from_i128 (_ : aeneas_oracle_shim.Env) (i : Std.I128) : Result aeneas_oracle_shim.I256 :=
  ok (i.val : Int)

/-- `Some` exactly when the value fits `i128`. -/
def to_i128 (v : aeneas_oracle_shim.I256) : Result (Option Std.I128) :=
  ok (IScalar.tryMkOpt .I128 v)

def add (a b : aeneas_oracle_shim.I256) : Result aeneas_oracle_shim.I256 :=
  checked ((a : Int) + b)

def sub (a b : aeneas_oracle_shim.I256) : Result aeneas_oracle_shim.I256 :=
  checked ((a : Int) - b)

def mul (a b : aeneas_oracle_shim.I256) : Result aeneas_oracle_shim.I256 :=
  checked ((a : Int) * b)

def div (a b : aeneas_oracle_shim.I256) : Result aeneas_oracle_shim.I256 :=
  if (b : Int) = 0 then fail .divisionByZero else checked (Int.tdiv a b)

def rem_euclid (a b : aeneas_oracle_shim.I256) : Result aeneas_oracle_shim.I256 :=
  if (b : Int) = 0 then fail .divisionByZero
  else if (a : Int) = MIN ∧ (b : Int) = -1 then fail .integerOverflow
  else ok (Int.emod a b)

def Insts.CoreCmpPartialEqI256.eq (a b : aeneas_oracle_shim.I256) : Result Bool :=
  ok (decide ((a : Int) = b))

def Insts.CoreCmpPartialOrdI256.partial_cmp (a b : aeneas_oracle_shim.I256) :
    Result (Option Ordering) :=
  ok (some (compare (a : Int) b))

end aeneas_oracle_shim.I256

namespace aeneas_oracle_shim.U256

/-- Largest host U256 value. -/
def MAX : Nat := 2^256 - 1

/-- A host arithmetic result: the exact value, or a trap if it exceeds `MAX`. -/
def checked (v : Nat) : Result aeneas_oracle_shim.U256 :=
  if v ≤ MAX then ok v else fail .integerOverflow

/-- `Clone`: a copy of the same value. -/
def Insts.CoreCloneClone.clone (a : aeneas_oracle_shim.U256) : Result aeneas_oracle_shim.U256 :=
  ok a

def from_u32 (_ : aeneas_oracle_shim.Env) (u : Std.U32) : Result aeneas_oracle_shim.U256 :=
  ok (u.val : Nat)

def from_u128 (_ : aeneas_oracle_shim.Env) (u : Std.U128) : Result aeneas_oracle_shim.U256 :=
  ok (u.val : Nat)

/-- `Some` exactly when the value fits `u128` (the high 128 bits are zero). -/
def to_u128 (v : aeneas_oracle_shim.U256) : Result (Option Std.U128) :=
  ok (UScalar.tryMkOpt .U128 v)

def add (a b : aeneas_oracle_shim.U256) : Result aeneas_oracle_shim.U256 :=
  checked ((a : Nat) + b)

/-- Unsigned subtraction traps on underflow. -/
def sub (a b : aeneas_oracle_shim.U256) : Result aeneas_oracle_shim.U256 :=
  if (b : Nat) ≤ a then ok ((a : Nat) - b) else fail .integerOverflow

def mul (a b : aeneas_oracle_shim.U256) : Result aeneas_oracle_shim.U256 :=
  checked ((a : Nat) * b)

/-- Division rounds down and traps on a zero divisor. -/
def div (a b : aeneas_oracle_shim.U256) : Result aeneas_oracle_shim.U256 :=
  if (b : Nat) = 0 then fail .divisionByZero else ok ((a : Nat) / b)

/-- `checked_mul`: the exact product, or `None` if it exceeds `MAX`. -/
def checked_mul (a b : aeneas_oracle_shim.U256) : Result (Option aeneas_oracle_shim.U256) :=
  ok (if (a : Nat) * b ≤ MAX then some ((a : Nat) * b) else none)

def Insts.CoreCmpPartialEqU256.eq (a b : aeneas_oracle_shim.U256) : Result Bool :=
  ok (decide ((a : Nat) = b))

def Insts.CoreCmpPartialOrdU256.partial_cmp (a b : aeneas_oracle_shim.U256) :
    Result (Option Ordering) :=
  ok (some (compare (a : Nat) b))

end aeneas_oracle_shim.U256
