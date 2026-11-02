# Komet formal-verification inventory

Every Komet claim, K lemma module and Lean file in this branch, plus the
Certora rules used to choose new Komet targets. Status words:

* **proved (server)**: re-proved by this pipeline (`run-proofs.sh`) with the
  pinned toolchain; evidence in the proof's README and the CI bundle.
* **proved (Mac, historical)**: completed on the original macOS runtime only.
* **control fails (server)** / **(Mac)**: a deliberately false claim rejected
  at a completed `false` assertion.
* **open**: a symbolic run was attempted and did not complete (reason given).
* **concrete only**: only concrete `komet test` runs or native `cargo test`.
* **never run**: no symbolic Komet run yet.

Statuses are updated as runs complete; see §5 and
each proof README for numbers.

## 1. Komet harness claims

`pm` = `komet/pool-math` (`pool-math-proof`), features in brackets.
"Domain" is the guard outside which the harness returns `true`.

### Fixed-point and rounding (`pm`)

| Test | Domain | Claim | Production | Status |
|---|---|---|---|---|
| `test_half_up_exact` [half-up] | all i128 triples | `try_mul_div_half_up` is `None` exactly for invalid inputs or a quotient >= 2^127; otherwise `q*d <= x*y + d/2 < (q+1)*d` | `fp_core::try_mul_div_half_up` | concrete only; Mac runs interrupted |
| `test_half_two` [half-two] | every i128 | `div_by_int_half_up(a, 2) == a/2 + a%2` (ties away from zero) | `fp_core::div_by_int_half_up` | concrete only; Mac runs interrupted |
| `test_half_two_wrong` [half-two] | — | `half(1) == 0` | same | control fails (Mac) |
| `test_floor_exact` [signed-floor] | divisor != 0, floor fits | `q*D <= x*y < (q+1)*D` (sign-normalised) | `fp_core::mul_div_floor` | concrete only |
| `test_ceil_exact` [signed-ceil] | divisor != 0, ceil fits | `(q-1)*D < x*y <= q*D` | `fp_core::mul_div_ceil` | concrete only |
| `test_saturating_floor` [signed-saturation] | divisor != 0 | floor, saturating at i128::MIN/MAX | `fp_core::mul_div_floor_saturating` | concrete only |
| `test_ray_mul_floor` / `_ceil` [ray-mul-rounding] | x, y >= 2^64, result fits | exact floor / ceil of `x*y/RAY` (widened I256 path) | `Ray::mul_floor`, `Ray::mul_ceil` | never run (new) |
| `test_ray_mul_floor_wrong` / `_ceil_wrong` | — | `(RAY+1)^2/RAY` rounded the wrong way | same | never run (new) |
| `test_ray_div_floor` / `_ceil` [ray-div-rounding] | x >= 2^64, d > 0, fits | exact floor / ceil of `x*RAY/d` | `Ray::div_floor`, `Ray::div_ceil` (ADR-0003 supply floor / debt ceil core) | never run (new) |
| `test_ray_div_floor_wrong` / `_ceil_wrong` | — | `2^64*RAY/(3*RAY)` rounded the wrong way | same | never run (new) |
| `test_wad_try_mul_negative` [noop-guards] | a < 0 or b < 0 (all i128) | `Wad::try_mul` is `None` | `Wad::try_mul` | never run (new) |
| `test_wad_try_mul_negative_wrong` | — | `try_mul(-1, 0) == Some(0)` | same | never run (new) |
| `test_from_asset_7_exact` [rescale-up] | product fits i128 (all signs) | `Ray::from_asset(a, 7) == a * 10^20` | `Ray::from_asset` → `rescale_half_up` | never run (new) |
| `test_from_asset_7_wrong` | — | one unit is 10^19 | same | never run (new) |
| `test_widened_ceil` [controls] | — | `mul_div_ceil(RAY+1, RAY+1, RAY) == RAY+3` | `fp_core::mul_div_ceil` | concrete only |

### Rates and indexes (`pm`)

| Test | Domain | Claim | Production | Status |
|---|---|---|---|---|
| `test_borrow_double` [borrow-double] | `RAY <= old <= 10^36` | `update_borrow_index(old, 2*RAY) == min(2*old, 10^36)` | `rates::update_borrow_index` | proved (Mac, historical); server re-proof: see §5 |
| `test_borrow_double_wrong` | — | at `old = cap/2+1`, result is `2*old` | same | control fails (Mac) |
| `test_borrow_index_identity` [index-identity] | `RAY <= old <= 10^36` | growth by RAY is the identity | same | never run (new) |
| `test_borrow_index_identity_wrong` | — | `update(RAY+1, RAY) == RAY` | same | never run (new) |
| `test_borrow_cap_sticky` [index-identity] | `RAY < factor <= 100*RAY` | `update(10^36, factor) == 10^36` | same | never run (new) |
| `test_borrow_cap_sticky_wrong` | — | doubling at the cap gives `2*10^36` | same | never run (new) |
| `test_borrow_index_bounds` [borrow-index] | `RAY <= old <= 10^36`, `RAY <= factor <= 8*RAY` | `old <= result <= 10^36` | same | concrete only |
| `test_borrow_index_exact` [index-exact] | valid old, `factor >= RAY`, pre-cap result fits | exact half-up growth, then cap; `result >= old` | same | concrete only |
| `test_supply_index_exact` [supply-index] | supplied >= 0, `10^24 <= old <= 10^36`, rewards >= 0, values fit | early returns exact; else `clamp(floor((oldVal+rewards)*RAY/supplied), old, 10^36)` | `rates::update_supply_index` | concrete only |
| `test_supply_zero_inputs` [noop-guards] | supplied == 0 or rewards == 0 (all i128) | result == old | same | never run (new) |
| `test_supply_zero_inputs_wrong` | — | `update(0, RAY, RAY) == RAY+1` | same | never run (new) |
| `test_supply_rewards` [reward-conservation] | as supply-index | distributed + shortfall == rewards, both bounded | `update_supply_index`, `supply_index_reward_shortfall` | concrete only |
| `test_interest_split` [interest-split] | valid monotone indexes, values fit, bps < 10000 | supplier + fee == accrued, fee = half-up bps share | `rates::calculate_supplier_rewards` | concrete only |
| `test_fee_shares` [fee-shares] | fee, supplied >= 0, valid index | `clamp(floor(fee*RAY/index), 0, MAX - supplied)` | `rates::protocol_fee_shares` | concrete only |
| `test_flash_fee` [flash-fee] | amount >= 0, every u32 rate, fits | half-up fee, minimum 1 for a positive rate | `Bps::flash_loan_fee_on` | concrete only |
| `test_compound_zero_delta` [noop-guards] | every i128 rate | `compound_interest(rate, 0) == RAY` | `rates::compound_interest` | never run (new) |
| `test_compound_zero_delta_wrong` | — | `compound(1, 0) == RAY+1` | same | never run (new) |
| `test_utilization_bounds` [utilization] | `0 <= b <= s`, `s > 0` | `0 <= U <= RAY` | `rates::utilization` | concrete only |
| `test_utilization_exact` [utilization] | b, s >= 0; s == 0 or b <= s | `U == 0` at s == 0; else exact half-up quotient in `[0, RAY]` | same | open (Mac: 39-rule module, not closed) |
| `test_utilization_round_wrong` | — | `U(1, 2*RAY) == 0` | same | control fails (Mac) |
| `test_util_zero_supply` [controls] | every i128 borrowed | `U(b, 0) == 0` | same | proved (Mac, historical; older semantics) |
| `test_utilization_wrong` [controls] | `0 <= b <= s`, `s > 0` | `U < RAY` (false at full use) | same | concrete control |
| `test_full_utilization` / `_wrong` [controls] | — | `U(RAY, RAY) == RAY` / `< RAY` | same | Mac diagnostic only |

### Settlements (`pm`)

| Test | Claim | Production | Status |
|---|---|---|---|
| `test_repay` [repay] | full close or exact floor burn; refund and burn bounds | `rates::resolve_repay` | concrete only |
| `test_withdrawal` [withdrawal] | half-up full-close threshold, floor payout, ceil partial burn | `rates::resolve_withdrawal` | concrete only |
| `test_net_settle` [net-settle] | overlap = min(request, floor supply, ceil debt); burns bounded | `rates::resolve_net_settle` | concrete only |

### Other harnesses

| Crate | Test | Claim | Status |
|---|---|---|---|
| `komet/pool-state` | `test_supply_exact(amount)` | real pool `supply` on an empty 7-decimal market: exact mutation and state deltas | open (Mac runs interrupted at 23–26 min) |
| `komet/pool-state` | `test_supply_one_unit()` | the same at amount 1 | concrete only |
| `komet/oracle-staleness` | `test_fresh_never_stale(now, max_age)` | a feed stamped `now` is never stale | proved (Mac, historical) |
| `komet/oracle-staleness` | `test_fresh_wrong(now, max_age)` | false inverse | control fails (Mac) |

## 2. K lemma modules

| Module | Location | Rules | Justification |
|---|---|---|---|
| `INT-BITWISE-LEMMAS` | Komet `komet-lemmas.md` @7e74d11 | 29 (incl. 10 U64 projection rules) + 6 side-condition definition rules | Lean: `pipeline/proofs/komet-lemmas/lean/{IntBitwiseLemmas,U64Lemmas}.lean` |
| `HOST-OBJECT-LEMMAS` | same | 14 | Lean: `HostObjectLemmas.lean` (minWidth, signed/unsigned, memory model) |
| `U64-BITWISE-SMT` | same | `#u64Or`, `#u64Xor` + 2 concrete rules + bv SMT hooks | Lean: `U64Lemmas.smt_hook_or/xor` |
| `SIGNED-…-FIXED64-XOR` | `pool-math/proofs/borrow-double/double-cli.k` | 17 | Lean: `DoubleCliRules.lean` (one theorem per rule) |
| `UTILIZATION-COMPLEMENT-LEMMAS` | `pool-math/proofs/utilization/arithmetic.k` | 39 (17 shared + 22 `utilization-*`) | Lean: `utilization/lean/*` (rule-by-rule mapping pending) |

Backend repairs that matter for soundness: the erased-operand definedness
guard (with requires rollback) in Booster `ApplyEquations.hs`; see
[TRUSTED-BASE.md](TRUSTED-BASE.md).

## 3. Lean files (Lean 4.22.0, `--trust=0`)

| Directory | Files | Content |
|---|---|---|
| `pipeline/proofs/komet-lemmas/lean` | KBits, IntBitwiseLemmas, HostObjectLemmas, U64Lemmas | bit model of K integers; every Komet lemma rule; SMT hook agreement |
| `pool-math/proofs/borrow-double/lean` | 13 originals + DoubleCliRules | borrow-index doubling arithmetic, word/packing/sign lemmas, strict partial lifting; rule-shaped theorems |
| `pool-math/proofs/utilization/lean` | 11 | utilization formula and its word-level helper lemmas |

## 4. Certora rules (by area) and Komet target selection

388 `#[rule]`s, each wired to one conf. Areas: fixed-point math
(`common/spec/math_rules.rs`), rates/curve/compound/index (`rates_rules.rs`,
`rate_index_accounting_rules.rs`), extremes and LP pricing
(`fp_extremes_rules.rs`, `lp_math_rules.rs`), value math and HF lemmas
(`value_math_rules.rs`, `controller/spec/hf_lemma_rules.rs`), stateful pool
operations (`pool/spec/*`), stateful controller (`controller/spec/*`) and the
price aggregator (`price-aggregator/spec/*`). No README records per-rule
verdicts; the `split_*` (math) and `additivity_*` (pool) rules have never run
through the prover, and `isqrt_is_the_integer_floor_of_the_root` is
unprovable in the prover's U256 model.

Komet targets chosen from them (function-level, no storage):

| Certora rule | Komet claim | Status |
|---|---|---|
| `borrow_index_identity_is_noop` | `test_borrow_index_identity` | see §1 |
| `borrow_index_cap_is_sticky` | `test_borrow_cap_sticky` | see §1 |
| `supply_index_zero_inputs_are_noop` | `test_supply_zero_inputs` (all i128) | see §1 |
| `compound_interest_identity_at_zero_delta` | `test_compound_zero_delta` (all i128) | see §1 |
| `scaled_price_rejects_negative_operands` | `test_wad_try_mul_negative` | see §1 |
| `rescale_upscale_lossless` | `test_from_asset_7_exact` (7 → 27 decimals) | see §1 |
| `update_borrow_index_monotonic/_capped`, `borrow_index_strictly_grows_below_cap` | `test_borrow_index_bounds`, `test_borrow_index_exact` | see §1 |
| `ray_div_floor_never_exceeds_half_up_widened`, ADR-0003 scaled conversions | `test_ray_div_floor/_ceil` | see §1 |
| `protocol_fee_shares_*` (bv, 1800 s on Certora) | `test_fee_shares` | see §1 |

Poor Komet fits (left to Certora/tests): symbolic compound-interest and
borrow-rate curves, LP/StableSwap pricing (U256, Newton loop), and every
stateful pool, controller and oracle rule. Claims whose production path runs
native i128 division with a symbolic operand (`__divti3`/`__udivti3` loops)
are tier C: feasible only with further word-level lemmas.

## 5. Server results

Filled in as runs complete (verdict, nodes, rewrite steps, wall time, Wasm
SHA-256, extra module).
