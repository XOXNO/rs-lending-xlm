# Common math verification

These rules prove the library math that the pool and controller use. They load
no contract state; the WASM harness is `harness.rs`.

**Invariant.** Fixed-point values stay in their domain (BPS, WAD, RAY), rescale
correctly across token decimals, and round half-up unless the call site floors
or ceils. The borrow rate rises with utilization. The borrow index never falls,
and the supply index falls only on bad-debt paths. Utilization is zero in an
empty market.

**Assumption.** Inputs are in the ranges that production call sites pass.

## Rule modules

| Module | Confs | Proves |
|---|---|---|
| `math_rules.rs` | `math`, `math-hard`\*, `math-sanity` | RAY, WAD and BPS identities and roundtrip bounds; the BPS-to-WAD floor chain |
| | `fp-identities`, `fp-identities-bv`\*, `fp-identities-reverts`, `-reverts-sanity` | Half-up, rescale and split multiply/divide identities; rounding direction at bit precision; division by zero reverts |
| `rates_rules.rs` | `rates`, `interest-curve`, `rate-accounting`, `rate-accounting-hard`\*, `rate-indexes`, `compound-interest`, `rates-sanity` | Utilization and deposit rate; borrow curve and compounding; reward plus fee equals accrued interest; fee-share bounds; index caps and monotonicity |
| `rate_index_accounting_rules.rs` | `rate-index-accounting`, `index-projection` | Kink rates, index update and interest split of one accrual; zero-time stability, view/accrue agreement, time monotonicity |
| `lp_math_rules.rs` | `lp-math`, `lp-math-isqrt`\*, `lp-math-stable` | Constant-product and StableSwap LP fair value |
| `fp_extremes_rules.rs` | `fp-extremes`, `fp-extremes-sanity` | Fixed-point behavior at the edges of its domain |
| `value_math_rules.rs` | `value-math`, `value-math-sanity` | Health-factor, seizure-split and protocol-fee lemmas |

\* `heavy` profile.

`lp-math-stable` is the only conf with `optimistic_loop`: the Newton solver
for D loops up to 255 times. `lp-math-isqrt` cannot be proved with the current
U256 comparison model; see [certora/README.md](../../README.md#known-prover-limits).

## Native and widened lemmas

When a nonlinear step is too hard, split the rule in two instead of raising the
budget: a native lemma for the `i128` fast path and a widened lemma for the
`I256` path, each with its branch condition assumed. The rule names end in
_native and _widened.

Prove `math.conf` and `rates.conf` before the controller confs that use the
same functions.
