-- Hand-written: meanings for the functions Aeneas leaves abstract. Trusted base.
--
-- Three groups:
-- * Rust core library functions the Aeneas standard library does not model
--   yet. Each follows the Rust documentation. Copied from
--   `RatesAeneas/FunsExternal.lean`, plus `i128::from(u32)`.
-- * The Soroban host's I256 functions, copied from
--   `RatesAeneas/FunsExternal.lean`: exact integer arithmetic with the host's
--   traps (add/sub/mul/div trap when the exact result leaves the 256-bit range,
--   div and rem_euclid trap on a zero divisor). Division truncates toward zero
--   (`Int.tdiv`); the Euclidean remainder is `Int.emod`.
-- * The Soroban SDK `Vec<T>` methods the extracted functions call, on the
--   list model of `TypesExternal.lean`, following the soroban-sdk
--   documentation of `Vec`:
--     `Vec::new`        -> the empty vector;
--     `Vec::push_back`  -> appends at the back;
--     `Vec::iter`       -> yields the elements front to back;
--     `Clone::clone`    -> the same elements (a host-handle copy).
--   The host's limit of `u32::MAX` elements is not modelled: `push_back`
--   never fails here. Every theorem that uses `push_back` builds a vector no
--   longer than one it was given, so the limit cannot be reached.
-- * The Soroban SDK `Map<K, V>` methods (association-list model, see the Map
--   section) and the controller `Context` reads and effects (see the Context
--   section): documented one by one below and in `docs/controller.md`.
module
public import Aeneas
public import ControllerAeneas.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
open controller_aeneas
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option linter.style.whitespace false
set_option linter.style.setOption false
set_option linter.style.longLine false
set_option linter.internalConstructors false

/-! ## Rust core library -/

/-- `i128::from(u32)`: the same number; every `u32` fits `i128`. -/
def I128.Insts.CoreConvertFromU32.from (x : Std.U32) : Result Std.I128 :=
  IScalar.tryMk .I128 x.val

/-- `i128::checked_pow`: the exact power when it fits `i128`, else `None`. -/
def core.num.I128.checked_pow (x : Std.I128) (e : Std.U32) : Result (Option Std.I128) :=
  ok (IScalar.tryMkOpt .I128 (x.val ^ e.val))

/-- `u32::abs_diff`: `|a - b|`, which always fits. -/
def core.num.U32.abs_diff (a b : Std.U32) : Result Std.U32 :=
  UScalar.tryMk .U32 (if b.val ≤ a.val then a.val - b.val else b.val - a.val)

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

/-! ## Soroban host: I256 -/

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

/-! ## Soroban SDK: Address, Vec -/

/-- `Address::clone`: copies the handle. -/
def aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone
    (a : aeneas_soroban_shim.Address) : Result aeneas_soroban_shim.Address :=
  ok a

/-- `Vec::clone`: copies the host handle; the elements are the same. -/
def aeneas_soroban_shim.Vec.Insts.CoreCloneClone.clone {T : Type _}
    (_ : core.clone.Clone T) (v : aeneas_soroban_shim.Vec T) :
    Result (aeneas_soroban_shim.Vec T) :=
  ok v

/-- `Vec::new`: the empty vector. -/
def aeneas_soroban_shim.Vec.new (T : Type _) (_ : aeneas_soroban_shim.Env) :
    Result (aeneas_soroban_shim.Vec T) :=
  ok ([] : List T)

/-- `Vec::push_back`: appends `x` at the back. -/
def aeneas_soroban_shim.Vec.push_back {T : Type _} (v : aeneas_soroban_shim.Vec T) (x : T) :
    Result (aeneas_soroban_shim.Vec T) :=
  ok ((v : List T) ++ [x])

/-- `Vec::iter`: an iterator over all elements, front first. -/
def aeneas_soroban_shim.Vec.iter {T : Type _} (v : aeneas_soroban_shim.Vec T) :
    Result (aeneas_soroban_shim.VecIter T) :=
  ok v

/-- `Iterator::next` on the vector iterator: the next element, front first,
or `None` once every element has been yielded. -/
def aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next {T : Type _}
    (it : aeneas_soroban_shim.VecIter T) :
    Result (Option T × aeneas_soroban_shim.VecIter T) :=
  match (it : List T) with
  | [] => ok (none, ([] : List T))
  | x :: xs => ok (some x, xs)

/-! ## Rust core library (added for the liquidation flow) -/

/-- `i128::saturating_mul`: the exact product clamped to `[i128::MIN, i128::MAX]`. -/
def core.num.I128.saturating_mul (a b : Std.I128) : Result Std.I128 :=
  IScalar.tryMk .I128 (max (-2^127) (min (2^127 - 1) (a.val * b.val)))

/-- `Option::map_or`: `f x` for `Some x`, the default for `None`. -/
def core.option.Option.map_or {T : Type _} {U : Type _} {F : Type _}
    (inst : core.ops.function.FnOnce F T U) (o : Option T) (d : U) (f : F) : Result U :=
  match o with
  | some x => inst.call_once f x
  | none => ok d

/-- `Iterator::all`: calls the predicate on each item front to back and stops
at the first `false`; `true` when every item satisfies it (also when empty).
Returns the iterator in the state where it stopped. -/
def core.iter.traits.iterator.Iterator.all.default
    {Self : Type _} {F : Type _} {Item : Type _}
    (it : core.iter.traits.iterator.Iterator Self Item)
    (fm : core.ops.function.FnMut F Item Bool) (s : Self) (f : F) : Result (Bool × Self) :=
  loop (fun (p : Self × F) => do
      let (o, s1) ← it.next p.1
      match o with
      | none => ok (ControlFlow.done (true, s1))
      | some x => do
        let (b, f1) ← fm.call_mut p.2 x
        if b then ok (ControlFlow.cont (s1, f1)) else ok (ControlFlow.done (false, s1)))
    (s, f)

/-- `Iterator::find`: the first item (front to back) satisfying the
predicate, or `None`; returns the iterator in the state where it stopped. -/
def core.iter.traits.iterator.Iterator.find.default
    {Self : Type _} {P : Type _} {Item : Type _}
    (it : core.iter.traits.iterator.Iterator Self Item)
    (fm : core.ops.function.FnMut P Item Bool) (s : Self) (f : P) :
    Result (Option Item × Self) :=
  loop (fun (p : Self × P) => do
      let (o, s1) ← it.next p.1
      match o with
      | none => ok (ControlFlow.done (none, s1))
      | some x => do
        let (b, f1) ← fm.call_mut p.2 x
        if b then ok (ControlFlow.done (some x, s1)) else ok (ControlFlow.cont (s1, f1)))
    (s, f)

/-! ## Soroban SDK: Address equality, more Vec methods -/

/-- `Address == Address`: equality of the handles (the host compares the
underlying addresses). -/
def aeneas_soroban_shim.Address.Insts.CoreCmpPartialEqAddress.eq
    (a b : aeneas_soroban_shim.Address) : Result Bool :=
  ok (decide (a = b))

/-- `Vec::len`: the number of elements (a `u32`; the host never holds more). -/
def aeneas_soroban_shim.Vec.len {T : Type _} (v : aeneas_soroban_shim.Vec T) : Result Std.U32 :=
  UScalar.tryMk .U32 (v : List T).length

/-- `Vec::is_empty`. -/
def aeneas_soroban_shim.Vec.is_empty {T : Type _} (v : aeneas_soroban_shim.Vec T) : Result Bool :=
  ok (v : List T).isEmpty

/-- `Vec::get(i)`: the element at index `i` (0 = front), `None` out of range. -/
def aeneas_soroban_shim.Vec.get {T : Type _} (v : aeneas_soroban_shim.Vec T) (i : Std.U32) :
    Result (Option T) :=
  ok ((v : List T)[i.val]?)

/-- `Vec::set(i, x)`: replaces the element at index `i`; the host traps when
`i` is out of range. -/
def aeneas_soroban_shim.Vec.set {T : Type _} (v : aeneas_soroban_shim.Vec T) (i : Std.U32) (x : T) :
    Result (aeneas_soroban_shim.Vec T) :=
  if i.val < (v : List T).length then ok (List.set (v : List T) i.val x) else fail .panic

/-- `Vec::remove(i)`: removes the element at index `i`, shifting the later
ones down, and returns `Some(())`; returns `None` and leaves the vector
unchanged when `i` is out of range. -/
def aeneas_soroban_shim.Vec.remove {T : Type _} (v : aeneas_soroban_shim.Vec T) (i : Std.U32) :
    Result (Option Unit × aeneas_soroban_shim.Vec T) :=
  if i.val < (v : List T).length then ok (some (), List.eraseIdx (v : List T) i.val)
  else ok (none, v)

/-- `Vec::append(other)`: appends the elements of `other` at the back. -/
def aeneas_soroban_shim.Vec.append {T : Type _} (v w : aeneas_soroban_shim.Vec T) :
    Result (aeneas_soroban_shim.Vec T) :=
  ok ((v : List T) ++ (w : List T))

/-- `IntoIterator for Vec`: iterates the elements front to back. -/
def aeneas_soroban_shim.Vec.Insts.CoreIterTraitsCollectIntoIteratorTVecIter.into_iter
    {T : Type _} (v : aeneas_soroban_shim.Vec T) : Result (aeneas_soroban_shim.VecIter T) :=
  ok v

/-- `IntoIterator for &Vec`: iterates the elements front to back. -/
def Shared0Vec.Insts.CoreIterTraitsCollectIntoIteratorTVecIter.into_iter
    {T : Type _} (v : aeneas_soroban_shim.Vec T) : Result (aeneas_soroban_shim.VecIter T) :=
  ok v

/-! ## Soroban SDK: Map

`Map<K, V>` is a list of (key, value) pairs in iteration order
(`TypesExternal.lean`). Keys are compared by equality of their values (the
host compares keys by value; `HubAssetKey` and `Address` keys are plain data),
decided classically. What the proofs use:

* `get k` returns the value of the first entry whose key is `k`, `None` if
  there is none;
* `iter` / `keys` / `len` / `is_empty` read the entry list as it is;
* `set k v` replaces the value of the entry with key `k`, or adds `(k, v)`
  when there is none (keys stay unique).

Not modelled: the host's ascending key order. No theorem depends on the
iteration order, so they hold for the host's order in particular. -/

/-- `Map::new`: the empty map. -/
def aeneas_soroban_shim.Map.new (K : Type _) (V : Type _) (_ : aeneas_soroban_shim.Env) :
    Result (aeneas_soroban_shim.Map K V) :=
  ok ([] : List (K × V))

open Classical in
/-- `Map::get(k)`: the value stored under `k`, or `None`. -/
noncomputable def aeneas_soroban_shim.Map.get {K : Type _} {V : Type _}
    (m : aeneas_soroban_shim.Map K V) (k : K) : Result (Option V) :=
  ok (((m : List (K × V)).find? (fun e => decide (e.1 = k))).map Prod.snd)

open Classical in
/-- `Map::set(k, v)`: stores `v` under `k`, replacing an existing value. -/
noncomputable def aeneas_soroban_shim.Map.set {K : Type _} {V : Type _}
    (m : aeneas_soroban_shim.Map K V) (k : K) (v : V) : Result (aeneas_soroban_shim.Map K V) :=
  if (m : List (K × V)).any (fun e => decide (e.1 = k)) then
    ok ((m : List (K × V)).map (fun e => if e.1 = k then (k, v) else e))
  else ok ((m : List (K × V)) ++ [(k, v)])

/-- `Map::len`: the number of entries. -/
def aeneas_soroban_shim.Map.len {K : Type _} {V : Type _} (m : aeneas_soroban_shim.Map K V) :
    Result Std.U32 :=
  UScalar.tryMk .U32 (m : List (K × V)).length

/-- `Map::is_empty`. -/
def aeneas_soroban_shim.Map.is_empty {K : Type _} {V : Type _} (m : aeneas_soroban_shim.Map K V) :
    Result Bool :=
  ok (m : List (K × V)).isEmpty

/-- `Map::keys`: the keys, in iteration order. -/
def aeneas_soroban_shim.Map.keys {K : Type _} {V : Type _} (m : aeneas_soroban_shim.Map K V) :
    Result (aeneas_soroban_shim.Vec K) :=
  ok ((m : List (K × V)).map Prod.fst)

/-- `Map::iter`: the (key, value) entries, in iteration order. -/
def aeneas_soroban_shim.Map.iter {K : Type _} {V : Type _} (m : aeneas_soroban_shim.Map K V) :
    Result (aeneas_soroban_shim.VecIter (K × V)) :=
  ok m

/-! ## Controller `Context` and host effects

The extracted `Context` (`controller/src/context.rs`) holds the two caches
the liquidation and risk code reads: `token_prices` (asset → price feed) and
`market_indexes` (hub asset → borrow/supply index). `cached_price` and
`cached_market_index` are extracted from their (production) Rust bodies. The
rest of the host state (oracle prices not yet cached, pool indexes, spoke
configuration and usage, token balances, the pool's books) is represented by
the uninterpreted constants below: a theorem proved with them holds for every
value they could stand for. Functions that only write state the model does
not represent (event buffers, spoke usage, storage) leave the modelled
`Context` unchanged. -/

/-- The pool's simulated current index for a hub asset (what
`fetch_pool_bulk_indexes` returns for it). Uninterpreted. -/
opaque poolIndex : types.HubAssetKey → types.MarketIndexRaw :=
  fun _ => { borrow_index := 0#i128, supply_index := 0#i128 }

/-- The price the aggregator returns for an asset, `None` when it cannot
price it (the fetch then aborts). Uninterpreted. -/
opaque oraclePrice : aeneas_soroban_shim.Address → Option types.PriceFeedRaw := fun _ => none

/-- `external::fetch_market_index` (one-key `fetch_pool_bulk_indexes`): the
pool's current index for the key. -/
def external.fetch_market_index (_ : aeneas_soroban_shim.Env) (k : types.HubAssetKey) :
    Result types.MarketIndexRaw :=
  ok (poolIndex k)

open Classical in
/-- One step of `load_markets` for key `k`: cache the price of `k.asset` from
the oracle if missing (abort if the oracle has none), and the pool index of
`k` if missing. Cached entries are kept. -/
noncomputable def loadOne (ctx : context.Context) (k : types.HubAssetKey) : Result context.Context :=
  let withPrice : Result context.Context :=
    if (ctx.token_prices : List _).any (fun e => decide (e.1 = k.asset)) then ok ctx
    else match oraclePrice k.asset with
      | some p => ok { ctx with token_prices := (ctx.token_prices : List _) ++ [(k.asset, p)] }
      | none => fail .panic
  do
    let c ← withPrice
    if (c.market_indexes : List _).any (fun e => decide (e.1 = k)) then ok c
    else ok { c with market_indexes := (c.market_indexes : List _) ++ [(k, poolIndex k)] }

/-- `Context::load_markets`: loads every missing price (by token address) and
every missing index (by hub-asset key) of the keys; aborts if the oracle has
no price for a missing asset. Already cached values are retained. (Production
batches the fetches; the resulting cache is the same.) -/
noncomputable def context.Context.load_markets (ctx : context.Context)
    (keys : aeneas_soroban_shim.Vec types.HubAssetKey) : Result context.Context :=
  (keys : List types.HubAssetKey).foldlM loadOne ctx

/-- The controller's configured pool address. Uninterpreted. -/
opaque poolAddress : aeneas_soroban_shim.Address := 0

/-- `Context::cached_pool_address`: the pool address; the modelled caches are unchanged. -/
def context.Context.cached_pool_address (ctx : context.Context) :
    Result (aeneas_soroban_shim.Address × context.Context) :=
  ok (poolAddress, ctx)

/-- The listed configuration of a hub asset in a spoke, `None` if unlisted.
Uninterpreted. -/
opaque spokeAssetConfig : Std.U32 → types.HubAssetKey → Option types.AssetConfig := fun _ _ => none

/-- `Context::require_spoke_asset`: the listed config; aborts if unlisted. -/
def context.Context.require_spoke_asset (ctx : context.Context) (spoke : Std.U32)
    (k : types.HubAssetKey) : Result (types.AssetConfig × context.Context) :=
  match spokeAssetConfig spoke k with
  | some c => ok (c, ctx)
  | none => fail .panic

/-- Whether the spoke-usage bookkeeping accepts an exit (it aborts on a
spoke mismatch). Uninterpreted. -/
opaque spokeExitOk : Std.U32 → spoke_usage.UsageSide → types.HubAssetKey → math.fp.Ray → Bool :=
  fun _ _ _ _ => true

/-- `Context::apply_spoke_exit`: updates spoke usage (not modelled) or aborts. -/
def context.Context.apply_spoke_exit (ctx : context.Context) (spoke : Std.U32)
    (side : spoke_usage.UsageSide) (k : types.HubAssetKey) (d : math.fp.Ray) :
    Result context.Context :=
  if spokeExitOk spoke side k d then ok ctx else fail .panic

/-- `Context::record_supply_position_update`: buffers an event (not modelled). -/
def context.Context.record_supply_position_update (ctx : context.Context)
    (_ : events.PositionAction) (_ : types.HubAssetKey) (_ _ : Std.I128)
    (_ : types.AccountPosition) : Result context.Context :=
  ok ctx

/-- Whether a spoke asset's flags allow the leg under the policy. Uninterpreted. -/
opaque spokeFlagsAllow : Std.U32 → types.HubAssetKey → positions.FreezePolicy → Bool :=
  fun _ _ _ => true

/-- `positions::enforce_spoke_asset_flags`: aborts when the flags forbid the
leg; otherwise the modelled caches are unchanged. -/
def positions.enforce_spoke_asset_flags (_ : aeneas_soroban_shim.Env) (ctx : context.Context)
    (spoke : Std.U32) (k : types.HubAssetKey) (p : positions.FreezePolicy) : Result context.Context :=
  if spokeFlagsAllow spoke k p then ok ctx else fail .panic

/-- The balance increase measured at `to` when `amount` of `asset` is
transferred from `from`. Uninterpreted (any token behaviour). -/
opaque transferReceived : aeneas_soroban_shim.Address → aeneas_soroban_shim.Address →
    aeneas_soroban_shim.Address → Std.I128 → Std.I128 := fun _ _ _ a => a

/-- `common::token::transfer_amount_measured`: aborts unless `amount > 0`;
returns the measured receipt. -/
def payment_legs.transfer_amount_measured (_ : aeneas_soroban_shim.Env)
    (asset src dst : aeneas_soroban_shim.Address) (amount : Std.I128) (_ : errors.GenericError) :
    Result Std.I128 :=
  if 0 < amount.val then ok (transferReceived asset src dst amount) else fail .panic

/-- `apply_repay_batch`: the pool repay call and its application to the
account and the cached indexes. Uninterpreted. -/
opaque positions.apply_repay_batch : aeneas_soroban_shim.Env → types.Account →
    aeneas_soroban_shim.Address → events.PositionAction → aeneas_soroban_shim.Vec types.PoolAction →
    context.Context →
    Result (aeneas_soroban_shim.Vec types.PoolPositionMutation × types.Account × context.Context)

/-- `apply_withdraw_batch`: the pool withdraw call and its application to the
account and the cached indexes. Uninterpreted. -/
opaque positions.apply_withdraw_batch : aeneas_soroban_shim.Env → types.Account →
    aeneas_soroban_shim.Address → positions.WithdrawKind → events.PositionAction →
    aeneas_soroban_shim.Vec types.PoolWithdrawEntry → context.Context →
    Result (aeneas_soroban_shim.Vec types.PoolPositionMutation × types.Account × context.Context)

/-- `pool_seize_positions_call`: the pool call reclassifying fee shares as
revenue. Uninterpreted. -/
opaque external.pool.pool_seize_positions_call : aeneas_soroban_shim.Env →
    aeneas_soroban_shim.Address → aeneas_soroban_shim.Vec types.PoolSeizeEntry → Result Unit

/-- `account::cleanup_account_if_empty` (defined in `account_store`) (defined in `account_store`): removes an empty account from
storage. Uninterpreted. -/
opaque account_store.cleanup_account_if_empty : aeneas_soroban_shim.Env → types.Account → Std.U64 →
    Result Unit

/-- `account::update_or_remove_supply_position` (defined in `account_store`): writes (or removes, at zero)
the supply position in the in-memory account. Uninterpreted. -/
opaque account_store.update_or_remove_supply_position : types.Account → types.HubAssetKey →
    types.AccountPosition → Result types.Account

/-- `bad_debt::execute_bad_debt_cleanup`: seizes the remaining collateral and
socializes the remaining debt through the pool. Uninterpreted. -/
opaque positions.liquidation.bad_debt.execute_bad_debt_cleanup : aeneas_soroban_shim.Env →
    context.Context → Std.U64 → types.Account → risk.totals.AccountRiskTotals →
    Result context.Context

/-- `math::repays_every_debt_leg` (private, kept opaque: its `Iterator::all`
closure mutably borrows the context, which the Aeneas library's `FnMut`
model cannot express): whether every debt position has a repayment at its
ceiling-rounded balance. Uninterpreted, so the `repays_all_debt` flag of a
normalized plan is arbitrary in the model and no theorem depends on its value. -/
opaque positions.liquidation.math.repays_every_debt_leg : aeneas_soroban_shim.Env →
    types.Account → aeneas_soroban_shim.Vec types.RepayEntry → context.Context →
    Result (Bool × context.Context)

