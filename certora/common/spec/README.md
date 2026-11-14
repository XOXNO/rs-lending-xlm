# Common math verification

These rules prove the library math the pool and controller use. They load no
contract state; the WASM harness is `harness.rs`.

**Invariant.** Fixed-point values stay in domain (BPS, WAD, RAY), rescale
correctly across decimals, and round half-up unless the call site floors or
ceils. The borrow rate rises with utilization. The borrow index never falls;
the supply index falls only on bad-debt paths. An empty market has zero
utilization.

**Assumption.** Inputs lie in production call-site ranges.

## Rule modules

| Module | Confs | Proves |
|---|---|---|
| `math_rules.rs` | `math`, `math-hard`\*, `math-sanity` | RAY, WAD, BPS identities and roundtrip bounds; BPS-to-WAD floor chain |
| | `fp-identities`, `fp-identities-bv`\*, `fp-identities-reverts`, `-reverts-sanity` | Half-up, rescale, split multiply/divide identities; bit-precise rounding direction; division by zero reverts |
| `rates_rules.rs` | `rates`, `interest-curve`, `rate-accounting`, `rate-accounting-hard`\*, `rate-indexes`, `compound-interest`, `rates-sanity` | Utilization, deposit rate; borrow curve, compounding; reward + fee = accrued interest; fee-share bounds; index caps, monotonicity |
| `rate_index_accounting_rules.rs` | `rate-index-accounting`, `index-projection` | Kink rates, one accrual's index update and interest split; zero-time stability, view/accrue agreement, time monotonicity |
| `lp_math_rules.rs` | `lp-math`, `lp-math-isqrt`\*, `lp-math-stable` | Constant-product and StableSwap LP fair value |
| `fp_extremes_rules.rs` | `fp-extremes`, `fp-extremes-sanity` | Fixed-point behavior at domain edges |
| `value_math_rules.rs` | `value-math`, `value-math-sanity` | Health-factor, seizure-split and protocol-fee lemmas |

\* `heavy` profile.

`lp-math-stable` is the only conf with `optimistic_loop`: the Newton solver
for D loops up to 255 times. `lp-math-isqrt` cannot be proved
([Known prover limits](../../README.md#known-prover-limits)).

## Native and widened lemmas

When a nonlinear step is too hard, split the rule instead of raising the
budget: a native lemma for the `i128` fast path and a widened lemma for the
`I256` path, each assuming its branch condition (names end in _native and
_widened). Prove `math.conf` and
`rates.conf` first ([proof order](../../controller/spec/README.md#proof-order)).
