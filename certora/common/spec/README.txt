Common numeric model — Certora proof domain
============================================

Core invariant
--------------
Fixed-point values stay in their declared domain (BPS / WAD / RAY), rescale
correctly across token decimals, and use half-up multiply/divide unless a call
site explicitly floors or ceils. Rate and index math is monotone where
production requires it: the borrow rate as utilization rises, the borrow
index, and the supply index outside bad-debt paths. Utilization is zero in an
empty market.

Assumptions
-----------
- Inputs are within the domains exercised by production call sites.
- Certora builds use the common WASM harness in spec/harness.rs.
- Heavy controller paths are out of scope here; this layer proves library math.

Conf → spec map
---------------
math_rules.rs
  math.conf — ray/wad/bps identities, roundtrip bounds
  math-hard.conf — bps->wad floor chain as _native and _widened lemmas
    (heavy profile)
  math-sanity.conf — common_math_reachability
  fp-identities.conf — half-up, rescale and split multiply/divide identities
  fp-identities-bv.conf — half-up rounding direction and roundtrip error,
    bit-precise (heavy profile)
  fp-identities-reverts.conf — division by zero reverts (revert-shaped)
  fp-identities-reverts-sanity.conf — div_by_zero_sanity_fixture_completes

rates_rules.rs
  rates.conf — utilization and deposit-rate lemmas (_native / _widened splits)
  interest-curve.conf — borrow-rate curve, compounding and supplier-reward
    lemmas over a symbolic market
  rate-accounting.conf — supplier reward plus fee equals accrued interest
  rate-accounting-hard.conf — protocol fee-share bounds (heavy profile,
    bit-precise escalation)
  rate-indexes.conf — borrow/supply index caps and monotonicity
  compound-interest.conf — zero-time identity
  rates-sanity.conf — rates_reachability

rate_index_accounting_rules.rs
  rate-index-accounting.conf — kink rates, index updates and the interest
    split of one accrual
  index-projection.conf — market index: zero-time stability, view/accrue
    agreement, time monotonicity (core profile)

lp_math_rules.rs
  lp-math.conf — constant-product LP fair value (reaches isqrt)
  lp-math-isqrt.conf — isqrt floor property; not provable under the current
    U256 comparison model (heavy profile; see certora/README.md)
  lp-math-stable.conf — StableSwap LP fair value (Newton D solver; the one
    conf that runs with optimistic_loop, because the solver bound is 255)

fp_extremes_rules.rs
  fp-extremes.conf — fixed-point behaviour at the edges of its domain
  fp-extremes-sanity.conf — reachability of the edge fixtures

value_math_rules.rs
  value-math.conf — health-factor, seizure-split and protocol-fee lemmas
  value-math-sanity.conf — hf_lemmas_reachability

Lemma-before-main
-----------------
Run math.conf and rates.conf before controller confs that depend on the same
primitives. A rule whose nonlinear step is hard is split into a `_native` and a
`_widened` lemma, each with its branch condition assumed, rather than given a
larger budget. `_native` covers the `i128` fast path; `_widened` covers the
`I256` path.
