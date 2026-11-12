# LP-token oracle pricing

Machine-checked Lean proofs (Aeneas extraction, library `OracleAeneas`) about
the LP-share fair-value pricing in `common/src/oracle/lp.rs` and
`common/src/oracle/lp_stable.rs`, and the helpers they call. Each theorem is
stated about the generated `lean/OracleAeneas/Funs.lean` definition of the
named Rust function. Overview and conventions: [../README.md](../README.md),
[../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

* `oracle::lp::isqrt_of_product`: exact `⌊√(a·b)⌋` for **every** pair of
  `u128` inputs, including termination of its Newton loop. The Certora rule
  `isqrt_is_the_integer_floor_of_the_root` cannot check this
  ([certora/README.md](../../../certora/README.md), "Known prover limits"), so
  this closes that gap, and for the full `u128` domain rather than the rule's
  `u64` inputs.
* `oracle::lp::fair_lp_price_wad` and its helper `reserve_value_wad`:
  degenerate inputs rejected, leg-swap symmetry, the exact price formula,
  balanced-pool agreement with `reserve_value_wad`, and fail-closed behaviour
  (never aborts; every error is `InvalidPrice`).
* `oracle::observation::{try_amount_to_wad, try_u256_to_i128}`,
  `math::fp_core::try_mul_div_half_up`, and the typed wrappers the oracle
  code reaches it through since #224 (`math::fp::Wad::{try_value_of_token,
  try_from_token_upscale, try_mul}`): exact semantics over the whole domain.
* `oracle::lp_stable::{solve_stable_d, fair_stable_lp_price_wad}`: structural
  properties that hold for every input (leg-swap symmetry, only the cheaper
  leg's price matters, out-of-range amplification and out-of-range reserves
  rejected, degenerate inputs rejected); **trap freedom of the Newton loop**
  on every input the guards admit, with an inductive bound `1 ≤ d ≤ 2(x+y)` on
  every iterate; an **accuracy bound** on the returned invariant
  (`D* − 1 < D ≤ D* + 5`, `D*` the exact real root); and the stable price
  formula `⌊D·min_price/S⌋` with fail-closed behaviour for every input. Not
  proved: that the loop always stops before its 255-round cap (see "Not
  covered").

All statements are total correctness (`f ⦃ r => P r ⦄`: the call returns, does
not abort, and `P` holds) or equalities of whole computations (`f x = f y`:
same value, same error, or the same abort). Specifications never divide; they
are in `Proofs/Spec.lean`:

* `IsFloorPos n d q`: `q*d ≤ n < q*d + d` (`q = ⌊n/d⌋`, `d > 0`);
* `IsHalfUp n d q`: `2qd ≤ 2n + d < 2qd + 2d` (`q = ⌊(n + ⌊d/2⌋)/d⌋`, the
  rounding of `try_mul_div_half_up`, `d > 0`);
* the square root is stated as `r*r ≤ a*b < (r+1)*(r+1)`.

Notation: `WAD = 10^18`; `va`, `vb` are the leg values returned by
`reserve_value_wad`; `S` is the share supply returned by `try_amount_to_wad`.

## Extraction crate and shim

`verification/aeneas/oracle/` is its own Cargo workspace (crate
`oracle-aeneas`, namespace `oracle_aeneas` in Lean), never part of the root
workspace or any Wasm build.

Included **verbatim** through `#[path]`:

| Module | Production file |
|---|---|
| `oracle::lp` | `common/src/oracle/lp.rs` |
| `oracle::lp_stable` | `common/src/oracle/lp_stable.rs` |
| `oracle::observation` | `common/src/oracle/observation.rs` (whole file; it extracts without trouble) |
| `math::fp` | `common/src/math/fp.rs` (whole file: `Ray`, `Wad`, `Bps`; the oracle code calls `Wad::from`, `Wad::raw`, `Wad::try_value_of_token`, `Wad::try_from_token_upscale`) |
| `math::fp_core` | `common/src/math/fp_core.rs` (private module, as in production since #224) |
| `constants` | `common/src/constants/mod.rs` (and its `pool`/`shared` submodules) |

Hand-written stand-ins:

* `src/errors.rs`: `GenericError { MathOverflow = 33, DivisionByZero = 55 }` and
  `OracleError { PriceFeedStale = 206, InvalidPrice = 217 }`, the variants the
  included files raise, with the codes of `common/src/errors.rs`. `fp.rs`
  raises only `GenericError::MathOverflow`, so it needed no new variant.
* `shim/` (crate `aeneas-oracle-shim`, renamed to `soroban-sdk`): a copy of the
  rates shim (`Env`, `Error`, `I256`, `panic_with_error!`) extended with
  `U256` (`from_u32`, `from_u128`, `to_u128`, `add`, `sub`, `mul`, `div`,
  `checked_mul`, `Clone`, `PartialEq`, `PartialOrd`, the subset of
  soroban-sdk 28.0.0 `src/num.rs` the code calls) and `assert_with_error!`
  (as in soroban-sdk 28.0.0 `src/lib.rs`). Every shim item is opaque to
  Aeneas.

`oracle/providers/` (contract clients) is not used by the LP code and is
not included. `fp.rs` is included whole, so its other methods are extracted
too (and `Ray::to_wad_floor`/`to_wad_ceil`, `pub(crate)` and unused here, draw
a dead-code warning); only the wrappers listed above carry theorems. The test-only
`solve_stable_y` in `lp_stable.rs` lives in `#[cfg(test)]` and is not
compiled, so it is not extracted.

## Trusted base additions

`lean/OracleAeneas/TypesExternal.lean` and `FunsExternal.lean`:

* **Host `U256` as `Nat`** (new). Every value comes from `from_u32`,
  `from_u128` or an operation that traps unless its exact result is in
  `[0, 2^256)`: `add` and `mul` trap above `2^256 - 1`, `sub` traps on
  underflow, `div` traps on a zero divisor and otherwise rounds down,
  `checked_mul` returns `None` instead of trapping, `to_u128` is `Some`
  exactly below `2^128`, `Clone` is the identity, comparisons are the order
  of the naturals. This follows soroban-sdk 28.0.0 `src/num.rs`, which calls
  the host's `U256.add`/`U256.sub`/`U256.mul`/`U256.div` (trapping) and
  `U256.checked_mul` (returning void on overflow).
* **Host `I256` as `Int`**: copied unchanged from
  `RatesAeneas/FunsExternal.lean` (used by `fp_core`'s widened path).
* **Rust core** (new): `bool::then_some` (`if b then Some(t) else None`),
  `u128::isqrt` (`Nat.sqrt`, "rounded down" per the Rust docs),
  `u128::pow` (exact, failing on overflow; only used for the constant
  `10u128.pow(34)`), and `RangeInclusive::contains` (`start <= item &&
  item <= end`, end exclusive once exhausted, per
  `RangeInclusive::end_bound`). Including `fp.rs` (#224) added no external
  declaration: `Wad::from`'s `Into<i128>` is the Aeneas library's
  `IntoFrom`/`FromSame` (identity), `Option::map` is the library's. `i128::checked_pow`, `u32::abs_diff`,
  `Option::{unwrap_or_else, and_then}` and `?` on `Option` are copied from
  the rates library.

`panic_with_error!` aborts (Aeneas failure), as in the rates library; error
codes of aborts are not modelled, but returned `Err` values are (the
`OracleError` enum is extracted).

## Theorems

Files are under `lean/OracleAeneas/Proofs/`. Every theorem has been checked
with `#print axioms`: each depends only on `propext`, `Classical.choice`,
`Quot.sound`. No `sorry`, `admit` or `axiom`.

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora rule |
|---|---|---|---|---|---|---|
| `isqrt_of_product_spec` | `Isqrt.lean` | `common/src/oracle/lp.rs::isqrt_of_product` | Returns (no trap, loop terminates) `r` with `r*r ≤ a*b < (r+1)*(r+1)` | none: every `a, b : u128` | INV-ORACLE-01 (LP prices are well defined) | `isqrt_is_the_integer_floor_of_the_root` (unprovable there) |
| `isqrt_loop_spec` | `Isqrt.lean` | the `while y < x` loop of `isqrt_of_product` | From any `x` with `⌊√n⌋ ≤ x ≤ 2^128` and `y` the first Newton step, the loop returns exactly `⌊√n⌋`; measure `x` strictly decreases | `2 ≤ n < 2^256`; established by `isqrt_of_product` (`n ≤ 1` returns early; seed `(isqrt a+1)(isqrt b+1)` is in `(⌊√n⌋, 2^128]`) | — | — |
| `isqrt_of_product_eq`, `isqrt_of_product_comm` | `Lp.lean` | `isqrt_of_product` | Equational form `= ok (Nat.sqrt (a*b))`; argument order irrelevant | none | — | — |
| `try_mul_div_half_up_spec` | `HalfUp.lean` | `common/src/math/fp_core.rs::try_mul_div_half_up` | Never aborts; `Some q` with `IsHalfUp (x*y) d q`, or `None` only when that quotient is `≥ 2^127`; native and `I256` paths | `x ≥ 0`, `y ≥ 0`, `d > 0` | INV-RISK-02 (rounding is explicit) | — |
| `try_mul_div_half_up_rejects` | `HalfUp.lean` | same | `None` (no abort) for `x < 0`, `y < 0` or `d ≤ 0` | that condition | — | — |
| `try_u256_to_i128_spec` | `Lp.lean` | `common/src/oracle/observation.rs::try_u256_to_i128` | Never aborts; `Some v` iff `v < 2^127`, else `None` | none | INV-ORACLE-01 | — |
| `Wad_try_value_of_token_spec` | `Fp.lean` | `common/src/math/fp.rs::Wad::try_value_of_token` | Never aborts; `Some w` with `IsHalfUp (amount·price) (10^dec) w` (one rounding, the amount is not normalised first), or `None` only on `10^dec` overflow, a negative input, or a value `≥ 2^127` | none | INV-RISK-02 (rounding is explicit) | — |
| `Wad_try_from_token_upscale_spec` | `Fp.lean` | `fp.rs::Wad::try_from_token_upscale` | Never aborts; `Some(amount·10^(18−dec))` exactly, only if `amount ≥ 0`, `dec ≤ 18`; `None` only if `amount < 0`, `dec > 18` or the product `≥ 2^127` | none | INV-ORACLE-01 | — |
| `Wad_try_mul_spec` | `Fp.lean` | `fp.rs::Wad::try_mul` | Never aborts; `Some w` with `IsHalfUp (x·y) WAD w`, or `None` only for a negative operand or a product `≥ 2^127` (not called by the oracle code; proved because it is the third `Option`-returning `Wad` wrapper) | none | INV-RISK-02 | — |
| `try_amount_to_wad_spec` | `Lp.lean` | `observation.rs::try_amount_to_wad` | Never aborts; `Ok(amount·10^(18−dec))` only if `amount ≥ 0`, `dec ≤ 18`; `Err(InvalidPrice)` only if `amount < 0`, `dec > 18` or overflow. **Changed (#224):** the function lost its `env` argument, so the theorem did too; the claim is unchanged | none | INV-ORACLE-01 | — |
| `reserve_value_wad_spec` | `Lp.lean` | `lp.rs::reserve_value_wad` | Never aborts; `Ok(v)` with `IsHalfUp (reserve·price) (10^dec) v`, or `Err(InvalidPrice)` only on `10^dec` overflow, a negative input, or a value `≥ 2^127`. Unchanged by #224 (now via `Wad::try_value_of_token`, same computation) | none | INV-ORACLE-01, INV-RISK-02 | — |
| `fair_lp_price_wad_rejects_degenerate` | `Lp.lean` | `lp.rs::fair_lp_price_wad` | `= ok (Err InvalidPrice)` (rejects, does not abort) | a reserve, price or the share supply `≤ 0` (any decimals) | INV-ORACLE-01 | `lp_price_rejects_degenerate_inputs` |
| `fair_lp_price_wad_rejects_share_decimals` | `Lp.lean` | same | Returns (no abort) `Err(InvalidPrice)` for every reserve, price and share amount | share `decimals > 18` | INV-ORACLE-01 | `lp_price_rejects_share_decimals_past_wad` |
| `fair_lp_price_wad_symmetric` | `Lp.lean` | same | `fair(a, b, s) = fair(b, a, s)` as computations | none (no input bounds) | — | `lp_price_is_symmetric_under_leg_swap` |
| `fair_lp_price_wad_value` | `Lp.lean` | same | Never aborts; `Ok(p)` with `IsFloorPos (2·⌊√(va·vb)⌋·WAD) S p`, or `Err(InvalidPrice)` exactly when that floor is `≥ 2^127` (formula of `docs/reference/formulas.md`, "LP share prices") | all reserves, prices, shares `> 0`; `reserve_value_wad` gives `Ok va`, `Ok vb`; `try_amount_to_wad(shares)` gives `Ok S`, `S > 0` | INV-ORACLE-01 | — |
| `fair_lp_price_wad_balanced` | `Lp.lean` | same | With both legs worth `v`: `IsFloorPos (2·v·WAD) S p`, i.e. the pool is priced at the sum of its reserve values | as above with `va = vb = v` | INV-ORACLE-01 | unit test `balanced_pool_prices_at_reserve_value` |
| `fair_lp_price_wad_fails_closed` | `Lp.lean` | same | For every input: returns (no trap or panic) `Ok _` or `Err(InvalidPrice)` | none | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` |
| `solve_stable_d_symmetric` | `LpStable.lean` | `lp_stable.rs::solve_stable_d` | `solve(xa, xb, amp) = solve(xb, xa, amp)` as computations (same `D`, error or abort) | none | — | unit test `imbalanced_pool_solves_exactly_in_either_leg_order` |
| `solve_stable_d_rejects_amp` | `LpStable.lean` | same | `= ok (Err InvalidPrice)`, loop not entered | `amp < 1` or `amp > 10^6` | INV-ORACLE-01 | — |
| `solve_stable_d_rejects_reserve` | `LpStable.lean` | same | `= ok (Err InvalidPrice)`, loop not entered | a reserve `≤ 0` or `> 10^34` (`MAX_NORMALIZED_RESERVE_WAD`) | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` (cap case) |
| `fair_stable_lp_price_wad_rejects_degenerate` | `LpStable.lean` | `lp_stable.rs::fair_stable_lp_price_wad` | `= ok (Err InvalidPrice)` | a reserve, price or the share supply `≤ 0`; any `amp` | INV-ORACLE-01 | `stable_lp_price_rejects_degenerate_inputs` |
| `fair_stable_lp_price_wad_rejects_amp` | `LpStable.lean` | same | `= ok (Err InvalidPrice)` for every pool | `amp < 1` or `amp > 10^6` | INV-ORACLE-01 | `stable_lp_price_rejects_out_of_range_amp` |
| `fair_stable_lp_price_wad_symmetric` | `LpStable.lean` | same | `fair(a, b, s, amp) = fair(b, a, s, amp)` as computations | none | — | `stable_lp_price_is_symmetric_under_leg_swap` |
| `fair_stable_lp_price_wad_tracks_cheaper_leg` | `LpStable.lean` | same | Changing leg `b`'s price between two values `p₁, p₂ ≥ price_a` does not change the computation | `price_a ≤ p₁`, `price_a ≤ p₂` | — | `stable_lp_price_tracks_only_the_cheaper_leg` |
| `solve_stable_d_spec` | `StableD.lean` | `lp_stable.rs::solve_stable_d` | Never aborts; returns `Err(InvalidPrice)` or `Ok(D)` with `1 ≤ D ≤ 2(x+y)`, `G(D+1) > 0`, `G(D−5) ≤ 0` (i.e. `D* − 1 < D ≤ D* + 5`), and `D = stableNewton(d)` for an iterate `d` with `|D − d| ≤ 1` (the code's stopping test). `x = min`, `y = max` of the reserves, `G` below | `amp ∈ [1, 10^6]`, `xa, xb ∈ (0, 10^34]` (exactly what the guards admit) | INV-ORACLE-01 | unit tests `imbalanced_pool_grid_solves_symmetrically` (bounds), `imbalanced_pool_solves_exactly_in_either_leg_order` |
| `solve_stable_d_loop_spec` | `StableD.lean` | the `for _ in 0..MAX_D_ITERATIONS` loop of `solve_stable_d` | From any iterate `1 ≤ d ≤ 2(x+y)` and any counter state: never aborts, result as in `solve_stable_d_spec` (`StableDPost`). Invariant `1 ≤ d ≤ 2(x+y)` | `n = 2`, `one = 1`, `1 ≤ x ≤ y ≤ 10^34`, `sum = x+y`, `2 ≤ ann ≤ 2·10^6` (established by `solve_stable_d`) | — | — |
| `solve_stable_d_ok_post` | `StableD.lean` | `solve_stable_d` | `Ok(D)` implies the inputs were in the admitted domain and `D` satisfies `StableDPost` (so `D ≤ 4·10^34`) | `solve_stable_d = ok (Ok D)` | INV-ORACLE-01 | — |
| `solve_stable_d_fails_closed` | `StableD.lean` | `solve_stable_d` | For every input: returns (no trap) `Ok _` or `Err(InvalidPrice)` | none | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` |
| `fair_stable_lp_price_wad_value` | `StableD.lean` | `lp_stable.rs::fair_stable_lp_price_wad` | Never aborts; `Ok(p)` with `IsFloorPos (D·min(price_a, price_b)) S p` (one floor), or `Err(InvalidPrice)` exactly when that floor is `≥ 2^127` | reserves, prices, shares `> 0`; `try_amount_to_wad` gives `Ok xa`, `Ok xb`, `Ok S`; `solve_stable_d(xa, xb, amp) = Ok D` (then `D` satisfies `StableDPost` by `solve_stable_d_ok_post`) | INV-ORACLE-01 | unit tests `fair_price_matches_reference_snapshot`, `depeg_marks_to_the_cheaper_leg` (instances) |
| `fair_stable_lp_price_wad_fails_closed` | `StableD.lean` | same | For every input: returns (no trap or panic) `Ok _` or `Err(InvalidPrice)` | none | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` |

Why the hypotheses hold in production: `fair_lp_price_wad_value` /
`fair_lp_price_wad_balanced` take the helper results as hypotheses; `reserve_value_wad_spec`
and `try_amount_to_wad_spec` say exactly when those are `Ok`, and
`try_amount_to_wad_pos` (`Lp.lean`) proves `S > 0` whenever the share supply
is positive, so the `share_supply_wad <= 0` branch in both pricers is
unreachable. `fair_lp_price_wad_fails_closed` combines all cases with no
hypothesis. The Certora symmetry and cheaper-leg rules bound reserves, prices
and shares; the Lean statements need no bounds.

Supporting lemmas (documented in the files): `newton_ge_root` (integer
AM-GM: a Newton step never undershoots `⌊√n⌋`), `newton_lt_of_gt_root`,
`newton_quot_le`, `root_lt`, `sqrt_u128_lt` (`Isqrt.lean`);
`i128_prod_bounds` (`HalfUp.lean`); `checked_pow_ten_le_18` (`Fp.lean`);
`spec_ok_eq`, `reserve_value_wad_cases`,
`reserve_value_wad_nonneg`, `hcast_u128_val`, `try_amount_to_wad_cases`,
`try_amount_to_wad_pos`, `fair_lp_price_wad_share_err` (`Lp.lean`); `amp_contains`,
`max_normalized_reserve_eq`, `i128_min_comm`, `i128_max_comm`
(`LpStable.lean`); `stableG`, `stableNewton`, `StableDPost` (definitions),
`stableG_mono`, `stableG_neg`, `newton_lower`, `newton_upper`,
`newton_stop_accuracy`, `stable_p_bounds`, `stableNewton_facts`,
`clone_u32`, `lt_u32` (`StableD.lean`); `IsHalfUp.unique`, `IsFloorPos.unique`, `halfUp_of_ediv`,
`floorPos_of_ediv`, `IsHalfUp.one` (`Spec.lean`); the `@[step]` host specs in
`Host.lean`.

## The `solve_stable_d` Newton loop (`Proofs/StableD.lean`)

Notation (naturals, as in the code): `x ≤ y` the sorted WAD reserves,
`S = x + y`, `a = ann = 2·amp`, `K = 4xy`. The invariant the loop solves,
multiplied through by `4xy`, is

    G(D) = D³ + K(a − 1)·D − K·a·S        (G(D) = 0  ⇔  a·S + D = a·D + D³/(4xy))

`G` is strictly increasing (`stableG_mono`), negative at `D ≤ 0`
(`stableG_neg`) and `G(S) = S(S² − 4xy) ≥ 0`, so it has one real root
`D* ∈ (0, S]`. One round from `d` computes `p = ⌊⌊d²/(2x)⌋·d/(2y)⌋`
(≈ `d³/K`) and `d' = ⌊(a·S + 2p)·d / ((a−1)·d + 3p)⌋`, which is exactly
Newton's step for `G` (`stableNewton`). The code stops with `Ok(d')` when
`|d' − d| ≤ 1`, and returns `Err(InvalidPrice)` when a `checked_mul`
(`d_p·d`, `(ann·sum + 2d_p)·d`) overflows or after 255 rounds.

**Iterate bound: every iterate satisfies `1 ≤ d ≤ 2S`.** It starts at `S`.
Upper half (`newton_upper`): the Newton quotient is a weighted mediant of
`a·S/(a−1) ≤ 2S` (as `a ≥ 2`) and `2d/3 ≤ 2S`. Lower half
(`newton_lower`): for `K·p ≤ d³` (true for the rounded `p`,
`stable_p_bounds`), every `s` above the real quotient has `G(s) > 0`; this
is the tangent-line argument in polynomial form,
`d·G(s) > d·s³ − K·p·(3s − 2d) ≥ d·(s − d)²·(s + 2d) ≥ 0`. With `s = d' + 1`
it gives `d' ≥ ⌊D*⌋`, and `⌊D*⌋ ≥ 1` because `G(1) < 0`.

**Why nothing traps.** With `S ≤ 2·10^34` and `d ≤ 2S`: `d·d ≤ 16·10^68`;
`d_p ≤ 2d² ≤ 32·10^68` (`stable_p_bounds`, using `d ≤ 4y`, which is why
dividing by the smaller reserve first matters); `ann·sum ≤ 4·10^40`;
`(ann−1)·d ≤ 8·10^40`; all far below `2^256 ≈ 1.16·10^77`. `ann − 1` never
underflows (`ann ≥ 2`), the convergence test subtracts the smaller value,
and every divisor is positive (`x·2`, `y·2`, denominator `≥ (ann−1)·d ≥ 1`).
The two `checked_mul`s may overflow (very imbalanced pools, e.g. the unit
test's `10^33 : 10^21` at amp 100); that returns `Err(InvalidPrice)`.

**Accuracy of the result (`newton_stop_accuracy`).** When the stopping
test holds, `d' ≥ d − 1`, so `(d − 1)·den ≤ num`. Together with the
rounding bound `K(p + 1) + 2xd > d³` this gives `G(d) ≤ 3d² + K(a + 2)`,
and since `G(d) − G(d−4) = 12d² − 48d + 64 + 4K(a−1)`,
`G(d − 4) ≤ −(3d − 8)² − 3K(a − 2) ≤ 0`. So `d ≤ D* + 4` and the returned
`D ≤ d + 1 ≤ D* + 5`; with the lower half, **`D* − 1 < D ≤ D* + 5`**
(stated without reals as `G(D + 1) > 0` and `G(D − 5) ≤ 0`). Also
`D ≤ 2S`. Sampling 400 000 admitted inputs (all amplifications, ratios up to
the `checked_mul` limit) found `D ∈ {⌊D*⌋, ⌊D*⌋ + 1}` every time and never an
iterate above `S`; the proved window is wider because the proof does not
track the sign of the last step.

**Rounds.** The loop is a `0..255` range loop, so it always terminates;
the proof makes no claim about how many rounds are used. On the same
sample the maximum was 53 rounds (`x = 1`, `y ≈ 4.7·10^25`, amp 1, just
inside the `checked_mul` limit), and no input exhausted the cap. If the cap
were reached the code returns `Err(InvalidPrice)`, which
`solve_stable_d_spec` covers.

`fair_stable_lp_price_wad` then multiplies `D ≤ 2(xa + xb) ≤ 4·10^34` by
`min_price < 2^127` (product `< 2^256`) and divides by the share supply
`S > 0`, so it never traps either.

## Changes after #224 (typed fixed-point boundary)

Commit 8f476115 made `fp_core` private and routed the oracle code through the
`Wad` newtype in `fp.rs`:

* `reserve_value_wad` now calls `Wad::from(price).try_value_of_token(env,
  reserve, decimals)`. That method computes `10^decimals` with
  `checked_pow` and then `try_mul_div_half_up(reserve, price, 10^decimals)`,
  the same two steps, with the same argument order, as the old body. It does
  **not** normalise the reserve to WAD first, so there is still a single
  half-up rounding: the exact value is unchanged and `reserve_value_wad_spec`,
  `fair_lp_price_wad_value` and `fair_lp_price_wad_balanced` keep their
  statements.
* `try_amount_to_wad(env, amount, decimals)` became
  `try_amount_to_wad(amount, decimals)` over `Wad::try_from_token_upscale`
  (sign check, `checked_sub`, `checked_pow`, `checked_mul`) instead of
  `try_mul_div_half_up(amount, 10^(18−decimals), 1)`. For a non-negative
  amount the half-up quotient by 1 is the exact product and both fail exactly
  when it exceeds `i128::MAX`; a negative amount and `decimals > 18` give
  `Err(InvalidPrice)` in both (only the order of the checks changed, which is
  not observable). Same results on every input; the theorems
  (`try_amount_to_wad_spec`, `try_amount_to_wad_cases`, `try_amount_to_wad_pos`) only lost the `env` argument.
  `try_normalize_positive_price` likewise now calls
  `Wad::try_from_token_upscale` (no theorem covers it).
* The extraction crate now includes `fp.rs` verbatim and keeps `fp_core`
  private; no stand-in or trusted-base change was needed. The new
  `Fp.lean` proves the wrappers on their own (`Wad_try_value_of_token_spec`,
  `Wad_try_from_token_upscale_spec`, `Wad_try_mul_spec`) and the `Lp.lean`
  specs are now derived from them.

No previously proved property fails after the refactor, and no production
behaviour change was found.

## Rebuilding

```sh
export AENEAS_HOME=/path/to/aeneas-nightly-2026.11.12-0d21a9e   # see ../README.md, "Setup"
export PATH=$HOME/.elan/bin:$PATH
verification/aeneas/run.sh oracle
```

or step by step:

```sh
cd verification/aeneas/oracle
"$AENEAS_HOME/charon" cargo --preset=aeneas --dest-file target/llbc/oracle_aeneas.llbc
"$AENEAS_HOME/aeneas" -backend lean -dest ../lean/OracleAeneas -split-files target/llbc/oracle_aeneas.llbc
rm -f ../lean/OracleAeneas/*_Template.lean
cd ../lean && lake build OracleAeneas
```

Re-extraction is deterministic (checked: a second run produces an identical
`Funs.lean`). `run.sh` fails if the regenerated `Funs.lean`/`Types.lean` differ
from the committed ones once they are tracked (`ALLOW_DRIFT=1` overrides).

## Not covered

* **That `solve_stable_d` always stops before its 255-round cap.** Trap
  freedom, the iterate bound and the accuracy of every returned `D` are
  proved (`solve_stable_d_spec`), but a round count needs a quantitative
  convergence argument (geometric decrease far from the root, about
  `log_{3/2}(S/D*) ≤ 195` rounds, then a no-cycle argument near it) that is
  not formalised. So `Err(InvalidPrice)` from cap exhaustion is not excluded
  by a proof; sampling found at most 53 rounds (see the loop section).
* **A sharper accuracy window.** The proof gives `D* − 1 < D ≤ D* + 5`;
  sampling only ever found `D ∈ {⌊D*⌋, ⌊D*⌋ + 1}`, and never an iterate above
  `S` (the proof only bounds iterates by `2S`).
* **`solve_stable_y`** is test-only (`#[cfg(test)]`), not compiled into the
  extraction, so nothing is stated about it (and so not the unit test
  `swap_cannot_move_the_price`).
* **Error codes of aborts** are not modelled (as in the rates library);
  returned `Err(OracleError::…)` values are.

No production defect was found. Remarks: the `share_supply_wad <= 0` checks
in both pricers are unreachable (`try_amount_to_wad_pos`); the Certora
counterexample for `isqrt_is_the_integer_floor_of_the_root`
(`a = 0x55555555555556, b = 3`) is a prover-model artifact, since
`isqrt_of_product_spec` holds for every input.
