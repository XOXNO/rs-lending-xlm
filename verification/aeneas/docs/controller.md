# Controller liquidation and risk math (Aeneas + Lean)

Machine-checked Lean 4 proofs about the production controller liquidation
math and the position valuation it relies on. Charon and Aeneas translate the
Rust; every theorem is about the generated `lean/ControllerAeneas/Funs.lean`.
Library: `ControllerAeneas`. Conventions and pitfalls: [../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

The liquidation-flow additions (Context/Map models, `risk/totals.rs`,
`calculate_repayment_amounts`, `apply.rs`) were extracted and checked on
2026-11-12/13 at `c5e6def0` (#225); against `8f476115` (#224) the generated
`Funs.lean` differs only in source-line references (#225 touched
`common/src/math/fp.rs` lines the controller does not reach).

| Rust file | Functions extracted | Status |
|---|---|---|
| `common/src/rates/value.rs` | `position_value`, `position_value_floor`, `position_value_ceil` | exact rounding + bracket proved |
| `common/src/math/fp.rs` | `Bps::apply_to_wad_floor` (and everything the functions below call) | exact floor proved |
| same (typed wrappers the liquidation math calls since #224) | `Ray::mul_wad_ratio_floor`, `Wad::apply_ratio_to_amount_floor`, `Bps::from_fraction_ceil`, `Bps::apply_to_ray_ceil` | exact rounding proved |
| `contracts/controller/src/positions/liquidation/curve.rs` | `is_socializable_bad_debt`, `max_bonus_for_threshold`, `LiquidationCurve::bonus_scale`, `calculate_linear_bonus_with_target` | proved |
| same | `max_hf_preserving_bonus_bps`, `liquidation_at_target`, `estimate_liquidation_amount` | extracted, not proved (see Not covered) |
| `contracts/controller/src/positions/liquidation/math.rs` | `split_seized_shares`, `scale_seizures_to_received`, `sum_repaid_usd` | proved |
| same | `calculate_repayment_amounts` | proved (caps, exact refunds, exact USD total) |
| same | `calculate_seized_collateral` | extracted; seizure budget (first three steps) proved, per-leg loop not proved (see Not covered) |
| same | `get_account_bonus_params`, `calculate_seizure_proportions`, `normalize_repayment_plan`, `whole_unit_repayment`, `release_unbacked_repayment`, `process_excess_payment`, `one_unit_per_*` | extracted, not proved (see Not covered) |
| same | `repays_every_debt_leg` | kept opaque (closure with a mutable capture, see Aeneas issues) |
| `contracts/controller/src/positions/liquidation/apply.rs` | `apply_liquidation_repayments`, `credited_shares`, `check_bad_debt_after_liquidation` | proved |
| same | `apply_liquidation_seizures`, `apply_liquidation_share_credit`, `credit_supply_shares` | extracted, not proved (their observable effects are external calls) |
| `contracts/controller/src/risk/totals.rs` | `calculate_account_risk_totals`, `calculate_ltv_collateral_wad`, `sum_debt_usd` (and `sum_debt_usd_loaded`, `portfolio_hub_keys`) | proved (exact sums, INV-RISK-02 conservativeness) |

Every theorem is **total correctness** (`f args ⦃ r => P r ⦄`: the call
returns, does not abort, and `P` holds) over **unbounded `i128` inputs**
subject only to the stated domain hypotheses. Specifications never divide:
`IsFloor`, `IsCeil` and `IsHalfUp` (`lean/FixedPoint/Spec.lean`,
`FloorCeilSpec.lean`, `HalfUpSpec.lean`, shared with the other libraries) are stated with
multiplications only.

## The extraction crate (`verification/aeneas/controller/`)

A standalone Cargo workspace (`[workspace]` table, members `.` and `shim`),
never part of the root workspace or any Wasm build.

**Included verbatim** through `#[path]` (nothing under `common/` or
`contracts/` is copied or edited):

* `common/src/constants/` (`src/constants.rs`) and
  `contracts/controller/src/constants.rs` (re-exported together, so both
  `common::constants::X` and `crate::constants::BAD_DEBT_USD_THRESHOLD`
  resolve);
* `common/src/math/{fp,fp_core}.rs` (`src/math.rs`; `fp_core` is a private
  module, as in `common/src/math/mod.rs` since #224, so the included
  controller files can reach it only through the typed `fp` wrappers);
* `common/src/rates/{scaling,value}.rs` (`src/rates.rs`, with the
  `pub use` lines of `rates/mod.rs`; `curve`, `index`, `compound`, `simulate`
  are not needed);
* `contracts/controller/src/positions/liquidation/{curve,math,apply}.rs`
  (`src/positions/liquidation/mod.rs`).

**Generated from production** (one mechanical substitution):

* `src/risk/totals.rs` is written by `controller/gen_totals.py` (run by
  `extract.sh`) from `contracts/controller/src/risk/totals.rs`: verbatim
  except that the `fn(&Env, Ray, Ray, Wad) -> Wad` argument of
  `sum_debt_usd_loaded` becomes an enum `DebtValue { HalfUp, Ceil }` whose
  `apply` calls exactly `position_value` / `position_value_ceil`, the two
  functions production passes. Aeneas cannot translate `fn` pointers with a
  higher-ranked lifetime (see Aeneas issues). Each of the four substitutions
  must match exactly once, so a production edit there fails the extraction.

`src/lib.rs` declares `extern crate self as common;` so the controller files'
`common::...` paths resolve to this crate.

**Stand-ins** (in the crate, compiled but either never extracted or
field-for-field copies):

* `src/types.rs` — the structs the included files use (`SpokeConfig`,
  `HubAssetKey`, `PriceFeed(Raw)`, `MarketIndex(Raw)`, `AccountPosition(Raw)`,
  `DebtPosition(Raw)`, `PaymentTuple`, `SeizeEntry`, `RepayEntry`,
  `LiquidationResult`, `Account` without `mode`), with exactly the production
  fields and the production `From` conversions copied verbatim. The production
  types are `#[contracttype]`, which only adds XDR serialisation.
* `src/errors.rs` — the error variants raised, with production codes (codes
  are not modelled; every `panic_with_error!` is an abort).
* `src/validation.rs` — `expect_invariant`, `require_nonneg_amount`,
  `require_non_empty_payments`, copied verbatim.
* `src/payment_legs.rs` (re-exported as `crate::payments`) —
  `ZeroLeg`, `aggregate_positive_payments`, `aggregate_payments`,
  `aggregate_payment_amount` copied verbatim from
  `contracts/controller/src/payments.rs`; `transfer_amount_measured` opaque.
* `src/context.rs` — the `Context` model (next section).
* `src/storage.rs` — `iter_typed_positions` / `iter_debt_positions`: the
  production `map.iter().map(|(k, raw)| (k, From::from(&raw)))` written as a
  loop that pushes the same items in the same order and returns the vector's
  iterator (Aeneas translates neither `impl Iterator` return types nor the
  `map` adapter).
* `src/types.rs` additionally carries `AccountPositionType`,
  `AggregatedPayments`, `ScaledPositionRaw` (with its two `From` impls),
  `PoolAction`, `PoolWithdrawEntry`, `PoolSeizeEntry`, `AssetConfig`,
  `Account::get_or_create_supply_position` (verbatim) and a field-less
  `PoolPositionMutation`; `HubAssetKey` derives `PartialEq, Eq` as in
  production.
* Opaque stand-ins (signatures only, Charon `--opaque`, meaning in
  `FunsExternal.lean`): `account_store.rs` (re-exported as `crate::account`:
  `cleanup_account_if_empty`, `update_or_remove_supply_position`),
  `external/{mod,pool}.rs` (`fetch_market_index`, `pool_seize_positions_call`),
  `positions/mod.rs` (`enforce_spoke_asset_flags`, `apply_repay_batch`,
  `apply_withdraw_batch`; `make_pool_action`, `get_supply_position_or_panic`,
  `FreezePolicy`, `WithdrawKind` are verbatim), `positions/liquidation/bad_debt.rs`
  (`execute_bad_debt_cleanup`).
* Never extracted (signatures for compilation): `risk/validation.rs`,
  `collections.rs` (`push_unique_address`, verbatim), `events.rs`,
  `spoke_usage.rs` (enums only).

`controller/extract.sh` lists the roots (`--start-from`) and the opaque items
(`--opaque`), and passes `--inline-anon-consts` (see Aeneas issues).
`controller/postextract.sh` applies one documented fix to the generated
`Funs.lean` (see Aeneas issues). For scratch runs, `AENEAS_LEAN_DEST=<dir>`
redirects the output and `KEEP_TEMPLATES=1` keeps the `*_Template.lean`
files.

**Shim** (`controller/shim/`, package `aeneas-soroban-shim`, copied from
`verification/aeneas/shim` and extended): `Env`, `I256`, `Error`,
`panic_with_error!` as in the rates crate, plus `assert_with_error!`,
`Address`, `Vec<T>` (with `VecIter<T>` and `IntoIterator`) and `Map<K, V>`.
Charon translates only signatures of shim items, so they are opaque to Aeneas
and get their meaning in the trusted base below.

## Trusted base

1. Charon and Aeneas translate faithfully (pinned release, see
   [../README.md](../README.md)).
2. `lean/ControllerAeneas/TypesExternal.lean` and `FunsExternal.lean`:
   * **`I256`** and the Rust core functions (`checked_pow`, `abs_diff`,
     `Option::{unwrap_or_else, and_then}`, `?` on `Option`): copied unchanged
     from `RatesAeneas` (exact integer arithmetic with the host's 256-bit traps).
   * **`i128::from(u32)`** (new): the same number (always fits).
   * **`Address`** (new): an opaque handle (`Nat`); `clone` is the identity.
   * **`Vec<T>`** (new): a Lean `List T`, index 0 at the front.
     `Vec::new` = `[]`; `push_back v x` = `v ++ [x]`; `Vec::iter` yields the
     elements front to back (`VecIter T` = the list of elements not yet
     yielded; `next` pops the head, `None` when empty); `clone` returns the
     same elements. This follows the soroban-sdk `Vec` documentation: a host
     object with value semantics (`clone` copies the handle, mutations rebind
     the variable). Not modelled: the host's `u32::MAX` length limit
     (`push_back` never fails here); the only theorem that pushes builds an
     output no longer than its input.
   * **`Vec<T>`, further methods** (new): `len` (as `u32`), `is_empty`,
     `get(i)` (`None` out of range), `set(i, x)` (traps out of range),
     `remove(i)` (shifts later elements down; `None` and unchanged out of
     range), `append`, `into_iter` (front to back).
   * **`Address` equality** (new): equality of the handles.
   * **`Map<K, V>`** (new, "Map model"): a Lean `List (K × V)` of entries in
     iteration order. `get k` = the value of the first entry whose key equals
     `k` (keys compared by value, decided classically), `set k v` replaces
     that entry's value or appends `(k, v)`, `keys`/`iter`/`len`/`is_empty`
     read the list. **What the proofs rely on:** only `get` after the list is
     given, and `iter`/`keys` yielding the list itself. **Not modelled:** the
     host's ascending key order and key uniqueness of arbitrary inputs;
     every theorem quantifies over all entry lists, so it holds for the
     host's (sorted, unique) ones in particular.
   * **Rust core** (new): `i128::saturating_mul` (exact product clamped to the
     `i128` range), `Option::map_or`, and `Iterator::{all, find}` (front to
     back, stopping at the first `false` / match), each per the Rust docs.
   * **Controller `Context`** (new, "Context model", `src/context.rs`): a
     struct with the two caches the extracted code reads, `token_prices`
     (`Map<Address, PriceFeedRaw>`) and `market_indexes`
     (`Map<HubAssetKey, MarketIndexRaw>`). `cached_price` is the production
     body (aborts when the asset is not cached); `cached_market_index` is the
     production body with the one-key pool fetch as one call. The rest of the
     host state is represented by uninterpreted Lean constants (`opaque`
     definitions, not axioms), so a theorem holds for every value they could
     stand for: `poolIndex` (the pool's current index for a key, used by
     `fetch_market_index`), `oraclePrice` (the aggregator's price, `None` =
     the fetch aborts), `poolAddress`, `spokeAssetConfig`, `spokeExitOk`,
     `spokeFlagsAllow`, `transferReceived` (the measured balance change of a
     transfer: any token behaviour). `load_markets` caches the missing price
     of each key's asset (aborting if the oracle has none) and the missing
     index of each key, keeping cached entries (production batches the same
     fetches). `cached_pool_address`, `require_spoke_asset`,
     `apply_spoke_exit`, `record_supply_position_update` and
     `enforce_spoke_asset_flags` leave the two caches unchanged (in
     production they only touch spoke usage, configuration caches, event
     buffers and the pool-address cache), aborting where production can
     abort. `transfer_amount_measured` aborts unless `amount > 0` and returns
     `transferReceived`. `apply_repay_batch`, `apply_withdraw_batch`,
     `pool_seize_positions_call`, `cleanup_account_if_empty`,
     `update_or_remove_supply_position`, `execute_bad_debt_cleanup` and
     `repays_every_debt_leg` are uninterpreted functions: theorems about
     their callers either do not depend on them or state the call explicitly
     (an equation).
3. `panic_with_error!`/`assert_with_error!` abort (as in `RatesAeneas`).

## Reused proofs

The rounding specifications (`Spec`, `FloorCeilSpec`, `HalfUpSpec`) are not
copied: they live once in `lean/FixedPoint/` and this library imports them.

Each extraction generates its own copy of the common math, so the proofs
about that math exist once per library. `Host`, `MulDiv`
(`mul_div_floor_spec`), `FloorCeilRescale` (`rescale_floor`/`rescale_ceil`),
`HalfUp` (`mul_div_half_up_spec`) and `HalfUpDivInt` (`div_by_int_half_up`)
are the RatesAeneas files with the library, crate and shim names changed,
written by `../sync_copies.py` (`run.sh` fails if one is out of date).
`FloorCeilCore` (`mul_div_ceil_spec`) is a hand-adapted copy: the
`mul_div_floor_saturating` section is omitted because it is not extracted
here.

Also copied (header comment in each): `Saturating.lean` (the
`mul_div_floor_saturating` section of `RatesAeneas/Proofs/FloorCeilCore.lean`)
and `Scaling.lean` (`RatesAeneas/Proofs/FloorCeilScaling.lean` up to
`unscale_borrow_ceil_spec`, without the `calculate_scaled_*` theorems;
`unscale_supply_floor_spec` renamed `unscale_supply_floor_isFloor` because
`Seizure.lean` has its own).

`Model.lean` proves the Map/Context facts the liquidation proofs use:
`map_get_eq`/`mapLookup`, `cached_price_ok`, `cached_market_index_ok`,
`load_markets_cached` (no change when every key is cached),
`iter_typed_positions_spec`, `iter_debt_positions_spec`,
`i128_from_u32_val`.

New supporting files: `Basic.lean` (comparison/min/conversion lemmas for
`Wad`/`Ray`/`Bps`, `checked_add_raw`, `checked_sub_nonneg`),
`Rounding.lean` (division-free relations between floor, half-up and ceiling),
`Wrappers.lean` (the typed `fp.rs` wrappers added in #224: unfolding
equations to the raw `mul_div_*` call, exact-rounding specs, and the
ratio-at-most-one lemmas `floor_ratio_fits`, `floor_ratio_le`,
`ceil_ratio_le`).

## Theorems

Notation: `R = 10^27` (RAY), `B = 10^9` (RAY→WAD factor), `W = 10^18` (WAD),
`BPS = 10^4`; `hu` = rounding to nearest, halves up.
Value-domain hypothesis **(VD)** for `scaled s`, `index i`, `price p`:
`s,i,p ≥ 0`, `s·i + R ≤ 2^127·R` and `(s·i + 2RB)·p + WRB ≤ 2^127·WRB`
(the exact value plus rounding slack fits `i128`).

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora |
|---|---|---|---|---|---|---|
| `position_value_floor_spec` | `Proofs/Value.lean` | `common/src/rates/value.rs::position_value_floor` | returns `⌊⌊⌊s·i/R⌋/B⌋·p/W⌋` | VD | INV-RISK-02 | — |
| `position_value_spec` | `Proofs/Value.lean` | `value.rs::position_value` | returns `hu(hu(hu(s·i/R)/B)·p/W)` | VD | INV-RISK-02 | — |
| `position_value_ceil_spec` | `Proofs/Value.lean` | `value.rs::position_value_ceil` | returns `⌈⌈⌈s·i/R⌉/B⌉·p/W⌉` | VD | INV-RISK-02 | — |
| `position_value_bracket` | `Proofs/Value.lean` | all three | all three return and `floor ≤ half-up ≤ ceil` | VD | INV-RISK-02 | `position_value_ceil_ge_floor` (hf_lemma_rules; bounded) |
| `position_value_floor_ceil_exact` | `Proofs/Value.lean` | floor, ceil | `floor·RBW ≤ s·i·p ≤ ceil·RBW`: floor never overstates and ceil never understates the exact value | VD | INV-RISK-02 | — |
| `apply_to_wad_floor_spec` | `Proofs/Value.lean` | `common/src/math/fp.rs::Bps::apply_to_wad_floor` | returns `⌊v·rate/BPS⌋` (single downward rounding; per-leg LTV/threshold weighting in `risk/totals.rs`) | `0 ≤ rate ≤ BPS`, `v ≥ 0` | INV-RISK-02 | — |
| `is_socializable_bad_debt_spec` | `Proofs/Curve.lean` | `liquidation/curve.rs::is_socializable_bad_debt` | never aborts; true iff `coll < debt ∧ coll ≤ 5 WAD` | none | INV-LIQ-04 | — |
| `max_bonus_for_threshold_spec` | `Proofs/Curve.lean` | `curve.rs::max_bonus_for_threshold` | `p ≤ 0` → 0; `p > 0` → `⌊BPS(BPS−e)/e⌋`, `e = clamp(⌈p·BPS/W⌉, 1, BPS)` | none (all `i128`) | INV-LIQ-02 | — |
| `max_bonus_for_threshold_bound` | `Proofs/Curve.lean` | same | `0 ≤ m ≤ BPS(BPS−1)` and `(BPS+m)·p ≤ BPS·W` (`(1+bonus)·proportion ≤ 1`) | `0 < p ≤ W` | INV-LIQ-02 | `derived_bonus_respects_threshold` |
| `max_bonus_for_threshold_antitone` | `Proofs/Curve.lean` | same | `0 < p₁ ≤ p₂` → `m(p₂) ≤ m(p₁)` | as stated | INV-LIQ-02 | — |
| `bonus_scale_spec` | `Proofs/Curve.lean` | `curve.rs::LiquidationCurve::bonus_scale` | returns `min(hu((target−hf)·W/(target−hf_max)), W)` | `0 ≤ hf_max < target ≤ 10 WAD`, `0 ≤ hf ≤ target` | — | — |
| `calculate_linear_bonus_spec` | `Proofs/Curve.lean` | `curve.rs::calculate_linear_bonus_with_target` | returns `b`: `b = base` if `hf ≥ target`; else `b = base + hu(hu((max−base)·scale/W)·factor/BPS)`; always `base ≤ b ≤ max` | validated curve (`0 ≤ hf_max < target ≤ 10 WAD`, `0 ≤ factor ≤ BPS`), `hf ≥ 0`, `0 ≤ base ≤ max` | INV-LIQ-02 | `bonus_bounded`, `bonus_is_base_at_or_above_target` |
| `calculate_linear_bonus_antitone` | `Proofs/Curve.lean` | same | `0 ≤ hf₁ ≤ hf₂` → `bonus(hf₂) ≤ bonus(hf₁)` | as above | INV-LIQ-02 | `bonus_monotone_in_hf` |
| `calculate_linear_bonus_max_below_floor` | `Proofs/Curve.lean` | same | `hf ≤ hf_max`, factor = BPS → bonus = `max` | as above | INV-LIQ-02 | `bonus_is_max_below_curve_floor` |
| `split_seized_shares_spec` | `Proofs/Shares.lean` | `liquidation/math.rs::split_seized_shares` | returns `(fee, liq)`, `fee = ⌈b·f/BPS⌉`, `fee + liq = S` exactly, `0 ≤ fee ≤ b`, `0 ≤ liq ≤ S`; the two internal re-checks are dead code | `0 ≤ b ≤ S`, `f < BPS` | INV-LIQ-02 | none for this function (`seizure_split_math` / `protocol_fee_bonus_math` in value_math_rules check the transfer-mode fee split, bounded) |
| `scale_seizures_to_received_spec` | `Proofs/Seizure.lean` | `math.rs::scale_seizures_to_received` | `planned ≤ 0 ∨ received ≥ planned` → input unchanged; otherwise same length, each entry related by `ScaledEntry`: shares, bonus, amount floored by `received/planned`, amount capped at floor value of new shares, fee floored and capped at new amount, other fields unchanged | `received ≥ 0`; on a short receipt each entry has decimals in `[3,18]`, `shares, index ≥ 0`, `shares·index < 2^127·R` | INV-LIQ-03 | none for this function (`liquidation_does_not_increase_seized_collateral` is an account-level rule) |
| `ScaledEntry.proportional` | `Proofs/Seizure.lean` | same (per entry) | `x'·planned ≤ x·received` for shares and amount, `x' ≤ x`, `fee' ≤ amount'`, `bonus ≤ scaled` preserved | `0 ≤ received < planned`, `shares, amount ≥ 0` | INV-LIQ-03 | — |
| `mul_wad_ratio_floor_spec` | `Proofs/Wrappers.lean` | `common/src/math/fp.rs::Ray::mul_wad_ratio_floor` | returns `⌊x·n/d⌋` (one floor; the WAD ratio is not quantised first) | `d ≠ 0`, the floor fits `i128` | INV-LIQ-03 | — |
| `mul_wad_ratio_floor_le` | `Proofs/Wrappers.lean` | same | returns `⌊x·n/d⌋ ∈ [0, x]` | `x ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | INV-LIQ-03 | — |
| `apply_ratio_to_amount_floor_spec` | `Proofs/Wrappers.lean` | `fp.rs::Wad::apply_ratio_to_amount_floor` (`self = n`) | returns `⌊a·n/d⌋` | `d ≠ 0`, the floor fits `i128` | INV-LIQ-03 | — |
| `apply_ratio_to_amount_floor_le` | `Proofs/Wrappers.lean` | same | returns `⌊a·n/d⌋ ∈ [0, a]` | `a ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | INV-LIQ-03 | — |
| `from_fraction_ceil_spec` | `Proofs/Wrappers.lean` | `fp.rs::Bps::from_fraction_ceil` | returns `⌈n·BPS/d⌉` | `d ≠ 0`, the ceiling fits `i128` | INV-LIQ-02 | — |
| `from_fraction_ceil_nonneg` | `Proofs/Wrappers.lean` | same | returns `⌈n·BPS/d⌉ ≥ 0` | `n ≥ 0`, `d > 0`, `n·BPS ≤ (2^127−1)·d` | INV-LIQ-02 | — |
| `apply_to_ray_ceil_spec` | `Proofs/Wrappers.lean` | `fp.rs::Bps::apply_to_ray_ceil` | returns `⌈v·b/BPS⌉` | the ceiling fits `i128` | INV-LIQ-02 | — |
| `apply_to_ray_ceil_le` | `Proofs/Wrappers.lean` | same | returns `⌈v·b/BPS⌉ ∈ [0, v]` | `v ≥ 0`, `0 ≤ b ≤ BPS` | INV-LIQ-02 | — |
| `mul_wad_ratio_floor_eq`, `apply_ratio_to_amount_floor_eq`, `from_fraction_ceil_eq`, `apply_to_ray_ceil_eq` | `Proofs/Wrappers.lean` | the four wrappers | each wrapper equals one raw call: `mul_div_floor(x, n, d)`, `mul_div_floor(a, n, d)`, `mul_div_ceil(n, BPS, d)`, `mul_div_ceil(v, b, BPS)` | none | — | — |
| `sum_repaid_usd_spec` | `Proofs/Sum.lean` | `math.rs::sum_repaid_usd` | returns exactly `Σ usd_wad` | every prefix sum fits `i128` | INV-LIQ-02 | — |
| `sum_repaid_usd_nonneg` | `Proofs/Sum.lean` | same | returns `Σ usd_wad ≥ 0` | all `usd_wad ≥ 0`, total `< 2^127` | INV-LIQ-02 | — |
| `calculate_account_risk_totals_spec` | `Proofs/Totals.lean` | `risk/totals.rs::calculate_account_risk_totals` | returns, context unchanged; `total_collateral = Σ huv`, `ltv_collateral = Σ ⌊fv·min(LTV,LT)/BPS⌋`, `weighted_collateral = Σ ⌊fv·LT/BPS⌋`, `total_debt = Σ cv` (borrow index), `health_factor = i128::MAX` without debt, else `min(i128::MAX, ⌊weighted·W/debt⌋)` | TOT | INV-RISK-02 | — (Certora summarises this function outside the health/solvency rules) |
| `calculate_account_risk_totals_conservative` | `Proofs/Totals.lean` | same | `weighted·BPS·10^54 ≤ Σ LT·s·i·p`; `ltv·BPS·10^54 ≤ Σ min(LTV,LT)·s·i·p`; `ltv ≤ weighted ≤ total_collateral`; `Σ fv ≤ total_collateral ≤ Σ cv`; `Σ s·i·p ≤ total_debt·10^54` | TOT | INV-RISK-02 | `position_value_ceil_ge_floor` (per leg, bounded) |
| `calculate_ltv_collateral_wad_spec` | `Proofs/Totals.lean` | `totals.rs::calculate_ltv_collateral_wad` | returns `Σ ⌊fv·min(LTV,LT)/BPS⌋` (same as the totals' `ltv_collateral`), context unchanged | TOT (supply half) | INV-RISK-02 | — |
| `sum_debt_usd_spec` | `Proofs/Totals.lean` | `totals.rs::sum_debt_usd` | returns `Σ huv` (borrow index) `≤ Σ cv`, context unchanged | TOT (debt half) | INV-RISK-02 | — |
| `totals_loop_spec`, `ltv_loop_spec`, `debt_loop_spec` | `Proofs/Totals.lean` | the three generated loops | exact accumulation, context unchanged | per-leg TOT, accumulator + remaining ceiling sum `< 2^127` | INV-RISK-02 | — |
| `calculate_repayment_amounts_spec` | `Proofs/Repayment.lean` | `liquidation/math.rs::calculate_repayment_amounts` | for the merged legs `P`: returns, context unchanged; one entry per leg with `amount = min(offered, D)`, `usd_wad = legUsd` (floor value `fv` of the whole debt when the leg pays `D`, else `hu(offered·10^(18−dec)·p/W)`), feed/index snapshot; one refund `offered − D` per overpaid leg, in order; total `= Σ legUsd` | REP; `aggregate_positive_payments` returned `P` | INV-LIQ-02 | `liquidation_does_not_increase_repaid_debt` (account level, bounded) |
| `repay_entry_le_debt` | `Proofs/Repayment.lean` | same (per entry) | `amount ≤ D` and `amount ≤ offered` | an entry of the spec | INV-LIQ-02 | `liquidation_does_not_increase_repaid_debt` |
| `repay_kept_plus_refund` | `Proofs/Repayment.lean` | same (per leg) | overpaid leg: `kept + refund = offered`; otherwise `kept = offered` (excess refunded exactly) | an entry/refund of the spec | INV-LIQ-02 | — |
| `repay_loop_spec`, `usd_value_wad_spec`, `from_token_spec` | `Proofs/Repayment.lean` | loop; `PriceFeed::usd_value_wad`; `Wad::from_token` | loop invariant; `hu(a·10^(18−dec)·p/W)`; exact upscale `a·10^(18−dec)` | as stated | INV-LIQ-02 | — |
| `apply_liquidation_repayments_eq` | `Proofs/Apply.lean` | `liquidation/apply.rs::apply_liquidation_repayments` (`offered = None`) | equals: one pool action per leg repaying with the *received* amount, then the batch repay; returns `u = Σ legRecv`, `legRecv = usd_wad` if `received ≥ amount`, else `⌊usd_wad·received/amount⌋` | APP | INV-LIQ-03 | — |
| `legRecv_le_planned`, `recvLeg_bounds` | `Proofs/Apply.lean` | same | `0 ≤ u ≤ Σ usd_wad`; per leg `0 ≤ legRecv ≤ usd_wad` and `legRecv·amount ≤ usd_wad·received` on a short receipt | APP | INV-LIQ-03 | — |
| `underdelivery_scales_seizures` | `Proofs/Apply.lean` | `apply_liquidation_repayments` → `scale_seizures_to_received` | `u ≤ planned`; seizures unchanged when all arrived, else every entry floored by `u/planned` (`ScaledEntry`) | APP, planned `= Σ usd_wad`, seizure entries as in `scale_seizures_to_received_spec` | INV-LIQ-03 | `liquidation_does_not_increase_seized_collateral` (account level) |
| `credited_shares_spec` | `Proofs/Apply.lean` | `apply.rs::credited_shares` | returns `S − ⌈bonus·fee/BPS⌉ ∈ [0, S]` | `0 ≤ bonus ≤ S`, fee `< BPS` | INV-LIQ-02 | — |
| `check_bad_debt_after_liquidation_eq` | `Proofs/Apply.lean` | `apply.rs::check_bad_debt_after_liquidation` | equation: no debt left → only the empty-account cleanup; else socialization (`execute_bad_debt_cleanup` on the whole account) exactly when `coll < debt ∧ coll ≤ 5 WAD`; else nothing | none | INV-LIQ-04 | `bad_debt_socialization_threshold_boundary` |
| `seizure_budget_spec` | `Proofs/Apply.lean` | `math.rs::calculate_seized_collateral` (its `one_plus_bonus` / `total_seizure_usd` steps) | `total_seizure_usd = hu(repay·(BPS+bonus)/BPS)` | `repay ≥ 0`, `0 ≤ bonus ≤ BPS`, result fits | INV-LIQ-02 | — |
| `seizure_budget_exceeds_exact` | `Proofs/Apply.lean` | same | concrete case `repay = 10`, bonus 5%: budget 11 > exact 10.5 (rounding not in the protocol's favour, by < 1 WAD unit) | none | INV-LIQ-02 | — |
| `liq_bps_to_wad_val`, `div_floor_saturating_val` | `Proofs/Apply.lean`, `Proofs/Totals.lean` | `Bps::to_wad`, `Wad::div_floor_saturating` | `b·10^14`; `min(i128::MAX, ⌊a·W/b⌋)` for `a ≥ 0`, `b > 0` | as stated | — | — |

Hypothesis groups of the new rows (each per entry of the respective map or list):

* **TOT** (`RawSupplyOK` / `RawDebtOK`): the context serves the key (price of
  its asset and index cached; true after `load_markets`, which the functions
  call first and which then changes nothing); VD for (scaled, supply or
  borrow index, price); stored LTV and liquidation threshold `≤ BPS`; the
  ceiling values of all supply legs, and of all debt legs, each sum below
  `2^127`.
* **REP** (`RepayLegOK`): the context serves the key; the account has that
  debt position (production aborts with `DebtPositionNotFound` otherwise);
  VD for the debt; `decimals ≤ 18`; offered amount `> 0`; for a leg offered
  below its cap, `offered·10^(18−dec)` and its USD value fit `i128`; the leg
  values sum below `2^127`.
* **APP** (`ApplyLegOK`): the spoke flags allow the exit; planned amount
  `> 0` and `usd_wad ≥ 0` (plan validation); the measured receipt is
  non-negative; the account has the debt position; `Σ usd_wad < 2^127`.

### Why the hypotheses hold in production

* **VD** — scaled balances, indexes and prices are non-negative `Ray`/`Wad`
  values; the bound only asks that the exact USD value (plus at most
  `2·10^-18`-scale slack) fits `i128` (about `1.7·10^20` USD), far above any
  position.
* **Curve** — `validate_liquidation_curve` (`common/src/validation.rs`)
  enforces `WAD < target ≤ MAX_LIQUIDATION_TARGET_HF_WAD = 10 WAD`,
  `0 < hf_for_max_bonus < target` and `0 < bonus_factor ≤ BPS` on every stored
  curve; `estimate_liquidation_amount` passes `curve.target_hf` as `target`.
  `base = min(weighted bonus, max)` in `get_account_bonus_params` with both
  non-negative, so `0 ≤ base ≤ max`. Health factors are non-negative.
* **Seizure proportion** — `weighted / total` collateral with every
  liquidation threshold at most 100%, so `0 < p ≤ W` whenever collateral is
  positive.
* **Shares** — `LiquidationPlan::validate` rejects entries with
  `bonus_scaled > scaled_amount` or negative values, and stamped fee rates are
  below `BPS`.
* **Seizures** — `scale_seizures_to_received`'s own doc requires decimals
  `≥ MIN_BORROWABLE_ASSET_DECIMALS` (it aborts otherwise, as the theorem
  shows by requiring it); `MAX_ASSET_DECIMALS = 18`; received USD is
  non-negative.
* **Repayment legs** — every `usd_wad` is a floor or half-up valuation of a
  non-negative amount at a non-negative price.

## Rebuilding

```sh
export AENEAS_HOME=/path/to/aeneas-nightly-2026.11.12-0d21a9e   # or your unpacked release
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas
ALLOW_DRIFT=1 ./run.sh controller   # extract.sh, then postextract.sh, then lake build ControllerAeneas
```

`extract.sh` runs `gen_totals.py`, then `charon cargo --preset=aeneas
--inline-anon-consts --dest-file target/llbc/controller.llbc --start-from ...
--opaque ...` and `aeneas -backend lean -dest ../lean -subdir
ControllerAeneas -split-files target/llbc/controller.llbc` (the `-subdir`
makes the generated imports `ControllerAeneas.*`), then deletes the
`*_Template.lean` files. `run.sh` then runs `postextract.sh` on the
generated library (Aeneas issue 3).

Axioms: `ControllerAeneas.lean` ends with
`#assert_standard_axioms ControllerAeneas` (`lean/Audit.lean`), so the build
fails unless every declaration of the library depends only on `propext`,
`Classical.choice` and `Quot.sound`.

Stand-ins: the copied production items in `controller/src` (`types.rs`,
`cached_price`, the payment aggregation, `make_pool_action`, ...) are
compared with production by `../check_standins.py`, which `run.sh` runs
first; an adapted stand-in pins the hash of the production item it replaces.

## Not covered

* **Now proved in their own documents:** `estimate_liquidation_amount`,
  `liquidation_at_target`, `max_hf_preserving_bonus_bps`
  ([controller-estimate.md](controller-estimate.md));
  `calculate_seized_collateral` per leg, `get_account_bonus_params`,
  `calculate_seizure_proportions` and the post-liquidation health factor
  ([controller-seizure.md](controller-seizure.md));
  `process_excess_payment`, `release_unbacked_repayment`,
  `normalize_repayment_plan`, `whole_unit_repayment`, `one_unit_per_*`
  ([controller-repayment.md](controller-repayment.md)). Each lists what it
  still leaves out.
* **`repays_every_debt_leg`** — kept opaque: its `Iterator::all` closure
  mutably borrows the context, which the Aeneas library's `FnMut` model cannot
  type (see Aeneas issues). So the `repays_all_debt` flag is arbitrary in the
  model.
* **`apply_liquidation_seizures`, `apply_liquidation_share_credit`** —
  extracted, not proved: their effects are the external batch withdraw /
  account-map updates and pool calls (uninterpreted). Share conservation of
  the credit split is proved per entry (`split_seized_shares_spec`,
  `credited_shares_spec`).
* **`apply_liquidation_repayments` with `offered = Some(..)`** (full-close
  plans) — the pull amount is looked up with `Iterator::find`; the credited
  value per leg is the same function of the receipt, but the theorem is
  stated for `offered = None` only.
* **`aggregate_positive_payments`** — included verbatim and extracted; the
  repayment theorem takes its result as given (a hypothesis naming the merged
  list) instead of proving the aggregation.
* **Error codes** — every abort is just an abort; codes are checked by Kani /
  tests, not here.
* **`Vec` length limit** — not modelled (see Trusted base).
* **Map key order** — not modelled; no theorem depends on it.

## Aeneas issues (and how the extraction avoids them)

1. **Internal error on `process_excess_payment`** (`InterpAbs.ml` line 940,
   reported as `Interp.ml` line 686). Minimal reproduction, a loop whose
   condition compares a derived-`PartialOrd` newtype against a temporary and
   short-circuits:

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

   In production the condition is `remaining_excess_usd > Wad::ZERO &&
   current_index > 0`. Charon's `--inline-anon-consts` avoids it (so does
   `--no-code-duplication`, which breaks other functions); `extract.sh`
   passes `--inline-anon-consts`, which leaves the translation of every
   previously extracted function unchanged (checked by diffing `Funs.lean`).
2. **`fn` pointers with a higher-ranked lifetime are not supported** ("arrow
   types with locally quantified regions"), nor the coercion of a function
   item to one. Avoided by `gen_totals.py` (see the crate section). Passing
   `position_value` as a generic `F: Fn(&Env, ..)` fails too ("lifetime
   constraint relating a higher-ranked lifetime to a free lifetime").
3. **Provided `Iterator` methods emitted as instance fields.** For the shim's
   `impl Iterator for VecIter<T>`, Aeneas puts `all` and `find` into the trait
   instance although the Lean library's `Iterator` structure has no such
   fields (it warns about it), so `Funs.lean` does not compile. The calls go
   to `core.iter.traits.iterator.Iterator.{all,find}.default` directly, so
   `controller/postextract.sh` deletes exactly these two field definitions
   from that one instance and fails if the instance has another shape.
4. **Parameter names shadow module namespaces.** A function with a parameter
   named `account` (or `payments`) that calls `crate::account::f` is emitted
   as `account.f`, which Lean resolves as field access on the parameter. The
   crate defines those items in `account_store` / `payment_legs` and
   re-exports them as `crate::account` / `crate::payments`, so the generated
   names differ from the parameters.
5. **Closures with a mutable capture.** `repays_every_debt_leg`'s
   `Iterator::all` closure captures `cache: &mut Context`; the generated
   `FnMut` instance has type `closure → Item → Result (Bool × closure ×
   (closure → closure))`, which does not match the library's `FnMut`. The
   function is kept opaque.

## Changes after #224 (typed fixed-point boundary)

Commit `8f476115` made `common::math::fp_core` private and routed the
controller liquidation math through new typed wrappers in
`common/src/math/fp.rs`. The controller extraction was re-run and the proofs
re-checked against it.

* **No theorem statement changed.** Each new call site is the same raw
  primitive with the same arguments in the same order, so every exact value
  stated above is unchanged:
  * `split_seized_shares`: `mul_div_ceil(bonus, fees, BPS)` became
    `Bps::from(fees).apply_to_ray_ceil(bonus)` = `mul_div_ceil(bonus, fees, BPS)`.
  * `max_bonus_for_threshold`: `mul_div_ceil(p, BPS, WAD)` became
    `Bps::from_fraction_ceil(p, Wad::ONE)` = `mul_div_ceil(p, BPS, WAD)`.
  * `scale_seizures_to_received`: each `mul_div_floor(x, received, planned)`
    became `Ray::mul_wad_ratio_floor(x, received, planned)` (shares, bonus
    shares) or `received.apply_ratio_to_amount_floor(planned, x)` (amount,
    protocol fee); both are `mul_div_floor(x, received, planned)`, one floor
    with the unquantised ratio, as before. The generated loop now takes the
    two `Wad`s instead of their raw values.
* **Proof changes.** `Shares`, `Curve` and `Seizure` rewrite the wrapper
  calls with the `*_eq` lemmas of `Proofs/Wrappers.lean` and then use the raw
  `mul_div_*` specs exactly as before. The per-entry helper `scaleEntry`
  (`Proofs/Seizure.lean`) now mirrors the new loop body (wrapper calls);
  `floor_ratio_fits` moved from `Seizure.lean` to `Wrappers.lean`.
* **New specs** for the four wrappers the extracted code now calls (rows
  above).
* **Extraction crate.** `controller/src/math.rs` declares `fp_core` private,
  as production does; the generated Lean is identical either way.
* **Not extracted, checked by reading.** The other #224 edits in the
  controller (`whole_unit_repayment`: `Wad::div_floor` / `Wad::div_ceil` by
  `1 + bonus`; `get_account_bonus_params`: `Bps::from_weighted_sum`;
  `apply.rs`: `Wad::mul_ratio_floor`) are each the same `fp_core` call with the
  same arguments as before; they are outside the extracted set (see Not
  covered).

## Production code review notes

No economically relevant counterexample was found. One rounding direction
does not favour the protocol, by less than one WAD unit (`10^-18` USD):

* **The seizure budget of `calculate_seized_collateral` rounds half up.**
  `total_seizure_usd = repay_usd.mul(1 + bonus)` is `hu(repay·(BPS+bonus)/BPS)`
  (`seizure_budget_spec`). With `repay_usd = 10` WAD units and a 500 bps bonus
  the exact budget is 10.5 units and the code budgets 11
  (`seizure_budget_exceeds_exact`). The per-leg share (`Wad::div`), leg
  budget (`Wad::mul`) and price division (`Wad::div`) are also half up. The
  seizure is still capped by the held balance, and the excess is a fraction
  of a WAD unit per rounding step, so this is a documentation point (any
  claim "seizure ≤ repaid × (1 + bonus)" holds only up to these units), not
  an exploitable flaw.

Observations made while proving:

* `split_seized_shares`: the re-checks `fee_scaled > seized_scaled` and
  `fee_scaled + liquidator_scaled != seized_scaled` can never fire once the
  entry checks pass (`fee ≤ bonus ≤ seized`; the sum is exact). They are
  harmless defence in depth.
* `max_bonus_for_threshold` accepts any `i128` proportion without aborting;
  for `p > W` the effective threshold clamps to `BPS` and the bonus is 0.
