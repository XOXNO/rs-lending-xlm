# Certora Review — Domain 05: E-Mode + Isolation

**Files in scope**
- `controller/certora/spec/emode_rules.rs` (482 lines)
- `controller/certora/spec/isolation_rules.rs` (158 lines)

**Production references**
- `controller/src/positions/emode.rs` — `apply_e_mode_to_asset_config`, `effective_asset_config`, `validate_e_mode_asset`, `validate_isolated_collateral`, `ensure_e_mode_compatible_with_asset`, `validate_e_mode_isolation_exclusion`
- `controller/src/storage/emode.rs` — slim e-mode storage (map-per-category refactor)
- `controller/src/config.rs` — `add_e_mode_category`, `remove_e_mode_category`, `add_asset_to_e_mode_category`
- `controller/src/positions/supply.rs` — `prepare_deposit_plan`, `validate_bulk_isolation`
- `controller/src/positions/borrow.rs` — `validate_borrow_asset_preflight`, `handle_isolated_debt`
- `controller/src/utils.rs::adjust_isolated_debt_usd`, `controller/src/positions/repay.rs::adjust_isolated_debt_for_repay`
- `controller/src/storage/certora.rs` — Certora-only storage shims (`get_account_attrs`, `accounts::get_account_data`)
- `common/src/types.rs` — `Account`, `AccountAttributes`, `AccountMeta`, `EModeCategory`, `EModeAssetConfig`

**Totals:**
- emode_rules.rs: rules=14 — high=2 medium=5 low=4 ok=3 (sanity)
- isolation_rules.rs: rules=9 — high=2 medium=3 low=2 ok=2 (sanity)

---

## Summary of findings

Two rule sets cover the right *invariants in spirit* — whitelist enforcement, deprecated-category gating, parameter overrides, mutual exclusion, debt-ceiling, single-collateral. The implementation has real holes:

1. **`emode_overrides_asset_params` is unsound for deprecated categories** — `apply_e_mode_to_asset_config` short-circuits when `category.is_deprecated`, so the post-condition `asset_config.loan_to_value_bps == category.loan_to_value_bps` is provably false in that branch. The rule is missing `cvlr_assume!(!category.is_deprecated)` (or, better, two assertions with a `category.is_deprecated` split).
2. **Bulk-isolation invariant is unverified.** `validate_bulk_isolation` (validation.rs:97-111) is a load-bearing pre-flight gate ("isolated account or isolated first asset MUST imply len <= 1"). No rule exercises it.
3. **`emode_remove_category` cannot prove the side-map cleanup** — the rule only asserts `is_deprecated == true` after removal. The whole point of the recent slim-storage refactor (storage/emode.rs:83-88, config.rs:282-301) is that `remove_e_mode_category` walks the side map and clears the reverse index for every member asset and clears `e_mode_enabled` on each market. None of that is asserted. A regression that left the reverse index dirty would be invisible to the rules.
4. **`emode_account_cannot_enter_isolation` does not reach the conflict path.** It tries to create an account via `process_supply` with `account_id = 0` and an isolated asset, but the production code (`utils::create_account_for_first_asset` at utils.rs:108-130 and `prepare_deposit_plan` at supply.rs:166-195) calls `ensure_e_mode_compatible_with_asset` *before* `is_isolated` is ever set on the account. So the rule does verify the panic exists, but for the wrong reason — it catches the asset-side e-mode gate (`EModeWithIsolated` panic at emode.rs:48-50), not the account-side flag conflict. There is no rule covering an attempt to *set* `is_isolated = true` on an account that already has an e-mode category.
5. **`isolation_debt_ceiling_respected` reads ceiling from the wrong asset.** It uses `account_data.isolated_asset` — sound in itself — but the production gate (`borrow.rs::handle_isolated_debt` at lines 320-334) reads `account.try_isolated_token()` and reverts with `InternalError` if `None` while `is_isolated == true`. The rule does not exercise the `None`-but-`is_isolated` corruption case. More importantly, the rule's `accounts::get_account_data` summary (storage/certora.rs:148) substitutes `meta.owner` when `meta.isolated_asset == None`, so a buggy meta where `is_isolated == true && isolated_asset == None` would silently route the rule to read ceiling from the *owner address* (which has no asset config) — masking the bug rather than catching it.
6. **Two rules duplicated under "Rule 3" / "Rule 6" headers** — `emode_only_borrowable_assets` and `emode_only_collateralizable_assets` both labeled Rule 3; `deprecated_emode_allows_withdraw` and `emode_overrides_asset_params` both labeled Rule 6. Cosmetic but adds confusion when triaging.
7. **`isolation_debt_ceiling_respected` runs `borrow_single` against a real cross-contract path** with no summary application — likely to TAC-blow up unless the global summaries auto-apply. The rule is also unsound: it asserts `current_debt <= ceiling` *after* a successful borrow but does not state that the call was supposed to succeed. The current Certora set will treat *any* path that reverts as vacuously satisfying the post-condition.
8. **Coverage gaps**: no rule for `effective_asset_config(category_id == 0) == base_config`; no rule that `is_collateralizable`/`is_borrowable` flags strictly tighten (or override) the base; no rule that isolated-debt counter is monotone-non-increasing under repay; no rule that `validate_isolated_collateral` rejects mixing on the second supply; no rule that the second collateral on an isolated account reverts.

---

## Per-rule findings — `controller/certora/spec/emode_rules.rs`

### `emode_only_registered_assets` (lines 28-59)

**Severity:** ok-ish (medium)
**Right invariant:** YES. Sound preconditions: `amount > 0`, `e_mode_category_id > 0`, `!asset_cats.contains(category_id)`. Calls `process_supply` and asserts the path is unreachable.
**Sound preconditions/postconditions:** Mostly sound, but the rule uses `cvlr_satisfy!(false)` after the call — meaning "if this point is reached, the rule fails." That is the correct revert-detection idiom.
**Summary use:** Reads `storage::get_account_attrs` (Certora shim, returns `AccountAttributes` from `AccountMeta`). Reads `storage::get_asset_emodes` (real reverse-index reader). Both match production semantics.
**Catches real bug:** YES — would catch a regression where `validate_e_mode_asset` skips the `asset_cats.contains` check.
**Tautology:** No.
**Issue:** The rule does NOT guard the `category` itself with `!is_deprecated`. If `e_mode_category_id` points to a deprecated category, `process_supply` panics earlier via `active_e_mode_category` (emode.rs:81-85), and the rule still passes — but for the wrong reason. The rule's intent is "asset-not-in-category causes revert", not "deprecated category causes revert". A regression that *only* breaks the asset-not-in-category check would be masked when the prover happens to choose a deprecated category id.
**Fix:** add `cvlr_assume!(!storage::get_emode_category(&e, attrs.e_mode_category_id).is_deprecated);` after line 40.

### `emode_borrow_only_registered_assets` (lines 68-93)

**Severity:** medium (same issue as above)
**Right invariant:** YES — borrow-side mirror of #1.
**Issue:** Same masking by deprecated-category as #1. Same fix.
**Catches real bug:** YES — a borrow-side regression of the asset/category check.

### `emode_only_borrowable_assets` (lines 101-128, labelled "Rule 3")

**Severity:** medium
**Right invariant:** YES — `validate_e_mode_asset(env, _, _, false)` panics with `AssetNotBorrowable` when `cfg.is_borrowable == false`.
**Sound preconditions:** `emode_config.is_some()` and `!cfg.is_borrowable`. The rule constrains the asset to be a member.
**Issue 1:** Rule label is "Rule 3" but `emode_only_collateralizable_assets` (line 137) is *also* labelled "Rule 3". Cosmetic only.
**Issue 2:** Same deprecated-category masking as #1.
**Issue 3:** The rule does not call `emode_only_borrowable_assets` against the production `borrow_batch` after `validate_e_mode_asset` confirms `is_borrowable == false`; it instead invokes `borrow_batch` directly. The path also goes through `validate_borrow_asset_preflight` (borrow.rs:138-154) which has redundant `is_borrowable` checks. The rule does not distinguish which gate fires; that's fine as long as one of them does, but it weakens the bug-catching to "some gate revert" rather than "the e-mode gate reverts."
**Catches real bug:** Catches a regression of either gate. Borderline acceptable.

### `emode_only_collateralizable_assets` (lines 137-169, labelled "Rule 3")

**Severity:** medium (same shape as above)
**Right invariant:** YES — supply-side mirror of #3.
**Issue:** Same as #3.

### `deprecated_emode_blocks_new_supply` (lines 183-212)

**Severity:** ok
**Right invariant:** YES. `active_e_mode_category` (emode.rs:81-85) → `ensure_e_mode_not_deprecated` panics with `EModeCategoryDeprecated`.
**Sound preconditions:** `category_id > 0`, `category.is_deprecated == true`.
**Catches real bug:** YES — regression in `ensure_e_mode_not_deprecated`. Solid.
**Tautology:** No.

### `deprecated_emode_blocks_new_borrow` (lines 222-245)

**Severity:** ok
**Right invariant:** YES — borrow mirror.
**Catches real bug:** YES.

### `deprecated_emode_allows_withdraw` (lines 255-300, labelled "Rule 6")

**Severity:** medium (label collision + unsound)
**Right invariant:** Wind-down semantics — withdraw must NOT be blocked by deprecation. The intent is correct.
**Issue 1:** Label says "Rule 6" but `emode_overrides_asset_params` is also "Rule 6".
**Issue 2:** The rule asserts `cvlr_satisfy!(true)` after the withdraw call. This is **vacuous**: `cvlr_satisfy!(true)` is satisfiable on any reachable path including ones where the call itself reverted. To meaningfully verify "withdraw must not revert here," the call should sit inside `cvlr_satisfy!(<post-condition that requires withdraw to have completed>)`, e.g. an assertion on a balance/index update, or use a different idiom.
**Issue 3:** The rule does not constrain the withdraw amount against `pos.scaled_amount_ray`. It only requires `pos.scaled_amount_ray > 0`. A withdraw that exceeds the position would revert in `withdraw::process_withdraw` — and the rule wouldn't notice (because `satisfy(true)` is satisfied by *any* state, including post-revert).
**Catches real bug:** NO — vacuous post-condition. A regression that adds a deprecation check to withdraw would still satisfy `satisfy(true)` because the rule does not need the path to reach it.
**Fix:** Replace `cvlr_satisfy!(true)` with a non-trivial post-condition that *requires* the withdraw to have completed. E.g., assert that the in-memory `position` map no longer contains `asset` or that `cache.get_isolated_debt(...)` decreased. Or invert the rule: build a state where deprecation is set and withdraw should succeed, then read the position post-withdraw and `cvlr_assert!(new_pos.scaled_amount_ray < pos.scaled_amount_ray)`.

### `emode_overrides_asset_params` (lines 312-346, labelled "Rule 6")

**Severity:** **HIGH — unsound, will fail prover**
**Right invariant:** Half-right. The override is supposed to apply LTV/threshold/bonus from the category and `is_collateralizable`/`is_borrowable` from the asset config. But `apply_e_mode_to_asset_config` (emode.rs:20-29) explicitly *no-ops* when `cat.is_deprecated == true`.
**Issue 1 (unsoundness):** The rule does not assume `!category.is_deprecated`. When the prover instantiates a deprecated category, `apply_e_mode_to_asset_config` returns without writing, and the asserts on lines 338-345 fail because `asset_config` still holds base-config values.
**Issue 2:** The rule does not exercise the production *path* for override application. It calls `apply_e_mode_to_asset_config` directly. The whole point of `effective_asset_config` (emode.rs:33-44) is that it composes `cached_asset_config` + `cached_emode_asset` + `apply_e_mode_to_asset_config`. A regression that breaks `effective_asset_config`'s call site (e.g., wrong category passed) would be invisible.
**Catches real bug:** Partially. Catches a regression in `apply_e_mode_to_asset_config` itself; misses regressions in callers.
**Fix:**
1. Add `cvlr_assume!(!category.is_deprecated);` after line 317.
2. Add a sibling rule: `effective_asset_config_returns_base_when_category_zero` — set `account.e_mode_category_id = 0`, call `effective_asset_config`, assert it equals the base config byte-for-byte.
3. Add a sibling rule: `effective_asset_config_returns_base_when_deprecated` — set category as deprecated, call `effective_asset_config`, assert it equals the base config (this is the only path the production code takes for stale e-mode accounts after `remove_e_mode_category` runs).

### `emode_category_has_valid_params` (lines 360-367)

**Severity:** ok
**Right invariant:** YES — invariant from `add_e_mode_category` (config.rs:232) and `edit_e_mode_category` (config.rs:255): `threshold > ltv`. The rule guards with `!is_deprecated`. Note: deprecated categories may have any LTV/threshold (they were validated at create time but never re-validated; this rule is conservative).
**Catches real bug:** YES — config regression that allowed `threshold == ltv` or `threshold < ltv`.
**Tautology:** No (depends on storage state).

### `emode_remove_category` (lines 378-385)

**Severity:** **HIGH — incomplete coverage of refactored path**
**Right invariant:** Partial. After `remove_e_mode_category`, `is_deprecated == true` is asserted. But the production function (config.rs:271-304) does **much more**:
1. Walks the side map (`storage::get_emode_assets`).
2. For each member asset, removes `category_id` from the reverse index `AssetEModes(asset)`.
3. If the asset's reverse index becomes empty, clears `e_mode_enabled` on the market config.
4. Drops the entire side-map ledger entry via `storage::remove_emode_assets`.
**Issue:** None of (2)–(4) are asserted. The recent refactor (per the file header on storage/emode.rs:30-36) explicitly cited "single storage op instead of N orphan per-pair entries" as the goal. A regression that re-introduces orphan reverse-index entries would not be caught.
**Catches real bug:** Misses the load-bearing post-conditions of the slim-storage refactor.
**Fix:** Augment the rule:
```rust
// Pick an asset that was a member, assume it was in the category before.
cvlr_assume!(asset_cats_before.contains(category_id));
crate::config::remove_e_mode_category(&e, category_id);
let asset_cats_after = storage::get_asset_emodes(&e, &asset);
cvlr_assert!(!asset_cats_after.contains(category_id));
// And if asset belonged to no other category, e_mode_enabled is now false.
if asset_cats_after.is_empty() {
    cvlr_assert!(!storage::get_market_config(&e, &asset).asset_config.e_mode_enabled);
}
// And the side-map ledger entry is gone.
cvlr_assert!(storage::get_emode_assets(&e, category_id).is_empty());
```

### `emode_add_asset_to_deprecated_category` (lines 388-395)

**Severity:** ok
**Right invariant:** YES — `add_asset_to_e_mode_category` (config.rs:317-321) panics with `EModeCategoryDeprecated`.
**Issue:** The rule does not assume `category_id > 0` or that the category exists. Without the existence assumption, `try_get_emode_category` returns `None` and panics with `EModeCategoryNotFound` — which still satisfies the "must revert" intent but for the wrong reason. Tighten with `cvlr_assume!(storage::get_emode_category(&e, category_id).is_deprecated);`.

### `emode_account_cannot_enter_isolation` (lines 409-432)

**Severity:** medium (mis-targeted)
**Right invariant:** Intent is correct — both flags cannot be set on a single account.
**Issue:** The rule constructs an account via `process_supply(_, _, 0, e_mode_category, [(asset, amount)])` where `is_isolated_asset == true`. In production:
1. `resolve_supply_account` calls `create_account_for_first_asset` (utils.rs:108-130), which writes `is_isolated = true, e_mode_category_id = e_mode_category` to the meta — both set simultaneously.
2. `prepare_deposit_plan` then calls `ensure_e_mode_compatible_with_asset` (supply.rs:187), which panics with `EModeWithIsolated` because `asset_config.is_isolated_asset && e_mode_id > 0`.
   So the panic *does* fire — but it fires on the **asset gate**, not the account flag conflict. There is currently NO rule covering "an account already has e-mode-only state and someone tries to flip `is_isolated` on it" because the protocol does not have such a transition (account flags are immutable post-creation). That's actually a strong design property — but the rule should make that explicit, not pretend to catch a non-existent transition.
**Catches real bug:** Catches a regression in `ensure_e_mode_compatible_with_asset`. Does NOT catch a regression that leaves an account with both flags set due to corrupted state restoration or migration.
**Fix:** Either (a) rename the rule to `emode_isolated_asset_rejected_at_creation` and add a comment that account flags are immutable; or (b) add a sibling invariant rule that, given `account.has_emode() && asset_config.is_isolated_asset`, asserts no entry-point can produce a stored `AccountMeta` with both flags set. The latter is closer to the actual invariant.

### `emode_isolation_mutual_exclusion_invariant` (lines 437-447)

**Severity:** **HIGH — vacuous as a state invariant**
**Right invariant:** Intent: post-condition over storage state — "no account stored has both flags."
**Issue:** This rule reads storage via `get_account_attrs` and asserts a relationship. Without an `init_storage` hook or a precondition that the account was reached by the protocol's entry points (and not by an `havoc storage` step), the prover will assume the storage is arbitrary. With arbitrary storage, the assertion fails trivially because the prover can choose a meta with both flags set. The rule probably passes today only because Certora's default for `nondet_storage` may not havoc structured persistent entries — but treating that as a verification is fragile.
**Catches real bug:** Only if storage is constrained to "what entry points can produce". In the absence of such an `invariant`-style framing, the rule is either vacuous or fails on havoc.
**Fix:** Convert to an inductive invariant pattern: assume the property holds before *any* protocol entry point, run the entry point, assert it still holds after. The `validate_e_mode_isolation_exclusion` helper (emode.rs:158-162) and `ensure_e_mode_compatible_with_asset` (emode.rs:47-51) are the only places that gate it. There is no other writer of `is_isolated` or `e_mode_category_id` post-creation.

### `emode_supply_sanity` / `emode_borrow_sanity` (lines 453-482)

**Severity:** ok (sanity)
**Issue:** Reachability checks. Useful as smoke. They pass `cvlr_satisfy!(true)` after the call, which is vacuous on its own — but for sanity rules that's the standard idiom (you only care that *some* path reaches the end). Keep.

---

## Per-rule findings — `controller/certora/spec/isolation_rules.rs`

### `ltv_less_than_liquidation_threshold` (lines 24-29)

**Severity:** ok
**Right invariant:** YES — `validate_asset_config` (validation.rs:208-212) enforces `liquidation_threshold > loan_to_value`.
**Issue:** Rule reads from a *Certora-shim* (`storage::asset_config::get_asset_config`) which routes through the pool's `get_sync_data()` to fetch `reserve_factor_bps` (storage/certora.rs:79). For LTV/threshold the shim mirrors production reads; sound.
**Catches real bug:** YES — regression in `validate_asset_config`.
**Tautology:** No.

### `liquidation_bonus_capped` (lines 36-40)

**Severity:** ok
**Right invariant:** YES (`MAX_LIQUIDATION_BONUS = 1500 BPS`, validation.rs:214).
**Catches real bug:** YES.

### `reserve_factor_bounded` (lines 47-51)

**Severity:** ok
**Right invariant:** YES (validation.rs:193).
**Issue:** The `< 10000` constant should reference `BPS` for consistency with production. Cosmetic.

### `utilization_params_ordered` (lines 58-64)

**Severity:** ok
**Right invariant:** YES — `validate_interest_rate_model` (validation.rs:184-192) enforces `mid > 0`, `optimal > mid`, `optimal < RAY`.
**Catches real bug:** YES.

### `isolation_emode_exclusive` (lines 72-84)

**Severity:** **HIGH — same vacuity as `emode_isolation_mutual_exclusion_invariant`**
**Right invariant:** Same intent as the emode-side rule.
**Issue:** Same vacuity issue — read-only over storage, no inductive framing. With `accounts::get_account_data` reading from arbitrary `AccountMeta` storage, the prover can pick a meta with both flags set unless storage is constrained. Furthermore, this rule is a near-duplicate of `emode_isolation_mutual_exclusion_invariant` (emode_rules.rs:437-447) — they read the same field via different shims (`get_account_attrs` vs `get_account_data`). One file should own this invariant, not both.
**Fix:** Same as #emode-13. Plus: deduplicate by deleting one of the two.

### `isolated_single_collateral` (lines 90-113)

**Severity:** medium
**Right invariant:** YES — isolated account holds at most one collateral, and if it has one the asset matches `account.isolated_asset`.
**Issue 1:** Same vacuity issue as the mutual-exclusion rules — reads from arbitrary storage. The prover can choose `is_isolated == true` with two deposits in `SupplyPositions` storage; nothing in the rule prevents that.
**Issue 2:** The rule reads `account_data.isolated_asset` — but `accounts::get_account_data` (storage/certora.rs:148) silently substitutes `meta.owner` when `meta.isolated_asset == None`. If a buggy meta has `is_isolated == true && isolated_asset == None && supply_positions = {asset_X => ...}`, the rule will assert `asset_X == owner_address` — which fails (owner is not an asset address), so it actually *catches* this bug by accident. But the failure mode looks like a generic assert mismatch, not a clear "isolated asset is None" signal.
**Catches real bug:** YES — but the diagnosis is muddied by the shim's `unwrap_or_else(|| meta.owner)` fallback.
**Fix:** (a) Convert to inductive invariant. (b) Update `accounts::get_account_data` to return `Option<Address>` for `isolated_asset`, and have the rule assert `isolated_asset.is_some()` separately.

### `isolation_debt_ceiling_respected` (lines 122-142)

**Severity:** **HIGH — unsound, weak post-condition**
**Right invariant:** Intent: "after a borrow on isolated account, debt <= ceiling." Production gate at `borrow.rs::handle_isolated_debt` (lines 332-334) panics with `DebtCeilingReached` when `new_debt > ceiling`.
**Issue 1 (vacuity on revert):** The post-condition `cvlr_assert!(current_debt <= isolated_config.isolation_debt_ceiling_usd_wad)` runs only if `borrow_single` did NOT revert. If the borrow reverted, the rule is vacuously true. A regression that lets a borrow succeed *and* leaves debt above ceiling is the only real bug; a regression that mis-reads the ceiling or lets the borrow proceed without updating the counter would also be missed.
**Issue 2 (Wad units):** `isolation_debt_ceiling_usd_wad` is in WAD (USD wad). `get_isolated_debt` returns the same units (utils.rs:172, borrow.rs:329). The rule does not check that `current_debt` was updated by the borrow — it could be the pre-borrow value if the cache flush failed.
**Issue 3 (corrupt-state path):** The rule doesn't assume `account.try_isolated_token().is_some()` — if `is_isolated == true && isolated_asset == None`, `handle_isolated_debt` panics with `InternalError` (borrow.rs:320-322) and the rule's post-condition is vacuously satisfied. So a regression that clears `isolated_asset` on an isolated account would be missed.
**Issue 4:** No summary application visible — this rule calls `borrow_single` against the real cross-contract path. May TAC-blow up.
**Catches real bug:** Weakly. The "<=" invariant is asserted but the rule doesn't drive the precondition strongly enough to make the bug paths reachable.
**Fix:**
1. Add `cvlr_assume!(account_data.is_isolated && /* meta.isolated_asset.is_some() via a stronger shim */);`
2. After the call, also assert that `current_debt > pre_borrow_debt` to verify the counter was actually updated.
3. Add a sibling rule for the "should-revert" branch: precondition `pre_borrow_debt + amount_usd_wad > ceiling`, expectation `cvlr_satisfy!(false)` post-call (i.e., must revert).
4. Add a sibling rule for **monotonicity under repay**: a `repay_single` on an isolated account never *increases* `isolated_debt`. The rule for the dust-floor zero (`new_debt < WAD => 0`) is in `adjust_isolated_debt_usd` (utils.rs:181-183) and is currently unverified.

### `isolation_sanity` / `emode_sanity` (lines 148-158)

**Severity:** ok (sanity)
**Issue:** Reachability checks. Sound.

---

## Coverage gaps (rules that should exist)

Beyond the per-rule fixes above, these invariants from the brief have **no rule**:

1. **Bulk-isolation rejection** — `validation::validate_bulk_isolation` (validation.rs:97-111) rejects multi-asset batches when `account.is_isolated || first_config.is_isolated_asset`. No rule. *Needed*: rule that constructs a 2-asset batch on an isolated account and asserts revert.
2. **Second-collateral rejection on isolated account** — `validate_isolated_collateral` (emode.rs:131-155) rejects when `account.is_isolated && asset != existing_asset`. No rule. *Needed*: rule that supplies asset_B on an isolated account whose first collateral is asset_A and asserts revert with `MixIsolatedCollateral`.
3. **Non-isolated account rejecting isolated asset** — `validate_isolated_collateral` lines 141-143. No rule.
4. **`effective_asset_config(category_id == 0) == base_config`** — should be a 1-line rule.
5. **`effective_asset_config` with deprecated category returns base config** — currently the rule misses this corner.
6. **Isolated debt is monotone-non-increasing under repay** — the brief explicitly calls this out.
7. **Isolated debt counter cleared on `clear_position_isolated_debt`** (repay.rs:142-167).
8. **`is_collateralizable`/`is_borrowable` flag override semantics** — the brief asks "do e-mode flags strictly tighten base config?" Looking at apply_e_mode_to_asset_config (emode.rs:24-25), the assignment is `asset_config.is_collateralizable = aec.is_collateralizable` — this is **replacement, not strict tightening**. So an e-mode flag of `true` on a base `false` asset would *enable* it. That is a real risk: an isolated-asset base config with `is_collateralizable = false` could be flipped on by e-mode. There is no rule guarding "e-mode flags only narrow." Whether that is the intended semantics is a *design* question — but the audit should flag it.
9. **`add_asset_to_e_mode_category` rejects an isolated asset** — `ensure_e_mode_compatible_with_asset` is called at supply/borrow time, but `add_asset_to_e_mode_category` (config.rs:310-360) does NOT reject `is_isolated_asset`. So an admin can add an isolated asset to an e-mode category, and the supply gate will then panic at runtime for any user. That is a foot-gun. *Needed*: either a rule asserting `add_asset_to_e_mode_category(asset, _, _, _)` reverts when `asset.is_isolated_asset`, or a finding that the admin path lets a self-conflicting config be persisted.
10. **`apply_e_mode_to_asset_config` is a no-op when category is None or asset config is None** — currently no rule covers the `None` branches.

---

## Concrete bugs each (refined) rule would catch

| Rule | Bug it catches |
|---|---|
| `emode_only_registered_assets` (with deprecated guard added) | `validate_e_mode_asset` skips `asset_cats.contains` check; user supplies foreign asset under a high-LTV e-mode category, gaming health factor. |
| `emode_only_borrowable_assets` | E-mode asset with `is_borrowable == false` becomes borrowable due to a regression — user borrows an asset under tightened e-mode params they shouldn't have access to. |
| `deprecated_emode_blocks_new_supply/borrow` | `ensure_e_mode_not_deprecated` regression; users open new positions in a wound-down category. |
| `deprecated_emode_allows_withdraw` (with non-vacuous assertion) | Wind-down regression that traps user funds in a deprecated category. |
| `emode_overrides_asset_params` (with deprecated assume + new sibling rules) | `effective_asset_config` returns wrong values, e.g., applies e-mode override on a deprecated category (currently impossible — but a refactor that removes the early-return at emode.rs:21-22 would silently re-enable boosted LTV on stale accounts). |
| `emode_category_has_valid_params` | Category created with `threshold <= ltv` — instant liquidatability at max borrow. |
| `emode_remove_category` (augmented) | Side-map / reverse-index drift — orphan entries that re-inflate effective LTV after the asset is "removed" from the category. |
| `emode_account_cannot_enter_isolation` (refocused) | Asset-side gate regression — an isolated asset enters an e-mode category. |
| `emode_isolation_mutual_exclusion_invariant` (inductive) | A migration / restore path produces a meta with both flags. |
| `isolated_single_collateral` (inductive) | Bulk-isolation gate or `validate_isolated_collateral` regression — two collaterals on an isolated account. |
| `isolation_debt_ceiling_respected` (with monotone + must-revert siblings) | `handle_isolated_debt` reads ceiling from wrong asset, fails to update counter, or off-by-one in the cache. |
| `bulk_isolation_rejects_mixed_batch` (NEW) | `validate_bulk_isolation` regression — user supplies 2 assets in one tx where first is isolated. |
| `validate_isolated_collateral_rejects_second_asset` (NEW) | Second-collateral gate regression. |
| `isolated_debt_monotone_under_repay` (NEW) | `adjust_isolated_debt_usd` regression — counter drifts upward under repay. |
| `effective_asset_config_zero_category_returns_base` (NEW) | A regression in `e_mode_category(env, 0)` returning `Some` instead of `None`, making every account effectively e-moded. |
| `add_asset_to_e_mode_rejects_isolated_asset` (NEW or finding) | Admin foot-gun — persists a config that always reverts at user runtime. |

---

## Recommended action items (prioritized)

1. **HIGH — `emode_overrides_asset_params`**: add `cvlr_assume!(!category.is_deprecated)` to make the rule pass; add a sibling rule for the deprecated case (assert override is *not* applied).
2. **HIGH — `emode_remove_category`**: extend post-conditions to assert reverse-index cleanup, side-map removal, and `e_mode_enabled` flag clearing. This is the ONE rule that should validate the storage-refactor invariants.
3. **HIGH — Mutual-exclusion invariants** (`emode_isolation_mutual_exclusion_invariant` and `isolation_emode_exclusive`): convert to inductive form against the entry points, and de-duplicate.
4. **HIGH — `isolation_debt_ceiling_respected`**: add monotonicity + must-revert siblings; assume `isolated_asset.is_some()`.
5. **MEDIUM — `deprecated_emode_allows_withdraw`**: replace `cvlr_satisfy!(true)` with an assertion that the withdraw actually reduced the position.
6. **MEDIUM — `isolated_single_collateral`**: convert to inductive; tighten the `accounts::get_account_data` shim to return `Option<Address>`.
7. **MEDIUM — Coverage gaps**: add the 5 missing rules under "Coverage gaps".
8. **LOW — Cosmetic**: deduplicate "Rule 3"/"Rule 6" labels; replace `< 10000` with `BPS`; tighten `add_asset_to_deprecated_category` precondition.
9. **DESIGN-LEVEL FINDING (separate from rule fixes)**: `add_asset_to_e_mode_category` does not reject `is_isolated_asset`. The runtime gate (`ensure_e_mode_compatible_with_asset`) catches the conflict at supply/borrow time, but admins can persist an unusable membership. Decide whether to (a) gate it in `add_asset_to_e_mode_category` or (b) document that the runtime gate is the single source of truth.
10. **DESIGN-LEVEL FINDING**: e-mode `is_collateralizable` / `is_borrowable` flags **replace** rather than tighten the base config (emode.rs:24-25). If the intent is "e-mode can only narrow," add a rule. If intent is "e-mode replaces," add a comment to that effect on `apply_e_mode_to_asset_config`.

---

## Verdict

**Fail.** Two HIGH issues (`emode_overrides_asset_params` unsoundness with deprecated categories; `emode_remove_category` missing the load-bearing post-conditions of the recent storage refactor) plus the mutual-exclusion vacuity and the `isolation_debt_ceiling_respected` weak post-condition. The rule set covers the right *topics* but several of the spec's most important invariants — bulk-isolation, second-collateral rejection, isolated-debt monotonicity under repay, and the slim-storage cleanup — are unverified.
