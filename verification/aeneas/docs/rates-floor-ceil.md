# Floor and ceiling rounding, and the share-conversion directions

Machine-checked Lean proofs (Aeneas extraction, library `RatesAeneas`) about
the directed-rounding arithmetic in `common/src/math` and the share
conversions in `common/src/rates`. Each theorem is stated about the generated
`lean/RatesAeneas/Funs.lean` definition of the named Rust function. Overview
and conventions: [../README.md](../README.md), [../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

* `fp_core::mul_div_ceil` and `fp_core::mul_div_floor_saturating`: exact over
  the whole `i128` domain, both the native `i128` path and the `I256`
  fallback.
* `fp_core::rescale_floor` and `fp_core::rescale_ceil`: every direction,
  every `i128` value, including the downscale branch where `10^k` overflows
  `i128` and the upscale overflow branch.
* `Ray` wrappers: `mul_floor`, `mul_ceil`, `div_floor`, `div_ceil`,
  `div_floor_saturating`, `to_asset_floor`, `to_asset_ceil`.
* The other directed-rounding typed wrappers
  (`Ray::mul_wad_ratio_floor`, `Wad::div_ceil`, `Wad::mul_floor_saturating`,
  `Wad::mul_ratio_floor`, `Wad::apply_ratio_to_amount_floor`,
  `Bps::from_fraction_floor`, `Bps::from_fraction_ceil`, `Bps::apply_to_ceil`,
  `Bps::apply_to_ray_ceil`) and the checked `Ray::mul_int`,
  `Wad::try_from_token_upscale` (`TypedWrappers.lean`).
* `rates/scaling.rs`: `calculate_scaled_supply_ceil`, `calculate_scaled_borrow`,
  `calculate_scaled_borrow_floor`, `unscale_supply_floor`,
  `unscale_borrow_ceil`, `calculate_scaled_cap`, the two ADR-0003 round trips,
  `resolve_repay`, `resolve_net_settle` and `resolve_withdrawal`
  (`Withdrawal.lean`, which also uses the half-up specs of
  [rates-half-up-curve-index.md](rates-half-up-curve-index.md)).
* `rates/index.rs::protocol_fee_shares`.

All statements are total correctness (`f ⦃ r => P r ⦄`: the call returns, does
not abort, and `P` holds). Specifications never divide: `IsFloor n d q` is
`q*d ≤ n < q*d + d` and `IsCeil n d q` is `n ≤ q*d < n + d` (for
`d > 0`, mirrored for `d < 0`), in `lean/FixedPoint/FloorCeilSpec.lean`.

Notation in the table: `RAY = 10^27`, `k = 27 - decimals`,
`A = amount * 10^k` (the amount in RAY units, `Ray::from_asset`),
`P = RAY * 10^k`, `clamp(q) = max(i128::MIN, min(i128::MAX, q))`.

## Trusted base

No additions. These proofs use only the generated code, the
`FunsExternal.lean` / `TypesExternal.lean` meanings (host `I256`,
`checked_pow`, `abs_diff`, `Option` combinators) and the Aeneas standard
library. No `sorry`, `admit` or `axiom`. Every theorem below was checked with
`#print axioms`: each depends only on `propext`, `Classical.choice`,
`Quot.sound`.

## Theorems

Files are under `lean/RatesAeneas/Proofs/`.

### Specification lemmas (`FloorCeilSpec.lean`)

| Lean name | Claim |
|---|---|
| `IsCeil` | Definition: `q = ⌈n/d⌉` without division |
| `IsCeil.unique` | The ceiling is unique |
| `IsFloor.le_IsCeil` | Floor ≤ ceiling for `d > 0` |
| `truncCeil_isCeil` | Truncating division plus one when inexact and the signs agree is `⌈n/d⌉`, for every `n` and `d ≠ 0` |

### Arithmetic core

| Lean name | File | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|---|
| `div_ceil_i256_spec` | `FloorCeilCore.lean` | `math::fp_core::div_ceil_i256` | Returns `⌈n/d⌉` | `d ≠ 0`, `|n| ≤ 2^254` (every `i128` product), correct `nonneg` hint | ADR-0003 |
| `div_ceil_i128_spec` | `FloorCeilCore.lean` | `math::fp_core::div_ceil_i128` | Never aborts; any `Some q` is `⌈p/d⌉` | `d ≠ 0` | ADR-0003 |
| **`mul_div_ceil_spec`** | `FloorCeilCore.lean` | `math::fp_core::mul_div_ceil` | Returns exactly `⌈x*y/d⌉`, both paths | `d ≠ 0`; the ceiling fits `i128` | ADR-0003, INV-RISK-02 |
| `quotient_is_negative_spec` | `FloorCeilCore.lean` | `math::fp_core::quotient_is_negative` | True exactly when `x*y/d < 0` | none | — |
| `IsFloor.neg_iff` | `FloorCeilCore.lean` | — | `⌊n/d⌋ < 0` iff `n/d < 0` | `d ≠ 0` | — |
| **`mul_div_floor_saturating_spec`** | `FloorCeilCore.lean` | `math::fp_core::mul_div_floor_saturating` | Never aborts; returns `clamp(⌊x*y/d⌋)` (exact floor when it fits, `i128::MAX` above, `i128::MIN` below, i.e. by the sign of the exact quotient) | `d ≠ 0` | INV-HALT-03, INV-RISK-02 |
| `mul_div_floor_saturating_exact` | `FloorCeilCore.lean` | same | Exact floor when it fits | `d ≠ 0`, floor fits | INV-HALT-03 |
| `rescale_eq`, `rescale_down_eq`, `rescale_up_spec`, `rescale_up_overflow` | `FloorCeilRescale.lean` | `math::fp_core::rescale` | Equal decimals: identity. Downscale: hands `(a, checked_pow(10, from-to))` to the closure. Upscale: exact `a*10^(to-from)`; aborts when the factor (`to-from ≥ 39`) or the product overflows | upscale: `to-from ≤ 38`, product fits | — |
| **`rescale_floor_down_spec`** | `FloorCeilRescale.lean` | `math::fp_core::rescale_floor` | Downscale never aborts; truncation toward zero: `⌊a/10^(from-to)⌋` for `a ≥ 0`, `⌈a/10^(from-to)⌉` for `a < 0`; includes the overflowing-factor branch (returns 0) | `to < from` | INV-RISK-02 |
| `rescale_floor_eq`, `rescale_floor_up_spec` | `FloorCeilRescale.lean` | `math::fp_core::rescale_floor` | Identity / exact upscale | as `rescale_*` | — |
| **`rescale_ceil_down_spec`** | `FloorCeilRescale.lean` | `math::fp_core::rescale_ceil` | Downscale never aborts; returns `⌈a/10^(from-to)⌉` for every `a` (round up for `a ≥ 0`, truncation for `a < 0`, which is the ceiling); overflowing-factor branch returns `a > 0 ? 1 : 0` | `to < from` | INV-RISK-02 |
| `rescale_ceil_eq`, `rescale_ceil_up_spec` | `FloorCeilRescale.lean` | `math::fp_core::rescale_ceil` | Identity / exact upscale | as `rescale_*` | — |

### `Ray` wrappers and scaling (`FloorCeilScaling.lean`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `ray_mul_floor_spec` | `math::fp::Ray::mul_floor` | `⌊a*b/RAY⌋` | result fits | ADR-0003 |
| `ray_mul_ceil_spec` | `math::fp::Ray::mul_ceil` | `⌈a*b/RAY⌉` | result fits | ADR-0003 |
| `ray_div_floor_spec` | `math::fp::Ray::div_floor` | `⌊a*RAY/b⌋` | `b ≠ 0`, result fits | ADR-0003 |
| `ray_div_ceil_spec` | `math::fp::Ray::div_ceil` | `⌈a*RAY/b⌉` | `b ≠ 0`, result fits | ADR-0003 |
| `ray_div_floor_saturating_spec` | `math::fp::Ray::div_floor_saturating` | Never aborts; `clamp(⌊a*RAY/b⌋)` (exact floor when it fits) | `b ≠ 0` | INV-HALT-03 |
| `ray_to_asset_floor_spec` | `math::fp::Ray::to_asset_floor` | Never aborts; floor of `a/10^k` for `a ≥ 0`, ceiling for `a < 0` | `decimals ≤ 27` | INV-RISK-02 |
| `ray_to_asset_ceil_spec` | `math::fp::Ray::to_asset_ceil` | Never aborts; `⌈a/10^k⌉` | `decimals ≤ 27` | INV-RISK-02 |
| `IsFloor.nest`, `IsCeil.nest` | — | `⌊⌊n/a⌋/b⌋ = ⌊n/(a*b)⌋`, same for ceilings (`a, b > 0`) | — | — |
| **`calculate_scaled_supply_ceil_spec`** | `rates::scaling::calculate_scaled_supply_ceil` | `⌈A*RAY/index⌉` | `decimals < 27`, `index ≠ 0`, `A` and result fit | ADR-0003 |
| **`calculate_scaled_borrow_spec`** | `rates::scaling::calculate_scaled_borrow` | `⌈A*RAY/index⌉` (debt minted rounds up) | same | ADR-0003 |
| **`calculate_scaled_borrow_floor_spec`** | `rates::scaling::calculate_scaled_borrow_floor` | `⌊A*RAY/index⌋` (partial repay burns round down) | same | ADR-0003 |
| **`unscale_supply_floor_spec`** | `rates::scaling::unscale_supply_floor` | A single floor: `⌊scaled*index/P⌋` (the two floors compose) | `decimals ≤ 27`, `scaled*index ≥ 0`, `⌊scaled*index/RAY⌋` fits | ADR-0003, INV-ACCT-06 |
| **`unscale_borrow_ceil_spec`** | `rates::scaling::unscale_borrow_ceil` | A single ceiling: `⌈scaled*index/P⌉` | `decimals ≤ 27`, `⌈scaled*index/RAY⌉` fits | ADR-0003, INV-RISK-02 |
| **`calculate_scaled_cap_spec`** | `rates::scaling::calculate_scaled_cap` | Never aborts past `from_asset`; returns `clamp(⌊C*RAY/index⌋)`, `C = cap*10^k` (fails open by saturating). Proved through `ray_div_floor_saturating_spec` | `decimals < 27`, `index ≠ 0`, `C` fits | INV-HALT-03 |

### Typed wrappers (`TypedWrappers.lean`)

Each is one call into a core proved above (or in `MulDiv.lean`), with the
wrapper's constant substituted (`WAD = 10^18`, `BPS = 10_000`).

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `ray_mul_wad_ratio_floor_spec` | `math::fp::Ray::mul_wad_ratio_floor` | `⌊a*num/den⌋` (one floor, ratio not quantized) | `den ≠ 0`, result fits | ADR-0003 |
| `wad_div_ceil_spec` | `math::fp::Wad::div_ceil` | `⌈a*WAD/b⌉` | `b ≠ 0`, result fits | ADR-0003 |
| `wad_mul_floor_saturating_spec` | `math::fp::Wad::mul_floor_saturating` | Never aborts; `clamp(⌊a*b/WAD⌋)` | none | INV-HALT-03 |
| `wad_mul_ratio_floor_spec` | `math::fp::Wad::mul_ratio_floor` | `⌊a*num/den⌋` | `den ≠ 0`, result fits | ADR-0003 |
| `wad_apply_ratio_to_amount_floor_spec` | `math::fp::Wad::apply_ratio_to_amount_floor` | `⌊amount*self/den⌋` | `den ≠ 0`, result fits | ADR-0003 |
| `bps_from_fraction_floor_spec` | `math::fp::Bps::from_fraction_floor` | `⌊num*BPS/den⌋` | `den ≠ 0`, result fits | ADR-0003 |
| `bps_from_fraction_ceil_spec` | `math::fp::Bps::from_fraction_ceil` | `⌈num*BPS/den⌉` | `den ≠ 0`, result fits | ADR-0003 |
| `isCeil_ratio_bounds` | — | `⌈x*y/d⌉ ∈ [0, x]` for `0 ≤ x`, `0 ≤ y ≤ d` | — | — |
| `bps_apply_to_ceil_spec` | `math::fp::Bps::apply_to_ceil` | Never aborts; exact `⌈amount*bps/BPS⌉ ∈ [0, amount]` | `amount ≥ 0`, `0 ≤ bps ≤ BPS` | ADR-0003 |
| `bps_apply_to_ray_ceil_spec` | `math::fp::Bps::apply_to_ray_ceil` | same for a `Ray` value | `v ≥ 0`, `0 ≤ bps ≤ BPS` | ADR-0003 |
| `ray_mul_int_spec` | `math::fp::Ray::mul_int` | Exact `a*m` (no rescaling) | product fits | — |
| `ray_mul_int_aborts` | `math::fp::Ray::mul_int` | Aborts (`MathOverflow`) | product does not fit | — |
| `checked_pow10_some` | `i128::checked_pow(10, e)` | `Some(10^e)` | `e ≤ 38` | — |
| `wad_try_from_token_upscale_spec` | `math::fp::Wad::try_from_token_upscale` | Never aborts; `None` for `amount < 0` or `decimals > 18`; otherwise `Some(amount*10^(18-decimals))` when it fits, `None` exactly when it does not | none | — |
| `wad_val`, `wad_decimals_val` | constants | `WAD = 10^18`, `WAD_DECIMALS = 18` | — | — |

The half-up typed wrappers (`Ray::try_div`, `Bps::div`,
`Bps::try_from_fraction`, `Bps::from_weighted_sum`,
`Wad::try_value_of_token`) are in
[rates-half-up-curve-index.md](rates-half-up-curve-index.md).

### Round trips (`FloorCeilRoundTrip.lean`)

| Lean name | Rust functions | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| **`supply_round_trip`** | `calculate_scaled_supply` then `unscale_supply_floor` | Deposit then claim never pays out more: result `≤ amount`; the composition returns | `decimals < 27`, `0 < index`, `0 ≤ amount`, `A` fits, minted shares fit (claim-leg fit derived) | ADR-0003 |
| **`borrow_round_trip`** | `calculate_scaled_borrow` then `unscale_borrow_ceil` | Borrow then repay never owes less: result `≥ amount`; the composition returns | `decimals < 27`, `0 < index`, `0 ≤ amount`, debt shares fit, `(A+1)*RAY + index ≤ 2^127*RAY` (bounds the intermediate ceiling; with `index ≤ MAX_BORROW_INDEX_RAY = 10^36` this is `A + 10^9 + 1 ≤ 2^127`) | ADR-0003 |

### Resolvers and fee shares (`FloorCeilResolve.lean`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| **`resolve_repay_spec`** | `rates::scaling::resolve_repay` | Returns. With `D = ⌈pos*index/P⌉`: full repay (`D ≤ amount`) burns exactly `pos` and refunds `amount - D ≥ 0`; partial repay refunds 0 and burns `⌊A*RAY/index⌋`, so `burn*index ≤ A*RAY` (never clears more debt than paid) and `0 ≤ burn < pos` | `decimals < 27`, `0 < index`, `0 ≤ pos`, `0 ≤ amount`, `⌈pos*index/RAY⌉` fits (all other fits derived) | ADR-0003, INV-ACCT-05 |
| **`resolve_net_settle_spec`** | `rates::scaling::resolve_net_settle` | Returns. With `F = ⌊ss*si/P⌋`, `D = ⌈ds*bi/P⌉`: `0 ≤ burned_supply ≤ ss`, `0 ≤ burned_debt ≤ ds`, `settled ≥ 0`; a positive settlement is `≤ amount`, `≤ F`, `≤ D`; zero settlement burns nothing; `settled = F` burns all supply, `settled = D` burns all debt; partial burns are directed: `settled*10^k*RAY ≤ burned_supply*si` (ceiling) and `burned_debt*bi ≤ settled*10^k*RAY` (floor). The `min(_, position)` caps are never binding | `decimals < 27`, `0 < si`, `0 < bi`, `0 ≤ ss`, `0 ≤ ds`, `⌊ss*si/RAY⌋` and `⌈ds*bi/RAY⌉` fit (all other fits derived) | ADR-0003, INV-ACCT-05 |
| `ray_min_spec` | derived `Ord::min` on `Ray` | Minimum of the raw values | none | — |
| `i128_saturating_sub_val` | `i128::saturating_sub` (Aeneas library) | Exact difference when it fits | — | — |
| **`protocol_fee_shares_spec`** | `rates::index::protocol_fee_shares` | Never aborts; result `r = min(⌊fee*RAY/index⌋, i128::MAX - supplied)`, so `r ≥ 0` and `supplied + r ≤ i128::MAX` (cannot overflow). Proved through `ray_div_floor_saturating_spec` | `0 ≤ fee`, `0 < index`, `0 ≤ supplied` | INV-IDX-05, INV-ACCT-01 |

### Withdrawal resolver (`Withdrawal.lean`)

Unbounded counterpart of the Kani harness `withdrawal_never_overpays`.
Notation: `N = pos*index`, `r = hu(N, RAY)` (`Ray::mul`), `H = hu(r, 10^k)`
(`Ray::to_asset`; together the half-up value `unscale_supply` that the branch
compares against), `F = ⌊N/P⌋` (`unscale_supply_floor`), `hu` as in
[rates-half-up-curve-index.md](rates-half-up-curve-index.md).

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| **`resolve_withdrawal_spec`** | `rates::scaling::resolve_withdrawal` | Returns `(burn, paid)` with: always `0 ≤ burn ≤ pos`, `0 ≤ paid ≤ F`, `paid ≤ amount`; full withdrawal (`H ≤ amount`): `burn = pos`, `paid = F`; partial (`amount < H`): `paid = amount`, `burn = ⌈amount*10^k*RAY/index⌉`, so `amount*10^k*RAY ≤ burn*index` (burn worth at least the payout) and `amount*10^k*RAY ≤ pos*index`. The ceiling burn never rounds past the position, also when `amount < H` | `decimals < 27`, `0 < index` (no `≥ 10^24` needed), `0 ≤ pos`, `0 ≤ amount`, `hu(pos*index, RAY)` fits (the exact abort condition; every other fit derived) | ADR-0003, INV-ACCT-01, INV-ACCT-05 |
| **`resolve_withdrawal_aborts`** | same | Aborts whenever `hu(pos*index, RAY)` does not fit `i128` (first call, `Ray::mul` in `unscale_supply`) | none | — |
| **`resolve_withdrawal_returns_iff`** | same | On the production domain, returns **iff** `hu(pos*index, RAY)` fits `i128` | `decimals < 27`, `0 < index`, `0 ≤ pos`, `0 ≤ amount` | — |
| `withdrawal_partial_le` | — | `amount < hu(hu(N, RAY), D)` with `D ≥ 1` implies `amount*D*RAY ≤ N` (the double half-up branch value never exceeds the exact value by a full unit) | `D ≥ 1` | ADR-0003 |
| `withdrawal_floor_le_halfup` | — | `⌊N/(RAY*D)⌋ ≤ hu(hu(N, RAY), D)`, so a full withdrawal never pays more than the request | `D ≥ 1` | ADR-0003 |

Why the partial burn stays within the position: `amount ≤ H - 1` gives
`2*amount*D + D ≤ 2*r` and `2*r*RAY ≤ 2*N + RAY`, so
`2*amount*D*RAY ≤ 2*N - (D - 1)*RAY ≤ 2*N`; then
`⌈amount*D*RAY/index⌉ ≤ pos` and `amount ≤ F`. The partial branch can burn the
whole position, but only while paying the floor claim, e.g. `decimals = 18`,
`index = 0.9*10^36` (≤ `MAX_SUPPLY_INDEX_RAY`), `pos = 12`, `amount = 10`:
`H = 11`, `burn = ⌈10/0.9⌉ = 12 = pos`, `paid = 10 = F`. That is the
full-withdrawal outcome, not a loss.

### Why the hypotheses hold in production

* `decimals < 27` / `≤ 27`: listed assets have at most
  `MAX_ASSET_DECIMALS = 18` decimals (`common/src/constants`).
* Positive indexes: the supply index is at least
  `SUPPLY_INDEX_FLOOR_RAW = RAY / 1000` (INV-IDX-02, ADR-0012) and the borrow
  index at least `RAY` (INV-IDX-01).
* Non-negative share balances and supplies: INV-ACCT-01.
* Non-negative amounts: entry paths reject non-positive amounts
  (INV-ACCT-05); the protocol fee is the reserve-factor share of non-negative
  accrued interest.
* "Fits `i128`" hypotheses: when they fail, production aborts with
  `MathOverflow`. The theorems are total correctness, so they say nothing in
  that case. The exception is an explicit abort theorem, such as
  `rescale_up_overflow`.

## Rebuild

```sh
export AENEAS_HOME=/path/to/aeneas-release
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas/lean
lake build RatesAeneas.Proofs.FloorCeilResolve   # builds every FloorCeil* file it imports
```

`FloorCeilResolve` imports `FloorCeilRoundTrip` → `FloorCeilScaling` →
`FloorCeilRescale` → `FloorCeilCore` → `FloorCeilSpec`, plus the existing
`MulDiv` and `Scaling`. `TypedWrappers` imports `FloorCeilResolve` and
`HalfUpWrappers`. `Withdrawal` imports `FloorCeilResolve`, `HalfUpWrappers`
and `Index` (for `spec_bind_inv`); build it with
`lake build RatesAeneas.Proofs.Withdrawal`. The files are plain (non-`module`) Lean for the same
reason as `MulDiv.lean`. All of them are imported by `RatesAeneas.lean`, so
`lake build RatesAeneas` (and `run.sh rates`) checks them.

## Not covered

* The `Wad` wrappers `Wad::mul_floor`, `mul_ceil`, `div_floor`,
  `div_floor_saturating` and `to_token_floor`, and `Ray::to_wad_floor`,
  `to_wad_ceil` and `Ray::mul_ratio_ceil`. Each is a one-line call into a
  proved core (`mul_div_*`, `rescale_*`), in the same pattern as the `Ray`
  wrappers. No theorem states them.
* Error codes (`DivisionByZero` vs `MathOverflow`) are not modelled
  (see README "Trusted base", item 3).

## Observations on the production code

No proof failed because of the code. Two edge behaviours worth knowing, both
unreachable in production:

* `rescale` upscaling by 39 or more decimal places aborts even for `a = 0`,
  because `10^(to-from)` is computed (and overflows) before the multiply
  (`rescale_up_overflow`). Production upscales by at most 27 places.
* `rescale_floor` downscaling of a negative value truncates toward zero, i.e.
  returns the ceiling, not the floor (as its doc comment says;
  `rescale_floor_down_spec`). Callers pass non-negative amounts.
