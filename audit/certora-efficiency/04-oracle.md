# Domain 4 — Oracle (Certora rules) — Efficiency audit

**Phase:** Certora efficiency review (post-P0b rewrite)
**Files in scope:**
- `controller/certora/spec/oracle_rules.rs` (300 lines, 8 rules)

**Production references:**
- `controller/src/oracle/mod.rs:25-201` — `token_price`, `find_price_feed`, `normal_price`, `calculate_final_price`, `check_staleness`, `check_not_future`, dual-source aggregation
- `controller/src/oracle/mod.rs:372-392` — `is_within_anchor` (I256 ratio compute, summarised)
- `controller/src/oracle/reflector.rs:31-40` — Reflector trait (`lastprice`, `prices`) — **no summary**
- `controller/src/cache/mod.rs:34-67` — `allow_unsafe_price`, `allow_disabled_market_price`
- `controller/certora/spec/summaries/mod.rs:48-77` — `token_price_summary`, `is_within_anchor_summary`
- `common/src/fp_core.rs:13-77` — `mul_div_half_up`, `rescale_half_up` (I256 path)

**Totals:** broken=2  weak=4  nit=2  missing=4 (efficiency / scope mismatch)

**Rubric items used (standard 9):**
1. Branch fan-out unconstrained for the property under test
2. Heavy I256 / arithmetic path inlined when irrelevant to the property
3. Unsummarised entry point invoked when the property doesn't require it
4. Tautology / assertion-restates-assumption / vacuous
5. Wrong scope — too broad (should split into per-gate sub-rules)
6. Wrong scope — too narrow (degenerates to summary reachability check)
7. Storage / `cached_market_config` left fully havoced when shape would tighten the rule
8. Reflector cross-contract call left as pure havoc (no summary)
9. Redundant with another rule (rubric item already verified elsewhere)

---

## Summary verdict

The P0b rewrite **partially** moved oracle rules from "summary tautology" to "real-implementation invocation". Two rules (`price_staleness_enforced`, `price_cache_consistency`, `price_cache_sanity`) now call the unsummarised `crate::oracle::token_price::token_price` module path. `oracle_tolerance_sanity` and the three `_tolerance_*` rules invoke the unsummarised `crate::oracle::is_within_anchor::is_within_anchor`.

**The trade-off was poor.** Three problems compound:

1. **Reflector has no summary.** Every unsummarised `token_price` call traverses `cex_spot_price` → `ReflectorClient::lastprice` (and for non-`SpotOnly`, `client.prices` + a TWAP loop). The Soroban contract-call boundary is pure havoc to the prover — every loop iteration multiplies the path count without giving the prover any reasoning to do. So the rules pay full I256-storage-loop cost and still verify against havoced returns.
2. **Branch fan-out is uncontrolled.** `MarketStatus` (3) × `ExchangeSource` (3) × `allow_unsafe_price` (2) × `allow_disabled_market_price` (2) = **36 paths** through `token_price`, of which only 1–4 are relevant to any single rule. None of the rules pin the configuration.
3. **The unsummarised body still doesn't catch the bugs the previous review flagged.** The fix at `oracle_rules.rs:46` (`feed.timestamp <= now_secs + 60`) is *still* trivially provable from the construction `feed.timestamp = cache.current_timestamp_ms / 1000` at `oracle/mod.rs:55`. The unsummarised path doesn't make the rule check what its docstring claims (staleness against the *Reflector* timestamp).

In short: the rewrite paid a 10-100× prover cost increase for almost no additional verification value. **Calling unsummarised `token_price` without a Reflector summary and without pinning `(MarketStatus, ExchangeSource)` is the worst of both worlds**: heavy AND uninformative.

The fastest improvement is not "go back to summary" — it's:
- Author a Reflector summary (see "Reflector summary proposal" below).
- Pin `(MarketStatus, ExchangeSource, allow_unsafe_price, allow_disabled_market_price)` to one configuration per rule via `cvlr_assume!` on `cache.cached_market_config(asset)`.
- Split the four "single property of `token_price`" rules into per-gate rules that call `check_staleness`, `check_not_future`, `calculate_final_price` directly. `calculate_final_price` is **not** summarised (`oracle/mod.rs:114-157`) — it can be invoked with hand-built `OracleProviderConfig` values for free.

---

## Per-rule findings

### `oracle_rules.rs::price_staleness_enforced`

**Lines:** 32-47
**Severity:** broken
**Rubric items failed:** [1, 2, 3, 4, 8]

**Why:**
The rule pays the full unsummarised `token_price` cost — a `cached_market_config` storage read (havoced `MarketConfig`), a status-match (3 branches), a `find_price_feed` dispatch (2 branches), a `normal_price` dispatch (3 branches: `SpotOnly` / `DualOracle` / default `SpotVsTwap`), and a Reflector `lastprice`/`prices` call (havoced) — plus a possible `is_within_anchor` summary call inside `calculate_final_price`. That's at least **18 prover paths** through `token_price` before reaching the post-condition.

Then it asserts `feed.timestamp <= now_secs + 60`. But the production line at `oracle/mod.rs:55` constructs the returned feed as `timestamp: cache.current_timestamp_ms / 1000` — i.e. *exactly* the cache clock. So `now_secs <= now_secs + 60` is structurally true, regardless of whether `check_staleness` ran, regardless of `allow_unsafe_price`, regardless of what Reflector returned. **The rule passes by construction of the post-condition, not by virtue of any staleness gate.**

The 18-path traversal does zero verification work. Worse, the per-path Reflector return is havoced (rubric 8), so even if the rule did examine the *raw* feed timestamp, it could not constrain it. The unsummarised path is strictly more expensive than the summary version (which trivially constrained `timestamp <= cache.current_timestamp_ms / 1000 + 60`) and verifies the same trivial property.

**Decision:** Use a smaller per-gate assertion. Call `crate::oracle::check_staleness(&cache, feed_ts, max_stale)` directly with nondet `(feed_ts, max_stale, allow_unsafe_price)` and assert that the call panics iff `(now - feed_ts > max_stale && !allow_unsafe_price) || feed_ts > now + 60`. `check_staleness` is *unsummarised* (`oracle/mod.rs:174-189`) and takes only scalars — no storage, no Reflector. This costs one `now_secs` arithmetic line and verifies the actual gate.

---

### `oracle_rules.rs::first_tolerance_uses_safe_price`

**Lines:** 56-85
**Severity:** broken
**Rubric items failed:** [3, 4, 6]

**Why:**
The rule pays for one unsummarised `is_within_anchor` call (which crosses `Ray::div` → `mul_div_half_up` → I256 mul/add/div, then `rescale_half_up` — see `common/src/fp.rs:44-45` and `common/src/fp_core.rs:13-20`). That I256 path is exactly the prover-heavy compute the existing summary `is_within_anchor_summary` (`summaries/mod.rs:76-84`) was created to avoid.

Then the rule body, lines 79-84:
```
if within_first {
    let final_price = safe_price_val;
    cvlr_assert!(final_price == safe_price_val);
}
```
…assigns `safe_price_val` to a local and asserts the local equals `safe_price_val`. Reflexivity. **No oracle code is observed at the assertion site.** `calculate_final_price` (the function whose first-band branch returns `safe_price` at `oracle/mod.rs:130-131`) is never invoked.

Cost paid: I256 ratio compute. Coverage gained: zero.

**Decision:** Split into a per-gate rule that calls `crate::oracle::calculate_final_price` directly. `calculate_final_price` is **not** summarised (`oracle/mod.rs:114-157`); it takes scalar `Option<i128>` inputs and an `OracleProviderConfig`. Build the config locally with nondet tolerance values (already cheap), assume `is_within_anchor_summary` returns `true` for the first band, and assert the return equals `safe_price`. That replaces the I256 compute with the summary's nondet bool and verifies the actual production branch.

---

### `oracle_rules.rs::second_tolerance_uses_average`

**Lines:** 94-150
**Severity:** broken
**Rubric items failed:** [2, 3, 4, 6]

**Why:**
The rule invokes the unsummarised `is_within_anchor` **twice** (lines 117-130) — paying two full I256 ratio computations — then **re-implements the average** at line 134:
```
let final_price = (agg_price + safe_price_val) / 2;
```
…and asserts the bound `min(agg, safe) <= final_price <= max(agg, safe)`, which is a property of integer mid-points independent of any oracle code. Production's average lives at `oracle/mod.rs:140-142` (`agg_price.checked_add(safe_price).unwrap_or_else(|| panic_with_error!(MathOverflow)) / 2`), and is never executed by this rule.

The two I256 calls are wasted: the prover's `within_first = false, within_second = true` discriminant is *not used as input* to any production function — the rule's `final_price` is computed independently. Two unsummarised `is_within_anchor` calls, each with a 256-bit mul/add/div, is the most expensive arithmetic the prover sees in any oracle rule.

The rule also misses the `MathOverflow` guard (production uses `checked_add`; the rule uses raw `+`).

Cost paid: 2× I256 mul-div-rescale. Coverage gained: integer-arithmetic-trivia about midpoints.

**Decision:** Split. Call `calculate_final_price` directly, with `is_within_anchor` summarised (the existing nondet-bool summary). Bind the discriminant by asserting on the returned value: when (`within_first=false, within_second=true`), the return must equal `(agg + safe) / 2` (or panic with `MathOverflow`). This is one prover branch, one I256 call (the inner `checked_add`), and verifies the production average path.

---

### `oracle_rules.rs::beyond_tolerance_blocks_risk_ops`

**Lines:** 160-196
**Severity:** broken
**Rubric items failed:** [2, 3, 4, 9]

**Why:**
One unsummarised `is_within_anchor` call (lines 177-183) — full I256 cost — followed by a hand-coded `if !within_second && !allow_unsafe_price { cvlr_assert!(false); }`. As the certora-review noted, this is unsound under any wiring: either the rule fires on a legitimate combination (false positive) or the prover trivially evades it by picking `within_second = true` (rubric 4). **It does not invoke `calculate_final_price`'s panic path** at `oracle/mod.rs:144-148`.

This rule pays the I256 cost and verifies *nothing* — the assert-false hand-code does not connect to any production flow. The `cvlr_satisfy!(true)` at line 194 is also redundant: `oracle_tolerance_sanity` (lines 282-290) already proves nondet-bool reachability.

**Decision:** Replace with a direct `calculate_final_price` call asserting it panics in the strict mode and returns `safe_price` in permissive mode. `calculate_final_price` is unsummarised; this is the canonical place to verify the panic gate at `oracle/mod.rs:146`. The current I256 call is pure waste; the assertion never reaches production code.

---

### `oracle_rules.rs::price_cache_consistency`

**Lines:** 205-218
**Severity:** weak
**Rubric items failed:** [1, 7, 8]

**Why:**
Now that the rule calls the unsummarised `crate::oracle::token_price::token_price`, the cache-hit branch at `oracle/mod.rs:28-30` does the right thing structurally — `feed2` *will* equal `feed1`. So the rule is sound (no longer suspect of false-positive). But it's expensive:

The prover traverses `token_price` once with the full 18-path branch fan-out (status × source × cache flags), populates `prices_cache` for `asset`, then traverses again — and the cache hit short-circuits the second call. The second call is cheap; the **first call** dominates cost. But the first call's `(MarketStatus, ExchangeSource)` is not pinned, so the prover explores 9 configurations. The Reflector calls inside `cex_spot_price` / `cex_twap_price` / `dex_spot_price` are pure havoc (rubric 8) — pricing-correctness checks like `price > 0` (oracle/mod.rs:49) become panic-or-nondet branches.

The rule's only assertion is `feed1 == feed2`. The cache-hit guarantee at `oracle/mod.rs:28-30` is a single `Map::get` short-circuit — that's the property worth verifying. Everything else inside the first traversal is irrelevant.

**Decision:** Simplify. After the first `token_price` call, manually call `cache.set_price(&asset, &feed1)` (already idempotent) — no, simpler: after the first call, the cache is set; the second call hits the cache. The current rule structure is correct in shape; the issue is the unbounded first-call cost. Pin to `MarketStatus::Active + ExchangeSource::SpotOnly` (the simplest configuration) via `cvlr_assume!` on `cache.cached_market_config(asset).status` and `.oracle_config.exchange_source`. That collapses the 18-path fan-out to 1 path. Better still: replace the first call with a direct `cache.set_price(&asset, &nondet_feed)` and only verify the second call hits the cache — that's the actual cache-hit invariant, costing one map lookup instead of full oracle resolution.

---

### `oracle_rules.rs::tolerance_bounds_valid`

**Lines:** 229-269
**Severity:** broken
**Rubric items failed:** [4, 9]

**Why:**
`assume(P) → assert(P)` reflexivity, as the certora-review already flagged. The P0b rewrite did not touch this rule. It invokes no production code. Lines 237-250 `cvlr_assume!` the input bounds; lines 253-268 `cvlr_assert!` the same bounds. The prover proves the trivial implication.

The rule's `MIN_FIRST_TOLERANCE`, `MIN_LAST_TOLERANCE`, `MAX_*` bounds **are** load-bearing constants — but the place that enforces them is `controller/src/validation.rs` (the config-setter validation path), which this rule never invokes. Whether the constants are honored is a property of the *setter*, not of the *constants themselves*.

Cost paid: zero (no production code). Coverage gained: zero. The rule is fast precisely because it does nothing.

**Decision:** Delete or re-aim at the config setter. In its current form it is noise. If kept, point it at `validate_oracle_provider_config` (or whichever validation function in `controller/src/validation.rs` enforces the bounds) and assert that out-of-range inputs panic.

---

### `oracle_rules.rs::oracle_tolerance_sanity`

**Lines:** 282-290
**Severity:** nit
**Rubric items failed:** [6]

**Why:**
The rule calls the unsummarised `is_within_anchor` (paying one I256 ratio compute) and asserts `cvlr_satisfy!(within)`. Since `within` is the boolean output of the real I256 path (not the summary's nondet bool), the prover has to find concrete inputs in the bounded range `[1, 1_000_000 * WAD)` such that the ratio falls within the 2% tolerance. This is reachable (e.g. `agg = safe`), but the prover work to find it traverses the full mul-div-rescale chain.

For a sanity check, this is overkill: a `cvlr_satisfy!(true)` after `let _ = is_within_anchor(...)` would verify "the function is callable" at zero verification cost. The current form verifies "there exist bounded positive inputs for which the function returns true" — true, but trivial, and now expensive.

**Decision:** Acceptable as a sanity check post-rewrite. The cost is a single I256 traversal — bounded, terminating. Lower-priority cleanup.

---

### `oracle_rules.rs::price_cache_sanity`

**Lines:** 293-299
**Severity:** nit
**Rubric items failed:** [1, 6, 8]

**Why:**
Calls unsummarised `token_price` with `asset = e.current_contract_address()` and asserts `feed.price_wad > 0`. Because `token_price` is unsummarised, the rule traverses the full configuration fan-out (`status` × `exchange_source`) and the Reflector havoc, and `cvlr_satisfy!(feed.price_wad > 0)` requires the prover to find one path where the post-condition `price > 0` (production guard at `oracle/mod.rs:49`) holds. This is reachable, but every path through the unsummarised body has to be considered.

Compared to the previous summary-based form (`cvlr_assume!(price_wad > 0)` in `token_price_summary`), the rewrite turned a one-line satisfaction check into a 36-path traversal. The verification value remains "the function is callable and can return a positive price" — still nit-level.

**Decision:** Not worth the cost. Either revert to summary form (and rename to `_sanity_summary`) or pin the config (single `MarketStatus::Active`, single `ExchangeSource::SpotOnly`, populated `cex_oracle`) to keep the path count bounded. As a pure reachability check, summary-based is the right choice.

---

## Recommended trade-offs (per rule)

| Rule | Current call | Right call | Reason |
|---|---|---|---|
| `price_staleness_enforced` | unsummarised `token_price` | direct `check_staleness` with scalar inputs | Property is per-gate; full pipeline adds zero coverage |
| `first_tolerance_uses_safe_price` | unsummarised `is_within_anchor` (×1) | `calculate_final_price` + summarised `is_within_anchor` | Production branch is in `calculate_final_price`, not `is_within_anchor` |
| `second_tolerance_uses_average` | unsummarised `is_within_anchor` (×2) | `calculate_final_price` + summarised `is_within_anchor` | Same as above; ×2 I256 cost is pure waste |
| `beyond_tolerance_blocks_risk_ops` | unsummarised `is_within_anchor` (×1) | `calculate_final_price` (panics in strict mode) | The panic gate lives in `calculate_final_price`, not `is_within_anchor` |
| `price_cache_consistency` | unsummarised `token_price` (×2) | pin config + cache.set_price + one `token_price` | Cache hit is a 1-line `Map::get`, not the whole pipeline |
| `tolerance_bounds_valid` | (nothing) | aim at `validate_oracle_provider_config` or delete | Reflexivity rule — verifies nothing |
| `oracle_tolerance_sanity` | unsummarised `is_within_anchor` | acceptable as-is | Bounded I256 traversal, one branch |
| `price_cache_sanity` | unsummarised `token_price` | summarised `token_price` | Pure reachability check; full pipeline is overkill |

---

## Coverage gaps (efficiency angle)

The certora-review (`audit/certora-review/04-oracle.md` "Coverage gaps") listed 10 missing invariants. Of those, the most efficiency-friendly to add are the per-gate rules — they all fit the "scalar inputs, no Reflector, no storage" pattern:

- **Missing-1 (future-timestamp unconditional):** call `check_staleness(&cache, feed_ts, max_stale)` with nondet scalars and `allow_unsafe_price = true`. Assert it panics when `feed_ts > now + 60`. Cost: ~5 prover lines. **Priority: high.**
- **Missing-3 (PendingOracle rejection):** call `cache.cached_market_config(asset)` after writing a `PendingOracle` config to storage, then call `token_price`. Assert panic. Cost: 1 storage write + 1 traversal of the early-return branch in `token_price` (lines 33-41). **Priority: medium**, requires storage scaffolding.
- **Missing-10 ((None, None) panics):** call `calculate_final_price(&cache, None, None, &cfg)` directly. `calculate_final_price` is unsummarised; the input is scalar. Assert panic. Cost: ~3 prover lines. **Priority: high.**
- **Missing-6 (returned price non-negative):** would require the unsummarised `token_price` body and a Reflector summary that can return zero. **Priority: medium** — not addable until the Reflector summary exists.

Missing-2 / Missing-4 / Missing-5 / Missing-8 / Missing-9 require a Reflector summary to be feasible at reasonable cost. See next section.

---

## Reflector summary proposal

**Yes, author one.** A `controller/certora/spec/summaries/reflector.rs` is the single highest-leverage addition for oracle verification. Here's why and what.

### Why

Currently every unsummarised `token_price` call traverses one of:
- `cex_spot_price` → `client.lastprice(&ra)` → **havoced `Option<ReflectorPriceData>`**
- `cex_twap_price` / `cex_spot_and_twap_price` → `client.lastprice` + `client.prices(&ra, &records)` → **havoced `Option<Vec<ReflectorPriceData>>`** plus a TWAP loop summing `records` items
- `dex_spot_price` → `client.lastprice` → **havoced `Option<ReflectorPriceData>`**

Without a summary, the prover sees these as fully-undefined external returns. Three problems:

1. **Loop unrolling.** The TWAP loops at `oracle/mod.rs:266-275, 320-328` iterate `history.len()` times. With `twap_records` unbounded, the prover unrolls. The summary can return a fixed-length nondet vector (e.g. always 3 entries) that satisfies the post-condition `history.len() >= min_twap_observations(records)` — collapsing the loop to a fixed unroll count.
2. **Staleness proof can't fire.** `check_staleness(cache, oldest_ts, max_stale)` panics conditionally on the *Reflector-supplied* timestamp. With Reflector havoced, the prover doesn't know whether `oldest_ts > now` or `oldest_ts < now - max_stale`. A summary that exposes those two booleans as nondet but bounds the relation between `oldest_ts` and the fresh-sample `timestamp` makes the staleness rule actually fire.
3. **Decimals-mismatch invariants impossible.** Production at `Wad::from_token(pd.price, market.cex_decimals)` (`oracle/mod.rs:227, 249, 282, 359`) trusts the Reflector's price magnitude. A summary that ties `pd.price` to a configured `cex_decimals` makes "asset_decimals consistency" verifiable.

### What the summary should contain

```rust
// summaries/reflector.rs (proposed contents — do not implement now)

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Env, Vec};
use crate::oracle::reflector::{ReflectorAsset, ReflectorPriceData};

/// Summary for `ReflectorClient::lastprice`.
///
/// Production guarantees:
///   * Returns Some(pd) where pd.price > 0 in the happy path; None when the
///     asset is not tracked by the oracle.
///   * Timestamp is the oracle's last observation; can be any non-negative u64.
pub fn reflector_lastprice_summary(
    env: &Env,
    _asset: &ReflectorAsset,
) -> Option<ReflectorPriceData> {
    let some: bool = nondet();
    if !some {
        return None;
    }
    let price: i128 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price > 0);
    Some(ReflectorPriceData { price, timestamp })
}

/// Summary for `ReflectorClient::prices`.
///
/// Returns at most `records` samples. The summary returns a fixed 3-sample
/// vector to keep the TWAP-loop unroll bounded — sufficient for any
/// `min_twap_observations` proof since `min_twap_observations(records) =
/// max(1, records.div_ceil(2))`, capped at `ceil(records/2)`.
pub fn reflector_prices_summary(
    env: &Env,
    _asset: &ReflectorAsset,
    records: &u32,
) -> Option<Vec<ReflectorPriceData>> {
    let some: bool = nondet();
    if !some {
        return None;
    }
    // Two cases: empty (graceful fallback) or fixed 3-sample (TWAP path).
    let empty: bool = nondet();
    let mut out = Vec::new(env);
    if !empty {
        // Three nondet samples with positive prices and bounded relative
        // timestamps. Three is the smallest count that exercises both the
        // sum-loop and the oldest-ts tracking at oracle/mod.rs:266-275.
        for _ in 0..3 {
            let price: i128 = nondet();
            let timestamp: u64 = nondet();
            cvlr_assume!(price > 0);
            cvlr_assume!(*records >= 3);  // satisfies min_twap_observations
            out.push_back(ReflectorPriceData { price, timestamp });
        }
    }
    Some(out)
}
```

The wiring is the same `summarized!` indirection used elsewhere — but ReflectorClient is a `#[contractclient]` trait, not a `pub fn`, so the summary needs `apply_summary!` plumbing on the trait method. Easiest mechanical route: introduce thin wrapper functions in `controller/src/oracle/reflector.rs` (`fn lastprice(env, addr, asset) -> Option<...>` and `fn prices(env, addr, asset, records) -> Option<...>`) that call the trait methods, then `summarized!` those wrappers. That requires touching production — out of scope for an audit but a clean ~20-line refactor.

### Rules that benefit

| Rule | Benefit |
|---|---|
| `price_staleness_enforced` (rewritten to call `check_staleness` directly) | No benefit — already scalar |
| `price_cache_consistency` | High — collapses the 18-path `token_price` traversal to 1 |
| `price_cache_sanity` | High — same |
| Missing-2 `disabled_market_gating` | Required — the rule needs a successful Reflector return for the happy path |
| Missing-4 `SpotOnly_never_compares` | Required — needs `lastprice` to return without `prices` being called |
| Missing-5 `dual_source_disagreement` | Required — needs both CEX and DEX returns with controlled relative magnitudes |
| Missing-6 `price_non_negative` | Required — must let the prover try to return zero |
| Missing-7 `asset_decimals_consistency` | Required — must tie returned decimals to `MarketConfig.oracle_config.asset_decimals` |
| Missing-8 `TWAP_insufficient_observations` | Required — the rule needs `prices` returning a short vector |
| Missing-9 `DEX_staleness_soft` | Required — needs DEX lastprice with a stale timestamp to produce `None` rather than panic |

**Total:** 1 high-impact existing rule + 7 missing rules unblocked. The single Reflector summary is the difference between "oracle subsystem unverified" and "oracle subsystem half-verified".

### What the summary should NOT contain

- No bound on `timestamp` — the production gate is `check_staleness` and `check_not_future`. The summary should let the prover pick any `u64` so the staleness rule fires on the real branch.
- No bound on price magnitude beyond `> 0` — the `<= 0` panic at `oracle/mod.rs:49` is a production guard worth verifying. Allowing negatives in the summary lets the verifier check the guard.
- No correlation between successive calls — the cache-consistency rule should rely on **the controller's cache**, not Reflector returning the same value twice. Summarising "Reflector is consistent" hides the actual property under test.

---

## Conclusion

The P0b rewrite traded a tautology problem for a path-explosion problem without first fixing the underlying issue (no Reflector summary, no config pinning). All four "tolerance" rules (`first/second/beyond_tolerance_*`, plus `tolerance_bounds_valid`) should be rewritten to invoke the unsummarised `calculate_final_price` (which is *already* unsummarised, scalar-input, and not paying any I256 cost the rules don't need) instead of the unsummarised `is_within_anchor`. Two rules (`price_staleness_enforced`, `price_cache_consistency`) should be either pinned to a specific market configuration or refactored to call sub-functions directly. The two `_sanity` rules can stay nit-level if costs are acceptable, but `price_cache_sanity` should revert to summary-based for cost.

The dominating efficiency win is authoring `summaries/reflector.rs` — without it, every rule that touches `token_price` is paying for havoced cross-contract returns and gaining no verification. With it, ten currently-impossible rules become feasible.
