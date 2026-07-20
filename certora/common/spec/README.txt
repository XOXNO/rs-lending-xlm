Common numeric model — Certora proof domain
============================================

Core invariant
--------------
Fixed-point values stay in their declared domain (BPS / WAD / RAY), rescale
correctly across token decimals, and use half-up multiply/divide unless a call
site explicitly floors. Rate and index math is monotone where INVARIANTS.md
requires it (borrow index, supply index outside bad-debt paths, utilization at
empty markets).

Assumptions
-----------
- Inputs are within the domains exercised by production call sites.
- Certora builds use the common WASM harness in spec/harness.rs.
- Heavy controller paths are out of scope here; this layer proves library math.

Conf → spec map
---------------
math.conf
  spec/math_rules.rs
  Rules: ray/wad/bps identities, roundtrip bounds, common_math_reachability

math-hard.conf
  spec/math_rules.rs
  Rules: bps→wad floor chain (NIA-hard escalation pair for math.conf;
  runs in the heavy profile)

rates.conf
  spec/rates_rules.rs
  Rules: utilization, borrow/deposit rate caps, index monotonicity, interest
  split identity, rates_reachability

Lemma-before-main
-----------------
Run math.conf and rates.conf (basic sanity) before controller confs that depend
on the same primitives. See architecture/INVARIANTS.md sections 1.1–1.6.