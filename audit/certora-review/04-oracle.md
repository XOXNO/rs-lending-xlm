# Domain 4 — Oracle (Certora rules)

**Phase:** Certora formal-verification review
**Files in scope:**
- `controller/certora/spec/oracle_rules.rs` (298 lines)
- Cross-referenced summaries: `controller/certora/spec/summaries/mod.rs:50-77`

**Production references:**
- `controller/src/oracle/mod.rs:25-201` (price resolution + tolerance + staleness gates)
- `controller/src/oracle/reflector.rs:11-40`
- `controller/src/cache/mod.rs:24-67` (`allow_unsafe_price`, `allow_disabled_market_price`)
- `common/src/types.rs:235-253, 471-475` (`OraclePriceFluctuation`, `MarketStatus`)
- `common/src/constants.rs:46-54` (`MIN/MAX_FIRST/LAST_TOLERANCE`)

**Totals:** broken=4  weak=4  nit=2  missing=8 (coverage gaps)

---

## Summary verdict

The Oracle rule file mostly *describes* the production behaviour rather than *verifies* it. Five of the seven non-sanity rules either re-implement the property they claim to check (`first_tolerance_uses_safe_price`, `second_tolerance_uses_average`, `tolerance_bounds_valid`), restate the input assumption as the assertion (`tolerance_bounds_valid`), or call a summarized entry point whose nondet bound *already* satisfies the assertion (`price_staleness_enforced`, `price_cache_consistency`).

Combined with the fact that `token_price` is summarised in `summaries/mod.rs:50-62` and the rules import the summary path (not the unsummarised `crate::oracle::token_price::token_price` sub-module that the summary contract itself documents at `summaries/mod.rs:24-26`), every rule that calls `crate::oracle::token_price` exercises the *abstraction*, not production. The most critical guarantees of the Oracle subsystem — future-timestamp rejection regardless of `allow_unsafe`, dual-source disagreement panic, disabled-market gating, asset-decimals consistency, and dual-source vs single-source dispatching — are not verified by any rule in this file.

A reviewer reading this module would conclude that staleness, tolerance, and cache consistency are formally proven. They are not.

---

## oracle_rules.rs

### `oracle_rules.rs::price_staleness_enforced`

**Lines:** 32-47
**Severity:** broken
**Rubric items failed:** [1, 4, 5, 6]

**Why:**
1. **Wrong invariant.** The doc comment (lines 21-31) claims the rule verifies that "stale prices are rejected when `!allow_unsafe`". The assertion at line 46 only checks `feed.timestamp <= now_secs + 60`. That is the **future-timestamp** bound (`check_not_future`, `oracle/mod.rs:193-201`), not the **staleness** bound (`now - feed_ts > max_stale`, `oracle/mod.rs:174-184`). A feed dated `now - 999 days` passes the assertion trivially.
2. **Tautology under summary.** `crate::oracle::token_price` is summarised at `summaries/mod.rs:50-62`. The summary already constrains `timestamp <= cache.current_timestamp_ms / 1000 + 60` (line 56). The post-condition the rule asserts is *literally* the summary's `cvlr_assume!`. The rule is provable by inspection of the summary alone — the production `check_staleness` body never enters the proof. If somebody deletes `check_staleness` from `oracle/mod.rs`, this rule still passes.
3. **Soundness drift.** Production at `oracle/mod.rs:55` builds `feed.timestamp = cache.current_timestamp_ms / 1000` — i.e. the cache clock, never an oracle-supplied timestamp. The "stale leak" the rule claims to catch (a stale feed leaking into the returned `PriceFeed`) is *structurally impossible* in `token_price`'s post-condition because the returned timestamp is *always* the cache clock. Stale checks happen in `check_staleness` against the *raw Reflector timestamp* before the WAD conversion (`oracle/mod.rs:225, 248, 281, 334`). Those are not reachable from this rule.

**Scenario the rule misses:** Reflector returns `(price=$1, timestamp=now-7200)` with `max_stale=900`, `allow_unsafe_price=false`. Production panics at `oracle/mod.rs:183` with `PriceFeedStale`. The rule, even if wired to the unsummarised body, would not detect a regression that *removed* the `is_stale && !allow_unsafe_price` branch — because production still builds `feed.timestamp = now`, and `now <= now + 60` holds vacuously.

**Fix sketch:** invoke the unsummarised `crate::oracle::token_price::token_price` module path (cf. `summaries/mod.rs:24-26`), feed a nondet Reflector timestamp via the Reflector summary (which doesn't currently exist — see Coverage gap below), and assert that when `now - feed_ts > max_stale && !allow_unsafe`, the call panics (i.e. the assertion never reaches `cvlr_satisfy!(true)`). Without a Reflector summary, this rule cannot meaningfully fire.

---

### `oracle_rules.rs::first_tolerance_uses_safe_price`

**Lines:** 56-85
**Severity:** broken
**Rubric items failed:** [3, 6]

**Why:** Tautology. The rule body computes `let final_price = safe_price_val;` (line 82) and asserts `final_price == safe_price_val` (line 83). The comparison reduces to `safe_price_val == safe_price_val`. `is_within_anchor` is called with nondet inputs and is itself summarised to a nondet bool (`summaries/mod.rs:69-77`); the `if within_first` guard does not connect to any production invocation. Production's first-band branch at `oracle/mod.rs:130-131` (`safe_price`) is never executed, never inspected, never compared.

**What the rule should do:** Bind `agg_price` and `safe_price_val` into a fully-populated `OracleProviderConfig`, invoke the **unsummarised** `calculate_final_price` (which is *not* summarised — `oracle/mod.rs:114-157`), and assert the return value equals `safe_price_val` when `is_within_anchor(...,first)` returns `true`. As written the rule could be deleted with no loss of coverage.

---

### `oracle_rules.rs::second_tolerance_uses_average`

**Lines:** 94-150
**Severity:** broken
**Rubric items failed:** [3, 5, 6]

**Why:**
1. **Tautology.** `let final_price = (agg_price + safe_price_val) / 2;` (line 134). The two assertions at lines 147-148 prove the trivial fact that the integer-division mid-point of two positive numbers lies between them — a result independent of any oracle code.
2. **Misses the off-by-one bug it should catch.** Production uses `checked_add` then `/ 2` (`oracle/mod.rs:140-142`). The rule's `(agg_price + safe_price_val) / 2` *cannot* overflow under `cvlr_assume!(agg_price > 0 && safe_price_val > 0)` with i128 prover semantics, but production *can* overflow if `agg_price + safe_price_val > i128::MAX` and would panic with `MathOverflow`. The rule's mid-point spec doesn't exercise the overflow guard.
3. **Wrong invariant.** The rule does not verify the discriminant: that production returns the *average* in the second band only when **the first band fails**. The `if !within_first && within_second` guard relates two unrelated nondet booleans (the summary returns nondet for each call) — the prover can pick `within_first = false, within_second = true` even when the same inputs would produce `(true, true)` in production. Without the unsummarised `is_within_anchor` body, the implication is empty.

**Scenario the rule misses:** `agg=i128::MAX/2 + 100, safe=i128::MAX/2 + 100`, `within_first=false`, `within_second=true`. Production panics with `MathOverflow` at `oracle/mod.rs:141`. The rule computes `(agg + safe) / 2` which under i128 wrap-around yields a negative number; the rule's `min_price <= final_price <= max_price` assertion then fails — *but only because of the rule's own wrap-around*, not because of any production bug. The rule is testing its own arithmetic.

---

### `oracle_rules.rs::beyond_tolerance_blocks_risk_ops`

**Lines:** 160-196
**Severity:** broken
**Rubric items failed:** [1, 3, 5]

**Why:**
1. **Wrong shape.** The rule writes `if !within_second && !allow_unsafe_price { cvlr_assert!(false); }` (line 189) with a comment claiming "code panics before reaching here". But the rule **never invokes the production path that panics**. `is_within_anchor` is called in isolation; `calculate_final_price` (which contains the actual `panic_with_error!` at `oracle/mod.rs:146`) is not invoked. The `cvlr_assert!(false)` therefore fires unconditionally on the prover's branch where `within_second = false && allow_unsafe_price = false` — meaning **the rule reports a violation on every run**, regardless of whether production is correct.

   *Unless* the harness compiles this rule with the `is_within_anchor` summary that returns a nondet bool that the prover can pick to keep `within_second = true`, in which case the prover *trivially satisfies* the rule by always picking `within_second = true`. Either way, the rule is broken.
2. **Mode coverage missing.** Beyond-tolerance behaviour differs by `ExchangeSource`: `SpotOnly` never reaches `calculate_final_price`'s tolerance branch at all (it goes through the single-source `(Some, None)` arm at `oracle/mod.rs:151`). `DualOracle` calls `calculate_final_price(dex, Some(twap))` and *can* fall back to `Some(safe_price)` when DEX is None (graceful fallback at `oracle/mod.rs:99-100`). The rule treats all modes uniformly.
3. **Allow-unsafe semantics unverified.** Production at `oracle/mod.rs:148` returns `safe_price` (the TWAP), not the aggregator price, when `allow_unsafe_price = true && !within_second`. The rule's `cvlr_satisfy!(true)` (line 194) does not check **which** price is returned. A regression that returns `agg_price` (the unsafe aggregator) under permissive mode would not be caught.

**Scenario the rule misses:** `aggregator=$1000, safe=$1500, last_tol=200 BPS, allow_unsafe=true`. Production returns `safe=$1500` (the TWAP) per `oracle/mod.rs:148`. A regression that returns `agg_price=$1000` instead — exposing the user to a manipulated DEX — passes this rule because it only checks reachability, not the returned value.

---

### `oracle_rules.rs::price_cache_consistency`

**Lines:** 205-218
**Severity:** weak
**Rubric items failed:** [4, 6]

**Why:** Under `certora`, `crate::oracle::token_price` is the *summary*. The summary is a stateless function: each call produces fresh nondet `(price_wad, asset_decimals, timestamp)` (`summaries/mod.rs:50-62`). The rule then asserts the two calls return identical fields — which forces the prover to pick *the same nondet values twice*. There are two interpretations:

- If `apply_summary!` rewrites the call site at the spec module's import path (i.e. `crate::oracle::token_price` resolves to the summary), then the prover *can* pick distinct values for `feed1` and `feed2`, and the rule fails — incorrectly, because production's cache hit at `oracle/mod.rs:28-30` *does* return the same `PriceFeed`. So the rule is unsound: it can fail for a correct implementation.
- If the macro preserves the cache-checking body (`oracle/mod.rs:27-63`) at this call-site (i.e. only public callers are redirected, see `oracle/mod.rs:23-24`), then the rule trivially passes because line 28-30 short-circuits the second call with the cached feed. Either the assertion `feed1 == feed2` is a tautology (cache hit) or `find_price_feed`'s nested Reflector calls run *twice* and the summary returns *different* nondet — and the rule fails.

The `apply_summary!` documentation in `summaries/mod.rs:23-26` says the unsummarised body is reachable via `crate::oracle::token_price::token_price`. The rule at line 209 calls `crate::oracle::token_price` (single segment) — i.e. the summary, not the body. So this rule is most likely failing or vacuously passing depending on prover branch. **It does not verify the cache-consistency property described in the doc comment.**

**Fix sketch:** call `crate::oracle::token_price::token_price` (the unsummarised module-path), and supply Reflector summaries that return the same value on both invocations only under the cache-hit branch. Today the rule cannot distinguish "cache returned cached value" from "Reflector returned same nondet by luck".

---

### `oracle_rules.rs::tolerance_bounds_valid`

**Lines:** 229-269
**Severity:** broken
**Rubric items failed:** [3, 6]

**Why:** Pure tautology. Lines 237-250 are `cvlr_assume!` constraints that constrain the inputs into the valid range. Lines 253-268 are `cvlr_assert!` checks that *restate the same constraints*:

```
cvlr_assume!(first_upper_bps >= MIN_FIRST_TOLERANCE);   // line 237
...
cvlr_assert!(first_upper_bps >= MIN_FIRST_TOLERANCE);   // line 263
```

`cvlr_assume!(P) → cvlr_assert!(P)` is provable by reflexivity. The rule does not invoke any production code. It does not verify that the storage/config setter actually enforces these bounds (that lives in `controller/src/config.rs` validation paths — see `controller/src/validation.rs`). Verdict: testing the prover's `assume → assert` propagation, nothing else.

**Fix sketch:** call the actual config-setter (`crate::config::set_market_oracle_config` or similar) with nondet bps values and assert that out-of-range inputs panic. Or assert the invariants on a `MarketConfig` actually loaded from storage by `cache.cached_market_config(asset)` after a successful setter call.

---

### `oracle_rules.rs::oracle_tolerance_sanity`

**Lines:** 282-290
**Severity:** nit
**Rubric items failed:** [4]

**Why:** `is_within_anchor` is summarised to a nondet bool (`summaries/mod.rs:69-77`). `cvlr_satisfy!(within)` proves the trivial fact that "there exists a nondet bool whose value is true". The rule does not exercise the I256 ratio computation in production at `oracle/mod.rs:381-391`. Acceptable as a sanity check that the summary is wired, but its name (`tolerance_sanity`) overpromises.

---

### `oracle_rules.rs::price_cache_sanity`

**Lines:** 293-298
**Severity:** nit
**Rubric items failed:** [4]

**Why:** `feed.price_wad > 0` is *exactly* the summary's pre-existing constraint at `summaries/mod.rs:54`. `cvlr_satisfy!(true)` is reachable by construction. The sanity check confirms only that the summary's constraints are non-empty, not anything about the production oracle.

---

## Coverage gaps (missing rules)

The following invariants from the rubric have **no corresponding rule** in this file:

### Missing-1: future-timestamp rejection is unconditional

Production at `oracle/mod.rs:185-189` calls `check_not_future` *unconditionally* inside `check_staleness`, regardless of `allow_unsafe_price`. The unit test at `oracle/mod.rs:898-908` (`test_check_staleness_future_timestamp_panics_even_when_unsafe_allowed`) confirms this is a load-bearing invariant. **No Certora rule enforces it.** A regression that nests the future-timestamp check under `if !allow_unsafe_price` (a plausible refactor) would silently accept future-dated oracles for repay/views.

**Severity if missed:** high — future-dated prices are the standard signature of oracle-feed compromise.

### Missing-2: disabled-market gating

Production at `oracle/mod.rs:37-39` panics with `PairNotActive` for `MarketStatus::Disabled` unless `cache.allow_disabled_market_price` is true. The cache constructor `new_with_disabled_market_price` (`cache/mod.rs:41-43`) is the only path that sets the flag, and only repay uses it. **No Certora rule enforces this dual gate.** A regression that drops the `!cache.allow_disabled_market_price` guard would let liquidators price-query disabled markets.

### Missing-3: PendingOracle rejection

`oracle/mod.rs:34-36` panics for `MarketStatus::PendingOracle` regardless of any flag. Combined with Missing-2, the `MarketStatus`-gating logic is wholly unverified.

### Missing-4: dual-source dispatching invariant

Production at `oracle/mod.rs:90-101` selects `SpotOnly` (single-source, no tolerance gate), `DualOracle` (CEX-TWAP + DEX-spot, graceful fallback when DEX is None), or default (`SpotVsTwap`, both required). **No rule verifies that `SpotOnly` never enters `calculate_final_price`'s tolerance branches** — i.e. that a `SpotOnly` market with manipulated spot is *not* wrapped in a fake "within tolerance" check. This is the rubric's "SpotOnly mode never compares two sources" invariant.

### Missing-5: dual-source disagreement panics in strict mode

Rubric explicitly asks: "Out-of-tolerance deviation rejected when strict, fallback to safe-price when permissive". The rule `beyond_tolerance_blocks_risk_ops` *attempts* this but is broken (see entry above). No correct rule covers it.

### Missing-6: returned price non-negative invariant

Production at `oracle/mod.rs:49-51` panics with `InvalidPrice` if `price <= 0`. The summary at `summaries/mod.rs:54` enforces this via `cvlr_assume!(price_wad > 0)`. **No rule asserts this against the unsummarised body.** A regression that lets a zero or negative price through (e.g. someone changes `<=` to `<`) is invisible.

### Missing-7: `asset_decimals` matches MarketConfig

Production at `oracle/mod.rs:54` sets `asset_decimals: config.asset_decimals` (the `MarketConfig.oracle_config.asset_decimals` value). The summary lets `asset_decimals` be any nondet `u32 <= 27` (`summaries/mod.rs:55`). **No rule verifies the equality between returned `asset_decimals` and `cache.cached_market_config(asset).oracle_config.asset_decimals`.** A regression that returns the *Reflector* contract's decimals instead of the *configured* decimals (an easy off-by-one for any human reviewing the file) is invisible.

### Missing-8: TWAP insufficient-observations panic

`oracle/mod.rs:277-279, 330-332` panic with `TwapInsufficientObservations` when `history.len() < records / 2 (rounded up)`. Tested in unit tests at `oracle/mod.rs:1306-1369`. **No Certora rule.** A regression that drops the `min_twap_observations` floor would silently accept TWAPs computed from a single observation.

### Missing-9: DEX staleness is soft (graceful fallback)

`oracle/mod.rs:354-357` returns `None` when DEX is stale, while CEX staleness panics. This asymmetry — DEX outage degrades to TWAP-only, CEX outage blocks risk-ops — is a critical operational property. **No rule verifies it.**

### Missing-10: `(None, None)` panics

`oracle/mod.rs:153-155` panics with `NoLastPrice` when both aggregator and safe are absent. The unit test at `oracle/mod.rs:861-869` confirms it. **No rule.** A regression that returned 0 for the both-None branch would silently trade against zero-priced collateral.

---

## Summary table

| Rule                                | Verdict | Real coverage |
|-------------------------------------|---------|---------------|
| `price_staleness_enforced`          | broken  | none — asserts future-timestamp bound, not staleness; provable from summary alone |
| `first_tolerance_uses_safe_price`   | broken  | none — tautology `safe == safe` |
| `second_tolerance_uses_average`     | broken  | none — re-implements the average; doesn't invoke production |
| `beyond_tolerance_blocks_risk_ops`  | broken  | none — `cvlr_assert!(false)` without invoking the panicking path; doesn't check returned price under permissive mode |
| `price_cache_consistency`           | weak    | none — calls summary, not unsummarised cache body |
| `tolerance_bounds_valid`            | broken  | none — `assume(P) → assert(P)` reflexivity |
| `oracle_tolerance_sanity`           | nit     | trivial reachability of summary nondet |
| `price_cache_sanity`                | nit     | trivial reachability of summary nondet |

**Net:** 0 of 8 rules enforce a non-trivial production invariant. 10 critical Oracle invariants from the rubric are uncovered.

---

## Recommended remediation priorities

1. **Wire rules to the unsummarised module path.** The `summaries/mod.rs:23-26` comment promises that production bodies remain reachable at `crate::oracle::token_price::token_price` (etc). Every Oracle rule that invokes `crate::oracle::token_price` is reaching the summary. Fix the import to invoke the unsummarised path, and add Reflector summaries (currently absent — every Reflector call in `cex_spot_price`, `cex_twap_price`, `dex_spot_price` is a havoc to the prover).

2. **Replace tautology rules with calls to `calculate_final_price`.** `calculate_final_price` is *not* summarised (it doesn't appear in `summaries/mod.rs`). It is directly callable. Rules 2, 3, 4 should invoke it with nondet `(agg, safe, tolerance_config)` and assert the discriminant + returned value, replacing the current "compute the answer ourselves and assert it equals itself" pattern.

3. **Add a future-timestamp rule** that exercises `check_staleness` (or the unsummarised `token_price`) with `allow_unsafe_price = true` and a feed timestamp `> now + 60`, asserting the call panics. This is the highest-severity uncovered invariant.

4. **Add a `MarketStatus` gating rule** covering all four (status × allow_disabled) combinations.

5. **Add a `(None, None) panics` rule** by direct call to `calculate_final_price(cache, None, None, &cfg)`.

6. **Drop `tolerance_bounds_valid` or re-aim it at the config-setter validation path.** In its current form it adds noise without value.
