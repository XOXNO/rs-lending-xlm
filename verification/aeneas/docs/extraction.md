# Extraction crates, trusted models and Aeneas workarounds

This document describes how the controller and pool proofs get their Lean
code. It covers the extraction crates, the hand-written models the proofs
trust, the copied proof files, the rebuild commands and the Aeneas bugs the
extraction works around. The theorems are in [controller.md](controller.md)
and [pool.md](pool.md). The pinned tool versions are in
[../README.md](../README.md#trusted-base).

## Controller

### Controller extraction crate

`verification/aeneas/controller/` is a standalone Cargo workspace
(`[workspace]` table, members `.` and `shim`). It is never part of the root
workspace or any Wasm build.

**Included verbatim** through `#[path]`. Nothing under `common/` or
`contracts/` is copied or edited.

* `common/src/constants/` (`src/constants.rs`) and
  `contracts/controller/src/constants.rs`. They are re-exported together, so
  both `common::constants::X` and `crate::constants::BAD_DEBT_USD_THRESHOLD`
  resolve.
* `common/src/math/{fp,fp_core}.rs` (`src/math.rs`). `fp_core` is a private
  module, as in `common/src/math/mod.rs`. So the included controller files
  reach it only through the typed `fp` wrappers.
* `common/src/rates/{scaling,value}.rs` (`src/rates.rs`, with the `pub use`
  lines of `rates/mod.rs`). `curve`, `index`, `compound` and `simulate` are
  not needed.
* `contracts/controller/src/positions/liquidation/{curve,math,apply}.rs`
  (`src/positions/liquidation/mod.rs`).

**Generated from production** (one mechanical substitution):

* `controller/gen_totals.py` (run by `extract.sh`) writes `src/risk/totals.rs`
  from `contracts/controller/src/risk/totals.rs`.
* The copy is verbatim with one exception. The `fn(&Env, Ray, Ray, Wad) -> Wad`
  argument of `sum_debt_usd_loaded` becomes an enum `DebtValue { HalfUp, Ceil }`.
* Its `apply` calls exactly `position_value` / `position_value_ceil`, the two
  functions production passes.
* The reason: Aeneas cannot translate `fn` pointers with a higher-ranked
  lifetime ([issue 2](#controller-aeneas-issues)).
* Each of the four substitutions must match exactly once. So a production
  edit there fails the extraction.

`src/lib.rs` declares `extern crate self as common;` so the controller files'
`common::...` paths resolve to this crate.

**Stand-ins** are compiled in the crate. Each is either never extracted or a
field-for-field copy.

* `src/types.rs`: the structs the included files use (`SpokeConfig`,
  `HubAssetKey`, `PriceFeed(Raw)`, `MarketIndex(Raw)`, `AccountPosition(Raw)`,
  `DebtPosition(Raw)`, `PaymentTuple`, `SeizeEntry`, `RepayEntry`,
  `LiquidationResult`, `Account` without `mode`).
  * They have exactly the production fields. The production `From`
    conversions are copied verbatim.
  * The production types are `#[contracttype]`, which only adds XDR
    serialisation.
  * The file also carries `AccountPositionType`, `AggregatedPayments`,
    `ScaledPositionRaw` (with its two `From` impls), `PoolAction`,
    `PoolWithdrawEntry`, `PoolSeizeEntry`, `AssetConfig`,
    `Account::get_or_create_supply_position` (verbatim) and a field-less
    `PoolPositionMutation`. `HubAssetKey` derives `PartialEq, Eq` as in
    production.
* `src/errors.rs`: the error variants raised, with production codes. Codes
  are not modelled; every `panic_with_error!` is an abort.
* `src/validation.rs`: `expect_invariant`, `require_nonneg_amount`,
  `require_non_empty_payments`, copied verbatim.
* `src/payment_legs.rs` (re-exported as `crate::payments`): `ZeroLeg`,
  `aggregate_positive_payments`, `aggregate_payments` and
  `aggregate_payment_amount`, copied verbatim from
  `contracts/controller/src/payments.rs`. `transfer_amount_measured` is opaque.
* `src/context.rs`: the [Context model](#controller-trusted-base).
* `src/storage.rs`: `iter_typed_positions` / `iter_debt_positions`.
  * Production writes `map.iter().map(|(k, raw)| (k, From::from(&raw)))`.
    The stand-in writes it as a loop that pushes the same items in the same
    order and returns the vector's iterator.
  * Aeneas translates neither `impl Iterator` return types nor the `map`
    adapter.
* Opaque stand-ins: signatures only, Charon `--opaque`, meaning in
  `FunsExternal.lean`.
  * `account_store.rs` (re-exported as `crate::account`):
    `cleanup_account_if_empty`, `update_or_remove_supply_position`.
  * `external/{mod,pool}.rs`: `fetch_market_index`, `pool_seize_positions_call`.
  * `positions/mod.rs`: `enforce_spoke_asset_flags`, `apply_repay_batch`,
    `apply_withdraw_batch`. `make_pool_action`, `get_supply_position_or_panic`,
    `FreezePolicy` and `WithdrawKind` are verbatim.
  * `positions/liquidation/bad_debt.rs`: `execute_bad_debt_cleanup`.
* Never extracted (signatures for compilation): `risk/validation.rs`,
  `collections.rs` (`push_unique_address`, verbatim), `events.rs`,
  `spoke_usage.rs` (enums only).

`../check_standins.py` compares the copied production items in
`controller/src` (`types.rs`, `cached_price`, the payment aggregation,
`make_pool_action`, ...) with production. `run.sh` runs it first. An adapted
stand-in pins the hash of the production item it replaces.

**Extraction scripts.**

* `controller/extract.sh` lists the roots (`--start-from`) and the opaque
  items (`--opaque`). It passes `--inline-anon-consts`
  ([issue 1](#controller-aeneas-issues)).
* `controller/postextract.sh` applies one documented fix to the generated
  `Funs.lean` ([issue 3](#controller-aeneas-issues)).
* For scratch runs, `AENEAS_LEAN_DEST=<dir>` redirects the output and
  `KEEP_TEMPLATES=1` keeps the `*_Template.lean` files.

**Shim** (`controller/shim/`, package `aeneas-soroban-shim`, copied from
`verification/aeneas/shim` and extended):

* `Env`, `I256`, `Error` and `panic_with_error!` as in the rates crate;
* plus `assert_with_error!`, `Address`, `Vec<T>` (with `VecIter<T>` and
  `IntoIterator`) and `Map<K, V>`.

Charon translates only the signatures of shim items. So they are opaque to
Aeneas and get their meaning in the trusted base below.

### Controller trusted base

1. Charon and Aeneas translate faithfully (pinned release, see
   [../README.md](../README.md)).
2. `lean/ControllerAeneas/TypesExternal.lean` and `FunsExternal.lean` hold
   the hand-written models below.
3. `panic_with_error!`/`assert_with_error!` abort (as in `RatesAeneas`).

The models:

* **`I256`** and the Rust core functions (`checked_pow`, `abs_diff`,
  `Option::{unwrap_or_else, and_then}`, `?` on `Option`): copied unchanged
  from `RatesAeneas`. Exact integer arithmetic with the host's 256-bit traps.
* **`i128::from(u32)`**: the same number (always fits).
* **`Address`**: an opaque handle (`Nat`). `clone` is the identity. Equality
  is equality of the handles.
* **`Vec<T>`**: a Lean `List T`, index 0 at the front.
  * `Vec::new` = `[]`; `push_back v x` = `v ++ [x]`.
  * `Vec::iter` yields the elements front to back. `VecIter T` is the list of
    elements not yet yielded; `next` pops the head and returns `None` when
    empty.
  * `clone` returns the same elements. This follows the soroban-sdk `Vec`
    documentation: a host object with value semantics (`clone` copies the
    handle, mutations rebind the variable).
  * Further methods: `len` (as `u32`), `is_empty`, `get(i)` (`None` out of
    range), `set(i, x)` (traps out of range), `remove(i)` (shifts later
    elements down; `None` and unchanged out of range), `append`, `into_iter`
    (front to back).
  * Not modelled: the host's `u32::MAX` length limit. `push_back` never fails
    here. The only theorem that pushes builds an output no longer than its
    input.
* **Map model** (`Map<K, V>`): a Lean `List (K × V)` of entries in iteration
  order.
  * `get k` is the value of the first entry whose key equals `k` (keys
    compared by value, decided classically).
  * `set k v` replaces that entry's value or appends `(k, v)`.
  * `keys`/`iter`/`len`/`is_empty` read the list.
  * The proofs rely only on `get` after the list is given, and on
    `iter`/`keys` yielding the list itself.
  * Not modelled: the host's ascending key order and key uniqueness of
    arbitrary inputs. Every theorem quantifies over all entry lists, so it
    holds for the host's sorted, unique lists in particular.
* **Rust core**, each per the Rust docs: `i128::saturating_mul` (exact product
  clamped to the `i128` range), `Option::map_or`, and `Iterator::{all, find}`
  (front to back, stopping at the first `false` / match).
* **Context model** (`src/context.rs`): a struct with the two caches the
  extracted code reads, `token_prices` (`Map<Address, PriceFeedRaw>`) and
  `market_indexes` (`Map<HubAssetKey, MarketIndexRaw>`).
  * `cached_price` is the production body. It aborts when the asset is not
    cached.
  * `cached_market_index` is the production body, with the one-key pool fetch
    as one call.
  * Uninterpreted Lean constants (`opaque` definitions, not axioms) represent
    the rest of the host state. So a theorem holds for every value they could
    stand for. The constants:
    * `poolIndex`: the pool's current index for a key, used by
      `fetch_market_index`;
    * `oraclePrice`: the aggregator's price (`None` means the fetch aborts);
    * `poolAddress`, `spokeAssetConfig`, `spokeExitOk`, `spokeFlagsAllow`;
    * `transferReceived`: the measured balance change of a transfer (any
      token behaviour).
  * `load_markets` caches the missing price of each key's asset (aborting if
    the oracle has none) and the missing index of each key. It keeps cached
    entries. Production batches the same fetches.
  * `cached_pool_address`, `require_spoke_asset`, `apply_spoke_exit`,
    `record_supply_position_update` and `enforce_spoke_asset_flags` leave the
    two caches unchanged. They abort where production can abort. In
    production they only touch spoke usage, configuration caches, event
    buffers and the pool-address cache.
  * `transfer_amount_measured` aborts unless `amount > 0` and returns
    `transferReceived`.
  * `apply_repay_batch`, `apply_withdraw_batch`, `pool_seize_positions_call`,
    `cleanup_account_if_empty`, `update_or_remove_supply_position`,
    `execute_bad_debt_cleanup` and `repays_every_debt_leg` are uninterpreted
    functions. A theorem about a caller either does not depend on them or
    states the call explicitly (an equation).

### Controller copied proofs

The rounding specifications (`Spec`, `FloorCeilSpec`, `HalfUpSpec`) are not
copied. They live once in `lean/FixedPoint/` and this library imports them.

Each extraction generates its own copy of the common math. So the proofs
about that math exist once per library.

* `Host`, `MulDiv` (`mul_div_floor_spec`), `FloorCeilRescale`
  (`rescale_floor`/`rescale_ceil`), `HalfUp` (`mul_div_half_up_spec`) and
  `HalfUpDivInt` (`div_by_int_half_up`) are the RatesAeneas files with the
  library, crate and shim names changed. `../sync_copies.py` writes them, and
  `run.sh` fails if one is out of date.
* `FloorCeilCore` (`mul_div_ceil_spec`) is a hand-adapted copy. It omits the
  `mul_div_floor_saturating` section, which is not extracted here.
* `Saturating.lean` is the `mul_div_floor_saturating` section of
  `RatesAeneas/Proofs/FloorCeilCore.lean` (header comment says so).
* `Scaling.lean` is `RatesAeneas/Proofs/FloorCeilScaling.lean` up to
  `unscale_borrow_ceil_spec`, without the `calculate_scaled_*` theorems.
  `unscale_supply_floor_spec` is renamed `unscale_supply_floor_isFloor`,
  because `Seizure.lean` has its own.

### Controller rebuild

```sh
export AENEAS_HOME=/path/to/aeneas-nightly-2026.11.12-0d21a9e   # or your unpacked release
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas
ALLOW_DRIFT=1 ./run.sh controller   # extract.sh, then postextract.sh, then lake build ControllerAeneas
```

`extract.sh` does the following:

1. Runs `gen_totals.py`.
2. Runs `charon cargo --preset=aeneas --inline-anon-consts --dest-file
   target/llbc/controller.llbc --start-from ... --opaque ...`.
3. Runs `aeneas -backend lean -dest ../lean -subdir ControllerAeneas
   -split-files target/llbc/controller.llbc`. The `-subdir` makes the
   generated imports `ControllerAeneas.*`.
4. Deletes the `*_Template.lean` files.

`run.sh` then runs `postextract.sh` on the generated library
([issue 3](#controller-aeneas-issues)).

### Controller Aeneas issues

The numbered issues below are the Aeneas bugs the controller extraction
avoids.

1. **Internal error on `process_excess_payment`** (`InterpAbs.ml` line 940,
   reported as `Interp.ml` line 686). The trigger is a loop whose condition
   compares a derived-`PartialOrd` newtype against a temporary and
   short-circuits. Minimal reproduction:

   ```rust
   #[derive(Clone, Copy, PartialEq, PartialOrd)]
   pub struct W9(i128);
   pub fn v9(excess: W9, mut n: u32) -> W9 {
       let mut rem = excess;
       while rem > W9(0) && n > 0 {   // `rem > W9(0)` alone, or `rem > z` with a local `z`, works
           n -= 1;
           rem = W9(rem.0 - 1);
       }
       rem
   }
   ```

   * In production the condition is `remaining_excess_usd > Wad::ZERO &&
     current_index > 0`.
   * Charon's `--inline-anon-consts` avoids it. `--no-code-duplication` also
     avoids it, but breaks other functions.
   * `extract.sh` passes `--inline-anon-consts`. It leaves the translation of
     every other extracted function unchanged (checked by diffing
     `Funs.lean`).
2. **`fn` pointers with a higher-ranked lifetime are not supported** ("arrow
   types with locally quantified regions"), nor the coercion of a function
   item to one.
   * `gen_totals.py` avoids it ([crate section](#controller-extraction-crate)).
   * Passing `position_value` as a generic `F: Fn(&Env, ..)` fails too
     ("lifetime constraint relating a higher-ranked lifetime to a free
     lifetime").
3. **Provided `Iterator` methods emitted as instance fields.**
   * For the shim's `impl Iterator for VecIter<T>`, Aeneas puts `all` and
     `find` into the trait instance.
   * The Lean library's `Iterator` structure has no such fields (Aeneas warns
     about it). So `Funs.lean` does not compile.
   * The calls go to `core.iter.traits.iterator.Iterator.{all,find}.default`
     directly. So `controller/postextract.sh` deletes exactly these two field
     definitions from that one instance. It fails if the instance has another
     shape.
4. **Parameter names shadow module namespaces.**
   * A function with a parameter named `account` (or `payments`) that calls
     `crate::account::f` is emitted as `account.f`. Lean resolves that as
     field access on the parameter.
   * The crate defines those items in `account_store` / `payment_legs` and
     re-exports them as `crate::account` / `crate::payments`. So the
     generated names differ from the parameters.
5. **Closures with a mutable capture.**
   * `repays_every_debt_leg`'s `Iterator::all` closure captures
     `cache: &mut Context`.
   * The generated `FnMut` instance has type `closure → Item → Result (Bool ×
     closure × (closure → closure))`, which does not match the library's
     `FnMut`.
   * The function is kept opaque.

## Pool

### Pool extraction crate

`verification/aeneas/pool/` is a standalone Cargo workspace (own
`[workspace]` table). It is never part of the root workspace or any Wasm
build.

| Path | Contents |
|---|---|
| `pool/Cargo.toml` | Package `pool-aeneas`; `soroban-sdk` renamed to the shim; `[package.metadata.charon]` lists the extraction roots |
| `pool/src/lib.rs` | Crate root; `#[path]` includes; `extern crate self as common` |
| `pool/src/{types,oracle,ownable,storage,time,events}.rs` | [Stand-ins](#pool-stand-ins-and-shim) |
| `pool/shim/` | `aeneas-pool-shim`: signature-only `soroban-sdk` stand-in (copied from `../shim`, extended) |
| `pool/shim-macros/` | `aeneas-pool-shim-macros`: identity `#[contracttype]` / `#[contracterror]` |

**Included verbatim** (`#[path]`). Nothing in `common/` or `contracts/` is
copied or edited.

* From `common/src`: `constants/mod.rs` (with `pool.rs`, `shared.rs`),
  `errors.rs`, `math/mod.rs` (`fp.rs`, `fp_core.rs`), `rates/mod.rs` (all six
  submodules), `validation.rs`, `ttl.rs`, `types/pool.rs`, `types/shared.rs`.
* From `contracts/pool/src`: `cache/mod.rs` (and through it `cache/cash.rs`,
  `cache/report.rs`, `cache/scale.rs`, `cache/shares.rs`), `guards.rs`,
  `interest.rs`, `ops/mod.rs` (and through it every leg: `borrow.rs`,
  `flash.rs`, `market.rs`, `net_settle.rs`, `recapitalize.rs`, `repay.rs`,
  `revenue.rs`, `seize.rs`, `strategy.rs`, `supply.rs`, `withdraw.rs`).
* `ops/mod.rs` is included verbatim, so `synced_market`, `renewed_market`
  and `load_leg` are the production code.
* `revenue.rs` imports `stellar_access::ownable`. That resolves through a
  second `extern crate self as stellar_access` to the `ownable` stand-in.

**Layout trick.** The crate root mirrors both crates:

* The `common` modules sit at the root exactly where `common/src/lib.rs` puts
  them, so their own `crate::` paths resolve.
* `extern crate self as common` makes the pool files' `common::...` paths
  resolve to the same modules.
* The cache module is mounted as `market_cache` and re-exported with
  `use market_cache as cache`. So the production paths
  (`crate::cache::Cache`, `super::Cache`) resolve unchanged.

The extra name works around an Aeneas name clash. Generated Lean refers to
`cache.Cache.…` relative to the crate namespace. A function parameter named
`cache` (in `guards.rs`, `interest.rs`, `flash.rs`) shadows that name. So an
extraction under the name `cache` does not compile.

### Pool extraction roots

Charon translates only what these items reach (`start_from` in
`pool/Cargo.toml`):

* `crate::market_cache::shares`, `crate::market_cache::cash` (minus
  `transfer_out`, excluded because it calls the token client);
* `crate::guards`;
* `crate::interest::{global_sync, accrue_chunk, add_protocol_revenue, apply_bad_debt_to_supply_index}`.
  `global_sync` needs the `postextract.sh` workaround
  ([Aeneas bug](#aeneas-bug-duplicate-parent-clause-field));
* `crate::ops::flash::{terms, book_fee, finalize}`. `finalize` is the tail of
  a flash loan after the balance checks: fee booking, entry ceiling, commit;
* `crate::rates::scaling`, so the copied RatesAeneas scaling proofs find
  every function they name;
* the operation legs, at the function that does no token call:
  `ops::supply::apply`, `ops::borrow::accounting`,
  `ops::withdraw::accounting`, `ops::repay::accounting`,
  `ops::net_settle::apply`, `ops::seize::apply`,
  `ops::recapitalize::accounting`, `ops::strategy::accounting`,
  `ops::revenue::accounting`.

`opaque = ["crate::storage", "crate::time", "crate::events"]`. Charon
translates the storage, ledger-time and event stand-ins as signatures only.
Their meaning is in `FunsExternal.lean` ([trusted base](#pool-trusted-base)).

Through the legs, these are in the Lean code:

* `Cache::load` (storage reads, `now_ms`) and `Cache::commit` (storage
  write);
* `ttl::renew_instance` (instance TTL) and the full `global_sync`;
* the two events they publish: `events::emit_market_value_ceiling`
  (`global_sync`) and `events::emit_market_state` (`flash::finalize`).

These are not reached:

* token transfers: `Cache::transfer_out` (excluded) and the `apply` wrappers
  of borrow/withdraw/repay/recapitalize/strategy/revenue;
* the other events;
* the flash-loan callback and its balance checks (`flash::apply`);
* `run_batch`, `market.rs` and the contract entry points (`lib.rs`, so also
  `get_sync_data`).

### Pool stand-ins and shim

Hand-written Rust stand-ins:

| File | Stands in for | Reaches Lean? |
|---|---|---|
| `src/types.rs` | `common::types` | Yes (types only) |
| `src/oracle.rs` | `common::oracle::observation` | No |
| `src/storage.rs` | `contracts/pool/src/storage.rs` | Signatures only (`opaque`) |
| `src/time.rs` | `contracts/pool/src/time.rs` | Signature only (`opaque`) |
| `src/events.rs` | `contracts/pool/src/events.rs` | Signatures only (`opaque`) |
| `src/ownable.rs` | `stellar_access::ownable` | No |

* `src/types.rs` includes `types/pool.rs` and `types/shared.rs` verbatim. It
  adds a copy of the `OracleTolerance` record (fields only) that
  `validation.rs` names in a function no root reaches. The controller/oracle
  type files are left out.
* `src/oracle.rs` holds the three constants `validation.rs` imports, copied
  with production values.
* `src/storage.rs` has the production signatures with `unimplemented!()`
  bodies (never executed).
  * Reached: `read_params`, `read_state`, `write_state`, `renew_market`.
  * `FunsExternal.lean` models persistent storage: reads are uninterpreted,
    writes return `()`.
  * `market_exists`, `write_params`, `write_rate_model`, `load_state` and
    `load_sync_data` exist only for `market.rs` / type-checking.
* `src/time.rs` declares `now_ms`, uninterpreted in Lean.
* `src/events.rs` declares the five emitters' signatures.
  * Reached: `emit_market_value_ceiling` and `emit_market_state`.
    `global_sync` calls `emit_market_value_ceiling` when the value ceiling
    held a chunk's interest. `flash::finalize` calls `emit_market_state`
    after its commit.
  * Only the `apply` wrappers, `run_batch` and `market.rs` call the other
    three, so they are not reached.
* `src/ownable.rs` declares the `get_owner` signature, for
  `ops::revenue::apply` (not a root).

Charon translates only the signatures of shim items (`pool/shim`,
`pool/shim-macros`), so they are opaque to Aeneas. Items that reach Lean get
a meaning in the trusted base below. The rest exist only so the verbatim
files type-check.

| Shim item | Reaches Lean | Notes |
|---|---|---|
| `Env` | Yes | Unit struct; Aeneas translates it to `Unit` (in `Types.lean`) |
| `I256` and its 7 methods, `PartialEq`, `PartialOrd` | Yes | As in the rates shim |
| `Address` | Yes (type only) | Stored in `Cache` (`hub_asset`, `params.asset_id`), never inspected |
| `panic_with_error!`, `assert_with_error!` | — | Macros: `panic!` / `if !cond { panic!(..) }`, i.e. abort |
| `contracttype`, `contracterror` | — | Identity attributes (the real ones derive host conversions the arithmetic never calls) |
| `Env: Clone`, `Address: Clone` | Yes | `Cache::load` clones the env and the market key |
| `Storage`, `Instance`, `Env::storage`, `Storage::instance`, `Instance::extend_ttl` | Yes | For the verbatim `common/src/ttl.rs` (`renew_instance`, called by `ops::renewed_market`) |
| Type-check-only items (list below) | No | Needed only so the verbatim files compile |

The type-check-only items are `Error`, `Bytes`, `BytesN`, `Symbol`,
`String`, `Val`, `IntoVal`, `TryFromVal`, `Vec` (`new`, `is_empty`, `iter`,
`contains`, `push_back`), `Executable`, `Address::executable`,
`Env::{current_contract_address, invoke_contract}` and `token::Client`
(incl. `try_transfer`).

* They type-check `validation.rs`, `cash.rs::transfer_out`, the
  token/callback half of `flash.rs`, `ops::run_batch` and `withdraw::apply`.
* `TryFromVal`, the three `Vec` methods and `try_transfer` serve the
  verbatim `ops/mod.rs` / `withdraw.rs`.

### Pool trusted base

Beyond Charon/Aeneas themselves (pinned in `../README.md`), the proofs trust
the hand-written meanings in `lean/PoolAeneas/TypesExternal.lean` and
`FunsExternal.lean`.

| Item | Models | Definition |
|---|---|---|
| `aeneas_pool_shim.I256` | Soroban host 256-bit integer | `Int` (copied from RatesAeneas) |
| `I256.{from_i128, to_i128, add, sub, mul, div, rem_euclid, eq, partial_cmp}`, `MIN`, `MAX`, `checked` | Host I256 arithmetic | Exact integer arithmetic trapping outside `[-2^255, 2^255)`, truncating `div`, Euclidean `rem_euclid`, zero-divisor traps (copied from RatesAeneas) |
| `core.num.I128.checked_pow`, `core.num.U32.abs_diff`, `Option::{unwrap_or_else, map_or, and_then}`, `?` on `Option`, `Range::contains` | Rust core | Per the Rust documentation (copied from RatesAeneas) |
| `aeneas_pool_shim.Address` | `soroban_sdk::Address` | `Nat`, an opaque handle (as in the controller library); the extracted code only stores and clones it |
| `core.array.iter.IntoIter T N` | `core::array::IntoIter` | `List T`, the elements not yet yielded |
| `Array…IntoIterator.into_iter` | `<[T; N]>::into_iter` | The array's elements in order |
| `IntoIter…Iterator.next` | `<array::IntoIter>::next` | Head and tail; `None` when exhausted |
| `I128.Insts.CoreConvertFromU32.from` | `impl From<u32> for i128` | The same integer (always fits) |
| `I128.Insts.CoreConvertFromU64.from` | `impl From<u64> for i128` | The same integer ([note](#from-u64-model)) |
| `core.num.niche_types.NonZeroU64Inner` (type) | `core::num::niche_types::NonZeroU64Inner` | `Std.U64`, the value it wraps; never inspected |
| `core.num.nonzero.NonZero T Inner` (type) | `core::num::NonZero<T>` | The bare `T` ([note](#nonzero-model)) |
| `core.num.nonzero.NonZero.new` | `NonZero::<u64>::new` | `ok (if x = 0 then none else some x)`; never panics ([note](#nonzero-model)) |
| `core.num.nonzero.NonZero.get` | `NonZero::<u64>::get` | `ok x`, the wrapped value |
| `NonZeroU64Inner.Insts.CoreCloneClone.clone` | `<NonZeroU64Inner as Clone>::clone` | `ok x`; only referenced by the generated `Copy` instance |
| `pool/postextract.sh` | Mechanical fix of generated code | Renames the duplicated `ZeroablePrimitive` field ([Aeneas bug](#aeneas-bug-duplicate-parent-clause-field)) |
| `aeneas_pool_shim.Storage`, `aeneas_pool_shim.Instance` (types) | Soroban storage handles | `Unit` (only the TTL extension is called on them) |
| `Env.Insts.CoreCloneClone.clone`, `Address.Insts.CoreCloneClone.clone` | `Clone` of the env / an address | `ok` of the same (unit) value |
| `Env.storage`, `Storage.instance` | Handle accessors | `ok ()` |
| `Instance.extend_ttl` | Instance TTL extension (`common::ttl::renew_instance`) | `ok ()`; TTL and host resource traps are not modelled |
| `ledgerNowMs` (`opaque`), `time.now_ms` | Ledger time in ms (`time::now_ms` aborts on `u64` overflow) | An uninterpreted `Result U64` constant ([note](#storage-and-time)) |
| `storedParams`, `storedState` (`opaque`), `storage.read_params`, `storage.read_state` | Persistent reads of a market's params and state (aborting for a missing market) | Uninterpreted functions of the key ([note](#storage-and-time)) |
| `storage.renew_market` | Persistent TTL extension of a market | `ok ()` |
| `storage.write_state` | Persistent write of a market's state (`Cache::commit`) | `ok ()` ([note](#storage-and-time)) |
| `events.emit_market_value_ceiling` | Publication of `MarketValueCeilingEvent` (`interest::global_sync`, when the value ceiling held interest) | `ok ()` ([note](#events)) |
| `events.emit_market_state` | Publication of the market state snapshot (`ops::flash::finalize`, after the commit) | `ok ()` ([note](#events)) |

#### From u64 model

* `I128.Insts.CoreConvertFromU64.from` is `IScalar.tryMk .I128 x.val`,
  written like the `u32` model.
* It always fits (`u64::MAX < 2^127`). So the failure branch is dead, proved
  by `i128_from_u64_spec`.
* Aeneas maps `i128::from(u64)` to this name, which its Lean library does not
  define.
* It is reached only through `rates::compound::compound_interest`
  (`i128::from(delta_ms)`). `compound_interest_spec` and everything above it
  depend on it.

#### NonZero model

* `NonZero T Inner` is the bare `T`. The non-zero invariant is not in the
  type. But the only constructor in the model (`new`) yields `Some` only for
  a non-zero value, as in Rust.
* `NonZero.new` is modelled monomorphically at `u64`, the one instance
  reached (`NonZeroU64::new(remaining)` in `global_sync`). The generated
  generic call elaborates against it.

#### Storage and time

* `ledgerNowMs` is a Lean `opaque` constant: a definition with a hidden body,
  not an axiom. Theorems hold for every value, including failure.
* The leg theorems never inspect `storedParams` or `storedState`. They name
  the synced cache by the hypothesis `ops.synced_market env k = ok c`, and
  `load_ok` relates the loaded cache to the stored state.
* Modelling a read as a function of the key is sound for one leg, because
  each leg loads its market once.
* The written state is the committed cache, which the theorems describe
  (`commit_eq`: `commit c = ok (snapOf c)`). Nothing in a leg reads storage
  after its commit. Host write traps (resource limits) are not modelled.

#### Events

* An event writes only the host event buffer, which nothing in the extracted
  code reads.
* `emit_market_value_ceiling` leaves the cache unchanged
  (`global_sync_spec_of_inv`).
* The snapshot `emit_market_state` is given is the committed cache
  (`commit_eq`), which `flash_finalize_keeps_ceiling` describes.

The array iterator items are reached through `rates::compound`.
`compound_interest_spec` depends on them: the loop runs over the array's
seven divisors in order.

### Aeneas bug: duplicate parent-clause field

Affected: Aeneas `nightly-2026.11.12-0d21a9e`, bundled Charon `454586e`.

**Bug.** A trait can have two parent clauses for the *same* trait (here two
`Copy` bounds). Aeneas then gives both structure fields the same name, and
Lean rejects the structure.

* `core::num::nonzero::ZeroablePrimitive` is
  `trait ZeroablePrimitive: Sized + Copy + Sealed { type NonZeroInner: Sized + Copy; }`.
* Charon turns the associated type into a type parameter. So the trait has
  the parent clauses `Self: Copy` and `Self::NonZeroInner: Copy`, and Aeneas
  emits:

```lean
structure core.num.nonzero.ZeroablePrimitive (Self : Type _) (Self_NonZeroInner : Type _) where
  markerCopyInst : core.marker.Copy Self
  markerCopyInst : core.marker.Copy Self_NonZeroInner   -- error: already declared
```

* `global_sync` reaches it through `NonZeroU64::new`/`get`.
* The Charon options `--hide-marker-traits`, `--remove-unused-clauses`,
  `--opaque` and `--exclude` (on the trait) do not avoid it.

**Minimal reproduction** (for an upstream issue), independent of `core`:

```rust
pub trait Zeroable: Copy { type Inner: Copy; }
impl Zeroable for u64 { type Inner = u64; }
pub fn get<T: Zeroable>(x: T) -> T { x }
pub fn use_it(x: u64) -> u64 { get(x) }
```

* `charon cargo --preset=aeneas` then `aeneas -backend lean` produces
  `structure Zeroable (Self) (Self_Inner) where coremarkerCopyInst : core.marker.Copy Self;
  coremarkerCopyInst : core.marker.Copy Self_Inner` and an instance with the
  duplicated field.
* `lake env lean` reports
  ``Field `coremarkerCopyInst` has already been declared``.
* Expected: distinct field names (for example a numeric suffix, as Aeneas
  does for other name clashes).

**Workaround.** `pool/postextract.sh` runs after Aeneas and before the build
(`run.sh` calls it). It renames the *second* field to `markerCopyInst1` in
exactly three places:

* the structure declaration (`Types.lean`);
* its `parentClauses` attribute (`Types.lean`);
* the one instance, `u64`'s impl
  `U64.Insts.CoreNumNonzeroZeroablePrimitiveNonZeroU64Inner` (`Funs.lean`).

The rename preserves semantics:

* A field name is a label. Both fields keep their types, order and values.
* No generated function reads either field. They are `Copy` marker-trait
  instances with no computational content.
* The functions that use the trait, `NonZero::new`/`get`, are external models
  that ignore the instance argument.
* Each substitution must match exactly once or the script fails. So a
  changed Aeneas output is noticed.
* The production code stays verbatim.

### Pool copied proofs

The rounding specifications (`Spec`, `FloorCeilSpec`, `HalfUpSpec`) are not
copied. They live once in `lean/FixedPoint/` and this library imports them.

**Generated copies.** `Host`, `MulDiv`, `Scaling`, `FloorCeilCore`,
`FloorCeilRescale`, `FloorCeilRoundTrip`, `HalfUp` and `HalfUpDivInt` are the
files of the same name in `lean/RatesAeneas/Proofs/`, with the names changed:
`RatesAeneas`→`PoolAeneas`, `rates_aeneas`→`pool_aeneas`,
`aeneas_soroban_shim`→`aeneas_pool_shim`, `constants.production.`→`constants.`.
`../sync_copies.py` writes them, and `run.sh` fails if one is out of date.

**Hand-adapted copies.** `FloorCeilScaling`, `FloorCeilResolve` and
`Withdrawal` use the same renaming, each with a provenance comment.

* Together with the generated copies they supply `mul_div_floor_spec`,
  `mul_div_ceil_spec`, `mul_div_half_up_spec`,
  `mul_div_floor_saturating_spec`, `ray_mul_floor_spec`, `ray_mul_ceil_spec`,
  `ray_div_floor_spec`, `ray_div_ceil_spec`, `ray_to_asset_floor_spec`,
  `unscale_supply_floor_spec` and `from_asset_spec`.
* The three added for the legs supply `resolve_repay_spec`,
  `resolve_net_settle_spec` and `resolve_withdrawal_spec` (with
  `exists_isFloor`/`exists_isCeil`).
* `FloorCeilResolve` omits `i128_saturating_sub_val` and
  `protocol_fee_shares_spec`. The pool has its own in `Accrual.lean`.
* `Withdrawal` keeps only `resolve_withdrawal_spec` and its two arithmetic
  lemmas. It imports the pool's `RatesHalfUpWrappers`/`RatesIndex` and opens
  `PoolAeneas.RatesProofs` for `ray_mul_spec`/`ray_to_asset_spec`.

**Accrual copies.** `RatesHalfUpWrappers`, `RatesCurveBase`, `RatesCurve`,
`RatesCurveRates` and `RatesIndex` are copies of `HalfUpWrappers`,
`CurveBase`, `Curve`, `CurveRates` and `Index`.

* They use the same renaming plus `types.MarketParams`→`types.pool.MarketParams`.
* They are in the namespace `PoolAeneas.RatesProofs`, because several names
  clash with pool-local lemmas (for example `ray_checked_add_spec`,
  `ray_eq_eq`, `bps_val`).
* `RatesCurveRates` omits `calculate_deposit_rate_spec` and
  `range_contains_eq`. `calculate_deposit_rate` is not reachable from the
  pool roots.
* They supply `calculate_borrow_rate_spec`, `utilization_spec`,
  `update_borrow_index_spec`, `calculate_supplier_rewards_spec`,
  `ray_mul_spec` and the `Ray` `Ord` min/max specs used by
  `accrue_step_spec`.

**Pool-local differences in the copies:**

* `RatesHalfUpWrappers.lean` carries `ray_div_spec`, because `utilization`
  calls `Ray::div`.
* `RatesHalfUpWrappers.lean` omits `ray_try_div_spec`, because the pool
  extraction does not contain `Ray::try_div`. It also omits
  `ray_try_mul_spec`, because the pool's own copy is in `Wrappers.lean`.
* `RatesCurveBase.lean` carries `utilization_spec` (result in `[0, RAY]`)
  and `ray_ge_eq`.
* `FloorCeilScaling.lean` carries `ray_div_floor_saturating_spec`.
  `calculate_scaled_cap_spec` is proved through it, because
  `calculate_scaled_cap` calls `Ray::div_floor_saturating`.

### Pool rebuild

```sh
export AENEAS_HOME=/path/to/aeneas-nightly-2026.11.12-0d21a9e   # the pinned release
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas/pool
mkdir -p target/llbc
"$AENEAS_HOME/charon" cargo --preset=aeneas --dest-file target/llbc/pool_aeneas.llbc
"$AENEAS_HOME/aeneas" -backend lean -dest ../lean/PoolAeneas -split-files target/llbc/pool_aeneas.llbc
rm -f ../lean/PoolAeneas/*_Template.lean
./postextract.sh ../lean/PoolAeneas
cd ../lean
lake build PoolAeneas
```

* Charon reads the extraction roots and the `transfer_out` exclusion from
  `[package.metadata.charon]` in `pool/Cargo.toml`.
* After Aeneas, run `pool/postextract.sh ../lean/PoolAeneas` (`run.sh` does).
  It fixes the duplicate `ZeroablePrimitive` field.
* The templates Aeneas writes list the external declarations. Their meanings
  live in `FunsExternal.lean` and `TypesExternal.lean`. So after a
  re-extraction, diff the templates against those files before you delete
  them.
