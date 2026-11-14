# Extraction crates, trusted models and Aeneas workarounds

How the controller and pool proofs get their Lean code. Theorems:
[controller.md](controller.md), [pool.md](pool.md). Pinned tool versions and
shared trust assumptions (faithful translation, panic = abort):
[../README.md](../README.md#trusted-base). Neither library copies the
rounding specifications (`Spec`, `FloorCeilSpec`, `HalfUpSpec`); both import
them from `lean/FixedPoint/`.

## Controller

### Controller extraction crate

`verification/aeneas/controller/`: standalone Cargo workspace (`[workspace]`
table, members `.` and `shim`), outside the root workspace and every Wasm
build.

**Included verbatim** through `#[path]`:

* `common/src/constants/` (`src/constants.rs`) and
  `contracts/controller/src/constants.rs`, re-exported together so both
  `common::constants::X` and `crate::constants::BAD_DEBT_USD_THRESHOLD`
  resolve.
* `common/src/math/{fp,fp_core}.rs` (`src/math.rs`). `fp_core` stays private,
  as in `common/src/math/mod.rs`, so controller files reach it only through
  the typed `fp` wrappers.
* `common/src/rates/{scaling,value}.rs` (`src/rates.rs`, with the `pub use`
  lines of `rates/mod.rs`); `curve`, `index`, `compound`, `simulate` unneeded.
* `contracts/controller/src/positions/liquidation/{curve,math,apply}.rs`
  (`src/positions/liquidation/mod.rs`).

**Generated from production.** `controller/gen_totals.py` (run by
`extract.sh`) copies `contracts/controller/src/risk/totals.rs` to
`src/risk/totals.rs`, replacing the `fn(&Env, Ray, Ray, Wad) -> Wad`
argument of `sum_debt_usd_loaded` with an enum `DebtValue { HalfUp, Ceil }`
whose `apply` calls exactly `position_value` / `position_value_ceil` (the two
functions production passes); Aeneas cannot translate higher-ranked `fn`
pointers ([issue 2](#controller-aeneas-issues)). Each of the four
substitutions must match exactly once, so a production edit there fails the
extraction.

`src/lib.rs` declares `extern crate self as common;` so `common::...` paths
resolve to this crate.

**Stand-ins**, each never extracted or a field-for-field copy:

* `src/types.rs`: `SpokeConfig`, `HubAssetKey` (derives `PartialEq, Eq` as in
  production), `PriceFeed(Raw)`, `MarketIndex(Raw)`, `AccountPosition(Raw)`,
  `DebtPosition(Raw)`, `PaymentTuple`, `SeizeEntry`, `RepayEntry`,
  `LiquidationResult`, `Account` without `mode`, all with production fields
  and verbatim `From` conversions (production's `#[contracttype]` only adds
  XDR serialisation). Also `AccountPositionType`, `AggregatedPayments`,
  `ScaledPositionRaw` (two `From` impls), `PoolAction`, `PoolWithdrawEntry`,
  `PoolSeizeEntry`, `AssetConfig`, `Account::get_or_create_supply_position`
  (verbatim) and a field-less `PoolPositionMutation`.
* `src/errors.rs`: the raised variants with production codes; codes are not
  modelled, every `panic_with_error!` aborts.
* `src/validation.rs`: `expect_invariant`, `require_nonneg_amount`,
  `require_non_empty_payments`, verbatim.
* `src/payment_legs.rs` (re-exported as `crate::payments`): `ZeroLeg`,
  `aggregate_positive_payments`, `aggregate_payments`,
  `aggregate_payment_amount`, verbatim from
  `contracts/controller/src/payments.rs`; `transfer_amount_measured` opaque.
* `src/context.rs`: the [Context model](#controller-trusted-base).
* `src/storage.rs`: `iter_typed_positions` / `iter_debt_positions`.
  Production's `map.iter().map(|(k, raw)| (k, From::from(&raw)))` becomes a
  loop pushing the same items in order and returning the vector's iterator
  (Aeneas translates neither `impl Iterator` returns nor `map`).
* Opaque (signatures only, Charon `--opaque`, meaning in
  `FunsExternal.lean`):
  * `account_store.rs` (re-exported as `crate::account`):
    `cleanup_account_if_empty`, `update_or_remove_supply_position`;
  * `external/{mod,pool}.rs`: `fetch_market_index`, `pool_seize_positions_call`;
  * `positions/mod.rs`: `enforce_spoke_asset_flags`, `apply_repay_batch`,
    `apply_withdraw_batch` (`make_pool_action`, `get_supply_position_or_panic`,
    `FreezePolicy`, `WithdrawKind` verbatim);
  * `positions/liquidation/bad_debt.rs`: `execute_bad_debt_cleanup`.
* Never extracted (signatures for compilation): `risk/validation.rs`,
  `collections.rs` (`push_unique_address`, verbatim), `events.rs`,
  `spoke_usage.rs` (enums only).

`../check_standins.py`, run first by `run.sh`, compares the copied items in
`controller/src` (`types.rs`, `cached_price`, payment aggregation,
`make_pool_action`, ...) with production. An adapted stand-in pins the hash
of the item it replaces.

**Scripts.** `controller/extract.sh` lists the roots (`--start-from`) and
opaque items (`--opaque`) and passes `--inline-anon-consts`
([issue 1](#controller-aeneas-issues)). `controller/postextract.sh` applies
one fix to `Funs.lean` ([issue 3](#controller-aeneas-issues)). Scratch runs:
`AENEAS_LEAN_DEST=<dir>` redirects output, `KEEP_TEMPLATES=1` keeps
`*_Template.lean`.

**Shim** (`controller/shim/`, package `aeneas-soroban-shim`, extended from
`verification/aeneas/shim`): `Env`, `I256`, `Error`, `panic_with_error!` as
in the rates crate, plus `assert_with_error!`, `Address`, `Vec<T>` (with
`VecIter<T>`, `IntoIterator`) and `Map<K, V>`. Charon translates only shim
signatures; their meaning is in the trusted base.

### Controller trusted base

Beyond the [shared assumptions](../README.md#trusted-base), the proofs trust
the models in `lean/ControllerAeneas/TypesExternal.lean` and
`FunsExternal.lean`:

* **`I256`** and Rust core (`checked_pow`, `abs_diff`,
  `Option::{unwrap_or_else, and_then}`, `?` on `Option`): copied from
  `RatesAeneas`; exact integers with the host's 256-bit traps.
* **`i128::from(u32)`**: same number (always fits).
* **`Address`**: opaque handle (`Nat`); `clone` is identity; equality of
  handles.
* **`Vec<T>`**: Lean `List T`, index 0 first.
  * `Vec::new` = `[]`; `push_back v x` = `v ++ [x]`.
  * `Vec::iter` yields front to back; `VecIter T` is the not-yet-yielded
    list; `next` pops the head, `None` when empty.
  * `clone` returns the same elements (soroban-sdk `Vec` is a host object
    with value semantics: `clone` copies the handle, mutations rebind).
  * `len` (as `u32`), `is_empty`, `get(i)` (`None` out of range), `set(i, x)`
    (traps out of range), `remove(i)` (shifts later elements down; `None`
    and unchanged out of range), `append`, `into_iter` (front to back).
  * Not modelled: the host's `u32::MAX` length limit; `push_back` never
    fails. The only theorem that pushes builds an output no longer than its
    input.
* **Map model** (`Map<K, V>`): Lean `List (K × V)` in iteration order.
  * `get k`: value of the first entry with key `k` (compared by value,
    decided classically); `set k v` replaces it or appends `(k, v)`;
    `keys`/`iter`/`len`/`is_empty` read the list.
  * Proofs rely only on `get` after the list is given and on `iter`/`keys`
    yielding the list.
  * Not modelled: the host's ascending key order and key uniqueness.
    Theorems quantify over all entry lists, so they cover sorted, unique
    ones.
* **Rust core**, per the Rust docs: `i128::saturating_mul` (exact product
  clamped to `i128`), `Option::map_or`, `Iterator::{all, find}` (front to
  back, stopping at the first `false` / match).
* **Context model** (`src/context.rs`): the two caches the extracted code
  reads, `token_prices` (`Map<Address, PriceFeedRaw>`) and `market_indexes`
  (`Map<HubAssetKey, MarketIndexRaw>`).
  * `cached_price`: production body; aborts when the asset is uncached.
  * `cached_market_index`: production body, one-key pool fetch as one call.
  * Uninterpreted Lean constants (`opaque` definitions, not axioms) stand for
    the rest of the host state, so theorems hold for every value:
    `poolIndex` (pool's index for a key, used by `fetch_market_index`),
    `oraclePrice` (`None`: the fetch aborts), `poolAddress`,
    `spokeAssetConfig`, `spokeExitOk`, `spokeFlagsAllow`, `transferReceived`
    (measured balance change; any token behaviour).
  * `load_markets` caches each key's missing asset price (aborting if the
    oracle has none) and missing index, keeping cached entries; production
    batches the same fetches.
  * `cached_pool_address`, `require_spoke_asset`, `apply_spoke_exit`,
    `record_supply_position_update`, `enforce_spoke_asset_flags`: caches
    unchanged, abort where production can. In production they touch only
    spoke usage, configuration caches, event buffers and the pool-address
    cache.
  * `transfer_amount_measured` aborts unless `amount > 0`; returns
    `transferReceived`.
  * `apply_repay_batch`, `apply_withdraw_batch`, `pool_seize_positions_call`,
    `cleanup_account_if_empty`, `update_or_remove_supply_position`,
    `execute_bad_debt_cleanup`, `repays_every_debt_leg`: uninterpreted. A
    caller's theorem either ignores them or states the call as an equation.

### Controller copied proofs

Each extraction has its own copy of the common math, so its proofs exist
once per library.

* `Host`, `MulDiv` (`mul_div_floor_spec`), `FloorCeilRescale`
  (`rescale_floor`/`rescale_ceil`), `HalfUp` (`mul_div_half_up_spec`),
  `HalfUpDivInt` (`div_by_int_half_up`): the RatesAeneas files with library,
  crate and shim names changed, written by `../sync_copies.py`; `run.sh`
  fails if one is stale.
* `FloorCeilCore` (`mul_div_ceil_spec`): hand-adapted, without the
  unextracted `mul_div_floor_saturating` section, which is
  `Saturating.lean` (copied from `RatesAeneas/Proofs/FloorCeilCore.lean`,
  per its header).
* `Scaling.lean`: `RatesAeneas/Proofs/FloorCeilScaling.lean` up to
  `unscale_borrow_ceil_spec`, without `calculate_scaled_*`;
  `unscale_supply_floor_spec` renamed `unscale_supply_floor_isFloor` because
  `Seizure.lean` has its own.

### Controller rebuild

```sh
export AENEAS_HOME=/path/to/aeneas-nightly-2026.11.12-0d21a9e   # or your unpacked release
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas
ALLOW_DRIFT=1 ./run.sh controller   # extract.sh, then postextract.sh, then lake build ControllerAeneas
```

`extract.sh`:

1. Runs `gen_totals.py`.
2. Runs `charon cargo --preset=aeneas --inline-anon-consts --dest-file
   target/llbc/controller.llbc --start-from ... --opaque ...`.
3. Runs `aeneas -backend lean -dest ../lean -subdir ControllerAeneas
   -split-files target/llbc/controller.llbc` (`-subdir` makes imports
   `ControllerAeneas.*`).
4. Deletes `*_Template.lean`.

`run.sh` then runs `postextract.sh` ([issue 3](#controller-aeneas-issues)).

### Controller Aeneas issues

1. **Internal error on `process_excess_payment`** (`InterpAbs.ml` line 940,
   reported as `Interp.ml` line 686): a loop condition comparing a
   derived-`PartialOrd` newtype against a temporary, short-circuiting.
   Minimal reproduction:

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

   Production's condition: `remaining_excess_usd > Wad::ZERO &&
   current_index > 0`. Charon's `--inline-anon-consts` avoids it
   (`--no-code-duplication` too, but breaks other functions); `extract.sh`
   passes it, and diffing `Funs.lean` shows every other function's
   translation unchanged.
2. **Higher-ranked `fn` pointers unsupported** ("arrow types with locally
   quantified regions"), nor coercing a function item to one. `gen_totals.py`
   avoids it ([crate section](#controller-extraction-crate)). A generic
   `F: Fn(&Env, ..)` for `position_value` fails too ("lifetime constraint
   relating a higher-ranked lifetime to a free lifetime").
3. **Provided `Iterator` methods emitted as instance fields.** For the shim's
   `impl Iterator for VecIter<T>`, Aeneas puts `all` and `find` in the trait
   instance; the Lean library's `Iterator` has no such fields (Aeneas warns),
   so `Funs.lean` does not compile. Calls go to
   `core.iter.traits.iterator.Iterator.{all,find}.default` directly, so
   `controller/postextract.sh` deletes exactly these two fields from that one
   instance, failing on any other shape.
4. **Parameter names shadow module namespaces.** A function with parameter
   `account` (or `payments`) calling `crate::account::f` is emitted as
   `account.f`, which Lean reads as field access. The crate defines those
   items in `account_store` / `payment_legs`, re-exported as `crate::account`
   / `crate::payments`, so generated names differ from the parameters.
5. **Closures with a mutable capture.** `repays_every_debt_leg`'s
   `Iterator::all` closure captures `cache: &mut Context`; the generated
   `FnMut` instance has type `closure → Item → Result (Bool × closure ×
   (closure → closure))`, not the library's `FnMut`. Kept opaque.

## Pool

### Pool extraction crate

`verification/aeneas/pool/`: standalone Cargo workspace (own `[workspace]`
table), outside the root workspace and every Wasm build.

| Path | Contents |
|---|---|
| `pool/Cargo.toml` | Package `pool-aeneas`; `soroban-sdk` renamed to the shim; `[package.metadata.charon]` lists the roots |
| `pool/src/lib.rs` | Crate root; `#[path]` includes; `extern crate self as common` |
| `pool/src/{types,oracle,ownable,storage,time,events}.rs` | [Stand-ins](#pool-stand-ins-and-shim) |
| `pool/shim/` | `aeneas-pool-shim`: signature-only `soroban-sdk` stand-in (extended from `../shim`) |
| `pool/shim-macros/` | `aeneas-pool-shim-macros`: identity `#[contracttype]` / `#[contracterror]` |

**Included verbatim** (`#[path]`):

* From `common/src`: `constants/mod.rs` (with `pool.rs`, `shared.rs`),
  `errors.rs`, `math/mod.rs` (`fp.rs`, `fp_core.rs`), `rates/mod.rs` (all six
  submodules), `validation.rs`, `ttl.rs`, `types/pool.rs`, `types/shared.rs`.
* From `contracts/pool/src`: `cache/mod.rs` (with `cache/cash.rs`,
  `cache/report.rs`, `cache/scale.rs`, `cache/shares.rs`), `guards.rs`,
  `interest.rs`, `ops/mod.rs` (so `synced_market`, `renewed_market`,
  `load_leg` are production) with every leg: `borrow.rs`, `flash.rs`,
  `market.rs`, `net_settle.rs`, `recapitalize.rs`, `repay.rs`, `revenue.rs`,
  `seize.rs`, `strategy.rs`, `supply.rs`, `withdraw.rs`.
* `revenue.rs` imports `stellar_access::ownable`, resolved by a second
  `extern crate self as stellar_access` to the `ownable` stand-in.

**Layout.** The `common` modules sit at the root where `common/src/lib.rs`
puts them, so their `crate::` paths resolve; `extern crate self as common`
resolves the pool files' `common::...` paths to them. The cache module is
mounted as `market_cache` and re-exported with `use market_cache as cache`,
so `crate::cache::Cache` and `super::Cache` resolve unchanged. The extra name
avoids an Aeneas clash: generated Lean refers to `cache.Cache.…` relative to
the crate namespace, and a parameter named `cache` (in `guards.rs`,
`interest.rs`, `flash.rs`) shadows it.

### Pool extraction roots

Charon translates only what these reach (`start_from` in `pool/Cargo.toml`):

* `crate::market_cache::shares`, `crate::market_cache::cash` (minus
  `transfer_out`, which calls the token client);
* `crate::guards`;
* `crate::interest::{global_sync, accrue_chunk, add_protocol_revenue, apply_bad_debt_to_supply_index}`
  (`global_sync` needs the `postextract.sh`
  [workaround](#aeneas-bug-duplicate-parent-clause-field));
* `crate::ops::flash::{terms, book_fee, finalize}` (`finalize`: the flash
  loan's tail after the balance checks: fee booking, entry ceiling, commit);
* `crate::rates::scaling`, so the copied RatesAeneas scaling proofs find
  every function they name;
* each leg at its token-free function: `ops::supply::apply`,
  `ops::borrow::accounting`, `ops::withdraw::accounting`,
  `ops::repay::accounting`, `ops::net_settle::apply`, `ops::seize::apply`,
  `ops::recapitalize::accounting`, `ops::strategy::accounting`,
  `ops::revenue::accounting`.

`opaque = ["crate::storage", "crate::time", "crate::events"]`: signatures
only, meaning in `FunsExternal.lean` ([trusted base](#pool-trusted-base)).

Reached through the legs: `Cache::load` (storage reads, `now_ms`),
`Cache::commit` (storage write), `ttl::renew_instance`, the full
`global_sync`, and two events: `events::emit_market_value_ceiling`
(`global_sync`) and `events::emit_market_state` (`flash::finalize`).

Not reached: token transfers (`Cache::transfer_out`, excluded, and the
`apply` wrappers of borrow/withdraw/repay/recapitalize/strategy/revenue);
other events; the flash-loan callback and balance checks (`flash::apply`);
`run_batch`, `market.rs` and the entry points (`lib.rs`, so also
`get_sync_data`).

### Pool stand-ins and shim

| File | Stands in for | Reaches Lean? |
|---|---|---|
| `src/types.rs` | `common::types` | Yes (types only) |
| `src/oracle.rs` | `common::oracle::observation` | No |
| `src/storage.rs` | `contracts/pool/src/storage.rs` | Signatures only (`opaque`) |
| `src/time.rs` | `contracts/pool/src/time.rs` | Signature only (`opaque`) |
| `src/events.rs` | `contracts/pool/src/events.rs` | Signatures only (`opaque`) |
| `src/ownable.rs` | `stellar_access::ownable` | No |

* `src/types.rs`: `types/pool.rs` and `types/shared.rs` verbatim, plus the
  `OracleTolerance` record (fields only) that `validation.rs` names in an
  unreached function; controller/oracle type files left out.
* `src/oracle.rs`: the three constants `validation.rs` imports, production
  values.
* `src/storage.rs`: production signatures, `unimplemented!()` bodies (never
  run). Reached: `read_params`, `read_state`, `write_state`, `renew_market`.
  `market_exists`, `write_params`, `write_rate_model`, `load_state`,
  `load_sync_data` serve only `market.rs` / type-checking.
* `src/time.rs`: `now_ms`.
* `src/events.rs`: five emitter signatures; two reached
  ([above](#pool-extraction-roots)), the other three called only by the
  `apply` wrappers, `run_batch` and `market.rs`.
* `src/ownable.rs`: `get_owner`, for `ops::revenue::apply` (not a root).

Shim items (`pool/shim`, `pool/shim-macros`) are signature-only. Those that
reach Lean get a meaning in the trusted base; the rest only type-check the
verbatim files.

| Shim item | Reaches Lean | Notes |
|---|---|---|
| `Env` | Yes | Unit struct; `Unit` in `Types.lean` |
| `I256` and its 7 methods, `PartialEq`, `PartialOrd` | Yes | As in the rates shim |
| `Address` | Yes (type only) | Stored in `Cache` (`hub_asset`, `params.asset_id`), never inspected |
| `panic_with_error!`, `assert_with_error!` | — | `panic!` / `if !cond { panic!(..) }`: abort |
| `contracttype`, `contracterror` | — | Identity (the real ones derive host conversions the arithmetic never calls) |
| `Env: Clone`, `Address: Clone` | Yes | `Cache::load` clones env and market key |
| `Storage`, `Instance`, `Env::storage`, `Storage::instance`, `Instance::extend_ttl` | Yes | For verbatim `common/src/ttl.rs` (`renew_instance`, called by `ops::renewed_market`) |
| Type-check-only (below) | No | Compile the verbatim files |

Type-check-only: `Error`, `Bytes`, `BytesN`, `Symbol`, `String`, `Val`,
`IntoVal`, `TryFromVal`, `Vec` (`new`, `is_empty`, `iter`, `contains`,
`push_back`), `Executable`, `Address::executable`,
`Env::{current_contract_address, invoke_contract}`, `token::Client` (incl.
`try_transfer`). They serve `validation.rs`, `cash.rs::transfer_out`, the
token/callback half of `flash.rs`, `ops::run_batch` and `withdraw::apply`;
`TryFromVal`, the three `Vec` methods and `try_transfer` serve `ops/mod.rs` /
`withdraw.rs`.

### Pool trusted base

Beyond the [shared assumptions](../README.md#trusted-base), the proofs trust
`lean/PoolAeneas/TypesExternal.lean` and `FunsExternal.lean`. "RA" marks a
model copied from RatesAeneas.

| Item | Models | Definition |
|---|---|---|
| `aeneas_pool_shim.I256` | Host 256-bit integer | `Int` (RA) |
| `I256.{from_i128, to_i128, add, sub, mul, div, rem_euclid, eq, partial_cmp}`, `MIN`, `MAX`, `checked` | Host I256 arithmetic | Exact, trapping outside `[-2^255, 2^255)`; truncating `div`, Euclidean `rem_euclid`, zero divisor traps (RA) |
| `core.num.I128.checked_pow`, `core.num.U32.abs_diff`, `Option::{unwrap_or_else, map_or, and_then}`, `?` on `Option`, `Range::contains` | Rust core | Per the Rust docs (RA) |
| `aeneas_pool_shim.Address` | `soroban_sdk::Address` | `Nat` handle (as in the controller); only stored and cloned |
| `core.array.iter.IntoIter T N` | `core::array::IntoIter` | `List T`, not yet yielded |
| `Array…IntoIterator.into_iter` | `<[T; N]>::into_iter` | Elements in order |
| `IntoIter…Iterator.next` | `<array::IntoIter>::next` | Head and tail; `None` when exhausted |
| `I128.Insts.CoreConvertFromU32.from` | `impl From<u32> for i128` | Same integer (always fits) |
| `I128.Insts.CoreConvertFromU64.from` | `impl From<u64> for i128` | Same integer ([note](#from-u64-model)) |
| `core.num.niche_types.NonZeroU64Inner` (type) | `core::num::niche_types::NonZeroU64Inner` | `Std.U64`, the wrapped value; never inspected |
| `core.num.nonzero.NonZero T Inner` (type) | `core::num::NonZero<T>` | Bare `T` ([note](#nonzero-model)) |
| `core.num.nonzero.NonZero.new` | `NonZero::<u64>::new` | `ok (if x = 0 then none else some x)`; never panics |
| `core.num.nonzero.NonZero.get` | `NonZero::<u64>::get` | `ok x` |
| `NonZeroU64Inner.Insts.CoreCloneClone.clone` | `<NonZeroU64Inner as Clone>::clone` | `ok x`; only in the generated `Copy` instance |
| `pool/postextract.sh` | Fix of generated code | Renames the duplicated `ZeroablePrimitive` field ([bug](#aeneas-bug-duplicate-parent-clause-field)) |
| `aeneas_pool_shim.Storage`, `aeneas_pool_shim.Instance` (types) | Storage handles | `Unit` (only TTL extension is called) |
| `Env.Insts.CoreCloneClone.clone`, `Address.Insts.CoreCloneClone.clone` | `Clone` of env / address | `ok` of the same value |
| `Env.storage`, `Storage.instance` | Handle accessors | `ok ()` |
| `Instance.extend_ttl` | Instance TTL (`common::ttl::renew_instance`) | `ok ()`; TTL and host resource traps not modelled |
| `ledgerNowMs` (`opaque`), `time.now_ms` | Ledger ms (`time::now_ms` aborts on `u64` overflow) | Uninterpreted `Result U64` ([note](#storage-and-time)) |
| `storedParams`, `storedState` (`opaque`), `storage.read_params`, `storage.read_state` | Market params/state reads (abort for a missing market) | Uninterpreted functions of the key ([note](#storage-and-time)) |
| `storage.renew_market` | Market TTL extension | `ok ()` |
| `storage.write_state` | Market state write (`Cache::commit`) | `ok ()` ([note](#storage-and-time)) |
| `events.emit_market_value_ceiling` | `MarketValueCeilingEvent` (`interest::global_sync`, when the ceiling held interest) | `ok ()` ([note](#events)) |
| `events.emit_market_state` | Market state snapshot (`ops::flash::finalize`, after commit) | `ok ()` ([note](#events)) |

The array iterator items are reached through `rates::compound`:
`compound_interest_spec` loops over the array's seven divisors in order.

#### From u64 model

`I128.Insts.CoreConvertFromU64.from` is `IScalar.tryMk .I128 x.val`, like the
`u32` model. It always fits (`u64::MAX < 2^127`), so the failure branch is
dead (`i128_from_u64_spec`). Aeneas maps `i128::from(u64)` to this name,
which its Lean library lacks. Reached only via
`rates::compound::compound_interest` (`i128::from(delta_ms)`), so
`compound_interest_spec` and everything above it depend on it.

#### NonZero model

`NonZero T Inner` is the bare `T`: the non-zero invariant is not in the
type, but the model's only constructor (`new`) yields `Some` only for a
non-zero value, as in Rust. `NonZero.new` is modelled at `u64`, the one
instance reached (`NonZeroU64::new(remaining)` in `global_sync`); the
generic call elaborates against it.

#### Storage and time

* `ledgerNowMs` is a Lean `opaque` constant (not an axiom); theorems hold for
  every value, including failure.
* Leg theorems never inspect `storedParams`/`storedState`: they name the
  synced cache by `ops.synced_market env k = ok c`, and `load_ok` relates
  the loaded cache to stored state.
* A read as a function of the key is sound per leg: each leg loads its
  market once.
* The written state is the committed cache (`commit_eq`:
  `commit c = ok (snapOf c)`); no leg reads storage after its commit. Host
  write traps (resource limits) are not modelled.

#### Events

An event writes only the host event buffer, which no extracted code reads.
`emit_market_value_ceiling` leaves the cache unchanged
(`global_sync_spec_of_inv`). `emit_market_state` receives the committed
cache (`commit_eq`), described by `flash_finalize_keeps_ceiling`.

### Aeneas bug: duplicate parent-clause field

Affected: Aeneas `nightly-2026.11.12-0d21a9e`, bundled Charon `454586e`.

**Bug.** A trait with two parent clauses for the *same* trait (here two
`Copy` bounds) gets two structure fields of the same name, which Lean
rejects. `core::num::nonzero::ZeroablePrimitive` is
`trait ZeroablePrimitive: Sized + Copy + Sealed { type NonZeroInner: Sized + Copy; }`;
Charon turns the associated type into a parameter, giving parent clauses
`Self: Copy` and `Self::NonZeroInner: Copy`, and Aeneas emits:

```lean
structure core.num.nonzero.ZeroablePrimitive (Self : Type _) (Self_NonZeroInner : Type _) where
  markerCopyInst : core.marker.Copy Self
  markerCopyInst : core.marker.Copy Self_NonZeroInner   -- error: already declared
```

`global_sync` reaches it through `NonZeroU64::new`/`get`. Charon's
`--hide-marker-traits`, `--remove-unused-clauses`, `--opaque` and
`--exclude` (on the trait) do not avoid it.

**Minimal reproduction** (for an upstream issue), independent of `core`:

```rust
pub trait Zeroable: Copy { type Inner: Copy; }
impl Zeroable for u64 { type Inner = u64; }
pub fn get<T: Zeroable>(x: T) -> T { x }
pub fn use_it(x: u64) -> u64 { get(x) }
```

`charon cargo --preset=aeneas` then `aeneas -backend lean` produce
`structure Zeroable (Self) (Self_Inner) where coremarkerCopyInst : core.marker.Copy Self;
coremarkerCopyInst : core.marker.Copy Self_Inner` and an instance with the
duplicated field; `lake env lean` reports
``Field `coremarkerCopyInst` has already been declared``. Expected: distinct
names (e.g. a numeric suffix, as Aeneas does for other clashes).

**Workaround.** `pool/postextract.sh` (run by `run.sh` after Aeneas, before
the build) renames the *second* field to `markerCopyInst1` in exactly three
places: the structure declaration and its `parentClauses` attribute
(`Types.lean`), and `u64`'s instance
`U64.Insts.CoreNumNonzeroZeroablePrimitiveNonZeroU64Inner` (`Funs.lean`).
Semantics are preserved: field names are labels (types, order and values
unchanged); no generated function reads either field (`Copy` marker
instances, no computational content); `NonZero::new`/`get` are external
models that ignore the instance. Each substitution must match exactly once
or the script fails, so changed Aeneas output is noticed. Production code
stays verbatim.

### Pool copied proofs

**Generated copies.** `Host`, `MulDiv`, `Scaling`, `FloorCeilCore`,
`FloorCeilRescale`, `FloorCeilRoundTrip`, `HalfUp`, `HalfUpDivInt`: the
same-named files in `lean/RatesAeneas/Proofs/`, renamed
`RatesAeneas`→`PoolAeneas`, `rates_aeneas`→`pool_aeneas`,
`aeneas_soroban_shim`→`aeneas_pool_shim`, `constants.production.`→`constants.`,
written by `../sync_copies.py`; `run.sh` fails if one is stale.

**Hand-adapted copies** (same renaming, provenance comment each):
`FloorCeilScaling`, `FloorCeilResolve`, `Withdrawal`.

* With the generated copies they supply `mul_div_floor_spec`,
  `mul_div_ceil_spec`, `mul_div_half_up_spec`,
  `mul_div_floor_saturating_spec`, `ray_mul_floor_spec`, `ray_mul_ceil_spec`,
  `ray_div_floor_spec`, `ray_div_ceil_spec`, `ray_to_asset_floor_spec`,
  `unscale_supply_floor_spec`, `from_asset_spec`; for the legs,
  `resolve_repay_spec`, `resolve_net_settle_spec` and
  `resolve_withdrawal_spec` (with `exists_isFloor`/`exists_isCeil`).
* `FloorCeilResolve` omits `i128_saturating_sub_val` and
  `protocol_fee_shares_spec` (the pool's own are in `Accrual.lean`).
* `Withdrawal` keeps only `resolve_withdrawal_spec` and its two arithmetic
  lemmas; it imports the pool's `RatesHalfUpWrappers`/`RatesIndex` and opens
  `PoolAeneas.RatesProofs` for `ray_mul_spec`/`ray_to_asset_spec`.

**Accrual copies.** `RatesHalfUpWrappers`, `RatesCurveBase`, `RatesCurve`,
`RatesCurveRates`, `RatesIndex` copy `HalfUpWrappers`, `CurveBase`, `Curve`,
`CurveRates`, `Index`, with the same renaming plus
`types.MarketParams`→`types.pool.MarketParams`, in namespace
`PoolAeneas.RatesProofs` because names clash with pool lemmas (e.g.
`ray_checked_add_spec`, `ray_eq_eq`, `bps_val`). They supply
`calculate_borrow_rate_spec`, `utilization_spec`, `update_borrow_index_spec`,
`calculate_supplier_rewards_spec`, `ray_mul_spec` and the `Ray` `Ord`
min/max specs `accrue_step_spec` uses. `RatesCurveRates` omits
`calculate_deposit_rate_spec` and `range_contains_eq`
(`calculate_deposit_rate` is unreachable from the pool roots).

**Pool-local differences:**

* `RatesHalfUpWrappers.lean` adds `ray_div_spec` (`utilization` calls
  `Ray::div`); omits `ray_try_div_spec` (`Ray::try_div` not extracted) and
  `ray_try_mul_spec` (the pool's copy is in `Wrappers.lean`).
* `RatesCurveBase.lean` adds `utilization_spec` (result in `[0, RAY]`) and
  `ray_ge_eq`.
* `FloorCeilScaling.lean` adds `ray_div_floor_saturating_spec`, through which
  `calculate_scaled_cap_spec` is proved (`calculate_scaled_cap` calls
  `Ray::div_floor_saturating`).

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

Charon reads the roots and the `transfer_out` exclusion from
`[package.metadata.charon]` in `pool/Cargo.toml`. The `*_Template.lean`
files list the external declarations whose meanings live in
`FunsExternal.lean` and `TypesExternal.lean`; after a re-extraction, diff
them against those files before deleting them.
