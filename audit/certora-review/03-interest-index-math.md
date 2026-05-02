# Domain 3 — Interest, Index, Math

**Phase:** 1 (formal-verification rule review)
**Files in scope:**
- `controller/certora/spec/interest_rules.rs` (471 lines, 14 rules + 1 sanity)
- `controller/certora/spec/index_rules.rs` (144 lines, 5 rules + 1 sanity)
- `controller/certora/spec/math_rules.rs` (392 lines, 12 rules + sanity duplicates)
- `controller/certora/spec/summaries/mod.rs` (`update_asset_index_summary`, lines 88-101)

**Production reference:**
- `common/src/rates.rs` (`calculate_borrow_rate`, `calculate_deposit_rate`,
  `compound_interest`, `update_borrow_index`, `update_supply_index`,
  `calculate_supplier_rewards`, `simulate_update_indexes`)
- `common/src/fp.rs`, `common/src/fp_core.rs`
  (`mul_div_half_up`, `mul_div_half_up_signed`, `rescale_half_up`,
  `div_by_int_half_up`)
- `pool/src/interest.rs` (`global_sync`, `apply_bad_debt_to_supply_index`,
  `add_protocol_revenue_ray`)
- `common/src/constants.rs`: `RAY = 10^27`, `WAD = 10^18`,
  `BPS = 10_000`, `MILLISECONDS_PER_YEAR = 31_556_926_000`,
  `SUPPLY_INDEX_FLOOR_RAW = WAD = 10^18`, `MAX_BORROW_RATE_RAY = 2 * RAY`

**Totals:** broken=4 weak=11 nit=4 sound=8 missing=8

Severity legend
- **broken**: rule asserts a wrong property, hides a real bug, or is logically
  unsound (passing the rule provides no security signal, or worse, false
  comfort).
- **weak**: rule is sound but its preconditions over-constrain the search
  space, miss a relevant region of the input domain, or restate the
  implementation rather than an invariant.
- **nit**: cosmetic / minor coverage gap.
- **sound**: rule is well-formed.

---

## interest_rules.rs

### `interest_rules.rs::nondet_valid_params` (helper, lines 30-70)

**Severity:** weak
**Rubric items failed:** [2, 5]
**Why:** The helper allows `max_borrow_rate_ray <= RAY * 10`
(`interest_rules.rs:49`), `slope1/2/3 <= RAY * 10` (`:54-56`), and
`base_borrow_rate_ray <= RAY * 10` (`:53`). Production caps
`max_borrow_rate_ray` at `MAX_BORROW_RATE_RAY = 2 * RAY`
(`common/src/constants.rs:42`) and validates this in
`controller::validation::validate_interest_rate_model` and
`pool::Pool::update_params`. Allowing 5× the production cap exercises a
regime the production code is supposed to make unreachable, which is fine
for catching bugs in the rate function itself, but the helper also fails
to enforce two production-validated invariants:

1. `slope2_ray >= slope1_ray` and `slope3_ray >= slope2_ray` (kinked-curve
   monotonicity assumed by Aave-style models). Without this, a bug like
   "slope3 silently lower than slope1" cannot be caught here -- it can in
   fact pass `borrow_rate_monotonic` because each segment is independently
   monotone in its own region.
2. `base + slope1 + slope2 + slope3 >= 0` overflow check inside region 3.
   Production lets `base + slope1 + slope2 + contribution` be added in
   `Ray` (panicking via `Ray::Add`) -- the helper's `<= RAY * 10` cap on
   each lets the i128 sum reach `4 * RAY * 10 = 40 RAY` plus contribution,
   still well inside i128 but past the production cap.

The helper also forgets `optimal_utilization_ray <= RAY` (it asserts
`< RAY` on line 48, fine) and never cross-validates that
`reserve_factor_bps < BPS` is the same as the production validator
(`(0..BPS).contains` on line 50 -- correct). No correctness bug, but
under-constrained relative to reality.

**Patch (suggested):**
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
     cvlr_assume!((0..BPS).contains(&reserve_factor_bps));

-    // Ensure base + slopes do not overflow i128 before capping
-    cvlr_assume!(base_borrow_rate_ray <= RAY * 10);
-    cvlr_assume!(slope1_ray <= RAY * 10);
-    cvlr_assume!(slope2_ray <= RAY * 10);
-    cvlr_assume!(slope3_ray <= RAY * 10);
+    // Match the production caps validated by
+    // `validation::validate_interest_rate_model` so the rules exercise
+    // the same regime as the deployed contract.
+    cvlr_assume!(base_borrow_rate_ray <= MAX_BORROW_RATE_RAY);
+    cvlr_assume!(slope1_ray <= MAX_BORROW_RATE_RAY);
+    cvlr_assume!(slope2_ray <= MAX_BORROW_RATE_RAY);
+    cvlr_assume!(slope3_ray <= MAX_BORROW_RATE_RAY);
+    // Kinked-curve assumption: slopes are non-decreasing across regions.
+    cvlr_assume!(slope2_ray >= slope1_ray);
+    cvlr_assume!(slope3_ray >= slope2_ray);
+    cvlr_assume!(max_borrow_rate_ray <= MAX_BORROW_RATE_RAY);
```

Without the slope monotonicity assumption, a separate explicit rule
`borrow_rate_slopes_non_decreasing` should test that production rejects
configurations with `slope3 < slope1`.

---

### `interest_rules.rs::borrow_rate_zero_utilization` (lines 79-94)

**Severity:** weak
**Rubric items failed:** [3, 5]
**Why:** The expected formula on line 86 hand-rolls an `if base > max
{ max } else { base }` cap, then divides by `MILLISECONDS_PER_YEAR`. This
is a literal copy of the production conditional in
`common/src/rates.rs:36-41`. The rule asserts the implementation matches
itself rather than an intrinsic property -- a *tautology trap* (rubric
item 6). What the rule should additionally check is the underlying
property: `rate_zero_util * MILLISECONDS_PER_YEAR <= base_borrow_rate +
1` and `<= max_borrow_rate + 1`. The current single-equality form will
silently keep passing if production introduces a sign flip in the cap
that also flips the rule.

A separate concern: the rule does not catch the bug "base_borrow_rate is
silently scaled twice" -- a regression that doubles `base` would still
pass because the rule recomputes base the same wrong way.

**Patch (suggested):**
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
 fn borrow_rate_zero_utilization(e: Env) {
     let params = nondet_valid_params(&e);
     let rate = calculate_borrow_rate(&e, Ray::ZERO, &params);
-    let annual = if params.base_borrow_rate_ray > params.max_borrow_rate_ray {
-        params.max_borrow_rate_ray
-    } else {
-        params.base_borrow_rate_ray
-    };
-    let expected = div_by_int_half_up(annual, MILLISECONDS_PER_YEAR as i128);
-    cvlr_assert!(rate.raw() == expected);
+    // Property: at zero util the rate is no greater than min(base, max) /
+    // MS_PER_YEAR (+rounding) and never above the per-ms cap.
+    let cap_per_ms = div_by_int_half_up(
+        params.max_borrow_rate_ray, MILLISECONDS_PER_YEAR as i128);
+    let base_per_ms = div_by_int_half_up(
+        params.base_borrow_rate_ray, MILLISECONDS_PER_YEAR as i128);
+    cvlr_assert!(rate.raw() <= cap_per_ms + 1);
+    cvlr_assert!(rate.raw() <= base_per_ms + 1);
+    cvlr_assert!(rate.raw() >= 0);
```

---

### `interest_rules.rs::borrow_rate_monotonic` (lines 102-117)

**Severity:** sound
**Why:** Asserts `rate(util_a) <= rate(util_b)` for `util_a < util_b`,
both in `[0, RAY]`. This catches:
- A sign flip on any slope contribution.
- A swapped ordering of regions (e.g. region 1 formula used past `mid`).
- A cap applied below the cap value (would non-monotonically clip).

One tightening: with the recommended `slope_i+1 >= slope_i`
precondition, this is the *strongest* sanity property of the curve.
Without it, monotonicity holds piecewise but a config with `slope3 < 0`
or `slope2 < slope1` could violate global monotonicity, and the rule
*should* fail in that case -- so leaving the assumption out is actually
correct here. The helper's `slope*_ray >= 0` (lines 44-46) is the only
constraint needed.

The rule **catches** (named bug): a rate model that goes negative
(`slope_X` with sign flip; e.g. computing `excess - slope2` instead of
`+ slope2`). It would also catch the bug of returning the
*pre-capped* rate when the cap fires below current annual_rate.

---

### `interest_rules.rs::borrow_rate_capped` (lines 125-140)

**Severity:** sound (but weak in coverage)
**Rubric items failed:** [7]
**Why:** Correctly bounds `rate <= cap + 1` (allowing one half-up tick)
and `rate >= 0`. **Catches**: cap-disabled regression, negative-rate
regression, integer-overflow on `base + slope1 + slope2 + contribution`
that wraps past `i128::MAX`.

Coverage gap: does not test the *combined* upper bound `base + slope1 +
slope2 + slope3 / MS_PER_YEAR + 1` for the un-capped path when
`max_borrow_rate` is huge. Add a companion that asserts
`rate_per_ms <= (base + s1 + s2 + s3) / MS_PER_YEAR + 1` whenever the
sum is below `max_borrow_rate`.

---

### `interest_rules.rs::borrow_rate_continuity_at_mid` (lines 148-169)
### `interest_rules.rs::borrow_rate_continuity_at_optimal` (lines 178-199)

**Severity:** weak
**Rubric items failed:** [3, 5]
**Why:** Both rules assume `tolerance == 1` per-ms unit
(`borrow_rate_continuity_at_mid:168`, `:198`). The actual production
formula per
`common/src/rates.rs:20-34` uses two cascading `mul_div_half_up`
operations (`utilization.mul(env, s1).div(env, mid)`), each carrying up
to ±1 RAY-unit error before the final `div_by_int_half_up` by
`MS_PER_YEAR`. Per-ms tolerance of 1 i128 is therefore an over-tight
bound that may yield false counter-examples for slopes near the
boundary. The "tolerance: 1 unit of the per-ms rate" comment is too
optimistic.

Additionally, the rules step *down* by 1 RAY-unit (`mid_utilization_ray
- 1`), but production uses `Ray::from_raw(util)` -- so `mid - 1` raw is
a 1e-27 increment, not "the boundary minus delta". Continuity at
boundaries is fundamentally a property of `lim_{u -> mid^-} == lim_{u
-> mid^+}`. With integer arithmetic and i128 raw values, the only
meaningful test is "rate(mid) == base + slope1 within tolerance" --
exactly what the rule should compute, not the one-step-below trick.

This **catches** a discontinuous coefficient (e.g., the slope2
contribution incorrectly normalising by `optimal` instead of
`optimal - mid`), which is a real concern. But it is one-sided in two
ways: (a) tolerance is too tight (may produce false negatives); (b) it
does not assert what the boundary value *should* be (the named property
"rate at mid == base + slope1 / MS_PER_YEAR").

**Patch (suggested):** strengthen with a direct equality at the
boundary:
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
 fn borrow_rate_continuity_at_mid(e: Env) {
     let params = nondet_valid_params(&e);
     cvlr_assume!(params.mid_utilization_ray >= 2);
+    // Skip when capped -- the cap masks the boundary value.
+    cvlr_assume!((params.base_borrow_rate_ray + params.slope1_ray)
+        < params.max_borrow_rate_ray);

     let rate_below = calculate_borrow_rate(
         &e, Ray::from_raw(params.mid_utilization_ray - 1), &params);
     let rate_at = calculate_borrow_rate(
         &e, Ray::from_raw(params.mid_utilization_ray), &params);
+    let rate_at_per_ms = div_by_int_half_up(
+        params.base_borrow_rate_ray + params.slope1_ray,
+        MILLISECONDS_PER_YEAR as i128);

     let diff = if rate_at >= rate_below {
         rate_at.raw() - rate_below.raw()
     } else {
         rate_below.raw() - rate_at.raw()
     };
-    cvlr_assert!(diff <= 1);
+    // Adjacent points differ by at most 2 per-ms units (two cascading
+    // half-up rounds + base/year).
+    cvlr_assert!(diff <= 2);
+    // Boundary value matches base + slope1, within rounding tolerance.
+    let boundary_diff = if rate_at.raw() >= rate_at_per_ms {
+        rate_at.raw() - rate_at_per_ms
+    } else {
+        rate_at_per_ms - rate_at.raw()
+    };
+    cvlr_assert!(boundary_diff <= 2);
 }
```

The same fix applies to `borrow_rate_continuity_at_optimal`.

---

### `interest_rules.rs::deposit_rate_zero_when_no_utilization` (lines 207-223)

**Severity:** sound
**Why:** Asserts `calculate_deposit_rate(0, *, *) == 0`. Production
short-circuits at `common/src/rates.rs:52-54`. **Catches**: a regression
that drops the zero short-circuit and produces non-zero rates from
`utilization * borrow_rate * (1 - rf)` rounding noise.

Note: the rule also leaves `borrow_rate` and `reserve_factor_bps` free.
Good -- the property holds for all values in those domains.

---

### `interest_rules.rs::deposit_rate_less_than_borrow` (lines 232-252)

**Severity:** weak
**Rubric items failed:** [1, 3]
**Why:** Asserts `deposit_rate <= util * borrow_rate + 1`
(`interest_rules.rs:251`). The named invariant from the comment is
`deposit_rate = util * borrow_rate * (1 - rf/BPS)`, which is **strictly
tighter** than the asserted bound. The asserted form does not pin the
reserve-factor effect at all -- a bug that ignored
`reserve_factor_bps` (always paying 100% to suppliers) would still pass
because `util * borrow_rate * 1 <= util * borrow_rate + 1`.

**The reserve-factor underflow path is the named bug worth catching**:
if `reserve_factor_bps` is allowed to exceed `BPS` upstream, then `BPS -
reserve_factor_bps` goes negative and the deposit rate flips sign. The
production code defends against this at `common/src/rates.rs:59-61`. A
rule should test that defense: assume `reserve_factor_bps == BPS` (or
greater), then assert `calculate_deposit_rate(...) == Ray::ZERO`.

Additionally the rule never tests `deposit_rate >= 0` -- a sign-flip
regression in the `(BPS - rf) * borrow_rate` branch would slip through.

**Patch (suggested):**
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
 fn deposit_rate_less_than_borrow(e: Env) {
     ...
-    let upper_bound = mul_div_half_up(&e, utilization, borrow_rate, RAY);
-    cvlr_assert!(deposit_rate.raw() <= upper_bound + 1);
+    // Tight upper bound: util * borrow_rate * (BPS - rf) / BPS
+    let weighted = mul_div_half_up(&e, utilization, borrow_rate, RAY);
+    let expected = mul_div_half_up(&e, weighted, BPS - reserve_factor_bps, BPS);
+    let diff = if deposit_rate.raw() >= expected {
+        deposit_rate.raw() - expected
+    } else {
+        expected - deposit_rate.raw()
+    };
+    cvlr_assert!(diff <= 2);  // two cascading half-up rounds
+    cvlr_assert!(deposit_rate.raw() >= 0);
 }
```

Add a separate rule:
```rust
#[rule]
fn deposit_rate_zero_when_rf_at_or_above_bps(e: Env) {
    let util: i128 = cvlr::nondet::nondet();
    let br: i128 = cvlr::nondet::nondet();
    let rf: i128 = cvlr::nondet::nondet();
    cvlr_assume!((1..=RAY).contains(&util));
    cvlr_assume!((0..=RAY).contains(&br));
    cvlr_assume!(rf >= BPS);  // out-of-range
    let r = calculate_deposit_rate(&e, Ray::from_raw(util),
                                    Ray::from_raw(br), rf);
    cvlr_assert!(r == Ray::ZERO);
}
```

---

### `interest_rules.rs::compound_interest_identity` (lines 260-268)

**Severity:** sound
**Why:** `compound_interest(_, _, 0) == Ray::ONE`. Production
short-circuits at `common/src/rates.rs:71-73`. **Catches**: a regression
that drops the early-return guard and produces `Ray::ONE +
mul_div_half_up(0, 0, RAY) * sum_of_terms` ≈ `Ray::ONE`, but with
rounding noise from `(0 + RAY/2) / RAY = 0` -- which would still pass.
Better: the rule still acts as a sanity check.

---

### `interest_rules.rs::compound_interest_monotonic_in_time` (lines 277-292)
### `interest_rules.rs::compound_interest_monotonic_in_rate` (lines 301-316)

**Severity:** weak
**Rubric items failed:** [2, 5]
**Why:** Both rules cap `rate <= div_by_int_half_up(RAY,
MILLISECONDS_PER_YEAR)` (= ~3.17e16), which corresponds to **1 RAY/year =
100% APR**. Production guarantees `<0.01% accuracy for x <= 2 RAY`
(`pool/src/interest.rs:14-17`), and `MAX_BORROW_RATE_RAY = 2 * RAY`
(`common/src/constants.rs:42`) — i.e., the production envelope is
`x <= 2 RAY`, twice the regime tested here.

The rule does **not** test the production limit. Combined with `t <=
MILLISECONDS_PER_YEAR`, the highest x exercised is `RAY` (e^1 ≈ 2.7),
not `2 RAY` (e^2 ≈ 7.4). A bug whose monotonicity violation only
manifests at `x > 1 RAY` (where late Taylor terms dominate) would slip
through.

Tighter bug catch: if the 8-term Taylor expansion underestimates beyond
its envelope, monotonicity in rate could break (a higher rate applied
to the same time span gives a lower factor due to truncation error).
This is a real correctness concern at `x close to 2 RAY` because
truncation drops `term9 = x^9/362880` which is on the order of
`(2*RAY)^9/362880 ≈ 1.4e240 / 3.6e5` ≈ `3.9e234 / RAY^8 ≈ 3.9e10` at the
boundary -- non-trivial.

**Patch (suggested):** raise the rate cap to match the production cap:
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
 fn compound_interest_monotonic_in_time(e: Env) {
     ...
-    cvlr_assume!(rate <= div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128));
+    // Match the production envelope: x = rate*t <= 2 RAY across one year.
+    cvlr_assume!(rate <= div_by_int_half_up(2 * RAY, MILLISECONDS_PER_YEAR as i128));
     cvlr_assume!(t1 < t2);
     cvlr_assume!(t2 <= MILLISECONDS_PER_YEAR);
```

The same change applies to `compound_interest_monotonic_in_rate`.

The named bug each catches:
- `..._in_time`: a Taylor truncation that flips sign of an even-power
  term (e.g. `term4 = -x^4/24` instead of `+x^4/24`). Even at small x
  this would violate strict monotonicity for sequential `t1, t2`.
- `..._in_rate`: a denominator swap (e.g. `term3 = x^3/2` instead of
  `x^3/6`) that compounds with rate but breaks rate-ordering.

---

### `interest_rules.rs::compound_interest_ge_simple` (lines 331-351)

**Severity:** weak
**Rubric items failed:** [2, 3]
**Why:** Asserts `factor >= 1 + x - 2` for `x = rate*t`. The `-2`
tolerance is generous; production rounding loses at most ±2 across the
Taylor series for `x <= RAY`, so the bound should hold. **However**:

1. The rule only tests `x <= RAY` (rate cap = `RAY/MS_PER_YEAR`,
   `t <= MS_PER_YEAR`), not the production envelope `x <= 2 RAY`. At
   `x close to 2 RAY`, `e^x ~ 7.4` and `1+x = 3` so the bound `e^x >=
   1+x` is comfortably wide -- but the named bug here is "the 8-term
   sum underestimates `1+x` for tiny x". Tiny x is well-tested; large x
   is not, and that's where Taylor-truncation bias matters.
2. The "Taylor truncation can fall slightly below" comment (line 348)
   is misleading: with all positive terms, an 8-term truncation of e^x
   (x>0) is **strictly less** than `e^x` but **always >=** `1+x` when
   `x <= 2 RAY` because `1+x` is a 2-term truncation. The `-2`
   tolerance must come from rounding alone, not from Taylor structure.
3. The rule does not check the upper envelope: e^x is bounded above by
   the next-term Taylor approximation. A rule like
   `factor <= simple + (term_max + rounding_eps)` would catch
   *over-estimation* (e.g. a sign error producing `factor = e^(2x)`).

**Patch (suggested):** add the upper envelope companion and raise the
rate cap.
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
 fn compound_interest_ge_simple(e: Env) {
-    let max_rate = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
+    let max_rate = div_by_int_half_up(2 * RAY, MILLISECONDS_PER_YEAR as i128);
     ...
+    // Upper envelope: e^x < 1 + x + x^2 (for 0 < x < ~1)
+    let x_sq = x.checked_mul(x).expect("safe under bounds");
+    cvlr_assert!(factor.raw() <= simple + x_sq / RAY + 2);
 }
```

The named bug to catch with the upper envelope: "compound_interest
returns `interest_factor` of `e^(2x)` (a doubled-rate regression that
would credit suppliers with twice the intended interest)".

---

### `interest_rules.rs::supplier_rewards_conservation` (lines 362-408)

**Severity:** sound (but with a precondition gap)
**Rubric items failed:** [2]
**Why:** Asserts `supplier_rewards + protocol_fee ≈ accrued_interest`
within ±1, and `protocol_fee ≈ accrued * rf / BPS` within ±1.
Reconstructs `accrued_interest = new_debt - old_debt` (where
`new_debt = mul_div_half_up(borrowed, new_index, RAY)` and likewise for
old).

**Catches**: any change to the production split (e.g.
`supplier_rewards = accrued - 2 * protocol_fee`, missing fee, fee
rounded against the protocol).

**Gap (rubric 2)**: the rule allows `borrowed > 0` (line 370) and
`new_borrow_index <= RAY * 10` (`:375`), giving accrued_interest up to
~`RAY * 1_000_000 * (10 RAY - RAY) / RAY ~ 9_000_000 RAY`. Production
runs with a bounded `borrowed` (capped via supply-of-debt). The rule's
tolerance of ±1 must be re-examined: with i128 multiplication of
ray-scaled `borrowed` and `index`, two `mul_div_half_up` rounds give
±2 each, total ±4. **The asserted ±1 may produce false positives at
extreme values.**

**Patch (suggested):**
```diff
--- before
+++ after
@@ controller/certora/spec/interest_rules.rs @@
-    cvlr_assert!(diff <= 1);
+    cvlr_assert!(diff <= 4);  // four cascading half-up rounds
     ...
-    cvlr_assert!(fee_diff <= 1);
+    cvlr_assert!(fee_diff <= 4);
```

Sharper variant: assert exact equality on the *non-rounded* path by
constraining inputs:
```rust
cvlr_assume!(borrowed % RAY == 0);  // align to exact integer
```
Then the asserted ±1 is achievable.

---

### `interest_rules.rs::update_borrow_index_monotonic` (lines 416-428)

**Severity:** sound (but a potential overflow gap)
**Rubric items failed:** [2]
**Why:** Asserts `new_index >= old_index` when `interest_factor >= RAY`.
**Catches**: a regression in `update_borrow_index` that divides instead
of multiplies, or that subtracts the factor.

**Gap**: no upper bound on `interest_factor`. Production guarantees
`interest_factor` is the result of `compound_interest`, which is bounded
by `e^2 RAY ~ 7.4 RAY`. The rule lets `interest_factor` reach `i128::MAX`
(unbounded `nondet`), potentially overflowing inside
`mul_div_half_up(old_index, factor, RAY)` via the I256 path. The
production caller would never trigger that. Add:
```rust
cvlr_assume!(interest_factor <= 8 * RAY);  // per the e^2 envelope
```

---

### `interest_rules.rs::update_supply_index_monotonic` (lines 437-457)

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts `new_index >= old_index` when `rewards_increase >= 0`.
But the production code at `common/src/rates.rs:114-116` short-circuits
to `old_index` when *either* `supplied == 0` *or* `rewards_increase ==
0`. The rule's named property is "monotone in non-bad-debt path", which
is correct -- but a stronger version would also assert
`rewards_increase == 0` => `new_index == old_index` (exact equality, not
just `>=`).

The current `>=` form misses the bug "supply index always grows by `rewards
* old_index / supplied` even when no rewards accrued" (a missing
short-circuit). Such a bug would still satisfy `new >= old`.

**Patch (suggested):** split into two rules:
```rust
#[rule]
fn update_supply_index_idempotent_when_no_rewards(e: Env) {
    // ... same setup
    let new_index = update_supply_index(&e, ..., Ray::ZERO);
    cvlr_assert!(new_index.raw() == old_index);
}

#[rule]
fn update_supply_index_increases_with_rewards(e: Env) {
    // ... rewards > 0
    cvlr_assume!(rewards_increase > 0);
    cvlr_assume!(supplied > 0);
    cvlr_assert!(new_index.raw() > old_index);  // strict
}
```

---

### `interest_rules.rs::interest_rules_sanity` (lines 463-471)

**Severity:** sound -- standard `cvlr_satisfy` reachability check.

---

## index_rules.rs

### `index_rules.rs::supply_index_above_floor` (lines 25-35)

**Severity:** broken
**Rubric items failed:** [1, 3, 4]
**Why:** Reads the index via
`crate::storage::market_index::get_market_index(&e, &asset)` (line 32),
which delegates to a **cross-contract** `LiquidityPoolClient::get_sync_data()`
call (`controller/src/storage/certora.rs:107-109`). Cross-contract
calls are pure havoc to the Certora prover (the summaries module
itself notes this:
`controller/certora/spec/summaries/mod.rs:12-13`). With no summary
applied to `get_sync_data`, the prover models
`cache_entry.supply_index_ray` as an **arbitrary i128** -- including
zero or negative.

The rule has no `cvlr_assume!(cache_entry.supply_index_ray >=
SUPPLY_INDEX_FLOOR_RAW)` to encode the production invariant, so the
prover is free to falsify the assertion immediately. Either:
1. The rule passes only because the prover misbehaves on the cross-shard
   call (false comfort), or
2. The rule fails because the index is havoced to 0 (true counterexample
   the rule cannot meaningfully fix without a summary).

The named property "supply index >= floor" is a **storage invariant**
that lives entirely inside the pool contract, enforced by
`pool/src/interest.rs:apply_bad_debt_to_supply_index:158-162`. To
verify it, the rule must either:
- Run inside the pool contract and read its instance storage directly
  (not through a cross-contract proxy), OR
- Apply a summary to `LiquidityPoolClient::get_sync_data` whose
  post-condition encodes the floor.

The same critique applies to **`borrow_index_gte_ray`** (lines 43-52),
**`borrow_index_monotonic_after_accrual`** (lines 60-76), and
**`supply_index_monotonic_after_accrual`** (lines 84-97) -- all rely on
unsummarised cross-contract reads.

**The summary `update_asset_index_summary` exists**
(`controller/certora/spec/summaries/mod.rs:88-101`), but it's
applied to `crate::oracle::update_asset_index`, **not** to
`get_sync_data`. So `index_rules` reads the unsummarised path.

**Patch (suggested):** add a `get_sync_data_summary` and apply it via
`#[summarized!]` to `LiquidityPoolClient::get_sync_data`:
```rust
// summaries/mod.rs
pub fn get_sync_data_summary(_env: &Env, _asset: &Address) -> PoolSyncData {
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    let supplied_ray: i128 = nondet();
    let borrowed_ray: i128 = nondet();
    let last_timestamp: u64 = nondet();
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);
    cvlr_assume!(supplied_ray >= 0);
    cvlr_assume!(borrowed_ray >= 0);
    cvlr_assume!(borrowed_ray <= supplied_ray);  // utilization invariant
    PoolSyncData { /* ... */ }
}
```

Until this is added, **all four index rules read havoced data and
assert against it -- they are unsound**. Rule 1 and 2 (the floor/RAY
checks) will be falsified. Rules 3 and 4 (monotonicity through
`supply_single`) are even harder: `supply_single` calls into
the controller, which calls `cached_pool_sync_data`, which is
**also unsummarised**, so the before/after pair are independently
havoced -- the assertion `borrow_after >= borrow_before` over two
unrelated nondet values fails.

---

### `index_rules.rs::indexes_unchanged_when_no_time_elapsed` (lines 105-134)

**Severity:** sound
**Why:** Tests the math primitives `compound_interest`,
`update_borrow_index`, `update_supply_index` directly (no cross-contract
call). Asserts:
- `compound_interest(_, _, 0) == RAY`,
- `update_borrow_index(old, RAY) == old`,
- `update_supply_index(_, old, 0) == old`.

**Catches**: a regression that drops the `delta_ms == 0` short-circuit
in `compound_interest`, the `(1 + epsilon) * old` rounding bias when
`epsilon = 0`, or the `rewards == 0` short-circuit at
`common/src/rates.rs:114`.

Note: the rule re-derives the property from the math; it does not depend
on cross-contract reads, so it's a useful direct check.

---

### `index_rules.rs::index_sanity` (lines 140-144)

**Severity:** broken (same root cause as Rule 1 of this module)
**Rubric items failed:** [1, 4]
**Why:** Uses `cvlr_satisfy!(idx.supply > 0 && idx.borrow > 0)` on a
havoced cross-contract read. Trivially satisfiable by the prover
choosing positive values. Provides no meaningful reachability signal.

---

## math_rules.rs

### `math_rules.rs::mul_half_up_commutative` (lines 23-38)

**Severity:** sound
**Why:** Asserts `mul_div_half_up(a, b, p) == mul_div_half_up(b, a, p)`.
Production multiplies via I256 which is commutative; the half-up bias
is symmetric in `(x, y)`. **Catches**: a regression that uses asymmetric
rounding (e.g., always rounding the second operand up), which would
produce non-commutative results.

---

### `math_rules.rs::mul_half_up_zero` (lines 44-61)

**Severity:** sound
**Why:** Standard zero-element check. **Catches**: a regression where
the half-up adjust dominates `(0 * b + p/2) / p > 0`. The comment on
line 54 ("p/2 / p = 0 for any p >= 2") is correct because `p/2 < p`
forces integer division to zero.

---

### `math_rules.rs::mul_half_up_identity` (lines 67-91)

**Severity:** sound
**Why:** Asserts `mul_div_half_up(a, RAY, RAY) == a` exactly for `0 <=
a <= RAY * 1000`. Algebraically: `(a*RAY + RAY/2) / RAY = a + (RAY/2)/RAY
= a + 0 = a` (integer division). **Catches**: a regression that swaps
operand and divisor, or that adds the half *after* division.

---

### `math_rules.rs::div_half_up_inverse` (lines 97-111)

**Severity:** sound (but tolerance possibly tight)
**Why:** Round-trip `mul_div_half_up(mul_div_half_up(a, b, RAY), RAY, b)
~ a` within ±2. **Catches**: a sign error in the half-up bias (which
would shift the recovered value by ±1 each round, totaling >2).

The bound ±2 may be exceeded for very-low `b`. Specifically, when
`b = 1` and `a ~ RAY*100`, the intermediate product is `~RAY*100`,
recovered = `(~RAY*100 * RAY + 1/2) / 1 = ~RAY^2 * 100`, which then
overflows the i128 conversion in `mul_div_half_up` (the I256 fits, but
`to_i128()` panics). **The rule will fail not from a rounding violation
but from a panic.**

**Patch (suggested):** add a lower bound on `b`:
```diff
-    cvlr_assume!(b > 0 && b <= RAY * 100);
+    cvlr_assume!(b >= RAY / 1_000 && b <= RAY * 100);
```

This keeps `recovered ~ a` finite. Without the lower bound on `b`, the
rule can fail spuriously on degenerate inputs.

---

### `math_rules.rs::div_half_up_zero_numerator` (lines 130-142)

**Severity:** sound
**Why:** `mul_div_half_up(0, RAY, b) == 0` for `b > 0`. Trivial but
catches a "divide-by-half-of-divisor" bug.

---

### `math_rules.rs::mul_half_up_rounding_direction` (lines 161-176)

**Severity:** sound
**Why:** Asserts `result * WAD >= a*b - (WAD - 1)`, i.e., `result` is
not below the mathematical floor of `a*b/WAD`. With `a, b <= 1e14`, the
linearised form avoids the I256 timeout that the previous version
hit (per the comment on lines 150-160).

**Catches**: a rounding regression that systematically rounds *down*
beyond the half-up bias.

Coverage gap: this rule asserts only the **lower** bound. The upper
bound (`result * WAD <= a*b + WAD`) is missing. Without it, a
regression that rounds up by *too much* (e.g., adds `WAD` instead of
`WAD/2`) slips through. The companion sanity rule
`mul_half_up_rounding_direction_sanity` is just `result >= 0`, which
does not add coverage.

**Patch (suggested):**
```diff
@@ controller/certora/spec/math_rules.rs @@
     cvlr_assert!(result * WAD >= a * b - (WAD - 1));
+    // Half-up rounding never rounds more than (WAD/2) above the true
+    // value. result*WAD <= a*b + WAD is a safe linear envelope.
+    cvlr_assert!(result * WAD <= a * b + WAD);
```

---

### `math_rules.rs::div_half_up_rounding_direction` (lines 205-223)

**Severity:** sound
**Why:** Two-sided linear envelope `floor <= result <= floor + 1`,
expressed as `result * b >= a*WAD - (b-1)` and `result * b <= a*WAD + b`.
Correctly captures the half-up window without identifying which branch
fires (the comment justifies this on lines 196-204).

**Catches**: any rounding deviation outside the half-up window of
±1 ulp.

---

### `math_rules.rs::rescale_upscale_lossless` (lines 229-244)

**Severity:** sound (limited domain)
**Rubric items failed:** [7]
**Why:** Tests **only** `from=7, to=18` (hardcoded on lines 232-233).
Production calls `rescale_half_up` from many decimal pairs (asset
decimals 0..27, RAY=27, WAD=18, BPS=4). A regression specific to other
decimal deltas (e.g. `from=0, to=27`, where the factor is `10^27` and
sits at the limit of i128 representation) would not be caught.

**Patch (suggested):** parametrize the rule:
```diff
-    let from: u32 = 7;
-    let to: u32 = 18;
+    let from: u32 = cvlr::nondet::nondet();
+    let to: u32 = cvlr::nondet::nondet();
+    cvlr_assume!(from <= 27 && to <= 27 && from <= to);
+    let diff = to - from;
+    cvlr_assume!(diff <= 18);  // 10^18 fits comfortably; 10^27 does not for x near i128::MAX
```

---

### `math_rules.rs::rescale_roundtrip` (lines 259-277)

**Severity:** sound
**Why:** Round-trip 7 -> 18 -> 7 is exact because upscale is lossless
multiplication by `10^11` and downscale's half-up adjustment yields
`(x*10^11 + 5e10) / 10^11 = x` for non-negative `x` (since the residue
`5e10 < 10^11`). The assertion `recovered == x` is correct.

Coverage gap: same as `rescale_upscale_lossless` -- only one decimal
pair tested.

---

### `math_rules.rs::signed_mul_away_from_zero` (lines 307-322)

**Severity:** broken
**Rubric items failed:** [3]
**Why:** This is the most subtle bug in the whole math file. The rule
asserts:
```rust
cvlr_assert!(result * RAY <= a * b);          // line 320
cvlr_assert!(result * RAY >= a * b - RAY);    // line 321
```

Walk through with concrete values. Let `a = -34, b = RAY/10, d = RAY`:
- `a * b = -3.4e27` (= `-3.4 RAY`)
- Production (`fp_core.rs:43-48`): `product < 0`, so
  `rounded = product - half = -3.4 RAY - 0.5 RAY = -3.9 RAY`
- `rounded / d` truncates toward zero in Rust: `-3.9 RAY / RAY = -3`
  (since `|-3.9 RAY| / RAY = 3.9`, integer-divided to `3`, sign flipped
  back to `-3`).
- Half-up away-from-zero rounding of `-3.4` correctly yields `-3`
  (because `|−0.4| < 0.5`, we round toward zero in this case, which is
  away-from-zero on a number less negative than `-0.5` -- both
  conventions coincide).

Now check the asserted bound:
- `result * RAY = -3 * RAY = -3 RAY = -3e27`
- `a * b = -3.4 RAY = -3.4e27`
- Is `-3e27 <= -3.4e27`? **No.** `-3e27 > -3.4e27` on the number line.

The rule's first inequality (`result * RAY <= a * b`) is **wrong-way
around** for any `a*b` whose absolute value has a fractional part below
0.5. The half-up away-from-zero scheme returns a **less-negative**
result than `floor(a*b/d)`, not a more-negative one. The correct
envelope is the symmetric one already used by
`mul_half_up_rounding_direction` and `div_half_up_rounding_direction`:
```rust
// |result * d - a*b| < d   (half-up bounded by ±d)
cvlr_assert!(result * RAY >= a * b - RAY);
cvlr_assert!(result * RAY <= a * b + RAY);
```

The named bug this rule is *intended* to catch -- "half-up signed
rounds toward zero instead of away from zero" -- requires a different
formulation. For exact halves (e.g. `a*b = -3.5 RAY`), away-from-zero
gives `-4`, toward-zero gives `-3`. The rule should assert that
**when `a*b` mod `RAY` is exactly `RAY/2`** (or, more practically,
when `a*b` lands at a half-tick), the result is the away-from-zero
value, not the toward-zero value:
```rust
// Construct an exact-half input
cvlr_assume!(a * b == -3 * RAY - RAY / 2);  // -3.5 RAY exactly
let result = mul_div_half_up_signed(&e, a, b, RAY);
cvlr_assert!(result == -4);  // away from zero
```

**This rule as written will report violations on legitimate
half-up-away-from-zero behaviour and is therefore broken.**

**Patch (suggested):**
```diff
@@ controller/certora/spec/math_rules.rs @@
 fn signed_mul_away_from_zero(e: Env) {
     let a: i128 = cvlr::nondet::nondet();
     let b: i128 = cvlr::nondet::nondet();
     cvlr_assume!((-100_000_000_000_000..0).contains(&a));
     cvlr_assume!(b > 0 && b <= 100_000_000_000_000);
     let result = mul_div_half_up_signed(&e, a, b, RAY);
-    cvlr_assert!(result * RAY <= a * b);
-    cvlr_assert!(result * RAY >= a * b - RAY);
+    // Linear envelope on half-up rounding: |result*RAY - a*b| <= RAY
+    cvlr_assert!(result * RAY >= a * b - RAY);
+    cvlr_assert!(result * RAY <= a * b + RAY);
+    // Sign preserved: result is non-positive for negative a*b.
+    cvlr_assert!(result <= 0);
 }
```

Add a separate **direction** rule that exercises the away-from-zero
property exactly:
```rust
#[rule]
fn signed_mul_exact_half_rounds_away_from_zero(e: Env) {
    // Use a representable exact half: a = -1, b = RAY/2, d = RAY.
    // a*b = -RAY/2, half-up away-from-zero -> -1 (more negative).
    let result = mul_div_half_up_signed(&e, -1, RAY / 2, RAY);
    cvlr_assert!(result == -1);

    // a = -3, b = RAY/2, d = RAY -> -1.5 RAY -> -2.
    let result = mul_div_half_up_signed(&e, -3, RAY / 2, RAY);
    cvlr_assert!(result == -2);
}
```

---

### `math_rules.rs::i256_no_overflow` (lines 343-360)

**Severity:** sound
**Why:** Asserts `mul_div_half_up(a, b, RAY)` does not panic for `a, b
<= 10 * RAY`. The intermediate `a*b` reaches `100 RAY^2 = 10^56`, well
inside I256. The result `100 * RAY = 10^29`, comfortably inside i128
(`max ~ 1.7e38`).

**Catches**: a regression that drops the I256 promotion and uses i128
multiplication directly, which would overflow at `a*b > i128::MAX`.

The upper bound `result <= 100 * RAY + 1` is correct (`100 RAY + half-up
tick`).

---

### `math_rules.rs::div_by_zero_sanity` (lines 382-392)

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Asserts that `mul_div_half_up(a, RAY, 0)` is unreachable. The
rule's logic (assume zero divisor, then assert false) tests that the
prover's panic-modeling correctly identifies division-by-zero as a
panic. **However**: production reaches the divisor via `d256 =
I256::from_i128(env, 0)`, then computes `half = d256.div(I256::from(2))`
**before** `product.div(d256)`. The first division (`0 / 2`) is **valid
and equals zero**. The second division (`product / 0`) panics. The rule
correctly tests the second-division panic.

**But**: Soroban's I256 division behaviour on zero divisor is not
universally documented to panic. If the prover models I256 division as
a partial function returning a fresh nondet on zero divisor (a sound
abstraction in the absence of explicit panic modeling), the rule
silently passes despite no runtime panic in the harness. To make the
rule robust, **also assert** that the production wrapper
(`fp_core::to_i128`) panics on `MathOverflow`, which is a cleaner
panic-pathway test. That test belongs in a separate rule that exercises
the i128 conversion ceiling.

---

### `math_rules.rs::*_sanity` (lines 84-91, 113-124, 178-188, 246-253, 279-287, 324-334, 362-372)

**Severity:** nit
**Why:** All seven `cvlr_satisfy!`-based sanity rules duplicate the
preconditions of their corresponding assertion rules and check
reachability of a trivial postcondition. They add solver cycles without
extra coverage. Acceptable for "this rule is reachable", but
`mul_half_up_identity_sanity` (line 84-91) is identical to
`mul_half_up_identity` modulo the `cvlr_satisfy!` instead of `cvlr_assert!`
-- pure duplication.

Consolidate into a single `#[cfg(any(test, feature = "certora_sanity"))]`
sanity module, or drop the duplicates.

---

## Summary integrity (`update_asset_index_summary`, lines 88-101)

The summary returns a `MarketIndex` with these post-conditions:
1. `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW` (= `WAD = 10^18`).
   ✓ Matches `pool/src/interest.rs:158-162` floor.
2. `borrow_index_ray >= RAY`.
   ✓ Matches `update_borrow_index` monotonicity from initial `RAY`.
3. `borrow_index_ray >= supply_index_ray`.
   **Questionable.** This is *empirically* true in steady state with
   `reserve_factor_bps > 0`, because supply rewards = `(1 - rf) *`
   borrow accrual. But:
   - At pool genesis both indexes equal `RAY`; the inequality holds
     by equality.
   - With `reserve_factor_bps == 0`, supply_index can grow as fast as
     borrow_index when utilization is 100%. Only the ratio of borrowed
     to supplied caps it. With `borrowed << supplied`, supply grows
     much slower, so inequality holds; with `borrowed close to
     supplied` and `rf == 0`, supply grows close to borrow.
   - **However**, the inequality *can* break across multiple accrual
     cycles if a stuck-pool scenario causes the supply_index to be
     refreshed but the borrow_index to not be -- which the production
     `simulate_update_indexes` updates atomically (both or neither),
     so this case is unreachable.
   - Conclusion: the assumption is **likely sound** but **not
     documented in production code**. If a future change reorders the
     accrual sequence, this assumption becomes silently unsound.

   **Recommendation:** drop or move to a documented invariant. The
   strictly safer post-condition is just `borrow_index >= RAY` and
   `supply >= floor` independently. Drop line 96.

4. **Missing post-condition**: `last_timestamp <= cache.current_timestamp_ms`.
   The summary docstring (`mod.rs:87`) **claims** this post-condition,
   but the summary returns a `MarketIndex` -- which has no
   `last_timestamp` field (`common/src/types.rs:319-322`). The
   docstring is therefore misleading; the post-condition cannot be
   asserted because the return type does not carry the timestamp.

   In production, `simulate_update_indexes` does *not* return a
   timestamp -- the caller must read it separately. The summary is
   sound on this front, but the doc claim should be removed.

5. **Subtle silent unsoundness**: production
   `simulate_update_indexes` returns the `(supply, borrow)` pair *after*
   accrual from `last_timestamp` to `current_timestamp`. The summary
   havocs both indexes independently. This loses the relationship that
   `(supply_after - supply_before)` and `(borrow_after - borrow_before)`
   are linked by `(1 - rf) * borrowed * (borrow_after - borrow_before)
   / (supplied * supply_before) ≈ supply_after / supply_before - 1`.
   No rule depends on this linkage today, but rules that try to verify
   "supply_index gain proportional to borrow_index gain" (a missing
   invariant) would need a tighter summary.

---

## Missing rules (coverage gaps the file should add)

The rubric calls out several invariants. Cross-referencing what's
covered vs. what's missing:

| # | Invariant                                                       | Status | Severity |
|---|-----------------------------------------------------------------|--------|----------|
| 1 | Borrow index monotonically non-decreasing                        | covered (interest_rules R13) | -- |
| 2 | Supply index >= SUPPLY_INDEX_FLOOR_RAW                           | covered locally (index_rules R5) but storage view is broken | high (because R1 is broken) |
| 3 | Borrow rate monotone in utilization within each region           | partly (interest_rules R2 is global, not per-region) | medium |
| 4 | Total debt = sum(scaled × index)                                 | **missing** | high |
| 5 | No interest accrual when no debt exists (utilization == 0)       | partly (R6 zero-deposit-rate, but no rule for borrow_index unchanged when utilization == 0) | medium |
| 6 | Compound interest within Taylor envelope (no overflow)           | partly (R11 lower envelope only, no upper) | medium |
| 7 | Half-up rounding round-trip identity (within ulp)                | covered (math R4) | -- |
| 8 | Wad × Wad / Wad equality (associativity proxy)                   | **missing** | low |
| 9 | Ray to asset-decimals truncation never returns more than scaled  | **missing** | medium |
| 10| Conservation: scaled_supplied + scaled_borrowed >= 0             | **missing** | medium |
| 11| No underflow on subtraction in scaled-amount math                | **missing** | medium |
| 12| Negative reserve_factor / out-of-range produces zero deposit     | **missing** (the defense at `rates.rs:59-61` is untested) | high |
| 13| `apply_bad_debt_to_supply_index` clamps at floor                 | **missing** (would catch a regression that drops the floor) | critical |
| 14| `add_protocol_revenue_ray` skip-when-floor invariant             | **missing** | medium |
| 15| Compound interest upper envelope `factor <= 1 + x + x^2 + ...`   | **missing** | medium |
| 16| Compound interest at MAX_BORROW_RATE_RAY * MS_PER_YEAR converges | **missing** (the rule caps at `RAY/MS_PER_YEAR`, not the production cap) | medium |

### Suggested new rules

```rust
// Invariant 4: total_debt across positions == sum(scaled * index).
// Sketch: nondet two scaled amounts s1, s2 and an index i;
// assert mul(s1, i) + mul(s2, i) == mul(s1 + s2, i) within rounding.
#[rule]
fn debt_sum_equals_sum_of_debts(e: Env) { /* ... */ }

// Invariant 5: when utilization == 0, borrow_index unchanged after
// global_sync.
#[rule]
fn borrow_index_unchanged_at_zero_utilization(e: Env) {
    let old: i128 = cvlr::nondet::nondet();
    cvlr_assume!(old >= RAY);
    let factor = compound_interest(&e, Ray::ZERO, MILLISECONDS_PER_YEAR);
    let new = update_borrow_index(&e, Ray::from_raw(old), factor);
    cvlr_assert!(new.raw() == old);
}

// Invariant 9: Ray::to_asset truncation upper bound.
#[rule]
fn ray_to_asset_truncates_below_input(e: Env) {
    let scaled: i128 = cvlr::nondet::nondet();
    let decimals: u32 = cvlr::nondet::nondet();
    cvlr_assume!(scaled >= 0 && scaled <= RAY * 1_000_000);
    cvlr_assume!(decimals <= 27);
    let asset = Ray::from_raw(scaled).to_asset(decimals);
    // asset_amount <= scaled / 10^(27 - decimals) + 1
    if decimals < 27 {
        let factor = 10i128.pow(27 - decimals);
        cvlr_assert!(asset * factor <= scaled + factor);
        cvlr_assert!(asset * factor >= scaled - factor);
    } else {
        cvlr_assert!(asset == scaled);
    }
}

// Invariant 12: out-of-range reserve factor zeroes deposit rate.
#[rule]
fn deposit_rate_zero_when_rf_overflows(e: Env) {
    let util: i128 = cvlr::nondet::nondet();
    let br: i128 = cvlr::nondet::nondet();
    let rf: i128 = cvlr::nondet::nondet();
    cvlr_assume!((1..=RAY).contains(&util));
    cvlr_assume!((0..=RAY).contains(&br));
    cvlr_assume!(rf < 0 || rf >= BPS);
    let r = calculate_deposit_rate(&e, Ray::from_raw(util),
                                    Ray::from_raw(br), rf);
    cvlr_assert!(r == Ray::ZERO);
}

// Invariant 13: apply_bad_debt clamps at floor.
// (Lives in pool spec, not controller -- noted here for tracking.)

// Invariant 14: add_protocol_revenue_ray short-circuits at/below floor.
// (Lives in pool spec.)

// Invariant 15: Compound interest upper envelope.
#[rule]
fn compound_interest_upper_envelope(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let t: u64 = cvlr::nondet::nondet();
    let max_rate = div_by_int_half_up(2 * RAY, MILLISECONDS_PER_YEAR as i128);
    cvlr_assume!(rate >= 0 && rate <= max_rate);
    cvlr_assume!(t > 0 && t <= MILLISECONDS_PER_YEAR);
    let factor = compound_interest(&e, Ray::from_raw(rate), t);
    let x = rate * (t as i128);
    // e^x < 1 + x + x^2 for 0 < x < 1 (one-term-ahead bound).
    let x_sq = x.checked_mul(x).expect("safe under bounds") / RAY;
    cvlr_assert!(factor.raw() <= RAY + x + x_sq + 4);
}
```

---

## Action items (severity-tagged)

### Critical (block any production-claim of "Certora-verified")

- **C1.** Fix `signed_mul_away_from_zero` (`math_rules.rs:307-322`) --
  the asserted inequality is wrong-way-around and falsifies on
  legitimate input. Replace with the symmetric `|result*d - a*b| <= d`
  envelope and add an exact-half direction rule.
- **C2.** Add a `get_sync_data_summary` (or an inline summary on
  `cached_pool_sync_data`) so that `index_rules` R1, R2, R3, R4, R6
  read indexes through a constraint that encodes the production
  invariants. **Without this, the four index rules are unsound** -- they
  read havoced cross-contract data and assert against it.
- **C3.** Add a rule for `apply_bad_debt_to_supply_index` floor clamp
  (`pool/src/interest.rs:158-162`). This is the most security-critical
  invariant in the whole module (a regression here drains supplier
  funds). Today **no rule tests it**.

### High

- **H1.** Tighten `nondet_valid_params` (`interest_rules.rs:30-70`) to
  match the production validator: cap rates at `MAX_BORROW_RATE_RAY`,
  enforce slope monotonicity (`s2 >= s1`, `s3 >= s2`).
- **H2.** Replace `borrow_rate_zero_utilization`'s tautology
  re-implementation with property assertions (rate <= cap_per_ms + 1
  and rate <= base_per_ms + 1).
- **H3.** Replace `deposit_rate_less_than_borrow`'s loose upper bound
  with a tight `util * borrow_rate * (BPS - rf) / BPS` equality
  (within rounding tolerance), and add a rule for the
  reserve-factor-out-of-range zero-deposit defense
  (`common/src/rates.rs:59-61`).
- **H4.** Drop `update_asset_index_summary`'s `borrow_index >=
  supply_index` assumption (`summaries/mod.rs:96`) -- it is empirically
  true today but not documented as a production invariant.
- **H5.** Remove the misleading doc claim about `last_timestamp` from
  `update_asset_index_summary` (`summaries/mod.rs:87`) -- the return
  type carries no timestamp.
- **H6.** Add an upper-envelope rule for `compound_interest`
  (catches doubled-rate regression).

### Medium

- **M1.** Raise the rate cap in `compound_interest_monotonic_in_*` and
  `compound_interest_ge_simple` from `RAY/year` to
  `2*RAY/year` to match the production envelope.
- **M2.** Tighten `supplier_rewards_conservation` tolerance from ±1 to
  ±4 (four cascading half-up rounds).
- **M3.** Add upper bound on `interest_factor` in
  `update_borrow_index_monotonic` (`<= 8 * RAY`, the e^2 ceiling).
- **M4.** Strengthen `update_supply_index_monotonic` by splitting into
  idempotent-when-no-rewards and strict-increase-with-rewards rules.
- **M5.** Replace `borrow_rate_continuity_at_*`'s `<=1` tolerance with
  `<=2`, and add a boundary-value equality assertion.
- **M6.** Parametrize `rescale_upscale_lossless` and `rescale_roundtrip`
  over decimal pairs.
- **M7.** Add upper bound on rounding direction
  (`mul_half_up_rounding_direction`, line 175 -- only lower bound
  asserted).
- **M8.** Add a lower bound on `b` in `div_half_up_inverse` (line 104
  is `b > 0`, but small `b` causes the recovered value to overflow
  i128 inside the second `mul_div_half_up` -- false counterexamples).

### Low

- **L1.** Drop the seven `*_sanity` companion rules or move them under
  a feature flag -- they add solver cycles without extra coverage.
- **L2.** Add a rule for `Ray::to_asset` truncation upper bound (named
  invariant: "scaled amount converted to asset decimals never exceeds
  the original scaled amount divided by the decimal factor").
- **L3.** Add a rule for `update_supply_index` zero-supplied
  short-circuit (`common/src/rates.rs:114`).
- **L4.** Document in `summaries/mod.rs` that
  `update_asset_index_summary` is **not** the only un-summarised
  cross-contract path -- `cached_pool_sync_data` and the storage
  proxy `get_market_index` are also unsummarised, and rules that read
  through them are unsound.

---

## Bugs each rule's failure would catch (named regression model)

| Rule                                          | Named bug it catches                                                |
|-----------------------------------------------|---------------------------------------------------------------------|
| `borrow_rate_zero_utilization`                | base-rate sign flip; cap-direction bug                              |
| `borrow_rate_monotonic`                       | slope sign flip; region misalignment; pre-cap leak                  |
| `borrow_rate_capped`                          | cap removal; negative-rate; intermediate i128 overflow              |
| `borrow_rate_continuity_at_mid/optimal`       | discontinuous coefficient; wrong normalisation in region 2/3       |
| `deposit_rate_zero_when_no_utilization`       | dropped `util == 0` short-circuit                                    |
| `deposit_rate_less_than_borrow` (current)     | catastrophic over-payment to suppliers (loose bound; weak)           |
| `deposit_rate_less_than_borrow` (suggested)   | reserve-factor inversion; missing `(BPS - rf)` factor                |
| `compound_interest_identity`                  | dropped `delta_ms == 0` short-circuit                                |
| `compound_interest_monotonic_in_time`         | sign flip in even Taylor term                                        |
| `compound_interest_monotonic_in_rate`         | denominator swap (e.g. `term3/2` instead of `/6`)                    |
| `compound_interest_ge_simple`                 | linear-only approximation regression                                 |
| `supplier_rewards_conservation`               | fee/reward split not summing to accrued                              |
| `update_borrow_index_monotonic`               | divide-instead-of-multiply; subtraction regression                   |
| `update_supply_index_monotonic`               | (weak) doesn't catch always-grow-with-zero-reward bug                |
| `supply_index_above_floor`                    | (broken) cannot catch anything; reads havoc                          |
| `borrow_index_gte_ray`                        | (broken) cannot catch anything; reads havoc                          |
| `borrow_index_monotonic_after_accrual`        | (broken) before/after both havoced; no inter-call linkage            |
| `supply_index_monotonic_after_accrual`        | (broken) same as above                                               |
| `indexes_unchanged_when_no_time_elapsed`      | dropped early-return in `compound_interest`                          |
| `mul_half_up_commutative`                     | asymmetric rounding bias                                             |
| `mul_half_up_zero`                            | half-bias dominating zero product                                    |
| `mul_half_up_identity`                        | swapped operand/divisor; misplaced half-add                          |
| `div_half_up_inverse`                         | half-bias sign error (cumulative ±2 on round-trip)                   |
| `div_half_up_zero_numerator`                  | divide-by-half-of-divisor                                            |
| `mul_half_up_rounding_direction`              | systematic rounding-down regression (lower side only)                |
| `div_half_up_rounding_direction`              | rounding outside the half-up window                                  |
| `rescale_upscale_lossless`                    | upscale lossiness; off-by-one factor                                 |
| `rescale_roundtrip`                           | downscale half-up bias error (only `7 -> 18 -> 7`)                   |
| `signed_mul_away_from_zero` (current)         | (broken) asserts wrong inequality direction; falsified on valid math |
| `signed_mul_away_from_zero` (suggested)       | toward-zero-rounding regression on negative products                 |
| `i256_no_overflow`                            | dropped I256 promotion (i128 overflow)                               |
| `div_by_zero_sanity`                          | divide-by-zero unreachable claim (depends on prover panic modeling)  |

---

## Provenance

All rule line numbers are from the SHA at the time of review (working
tree at `/Users/mihaieremia/GitHub/rs-lending-xlm`, branch `main`).
Production code references cite `common/src/rates.rs`,
`common/src/fp_core.rs`, `common/src/constants.rs`, and
`pool/src/interest.rs` at the file:line shown.
