# Pool accounting: Aeneas + Lean proofs

Machine-checked proofs about the production pool accounting code in
`contracts/pool/src`. They cover:

* the share-total mints and burns, and the revenue claim;
* the solvency and utilization guards;
* cash bookkeeping, protocol revenue booking and bad-debt socialization;
* the flash-loan fee arithmetic;
* interest accrual (`compound_interest`, `accrue_step`, `accrue_chunk`,
  `global_sync`);
* the operation legs of `contracts/pool/src/ops/` (supply, borrow, withdraw,
  repay, net settlement, seize, recapitalize, strategy, revenue claim), and
  the Certora pool summaries they discharge.

Charon + Aeneas (pinned in `../README.md`) translate the Rust to Lean. Every
theorem is stated about the generated `lean/PoolAeneas/Funs.lean`.

Every headline theorem depends only on `propext`, `Classical.choice` and
`Quot.sound` (checked with `#print axioms`, see "Axiom check"). There is no
`sorry`, `admit` or `axiom` in `lean/PoolAeneas/`.

## Conventions

* `f ⦃ r => P r ⦄` is total correctness: `f` returns (does not abort) and
  `P` holds. `(∃ u, f = ok u) ↔ C` is an exact pass/fail condition: the
  guard returns iff `C`, and aborts otherwise. `f = ok c' → P c'` says what
  holds after every successful return, whatever the input.
* Specifications never divide. `IsFloor n d q` is `q*d ≤ n < q*d + d`,
  `IsCeil n d q` is `n ≤ q*d < n + d`, `IsHalfUp n d q` is
  `2qd ≤ 2n + d < 2qd + 2d` (all for `d > 0`).
* `RAY = 10^27`, `dec` is the asset's decimals, `k = 27 - dec`. Values written
  `x.val` are the integers inside `i128`/`Ray`.
* Error *codes* are not modelled: `panic_with_error!`/`assert_with_error!`
  abort, which Aeneas translates to failure.

## The extraction crate (`verification/aeneas/pool/`)

A standalone Cargo workspace (own `[workspace]` table), never part of the
root workspace or any Wasm build.

| Path | Contents |
|---|---|
| `pool/Cargo.toml` | Package `pool-aeneas`; `soroban-sdk` renamed to the shim; `[package.metadata.charon]` lists the extraction roots |
| `pool/src/lib.rs` | Crate root; `#[path]` includes; `extern crate self as common` |
| `pool/src/{types,oracle,ownable,storage,time,events}.rs` | Stand-ins (below) |
| `pool/shim/` | `aeneas-pool-shim`: signature-only `soroban-sdk` stand-in (copied from `../shim`, extended) |
| `pool/shim-macros/` | `aeneas-pool-shim-macros`: identity `#[contracttype]` / `#[contracterror]` |

### Included verbatim (`#[path]`)

From `common/src`: `constants/mod.rs` (with `pool.rs`, `shared.rs`),
`errors.rs`, `math/mod.rs` (`fp.rs`, `fp_core.rs`), `rates/mod.rs` (all six
submodules), `validation.rs`, `ttl.rs`, `types/pool.rs`, `types/shared.rs`.

From `contracts/pool/src`: `cache/mod.rs` (and through it `cache/cash.rs`,
`cache/report.rs`, `cache/scale.rs`, `cache/shares.rs`), `guards.rs`,
`interest.rs`, `ops/mod.rs` (and through it every leg: `borrow.rs`,
`flash.rs`, `market.rs`, `net_settle.rs`, `recapitalize.rs`, `repay.rs`,
`revenue.rs`, `seize.rs`, `strategy.rs`, `supply.rs`, `withdraw.rs`).
`ops/mod.rs` is included verbatim, so `synced_market`, `renewed_market` and
`load_leg` are the production code.
`revenue.rs` imports `stellar_access::ownable`, which resolves through a
second `extern crate self as stellar_access` to the `ownable` stand-in.

Nothing in `common/` or `contracts/` is copied or edited.

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
`cache` (in `guards.rs`, `interest.rs`, `flash.rs`) shadows that name, so an
extraction under the name `cache` does not compile.

### Extraction roots

Charon translates only what these items reach (`start_from` in
`pool/Cargo.toml`):

* `crate::market_cache::shares`, `crate::market_cache::cash` (minus
  `transfer_out`, excluded: it calls the token client),
* `crate::guards`,
* `crate::interest::{global_sync, accrue_chunk, add_protocol_revenue, apply_bad_debt_to_supply_index}`
  (`global_sync` needs the `postextract.sh` workaround, see
  "Aeneas bug: duplicate parent-clause field"),
* `crate::ops::flash::{terms, book_fee, finalize}` (`finalize` is the tail of
  a flash loan after the balance checks: fee booking, entry ceiling, commit),
* `crate::rates::scaling` (so the copied RatesAeneas scaling proofs find
  every function they name),
* the operation legs, at the function that does no token call:
  `ops::supply::apply`, `ops::borrow::accounting`,
  `ops::withdraw::accounting`, `ops::repay::accounting`,
  `ops::net_settle::apply`, `ops::seize::apply`,
  `ops::recapitalize::accounting`, `ops::strategy::accounting`,
  `ops::revenue::accounting`.

`opaque = ["crate::storage", "crate::time", "crate::events"]`: the storage,
ledger-time and event stand-ins are translated as signatures only, and
their meaning is in `FunsExternal.lean` (trusted base below).

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

### Aeneas bug: duplicate parent-clause field (`postextract.sh`)

**Bug (Aeneas `nightly-2026.11.12-0d21a9e`, bundled Charon `454586e`).** A
trait can have two parent clauses for the *same* trait (here two `Copy`
bounds). Aeneas then gives both structure fields the same name, and Lean
rejects the structure. `core::num::nonzero::ZeroablePrimitive` is
`trait ZeroablePrimitive: Sized + Copy + Sealed { type NonZeroInner: Sized + Copy; }`;
Charon turns the associated type into a type parameter, so the trait has the
parent clauses `Self: Copy` and `Self::NonZeroInner: Copy`, and Aeneas emits

```lean
structure core.num.nonzero.ZeroablePrimitive (Self : Type _) (Self_NonZeroInner : Type _) where
  markerCopyInst : core.marker.Copy Self
  markerCopyInst : core.marker.Copy Self_NonZeroInner   -- error: already declared
```

`global_sync` reaches it through `NonZeroU64::new`/`get`. The Charon options
`--hide-marker-traits`, `--remove-unused-clauses`, `--opaque` and
`--exclude` (on the trait) do not avoid it.

**Minimal reproduction (for an upstream issue),** independent of `core`:

```rust
pub trait Zeroable: Copy { type Inner: Copy; }
impl Zeroable for u64 { type Inner = u64; }
pub fn get<T: Zeroable>(x: T) -> T { x }
pub fn use_it(x: u64) -> u64 { get(x) }
```

`charon cargo --preset=aeneas` then `aeneas -backend lean` produces
`structure Zeroable (Self) (Self_Inner) where coremarkerCopyInst : core.marker.Copy Self;
coremarkerCopyInst : core.marker.Copy Self_Inner` and an instance with the
duplicated field; `lake env lean` reports
``Field `coremarkerCopyInst` has already been declared``. Expected: distinct
field names (for example a numeric suffix, as Aeneas does for other name
clashes).

**Workaround (`pool/postextract.sh`, run by `run.sh` after Aeneas and before the
build).** It renames the *second* field to `markerCopyInst1` in exactly three
places:

* the structure declaration (`Types.lean`);
* its `parentClauses` attribute (`Types.lean`);
* the one instance, `u64`'s impl
  `U64.Insts.CoreNumNonzeroZeroablePrimitiveNonZeroU64Inner` (`Funs.lean`).

The rename preserves semantics. A field name is a label, and both fields
keep their types, order and values. No generated function reads either
field. They are `Copy` marker-trait instances with no computational content.
The functions that use the trait, `NonZero::new`/`get`, are external models
that ignore the instance argument. Each substitution must match exactly once
or the script fails, so a changed Aeneas output is noticed. The production code stays
verbatim.

### Stand-ins (hand-written Rust)

| File | What it stands in for | Reaches Lean? | Why |
|---|---|---|---|
| `src/types.rs` | `common::types` | Yes (types only) | Includes `types/pool.rs` and `types/shared.rs` verbatim; adds a copy of the `OracleTolerance` record (fields only) that `validation.rs` names in a function no root reaches. The controller/oracle type files are left out |
| `src/oracle.rs` | `common::oracle::observation` | No | The three constants `validation.rs` imports, copied with production values |
| `src/storage.rs` | `contracts/pool/src/storage.rs` | Signatures only (`opaque`): `read_params`, `read_state`, `write_state`, `renew_market` | Production signatures, `unimplemented!()` bodies (never executed). Persistent storage is modelled in `FunsExternal.lean`: reads are uninterpreted, writes return `()`. `market_exists`, `write_params`, `write_rate_model`, `load_state`, `load_sync_data` exist only for `market.rs` / type-checking |
| `src/time.rs` | `contracts/pool/src/time.rs` | Signature only (`opaque`) | `now_ms`, uninterpreted in Lean |
| `src/events.rs` | `contracts/pool/src/events.rs` | Signature only (`opaque`): `emit_market_value_ceiling`, `emit_market_state` | The five emitters' signatures. `global_sync` calls `emit_market_value_ceiling` when the value ceiling held a chunk's interest, and `flash::finalize` calls `emit_market_state` after its commit; only the `apply` wrappers, `run_batch` and `market.rs` call the other three, so they are not reached |
| `src/ownable.rs` | `stellar_access::ownable` | No | `get_owner` signature, for `ops::revenue::apply` (not a root) |

### Shim (`pool/shim`, `pool/shim-macros`)

Charon translates only the signatures of shim items, so they are opaque to
Aeneas. Items that reach Lean get a meaning in the trusted base below; the
rest exist only so the verbatim files type-check.

| Shim item | Reaches Lean | Notes |
|---|---|---|
| `Env` | Yes | Unit struct; Aeneas translates it to `Unit` (in `Types.lean`) |
| `I256` and its 7 methods, `PartialEq`, `PartialOrd` | Yes | As in the rates shim |
| `Address` | Yes (type only) | Stored in `Cache` (`hub_asset`, `params.asset_id`), never inspected |
| `panic_with_error!`, `assert_with_error!` | — | Macros: `panic!` / `if !cond { panic!(..) }`, i.e. abort |
| `contracttype`, `contracterror` | — | Identity attributes (the real ones derive host conversions the arithmetic never calls) |
| `Env: Clone`, `Address: Clone` | Yes | `Cache::load` clones the env and the market key |
| `Storage`, `Instance`, `Env::storage`, `Storage::instance`, `Instance::extend_ttl` | Yes | Added for the verbatim `common/src/ttl.rs` (`renew_instance`, called by `ops::renewed_market`) |
| `Error`, `Bytes`, `BytesN`, `Symbol`, `String`, `Val`, `IntoVal`, `TryFromVal`, `Vec` (`new`, `is_empty`, `iter`, `contains`, `push_back`), `Executable`, `Address::executable`, `Env::{current_contract_address, invoke_contract}`, `token::Client` (incl. `try_transfer`) | No | Needed only to type-check `validation.rs`, `cash.rs::transfer_out`, the token/callback half of `flash.rs`, `ops::run_batch` and `withdraw::apply`. `TryFromVal`, the three `Vec` methods and `try_transfer` were added for the verbatim `ops/mod.rs` / `withdraw.rs` |

## Trusted base

Beyond Charon/Aeneas themselves (pinned in `../README.md`), the hand-written
meanings in `lean/PoolAeneas/TypesExternal.lean` and `FunsExternal.lean`:

| Item | Models | Definition |
|---|---|---|
| `aeneas_pool_shim.I256` | Soroban host 256-bit integer | `Int` (copied from RatesAeneas) |
| `I256.{from_i128, to_i128, add, sub, mul, div, rem_euclid, eq, partial_cmp}`, `MIN`, `MAX`, `checked` | Host I256 arithmetic | Exact integer arithmetic trapping outside `[-2^255, 2^255)`, truncating `div`, Euclidean `rem_euclid`, zero-divisor traps (copied from RatesAeneas) |
| `core.num.I128.checked_pow`, `core.num.U32.abs_diff`, `Option::{unwrap_or_else, map_or, and_then}`, `?` on `Option`, `Range::contains` | Rust core | Per the Rust documentation (copied from RatesAeneas) |
| `aeneas_pool_shim.Address` | `soroban_sdk::Address` | `Nat`: an opaque handle (as in the controller library); the extracted code only stores and clones it |
| `core.array.iter.IntoIter T N` | `core::array::IntoIter` | `List T`, the elements not yet yielded |
| `Array…IntoIterator.into_iter` | `<[T; N]>::into_iter` | The array's elements in order |
| `IntoIter…Iterator.next` | `<array::IntoIter>::next` | Head and tail; `None` when exhausted |
| `I128.Insts.CoreConvertFromU32.from` | `impl From<u32> for i128` | The same integer (always fits) |
| `I128.Insts.CoreConvertFromU64.from` | `impl From<u64> for i128` | The same integer, `IScalar.tryMk .I128 x.val`, written like the `u32` model; it always fits (`u64::MAX < 2^127`, so the failure branch is dead, proved by `i128_from_u64_spec`). Aeneas maps `i128::from(u64)` to this name, which its Lean library does not define. Reached only through `rates::compound::compound_interest` (`i128::from(delta_ms)`); `compound_interest_spec` and everything above it depend on it |
| `core.num.niche_types.NonZeroU64Inner` (type) | `core::num::niche_types::NonZeroU64Inner` | `Std.U64`, the value it wraps; never inspected |
| `core.num.nonzero.NonZero T Inner` (type) | `core::num::NonZero<T>` | The bare `T`. The non-zero invariant is not in the type, but the only constructor in the model (`new`) yields `Some` only for a non-zero value, as in Rust |
| `core.num.nonzero.NonZero.new` | `NonZero::<u64>::new` | `ok (if x = 0 then none else some x)`; never panics. Modelled monomorphically at `u64`, the one instance reached (`NonZeroU64::new(remaining)` in `global_sync`); the generated generic call elaborates against it |
| `core.num.nonzero.NonZero.get` | `NonZero::<u64>::get` | `ok x`, the wrapped value |
| `NonZeroU64Inner.Insts.CoreCloneClone.clone` | `<NonZeroU64Inner as Clone>::clone` | `ok x`; only referenced by the generated `Copy` instance |
| `pool/postextract.sh` | (mechanical fix of generated code) | Renames the duplicated `ZeroablePrimitive` field, see "Aeneas bug" above |
| `aeneas_pool_shim.Storage`, `aeneas_pool_shim.Instance` (types) | Soroban storage handles | `Unit` (only the TTL extension is called on them) |
| `Env.Insts.CoreCloneClone.clone`, `Address.Insts.CoreCloneClone.clone` | `Clone` of the env / an address | `ok` of the same (unit) value |
| `Env.storage`, `Storage.instance` | Handle accessors | `ok ()` |
| `Instance.extend_ttl` | Instance TTL extension (`common::ttl::renew_instance`) | `ok ()`: TTL is not modelled; host resource traps are not modelled |
| `ledgerNowMs` (`opaque`), `time.now_ms` | Ledger time in ms (`time::now_ms`, which aborts on `u64` overflow) | An **uninterpreted** `Result U64` constant (Lean `opaque`, a definition with a hidden body, not an axiom): theorems hold for every value, including failure |
| `storedParams`, `storedState` (`opaque`), `storage.read_params`, `storage.read_state` | Persistent reads of a market's params and state (aborting for a missing market) | **Uninterpreted** functions of the key. The leg theorems never inspect them: they name the synced cache by the hypothesis `ops.synced_market env k = ok c`, and `load_ok` relates the loaded cache to the stored state. Modelling a read as a function of the key is sound for one leg: each leg loads its market once |
| `storage.renew_market` | Persistent TTL extension of a market | `ok ()` |
| `storage.write_state` | Persistent write of a market's state (`Cache::commit`) | `ok ()`. The written state is the committed cache, which the theorems describe (`commit_eq`: `commit c = ok (snapOf c)`); nothing in a leg reads storage after its commit. Host write traps (resource limits) are not modelled |
| `events.emit_market_value_ceiling` | Publication of `MarketValueCeilingEvent` (`interest::global_sync`, when the value ceiling held interest) | `ok ()`. An event writes only the host event buffer, which nothing in the extracted code reads; the cache is unchanged (`global_sync_spec_of_inv`) |
| `events.emit_market_state` | Publication of the market state snapshot (`ops::flash::finalize`, after the commit) | `ok ()`, for the same reason. The snapshot it is given is the committed cache (`commit_eq`), which `flash_finalize_keeps_ceiling` describes |

Storage and time are not state in the model: nothing here says that a read
returns what an earlier write stored. So the argument "the stored-state
invariant is inductive" is made explicitly, in three steps:

1. Every leg's committed cache satisfies `PoolInv` (the leg theorems).
2. `load_ok` says a load copies the stored fields.
3. `synced_market_spec` carries `PoolInv` through accrual.

The array iterator items are reached through `rates::compound`;
`compound_interest_spec` depends on them (the loop runs over the array's seven
divisors in order).

## Theorems

Files are under `lean/PoolAeneas/Proofs/`.

### Share totals and revenue claims (`Shares.lean`, `contracts/pool/src/cache/shares.rs`)

`SharesInv c` is INV-ACCT-01 on one cache: `0 ≤ revenue ≤ supplied` and
`0 ≤ borrowed`.

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `mint_supply_preserves` | `Cache::mint_supply` | On success `supplied += s`, nothing else changes, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `burn_supply_preserves` | `Cache::burn_supply` | On success `0 ≤ s ≤ supplied`, `supplied -= s`, nothing else changes, `SharesInv` preserved | `SharesInv` only | INV-ACCT-01 |
| `mint_debt_preserves` | `Cache::mint_debt` | On success `borrowed += s`, nothing else changes, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `burn_debt_preserves` | `Cache::burn_debt` | On success `0 ≤ s ≤ borrowed`, `borrowed -= s`, `SharesInv` preserved | `SharesInv` only | INV-ACCT-01 |
| `accrue_revenue_preserves` | `Cache::accrue_revenue` | On success `revenue += s` and `supplied += s` (equally), nothing else, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `absorb_supply_as_revenue_preserves` | `Cache::absorb_supply_as_revenue` | On success `revenue += s`, `supplied` unchanged, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `burn_claimable_revenue_spec` | `Cache::burn_claimable_revenue` | Never aborts. With `T = ⌊revenue·supply_index/(RAY·10^k)⌋`: payout `amt` satisfies `0 ≤ amt ≤ cash`, `amt ≤ T`; `amt = 0` iff `cash ≤ 0 ∨ T ≤ 0` (and then the cache is unchanged); a positive `amt = min(cash, T)` burns `burned > 0` shares (at most all revenue) from `revenue` and `supplied` only; full claim burns all revenue, a cash-limited one burns `⌈revenue·amt/T⌉`; and `amt·RAY·10^k ≤ burned·supply_index` (the payout never exceeds the burned shares' value). **The `scaled_to_burn != Ray::ZERO` `InternalError` assert is unreachable.** | `supply_index > 0`, `0 ≤ revenue ≤ supplied`, `cash ≥ 0`, `dec ≤ 27`, `revenue·supply_index < 2^127·RAY` | INV-ACCT-06, INV-ACCT-01, ADR-0003 |
| `burn_claimable_revenue_preserves` | `Cache::burn_claimable_revenue` | Returns a cache satisfying `SharesInv` | `SharesInv` and the hypotheses above | INV-ACCT-01 |

Why the `s ≥ 0` hypotheses hold. A mint receives one of three values:

* `calculate_scaled_supply` / `calculate_scaled_borrow` of a validated
  non-negative amount (floor/ceiling of a non-negative quotient);
* `protocol_fee_shares` (proved non-negative in `add_protocol_revenue_spec`);
* a seized position's validated non-negative scaled amount.

The burns need no hypothesis. The checked subtraction and the
`require_revenue_backed` assert reject every input that would break the
invariant.

### Guards (`Guards.lean`, `contracts/pool/src/guards.rs`)

`B = ⌈borrowed·borrow_index/RAY⌉` (ceiled debt value), `S =
⌊supplied·supply_index/RAY⌋` (floored supply value). All guards share two
hypotheses:

* share totals and indexes are non-negative (INV-ACCT-01; indexes start at
  RAY and are floored at `SUPPLY_INDEX_FLOOR_RAW`);
* the RAY values fit `i128` (otherwise `mul_floor`/`mul_ceil` abort, also a
  rejection).

| Lean name | Rust function | Claim | Extra hypotheses | Invariants |
|---|---|---|---|---|
| `require_supply_for_debt_spec` | `guards::require_supply_for_debt` | Passes iff `¬(supplied = 0 ∧ borrowed ≠ 0)` | none | INV-ACCT-09 |
| `require_utilization_below_max_spec` | `guards::require_utilization_below_max` (debt mints) | Passes iff `supplied = 0 ∨ B = 0 ∨ B·RAY ≤ max_utilization·S` (`UtilPass`), at every ceiling including 100%, where it reads `B ≤ S`. The code compares `B` with `⌊S·max_utilization/RAY⌋` and divides nothing: debt rounded up, supply and the allowed side down, so rounding cannot admit utilization above the ceiling and no quotient can overflow; debt against zero floored supply fails | `B`, `S` fit; `0 ≤ max_utilization ≤ RAY` (`verify`) | INV-ACCT-08, audit S-03 |
| `require_exit_utilization_below_max_spec` | `guards::require_exit_utilization_below_max` (ordinary withdrawals, revenue claims) | Passes iff `max_utilization ≥ RAY ∨ UtilPass`: the same gate below a 100% ceiling, no check at 100% | as above | INV-ACCT-08 |
| `require_exit_utilization_ok` | `guards::require_exit_utilization_below_max` | Returned ⇒ `max_utilization ≥ RAY`, or the full gate returned on the same cache | none | INV-ACCT-08 |
| `max_market_value_spec`, `max_market_entry_spec`, `maxMarketEntry_eq` | `constants::MAX_MARKET_VALUE_RAY`, `MAX_MARKET_ENTRY_RAY` | Evaluate to `MaxMarketValue = 2^127 − 1 − 10^27` and `MaxMarketEntry = MaxMarketValue / 8` (rounded down) | none | INV-IDX-06, INV-ACCT-11 |
| `fits_entry_ceiling_spec` | `guards::fits_entry_ceiling` | Never aborts; `true` iff `EntryFits shares index`: the share count is at most `MaxMarketEntry` and so is its half-up value. A value that does not fit `i128` gives `false`, not an abort | `shares, index ≥ 0` | INV-ACCT-11 |
| `require_within_entry_ceiling_spec` | `guards::require_within_entry_ceiling` (supply, borrow, strategy and flash-loan entries) | Passes iff `EntryFits supplied supply_index ∧ EntryFits borrowed borrow_index` | share totals and indexes `≥ 0` | INV-ACCT-11 |
| `require_within_entry_ceiling_congr` | `guards::require_within_entry_ceiling` | Depends only on the share totals and the indexes: two caches that agree on them get the same answer | none | INV-ACCT-11 |
| `require_liquidation_buffer_spec` | `guards::require_liquidation_buffer` (reserve via `Bps::apply_to_ceil`) | With `Sa = ⌊supplied·supply_index/(RAY·10^k)⌋` and `R = ⌈Sa·200/10000⌉`: passes iff `cash − draw ≥ R`, iff `Sa·200 ≤ (cash − draw)·10000` | `cash ≥ 0`, `draw ≥ 0`, `dec ≤ 27` | INV-ACCT-07 |
| `backing_shortfall_spec` | `guards::backing_shortfall` | Never aborts. With `C = S` (floored claims) and `Dt = B` (ceiled debt): `C ≤ Dt → 0`; otherwise `max(0, U − cash)` with `U = ⌊(C − Dt)/10^k⌋` (one floor to token units); a positive shortfall satisfies `shortfall·10^k ≤ C − Dt − cash·10^k` (never over-reported) | `cash ≥ 0`, `dec ≤ 27` | INV-ACCT-04 |
| `require_backed_market_spec` | `guards::require_backed_market` | Passes iff `C ≤ Dt ∨ C − Dt < (cash + 1)·10^k` | as above | INV-ACCT-04 |

### Typed fixed-point wrappers (`Wrappers.lean`, `FloorCeilScaling.lean`; `common/src/math/fp.rs`)

`fp_core` is private. The pool code reaches it only through these `fp.rs`
wrappers. Each spec is total correctness about the generated
wrapper.

| Lean name | Rust function | Claim | Hypotheses | Used by |
|---|---|---|---|---|
| `ray_div_floor_saturating_spec` (in `FloorCeilScaling.lean`) | `Ray::div_floor_saturating` | Never aborts; returns `⌊a·RAY/b⌋` clamped to `[i128::MIN, i128::MAX]` | `b ≠ 0` | `protocol_fee_shares` (`add_protocol_revenue_spec`, `book_fee_spec`), `calculate_scaled_cap_spec` |
| `bps_apply_to_ceil_spec` | `Bps::apply_to_ceil` | Returns exactly `⌈amount·bps/10000⌉` (every sign) | that ceiling fits `i128` | `require_liquidation_buffer_spec` |
| `ray_try_mul_spec` | `Ray::try_mul` | Never aborts; `None` for a negative operand; for `a, b ≥ 0`: `Some(half-up a·b/RAY)` when it fits, `None` exactly when it does not | none | `rates::simulate::market_value` (`accrue_step`), `guards::fits_entry_ceiling` |
| `ray_mul_int_spec` | `Ray::mul_int` | Returns exactly `a·m` | `a·m` fits `i128` | `compound_interest_spec` |
| `ray_mul_int_overflow` | `Ray::mul_int` | Never returns when `a·m` does not fit (the `MathOverflow` panic); with the above, the exact pass condition | `a·m` does not fit | |
| `i128_from_u64_spec` | `i128::from(u64)` (trusted model) | Never aborts; the same integer | none | `compound_interest` |
| `bps_from_same` | `Bps::from::<i128>` | Wraps the value unchanged | none | `require_liquidation_buffer_spec` |

`Ray::try_div` is not in the pool extraction. `utilization` divides with
`Ray::div`, and only when `borrowed < supplied`.
The other `fp.rs` wrappers (`Ray::mul_wad_ratio_floor`, `Wad::*`,
`Bps::{div, from_fraction_floor, from_fraction_ceil, try_from_fraction,
from_weighted_sum, apply_to_ray_ceil}`) are not reached from the pool
extraction roots, so they are not in the pool Lean code.

### Cash (`Cash.lean`, `contracts/pool/src/cache/cash.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `require_nonneg_amount_iff` | `validation::require_nonneg_amount` | Passes iff `amount ≥ 0` | none | INV-ACCT-02 |
| `require_reserves_iff` | `Cache::require_reserves` | Passes iff `amount ≤ cash` | none | INV-ACCT-02 |
| `credit_cash_spec` | `Cache::credit_cash` | Returns iff `amount ≥ 0 ∧ cash + amount < 2^127`; then only `cash` changes, to `cash + amount` | none | INV-ACCT-02 |
| `debit_cash_spec` | `Cache::debit_cash` | Returns iff `0 ≤ amount ≤ cash`; then only `cash` changes, to `cash − amount ≥ 0` | none | INV-ACCT-02 |

### Interest booking (`Interest.lean`, `contracts/pool/src/interest.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `add_protocol_revenue_spec` | `interest::add_protocol_revenue` | Never aborts. Zero fee: unchanged. Otherwise mints `s = min(⌊fee·RAY/supply_index⌋ (saturating), i128::MAX − supplied) ≥ 0` into both `revenue` and `supplied`, nothing else; `SharesInv` preserved | `SharesInv`, `supply_index > 0`, `fee ≥ 0` | INV-ACCT-01, INV-IDX-05 (floor rounding of protocol shares) |
| `apply_bad_debt_spec` | `interest::apply_bad_debt_to_supply_index` | Never aborts; only `supply_index` changes. `V = 0` (V = half-up `supplied·supply_index/RAY`): unchanged. Else with `rem = V − min(bad, V)`, `f = ⌊rem·RAY/V⌋`, `n = ⌊supply_index·f/RAY⌋`: new index `= max(n, RAY/1000)`, so `≥ SUPPLY_INDEX_FLOOR_RAW`, `≤ max(old, floor)`, and `n·V ≤ old·rem` (at least the exact pro-rata write-down) | `supplied ≥ 0`, `supply_index > 0`, `bad ≥ 0`, `V` fits | INV-IDX-03, INV-IDX-02 (lower clamp) |

### Flash loans (`Flash.lean`, `contracts/pool/src/ops/flash.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `flash_loan_fee_on_spec` | `Bps::flash_loan_fee_on` (`common/src/math/fp.rs`) | Never aborts; fee = half-up `amount·bps/10000`, or 1 when the rate is positive and that is 0 | `0 ≤ bps ≤ 10000`, `amount ≥ 0` | INV-FLASH-01 |
| `terms_spec` | `flash::terms` | Never aborts; `0 ≤ F ≤ P`, `F > 0` when `bps > 0`; `total_repayment = P + F`, `balance_after_payout = B − P`, `balance_after_repayment = B + F`, hence `(B − P) + (P + F) = B + F` | `P > 0` (`require_positive_amount`), `bps ≤ 10000` (`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`), `P ≤ B` (the payout comes out of `B`), `B + P < 2^127` | INV-FLASH-01 |
| `book_fee_spec` | `flash::book_fee` | Never aborts; `cash += F`, `revenue` and `supplied` both `+= s ≥ 0`, nothing else; `SharesInv` preserved | `SharesInv`, `supply_index > 0`, `F ≥ 0`, `dec < 27`, `F·10^k` and `cash + F` fit | INV-FLASH-01, INV-ACCT-01, INV-ACCT-02 |

### Compound interest (`Compound.lean`, `common/src/rates/compound.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `compound_interest_spec` | `rates::compound_interest` | Never aborts; returns `f` with `RAY + rate·delta_ms ≤ f ≤ 21·RAY` (exactly `RAY` for `delta_ms = 0`). Proof: the k-th power stays `≤ 3^k·RAY` (half-up rounding cannot exceed `3B` when the product is below `3B·RAY + RAY/2`), every term is non-negative, and `4 + Σ_{k=2..8} 3^k/k! < 21` | `rate ≥ 0`, `rate·delta_ms ≤ 3·RAY` (production: `≤ 2·RAY + 1.6·10^10`, `compound_x_le`) | INV-IDX-01 (factor `≥ RAY`), INV-IDX-04 |
| `ci_step`, `ci_loop_cons`, `ci_loop_nil(_spec)`, `hu_pow_bound` | the Taylor loop | One iteration with bounds; loop unfolding | | |

### Accrual step (`Accrual.lean`, `common/src/rates/simulate.rs`, `index.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `compound_x_le` | (curve → compound) | The production exponent: `r = half_up(a/31_556_926_000)` with `0 ≤ a ≤ 2·RAY` and `delta ≤ 31_556_926_000` give `0 ≤ r` and `r·delta ≤ 3·RAY` | `a` from `calculate_borrow_rate_spec` | INV-IDX-04 |
| `protocol_fee_shares_spec` | `rates::protocol_fee_shares` | Never aborts; `0 ≤ sh`, `supplied + sh ≤ i128::MAX`, `sh·supply_index ≤ fee·RAY` | `fee ≥ 0`, `supply_index > 0`, `supplied ≥ 0` | INV-IDX-05, INV-ACCT-01 |
| `update_supply_index_strong` | `rates::update_supply_index` | Never aborts; `old ≤ n ≤ 10^36`, and `n = old` or `supplied·n ≤ (V + rewards)·RAY` (the index move distributes at most `rewards`) | `supplied, rewards ≥ 0`, `0 < old ≤ 10^36`, `V + rewards` fits | INV-IDX-02 |
| `supply_index_reward_shortfall_total` | `rates::supply_index_reward_shortfall` | Never aborts when `t ≤ t' ≤ t + rewards < 2^127` (old/new supply values); returns `rewards − (t' − t) ∈ [0, rewards]` | as stated | INV-IDX-05 |
| `market_value_spec` | `rates::simulate::market_value` | Never aborts on non-negative operands; `Some(hu(shares·index, RAY))` when that fits `i128`, `None` exactly when it does not | `shares, index ≥ 0` | INV-IDX-06 |
| `value_room_spec` | `rates::simulate::value_room` | Never aborts; `room = max(0, MAX_MARKET_VALUE_RAY − max(debt_value, supply_value))` | values `≥ 0` | INV-IDX-06 |
| `cap_borrow_index_spec` | `rates::simulate::cap_borrow_index` | Never aborts; result `n` with `old ≤ n ≤ grown` and `borrowed·(n − old) ≤ room·RAY` (the debt value at `n` exceeds its value at `old` by at most `room`; the allowed index growth rounds down) | `borrowed ≥ 0`, `0 ≤ old ≤ grown`, `room ≥ 0` | INV-IDX-06, INV-IDX-01 |
| `accrue_step_spec` | `rates::accrue_step` | **Never aborts, with no hypothesis on the size of the market values.** `borrow_index ≤ b' ≤ 10^36`, `supply_index ≤ s' ≤ 10^36`, `0 ≤ revenue_shares`, `supplied + revenue_shares ≤ i128::MAX`, `revenue_shares·s' + (t' − t)·RAY ≤ (nd − od)·RAY` (revenue value plus value distributed to suppliers ≤ accrued interest; `od, nd` old/new debt values, `t ≤ t'` old/new supply values), and `nd − od ≤ max(0, MAX_MARKET_VALUE_RAY − max(od, t))` (interest is at most the room below the value ceiling; zero when a total is at or above it, or does not fit `i128`) | `ValidRateModel` (incl. `max_borrow_rate ≤ 2·RAY`), `borrowed, supplied ≥ 0`, `0 ≤ borrow_index ≤ 10^36`, `0 < supply_index ≤ 10^36`, `delta ≤ MAX_COMPOUND_DELTA_MS` | INV-IDX-06, INV-IDX-01, INV-IDX-02, INV-IDX-05, audit S-04 |
| `accrue_step_within_ceiling` | (arithmetic consequence of `accrue_step_spec`) | If the old debt value and the old supply value are at most `MAX_MARKET_VALUE_RAY`, so are the new debt value and the supply value after the step's revenue shares are added at the new index | the last two conclusions of `accrue_step_spec` | INV-IDX-06 |
| `hu_scale_le`, `hu_le_of_le`, `hu_lt_of_lt`, `hu_add_le`, `max_compound_delta_val` | | Half-up bounds (`hu_add_le`: `n' ≤ n + r·RAY → hu(n') ≤ hu(n) + r`); `MAX_COMPOUND_DELTA_MS = 31_556_926_000` | | |

`accrue_step_spec` has no hypothesis that a market value fits `i128`
(audit S-04). The step values both totals with `Ray::try_mul`. It caps the
borrow index at the room below `MAX_MARKET_VALUE_RAY`. Every later product is
then bounded by an old value plus that room, and the proof uses this bound.
The only abort paths are the sign checks in `market_value`. INV-ACCT-01 and
the index bounds exclude them.

### Chunk and `global_sync` (`GlobalSync.lean`, `contracts/pool/src/interest.rs`)

`AccrualDomain c` is the hypothesis list of `accrue_step_spec` on the cache
fields plus `SharesInv c`: a validated rate model, INV-ACCT-01 and indexes in
range. It has no hypothesis on the size of the market values. `ChunkPost c c'`: `SharesInv c'`,
both indexes monotone and `≤ 10^36`, `revenue` and `supplied` grow by the same
`sh ≥ 0`, no other field changes. `ChunkPost` is reflexive on `AccrualDomain`
and transitive. A chunk keeps the cache in `AccrualDomain`
(`ChunkPost.refl`, `ChunkPost.trans`, `AccrualDomain.of_chunk`). So it is a
loop invariant for any number of chunks.

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `accrue_chunk_spec` | `interest::accrue_chunk` | Never aborts; `ChunkPost c c'` (the returned flag says whether the value ceiling held the chunk's interest) | `AccrualDomain c`, `delta_ms ≤ MAX_COMPOUND_DELTA_MS` | INV-IDX-01, INV-IDX-02, INV-IDX-06, INV-ACCT-01 |
| `global_sync_loop_spec` | `global_sync`'s loop | Induction principle: any `Inv remaining cache` preserved by every chunk `k = min(r, MAX_COMPOUND_DELTA_MS)` with `r > 0` (from `Inv r` to `Inv (r − k)`) holds at `remaining = 0`; the loop terminates. So chunks are positive, at most one year, all but the last full, and sum to the elapsed time | the step hypothesis | INV-IDX-04 |
| `global_sync_noop` | `interest::global_sync` | `current ≤ last` (no elapsed time): returns the cache unchanged, no timestamp advance | none | INV-IDX-04 |
| `global_sync_spec_of_inv` | `interest::global_sync` | With `e = current − last`: given a chunk invariant with `Inv e c`, never aborts; `e = 0` → unchanged; else the result is `{x with last_timestamp := current}` for an `x` with `Inv 0 x` | as `global_sync_loop_spec` | INV-IDX-04 |
| `global_sync_spec` | `interest::global_sync` | **Never aborts, for any elapsed time.** `e = 0` → unchanged; otherwise the elapsed time is accrued in chunks of at most one year, the result `x` satisfies `ChunkPost c x` and `AccrualDomain x`, and `last_timestamp = current_timestamp`. The value-ceiling event, when published, changes nothing in the cache | `AccrualDomain c` | INV-IDX-06, INV-IDX-01, INV-IDX-02, INV-IDX-04, INV-ACCT-01, audit S-04 |
| `min_u64_val`, `sat_sub_u64_val`, `elapsed_val` | `u64::min`, `u64::saturating_sub`, `Cache::elapsed_ms` | Value lemmas | | |

INV-IDX-04's "mutating accrual and read-only projection use the same
calculation" holds by construction (both call `accrue_step`); the projection
`simulate_update_indexes` is not extracted.

### Operation legs (`Legs.lean`, `Legs2.lean`, `contracts/pool/src/ops/`)

Shape. A leg loads and accrues its market (`ops::synced_market` =
`Cache::load` + `global_sync`), mutates the cache, and commits it. The leg
theorems name the synced cache by the hypothesis `ops.synced_market env key
= ok c` and assume `PoolInv c` on it. They then describe **every successful
return** of the leg (`leg = ok r → …`). That is the form a Certora summary
needs: a summary constrains the values a call returns, and an aborting call
reverts the transaction. The leg's own guards (`require_*`, checked
arithmetic, the `*RoundsToZeroShares` asserts) are facts the theorems
derive, not hypotheses. Fit hypotheses ("`amount·10^k < 2^127`", "the RAY
value of the position fits") are the domains on which the conversion
helpers are proved exact.

`PoolInv c` (stored-state invariant): `0 ≤ revenue ≤ supplied`,
`0 ≤ borrowed` (INV-ACCT-01), `cash ≥ 0` (INV-ACCT-02),
`10^24 ≤ supply_index ≤ 10^36`, `10^27 ≤ borrow_index ≤ 10^36`.
`ParamsOk c`: `asset_decimals ≤ 18`, `flashloan_fee ≤ 500` (established by
`MarketParamsRaw::verify` at creation / rate-model replacement, not
extracted; no leg changes params). `snapOf c` is the snapshot `commit`
returns, `indexOf c` the returned index pair. `k = 27 − decimals`.

| Lean name | Rust function | Claim (on every return) | Hypotheses | Invariants |
|---|---|---|---|---|
| `commit_eq` | `Cache::commit` | Never aborts; returns `snapOf c` | none | |
| `load_leg_ok` | `ops::load_leg` | Amount and position were `≥ 0`, the market synced to the returned cache, the returned position is the action's | none | INV-ACCT-05 (validation) |
| `renewed_market_ok` | `ops::renewed_market` | Returned `c` ⇒ `synced_market` returned `c` | none | |
| `market_index_eq`, `position_mutation_eq`, `strategy_mutation_eq` | `Cache::{market_index, position_mutation, strategy_mutation}` | The DTOs carry the given position/amounts, `indexOf c` and the cache's decimals | none | |
| `nonneg_of_ok`, `positive_of_ok`, `massert_ok`, `sub_unwrap_eq_ok`, `ceil_le_of_index_ge_ray` | validation helpers, `checked_sub().unwrap_or_else(panic)` | Returned ⇒ the checked condition; the checked difference is exact; `⌈A·RAY/bi⌉ ≤ A` for `bi ≥ RAY` | as stated | |
| `supply_apply_spec` | `ops::supply::apply` | `amount, position ≥ 0`; minted `m = ⌊amount·10^k·RAY/si⌋ ≥ 0` (floor), `m > 0` if `amount > 0`, `m = 0` if `amount = 0`; new position `= position + m`; `actual_amount = amount`; committed cache = `c` with `supplied += m`, `cash += amount` only; `PoolInv`; the committed cache passes the entry-ceiling guard | `PoolInv c`, `dec ≤ 18`, `amount·10^k < 2^127`, floor fits | INV-ACCT-01/02/03/05/10/11, ADR-0003 |
| `mint_debt_ok` | `ops::borrow::mint_debt` | `amount > 0`, `amount ≤ cash`, liquidation-buffer guard passed on the gross amount, minted `m = ⌈amount·10^k·RAY/bi⌉ > 0`, `m ≤ amount·10^k`, position `+= m`, cache `borrowed += m` only, the utilization guard (every ceiling, 100% included) and the entry-ceiling guard passed after | `PoolInv c`, `dec ≤ 18`, `amount·10^k < 2^127` | INV-ACCT-05/07/08/11 |
| `borrow_accounting_spec` | `ops::borrow::accounting` | As `mint_debt_ok`, plus `actual_amount = amount`, committed cache = `c` with `borrowed += m`, `cash -= amount` only, `PoolInv`; the committed cache passes the entry-ceiling guard | as above | INV-ACCT-01/02/05/07/08/10/11 |
| `repay_accounting_spec` | `ops::repay::accounting` | With `D = ⌈position·bi/(RAY·10^k)⌉`: `0 ≤ net ≤ amount`, `overpayment = amount − net`; full (`D ≤ amount`): new `= 0`, `net = D`; partial: `net = amount`, burn `= ⌊amount·10^k·RAY/bi⌋`; `amount > 0 ∧ position > 0 → new < position`, otherwise `new = position`; cache `borrowed -= burn`, `cash += net` only; `PoolInv` | `PoolInv c`, `dec ≤ 18`, `⌈position·bi/RAY⌉` fits | INV-ACCT-01/02/03/05/10, ADR-0003 |
| `withhold_liquidation_fee_ok` | `withdraw::withhold_liquidation_fee` | No fee-bearing liquidation: `net = gross`, cache unchanged; otherwise `fee ≤ gross`, `net = gross − fee`, fee booked as `s ≥ 0` shares into `revenue` and `supplied`, worth at most the fee: `s·supply_index ≤ fee·10^k·RAY` | `SharesInv`, `si > 0`, `fee ≥ 0`, `dec < 27`, `gross·10^k < 2^127` | INV-ACCT-01 |
| `gate_and_debit_ok` | `withdraw::gate_and_debit` | `0 ≤ net ≤ cash`; the exit utilization guard passed unless skipped; not `supplied = 0 ∧ borrowed ≠ 0`; cache `cash -= net` only | none | INV-ACCT-02/08/09 |
| `withdraw_accounting_spec` | `ops::withdraw::accounting` | With `H` the half-up position value, `F` its floor value, `G` gross: `0 ≤ G ≤ F`, `G ≤ amount`, `0 ≤ new ≤ position`; full (`H ≤ amount`): `new = 0`, `G = F`; partial: `G = amount`, burn `= ⌈amount·10^k·RAY/si⌉`; `amount = 0 ∧ H > 0 → new = position`; `G > 0 → new < position`; `amount > 0 ∧ position > 0 → new < position`; `position = 0 → new = 0`; `net = G` or `G − fee` (fee-bearing liquidation), `0 ≤ net ≤ G`; cache `supplied := supplied − burn + s`, `revenue += s` (`s = 0` without fee, and always `s ≤ burn`: the fee shares never exceed the shares burned), `cash -= net` only; `PoolInv`; not `supplied = 0 ∧ borrowed ≠ 0` after | `PoolInv c`, `dec ≤ 18`, `hu(position·si, RAY)` fits (the exact abort condition of `resolve_withdrawal`) | INV-ACCT-01/02/05/09/10, ADR-0003 |
| `net_settle_apply_spec` | `ops::net_settle::apply` | `0 ≤ settled ≤ amount` (and `≤ F`, `≤ D` when positive); `0 ≤ ns ≤ ss`, `0 ≤ nd ≤ ds`; `settled > 0` burns both sides, `settled = 0` burns neither; full close only when exhausted; partial burns favour the protocol; cache `supplied -= ss − ns`, `borrowed -= ds − nd` only (no cash); `PoolInv`; not `supplied = 0 ∧ borrowed ≠ 0` | `PoolInv c`, `dec ≤ 18`, `⌊ss·si/RAY⌋`, `⌈ds·bi/RAY⌉` fit | INV-ACCT-01/02/05/09/10, ADR-0003 |
| `seize_apply_spec` | `ops::seize::apply` | Position `≥ 0`; committed cache satisfies `PoolInv`, cash unchanged; deposit side: `revenue += p` only; borrow side: `borrowed -= p`, supply index written down (`≥ 10^24`, `≤ old`), nothing else | `PoolInv c`; borrow side: `hu(supplied·si, RAY)` fits | INV-ACCT-01/10, INV-IDX-03 |
| `backing_shortfall_nonneg` | `guards::backing_shortfall` | Returned ⇒ `≥ 0` | none | INV-ACCT-04 |
| `recapitalize_accounting_spec` | `ops::recapitalize::accounting` | `amount ≥ 0`; `applied = min(amount, sf)` for the returned shortfall `sf ≥ 0`, so `0 ≤ applied ≤ amount`; `refund = amount − applied`; cache `cash += applied` only (no shares, no index move); `PoolInv` | `PoolInv c` | INV-ACCT-02/03/04 |
| `compute_fee_ok` | `strategy::compute_fee` | `0 ≤ fee ≤ amount`; 0 without `charge_fee`; otherwise the flash-loan fee | `amount ≥ 0`, `flashloan_fee ≤ 10000` | INV-FLASH-01 |
| `strategy_accounting_spec` | `ops::strategy::accounting` | `amount > 0`; buffer guard on the gross amount; `0 ≤ fee ≤ amount`; minted `m > 0` (ceiling); position `+= m`; `actual_amount = amount`, `amount_received = amount − fee`; cache `borrowed += m`, `revenue` and `supplied += s ≥ 0` (`0` for zero fee), `cash -= amount − fee` only; `PoolInv`; the committed cache passes the entry-ceiling guard (checked after the mint and again after the fee shares) | `PoolInv c`, `ParamsOk c`, `amount·10^k < 2^127` | INV-ACCT-01/02/05/07/10/11 |
| `revenue_accounting_spec` | `ops::revenue::accounting` | `burn_claimable_revenue` gave `(amt, c1)` with `ClaimResult c T amt c1`; `0 ≤ amt ≤ cash`; the exit utilization guard passed on `c1`; committed cache = `c1` with `cash -= amt`; `PoolInv`; not `supplied = 0 ∧ borrowed ≠ 0` | `PoolInv c`, `dec ≤ 27`, `revenue·si < 2^127·RAY` | INV-ACCT-01/02/06/08/09 |
| `load_ok` | `Cache::load` | The loaded share totals, indexes, cash and timestamp are the stored ones | `storedState k = ok st` | |
| `synced_market_spec` | `ops::synced_market` | **Never aborts, for any elapsed time and any size of the market values**; `PoolInv` preserved, cash and params unchanged, indexes not lower | load returned `c0`, `PoolInv c0`, `ValidRateModel c0.params` | INV-IDX-06, INV-IDX-01/02/04, INV-ACCT-01/02, audit S-04 |
| `AccrualDomain.of_poolInv` | | `PoolInv c` with a validated rate model gives `AccrualDomain c` | as stated | |

INV-ACCT-10 (pool side). The invariant is "Σ account supply shares =
supplied − revenue, Σ account debt shares = borrowed". The pool keeps no
per-account book. So the invariant holds if every leg moves the pool totals
by exactly the change of the account position it returns. The leg theorems
state exactly that:

* supply: `Δsupplied = Δposition`;
* borrow/strategy: `Δborrowed = Δposition` (fee shares go to `revenue` and
  `supplied` equally);
* repay: `Δborrowed = Δposition`;
* withdraw: `Δ(supplied − revenue) = Δposition`;
* net settle: the same on both sides;
* seize deposit: `Δ(supplied − revenue) = −p`; seize borrow:
  `Δborrowed = −p` (the controller then removes the seized position);
* accrual moves `revenue` and `supplied` equally (`ChunkPost`).

The controller half (it writes the returned position) is not covered here.

INV-ACCT-03 (pool side). The pool credits exactly the amount the
controller reports as received (supply: `cash += amount`; repay: `cash +=
net`, refund `= amount − net`; recapitalize: `cash += applied`, refund `=
amount − applied`). The measurement of the receipt (balance before/after)
is in the controller and the token, outside this model.

### The market value ceiling as a stored-state invariant (`ValueCeiling.lean`)

`ValueCeil c` (INV-IDX-06): the half-up value of total supply at the supply
index and of total debt at the borrow index are both at most
`MAX_MARKET_VALUE_RAY`. Every extracted leg keeps it, so no stored state
reached through these legs has a market total above the ceiling. Each exit
theorem takes the hypotheses of its leg theorem plus `ValueCeil c` on the
synced cache. The entry theorems and the flash-loan theorem need no ceiling
hypothesis.

| Lean name | Rust function | Claim | Why | Invariants |
|---|---|---|---|---|
| `accrue_chunk_keeps_ceiling` | `interest::accrue_chunk` | Never aborts; `ChunkPost` and `ValueCeil` of the result | `accrue_step_spec` caps interest at the room below the ceiling (`accrue_step_within_ceiling`) | INV-IDX-06 |
| `global_sync_keeps_ceiling` | `interest::global_sync` | Never aborts; `ValueCeil` of the result, **for any elapsed time** | chunk invariant | INV-IDX-06 |
| `synced_market_keeps_ceiling` | `ops::synced_market` | Never aborts; `ValueCeil` of the synced cache every leg starts from | load + `global_sync` | INV-IDX-06 |
| `supply_keeps_ceiling`, `borrow_keeps_ceiling`, `strategy_keeps_ceiling` | `ops::supply::apply`, `ops::borrow::accounting`, `ops::strategy::accounting` | `ValueCeil` of the committed cache, with no ceiling hypothesis | the committed cache passes the entry-ceiling guard, and `MAX_MARKET_ENTRY_RAY ≤ MAX_MARKET_VALUE_RAY` (`ValueCeil.of_entry_guard`) | INV-ACCT-11 |
| `repay_keeps_ceiling` | `ops::repay::accounting` | `ValueCeil` of the committed cache | debt shares only fall | INV-IDX-06 |
| `withdraw_keeps_ceiling` | `ops::withdraw::accounting` | `ValueCeil` of the committed cache, with or without a liquidation fee | fee shares minted `≤` supply shares burned | INV-IDX-06 |
| `net_settle_keeps_ceiling` | `ops::net_settle::apply` | `ValueCeil` of the committed cache | supply and debt shares only fall | INV-IDX-06 |
| `seize_keeps_ceiling` | `ops::seize::apply` | `ValueCeil` of the committed cache, both sides | deposit side reclassifies shares; borrow side burns debt and lowers the supply index | INV-IDX-06, INV-IDX-03 |
| `recapitalize_keeps_ceiling` | `ops::recapitalize::accounting` | `ValueCeil` of the committed cache | only cash changes | INV-IDX-06 |
| `revenue_keeps_ceiling` | `ops::revenue::accounting` | `ValueCeil` of the committed cache | revenue and supply shares only fall | INV-IDX-06 |
| `flash_finalize_keeps_ceiling` | `ops::flash::finalize` | `PoolInv` and `ValueCeil` of the returned (committed) cache, with no ceiling hypothesis; `cash += fee` | the fee is booked (`book_fee_spec`), then the cache passes the entry-ceiling guard | INV-ACCT-11, INV-FLASH-01 |
| `ValueWithin.mono`, `ValueWithin.of_entryFits`, `ValueCeil.of_le`, `ValueCeil.of_entry_guard` | | A smaller product has a smaller half-up value; the entry ceiling is below the value ceiling | | |

`flash_finalize_keeps_ceiling` takes the hypotheses of `book_fee_spec`
(`fee ≥ 0`, `decimals < 27`, `fee·10^k` and `cash + fee` fit `i128`).
`finalize` runs after `flash::apply` has checked the token balances; that
part (callback, balance measurement, transfers) is not extracted.

Not covered by these: market creation, which writes zero totals.

**The ceiling discharges the fit hypotheses.** The leg theorems assume that
the values they convert fit `i128`. Those assumptions follow from
`ValueCeil`:

| Lean name | Claim | Discharges |
|---|---|---|
| `ValueCeil.supply_value_fits` | Within the ceiling, the half-up value of total supply is below `2^127` | the fit hypothesis of `seize_apply_spec`; `seize_keeps_ceiling` does not take it |
| `ValueCeil.revenue_fits` | Within the ceiling and with `revenue ≤ supplied`, `revenue·supply_index < 2^127·RAY` | the fit hypothesis of `revenue_accounting_spec`; `revenue_keeps_ceiling` does not take it |
| `ValueWithin.fits_half_up`, `ValueWithin.fits_floor`, `ValueWithin.fits_ceil` | For `p ≤ total` and `index ≥ 0`: the half-up, floored and ceiled value of `p` at the index is below `2^127` (the ceiled value is at most the half-up value plus one, and the ceiling leaves one whole token below `i128::MAX`) | the position fit hypotheses, given "position ≤ market total" |
| `isFloor_le_halfUp`, `isCeil_le_halfUp_succ` | `⌊n/RAY⌋ ≤ hu(n/RAY)` and `⌈n/RAY⌉ ≤ hu(n/RAY) + 1` | |
| `withdraw_keeps_ceiling_of_position_le` | `withdraw_keeps_ceiling` with `position ≤ supplied` in place of the fit hypothesis | |
| `repay_keeps_ceiling_of_position_le` | `repay_keeps_ceiling` with `position ≤ borrowed` in place of the fit hypothesis | |
| `net_settle_keeps_ceiling_of_position_le` | `net_settle_keeps_ceiling` with `supply position ≤ supplied` and `debt position ≤ borrowed` in place of both fit hypotheses | |

So the ceiling discharges the total-based fit assumptions (seize, revenue
claim). The position-based ones (withdraw, repay, net settlement) reduce to
one statement: **the caller's position is at most its market's total**. The
pool keeps no per-account book, so it cannot prove that statement. It is the
pool-side shape of INV-ACCT-10, which the controller maintains. The entry
legs keep their hypotheses on the *amount*: `amount·10^k` fits, and for
supply the minted share count fits. These are about the call's input, not
stored state.

### Certora summaries in their own terms (`Summaries.lean`)

`IndexInRange m`: `10^24 ≤ m.supply_index ≤ 10^36`, `10^27 ≤
m.borrow_index ≤ 10^36` (what `nondet_market_index_raw` assumes);
`indexOf_inRange`: `PoolInv c → IndexInRange (indexOf c)`.

| Lean name | Summary | Claim (every return of the leg) |
|---|---|---|
| `supply_summary_holds` | `supply_summary` | `amount > 0 → new > old`, else `new = old`; `actual_amount = amount`; `IndexInRange`; decimals `≤ 18` |
| `borrow_summary_holds` | `borrow_summary` | `amount > 0` and `new > old`; `actual_amount = amount`; `IndexInRange`; decimals `≤ 18` |
| `withdraw_summary_partial` | `withdraw_summary` | `new ≥ 0`; `amount > 0 ∧ position > 0 → new < position`; `position ≤ 0 → new = position`; `0 ≤ actual ≤ amount`; `IndexInRange`; decimals `≤ 18` |
| `withdraw_zero_amount_closes_dust`, `withdraw_dust_instance` | `withdraw_summary` (gap) | `amount = 0` with half-up value `H = 0` closes the position (`new = 0`, `actual = 0`); instance: decimals 0, `si = RAY`, position 1 |
| `repay_summary_holds` | `repay_summary` | Exactly the summary's position clause; `0 ≤ actual ≤ amount`; `IndexInRange`; decimals `≤ 18` |
| `net_settle_summary_holds` | `net_settle_summary` | Exactly the summary's clauses; `IndexInRange` |
| `recapitalize_summary_holds` | `recapitalize_summary` | `0 ≤ actual ≤ amount` |
| `create_strategy_summary_holds` | `create_strategy_summary` | `new ≥ old`, `amount ≥ 0`, fee `f ∈ [0, amount]` (`0` without `charge_fee`), `actual = amount`, `received = amount − f`; `IndexInRange`; decimals `≤ 18` |
| `claim_revenue_summary_holds` | `claim_revenue_summary` | `0 ≤ actual ≤ cash` |

### Supporting lemmas (`Basic.lean`)

* `Ray` comparisons (`ray_le_eq`, `ray_min_eq`, ...);
* the checked `Ray` addition/subtraction (`ray_checked_add_spec`,
  `ray_checked_sub_eq_ok`, ...);
* `sat_sub_val` (`i128::saturating_sub` is subtraction when it fits);
* `isFloor_ediv`/`isCeil_exists`/`floor_fits`;
* `mul_div_ceil_eq_ok`: whatever `fp_core::mul_div_ceil` returns is the
  exact ceiling. This is partial correctness with no fit hypothesis; an
  unrepresentable ceiling aborts. `seize_apply_spec` uses it.

### Copied math (from RatesAeneas)

The rounding specifications (`Spec`, `FloorCeilSpec`, `HalfUpSpec`) are not
copied: they live once in `lean/FixedPoint/` and this library imports them.

`Host`, `MulDiv`, `Scaling`, `FloorCeilCore`, `FloorCeilRescale`,
`FloorCeilRoundTrip`, `HalfUp` and `HalfUpDivInt` are the files of the same
name in `lean/RatesAeneas/Proofs/` with the names changed
(`RatesAeneas`→`PoolAeneas`, `rates_aeneas`→`pool_aeneas`,
`aeneas_soroban_shim`→`aeneas_pool_shim`, `constants.production.`→`constants.`).
`../sync_copies.py` writes them, and `run.sh` fails if one is out of date.
`FloorCeilScaling`, `FloorCeilResolve` and `Withdrawal` are hand-adapted
copies with the same renaming, each with a provenance comment. They
supply `mul_div_floor_spec`, `mul_div_ceil_spec`, `mul_div_half_up_spec`,
`mul_div_floor_saturating_spec`, `ray_mul_floor_spec`, `ray_mul_ceil_spec`,
`ray_div_floor_spec`, `ray_div_ceil_spec`, `ray_to_asset_floor_spec`,
`unscale_supply_floor_spec` and `from_asset_spec`. The three added for the
legs supply `resolve_repay_spec`, `resolve_net_settle_spec` and
`resolve_withdrawal_spec` (with `exists_isFloor`/`exists_isCeil`).
`FloorCeilResolve` omits `i128_saturating_sub_val` and
`protocol_fee_shares_spec` (the pool has its own in `Accrual.lean`);
`Withdrawal` keeps only `resolve_withdrawal_spec` and its two arithmetic
lemmas, imports the pool's `RatesHalfUpWrappers`/`RatesIndex` and opens
`PoolAeneas.RatesProofs` for `ray_mul_spec`/`ray_to_asset_spec`.

For the accrual proofs, `RatesHalfUpWrappers`, `RatesCurveBase`, `RatesCurve`,
`RatesCurveRates` and `RatesIndex` are copies of `HalfUpWrappers`, `CurveBase`,
`Curve`, `CurveRates` and `Index` with the same renaming plus
`types.MarketParams`→`types.pool.MarketParams`. They are in the namespace
`PoolAeneas.RatesProofs`, because several names clash with pool-local lemmas
(for example `ray_checked_add_spec`, `ray_eq_eq`, `bps_val`). `RatesCurveRates` omits
`calculate_deposit_rate_spec` and `range_contains_eq` (`calculate_deposit_rate`
is not reachable from the pool roots). They supply
`calculate_borrow_rate_spec`, `utilization_spec`, `update_borrow_index_spec`,
`calculate_supplier_rewards_spec`, `ray_mul_spec` and the `Ray` `Ord`
min/max specs used by `accrue_step_spec`.

Pool-local differences in the copies:

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

### Certora summaries (`certora/shared/summaries/pool.rs`): discharged / partial / not

The table shows what these theorems discharge of each of the 13 summaries,
plus the shared index generator.

"Discharged" means: every successful return of the production leg satisfies
every postcondition the summary assumes on the returned values. This holds
on a market whose synced state satisfies `PoolInv` (and `ParamsOk` where
decimals or the fee rate are used), within the stated fit domains.

In every row:

* the token transfers, events and storage I/O around the leg are **not**
  covered (shimmed);
* the conditions on the synced market are hypotheses. Their inductiveness is
  shown separately: `PoolInv` by every leg and `synced_market_spec`;
  `ParamsOk` only by the unextracted `MarketParamsRaw::verify`.

| Summary | Status | Theorems | What is not discharged |
|---|---|---|---|
| `nondet_market_index_raw` | Discharged | `indexOf_inRange`; every leg's `PoolInv` of its committed cache; `synced_market_spec` (any elapsed time), `apply_bad_debt_spec`, `accrue_chunk_spec` | |
| `supply_summary` | Discharged | `supply_summary_holds`, `supply_apply_spec` | Decimals `≥ 0` is by type (`u32`), `≤ 18` from `ParamsOk` |
| `borrow_summary` | Discharged | `borrow_summary_holds`, `borrow_accounting_spec` | The outbound transfer of `actual_amount` (`borrow::apply`) |
| `withdraw_summary` | **Partial** | `withdraw_summary_partial`, `withdraw_accounting_spec`, `withdraw_zero_amount_closes_dust` | **The summary's `amount = 0 → new = position` is false** for dust positions (Findings 1). All other clauses hold. Transfer of `net_transfer` not covered |
| `repay_summary` | Discharged | `repay_summary_holds`, `repay_accounting_spec` | Refund transfer of `overpayment` |
| `net_settle_summary` | Discharged | `net_settle_summary_holds`, `net_settle_apply_spec` | |
| `update_indexes_summary` (empty) | Discharged | `synced_market_spec`, `global_sync_spec` | `ops::market::accrue` (loop over keys, events) not extracted |
| `recapitalize_summary` | Discharged | `recapitalize_summary_holds`, `recapitalize_accounting_spec` | Refund transfer |
| `flash_loan_summary` | Discharged (fee arithmetic) | `terms_spec`, `book_fee_spec`, `flash_finalize_keeps_ceiling` | `flash::apply` (callback, balance measurement, transfers) not extracted |
| `create_strategy_summary` | Discharged | `create_strategy_summary_holds`, `strategy_accounting_spec` | Transfer of `amount_received`, strategy-fee event |
| `seize_positions_summary` (empty) | Discharged per entry | `seize_apply_spec` | The batch (`run_batch`, `unique_markets = false`) and events not extracted; that the summary has no return value means nothing more is assumed |
| `claim_revenue_summary` | Discharged | `claim_revenue_summary_holds`, `revenue_accounting_spec` | Owner lookup and payout transfer (`revenue::apply`) |
| `get_sync_data_summary` | **Partial** | `PoolInv` preserved by every leg + `load_ok` + `synced_market_spec` | The state clauses (`supplied, borrowed, revenue, cash ≥ 0`, index ranges) are the inductive `PoolInv`. The rate-model clauses and `reserve_factor < BPS`, `flashloan_fee ≤ 500`, decimals range come from `MarketParamsRaw::verify` / `InterestRateModel::verify` at market creation and rate-model replacement (`ops::market`, not extracted); no extracted leg writes params. `get_sync_data` itself (a storage read in `lib.rs`) is not extracted |

## Rebuilding

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

The extraction roots and the `transfer_out` exclusion are read from
`[package.metadata.charon]` in `pool/Cargo.toml`. After Aeneas, run
`pool/postextract.sh ../lean/PoolAeneas` (`run.sh` does), which fixes the
duplicate `ZeroablePrimitive` field. The templates Aeneas writes list the
external declarations. Their meanings live in `FunsExternal.lean` and
`TypesExternal.lean`. So after a re-extraction, diff the templates against
those files before you delete them.

### Axiom check

`PoolAeneas.lean` ends with `#assert_standard_axioms PoolAeneas`
(`lean/Audit.lean`): `lake build PoolAeneas` fails unless every declaration
of the library depends only on `propext`, `Classical.choice` and
`Quot.sound` (so no `sorryAx`). The uninterpreted `storedParams`,
`storedState` and `ledgerNowMs` are Lean `opaque` definitions, not axioms,
and do not appear.

## Not covered

* **"Position ≤ market total" (INV-ACCT-10).** The withdraw, repay and
  net-settlement theorems assume that the value of the caller's position
  fits `i128`. With `ValueCeil` that follows from "the position is at most
  its market's total" (`*_keeps_ceiling_of_position_le`). That statement
  itself is not proved. The pool keeps no per-account book, and the
  controller, which does, is a separate model. It stays a hypothesis.
* **The flash loan before `finalize`.** `ops::flash::apply` (payout, the
  receiver callback, the two balance checks) is not extracted. So no proof
  here shows that `finalize` is reached only with the fee actually
  received.
* **Which accruals publish the ceiling event.** The `value_capped` flag is
  carried through the loop but no theorem relates it to the interest held.
* **`compound_interest` accuracy.** Only the bounds `RAY + x ≤ f ≤ 21·RAY`
  are proved, not the documented truncation error against `e^x`.
* **`simulate_update_indexes`** (the read-only projection) is not extracted;
  that it runs the same `accrue_step` is by construction.
* **Storage as state, events, token transfers and the flash-loan
  callback.** Storage reads and ledger time are uninterpreted: a read is not
  tied to an earlier write. So the leg theorems take the synced cache and
  `PoolInv` on it as hypotheses. Inductiveness is the chain "leg commits a
  `PoolInv` cache → `load_ok` → `synced_market_spec`". These are not
  extracted:
  * the `apply` wrappers (token transfers);
  * `run_batch` (the batch driver, its `unique_markets` check and batch
    events);
  * `ops::market` (create, rate-model replacement, `accrue`);
  * `ops::flash::apply` and the views.
* **Non-abort of the legs.** The leg theorems are about every successful
  return, which is what the summaries need. When a leg aborts (a guard
  fails) is not characterised beyond the guard specs (`Guards.lean`).
* **The controller half of INV-ACCT-10 and INV-ACCT-03** (writing the
  returned position; measuring the received balance).
* **`Cache::transfer_out`.** Excluded from extraction (token client).
* **Error codes.** Every abort is "failure"; which `CollateralError` /
  `GenericError` code is raised is not modelled.

## Audit findings S-03 and S-04

The Runtime Verification audit reported two pool findings. Both fixes are
in the extracted code, and the theorems below state what is proved about
them.

**Audit S-03 (utilization at a 100% ceiling).**

* `utilization_spec`: `utilization` never aborts on non-negative totals.
  Its result is in `[0, RAY]`. It returns `RAY` when
  `borrowed ≥ supplied > 0`, and divides only when `borrowed < supplied`.
* `require_utilization_below_max_spec`: the debt-mint guard passes exactly
  on `UtilPass` (`supplied = 0 ∨ B = 0 ∨ B·RAY ≤ max_utilization·S`). This
  holds at every ceiling, 100% included, where the condition reads `B ≤ S`.
  Hypotheses: share totals and indexes non-negative,
  `0 ≤ max_utilization ≤ RAY`, and `B`, `S` fit `i128`.
* `require_exit_utilization_below_max_spec`: the exit guard passes exactly
  on `max_utilization ≥ RAY ∨ UtilPass`, under the same hypotheses.
* `mint_debt_ok` and `borrow_accounting_spec` derive that the utilization
  guard passed after the mint.

**Audit S-04 (accrual overflow).**

* `accrue_step_spec`: one accrual step never aborts, with no hypothesis on
  the size of the market values. The accrued interest is at most the room
  below `MAX_MARKET_VALUE_RAY`. Hypotheses: `ValidRateModel`, non-negative
  share totals, indexes in range, and `delta ≤ MAX_COMPOUND_DELTA_MS`.
* `accrue_step_within_ceiling`: a step keeps both market totals at or below
  `MAX_MARKET_VALUE_RAY` when they start there.
* `global_sync_spec`: `global_sync` never aborts for any elapsed time on a
  cache in `AccrualDomain`. It keeps the cache in `AccrualDomain`.
* `synced_market_spec`: load plus accrual never aborts and keeps `PoolInv`,
  given a validated rate model.
* `ValueCeiling.lean`: every extracted leg keeps `ValueCeil`. The withdraw,
  repay and net-settlement theorems take "position ≤ market total" as a
  hypothesis (see "Not covered").

## Findings about the production code

1. **`withdraw_summary` is stricter than the code for zero-amount
   withdrawals of dust positions** (summary unsound in that corner, no
   economic impact). The summary assumes `amount = 0 → new_scaled =
   position`. The code compares the request with the position's half-up
   value `H = hu(hu(pos·si, RAY), 10^k)` and takes the full-close branch when
   `H ≤ amount`. For `amount = 0` and a position with `H = 0` it burns the
   whole position and pays `F = 0`. Concrete case (`withdraw_dust_instance`,
   `withdraw_zero_amount_closes_dust`): an asset with 0 decimals, supply
   index `RAY`, position of 1 scaled unit: `hu(1·RAY, RAY) = 1`,
   `hu(1, 10^27) = 0 = H ≤ 0`, so `burned = 1`, `gross = 0`, the
   `WithdrawRoundsToZeroShares` assert is skipped (gross is 0), and the
   returned position is 0, not 1. The position is worth `10^-27` units, so
   nothing of value moves. The user-facing `withdraw` maps `amount = 0` to
   `WITHDRAW_ALL_SENTINEL` (`contracts/controller/src/positions/supply.rs`).
   So the case is reachable only through controller paths that pass a
   literal 0 (`execute_withdrawal` callers, not checked here). Fix for the summary:
   in the else-branch allow `new_scaled = 0` when `amount == 0`
   (or `0 ≤ new_scaled ≤ position`).

No other counterexample was found. The proofs required the precisions
below:

* `burn_claimable_revenue` returns `0 > cash` if `cash` is negative. The claim
  `amount ≤ cash` therefore needs `cash ≥ 0` (INV-ACCT-02), which
  `credit_cash`/`debit_cash` maintain (`credit_cash_spec`, `debit_cash_spec`).
* `flash::terms` computes `amount + fee` with a checked add. `fee ≤ amount`
  makes it fit only when `2·amount` fits. So the theorem assumes
  `amount ≤ pre_balance` and `pre_balance + amount < 2^127`, which is always
  true for real token balances.
