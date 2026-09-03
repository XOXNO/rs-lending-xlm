Common numeric model — Certora proof domain
============================================

Core invariant
--------------
Fixed-point values stay in their declared domain (BPS / WAD / RAY), rescale
correctly across token decimals, and use half-up multiply/divide unless a call
site explicitly floors. Rate and index math is monotone where production
requires it (borrow index, supply index outside bad-debt paths, utilization at
empty markets).

Assumptions
-----------
- Inputs are within the domains exercised by production call sites.
- Certora builds use the common WASM harness in spec/harness.rs.
- Heavy controller paths are out of scope here; this layer proves library math.

Conf → spec map
---------------
math_rules.rs
  math.conf — ray/wad/bps identities, roundtrip bounds
  math-hard.conf — bps->wad floor chain, split into _native and _widened
    lemmas (NIA-hard escalation pair for math.conf; runs in the heavy profile)
  math-sanity.conf — common_math_reachability

rates_rules.rs
  rates.conf — utilization and deposit-rate lemmas (_native / _widened splits)
  rate-accounting.conf — supplier reward plus fee equals accrued interest
  rate-accounting-hard.conf — protocol fee-share bounds (heavy profile,
    bit-precise escalation)
  rate-indexes.conf — borrow/supply index caps and monotonicity
  compound-interest.conf — zero-time identity
  rates-sanity.conf — rates_reachability

rate_index_accounting_rules.rs
  rate-index-accounting.conf — index and interest allocation over one accrual

lp_math_rules.rs
  lp-math.conf — constant-product LP fair value (reaches isqrt)
  lp-math-stable.conf — StableSwap LP fair value (Newton D solver; the one
    conf that runs with optimistic_loop, because the solver bound is 255)

Lemma-before-main
-----------------
Run math.conf and rates.conf before controller confs that depend on the same
primitives. A rule whose nonlinear step is hard is split into a `_native` and a
`_widened` lemma, each with its branch condition assumed, rather than given a
larger budget.
