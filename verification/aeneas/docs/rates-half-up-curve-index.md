# Half-up rounding, the rate curve, and index accrual

Lean proofs (library `RatesAeneas`) of the half-up math in `common/src/math`,
the rate curve (`common/src/rates/curve.rs`) and index updates
(`common/src/rates/index.rs`). Conventions, trusted base and rebuild: as
[rates-floor-ceil.md](rates-floor-ceil.md).

## Scope

* `fp_core::try_mul_div_half_up`, `fp_core::mul_div_half_up`: exact on all
  `i128`, both paths, and exactly when `mul_div_half_up` aborts.
* `fp_core::div_by_int_half_up`: nearest, ties away from zero, every `i128`
  numerator; aborts iff `b ≤ 0`.
* `fp_core::rescale_half_up` downscaling, including an overflowing
  `10^(from - to)` (returns 0).
* Half-up `Ray`, `Bps`, `Wad` wrappers.
* `rates/curve.rs`: utilization, annual borrow rate (value, bounds, kinks,
  monotonicity over the whole curve), borrow and deposit rates.
* `rates/index.rs`: index updates, supplier rewards, reward shortfall.

Statements are total correctness unless stated; "aborts" is
`¬ (f ⦃ _ => True ⦄)`. Division-free specs (`lean/FixedPoint/HalfUpSpec.lean`):

* `IsHalfUp n d q` (`d > 0`): `2·q·d ≤ 2·n + d < 2·q·d + 2·d`: nearest,
  halves up (= `IsFloor (2n + d) (2d) q`, `IsHalfUp.iff_isFloor`).
* `IsHalfAway n d q` (`d > 0`): `IsHalfUp n d q` for `n ≥ 0`,
  `IsHalfUp (-n) d (-q)` for `n < 0`: nearest, halves away from zero.

Notation: `RAY = 10^27`, `BPS = 10_000`, `MAX = 10^36`
(`MAX_BORROW_INDEX_RAY = MAX_SUPPLY_INDEX_RAY`), `MPY = 31_556_926_000`
(`MILLISECONDS_PER_YEAR`), `hu(n, d)` = the `q` with `IsHalfUp n d q`.

## Trusted base

No additions ([rates-floor-ceil.md](rates-floor-ceil.md#trusted-base)).
Proved `Result`-monad lemmas: `spec_imp_eq_ok` (a total-correctness triple
pins the result to `ok v`; induction on the `DWP` least fixed point, failure
being the only effect) and `spec_bind_inv` (bind inversion).

## Rate-model hypotheses

`ValidRateModel p` (`Proofs/Curve.lean`) is a subset of
`InterestRateModel::verify` (`common/src/types/pool.rs`) on `MarketParams`
fields: `0 ≤ base`, `base ≤ slope1`, `0 ≤ slope2`,
`0 ≤ slope3`, `slope3 ≤ max`, `base < max`, `max ≤ MAX_BORROW_RATE_RAY = 2·RAY`,
`base + slope1 + slope2 + slope3 ≤ max`, `0 < mid < optimal < RAY`,
`optimal ≤ max_utilization ≤ RAY`, `0 ≤ reserve_factor < BPS`.

`verify` also requires `slope1, slope2 ≤ max` and non-falling steepness at
each kink (audit I-08); no theorem needs these. `verify` runs on market
creation and every parameter update
(`contracts/pool/src/ops/market.rs`, `contracts/governance/src/validate/asset.rs`,
`contracts/governance/src/op.rs`).

## Theorems

| Lean name | File | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|---|
| `isHalfUp_tdiv` | `HalfUpSpec.lean` | (core lemma) | `trunc((n + trunc(d/2)) / d) = hu(n, d)` | `n ≥ 0`, `d > 0` | — |
| `IsHalfUp.unique`, `.mono`, `.anti_div`, `.exists`, `.le_of_ratio_le_one` | `HalfUpSpec.lean` | (spec lemmas) | unique, monotone in `n`, antitone in `d`, exists; `0 ≤ y ≤ d ⇒ 0 ≤ hu(x·y, d) ≤ x` | as stated | — |
| `try_mul_div_half_up_spec` | `HalfUp.lean` | `math::fp_core::try_mul_div_half_up` | never aborts; `Some(hu(x·y, d))` iff `x, y ≥ 0`, `d > 0` and it fits `i128`, else `None` | none | — |
| `mul_div_half_up_spec` | `HalfUp.lean` | `math::fp_core::mul_div_half_up` | `hu(x·y, d)` | `x, y ≥ 0`, `d > 0`, result fits | — |
| `mul_div_half_up_returns_iff` | `HalfUp.lean` | `math::fp_core::mul_div_half_up` | returns **iff** `x, y ≥ 0`, `d > 0`, result fits; else aborts (`d = 0`, negative operand via `debug_assert`, overflow) | none | — |
| `div_by_int_half_up_spec` | `HalfUpDivInt.lean` | `math::fp_core::div_by_int_half_up` | never aborts; `IsHalfAway a b q`, every `i128` `a` | `b > 0` (call sites: `MPY`, powers of ten) | — |
| `div_by_int_half_up_aborts` | `HalfUpDivInt.lean` | `math::fp_core::div_by_int_half_up` | aborts | `b ≤ 0` | — |
| `rescale_half_up_down_spec` | `HalfUpWrappers.lean` | `math::fp_core::rescale_half_up` | downscaling never aborts and returns `q` with `IsHalfAway a (10^(from-to)) q` for every `i128` `a` and gap (gap ≥ 39 returns 0, which is that rounding) | `to < from` | — |
| `ray_to_asset_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::to_asset` | never aborts; `IsHalfAway ray (10^(27-dec)) q` | `dec < 27` (production `≤ MAX_ASSET_DECIMALS = 18`) | — |
| `ray_mul_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::mul` | `hu(a·b, RAY)` | `a, b ≥ 0`, result fits | — |
| `ray_div_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::div` | `hu(a·RAY, b)` | `a ≥ 0`, `b > 0`, result fits | — |
| `ray_try_div_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::try_div` | never aborts; `Some(hu(a·RAY, b))` iff `a ≥ 0`, `b > 0` and it fits, else `None` | none | — |
| `ray_try_mul_spec` | `HalfUpWrappers.lean` | `math::fp::Ray::try_mul` | never aborts; `Some(hu(a·b, RAY))` iff operands `≥ 0` and it fits, else `None` | none | INV-IDX-06, audit S-04 |
| `ray_mul_ray_ratio_spec` | `TypedWrappers.lean` | `math::fp::Ray::mul_ray_ratio` | `hu(x·n, d)` (one rounding, `n/d` not quantized) | `x, n ≥ 0`, `d > 0`, result fits | — |
| `ray_mul_ray_ratio_returns_iff` | `TypedWrappers.lean` | `math::fp::Ray::mul_ray_ratio` | returns **iff** `x, n ≥ 0`, `d > 0`, result fits | none | — |
| `ray_mul_ray_ratio_frac_spec` | `TypedWrappers.lean` | `math::fp::Ray::mul_ray_ratio` | never aborts; `hu(x·n, d) ∈ [0, x]` | `x ≥ 0`, `0 ≤ n ≤ d`, `d > 0` | — |
| `bps_div_spec` | `TypedWrappers.lean` | `math::fp::Bps::div` | `hu(a·BPS, b)` | `a ≥ 0`, `b > 0`, result fits | — |
| `bps_try_from_fraction_spec` | `TypedWrappers.lean` | `math::fp::Bps::try_from_fraction` | never aborts; `Some(hu(num·BPS, den))` iff `num ≥ 0`, `den > 0` and it fits, else `None` | none | — |
| `bps_from_weighted_sum_spec` | `TypedWrappers.lean` | `math::fp::Bps::from_weighted_sum` | never aborts; `IsHalfAway sum weight q`, every `i128` sum | `weight > 0` | — |
| `wad_try_value_of_token_spec` | `TypedWrappers.lean` | `math::fp::Wad::try_value_of_token` | never aborts; `Some(hu(amount·price, 10^decimals))` iff `decimals ≤ 38`, operands `≥ 0` and it fits, else `None` | none | — |
| `bps_apply_to_spec` | `HalfUpWrappers.lean` | `math::fp::Bps::apply_to` | never aborts; `hu(amount·bps, BPS) ∈ [0, amount]` | `amount ≥ 0`, `0 ≤ bps ≤ BPS` | INV-IDX-05 |
| `bps_apply_to_ray_spec` | `HalfUpWrappers.lean` | `math::fp::Bps::apply_to_ray` | same for a ray value | `v ≥ 0`, `0 ≤ bps ≤ BPS` | INV-IDX-05 |
| `flash_loan_fee_on_spec` | `HalfUpWrappers.lean` | `math::fp::Bps::flash_loan_fee_on` | never aborts; `0 ≤ fee ≤ amount`; `fee ≥ 1` when `bps > 0`; `fee = hu(amount·bps, BPS)` unless that is 0 with `bps > 0`, then 1 | `amount ≥ 1`, `0 ≤ bps ≤ BPS` (`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`, `verify`) | INV-FLASH-01 (the fee `F`) |
| `utilization_spec` | `CurveBase.lean` | `rates::curve::utilization` | never aborts; in `[0, RAY]`: `0` if `supplied = 0`, `RAY` if `borrowed ≥ supplied > 0` (by comparison), else `hu(b·RAY, s)` | `b, s ≥ 0` (pool totals) | INV-ACCT-08, audit S-03 |
| `slope_contribution_spec` | `Curve.lean` | `rates::curve::slope_contribution` | never aborts; `c = hu(slope·excess, range)` (`SegContrib`), `0 ≤ c ≤ slope` | `0 ≤ excess ≤ range`, `range > 0`, `slope ≥ 0` | — |
| `calculate_annual_borrow_rate_spec` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | never aborts; `AnnualRate` at `m = min(u, RAY)`: `min(base + slopes below + hu(slope·excess, range), max)` | `u ≥ 0`, `ValidRateModel p` | INV-IDX-01 ("validated rate parameters") |
| `calculate_annual_borrow_rate_bounds` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | `base_borrow_rate ≤ r ≤ max_borrow_rate` (so `≤ 2·RAY`) | `u ≥ 0`, `ValidRateModel p` | INV-IDX-01 |
| `calculate_annual_borrow_rate_mono` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | `u ≤ u' ⇒ r(u) ≤ r(u')` within one segment (special case of the next row) | `0 ≤ u ≤ u'`, `ValidRateModel p`, `SameSegment` | — |
| `calculate_annual_borrow_rate_monotone` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | **`u ≤ u' ⇒ r(u) ≤ r(u')` across segments**, `u > RAY` clamped; both calls return. Core `AnnualRate.mono`: each contribution `≤ slope` (`SegContrib.bound`), the next segment starts at `base` + slopes below | `0 ≤ u ≤ u'`, `ValidRateModel p` | INV-IDX-01 |
| `calculate_annual_borrow_rate_at_kinks` | `Curve.lean` | `rates::curve::calculate_annual_borrow_rate` | `r(mid) = base + slope1`, `r(optimal) = base + slope1 + slope2`, `r(u ≥ RAY) = base + slope1 + slope2 + slope3` (each equals its `min(…, max)`) | `u ≥ 0`, `ValidRateModel p` | — |
| `calculate_borrow_rate_spec` | `CurveRates.lean` | `rates::curve::calculate_borrow_rate` | never aborts; `hu(a, MPY)`, `a` the annual rate (`base ≤ a ≤ max`) | `u ≥ 0`, `ValidRateModel p` | INV-IDX-01 |
| `calculate_deposit_rate_spec` | `CurveRates.lean` | `rates::curve::calculate_deposit_rate` | never aborts; `0 ≤ d ≤ borrow_rate`; `0` if `u = 0` or `rf ∉ [0, BPS)`, else `hu(hu(u·br, RAY)·(BPS - rf), BPS)` | `0 ≤ u ≤ RAY`, `br ≥ 0` | — |
| `update_borrow_index_spec` | `Index.lean` | `rates::index::update_borrow_index` | never aborts; `n ≤ MAX`; `old ≤ n` when factor `≥ RAY`; `n = min(hu(old·f, RAY), MAX)` | `0 ≤ old ≤ MAX`, `0 ≤ f ≤ 100·RAY` | INV-IDX-01 |
| `update_supply_index_spec` | `Index.lean` | `rates::index::update_supply_index` | never aborts; `old` (early exits) or in `[min(old, MAX), MAX]` | `s, old, rewards ≥ 0`, `hu(s·old, RAY) + rewards < 2^127` | INV-IDX-02 (upper bound, no decrease) |
| `update_supply_index_monotone` | `Index.lean` | `rates::index::update_supply_index` | `old ≤ n ≤ MAX` | as above plus `old ≤ MAX` | INV-IDX-02 |
| `mul_div_floor_saturating_returns` | `Index.lean` | `math::fp_core::mul_div_floor_saturating` | never aborts (value: `mul_div_floor_saturating_spec`) | `d ≠ 0` | — |
| `ray_div_floor_saturating_returns` | `Index.lean` | `math::fp::Ray::div_floor_saturating` | never aborts (value: `ray_div_floor_saturating_spec`) | `b ≠ 0` | — |
| `ray_ord_min_spec`, `ray_ord_max_spec` | `Index.lean` | derived `Ord::min` / `Ord::max` on `Ray` | min / max of raw values | none | — |
| `calculate_supplier_rewards_spec` | `Index.lean` | `rates::index::calculate_supplier_rewards` | never aborts; `rewards + fee = accrued = hu(b·new, RAY) - hu(b·old, RAY)`; `rewards, fee ≥ 0`; `fee = hu(accrued·rf, BPS)` | `b ≥ 0`, `0 ≤ old ≤ new`, `0 ≤ rf ≤ BPS`, `hu(b·new, RAY)` fits | INV-IDX-05 |
| `supply_index_reward_shortfall_spec` | `Index.lean` | `rates::index::supply_index_reward_shortfall` | partial correctness (the aborts are the intended panics): any result is in `[0, rewards]` | none | INV-IDX-05 |

Production hypotheses:

* `0 ≤ u ≤ RAY` (curve, `calculate_deposit_rate`): `utilization_spec`.
* Borrow index `0 ≤ old ≤ MAX` and factor `≤ 100·RAY`: indexes start at `RAY`
  and are only written by `update_borrow_index`, which keeps them `≤ MAX`
  (induction over this theorem). The `100·RAY` bound is an assumption about
  `compound_interest`, which is not extracted here: it bounds its exponent at
  2 (`MAX_COMPOUND_DELTA_MS` × `MAX_BORROW_RATE_RAY`,
  `common/src/rates/compound.rs`), so its factor is about `7.39·RAY`.
* Product fits (`update_supply_index`, `calculate_supplier_rewards`):
  `accrue_step` caps interest at the market value ceiling first, so accrual
  needs no such hypothesis (`accrue_step_spec` in [pool.md](pool.md), INV-IDX-06).

## Observations about the production code

1. **The annual rate is monotone across segment boundaries
   (`calculate_annual_borrow_rate_monotone`)** because each segment
   contributes one rounding, `hu(slope·excess, range) ≤ slope`. A
   double-rounded `hu(hu(excess·slope, RAY)·RAY, range)` scales the first
   rounding by `RAY / range`, so the rate just below `mid` can exceed the rate
   at `mid` by `1` (`10^-27` APR): `base = 0`,
   `slope1 = slope2 = slope3 = 4·10^25 + 2`, `mid = 3·10^26`,
   `optimal = 8·10^26`, `max = 2·RAY` gives `rate(mid - 1) = 4·10^25 + 3 >
   rate(mid) = 4·10^25 + 2`.
2. **Deposit rate ≤ borrow rate (audit S-03)** needs `u ≤ RAY`, though bad
   debt allows `borrowed > supplied`. `utilization` returns `RAY` once
   `borrowed ≥ supplied` (`utilization_spec`), so it holds at the only call
   site, the `contracts/pool/src/views.rs` deposit-rate view.
3. **`update_supply_index` early exits return `old` unchanged**, even
   `old > MAX`, although the doc comment says the result is clamped to
   `[min(old, MAX), MAX]`. Not reachable while every stored index is `≤ MAX`,
   which `update_supply_index_monotone` preserves.

## Rebuild

```sh
cd verification/aeneas/lean   # environment as in rates-floor-ceil.md
lake build RatesAeneas.Proofs.HalfUpWrappers RatesAeneas.Proofs.CurveRates RatesAeneas.Proofs.Index
```

Files: `lean/FixedPoint/HalfUpSpec.lean`; in `RatesAeneas/Proofs/`: `HalfUp.lean`, `HalfUpDivInt.lean`,
`HalfUpWrappers.lean`, `CurveBase.lean`, `Curve.lean`, `CurveRates.lean`,
`Index.lean`, `TypedWrappers.lean`.

## Not covered

* **INV-IDX-02 lower bound (`10^24`, `SUPPLY_INDEX_FLOOR_RAW`)**, applied by
  the bad-debt write-down outside `rates/index.rs`; only "no decrease, `≤ MAX`"
  is proved.
* **Exact value of the grown supply index.** Only non-abort of
  `Ray::div_floor_saturating` is used here; its exact value is
  `ray_div_floor_saturating_spec`.
* **`rescale_half_up` upscaling** (that is `from_asset_spec`) and the
  `Wad`/`Bps::to_wad`/`apply_to_wad` wrappers.
* **`compound_interest`**: not extracted (array iteration); its growth-factor
  bound for `update_borrow_index` is a hypothesis.
