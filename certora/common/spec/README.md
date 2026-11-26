# Common math verification

These rules prove the library math the pool and controller use. They load no
contract state. The WASM harness is `harness.rs`.

**Invariant.** Fixed-point values stay in domain (BPS, WAD, RAY), rescale
correctly across decimals, and round half-up unless the call site floors or
ceils. The borrow rate rises with utilization. The borrow index never falls.
The supply index falls only on bad-debt paths. An empty market has zero
utilization.

**Assumption.** Inputs lie in production call-site ranges.

## Rule modules

| Module | Confs | Proves |
|---|---|---|
| `math_rules.rs` | `math`, `math-hard`\*, `math-sanity` | RAY, WAD, and BPS identities and roundtrip bounds. BPS-to-WAD floor chain |
| | `fp-identities`, `fp-identities-bv`\*, `fp-identities-reverts`, `-reverts-sanity` | Half-up, rescale, split multiply and divide identities. Bit-precise rounding direction. Division by zero reverts |
| `rates_rules.rs` | `rates`, `interest-curve`, `rate-accounting`, `rate-accounting-hard`\*, `rate-indexes`, `compound-interest`, `rates-sanity` | Utilization, deposit rate. Borrow curve, compounding. Reward + fee = accrued interest. Fee-share bounds. Index caps, monotonicity |
| `rate_index_accounting_rules.rs` | `rate-index-accounting`, `index-projection` | Kink rates, one accrual's index update and interest split. Zero-time stability, view and accrue agreement, time monotonicity |
| `lp_math_rules.rs` | `lp-math`, `lp-math-isqrt`\*, `lp-math-stable` | Constant-product and StableSwap LP fair value |
| `fp_extremes_rules.rs` | `fp-extremes`, `fp-extremes-sanity` | Fixed-point behavior at domain edges |
| `value_math_rules.rs` | `value-math`, `value-math-sanity` | Health-factor, seizure-split, and protocol-fee lemmas |

\* `heavy` profile.

`lp-math-stable` is the only conf with `optimistic_loop`. The Newton solver
for D loops up to 255 times. `lp-math-isqrt` cannot be proved
([Known prover limits](../../README.md#known-prover-limits)).

## Native and widened lemmas

When a nonlinear step is too hard, split the rule instead of raising the
budget. Use a native lemma for the `i128` fast path and a widened lemma for the
`I256` path. Each lemma assumes its branch condition (names end in _native and
_widened). Prove `math.conf` and
`rates.conf` first ([proof order](../../controller/spec/README.md#proof-order)).
