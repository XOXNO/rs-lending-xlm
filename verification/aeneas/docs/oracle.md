# LP-token oracle pricing

Lean proofs (library `OracleAeneas`, about its generated `Funs.lean`) of the
LP-share pricing in `common/src/oracle/lp.rs` and
`common/src/oracle/lp_stable.rs` and its helpers. Conventions:
[../README.md](../README.md), [../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

* `oracle::lp::isqrt_of_product`: exact `⌊√(a·b)⌋`, loop terminating, for
  **every** `u128` pair. The Certora rule
  `isqrt_is_the_integer_floor_of_the_root` cannot check this even on `u64` ([certora/README.md](../../../certora/README.md), "Known prover
  limits").
* `oracle::lp::fair_lp_price_wad` and `oracle::lp_stable::fair_stable_lp_price_wad`:
  degenerate inputs rejected, leg-swap symmetry, exact price formula, fail
  closed (never aborts; every error is `InvalidPrice`).
* `oracle::lp_stable::solve_stable_d`: **Newton loop never traps** on admitted
  inputs, every iterate `1 ≤ d ≤ 2(x+y)`, and **accuracy**
  `D* − 1 < D ≤ D* + 5` (`D*` the exact real root).
* Exact semantics of `reserve_value_wad`,
  `oracle::observation::{try_amount_to_wad, try_u256_to_i128}`,
  `math::fp_core::try_mul_div_half_up` and the wrappers
  `math::fp::Wad::{try_value_of_token, try_from_token_upscale, try_mul}`.

Statements are total correctness (`f ⦃ r => P r ⦄`) or equality of whole
computations (`f x = f y`: same value, error or abort). Division-free specs
(`Proofs/Spec.lean`):

* `IsFloorPos n d q`: `q*d ≤ n < q*d + d` (`q = ⌊n/d⌋`, `d > 0`);
* `IsHalfUp n d q`: `2qd ≤ 2n + d < 2qd + 2d` (`q = ⌊(n + ⌊d/2⌋)/d⌋`, the
  rounding of `try_mul_div_half_up`, `d > 0`);
* square root: `r*r ≤ a*b < (r+1)*(r+1)`.

Notation: `WAD = 10^18`; `va`, `vb` leg values from `reserve_value_wad`; `S`
share supply from `try_amount_to_wad`.

## Extraction crate and shim

`verification/aeneas/oracle/`: own Cargo workspace (crate `oracle-aeneas`,
namespace `oracle_aeneas`). **Verbatim** `#[path]` includes:

| Module | Production file |
|---|---|
| `oracle::lp` | `common/src/oracle/lp.rs` |
| `oracle::lp_stable` | `common/src/oracle/lp_stable.rs` |
| `oracle::observation` | `common/src/oracle/observation.rs` |
| `math::fp` | `common/src/math/fp.rs` (whole: `Ray`, `Wad`, `Bps`; oracle calls `Wad::from`, `Wad::raw`, `Wad::try_value_of_token`, `Wad::try_from_token_upscale`) |
| `math::fp_core` | `common/src/math/fp_core.rs` (private, as in production) |
| `constants` | `common/src/constants/mod.rs` with `pool`/`shared` |

Hand-written stand-ins:

* `src/errors.rs`: `GenericError { MathOverflow = 33, DivisionByZero = 55 }`,
  `OracleError { PriceFeedStale = 206, InvalidPrice = 217 }`: the variants
  raised, codes as in `common/src/errors.rs`.
* `shim/` (crate `aeneas-oracle-shim`, renamed `soroban-sdk`): rates shim (`Env`, `Error`, `I256`, `panic_with_error!`) plus `U256` (`from_u32`,
  `from_u128`, `to_u128`, `add`, `sub`, `mul`, `div`, `checked_mul`, `Clone`,
  `PartialEq`, `PartialOrd`: the subset of soroban-sdk 28.0.0 `src/num.rs` the
  code calls) and `assert_with_error!` (as in soroban-sdk 28.0.0
  `src/lib.rs`).

`oracle/providers/` is unused and not included. Other `fp.rs` methods are
extracted but carry no theorem; the unused `pub(crate)` `Ray::to_wad_floor`
and `to_wad_ceil` draw a dead-code warning. `#[cfg(test)]` `solve_stable_y`
is not extracted.

## Trusted base additions

`lean/OracleAeneas/TypesExternal.lean` and `FunsExternal.lean`:

* **Host `U256` as `Nat`** in `[0, 2^256)`, per soroban-sdk 28.0.0
  `src/num.rs` (host `U256.add`/`sub`/`mul`/`div` trap, `U256.checked_mul`
  returns void on overflow): `add`, `mul` trap above `2^256 - 1`; `sub` traps
  on underflow; `div` traps on zero, else rounds down; `checked_mul` gives
  `None`; `to_u128` is `Some` exactly below `2^128`; `Clone` is identity;
  comparisons are the order on naturals.
* **Host `I256` as `Int`**: copied from `RatesAeneas/FunsExternal.lean`
  (`fp_core`'s widened path).
* **Rust core**: `bool::then_some` = `if b then Some(t) else None`;
  `u128::isqrt` = `Nat.sqrt` ("rounded down", Rust docs); `u128::pow` exact,
  failing on overflow (only `10u128.pow(34)`); `RangeInclusive::contains` =
  `start <= item && item <= end`, end exclusive once exhausted
  (`RangeInclusive::end_bound`).

`fp.rs` adds no declaration: `Wad::from`'s `Into<i128>` is the library's
`IntoFrom`/`FromSame` (identity), `Option::map` the library's;
`i128::checked_pow`, `u32::abs_diff`, `Option::{unwrap_or_else, and_then}`
and `?` on `Option` are copied from the rates library. Returned `Err` values
are modelled (`OracleError` is extracted).

## Theorems

Files under `lean/OracleAeneas/Proofs/`; `Lp.lean` builds on the `Fp.lean`
wrapper specs. Unless a row says otherwise, the call never aborts, and "`Err`"
is `Err(InvalidPrice)`.

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora rule |
|---|---|---|---|---|---|---|
| `isqrt_of_product_spec` | `Isqrt.lean` | `common/src/oracle/lp.rs::isqrt_of_product` | `r` with `r*r ≤ a*b < (r+1)*(r+1)`; loop terminates | none | INV-ORACLE-01 | `isqrt_is_the_integer_floor_of_the_root` (unprovable there) |
| `isqrt_loop_spec` | `Isqrt.lean` | the `while y < x` loop of `isqrt_of_product` | From `⌊√n⌋ ≤ x ≤ 2^128`, `y` the first Newton step: returns `⌊√n⌋`; `x` strictly decreases | `2 ≤ n < 2^256` (from `isqrt_of_product`: `n ≤ 1` returns early; seed `(isqrt a+1)(isqrt b+1)` in `(⌊√n⌋, 2^128]`) | — | — |
| `isqrt_of_product_eq`, `isqrt_of_product_comm` | `Lp.lean` | `isqrt_of_product` | `= ok (Nat.sqrt (a*b))`; order irrelevant | none | — | — |
| `try_mul_div_half_up_spec` | `HalfUp.lean` | `common/src/math/fp_core.rs::try_mul_div_half_up` | `Some q`, `IsHalfUp (x*y) d q`; `None` only if `q ≥ 2^127`; both paths | `x ≥ 0`, `y ≥ 0`, `d > 0` | INV-RISK-02 | — |
| `try_mul_div_half_up_rejects` | `HalfUp.lean` | same | `None` | `x < 0`, `y < 0` or `d ≤ 0` | — | — |
| `try_u256_to_i128_spec` | `Lp.lean` | `common/src/oracle/observation.rs::try_u256_to_i128` | `Some v` iff `v < 2^127` | none | INV-ORACLE-01 | — |
| `Wad_try_value_of_token_spec` | `Fp.lean` | `common/src/math/fp.rs::Wad::try_value_of_token` | `Some w`, `IsHalfUp (amount·price) (10^dec) w` (one rounding, amount not normalised first); `None` only on `10^dec` overflow, negative input or `w ≥ 2^127` | none | INV-RISK-02 | — |
| `Wad_try_from_token_upscale_spec` | `Fp.lean` | `fp.rs::Wad::try_from_token_upscale` | `Some(amount·10^(18−dec))` if `amount ≥ 0`, `dec ≤ 18` and it is `< 2^127`; else `None` | none | INV-ORACLE-01 | — |
| `Wad_try_mul_spec` | `Fp.lean` | `fp.rs::Wad::try_mul` | `Some w`, `IsHalfUp (x·y) WAD w`; `None` only for a negative operand or `w ≥ 2^127` (unused by the LP pricers; the price aggregator uses it) | none | INV-RISK-02 | — |
| `try_amount_to_wad_spec` | `Lp.lean` | `observation.rs::try_amount_to_wad` | As `Wad_try_from_token_upscale_spec`, `Ok`/`Err` for `Some`/`None` | none | INV-ORACLE-01 | — |
| `reserve_value_wad_spec` | `Lp.lean` | `lp.rs::reserve_value_wad` | As `Wad_try_value_of_token_spec`, `Ok`/`Err` for `Some`/`None` | none | INV-ORACLE-01, INV-RISK-02 | — |
| `fair_lp_price_wad_rejects_degenerate` | `Lp.lean` | `lp.rs::fair_lp_price_wad` | `= ok (Err InvalidPrice)` | a reserve, price or share supply `≤ 0` | INV-ORACLE-01 | `lp_price_rejects_degenerate_inputs` |
| `fair_lp_price_wad_rejects_share_decimals` | `Lp.lean` | same | `Err` for every reserve, price, shares | share `decimals > 18` | INV-ORACLE-01 | `lp_price_rejects_share_decimals_past_wad` |
| `fair_lp_price_wad_symmetric` | `Lp.lean` | same | `fair(a, b, s) = fair(b, a, s)` | none | — | `lp_price_is_symmetric_under_leg_swap` |
| `fair_lp_price_wad_value` | `Lp.lean` | same | `Ok(p)`, `IsFloorPos (2·⌊√(va·vb)⌋·WAD) S p`; `Err` iff `p ≥ 2^127` (`docs/reference/formulas.md`, "LP share prices") | reserves, prices, shares `> 0`; helpers give `Ok va`, `Ok vb`, `Ok S`, `S > 0` | INV-ORACLE-01 | — |
| `fair_lp_ok_formula` | `Lp.lean` | same | Every `Ok p`: the helpers returned `Ok va`, `Ok vb`, `Ok S` and `IsFloorPos (2·⌊√(va·vb)⌋·WAD) S p` (converse of `fair_lp_price_wad_value`) | none | INV-ORACLE-01 | — |
| `fair_lp_price_wad_balanced` | `Lp.lean` | same | `IsFloorPos (2·v·WAD) S p`: priced at the sum of reserve values | as above with `va = vb = v` | INV-ORACLE-01 | unit test `balanced_pool_prices_at_reserve_value` |
| `fair_lp_price_wad_fails_closed` | `Lp.lean` | same | `Ok _` or `Err`, every input | none | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` |
| `solve_stable_d_symmetric` | `LpStable.lean` | `lp_stable.rs::solve_stable_d` | `solve(xa, xb, amp) = solve(xb, xa, amp)` | none | — | unit test `imbalanced_pool_solves_exactly_in_either_leg_order` |
| `solve_stable_d_rejects_amp` | `LpStable.lean` | same | `= ok (Err InvalidPrice)`, before the loop | `amp < 1` or `amp > 10^6` | INV-ORACLE-01 | — |
| `solve_stable_d_rejects_reserve` | `LpStable.lean` | same | `= ok (Err InvalidPrice)`, before the loop | a reserve `≤ 0` or `> 10^34` (`MAX_NORMALIZED_RESERVE_WAD`) | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` (cap case) |
| `fair_stable_lp_price_wad_rejects_degenerate` | `LpStable.lean` | `lp_stable.rs::fair_stable_lp_price_wad` | `= ok (Err InvalidPrice)` | a reserve, price or share supply `≤ 0`; any `amp` | INV-ORACLE-01 | `stable_lp_price_rejects_degenerate_inputs` |
| `fair_stable_lp_price_wad_rejects_amp` | `LpStable.lean` | same | `= ok (Err InvalidPrice)` | `amp < 1` or `amp > 10^6` | INV-ORACLE-01 | `stable_lp_price_rejects_out_of_range_amp` |
| `fair_stable_lp_price_wad_symmetric` | `LpStable.lean` | same | `fair(a, b, s, amp) = fair(b, a, s, amp)` | none | — | `stable_lp_price_is_symmetric_under_leg_swap` |
| `fair_stable_lp_price_wad_tracks_cheaper_leg` | `LpStable.lean` | same | Same computation for any leg-`b` price `p₁, p₂ ≥ price_a` | `price_a ≤ p₁`, `price_a ≤ p₂` | — | `stable_lp_price_tracks_only_the_cheaper_leg` |
| `solve_stable_d_spec` | `StableD.lean` | `lp_stable.rs::solve_stable_d` | `Err` or `Ok(D)`: `1 ≤ D ≤ 2(x+y)`, `G(D+1) > 0`, `G(D−5) ≤ 0` (`D* − 1 < D ≤ D* + 5`), `D = stableNewton(d)` for an iterate `d`, `\|D − d\| ≤ 1` (stopping test); `x`/`y` min/max reserve, `G` below | `amp ∈ [1, 10^6]`, `xa, xb ∈ (0, 10^34]` (the guards' domain) | INV-ORACLE-01 | unit tests `imbalanced_pool_grid_solves_symmetrically` (bounds), `imbalanced_pool_solves_exactly_in_either_leg_order` |
| `solve_stable_d_loop_spec` | `StableD.lean` | the `for _ in 0..MAX_D_ITERATIONS` loop of `solve_stable_d` | From any iterate `1 ≤ d ≤ 2(x+y)` (the invariant) and counter: result `StableDPost` | `n = 2`, `one = 1`, `1 ≤ x ≤ y ≤ 10^34`, `sum = x+y`, `2 ≤ ann ≤ 2·10^6` (from `solve_stable_d`) | — | — |
| `stableG_strict` | `StableD.lean` | (math) | `u < v ⇒ G(u) < G(v)` | `K > 0`, `a ≥ 2` | — | — |
| `post_brackets` | `StableD.lean` | (math) | `StableDPost (Ok D)`: every integer `z` with `G z ≤ 0` is `< D + 1`, every `z` with `G z > 0` is `> D − 5` | `x, y ≥ 1`, `a ≥ 2` | INV-ORACLE-01 | — |
| `solve_stable_d_ok_post` | `StableD.lean` | `solve_stable_d` | `Ok(D)` ⇒ inputs admitted, `StableDPost D` (so `D ≤ 4·10^34`) | `solve_stable_d = ok (Ok D)` | INV-ORACLE-01 | — |
| `solve_stable_d_fails_closed` | `StableD.lean` | `solve_stable_d` | `Ok _` or `Err`, every input | none | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` |
| `fair_stable_lp_price_wad_value` | `StableD.lean` | `lp_stable.rs::fair_stable_lp_price_wad` | `Ok(p)`, `IsFloorPos (D·min(price_a, price_b)) S p` (one floor); `Err` iff `p ≥ 2^127` | reserves, prices, shares `> 0`; `try_amount_to_wad` gives `Ok xa`, `Ok xb`, `Ok S`; `solve_stable_d = Ok D` (`StableDPost` by `solve_stable_d_ok_post`) | INV-ORACLE-01 | unit tests `fair_price_matches_reference_snapshot`, `depeg_marks_to_the_cheaper_leg` (instances) |
| `fair_stable_lp_price_wad_fails_closed` | `StableD.lean` | same | `Ok _` or `Err`, every input | none | INV-ORACLE-01 | unit test `absurd_reserve_errors_not_panics` |

Production hypotheses: `reserve_value_wad_spec` and `try_amount_to_wad_spec`
say exactly when the helper results `fair_lp_price_wad_value` /
`fair_lp_price_wad_balanced` assume are `Ok`. `try_amount_to_wad_pos`
(`Lp.lean`) gives `S > 0` for positive share supply, so the
`share_supply_wad <= 0` branch of both pricers is unreachable. The Certora
symmetry and cheaper-leg rules bound reserves, prices and shares; the Lean
statements do not.

Supporting lemmas are documented in the `Proofs/*.lean` files; the `@[step]`
host specs are in `Host.lean`.

## The `solve_stable_d` Newton loop (`Proofs/StableD.lean`)

Notation (naturals): `x ≤ y` sorted WAD reserves, `S = x + y`,
`a = ann = 2·amp`, `K = 4xy`. The invariant, times `4xy`:

    G(D) = D³ + K(a − 1)·D − K·a·S        (G(D) = 0  ⇔  a·S + D = a·D + D³/(4xy))

`G` increases (`stableG_mono`; strictly for `K > 0`, `a ≥ 2`: `stableG_strict`), is negative at `D ≤ 0`
(`stableG_neg`), and `G(S) = S(S² − 4xy) ≥ 0`: one real root
`D* ∈ (0, S]`. A round from `d` computes `p = ⌊⌊d²/(2x)⌋·d/(2y)⌋` (≈ `d³/K`)
and Newton's step `d' = ⌊(a·S + 2p)·d / ((a−1)·d + 3p)⌋` (`stableNewton`).
It returns `Ok(d')` when `|d' − d| ≤ 1`, and `Err(InvalidPrice)` when a
`checked_mul` (`d_p·d`, `(ann·sum + 2d_p)·d`) overflows or after 255 rounds.

**Iterate bound `1 ≤ d ≤ 2S`**, starting at `S`. Upper (`newton_upper`): the
quotient is a weighted mediant of `a·S/(a−1) ≤ 2S` (`a ≥ 2`) and
`2d/3 ≤ 2S`. Lower (`newton_lower`): for `K·p ≤ d³` (the rounded `p`,
`stable_p_bounds`), every `s` above the real quotient has `G(s) > 0`, by the
polynomial tangent-line argument
`d·G(s) > d·s³ − K·p·(3s − 2d) ≥ d·(s − d)²·(s + 2d) ≥ 0`. With `s = d' + 1`
it gives `d' ≥ ⌊D*⌋`, and `⌊D*⌋ ≥ 1` because `G(1) < 0`.

**Why nothing traps.** With `S ≤ 2·10^34` and `d ≤ 2S`: `d·d ≤ 16·10^68`;
`d_p ≤ 2d² ≤ 32·10^68` (`stable_p_bounds`, via `d ≤ 4y`: why the smaller
reserve divides first); `ann·sum ≤ 4·10^40`;
`(ann−1)·d ≤ 8·10^40`; all far below `2^256 ≈ 1.16·10^77`. `ann − 1` never
underflows (`ann ≥ 2`), the convergence test subtracts the smaller value,
every divisor is positive (`x·2`, `y·2`, denominator `≥ (ann−1)·d ≥ 1`).
The two `checked_mul`s may overflow on very imbalanced pools (unit test:
`10^33 : 10^21` at amp 100), returning `Err(InvalidPrice)`.

**Accuracy (`newton_stop_accuracy`).** At the stop, `d' ≥ d − 1`, so
`(d − 1)·den ≤ num`. With the rounding bound
`K(p + 1) + 2xd > d³` this gives `G(d) ≤ 3d² + K(a + 2)`, and since
`G(d) − G(d−4) = 12d² − 48d + 64 + 4K(a−1)`,
`G(d − 4) ≤ −(3d − 8)² − 3K(a − 2) ≤ 0`. So `d ≤ D* + 4` and the returned
`D ≤ d + 1 ≤ D* + 5`; with the lower half, **`D* − 1 < D ≤ D* + 5`**
(stated without reals as `G(D + 1) > 0` and `G(D − 5) ≤ 0`), and `D ≤ 2S`.

**Rounds.** The `0..255` loop always terminates; no round count is claimed.
Reaching the cap returns `Err(InvalidPrice)`, covered by
`solve_stable_d_spec`.

**Sampling.** 400 000 admitted inputs (all amplifications, ratios up to the
`checked_mul` limit) always gave `D ∈ {⌊D*⌋, ⌊D*⌋ + 1}`, never an
iterate above `S`, and at most 53 rounds (`x = 1`, `y ≈ 4.7·10^25`, amp 1,
just inside the `checked_mul` limit). No input exhausted the cap.

`fair_stable_lp_price_wad` multiplies `D ≤ 4·10^34` by `min_price < 2^127`
(`< 2^256`) and divides by `S > 0`: no trap.

## Rebuilding

```sh
export AENEAS_HOME=/path/to/aeneas-nightly-2026.11.12-0d21a9e   # see ../README.md, "Setup"
export PATH=$HOME/.elan/bin:$PATH
verification/aeneas/run.sh oracle
```

Manual steps: [the pipeline](../WRITING-PROOFS.md#pipeline) with crate
`oracle_aeneas`, library `OracleAeneas`. Re-extraction is deterministic.

## Not covered

* **`solve_stable_d` stopping before its 255-round cap**, so `Err(InvalidPrice)`
  from cap exhaustion is not excluded. That needs an unformalised convergence
  argument: geometric decrease far from the root (about
  `log_{3/2}(S/D*) ≤ 195` rounds), then no cycle near it.
* **A window sharper than `D* − 1 < D ≤ D* + 5`**, or an iterate bound below
  `2S`: the proof does not track the sign of the last step.
* **`solve_stable_y`** (test-only, not extracted), hence the unit test
  `swap_cannot_move_the_price`.
* **`try_normalize_positive_price`** itself (its callee
  `Wad::try_from_token_upscale` is proved).

No production defect found. The Certora counterexample for
`isqrt_is_the_integer_floor_of_the_root` (`a = 0x55555555555556, b = 3`) is a
prover-model artifact: `isqrt_of_product_spec` holds for every input.
