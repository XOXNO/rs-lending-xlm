-- Hand-written: meanings for the functions Aeneas leaves abstract. Trusted base.
--
-- Two groups:
-- * Rust core library functions the Aeneas standard library does not model
--   yet (checked_pow, abs_diff, Range::contains, three Option combinators and
--   the `?` operator on Option). Each follows the Rust documentation.
-- * The Soroban host's I256 functions, as exact integer arithmetic with the
--   host's traps: add/sub/mul/div trap when the exact result leaves the
--   256-bit range, div and rem_euclid trap on a zero divisor, rem_euclid traps
--   on (MIN, -1) like `checked_rem_euclid`. Division truncates toward zero
--   (`Int.tdiv`); the Euclidean remainder is `Int.emod` (in [0, |d|)).
module
public import Aeneas
public import RatesAeneas.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
open rates_aeneas
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option linter.style.whitespace false
set_option linter.style.setOption false
set_option linter.style.longLine false
set_option linter.internalConstructors false

/-! ## Rust core library -/

/-- `i128::checked_pow`: the exact power when it fits `i128`, else `None`. -/
def core.num.I128.checked_pow (x : Std.I128) (e : Std.U32) : Result (Option Std.I128) :=
  ok (IScalar.tryMkOpt .I128 (x.val ^ e.val))

/-- `u32::abs_diff`: `|a - b|`, which always fits. -/
def core.num.U32.abs_diff (a b : Std.U32) : Result Std.U32 :=
  UScalar.tryMk .U32 (if b.val ≤ a.val then a.val - b.val else b.val - a.val)

/-- `Range::contains`: `start <= item && item < end` (short-circuiting). -/
def core.ops.range.Range.contains {Idx : Type _} {U : Type _}
    (_ : core.cmp.PartialOrd Idx Idx) (idxU : core.cmp.PartialOrd Idx U)
    (uIdx : core.cmp.PartialOrd U Idx) (r : core.ops.range.Range Idx) (item : U) :
    Result Bool := do
  let lo ← idxU.le r.start item
  if lo then uIdx.lt item r.«end» else ok false

def core.option.Option.unwrap_or_else {T : Type _} {F : Type _}
    (inst : core.ops.function.FnOnce F Unit T) (o : Option T) (f : F) : Result T :=
  match o with
  | some x => ok x
  | none => inst.call_once f ()

def core.option.Option.map_or {T : Type _} {U : Type _} {F : Type _}
    (inst : core.ops.function.FnOnce F T U) (o : Option T) (default : U) (f : F) : Result U :=
  match o with
  | some x => inst.call_once f x
  | none => ok default

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

def aeneas_soroban_shim.Error.from_contract_error (code : Std.U32) :
    Result aeneas_soroban_shim.Error :=
  ok { code }

namespace aeneas_soroban_shim.I256

/-- Smallest and largest host I256 values. -/
def MIN : Int := -2^255
def MAX : Int := 2^255 - 1

/-- A host arithmetic result: the exact value, or a trap if it leaves the range. -/
def checked (v : Int) : Result aeneas_soroban_shim.I256 :=
  if MIN ≤ v ∧ v ≤ MAX then ok v else fail .integerOverflow

def from_i128 (_ : aeneas_soroban_shim.Env) (i : Std.I128) : Result aeneas_soroban_shim.I256 :=
  ok (i.val : Int)

/-- `Some` exactly when the value fits `i128`. -/
def to_i128 (v : aeneas_soroban_shim.I256) : Result (Option Std.I128) :=
  ok (IScalar.tryMkOpt .I128 v)

def add (a b : aeneas_soroban_shim.I256) : Result aeneas_soroban_shim.I256 :=
  checked ((a : Int) + b)

def sub (a b : aeneas_soroban_shim.I256) : Result aeneas_soroban_shim.I256 :=
  checked ((a : Int) - b)

def mul (a b : aeneas_soroban_shim.I256) : Result aeneas_soroban_shim.I256 :=
  checked ((a : Int) * b)

def div (a b : aeneas_soroban_shim.I256) : Result aeneas_soroban_shim.I256 :=
  if (b : Int) = 0 then fail .divisionByZero else checked (Int.tdiv a b)

def rem_euclid (a b : aeneas_soroban_shim.I256) : Result aeneas_soroban_shim.I256 :=
  if (b : Int) = 0 then fail .divisionByZero
  else if (a : Int) = MIN ∧ (b : Int) = -1 then fail .integerOverflow
  else ok (Int.emod a b)

def Insts.CoreCmpPartialEqI256.eq (a b : aeneas_soroban_shim.I256) : Result Bool :=
  ok (decide ((a : Int) = b))

def Insts.CoreCmpPartialOrdI256.partial_cmp (a b : aeneas_soroban_shim.I256) :
    Result (Option Ordering) :=
  ok (some (compare (a : Int) b))

end aeneas_soroban_shim.I256
