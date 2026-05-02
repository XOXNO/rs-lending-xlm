# Certora Efficiency Audit — Domain 05: E-Mode + Isolation

**Date:** 2026-05-01
**Files in scope**
- `controller/certora/spec/emode_rules.rs` (525 lines, 16 rules incl. 2 sanity)
- `controller/certora/spec/isolation_rules.rs` (199 lines, 9 rules incl. 2 sanity)

**Production references**
- `controller/src/positions/emode.rs:14-44, 47-51, 81-98, 106-123, 131-162`
- `controller/src/storage/emode.rs:38-88, 94-106`
- `controller/src/config.rs:271-304, 310-360`
- `controller/src/positions/supply.rs::process_supply` (76-104), `prepare_deposit_plan` (166-195), `validate_bulk_isolation` (validation.rs:97-111)
- `controller/src/positions/borrow.rs::borrow_batch` (86-104), `handle_isolated_debt` (306-341)
- `controller/src/positions/repay.rs::process_repay` (26-54), `adjust_isolated_debt_for_repay`
- `controller/src/storage/certora.rs` (compat shims `get_account_attrs`, `accounts::get_account_data`, `positions::*`)
- `controller/certora/spec/summaries/pool.rs` (defined but **not wired** — see F1 below)

---

## TL;DR

The P1b rewrites moved several rules in the right *direction* (mutual exclusion is now inductive after `process_supply`; `emode_remove_category` actually checks the side-map cleanup; `isolation_debt_ceiling_respected` switched to `compat::borrow_single` and reads from raw meta) but they **did not pay the cost-of-soundness price**: every rewritten rule now executes a deeply nested entry point — `process_supply`, `borrow_single`, `repay_single` — and the pool/SAC summaries those entry points pass through (`summaries/pool.rs`) **are still not wired** at any production callsite. Each of these rules now drags 1 cross-contract pool call, the whole `ControllerCache` machinery, and the oracle path — most of which is havoced into the prover's TAC graph rather than abstracted.

Concretely:

- The two new "after-supply" mutual-exclusion / debt-ceiling rules (~5 rules) each pull in `pool::supply` / `pool::borrow` as **havoc** because `summarized!` is not applied at the pool client callsite. That's a ~10× cost increase relative to the prior read-only rules. The work itself is sound, but the rules are fragile (large TAC graphs, slow runs, frequent vacuity / spurious counter-examples).
- `emode_remove_category` walks a `Map<Address, EModeAssetConfig>` symbolically. Without a side-map size cap, `members.keys().get(0)` and the `is_empty()` post-check expand to a quantifier over an unbounded map. This is the heaviest rule in the file by a wide margin and the most likely to TAC-blow.
- `emode_overrides_asset_params` is now correctly scoped (`!is_deprecated`) and is fast because it calls only `apply_e_mode_to_asset_config` directly. This is the model the rest of the rewrites should have followed.

The rules verify the right *invariants*. The cost rubric, not the invariants, is where the file under-delivers. **3 high, 4 medium, 4 low** efficiency findings below; no false claims of correctness, but several rules will cost 5–10× more prover time than necessary and a couple will likely time out unless side-map size is bounded.

---

## F1 (HIGH, cross-cutting): Pool client summaries are not wired — every `process_supply` / `borrow_single` / `repay_single` rule executes a real cross-contract call

**Location:** `controller/certora/spec/summaries/pool.rs:76-205` defines `supply_summary`, `borrow_summary`, `withdraw_summary`, `repay_summary`, `update_indexes_summary`. None of them are applied via `summarized!` at any callsite:

```bash
$ grep -rn "summarized!" controller/src/positions/
# (no matches)
```

The five callsites that go cross-contract — `supply.rs:370`, `borrow.rs:55`, `borrow.rs:262`, `repay.rs:104` — invoke `pool_interface::LiquidityPoolClient::new(env, pool_addr).{supply,borrow,repay,create_strategy,...}` directly. To Certora, every one of these is pure havoc.

**Rules affected by this finding (in this domain):**
- `emode_only_registered_assets` (49–55)
- `emode_borrow_only_registered_assets` (89)
- `emode_only_borrowable_assets` (124)
- `emode_only_collateralizable_assets` (159–165)
- `deprecated_emode_blocks_new_supply` (202–208)
- `deprecated_emode_blocks_new_borrow` (241)
- `deprecated_emode_allows_withdraw` (294–296)
- `emode_account_cannot_enter_isolation` (460)
- `emode_isolation_mutual_exclusion_after_supply` (484–485) — **the P1b rewrite**
- `emode_supply_sanity` (510)
- `emode_borrow_sanity` (523)
- `isolation_debt_ceiling_respected` (135) — **the P1b rewrite**
- `isolation_repay_decreases_counter` (179) — **the P1b rewrite**

**Cost shape:**
- Each rule now expands the full `process_supply` / `borrow_batch` / `process_repay` control-flow graph + pool-client RPC + oracle path + `ControllerCache::flush_isolated_debts` (which iterates a `Map`). The "must revert" rules (1–5, 7, 11) are slightly cheaper because the prover only needs to find one revert path to satisfy `cvlr_satisfy!(false)` is unreachable — but they still drag the entire reachable-state graph in front of the gate.
- The new "after-supply" mutual-exclusion rule (`emode_isolation_mutual_exclusion_after_supply`) is the worst case: the post-state assertion forces the prover to fully model the success path, including the pool's nondet return, the cache's nondet flush, and the storage write. Expect 5–10× the runtime of the equivalent read-only invariant.
- `isolation_debt_ceiling_respected` and `isolation_repay_decreases_counter` go through `Controller::borrow` / `Controller::repay` (not `_batch` directly), so they additionally drag `caller.require_auth()`, `validation::require_not_flash_loaning`, and the `ControllerCache::new_with_disabled_market_price` path in `repay`.

**Why P1b's change is right in spirit but wrong on cost:** the inductive shape is the only way to actually verify mutual exclusion. But the action-focused rule is unaffordable until the pool summaries are wired — at which point the prover sees `supply_summary` / `borrow_summary` / `repay_summary` returning a tightly-bounded `PoolPositionMutation` and the rest of the call collapses. The rules cannot be made cheaper by tightening preconditions; the cost lives downstream of the entry point.

**Fix:**
1. Apply `summarized!(supply_summary, ...)` (and the other four) at each `LiquidityPoolClient` callsite. The macro indirection in `controller/src/lib.rs:13-22` is already in place — only the wrappers at the production sites are missing.
2. Until (1) lands, the action-focused rules in this domain are paying for verification they cannot afford. Consider gating them behind a separate `cargo certora --action-focused` profile so the read-only invariants in this file can run cheaply against the prover budget.

---

## F2 (HIGH): `emode_remove_category` walks an unbounded `Map<Address, EModeAssetConfig>` symbolically

**Location:** `emode_rules.rs:387-417`.

The rewrite verifies the load-bearing post-conditions of the slim-storage refactor (good) but does so by:
1. Reading `crate::storage::get_emode_assets(&e, category_id)` — the full side map (line 393).
2. `members_before.keys().get(0)` (line 395) — pins to the first asset; the prover has to pick a concrete index, which forces it to model the map's internal ordering.
3. `crate::config::remove_e_mode_category(&e, category_id)` (line 403) — walks every member, doing 1 read + 1 write per member to clear the reverse index (`config.rs:283-300`), plus an additional `try_get_market_config` + potential `set_market_config` when the asset's reverse index becomes empty.
4. Re-reads the side map after removal (`members_after.is_empty()` at 411) — another full read.

The map is not bounded. Soroban `Map`s the prover has to reason about as `BytesN`-keyed inner storage, and `keys().get(0)` plus `is_empty()` quantifies over the whole inner shape. Without a `cvlr_assume!(members_before.len() <= K)` for some small `K` (3–4), this rule almost certainly TAC-blows or spends >10 minutes per run.

**Cost shape (worst case):**
- 1 storage read for the side-map (line 393).
- 1 storage read for the reverse index of the sampled asset (396).
- The `remove_e_mode_category` body iterates `members.keys()` — for a map of size N, that's N reads + up to 2N writes, plus N `try_get_market_config` reads. Each iteration is a separate `storage::*` call the prover has to model.
- 1 storage read for the post-state side map (411).
- 1 storage read for the post-state reverse index (415).
- 1 storage read for the post-state category (`get_emode_category` at 406).

**Mitigation already in place (good):** the rule pins to `sample_asset` at line 395 rather than universally quantifying. That's the right idiom — "show me ONE member whose reverse index was cleared" rather than "for ALL members, the reverse index was cleared." Universal quantification would be even worse.

**Risk:** if `members_before.is_empty()` is the only branch the prover finds satisfiable (because constructing a non-empty `Map<Address, EModeAssetConfig>` symbolically is expensive), the assumption at line 394 makes the rule vacuous — the assertion never runs.

**Fix:**
1. Add `cvlr_assume!(members_before.len() <= 3);` after line 393. The production semantics do not depend on map size; capping it at 3 makes the rule prover-friendly without weakening the property.
2. Consider splitting the rule into two: one rule sized to `len() == 1` (fast path), another to `len() == 2` or 3 (covers the iteration). The iteration-correctness post-condition (`for every member, reverse index cleared`) is what's load-bearing; the size-1 case verifies the single-asset case and the size-2 case verifies the loop.
3. The `e_mode_enabled` post-condition from the production source (`config.rs:292-299`) is **not asserted** at all in the current rule. After deprecation, if the sampled asset's reverse index becomes empty, `market.asset_config.e_mode_enabled` should be `false`. The audit's prior review (`audit/certora-review/05-emode-isolation.md:127-140`) called this out and provided a fix snippet — it has not been applied. This is a coverage gap independent of efficiency.

---

## F3 (HIGH): `emode_isolation_mutual_exclusion_after_supply` — sound but the cheapest possible inductive form is much heavier than necessary

**Location:** `emode_rules.rs:471-490`.

The rule is correctly framed: havoc inputs, run `process_supply`, assert `!(is_isolated && e_mode_category_id > 0)` on the post-state. Compare against the prior read-only rule (which read havoced storage and was vacuous): that's a real correctness improvement.

But the rule does not constrain the *kind* of supply being verified. The prover explores:
- New-account creation (`account_id == 0`, `create_account_for_first_asset` path).
- Existing-account supply (`account_id > 0`, meta read path).
- Single-asset and multi-asset payloads.
- Isolated-asset and non-isolated-asset first payment.
- Deprecated and non-deprecated e-mode category.

Each branch is a separate sub-graph. The invariant only depends on **the writes to AccountMeta**, which happen in exactly two places:
- `utils::create_account_for_first_asset` (only for `account_id == 0`).
- Nowhere else — supply doesn't mutate `is_isolated` or `e_mode_category_id` for an existing account (`supply.rs:99-101` writes only `set_supply_positions`).

So the rule could focus on the new-account branch (`cvlr_assume!(account_id == 0)`), exercising the only writer. The existing-account branch is verified by a separate read-only invariant ("if account already had `!isolated || category == 0` and supply doesn't write meta, the property is preserved").

**Cost shape:** without `account_id == 0` precondition, the rule pays for 2 paths through `process_supply` (one for create, one for load), with the load path doing a full `storage::get_account_meta` + `get_supply_positions` reconstruction that adds nothing to the proof.

**Fix:**
1. Add `cvlr_assume!(account_id == 0);` to focus on the create-new branch — that's the only branch where AccountMeta is written.
2. Add a sibling read-only rule that asserts: "If an existing account is reached, supply never modifies `is_isolated` or `e_mode_category_id`." This is structurally cheap: read meta before, read meta after, assert equality.

**Coverage gap:** there's no rule for `Controller::multiply` — strategies also create accounts (`process_multiply` → `create_account_for_first_asset` indirectly via the supply path). The mutual-exclusion invariant should hold there too, but P1b only inducted the supply entry point.

---

## F4 (MEDIUM): `isolation_debt_ceiling_respected` — sound after rewrite, but post-condition is still weak

**Location:** `isolation_rules.rs:113-141`.

The rewrite is in the right direction: read raw `meta` (not the shim that defaults `isolated_asset` to `owner`), require `isolated_asset.is_some()`, exercise the public `Controller::borrow` path via `compat::borrow_single`. That fixes the main correctness issue from the prior review.

**Remaining issues:**

(a) **Vacuity on revert (carried over):** `cvlr_assert!(current_debt <= ...)` after the borrow only runs if the borrow did NOT revert. If the borrow reverts (e.g., `DebtCeilingReached`), the assertion is vacuous. A regression that lets `handle_isolated_debt` *not* update the counter and silently succeed past the gate would leave `current_debt` at its old value (which was already `<= ceiling`) and the rule passes. The production gate panics at `borrow.rs:332-334` BEFORE writing `set_isolated_debt`; the post-write state always satisfies `<=`. This rule is structurally weak.

(b) **Counter-update verification missing:** The rule does not assert `current_debt > pre_borrow_debt`. A regression where `handle_isolated_debt` is no-op'd entirely (e.g., the `account.is_isolated` early-return at `borrow.rs:313-315` is broadened) would let the borrow proceed without updating the counter — the post-condition `current_debt <= ceiling` is still satisfied (because `current_debt` is the unchanged pre-state). The rule would pass.

(c) **No "must-revert" sibling:** the production gate's load-bearing semantics are the **revert** path (`new_debt > ceiling → panic`). There's no rule of the form "if `pre_debt + amount_usd_wad > ceiling`, the borrow must revert." Without it, the asymmetric assertion only catches half the bugs — the half where the success path is wrong, not the half where the revert path is wrong.

(d) **F1 cost cascade:** `compat::borrow_single` calls `Controller::borrow` which traverses the full pool-borrow path with no summary applied. Drag from the unsummarized `pool::borrow` adds substantial TAC cost.

**Fix:**
1. Capture `let debt_before = storage::get_isolated_debt(&e, &isolated_asset);` before the borrow, then assert both `debt_after <= ceiling` AND `debt_after > debt_before` (the counter actually moved). The "moved" assertion converts a vacuous-on-revert rule into one that requires the success path.
2. Add a sibling rule for the revert branch: precondition `debt_before + amount_usd_wad > ceiling` (with `amount_usd_wad` reconstructed via the same path the production uses), call `borrow_single`, assert `cvlr_satisfy!(false)` post-call.
3. F1 fix unblocks the cost.

---

## F5 (MEDIUM): `isolation_repay_decreases_counter` — strict-decrease assertion is unsound under WAD dust-floor

**Location:** `isolation_rules.rs:151-183`.

The new rule asserts `debt_after < debt_before` after a positive repay. Two soundness issues:

(a) **Dust floor in `adjust_isolated_debt_usd`:** Per the prior review (`audit/certora-review/05-emode-isolation.md:231` and the production source at `utils.rs:181-183`), repay applies a dust-floor: when the new debt is below `WAD`, it gets snapped to `0`. That makes `<` strict — but what if the repay amount is so small that USD-WAD value is `0` (sub-WAD repay on a low-decimal asset)? The counter does not move at all in that case. The rule's `cvlr_assume!(debt_before > 0)` does not exclude this.

Concretely: repay of 1 unit of a 7-decimal asset at price 1e-12 USD/unit produces an `amount_in_usd_wad` of 0 (rounded down). Production: `current_debt - 0 = current_debt`. Rule assertion: `debt_after < debt_before` ⟹ FAIL. This is a spurious counter-example, not a real bug. The rule will fail prover runs on the legitimate edge case unless the precondition forces `amount_in_usd_wad > 0`.

(b) **`scaled_amount_ray > 0` doesn't imply repaid amount converts to USD-WAD > 0.** The production path is `amount_repaid (asset units) → Wad::from_token(amount, decimals) * price → raw USD-WAD`. For tiny repay amounts on high-decimal/low-price assets, the USD-WAD value rounds to 0.

(c) **F1 cost cascade:** same issue as F4 — `compat::repay_single` traverses the unsummarized pool-repay path.

**Fix:**
1. Weaken the assertion to `debt_after <= debt_before` (monotone non-increasing) — this is the actual production guarantee.
2. Add a sibling rule asserting *strict* decrease, but with a stronger precondition: e.g., `cvlr_assume!(amount * feed.price_wad / asset_decimals_factor > WAD);` — the repaid USD value must be at least 1 WAD. That's an over-approximation but rules out the dust-floor edge case.
3. Or model the dust floor explicitly: assert `debt_after < debt_before || (debt_before < WAD && debt_after == 0)`.

---

## F6 (MEDIUM): `emode_account_cannot_enter_isolation` — calls `process_supply` but the panic fires before any heavy work, wastefully expensive

**Location:** `emode_rules.rs:441-464`.

This rule constructs a new account with a non-zero e-mode category and an isolated asset, calls `process_supply`, and expects revert. Production traces:
1. `caller.require_auth()` — cheap.
2. `validation::require_not_flash_loaning` — cheap storage read.
3. `resolve_supply_account` → `create_account_for_first_asset` — sets `is_isolated = true, e_mode_category_id = e_mode_category` on the freshly-built `Account`.
4. `prepare_deposit_plan` calls `ensure_e_mode_compatible_with_asset` (`emode.rs:47-51`) which panics with `EModeWithIsolated`.

The panic is at step 4. Steps 1–3 are wasted work for the prover. But because the rule routes through the full `process_supply` entrypoint, the prover models all of it: `ControllerCache::new`, the `aggregate_positive_payments` map iteration, the `validate_bulk_position_limits` storage read, the `validate_bulk_isolation` cache miss, etc.

This is the cheapest acceptable form of the rule — the panic IS exercised on the production path — but a more efficient rule would call `ensure_e_mode_compatible_with_asset` directly: 1 line, no entry point, no cache. The trade-off: calling the helper directly does not verify that the helper is *invoked* by the supply path. So both forms have value; the current one is more thorough but costly.

**Fix:**
1. Keep the current rule as a "the supply path actually invokes the gate" coverage check.
2. Add a sibling unit-style rule that exercises `ensure_e_mode_compatible_with_asset` directly: precondition `asset_config.is_isolated_asset && e_mode_id > 0`, expect panic. That's a 5-line rule with zero downstream cost and verifies the helper itself.

---

## F7 (LOW): `emode_overrides_asset_params` is correctly scoped post-P1b — model for the rest

**Location:** `emode_rules.rs:312-351`.

P1b added `cvlr_assume!(!category.is_deprecated)` (line 322), fixing the unsoundness called out in the prior review. The rule now:
1. Reads `EModeCategory` (1 storage read).
2. Reads `get_emode_asset` (1 storage read for the side map).
3. Reads `get_asset_emodes` (1 storage read).
4. Reads `MarketConfig.asset_config` (1 storage read).
5. Calls `apply_e_mode_to_asset_config` directly — no cache, no pool, no oracle.
6. Asserts 5 field equalities.

Total: 4 storage reads + 1 pure-function call + 5 asserts. **This is the cheapest inductive shape for a parameter-override rule and should be the template for similar rules in this file.** No findings here.

**Coverage gap from prior review still open:** there's no sibling rule for `effective_asset_config(category_id == 0) == base_config` and no sibling for `effective_asset_config` returning base config when the category is deprecated. Both should be 5-line rules with the same cost shape as the current rule. The audit at `audit/certora-review/05-emode-isolation.md:107-108` proposed them; they weren't added.

---

## F8 (LOW): Sanity rules `emode_supply_sanity` / `emode_borrow_sanity` (496-525) — vacuously satisfied, low value

These are reachability checks. Each calls the entry point and asserts `cvlr_satisfy!(true)`. With pool summaries unwired (F1), the call's success branch is freely chosen by the prover via havoc — `satisfy(true)` is satisfied trivially. They serve as smoke tests that the rule file compiles and the entry point is reachable, not as verification.

**Cost:** moderate — same pool-call cost as the action-focused rules, but the assertion adds nothing.

**Fix:** either delete (the action-focused rules already exercise the entry points) or strengthen to assert a non-trivial post-condition (e.g., `acct_id > 0` after a successful supply).

---

## F9 (LOW): `deprecated_emode_allows_withdraw` — vacuous post-condition (carry-over from prior review)

**Location:** `emode_rules.rs:255-300`.

`cvlr_satisfy!(true)` after `process_withdraw` is satisfied by *any* reachable state — including states where the withdraw reverted. To verify "withdraw must succeed in deprecated category," the post-condition needs to require the withdraw actually completed (e.g., position scaled-amount decreased, or the position was removed entirely). The prior review called this out (`audit/certora-review/05-emode-isolation.md:93-96`); it has not been fixed.

This is also expensive — the rule calls `process_withdraw`, which traverses the same heavy path as supply/borrow.

**Fix:** capture `pos.scaled_amount_ray` before the call, assert `new_pos.scaled_amount_ray < pos.scaled_amount_ray` (or `new_pos.is_none()` for a full withdraw) after.

---

## F10 (LOW): Duplicate mutual-exclusion rule in `isolation_rules.rs` deleted, but reachability sanity duplicates remain

The prior review flagged a near-duplicate between `emode_isolation_mutual_exclusion_invariant` and `isolation_emode_exclusive`. P1b correctly deleted the read-only forms (a comment at `isolation_rules.rs:67-73` documents the move to `emode_rules.rs::emode_isolation_mutual_exclusion_after_supply`). Good.

But `isolation_rules.rs:189-198` still has two sanity rules (`isolation_sanity`, `emode_sanity`) that read storage and call `cvlr_satisfy!(...)`. Same low-value-vacuous pattern as F8. These are cheap (no entry point, just storage read), so the cost is small — but they remain decorative.

---

## F11 (LOW): `emode_add_asset_to_deprecated_category` (419-427) — does not assume category exists

**Location:** `emode_rules.rs:419-427`.

The rule expects revert when adding an asset to a deprecated category. But it does not `cvlr_assume!(storage::try_get_emode_category(&e, category_id).is_some())`. If the prover picks a non-existent `category_id`, `add_asset_to_e_mode_category` reverts with `EModeCategoryNotFound` (`config.rs:317-318`) — different reason, same outcome. The rule passes for the wrong reason on that branch.

**Cost:** rule itself is lightweight (single config call, no entry point). The fix is one extra `cvlr_assume!` line.

**Fix:** `cvlr_assume!(storage::get_emode_category(&e, category_id).is_deprecated);` at the top.

---

## What P1b got right

1. **Mutual exclusion is now inductive.** The previous form read havoced storage and was vacuous; the new form forces a write through `process_supply`. This is a real correctness gain even with F1's cost issue.
2. **`isolation_debt_ceiling_respected` reads raw meta.** The prior shim defaulted `isolated_asset` to `owner`; the new form correctly bails out on `meta.isolated_asset.is_some()` (line 129-130). That fixes a soundness issue.
3. **`isolation_repay_decreases_counter` is a needed coverage addition.** The prior review explicitly called out monotonicity-under-repay as a missing rule.
4. **`emode_overrides_asset_params` deprecated-category fix.** Single-line fix that resolves the prior-review HIGH finding.
5. **`emode_remove_category` actually verifies side-map cleanup (post-condition 2) and reverse index (post-condition 3).** That's the load-bearing piece of the storage refactor that was unverified.

---

## What P1b missed

1. **No pool/SAC summaries wired** (F1) — the action-focused rule rewrites pay 5–10× cost they could have avoided. This is the single largest efficiency item across the file.
2. **`emode_remove_category` does not assert `e_mode_enabled` flag clearing** (`config.rs:292-299`). The slim-storage refactor explicitly cited this as a guarantee; it's still unverified.
3. **No bound on `EModeAssets` side-map size** in `emode_remove_category`. Likely TAC-blow on first run unless capped.
4. **`isolation_debt_ceiling_respected` still vacuous on revert** — the rewrite improved the precondition but did not strengthen the post-condition with a `debt_after > debt_before` movement check.
5. **`isolation_repay_decreases_counter` strict-decrease assertion is wrong on dust-floor edge.** Should be `<=` with strict-decrease as a sibling under stronger preconditions.
6. **No multiply-path mutual-exclusion rule.** `process_multiply` also creates accounts; the inductive invariant doesn't cover that entry point.
7. **Coverage gaps from prior review still open**: bulk-isolation rejection, second-collateral rejection, `effective_asset_config(category=0)`, `effective_asset_config(deprecated)`, `add_asset_to_e_mode_category` rejecting isolated assets, `is_collateralizable`/`is_borrowable` flag override semantics.

---

## Per-rule cost rubric

| Rule | Lines | Entry pt | Storage reads | Pool calls | Cost class | Notes |
|---|---|---|---|---|---|---|
| `emode_only_registered_assets` | 28-59 | `process_supply` | 2 + cache | 1 (havoc) | HIGH | F1; correct invariant |
| `emode_borrow_only_registered_assets` | 68-93 | `borrow_batch` | 3 + cache | 1 (havoc) | HIGH | F1 |
| `emode_only_borrowable_assets` | 101-128 | `borrow_batch` | 3 + cache | 1 (havoc) | HIGH | F1; reaches gate via `validate_e_mode_asset` |
| `emode_only_collateralizable_assets` | 137-169 | `process_supply` | 3 + cache | 1 (havoc) | HIGH | F1 |
| `deprecated_emode_blocks_new_supply` | 183-212 | `process_supply` | 2 + cache | 1 (havoc) | HIGH | F1; reverts at `active_e_mode_category` |
| `deprecated_emode_blocks_new_borrow` | 222-245 | `borrow_batch` | 2 + cache | 1 (havoc) | HIGH | F1 |
| `deprecated_emode_allows_withdraw` | 255-300 | `process_withdraw` | 4 + cache | 1 (havoc) | HIGH | F1, F9 vacuous post-cond |
| `emode_overrides_asset_params` | 312-351 | none (direct) | 4 | 0 | LOW | model rule |
| `emode_category_has_valid_params` | 365-372 | none (direct) | 1 | 0 | LOW | clean |
| `emode_remove_category` | 388-417 | `remove_e_mode_category` | unbounded! | 0 | **VERY HIGH** | F2 — needs size cap |
| `emode_add_asset_to_deprecated_category` | 421-427 | direct config call | 1 | 0 | LOW | F11 |
| `emode_account_cannot_enter_isolation` | 441-464 | `process_supply` | 1 + cache | 1 (havoc) | HIGH | F1, F6 |
| `emode_isolation_mutual_exclusion_after_supply` | 471-490 | `process_supply` | 1 + cache | 1 (havoc) | HIGH | F1, F3 — P1b rewrite |
| `emode_supply_sanity` | 497-512 | `process_supply` | 0 + cache | 1 (havoc) | HIGH | F1, F8 vacuous |
| `emode_borrow_sanity` | 515-525 | `borrow_batch` | 1 + cache | 1 (havoc) | HIGH | F1, F8 vacuous |
| `ltv_less_than_liquidation_threshold` | 24-29 | none | 1 (+ pool sync_data via shim) | 1 read | MEDIUM | shim reads sync_data unnecessarily |
| `liquidation_bonus_capped` | 36-40 | none | 1 (+ pool sync) | 1 read | MEDIUM | same |
| `reserve_factor_bounded` | 47-51 | none | 1 (+ pool sync) | 1 read | MEDIUM | same |
| `utilization_params_ordered` | 58-64 | none | 1 (pool sync) | 1 read | MEDIUM | uses `get_market_params` shim |
| `isolated_single_collateral` | 80-102 | none | 2 (meta + supply map) | 0 | LOW | clean |
| `isolation_debt_ceiling_respected` | 113-141 | `Controller::borrow` | 3 + cache | 1 (havoc) | HIGH | F1, F4 — P1b rewrite |
| `isolation_repay_decreases_counter` | 150-183 | `Controller::repay` | 5 + cache | 1 (havoc) | HIGH | F1, F5 — P1b rewrite |
| `isolation_sanity` / `emode_sanity` | 189-198 | none | 1 each | 0 | LOW | F10 vacuous |

---

## Note on `storage::asset_config::get_asset_config` shim cost

`isolation_rules.rs::ltv_less_than_liquidation_threshold` (line 26) and the next 3 rules go through `storage::asset_config::get_asset_config` (`storage/certora.rs:77-99`). That shim does:
1. `get_market_config` (1 storage read).
2. `LiquidityPoolClient::new(env, &market.pool_address).get_sync_data()` (1 cross-contract call — havoc unless `get_sync_data_summary` is wired).

`get_sync_data_summary` is defined at `summaries/pool.rs:343-391` but, like the other pool summaries, **not wired** at the production callsite (`pool/src/lib.rs::get_sync_data`). So every read of `reserve_factor_bps` via this shim drags an unsummarized cross-contract call.

The 3 read-only invariants (`liquidation_bonus_capped`, `ltv_less_than_liquidation_threshold`, `reserve_factor_bounded`) only need fields from `MarketConfig.asset_config`. They could read `storage::get_market_config` directly and skip `get_sync_data` entirely. Only `reserve_factor_bounded` needs `reserve_factor_bps` — which lives in `MarketParams`, fetched via `get_sync_data` — so that one rule must traverse the pool. The other 2 are paying cost they don't need.

**Fix:** split the shim — one variant returns `AssetConfig` directly without touching the pool, one variant returns the merged shape only when needed. Rules that just need `loan_to_value_bps` / `liquidation_threshold_bps` / `liquidation_bonus_bps` use the cheap variant.

---

## Recommended action items (prioritized)

1. **HIGH — Wire pool summaries** at every `LiquidityPoolClient` callsite (`supply.rs:370`, `borrow.rs:55`, `borrow.rs:262`, `repay.rs:104`, plus the `get_sync_data` callsite in the certora shim). This unblocks F1 and reduces every action-focused rule's cost by 5–10×. Single-line `summarized!(supply_summary, ...)` wrappers — the macro is already in place.
2. **HIGH — Cap `EModeAssets` side-map size** in `emode_remove_category` via `cvlr_assume!(members_before.len() <= 3);`. Also augment with the `e_mode_enabled` flag-clearing post-condition.
3. **HIGH — `isolation_debt_ceiling_respected`**: capture `debt_before`, assert `debt_after > debt_before` AND `debt_after <= ceiling`. Add must-revert sibling for the over-ceiling branch.
4. **MEDIUM — `isolation_repay_decreases_counter`**: weaken to `<=` with strict-decrease as a sibling under stronger USD-WAD precondition.
5. **MEDIUM — `emode_isolation_mutual_exclusion_after_supply`**: scope to `account_id == 0` (only branch that writes meta); add multiply-path sibling.
6. **MEDIUM — `deprecated_emode_allows_withdraw`**: replace `cvlr_satisfy!(true)` with a real post-condition (position decreased).
7. **LOW — `emode_add_asset_to_deprecated_category`**: add existence assumption.
8. **LOW — Sanity rules**: either delete or strengthen with non-trivial post-conditions.
9. **LOW — Split `storage::asset_config::get_asset_config` shim** so rules that don't need `reserve_factor_bps` skip `get_sync_data`.

---

## Verdict

**P1b made the rules sounder but more expensive.** The action-focused rewrites (mutual exclusion, debt ceiling, repay monotonicity) verify the right invariants but trigger F1 — the unwired pool summaries — at every entry point, and the remediated `emode_remove_category` will likely TAC-blow without a side-map size cap. The correct prioritization is to wire pool summaries first; without that, the new rules in this domain are paying for cost they cannot afford to spend.

The non-rewritten parameter-override rule (`emode_overrides_asset_params`) is the model: 4 storage reads + 1 direct helper call + 5 asserts. The rewrites should converge toward that shape — direct helper exercise + read-only state checks — wherever the production semantics permit, and reserve entry-point traversal for invariants that genuinely require it (mutual exclusion, debt-counter movement). At the current ratio, half the rules in this file pay 10× cost for marginal coverage gain over a direct-helper rule.

**Score: 5/9.** Sound but expensive; coverage gaps from prior review still open; one rule (F2) likely to time out without intervention.
