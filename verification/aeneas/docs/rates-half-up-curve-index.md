# Half-up rounding, the rate curve, and index accrual

Machine-checked Lean proofs (Aeneas extraction, library `RatesAeneas`) about
the half-up arithmetic in `common/src/math`, the interest-rate curve in
`common/src/rates/curve.rs` and the index updates in
`common/src/rates/index.rs`. Each theorem is stated about the generated
`lean/RatesAeneas/Funs.lean` definition of the named Rust function. Overview
and conventions: [../README.md](../README.md), [../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

* `fp_core::try_mul_div_half_up`, `fp_core::mul_div_half_up`: exact half-up
  rounding over the whole `i128` domain, native and `I256` paths, with an
  exact characterisation of when `mul_div_half_up` aborts.
* `fp_core::div_by_int_half_up`: round to nearest, ties away from zero, every
  `i128` numerator; aborts exactly on `b ≤ 0`.
* `fp_core::rescale_half_up` downscaling (including the branch where
  `10^(from - to)` overflows `i128` and the code returns 0).
* Wrappers: `Ray::mul`, `Ray::div`, `Ray::try_div`, `Ray::try_mul`, `Ray::to_asset`,
  `Bps::apply_to`, `Bps::apply_to_ray`, `Bps::flash_loan_fee_on`, and the
  half-up wrappers added by #224: `Bps::div`, `Bps::try_from_fraction`,
  `Bps::from_weighted_sum`, `Wad::try_value_of_token`, and `Ray::mul_ray_ratio`
  (added by #225).
* `rates/curve.rs`: `utilization`, `slope_contribution`,
  `calculate_annual_borrow_rate` (exact value, bounds, kink values,
  monotonicity across the whole curve), `calculate_borrow_rate`,
  `calculate_deposit_rate`.
* `rates/index.rs`: `update_borrow_index`, `update_supply_index`,
  `calculate_supplier_rewards`, `supply_index_reward_shortfall`.

All statements are total correctness (`f ⦃ r => P r ⦄`: the call returns and
`P` holds) except where a theorem says otherwise. "Aborts" is stated as
`¬ (f ⦃ _ => True ⦄)`: the call never returns. Specifications never divide
(`lean/FixedPoint/HalfUpSpec.lean`):

* `IsHalfUp n d q` (`d > 0`): `2·q·d ≤ 2·n + d < 2·q·d + 2·d`, i.e. `n/d`
  rounded to nearest with exact halves up (equivalently
  `IsFloor (2n + d) (2d) q`, `IsHalfUp.iff_isFloor`).
* `IsHalfAway n d q` (`d > 0`): `IsHalfUp n d q` for `n ≥ 0` and
  `IsHalfUp (-n) d (-q)` for `n < 0`: nearest, exact halves away from zero.

Notation: `RAY = 10^27`, `BPS = 10_000`, `MAX = 10^36`
(`MAX_BORROW_INDEX_RAY = MAX_SUPPLY_INDEX_RAY`), `MPY = 31_556_926_000`
(`MILLISECONDS_PER_YEAR`), `hu(n, d)` = the `q` with `IsHalfUp n d q`.

## Trusted base

No additions (#224 added no external declaration to this library). The proofs use only the generated code, the existing
`FunsExternal.lean` / `TypesExternal.lean` meanings (host `I256`,
`checked_pow`, `abs_diff`, `Range::contains`, `Option` combinators) and the
Aeneas standard library. Two generic helper lemmas about the Aeneas `Result`
monad are proved, not assumed: `spec_imp_eq_ok` (a total-correctness triple
pins the result to `ok v`; by induction on the `DWP` least fixed point, using
that failure is the only effect) and `spec_bind_inv` (bind inversion). No
`sorry`, `admit` or `axiom`. Every headline theorem below was checked with
`#print axioms`: each depends only on `propext`, `Classical.choice`,
`Quot.sound`.

## Rate-model hypotheses

`ValidRateModel p` (`Proofs/Curve.lean`) is part of what
`InterestRateModel::verify` (`common/src/types/pool.rs`) checks, on the fields
`MarketParams` carries: `0 ≤ base`, `base ≤ slope1`, `0 ≤ slope2`,
`0 ≤ slope3`, `slope3 ≤ max`, `base < max`, `max ≤ MAX_BORROW_RATE_RAY = 2·RAY`,
`base + slope1 + slope2 + slope3 ≤ max`, `0 < mid < optimal < RAY`,
`optimal ≤ max_utilization ≤ RAY`, `0 ≤ reserve_factor < BPS`. `verify` also
requires `slope1, slope2 ≤ max` and steepness per unit of utilization that
does not fall at either kink (audit I-08); no theorem needs those, so every
theorem holds for more models than `verify` admits. Before I-08, `verify`
ordered the slopes themselves (`slope1 ≤ slope2 ≤ slope3`) and this
predicate did too; the proofs only ever used the non-negativity that
ordering implied. `verify` runs on market creation and every parameter update
(`contracts/pool/src/ops/market.rs`, `contracts/governance/src/validate/asset.rs`,
`contracts/governance/src/op.rs`).

## Theorems

| Lean name | File | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|---|
| `isHalfUp_tdiv` | `HalfUpSpec.lean` | (core lemma) | `trunc((n + trunc(d/2)) / d)` is `hu(n, d)` | `n ≥ 0`, `d > 0` | — |
| `IsHalfUp.unique`, `.mono`, `.anti_div`, `.exists`, `.le_of_ratio_le_one` | `HalfUpSpec.lean` | (spec lemmas) | half-up is unique, monotone in `n`, antitone in `d`, exists; `0 ≤ y ≤ d ⇒ 0 ≤ hu(x·y, d) ≤ x` | as stated | — |
| `try_mul_div_half_up_spec` | `HalfUp.lean` | `math::fp_core::try_mul_div_half_up` | never aborts; `None` if `x<0`, `y<0` or `d≤0`; otherwise `Some q` with `q = hu(x·y, d)` when that fits `i128`, `None` exactly when it does not | none | — |
| `mul_div_half_up_spec` | `HalfUp.lean` | `math::fp_core::mul_div_half_up` | returns exactly `hu(x·y, d)` | `x, y ≥ 0`, `d > 0`, result fits | — |
| `mul_div_half_up_returns_iff` | `HalfUp.lean` | `math::fp_core::mul_div_half_up` | returns **iff** `x ≥ 0 ∧ y ≥ 0 ∧ d > 0 ∧` the result fits; aborts otherwise (`d = 0`, negative operand via the `debug_assert`, overflow) | none | — |
| `div_by_int_half_up_spec` | `HalfUpDivInt.lean` | `math::fp_core::div_by_int_half_up` | never aborts, returns `q` with `IsHalfAway a b q`, every `i128` `a` | `b > 0` (call sites: `MPY`, powers of ten) | — |
| `div_by_int_half_up_aborts` | `HalfUpDivInt.lean` | `math::fp_core::div_by_int_half_up` | aborts | `b ≤ 0` | — |
| `rescale_half_up_down_spec` | `HalfUpWrappers.lean` | `math::fp_core::rescale_half_up` | downscaling returns `q` with `IsHalfAway a (10^(from-to)) q`, never aborts, every `i128` `a`, every gap (gap ≥ 39 returns 0, which is that rounding) | `to < from` | — |
| `ray_to_asset_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::to_asset` | `IsHalfAway ray (10^(27-dec)) q`, never aborts | `dec < 27` (production `≤ MAX_ASSET_DECIMALS = 18`) | — |
| `ray_mul_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::mul` | returns `hu(a·b, RAY)` | `a, b ≥ 0`, result fits | — |
| `ray_div_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::div` | returns `hu(a·RAY, b)` | `a ≥ 0`, `b > 0`, result fits | — |
| `ray_try_div_spec` (new, #224) | `HalfUpWrappers.lean` | `math::fp::Ray::try_div` | never aborts; `None` if `a < 0` or `b ≤ 0`; otherwise `Some(hu(a·RAY, b))` when it fits, `None` exactly when it does not | none | — |
| `ray_try_mul_spec` (new, S-04 fix) | `HalfUpWrappers.lean` | `math::fp::Ray::try_mul` | never aborts; `None` for a negative operand; otherwise `Some(hu(a·b, RAY))` when it fits, `None` exactly when it does not | none | INV-IDX-06 |
| `ray_mul_ray_ratio_spec` (new, #225) | `TypedWrappers.lean` | `math::fp::Ray::mul_ray_ratio` | returns exactly `hu(x·n, d)` (one rounding; the ratio `n/d` is not quantized) | `x, n ≥ 0`, `d > 0`, result fits | — |
| `ray_mul_ray_ratio_returns_iff` (new, #225) | `TypedWrappers.lean` | `math::fp::Ray::mul_ray_ratio` | returns **iff** `x ≥ 0 ∧ n ≥ 0 ∧ d > 0 ∧` the result fits; aborts otherwise | none | — |
| `ray_mul_ray_ratio_frac_spec` (new, #225) | `TypedWrappers.lean` | `math::fp::Ray::mul_ray_ratio` | never aborts; returns `hu(x·n, d) ∈ [0, x]` | `x ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | — |
| `bps_div_spec` (new, #224) | `TypedWrappers.lean` | `math::fp::Bps::div` | returns `hu(a·BPS, b)` | `a ≥ 0`, `b > 0`, result fits | — |
| `bps_try_from_fraction_spec` (new, #224) | `TypedWrappers.lean` | `math::fp::Bps::try_from_fraction` | never aborts; `None` if `num < 0` or `den ≤ 0`; otherwise `Some(hu(num·BPS, den))` when it fits, `None` exactly when it does not | none | — |
| `bps_from_weighted_sum_spec` (new, #224) | `TypedWrappers.lean` | `math::fp::Bps::from_weighted_sum` | never aborts; `IsHalfAway sum weight q`, every `i128` sum | `weight > 0` | — |
| `wad_try_value_of_token_spec` (new, #224) | `TypedWrappers.lean` | `math::fp::Wad::try_value_of_token` | never aborts; `None` if `decimals > 38` or a negative operand; otherwise `Some(hu(amount·price, 10^decimals))` when it fits, `None` exactly when it does not | none | — |
| `bps_apply_to_spec` | `HalfUpWrappers.lean` | `math::fp::Bps::apply_to` | never aborts, returns `hu(amount·bps, BPS) ∈ [0, amount]` | `amount ≥ 0`, `0 ≤ bps ≤ BPS` | INV-IDX-05 |
| `bps_apply_to_ray_spec` | `HalfUpWrappers.lean` | `math::fp::Bps::apply_to_ray` | same for a ray value | `v ≥ 0`, `0 ≤ bps ≤ BPS` | INV-IDX-05 |
| `flash_loan_fee_on_spec` | `HalfUpWrappers.lean` | `math::fp::Bps::flash_loan_fee_on` | never aborts; `0 ≤ fee ≤ amount`; `fee ≥ 1` when `bps > 0`; `fee = hu(amount·bps, BPS)` unless that is 0 with `bps > 0`, then 1 | `amount ≥ 1`, `0 ≤ bps ≤ BPS` (`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`, `verify`) | INV-FLASH-01 (the fee `F`) |
| `utilization_spec` (restated, S-03 fix) | `CurveBase.lean` | `rates::curve::utilization` | never aborts; the result is in `[0, RAY]`; `0` when `supplied = 0`; `RAY` when `borrowed ≥ supplied > 0` (by comparison, no division); `hu(b·RAY, s)` when `borrowed < supplied` | `b, s ≥ 0` (pool totals) | INV-ACCT-08 |
| `slope_contribution_spec` (new, #225) | `Curve.lean` | `rates::curve::slope_contribution` | never aborts; returns `c = hu(slope·excess, range)` (`SegContrib`, one rounding) with `0 ≤ c ≤ slope` | `0 ≤ excess ≤ range`, `range > 0`, `slope ≥ 0` | — |
| `calculate_annual_borrow_rate_spec` (restated, #225) | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | never aborts; returns exactly the capped three-segment formula at `m = min(u, RAY)` (`AnnualRate`): `min(base + slopes below + hu(slope·excess, range), max)`, one half-up rounding per segment | `u ≥ 0`, `ValidRateModel p` | INV-IDX-01 ("validated rate parameters") |
| `calculate_annual_borrow_rate_bounds` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | `base_borrow_rate ≤ r ≤ max_borrow_rate` (so `≤ 2·RAY`) | `u ≥ 0`, `ValidRateModel p` | INV-IDX-01 |
| `calculate_annual_borrow_rate_mono` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | `u ≤ u' ⇒ r(u) ≤ r(u')` when `min(u,RAY)`, `min(u',RAY)` are in the same segment (special case of the next row, kept for references) | `0 ≤ u ≤ u'`, `ValidRateModel p`, `SameSegment` | — |
| `calculate_annual_borrow_rate_monotone` (new, #225) | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | **`u ≤ u' ⇒ r(u) ≤ r(u')` across segment boundaries**, every non-negative `u`, `u'` (above `RAY` clamped); both calls return. Pure core `AnnualRate.mono`: each contribution is `≤ slope` (`SegContrib.bound`) and the next segment starts at `base` + the slopes below | `0 ≤ u ≤ u'`, `ValidRateModel p` | INV-IDX-01 |
| `calculate_annual_borrow_rate_at_kinks` (new, #225) | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | `r(mid) = min(base + slope1, max) = base + slope1`; `r(optimal) = min(base + slope1 + slope2, max) = base + slope1 + slope2`; `r(u) = base + slope1 + slope2 + slope3` (`= min(…, max)`) for `u ≥ RAY` | `u ≥ 0`, `ValidRateModel p` | — |
| `calculate_borrow_rate_spec` | `CurveRates.lean` | `rates::curve::calculate_borrow_rate` | never aborts; `r = hu(a, MPY)` with `a` the annual rate (`AnnualRate`, `base ≤ a ≤ max`) | `u ≥ 0`, `ValidRateModel p` | INV-IDX-01 |
| `calculate_deposit_rate_spec` | `CurveRates.lean` | `rates::curve::calculate_deposit_rate` | never aborts; `0 ≤ d ≤ borrow_rate`; `d = 0` when `u = 0` or `rf ∉ [0, BPS)`; else `d = hu(hu(u·br, RAY)·(BPS - rf), BPS)` | `0 ≤ u ≤ RAY`, `br ≥ 0` | — |
| `update_borrow_index_spec` | `Index.lean` | `rates::index::update_borrow_index` | never aborts; `n ≤ MAX`; `old ≤ n` when factor `≥ RAY`; `n = min(hu(old·f, RAY), MAX)` | `0 ≤ old ≤ MAX`, `0 ≤ f ≤ 100·RAY` | INV-IDX-01 |
| `update_supply_index_spec` | `Index.lean` | `rates::index::update_supply_index` | never aborts; returns `old` (early exits) or a value in `[min(old, MAX), MAX]`. Statement unchanged by #224 (see below) | `s, old, rewards ≥ 0`, `hu(s·old, RAY) + rewards < 2^127` | INV-IDX-02 (upper bound, no decrease) |
| `update_supply_index_monotone` | `Index.lean` | `rates::index::update_supply_index` | `old ≤ n ≤ MAX` | as above plus `old ≤ MAX` | INV-IDX-02 |
| `mul_div_floor_saturating_returns` | `Index.lean` | `math::fp_core::mul_div_floor_saturating` | never aborts (value not stated here; see `docs/rates-floor-ceil.md`) | `d ≠ 0` | — |
| `ray_div_floor_saturating_returns` (new, #224) | `Index.lean` | `math::fp::Ray::div_floor_saturating` | never aborts (exact value: `ray_div_floor_saturating_spec`, `docs/rates-floor-ceil.md`) | `b ≠ 0` | — |
| `ray_ord_min_spec`, `ray_ord_max_spec` (new, #224) | `Index.lean` | derived `Ord::min` / `Ord::max` on `Ray` | minimum / maximum of the raw values | none | — |
| `calculate_supplier_rewards_spec` | `Index.lean` | `rates::index::calculate_supplier_rewards` | never aborts; `rewards + fee = accrued = hu(b·new, RAY) - hu(b·old, RAY)` exactly; `rewards, fee ≥ 0`; `fee = hu(accrued·rf, BPS)` | `b ≥ 0`, `0 ≤ old ≤ new`, `0 ≤ rf ≤ BPS`, `hu(b·new, RAY)` fits | INV-IDX-05 |
| `supply_index_reward_shortfall_spec` | `Index.lean` | `rates::index::supply_index_reward_shortfall` | whenever it returns (partial correctness; the abort cases are the intended panics), the result is in `[0, rewards]` | none beyond "returns" | INV-IDX-05 |

Why the remaining hypotheses hold in production:

* `0 ≤ u ≤ RAY` for the curve and for `calculate_deposit_rate`: `utilization`
  returns a value in `[0, RAY]` for every non-negative pair of totals
  (`utilization_spec`).
* Borrow index `0 ≤ old ≤ MAX` and factor `≤ 100·RAY`: indexes start at `RAY`
  and are only written by `update_borrow_index`, which keeps them `≤ MAX`
  (induction over this theorem). `compound_interest` (not extracted) bounds its
  exponent at 2 (`MAX_COMPOUND_DELTA_MS` × `MAX_BORROW_RATE_RAY`, documented in
  `common/src/rates/compound.rs`), so its factor is about `7.39·RAY`; the
  `100·RAY` bound is an assumption about that unextracted function.
* Fit hypotheses on products (`update_supply_index`,
  `calculate_supplier_rewards`): these are value-representability conditions.
  `accrue_step` establishes them itself, by capping interest at the market
  value ceiling before it calls either function, so accrual as a whole needs
  no such hypothesis (`accrue_step_spec` in [pool.md](pool.md), INV-IDX-06).

## Observations about the production code

1. **Fixed by #225: the annual rate is monotone across segment boundaries
   (`calculate_annual_borrow_rate_monotone`).** Before #225 each segment
   contributed `hu(hu(excess·slope, RAY)·RAY, range)`: the first rounding was
   scaled by `RAY / range`, so the rate just below `mid` (or `optimal`) could
   exceed the rate at the breakpoint by `1` (`10^-27` APR), e.g. `base = 0`,
   `slope1 = slope2 = slope3 = 4·10^25 + 2`, `mid = 3·10^26`,
   `optimal = 8·10^26`, `max = 2·RAY` gave `rate(mid - 1) = 4·10^25 + 3 >
   rate(mid) = 4·10^25 + 2`. Each contribution is now one rounding,
   `hu(slope·excess, range) ≤ slope`, so for every model passing
   `InterestRateModel::verify` and all `0 ≤ u ≤ u'`, `rate(u) ≤ rate(u')`; on
   that model the theorem gives `rate(mid - 1) ≤ rate(mid)`. The kinks are
   hit exactly (`calculate_annual_borrow_rate_at_kinks`).
2. **Closed by the S-03 fix: `calculate_deposit_rate` could exceed the borrow
   rate when utilization was above `RAY`.** `utilization` used to return the
   unclamped ratio whenever it fit `i128` (borrowed > supplied after bad debt
   gave `u > RAY`), and the deposit rate uses `u` directly. `utilization` now
   returns `RAY` whenever `borrowed ≥ supplied` (`utilization_spec`), so the
   hypothesis `u ≤ RAY` of `calculate_deposit_rate_spec` holds at its only
   call site, the `contracts/pool/src/views.rs` deposit-rate view.
3. **`update_supply_index` early exits return `old` unchanged.** The doc
   comment says the result is clamped to `[min(old, MAX), MAX]`; on the
   early-exit paths an `old > MAX` would be returned as is. Not reachable
   while every stored index is `≤ MAX`, which `update_supply_index_monotone`
   preserves.

## Rebuild

```sh
export AENEAS_HOME=/path/to/aeneas-release   # see ../README.md
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas/lean
lake build RatesAeneas.Proofs.HalfUpWrappers RatesAeneas.Proofs.CurveRates RatesAeneas.Proofs.Index
```

Files: `lean/FixedPoint/HalfUpSpec.lean`; in `RatesAeneas/Proofs/`: `HalfUp.lean`, `HalfUpDivInt.lean`,
`HalfUpWrappers.lean`, `CurveBase.lean`, `Curve.lean`, `CurveRates.lean`,
`Index.lean`, `TypedWrappers.lean`. They are plain (non-`module`) files, like
`MulDiv.lean`, and are all imported from `RatesAeneas.lean`, so
`lake build RatesAeneas` checks them.

## Changes after #224 (typed fixed-point boundary)

Commit 8f476115 made `fp_core` private and routed callers through typed
wrappers in `common/src/math/fp.rs`. `fp_core.rs` is unchanged, so the core
theorems (`try_mul_div_half_up_spec`, `mul_div_half_up_*`,
`div_by_int_half_up_*`, `rescale_half_up_down_spec`) carried over untouched.
What changed in the proofs:

* `utilization` now computes `borrowed.try_div(env, supplied).unwrap_or(Ray::ONE)`
  instead of `try_mul_div_half_up(raw, RAY, raw).map_or(Ray::ONE, Ray::from)`.
  Same value. New `ray_try_div_spec`; `utilization_spec` re-proved through it.
  **Statement unchanged.**
* `update_supply_index` now computes
  `max(min(grown, cap), min(old, cap))` with the derived `Ord` on `Ray` and
  `grown = new_value.div_floor_saturating(env, supplied)`, instead of the same
  `max`/`min` on raw `i128` with an inlined `mul_div_floor_saturating`. Same
  value. New `ray_div_floor_saturating_returns`, `ray_ord_min_spec`,
  `ray_ord_max_spec`; `update_supply_index_spec` re-proved through them.
  **Statement unchanged** (and so is `update_supply_index_monotone`).
* New specs for the half-up wrappers #224 added (table above).
* All proof files are now imported from `RatesAeneas.lean`.

No previously proved property was lost and no theorem statement changed:
the refactor did not change production behaviour on anything these proofs
cover.

## Changes for audit findings S-03 and S-04

* `utilization` now compares before it divides: `supplied = 0` gives `0`,
  `borrowed ≥ supplied` gives `RAY`, and only `borrowed < supplied` reaches
  `Ray::div`, whose quotient is at most `RAY`. `utilization_spec` is restated
  with the stronger conclusion (result in `[0, RAY]`, no overflow case). The
  separate bound lemma that assumed `borrowed ≤ supplied` is removed: its
  conclusion now holds without that hypothesis.
* New `Ray::try_mul` wrapper (`ray_try_mul_spec`), used by `accrue_step` to
  value the market totals without aborting.
* `rates/simulate.rs` (`accrue_step`, with the new `market_value`,
  `value_room` and `cap_borrow_index`) is in this extraction but proved in
  the pool library, which extracts the same code: see
  [pool.md](pool.md), `accrue_step_spec`.

## Changes after #225 (single-rounded slope contribution)

#225 replaced each segment's `excess.mul(slope).div(range)`
(`hu(hu(excess·slope, RAY)·RAY, range)`) with
`slope_contribution(excess, slope, range) = slope.mul_ray_ratio(excess, range)`
(`hu(slope·excess, range)`, one `mul_div_half_up`). The rates extraction was
re-run. What changed in the proofs:

* New `ray_mul_ray_ratio_spec`, `ray_mul_ray_ratio_returns_iff`,
  `ray_mul_ray_ratio_frac_spec` (`TypedWrappers.lean`) and
  `slope_contribution_spec` (`Curve.lean`).
* `SegContrib e s g c` is now `IsHalfUp (s·e) g c`; `SegContrib.bound`
  strengthened from `c ≤ slope + RAY` to `c ≤ slope`. The old helper lemmas
  The two per-segment bound lemmas of the old proof were removed.
* `calculate_annual_borrow_rate_spec` restated for the single-rounded formula
  (same shape, `AnnualRate`); `calculate_annual_borrow_rate_bounds`,
  `calculate_annual_borrow_rate_mono`, `calculate_borrow_rate_spec` and
  `calculate_deposit_rate_spec` re-checked with unchanged statements, under
  the same `ValidRateModel` hypotheses.
* New `calculate_annual_borrow_rate_monotone` (global monotonicity) and
  `calculate_annual_borrow_rate_at_kinks`.

## Not covered

* **INV-IDX-02 lower bound (`10^24`, `SUPPLY_INDEX_FLOOR_RAW`).** That bound
  is applied by the bad-debt write-down, which is outside `rates/index.rs`.
  Here only "no decrease and `≤ MAX`" is proved.
* **Exact value of the grown supply index.** `update_supply_index` clamps the
  output of `mul_div_floor_saturating`, so only non-abort of that call is
  needed here; its exact floor/saturation value is in
  `docs/rates-floor-ceil.md`.
* ~~Global monotonicity of `calculate_annual_borrow_rate`~~: fixed by #225,
  monotone across segments (`calculate_annual_borrow_rate_monotone`).
* **`rescale_half_up` upscaling and `Wad`/`Bps::to_wad`/`apply_to_wad`
  wrappers.** Not in the requested scope; upscaling is the existing
  `from_asset_spec`.
* **`compound_interest`** is not extracted (array iteration), so the growth
  factor bound used for `update_borrow_index` is a stated hypothesis.
* **Error codes** (`DivisionByZero` vs `MathOverflow`) are not modelled; an
  abort is an abort.
