-- Hand-written: meanings for the functions Aeneas leaves abstract. Trusted base.
--
-- Copied from RatesAeneas/FunsExternal.lean (namespace renamed) and extended
-- for the pool extraction: `i128::from(u32)`, `i128::from(u64)` (since #224)
-- and the array by-value iterator.
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
public import PoolAeneas.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
open pool_aeneas
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

/-! ## Rust core library: additions for the pool extraction -/

/-- `impl From<u32> for i128`: the same integer (always fits). -/
def I128.Insts.CoreConvertFromU32.from (x : Std.U32) : Result Std.I128 :=
  IScalar.tryMk .I128 (x.val : Int)

/-- `impl From<u64> for i128` (Rust core, infallible lossless widening): the
same integer. It always fits, since `u64::MAX < 2^127`, so `tryMk` never takes
its failure branch (proved as `i128_from_u64_spec` in `Proofs/Wrappers.lean`). Reached
through `rates::compound::compound_interest` (`i128::from(delta_ms)`) since #224. -/
def I128.Insts.CoreConvertFromU64.from (x : Std.U64) : Result Std.I128 :=
  IScalar.tryMk .I128 (x.val : Int)

/-- `<[T; N] as IntoIterator>::into_iter`: an iterator over the elements in order. -/
def Array.Insts.CoreIterTraitsCollectIntoIteratorTIntoIter.into_iter {T : Type} {N : Std.Usize}
    (a : Array T N) : Result (core.array.iter.IntoIter T N) :=
  ok a.val

/-- `<array::IntoIter<T, N> as Iterator>::next`: the next element, or `None` when exhausted. -/
def core.array.iter.IntoIter.Insts.CoreIterTraitsIteratorIterator.next {T : Type} {N : Std.Usize}
    (it : core.array.iter.IntoIter T N) : Result (Option T × core.array.iter.IntoIter T N) :=
  match it with
  | [] => ok (none, [])
  | x :: xs => ok (some x, xs)

/-! ## Soroban host -/

namespace aeneas_pool_shim.I256

/-- Smallest and largest host I256 values. -/
def MIN : Int := -2^255
def MAX : Int := 2^255 - 1

/-- A host arithmetic result: the exact value, or a trap if it leaves the range. -/
def checked (v : Int) : Result aeneas_pool_shim.I256 :=
  if MIN ≤ v ∧ v ≤ MAX then ok v else fail .integerOverflow

def from_i128 (_ : aeneas_pool_shim.Env) (i : Std.I128) : Result aeneas_pool_shim.I256 :=
  ok (i.val : Int)

/-- `Some` exactly when the value fits `i128`. -/
def to_i128 (v : aeneas_pool_shim.I256) : Result (Option Std.I128) :=
  ok (IScalar.tryMkOpt .I128 v)

def add (a b : aeneas_pool_shim.I256) : Result aeneas_pool_shim.I256 :=
  checked ((a : Int) + b)

def sub (a b : aeneas_pool_shim.I256) : Result aeneas_pool_shim.I256 :=
  checked ((a : Int) - b)

def mul (a b : aeneas_pool_shim.I256) : Result aeneas_pool_shim.I256 :=
  checked ((a : Int) * b)

def div (a b : aeneas_pool_shim.I256) : Result aeneas_pool_shim.I256 :=
  if (b : Int) = 0 then fail .divisionByZero else checked (Int.tdiv a b)

def rem_euclid (a b : aeneas_pool_shim.I256) : Result aeneas_pool_shim.I256 :=
  if (b : Int) = 0 then fail .divisionByZero
  else if (a : Int) = MIN ∧ (b : Int) = -1 then fail .integerOverflow
  else ok (Int.emod a b)

def Insts.CoreCmpPartialEqI256.eq (a b : aeneas_pool_shim.I256) : Result Bool :=
  ok (decide ((a : Int) = b))

def Insts.CoreCmpPartialOrdI256.partial_cmp (a b : aeneas_pool_shim.I256) :
    Result (Option Ordering) :=
  ok (some (compare (a : Int) b))

end aeneas_pool_shim.I256

/-! ## Rust core library: `NonZeroU64` (for `interest::global_sync`) -/

/-- `<NonZeroU64Inner as Clone>::clone`: the same value. -/
def core.num.niche_types.NonZeroU64Inner.Insts.CoreCloneClone.clone
    (x : core.num.niche_types.NonZeroU64Inner) : Result core.num.niche_types.NonZeroU64Inner :=
  ok x

/-- `NonZero::<u64>::new` (Rust core): `Some(n)` iff `n != 0`, else `None`;
never panics. Modelled only at `T = u64`, the one instance the extracted code
reaches (`NonZeroU64::new(remaining)` in `interest::global_sync`); the
generated call elaborates against this monomorphic signature. -/
def core.num.nonzero.NonZero.new
    (_ : core.num.nonzero.ZeroablePrimitive Std.U64 core.num.niche_types.NonZeroU64Inner)
    (x : Std.U64) :
    Result (Option (core.num.nonzero.NonZero Std.U64 core.num.niche_types.NonZeroU64Inner)) :=
  ok (if x.val = 0 then none else some x)

/-- `NonZero::<u64>::get` (Rust core): the wrapped value; never panics. -/
def core.num.nonzero.NonZero.get
    (_ : core.num.nonzero.ZeroablePrimitive Std.U64 core.num.niche_types.NonZeroU64Inner)
    (x : core.num.nonzero.NonZero Std.U64 core.num.niche_types.NonZeroU64Inner) :
    Result Std.U64 :=
  ok x

/-! ## Host handles, storage, ledger time (for the `ops/*` legs)

The operation legs load a market (`Cache::load`: `storage::read_params`,
`storage::read_state`, `storage::renew_market`, `time::now_ms`), accrue,
mutate, and `commit` (`storage::write_state`). Persistent storage and ledger
time are not modelled as state: the reads are **uninterpreted constants**
(`opaque`, no axiom), so a theorem proved about a leg holds for every value
storage and the ledger could return; the theorems name the loaded values
through hypotheses (`ops.synced_market env k = ok c`). Writes and TTL
extensions return `()` and change nothing the extracted code reads back:
within one leg the market is loaded once and committed once, at the end.
Host resource-limit traps of these calls are not modelled. -/

/-- `<Env as Clone>::clone`: the environment handle is a unit value. -/
def aeneas_pool_shim.Env.Insts.CoreCloneClone.clone (e : aeneas_pool_shim.Env) :
    Result aeneas_pool_shim.Env :=
  ok e

/-- `<Address as Clone>::clone`: copies the handle. -/
def aeneas_pool_shim.Address.Insts.CoreCloneClone.clone (a : aeneas_pool_shim.Address) :
    Result aeneas_pool_shim.Address :=
  ok a

/-- `Env::storage`: the storage handle (unit). -/
def aeneas_pool_shim.Env.storage (_ : aeneas_pool_shim.Env) : Result aeneas_pool_shim.Storage :=
  ok ()

/-- `Storage::instance`: the instance-storage handle (unit). -/
def aeneas_pool_shim.Storage.instance (_ : aeneas_pool_shim.Storage) :
    Result aeneas_pool_shim.Instance :=
  ok ()

/-- `Instance::extend_ttl` (via `common::ttl::renew_instance`): extends the
instance TTL, which the model does not represent; returns `()`. -/
def aeneas_pool_shim.Instance.extend_ttl (_ : aeneas_pool_shim.Instance) (_ _ : Std.U32) :
    Result Unit :=
  ok ()

/-- The current ledger time in ms (`time::now_ms`, which aborts if
`timestamp·1000` overflows `u64`). Uninterpreted. -/
opaque ledgerNowMs : Result Std.U64 := fail .panic

/-- `time::now_ms`: the uninterpreted ledger time. -/
def time.now_ms (_ : aeneas_pool_shim.Env) : Result Std.U64 :=
  ledgerNowMs

/-- The stored market params for a key (`storage::read_params`, which aborts
with `PoolNotInitialized` for a missing market). Uninterpreted. -/
opaque storedParams : types.pool.HubAssetKey → Result types.pool.MarketParamsRaw :=
  fun _ => fail .panic

/-- The stored market state for a key (`storage::read_state`, aborting for a
missing market). Uninterpreted. -/
opaque storedState : types.pool.HubAssetKey → Result types.pool.PoolStateRaw :=
  fun _ => fail .panic

/-- `storage::read_params`: the uninterpreted stored params. -/
def storage.read_params (_ : aeneas_pool_shim.Env) (k : types.pool.HubAssetKey) :
    Result types.pool.MarketParamsRaw :=
  storedParams k

/-- `storage::read_state`: the uninterpreted stored state. -/
def storage.read_state (_ : aeneas_pool_shim.Env) (k : types.pool.HubAssetKey) :
    Result types.pool.PoolStateRaw :=
  storedState k

/-- `storage::renew_market`: extends the market's TTLs (not modelled); `()`. -/
def storage.renew_market (_ : aeneas_pool_shim.Env) (_ : types.pool.HubAssetKey) :
    Result Unit :=
  ok ()

/-- `storage::write_state` (in `Cache::commit`): persists the state; returns
`()`. The written state is what the theorems describe (the committed cache's
fields); nothing in a leg reads storage after its commit. -/
def storage.write_state (_ : aeneas_pool_shim.Env) (_ : types.pool.HubAssetKey)
    (_ : types.pool.PoolStateRaw) : Result Unit :=
  ok ()

/-- `events::emit_market_value_ceiling` (in `interest::global_sync`): publishes
`MarketValueCeilingEvent`; returns `()`. An event writes only the host event
buffer, which nothing in the extracted code reads. -/
def events.emit_market_value_ceiling (_ : aeneas_pool_shim.Env) (_ : types.pool.HubAssetKey) :
    Result Unit :=
  ok ()
