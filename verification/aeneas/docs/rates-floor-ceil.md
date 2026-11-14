# Floor and ceiling rounding, and the share-conversion directions

Lean proofs (library `RatesAeneas`, about its generated `Funs.lean`) of the
directed-rounding math in `common/src/math` and the share conversions in
`common/src/rates`. Conventions: [../README.md](../README.md),
[../WRITING-PROOFS.md](../WRITING-PROOFS.md). Half-up math, rate curve and
indexes: [rates-half-up-curve-index.md](rates-half-up-curve-index.md).

## Scope

* `fp_core::mul_div_ceil`, `fp_core::mul_div_floor_saturating`: exact on all
  `i128`, both paths.
* `fp_core::rescale_floor`, `fp_core::rescale_ceil`: every direction and
  `i128` value, including a `10^k` that overflows `i128` and upscale
  overflow.
* Directed-rounding `Ray`, `Wad`, `Bps` wrappers; checked `Ray::mul_int`,
  `Wad::try_from_token_upscale`.
* `rates/scaling.rs` conversions, both ADR-0003 round trips, `resolve_repay`,
  `resolve_net_settle`, `resolve_withdrawal`;
  `rates/index.rs::protocol_fee_shares`.

Statements are total correctness (`f ⦃ r => P r ⦄`). Division-free specs:
`IsFloor n d q` is
`q*d ≤ n < q*d + d` and `IsCeil n d q` is `n ≤ q*d < n + d` (for
`d > 0`, mirrored for `d < 0`), in `lean/FixedPoint/FloorCeilSpec.lean`.

Notation: `RAY = 10^27`, `k = 27 - decimals`,
`A = amount * 10^k` (amount in RAY units, `Ray::from_asset`),
`P = RAY * 10^k`, `clamp(q) = max(i128::MIN, min(i128::MAX, q))`.

## Trusted base

No additions, for both `RatesAeneas` documents: generated code, the
`FunsExternal.lean` / `TypesExternal.lean` meanings (host `I256`,
`checked_pow`, `abs_diff`, `Range::contains`, `Option` combinators) and the
Aeneas library.

## Theorems

Files under `lean/RatesAeneas/Proofs/`. "Fits" means fits `i128`.

### Specification lemmas (`FloorCeilSpec.lean`)

| Lean name | Claim |
|---|---|
| `IsCeil` | Definition of `q = ⌈n/d⌉` |
| `IsCeil.unique` | The ceiling is unique |
| `IsFloor.le_IsCeil` | Floor ≤ ceiling for `d > 0` |
| `truncCeil_isCeil` | Truncating division, plus one if inexact with agreeing signs, is `⌈n/d⌉` (`d ≠ 0`) |

### Arithmetic core

| Lean name | File | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|---|
| `div_ceil_i256_spec` | `FloorCeilCore.lean` | `math::fp_core::div_ceil_i256` | `⌈n/d⌉` | `d ≠ 0`, `\|n\| ≤ 2^254` (any `i128` product), correct `nonneg` hint | ADR-0003 |
| `div_ceil_i128_spec` | `FloorCeilCore.lean` | `math::fp_core::div_ceil_i128` | Never aborts; `Some q` is `⌈p/d⌉` | `d ≠ 0` | ADR-0003 |
| **`mul_div_ceil_spec`** | `FloorCeilCore.lean` | `math::fp_core::mul_div_ceil` | `⌈x*y/d⌉`, both paths | `d ≠ 0`, result fits | ADR-0003, INV-RISK-02 |
| `quotient_is_negative_spec` | `FloorCeilCore.lean` | `math::fp_core::quotient_is_negative` | True iff `x*y/d < 0` | none | — |
| `IsFloor.neg_iff` | `FloorCeilCore.lean` | — | `⌊n/d⌋ < 0` iff `n/d < 0` | `d ≠ 0` | — |
| **`mul_div_floor_saturating_spec`** | `FloorCeilCore.lean` | `math::fp_core::mul_div_floor_saturating` | Never aborts; `clamp(⌊x*y/d⌋)` (saturating by the sign of the exact quotient) | `d ≠ 0` | INV-HALT-03, INV-RISK-02 |
| `mul_div_floor_saturating_exact` | `FloorCeilCore.lean` | same | Exact floor | `d ≠ 0`, floor fits | INV-HALT-03 |
| `rescale_eq`, `rescale_down_eq`, `rescale_up_spec`, `rescale_up_overflow` | `FloorCeilRescale.lean` | `math::fp_core::rescale` | Equal decimals: identity. Downscale: passes `(a, checked_pow(10, from-to))` to the closure. Upscale: `a*10^(to-from)`; aborts if the factor (`to-from ≥ 39`) or product overflows | upscale: `to-from ≤ 38`, product fits | — |
| **`rescale_floor_down_spec`** | `FloorCeilRescale.lean` | `math::fp_core::rescale_floor` | Downscale never aborts; truncates toward zero (`⌊a/10^(from-to)⌋` for `a ≥ 0`, `⌈…⌉` for `a < 0`); overflowing factor returns 0 | `to < from` | INV-RISK-02 |
| `rescale_floor_eq`, `rescale_floor_up_spec` | `FloorCeilRescale.lean` | `math::fp_core::rescale_floor` | Identity / exact upscale | as `rescale_*` | — |
| **`rescale_ceil_down_spec`** | `FloorCeilRescale.lean` | `math::fp_core::rescale_ceil` | Downscale never aborts; `⌈a/10^(from-to)⌉` for every `a`; overflowing factor returns `a > 0 ? 1 : 0` | `to < from` | INV-RISK-02 |
| `rescale_ceil_eq`, `rescale_ceil_up_spec` | `FloorCeilRescale.lean` | `math::fp_core::rescale_ceil` | Identity / exact upscale | as `rescale_*` | — |

### `Ray` wrappers and scaling (`FloorCeilScaling.lean`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `ray_mul_floor_spec` | `math::fp::Ray::mul_floor` | `⌊a*b/RAY⌋` | result fits | ADR-0003 |
| `ray_mul_ceil_spec` | `math::fp::Ray::mul_ceil` | `⌈a*b/RAY⌉` | result fits | ADR-0003 |
| `ray_div_floor_spec` | `math::fp::Ray::div_floor` | `⌊a*RAY/b⌋` | `b ≠ 0`, result fits | ADR-0003 |
| `ray_div_ceil_spec` | `math::fp::Ray::div_ceil` | `⌈a*RAY/b⌉` | `b ≠ 0`, result fits | ADR-0003 |
| `ray_div_floor_saturating_spec` | `math::fp::Ray::div_floor_saturating` | Never aborts; `clamp(⌊a*RAY/b⌋)` | `b ≠ 0` | INV-HALT-03 |
| `ray_to_asset_floor_spec` | `math::fp::Ray::to_asset_floor` | Never aborts; `a/10^k` truncated toward zero | `decimals ≤ 27` | INV-RISK-02 |
| `ray_to_asset_ceil_spec` | `math::fp::Ray::to_asset_ceil` | Never aborts; `⌈a/10^k⌉` | `decimals ≤ 27` | INV-RISK-02 |
| `IsFloor.nest`, `IsCeil.nest` | — | `⌊⌊n/a⌋/b⌋ = ⌊n/(a*b)⌋`, likewise ceilings (`a, b > 0`) | — | — |
| **`calculate_scaled_supply_ceil_spec`** | `rates::scaling::calculate_scaled_supply_ceil` | `⌈A*RAY/index⌉` | `decimals < 27`, `index ≠ 0`, `A` and result fit | ADR-0003 |
| **`calculate_scaled_borrow_spec`** | `rates::scaling::calculate_scaled_borrow` | `⌈A*RAY/index⌉` (minted debt rounds up) | same | ADR-0003 |
| **`calculate_scaled_borrow_floor_spec`** | `rates::scaling::calculate_scaled_borrow_floor` | `⌊A*RAY/index⌋` (partial-repay burn rounds down) | same | ADR-0003 |
| **`unscale_supply_floor_spec`** | `rates::scaling::unscale_supply_floor` | One floor `⌊scaled*index/P⌋` (the two floors compose) | `decimals ≤ 27`, `scaled*index ≥ 0`, `⌊scaled*index/RAY⌋` fits | ADR-0003, INV-ACCT-06 |
| **`unscale_borrow_ceil_spec`** | `rates::scaling::unscale_borrow_ceil` | One ceiling `⌈scaled*index/P⌉` | `decimals ≤ 27`, `⌈scaled*index/RAY⌉` fits | ADR-0003, INV-RISK-02 |
| **`calculate_scaled_cap_spec`** | `rates::scaling::calculate_scaled_cap` | Never aborts past `from_asset`; `clamp(⌊C*RAY/index⌋)`, `C = cap*10^k` (saturates: fails open); via `ray_div_floor_saturating_spec` | `decimals < 27`, `index ≠ 0`, `C` fits | INV-HALT-03 |

### Typed wrappers (`TypedWrappers.lean`)

Each is one call into a core proved above (or in `MulDiv.lean`) with the
wrapper's constant (`WAD = 10^18`, `BPS = 10_000`).

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
| `bps_apply_to_ceil_spec` | `math::fp::Bps::apply_to_ceil` | Never aborts; `⌈amount*bps/BPS⌉ ∈ [0, amount]` | `amount ≥ 0`, `0 ≤ bps ≤ BPS` | ADR-0003 |
| `bps_apply_to_ray_ceil_spec` | `math::fp::Bps::apply_to_ray_ceil` | same for a `Ray` value | `v ≥ 0`, `0 ≤ bps ≤ BPS` | ADR-0003 |
| `ray_mul_int_spec` | `math::fp::Ray::mul_int` | `a*m`, no rescaling | product fits | — |
| `ray_mul_int_aborts` | `math::fp::Ray::mul_int` | Aborts (`MathOverflow`) | product overflows | — |
| `checked_pow10_some` | `i128::checked_pow(10, e)` | `Some(10^e)` | `e ≤ 38` | — |
| `wad_try_from_token_upscale_spec` | `math::fp::Wad::try_from_token_upscale` | Never aborts; `Some(amount*10^(18-decimals))` iff `amount ≥ 0`, `decimals ≤ 18` and it fits, else `None` | none | — |
| `wad_val`, `wad_decimals_val` | constants | `WAD = 10^18`, `WAD_DECIMALS = 18` | — | — |

The half-up typed wrappers (`Ray::try_div`, `Bps::div`,
`Bps::try_from_fraction`, `Bps::from_weighted_sum`,
`Wad::try_value_of_token`) are in the half-up document.

### Round trips (`FloorCeilRoundTrip.lean`)

| Lean name | Rust functions | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| **`supply_round_trip`** | `calculate_scaled_supply` then `unscale_supply_floor` | Returns; claim `≤ amount` | `decimals < 27`, `0 < index`, `0 ≤ amount`, `A` and minted shares fit (claim-leg fit derived) | ADR-0003 |
| **`borrow_round_trip`** | `calculate_scaled_borrow` then `unscale_borrow_ceil` | Returns; owed `≥ amount` | `decimals < 27`, `0 < index`, `0 ≤ amount`, debt shares fit, `(A+1)*RAY + index ≤ 2^127*RAY` (bounds the intermediate ceiling; with `index ≤ MAX_BORROW_INDEX_RAY = 10^36`: `A + 10^9 + 1 ≤ 2^127`) | ADR-0003 |

### Resolvers and fee shares (`FloorCeilResolve.lean`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| **`resolve_repay_spec`** | `rates::scaling::resolve_repay` | Returns. `D = ⌈pos*index/P⌉`. Full (`D ≤ amount`): burns `pos`, refunds `amount - D ≥ 0`. Partial: refunds 0, burns `⌊A*RAY/index⌋`, so `burn*index ≤ A*RAY` and `0 ≤ burn < pos` | `decimals < 27`, `0 < index`, `0 ≤ pos`, `0 ≤ amount`, `⌈pos*index/RAY⌉` fits (all other fits derived) | ADR-0003, INV-ACCT-05 |
| **`resolve_net_settle_spec`** | `rates::scaling::resolve_net_settle` | Returns. `F = ⌊ss*si/P⌋`, `D = ⌈ds*bi/P⌉`: `0 ≤ burned_supply ≤ ss`, `0 ≤ burned_debt ≤ ds`, `settled ≥ 0`, positive `settled ≤ min(amount, F, D)`; zero burns nothing; `settled = F` / `= D` burns all supply / debt; partial burns directed: `settled*10^k*RAY ≤ burned_supply*si`, `burned_debt*bi ≤ settled*10^k*RAY`. `min(_, position)` caps never bind | `decimals < 27`, `0 < si`, `0 < bi`, `0 ≤ ss`, `0 ≤ ds`, `⌊ss*si/RAY⌋` and `⌈ds*bi/RAY⌉` fit (all other fits derived) | ADR-0003, INV-ACCT-05 |
| `ray_min_spec` | derived `Ord::min` on `Ray` | Min of raw values | none | — |
| `i128_saturating_sub_val` | `i128::saturating_sub` (Aeneas library) | Exact difference if it fits | — | — |
| **`protocol_fee_shares_spec`** | `rates::index::protocol_fee_shares` | Never aborts; `r = min(⌊fee*RAY/index⌋, i128::MAX - supplied)`, so `r ≥ 0`, `supplied + r ≤ i128::MAX`; via `ray_div_floor_saturating_spec` | `0 ≤ fee`, `0 < index`, `0 ≤ supplied` | INV-IDX-05, INV-ACCT-01 |

### Withdrawal resolver (`Withdrawal.lean`)

Unbounded counterpart of the Kani harness `withdrawal_never_overpays`.
Notation: `N = pos*index`, `r = hu(N, RAY)` (`Ray::mul`), `H = hu(r, 10^k)`
(`Ray::to_asset`; together the half-up `unscale_supply` value the branch
compares against), `F = ⌊N/P⌋` (`unscale_supply_floor`), `hu` as in the
half-up document.

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| **`resolve_withdrawal_spec`** | `rates::scaling::resolve_withdrawal` | `(burn, paid)`: `0 ≤ burn ≤ pos`, `0 ≤ paid ≤ min(F, amount)`. Full (`H ≤ amount`): `burn = pos`, `paid = F`. Partial (`amount < H`): `paid = amount`, `burn = ⌈amount*10^k*RAY/index⌉`, so `amount*10^k*RAY ≤ burn*index` and `≤ pos*index` (the ceiling burn never passes the position) | `decimals < 27`, `0 < index` (no `≥ 10^24`), `0 ≤ pos`, `0 ≤ amount`, `hu(pos*index, RAY)` fits (the exact abort condition; other fits derived) | ADR-0003, INV-ACCT-01, INV-ACCT-05 |
| **`resolve_withdrawal_aborts`** | same | Aborts if `hu(pos*index, RAY)` overflows (`Ray::mul` in `unscale_supply`) | none | — |
| **`resolve_withdrawal_returns_iff`** | same | On the production domain, returns **iff** `hu(pos*index, RAY)` fits | `decimals < 27`, `0 < index`, `0 ≤ pos`, `0 ≤ amount` | — |
| `withdrawal_partial_le` | — | `amount < hu(hu(N, RAY), D)` ⇒ `amount*D*RAY ≤ N` (double half-up exceeds the exact value by under one unit) | `D ≥ 1` | ADR-0003 |
| `withdrawal_floor_le_halfup` | — | `⌊N/(RAY*D)⌋ ≤ hu(hu(N, RAY), D)`: a full withdrawal pays at most the request | `D ≥ 1` | ADR-0003 |

Partial burn within the position: `amount ≤ H - 1` gives
`2*amount*D + D ≤ 2*r` and `2*r*RAY ≤ 2*N + RAY`, so
`2*amount*D*RAY ≤ 2*N - (D - 1)*RAY ≤ 2*N`; then
`⌈amount*D*RAY/index⌉ ≤ pos` and `amount ≤ F`. A partial burn of the whole
position pays the floor claim, e.g. `decimals = 18`,
`index = 0.9*10^36` (≤ `MAX_SUPPLY_INDEX_RAY`), `pos = 12`, `amount = 10`:
`H = 11`, `burn = ⌈10/0.9⌉ = 12 = pos`, `paid = 10 = F`: the full-withdrawal
outcome, not a loss.

### Why the hypotheses hold in production

* `decimals < 27` / `≤ 27`: `MAX_ASSET_DECIMALS = 18` (`common/src/constants`).
* Positive indexes: supply index `≥ SUPPLY_INDEX_FLOOR_RAW = RAY / 1000`
  (INV-IDX-02, ADR-0012), borrow index `≥ RAY` (INV-IDX-01).
* Non-negative share balances and supplies: INV-ACCT-01.
* Non-negative amounts: entry paths reject non-positive amounts
  (INV-ACCT-05); the fee is a reserve-factor share of non-negative interest.
* "Fits": failing, production aborts with `MathOverflow`; only explicit abort
  theorems (such as `rescale_up_overflow`) cover that case.

## Rebuild

```sh
export AENEAS_HOME=/path/to/aeneas-release
export PATH=$HOME/.elan/bin:$PATH
cd verification/aeneas/lean
lake build RatesAeneas.Proofs.FloorCeilResolve   # builds every FloorCeil* file it imports
```

`FloorCeilResolve` imports `FloorCeilRoundTrip` → `FloorCeilScaling` →
`FloorCeilRescale` → `FloorCeilCore` → `FloorCeilSpec`, plus `MulDiv` and
`Scaling`. `TypedWrappers` imports `FloorCeilResolve` and `HalfUpWrappers`.
`Withdrawal` (`lake build RatesAeneas.Proofs.Withdrawal`) imports
`FloorCeilResolve`, `HalfUpWrappers` and `Index` (for `spec_bind_inv`). All
proof files of both `RatesAeneas` documents are plain (non-`module`) Lean,
like `MulDiv.lean`, imported by `RatesAeneas.lean`; `lake build RatesAeneas`
(and `run.sh rates`) checks them all.

## Not covered

* `Wad::mul_floor`, `mul_ceil`, `div_floor`, `div_floor_saturating`,
  `to_token_floor`, `Ray::to_wad_floor`, `to_wad_ceil`,
  `Ray::mul_ratio_ceil`: one-line calls into proved cores (`mul_div_*`,
  `rescale_*`) like the `Ray` wrappers, but unstated.

## Observations on the production code

No proof failed because of the code. Two edge behaviours, both unreachable in
production:

* `rescale` upscaling by 39+ places aborts even for `a = 0`: `10^(to-from)`
  overflows before the multiply (`rescale_up_overflow`). Production upscales
  by at most 27.
* `rescale_floor` truncates a negative value toward zero, returning the
  ceiling (as documented; `rescale_floor_down_spec`). Callers pass
  non-negative amounts.
