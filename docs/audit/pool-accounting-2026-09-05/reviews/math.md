# Independent fixed-point audit

Revision: `99613335b410f70ff42dd99d13ff530f6adaee67`.
Scope: every line and function in `common/src/math/{mod.rs,fp.rs,fp_core.rs}`, with minimal pool/rates/validation caller context. Read-only production review. No network, deployed-state checks, or edits to repository files.

## Result

No pool-reachable security finding confirmed in the fixed-point implementation. Integer widening, signed floor/ceil, nonnegative half-up, saturation, scale conversion, and basis-point application agree with the stated arithmetic domains. Two raw signed edge underflows are reproducible library robustness defects, but no production path supplying those negative values was identified. Do not promote these to public vulnerabilities without a reachable caller.

## Exact coverage

| Source | Coverage | SHA-256 |
|---|---|---|
| `common/src/math/mod.rs` | Lines 1–7, both module exports | `707ae8fdbd3d2d8c3d5c0f4ca86db7ac98ba1e647163ff9c340b4dd19d69d5f2` |
| `common/src/math/fp.rs` | Lines 1–342, 47 functions/methods and all three types/constants | `bf002a66a981d03f0947654e76711bb55cce001e15a241d7bf2dfdf1a6445a1f` |
| `common/src/math/fp_core.rs` | Lines 1–316, 19 functions | `677f26fe2892853e70c5d6411d89ac6efed85e5df8e1020ce9232b44f66167c0` |

`fp_core` inventory: `to_i256_operands`, `require_nonzero_divisor`, `quotient_is_negative`, `div_floor_i128`, `div_ceil_i128`, `div_floor_i256`, `div_ceil_i256`, `quotient_is_nonnegative`, `mul_div_half_up`, `try_mul_div_half_up`, `mul_div_floor`, `mul_div_ceil`, `mul_div_floor_saturating`, `rescale`, `rescale_half_up`, `rescale_floor`, `rescale_ceil`, `div_by_int_half_up`, `to_i128`.

`fp` helpers: `checked_add_raw`, `checked_sub_nonneg`.

`Ray` methods: `from`, `raw`, `mul`, `div`, `div_floor`, `div_ceil`, `div_by_int`, `mul_floor`, `mul_ceil`, `mul_ratio_ceil`, `to_wad`, `to_wad_floor`, `to_wad_ceil`, `to_asset`, `to_asset_floor`, `to_asset_ceil`, `from_fraction`, `from_asset`, `checked_sub`, `checked_add`.

`Wad` methods: `from`, `raw`, `mul`, `try_mul`, `div`, `div_floor`, `div_floor_saturating`, `mul_floor`, `mul_ceil`, `from_token`, `to_token`, `to_token_floor`, `to_ray`, `checked_add`, `checked_sub`.

`Bps` methods: `from`, `raw`, `to_wad`, `apply_to`, `flash_loan_fee_on`, `apply_to_wad`, `apply_to_wad_floor`, `apply_to_ray`, `checked_add`, `checked_sub`.

Also read both existing math test files, all 1,554 lines. Caller context inspected in `common/src/rates/{curve,compound,scaling,index}.rs`, `common/src/constants/{shared,pool,mod}.rs`, `common/src/validation.rs`, relevant `common/src/types/pool.rs` parameter checks, pool `ops/{mod,market,supply,repay,withdraw,strategy,flash}.rs`, pool `cache/{scale,shares}.rs`, and controller health-factor division. This context was used to establish domains, not to claim exhaustive audits of these other files.

MCP graph discovery used `search_graph`, `query_graph`, and `trace_path`. The graph omitted several method callers (e.g. raw rescale and integer division traced only to `fp.rs`); authoritative source searches filled those gaps. No index mutation requested.

## Arithmetic invariants and reasoning

1. **Widening is sufficient.** Any two signed `i128` operands multiply to magnitude at most `2^254`, inside signed `I256`. A valid nonnegative half-up bias adds at most `2^126-1`; the biased product remains below `2^255`. The widened division cannot encounter `I256::MIN / -1`. Native products are checked before use.
2. **Signed floor/ceil match exact rationals.** Native checked division precedes `%`, avoiding `i128::MIN % -1`. When a remainder exists, floor decrements only a negative rational; ceil increments only a positive rational. Widened remainder testing uses only zero/nonzero, so Euclidean remainder with a negative divisor is safe. The fast and widened paths were compared against independent Python unbounded integers.
3. **Half-up is correct in its explicit domain.** `floor((x*y + floor(d/2))/d)` is nearest rounding for integer `x*y`, with positive half ties upward, including odd `d`. `try_mul_div_half_up` rejects `x<0`, `y<0`, `d<=0`, and out-of-range output. The panicking wrapper enforces the release domain through this function; debug assertions do not supply the only protection.
4. **Zero division is fail-closed.** Panicking raw division helpers check zero before arithmetic and use `GenericError::DivisionByZero` (#55). The fallible half-up helper returns `None`. Debug zero checks precede its domain assertion. Existing typed-error tests pass.
5. **Saturation uses the rational sign.** `mul_div_floor_saturating` clamps an overflowing exact floor to `i128::MIN` or `MAX`, using the XOR of all operand signs. Zero numerators remain zero. No silent wrapping occurs.
6. **Saturation matches caller caps.** `update_supply_index` clamps the saturated result again to `MAX_SUPPLY_INDEX_RAY` (`common/src/rates/index.rs:41–44`); `protocol_fee_shares` clamps to remaining supply headroom (`:95–98`). For nonnegative exact `q` and bound `b<=i128::MAX`, `min(saturate(q),b)==min(q,b)`. `calculate_scaled_cap` similarly treats an unrepresentably large cap/share ratio as nonbinding, while the prior asset-to-Ray rescale remains checked. Health-factor saturation represents an extremely healthy value; zero debt bypasses division (`contracts/controller/src/risk/totals.rs:216–219`).
7. **Scale factors and units are consistent.** Ray = `10^27`, Wad = `10^18`, Bps = `10^4`. Multiplication divides once by the fixed-point scale; division multiplies once by the same scale; `mul_ratio_ceil` consumes an unscaled ratio. Bps-to-Wad is exactly multiplication by `10^14`, so `apply_to_wad` has no intermediate quantization. Asset conversions from protocol decimals `<=18` to Ray are exact multiplication by `10^(27-decimals)`.
8. **Conservative conversion composition is preserved.** Supply deposit shares use floor; debt origination shares use ceil; partial supply burns use ceil; partial debt burns use floor. Conservative unscaling composes floor/floor or ceil/ceil (`common/src/rates/scaling.rs:35–92`). Treasury partial claims use a ceiling proportion and cannot burn more than held for `0<amount<=treasury_actual` (`contracts/pool/src/cache/shares.rs:54–74`).
9. **Rescaling bounds are explicit.** Equal scales return the operand. Upscaling rejects an overflowing factor/value. Downscaling by `10^k` for `k>=39` returns zero for truncation/half-up, and one only for positive ceil: `10^39/2` exceeds every signed `i128` magnitude. `rescale_floor` intentionally truncates negative inputs toward zero; its documentation says so. It is not the signed mathematical floor routine.
10. **Newtypes do not enforce positivity.** `Ray::from`, `Wad::from`, and `Bps::from` deliberately accept raw signed integers. `checked_add` checks overflow but not signs. `checked_sub` rejects negative operands and negative results. Pool amounts are validated by `ops::load_leg` (`ops/mod.rs:43–46`), borrow/flash positive checks, and strategy/fee nonnegative checks. Market rate verification establishes `0<=base<=slopes<=max<=2*RAY`, valid nonzero utilization ranges, reserve `<BPS`, and bounded flash fees. Stored/controller-provided positions remain trusted inputs; arbitrary forged negative positions are not an attacker reachability proof.
11. **Integer division callers are bounded.** The only production `Ray::div_by_int` callers found are annual-to-millisecond rate conversion (`common/src/rates/curve.rs:65–67`) and Taylor terms (`compound.rs:47–49`). Divisors are fixed positive constants. Maximum configured annual rate is `2*RAY`; valid compounding chunks bound the exponent close to two and highest power close to `256*RAY`, far below `i128::MAX`. Thus the edge bias overflows below are outside these caller ranges.
12. **Minimum flash fee matches execution gates.** A positive Bps rate yields at least one unit, including the helper's zero-amount input. Actual flash loans reject zero principal (`ops/flash.rs:76–77`). Strategy fee calculation rejects fee greater than principal (`ops/strategy.rs:94–100`), so the zero-amount/minimum-fee edge cannot create a free or negative repayment.

## Concrete candidates and false-positive disposition

### FP-EDGE-01: signed downscale bias can underflow

- Location: `common/src/math/fp_core.rs:251`.
- Actual call: `rescale_half_up(&env, i128::MIN, 1, 0)`.
- Mathematical result: `-17014118346046923173168730371588410573`, which fits `i128`.
- Observed result with release-like overflow settings: `attempt to subtract with overflow`.
- Cause: negative branch subtracts `factor/2` before dividing. For `a < i128::MIN + factor/2`, the intermediate underflows despite a representable quotient. This is a real mismatch for the documented signed raw helper.
- Reachability disposition: **informational library robustness issue, no confirmed pool vulnerability**. Production consumers reached through Ray/Wad conversion are nonnegative. Negative `i128::MIN` is unsupported by the nonnegative wrapper domain and no external path was identified that could set it.
- Minimal future fix if the signed contract is retained: quotient/remainder rounding for the negative branch, avoiding a biased numerator. No implementation change made in this audit.

### FP-EDGE-02: signed integer-division bias can underflow

- Location: `common/src/math/fp_core.rs:303`.
- Actual call: `div_by_int_half_up(&env, i128::MIN, 2)`.
- Mathematical result: `-85070591730234615865843651857942052864`, exactly representable.
- Observed result: `attempt to subtract with overflow`.
- Cause: unchecked `a - b/2` before division. The generic signed helper and tests support negative `a`, but not the extreme end correctly.
- Reachability disposition: **informational library robustness issue, no confirmed pool vulnerability**. Both production callers use nonnegative bounded rates/powers and fixed positive divisors. Negative divisors are also only debug-asserted; they remain outside both documented/intended and production caller domains.

### Rejected: positive division bias overflow

`div_by_int_half_up(&env, i128::MAX, 2)` panics with typed `MathOverflow` (#33) although the quotient fits. This fail-closed restriction is expressly documented and covered by an existing test. Same bounded callers exclude it. Not an independent security finding.

### Rejected: extreme decimals / zero upscale

`rescale(0,0,39)` would reject an overflowing factor although zero times that factor is zero. The behavior is documented and protocol decimal bounds do not reach it. Downscales with huge decimal differences were tested through `u32::MAX`; they do not trap or wrap.

### Rejected: unchecked constructors or saturating caps alone

Constructor sign freedom is explicit, and caller-domain violations alone are not exploit proofs. Saturation is mathematically compatible with the current bounded/nonbinding consumers described above. No loss-of-value path was established from either property.

## Verification and retained evidence

Directory: `/private/tmp/astra-fixed-point-probe/`.

- `probe.rs`: runnable Rust source importing the exact production math/errors/constants files by path.
- `generate.py`: seeded Python standard-library generator, unbounded-integer expected values.
- `mul-div.tsv`: 20,840 reference vectors, including i128 extrema, zero numerators, signs, negative divisors, full-width random values, odd denominators, widened products, and saturation.
- `run.py`: rebuilds existing math tests offline, generates vectors, compiles actual source with overflow checks enabled and debug assertions disabled, then runs the probe. Accepts another checkout root; rewrites only the generated source path references.
- `commands.json`, `common-math-tests.log`, `generate.log`, `compile.log`, `probe.log`: exact commands and complete output.

Reproduce:

```sh
python3 /private/tmp/astra-fixed-point-probe/run.py /Users/mihaieremia/GitHub/rs-lending-xlm
```

Results:

- Existing suite: **149 passed**, 0 failed, 238 filtered, `RUSTC_WRAPPER= cargo test -p common math:: --offline`.
- Actual-source probe: **72,378 mul/div assertions** against the independent vectors.
- **684** exact conversion assertions across decimals `0..=18`, including representable ceilings.
- **10,001** exact Bps-to-Wad ratios, covering every value `0..=10000`.
- **729** nonnegative/extreme-factor rescale assertions.
- Three edge panics explicitly reproduced: two signed intermediate underflows plus the documented positive typed overflow.

Limitations: native Rust + real Soroban SDK host arithmetic, not full compiled-Wasm contract execution or live deployment proof. Boundary/random checks supplement reasoning; they do not exhaust every possible tuple. Caller review was intentionally limited to establishing math domains, and did not independently re-audit all controller authorization or stored-state invariants.

Memory used only for audit-scope/reachability guidance: `MEMORY.md:268–280`, historical rollout `019f787d-27a5-79a0-9c4d-2b50cc1cb704`. All arithmetic results and code references above were verified live in this checkout.
