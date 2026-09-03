# A083 — Cross-spoke isolation of usage maps

- Agent: A083
- Theme: T5
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/context/spoke.rs:12–143` (`ensure_spoke_context`, `reset_spoke_context`, apply/persist); `contracts/controller/src/spoke_usage.rs:61–141` (`SpokeUsageContext` pin + `persist`/`load_usage_row`); `contracts/controller/src/storage/spoke.rs:56–79` (`ControllerKey::SpokeUsage(spoke_id, HubAssetKey)`); `contracts/controller/src/account.rs:74,154–158` (immutable bind + `require_spoke_match`); `contracts/controller/src/positions/{mod,supply,debt}.rs` (usage always via `account.spoke_id`); `contracts/controller/src/positions/liquidation/{mod,apply,bad_debt}.rs` (same-spoke Credit + exits); `contracts/controller/src/keepers.rs:86–91` (only production `reset_spoke_context`)
- Defense: Durable usage is keyed by `(spoke_id, hub_asset)`. In-tx Cache holds **at most one** `SpokeUsageContext`; `ensure_spoke_context` panics `SpokeMismatch` (#310) if a second spoke is requested without `reset_spoke_context`. All production writers attribute deltas to `account.spoke_id` (never a free-floating caller id at merge time). Accounts cannot rebind spoke (ADR-0009 / INV-AUTH-06). Credit liquidations re-assert receiver spoke equality before share credit / fee exit.
- Gap: (1) **No dedicated unit test** that `ensure_spoke_context(spoke_a)` then `ensure_spoke_context(spoke_b)` panics without reset — isolation is proven by construction + account-level harness, not by a Cache-pin regression. (2) **Footgun** (also A078/A103): `reset_spoke_context` drops unpersisted buffered usage rows; safe today because the only caller (`update_account_threshold`) never buffers usage. Future multi-spoke mutation in one Cache would need persist-then-reset.
- Impact: No path found that can charge spoke B’s cap occupancy from spoke A’s positions, or write `SpokeUsage(A, …)` under B’s identity. Wrong attribution would distort soft governance caps (INV-HALT-03) per spoke market — capacity integrity, not direct theft. That class is currently closed.
- Evidence: INV-AUTH-06, INV-HALT-03, ADR-0009, ADR-0015; A028 key domain; A076 one-spoke Cache pin; A078 reset footgun; A063/A013 SpokeMismatch on account/Credit; harness `spoke.rs` / `keeper.rs` (`test_update_account_threshold_mixed_spokes_batch`); unit `spoke_usage_context_preserves_spoke_id`
- Opinion: Closes A103 §7.3 coverage debt. Cross-spoke usage isolation is a **load-bearing defended surface**, not a residual critical. Do not “optimize” by removing the pin or by mid-flow reset without persist. Recommend a small unit test for the pin panic as P3 hygiene.

---

## 1. Mission and method

Confirm that spoke usage maps cannot mix across spokes: storage domain, Cache pin (`SpokeMismatch`), reset semantics, and every production apply/persist path.

Method:

1. Read `COORDINATION.md`, `SEED.md`, peers A076 / A103 (and supporting A028, A078, A063, A013, A086).
2. Trace `ensure_spoke_context` / `reset_spoke_context` / `apply_spoke_{entry,exit}` / `persist_spoke_usage` to all callers.
3. Verify `spoke_id` provenance on every writer (account bind vs caller arg).
4. Enumerate attack scenarios that would attribute usage to the wrong spoke.
5. Inventory tests / Certora adjacency (isolation vs delta tracking).

No production Rust edited. No git operations.

---

## 2. Isolation layers (defense-in-depth)

Cross-spoke isolation is not a single check. Four layers must all fail for wrong-spoke attribution:

| Layer | Mechanism | Failure mode if removed |
|---|---|---|
| **L1 Durable key** | `ControllerKey::SpokeUsage(spoke_id, HubAssetKey)` | Same hub-asset on two spokes share one occupancy counter → caps bleed |
| **L2 In-tx pin** | `Cache.spoke_usage: Option<SpokeUsageContext>` with fixed `spoke_id`; `ensure_spoke_context` asserts match | Buffered map could load/write rows under the wrong spoke id mid-tx |
| **L3 Account bind** | `AccountMeta.spoke_id` set once; merges use `account.spoke_id` | Caller could pass foreign `spoke_id` into apply while mutating another account’s positions |
| **L4 Entry/Credit gates** | `require_spoke_match` / receiver `spoke_id == account.spoke_id` | Wrong-regime risk params + wrong listing/caps for the account |

Today all four hold on inventoried paths.

---

## 3. L1 — Durable storage domain

```56:79:contracts/controller/src/storage/spoke.rs
pub(crate) fn get_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeUsageRaw> {
    get_shared(env, &ControllerKey::SpokeUsage(spoke_id, hub_asset.clone()))
}

pub(crate) fn set_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    usage: &SpokeUsageRaw,
) { /* write or prune both-zero */ }
```

- Same token under hub H listed on spoke 1 and spoke 2 is **two** usage domains (A028). Caps (`supply_cap` / `borrow_cap`) live on `SpokeAsset(spoke_id, hub)` and are read only after the Cache pin (see §4).
- Production writes to usage go only through `SpokeUsageContext::persist` → `set_spoke_usage(env, self.spoke_id, …)`. The context’s `spoke_id` is fixed at `SpokeUsageContext::new` (via `ensure_spoke_context`). There is no persist-time override.
- Admin `remove_asset_from_spoke` gates on `get_spoke_usage(env, spoke_id, &hub_asset)` for **that** spoke only — cannot clear listing on spoke A based on spoke B’s zeros (or vice versa).

**Verdict L1:** Defended. Key collision across spokes is structurally impossible under the typed enum.

---

## 4. L2 — Cache pin: `ensure_spoke_context` / `reset_spoke_context`

### 4.1 Pin semantics

```12:29:contracts/controller/src/context/spoke.rs
pub(crate) fn ensure_spoke_context(&mut self, spoke_id: u32) {
    if let Some(ctx) = &self.spoke_usage {
        assert_with_error!(
            &self.env,
            ctx.spoke_id() == spoke_id,
            SpokeError::SpokeMismatch
        );
        return;
    }
    self.spoke_usage = Some(SpokeUsageContext::new(&self.env, spoke_id));
}

pub(crate) fn reset_spoke_context(&mut self) {
    self.spoke_usage = None;
    self.spoke_config = None;
    self.spoke_assets = Map::new(&self.env);
}
```

Properties:

1. First spoke-scoped access creates an empty `SpokeUsageContext` for that `spoke_id` (even if the caller only wanted config/assets).
2. Any later request with a different `spoke_id` panics `#310 SpokeMismatch` — fail closed.
3. `reset_spoke_context` clears **usage + config + assets together**. There is no API that clears only one field and leaves a stale pin.
4. `spoke_assets` is keyed by `HubAssetKey` **without** embedding `spoke_id`. Correctness of that memo depends entirely on the pin (and on reset clearing the map). Same for untagged `spoke_config: Option<SpokeConfig>`.

Call graph into the pin (all go through `ensure_spoke_context`):

| Accessor | Role |
|---|---|
| `require_spoke_usage_context` | apply entry/exit |
| `cached_spoke_asset` / `require_spoke_asset*` / `require_listed_active_config` | listing + caps |
| `spoke_config` / `active_spoke` | curve / deprecation |

`apply_spoke_entry` loads **cap from `require_spoke_asset_config(spoke_id, …)`** then applies into **`require_spoke_usage_context(spoke_id)`** — same argument, both ensure-pinned. Cap config and usage occupancy cannot silently come from different spokes in one Cache.

### 4.2 Persist binding

```138:143:contracts/controller/src/context/spoke.rs
pub(crate) fn persist_spoke_usage(&self) {
    if let Some(ctx) = &self.spoke_usage {
        ctx.persist();
    }
}
```

`SpokeUsageContext::persist` writes every touched in-memory row under `self.spoke_id`. Lazy `load_usage_row` reads `get_spoke_usage(env, self.spoke_id, hub_asset)`. RAM buffer and durable domain stay aligned to the pinned id.

### 4.3 Sole production reset: keepers

`reset_spoke_context` appears **once** in production code:

```86:91:contracts/controller/src/keepers.rs
let mut cache = Cache::new(env);
for account_id in account_ids {
    cache.reset_spoke_context();
    sync_account_thresholds(env, account_id, scope, &mut cache);
}
```

`sync_account_thresholds` reads `cached_spoke_asset(account.spoke_id, …)` and risk-stamps supply positions. It does **not** call `apply_spoke_entry` / `apply_spoke_exit` / `persist_spoke_usage`.

Why reset is mandatory here: a batch can mix accounts on different spokes (harness `test_update_account_threshold_mixed_spokes_batch`). Without reset, processing Alice (spoke 1) then Bob (spoke 2) would hit `SpokeMismatch` on the second `ensure`, or — if the pin were weaker — serve Alice’s memoized `SpokeAssetConfig` for Bob’s hub key.

Why reset is safe here: no buffered usage to drop. This is the opposite of the A078 mid-flow footgun (reset after apply, before persist).

Liquidation / strategy / ordinary position flows **never** reset. Credit double-finalize reuses one Cache on one spoke (A078/A084).

### 4.4 Footgun (not a present bug)

If future code:

1. `apply_spoke_*` for spoke A (RAM dirty),
2. `reset_spoke_context()` without `persist_spoke_usage`,
3. later persist for spoke B (or empty),

then A’s deltas are discarded → under-count on A (same capacity direction as A080, different mechanism). **Not observed** on current liq/strategy/keeper paths. Document as regression hazard for multi-spoke-in-one-tx designs.

**Verdict L2:** Defended for current call graph; residual is operational discipline around reset.

---

## 5. L3 — Writer provenance: always `account.spoke_id`

Inventory of production usage mutators:

| Path | Spoke id source | Persist |
|---|---|---|
| `merge_supply_leg` / `merge_withdraw_leg` | `account.spoke_id` → `apply_leg_usage` | `finalize_position_flow` |
| `merge_debt_leg` (borrow/repay) | `account.spoke_id` | `finalize_position_flow` |
| Credit fee exit (`apply.rs`) | `account.spoke_id` (+ receiver equality assert) | victim/receiver finalize |
| Bad-debt exits (`bad_debt.rs`) | `account.spoke_id` | `persist_spoke_usage` post-seize |
| Strategies | single account → same merges / finalize | one finalize (A032/A084) |

`apply_leg_usage` takes an explicit `spoke_id`, but every production call site passes `account.spoke_id`. Callers that accept a user `spoke_id` (`supply`, multiply, flash_position, migrate) run `require_spoke_match` **before** pool/merge (`AccountGuard`). Borrow / withdraw / repay omit caller `spoke_id` and read the account bind (INV-AUTH-06).

There is **no** production path that applies usage with a literal foreign `spoke_id` while mutating another account’s positions.

`set_account_meta` in production is only `create_account_with` (A021). No rebinding setter — ADR-0009.

**Verdict L3:** Defended.

---

## 6. L4 — Cross-account / cross-spoke gates

| Scenario | Gate | Outcome |
|---|---|---|
| Supply/strategy with wrong `spoke_id` on existing account | `require_spoke_match` | `#310` |
| Credit seize to foreign-spoke receiver | `resolve_seize_receiver` + re-assert in `apply` | `#310` |
| Transfer seize | Liquidator wallet; usage via victim withdraw merge on `account.spoke_id` | same spoke domain |
| Credit fee-only usage exit | Same-spoke cancel of debit/credit; fee exit on victim spoke | intentional (A084); not cross-spoke |
| New account on unknown/zero spoke | `SpokeNotFound` / `spoke_id >= 1` | defended (A063) |

Credit’s design comment is isolation-relevant: account↔account share move cancels **only because both accounts share the spoke**. The re-assert before booking makes that assumption a hard panic, not a soft invariant.

**Verdict L4:** Defended.

---

## 7. Attack / misuse scenarios (isolation-specific)

| # | Attempt | What would go wrong if undefended | Actual outcome |
|---|---|---|---|
| 1 | Fill spoke A to cap; supply on spoke B hoping to share A’s headroom | Caps would be global per hub-asset | Separate `SpokeUsage` keys; B has its own cap (A028/A083) |
| 2 | Call `supply(spoke_id=B)` on account bound to A | Usage credited to B while positions live on A’s risk regime | `SpokeMismatch` at load |
| 3 | Credit liquidate into receiver on spoke B | Positions/risk migrate across regimes; usage cancel broken | `SpokeMismatch` before books move |
| 4 | Keeper batch Alice@1 then Bob@2 without reset | Second ensure panics **or** stale asset config memo | Reset each iteration; mixed-spoke harness passes |
| 5 | Mid-liq `reset_spoke_context` after fee exit, before persist | Fee exit dropped → overstated capacity on victim spoke | No such call in liq code |
| 6 | Direct `set_spoke_usage` from user entrypoint | Arbitrary occupancy forge | Not exposed; only context persist / tests / Certora fixture |
| 7 | View `get_spoke_usage(spoke, hub)` with forged spoke | Read confusion only | Read-only; defaults absent to zero; no write |

None of 1–4 / 6–7 yield a live isolation bypass. #5 is a future-edit hazard.

---

## 8. Interaction with peer residuals (not novel criticals)

| Peer | Relation to A083 |
|---|---|
| **A080** | Missing-row exit under-counts **within** a spoke. Does not move occupancy across spokes. Orthogonal. |
| **A078** | Persist-after-pool + reset footgun. A083 confirms reset is only used where usage is clean. |
| **A028** | Key-family domain is the durable half of isolation; A083 owns the Cache pin + attribution. |
| **A076** | Semantics assume one context per invocation; A083 deep-dives that claim. |
| **A084** | Double finalize same Cache / same spoke — compatible with pin. |
| **A086** | Cache inventory lists pin/reset; A083 proves no wrong-spoke writer. |
| **A063 / A013** | Account and Credit `SpokeMismatch` gates feed L4. |

No disagreement file required vs A076/A103 provisional §7.3 (“isolation looks defended”).

---

## 9. Evidence matrix (tests / formal)

| Claim | Evidence | Gap? |
|---|---|---|
| Account cannot act under foreign spoke_id | Harness `test_supply_rejects_spoke_mismatch_*`; unit `load_existing_account_rejects_spoke_mismatch`; integration flash/migrate/liq Credit | No |
| Context remembers spoke_id | Unit `spoke_usage_context_preserves_spoke_id` | Narrow |
| Mixed-spoke keeper batch works | `test_update_account_threshold_mixed_spokes_batch` | Implies reset necessity |
| Cap/usage per spoke listing | `spoke_caps.rs` (spoke 2 markets); storage round-trips A028 | No explicit “spoke1 full ≠ blocks spoke2” twin-cap test |
| Cache pin panic without reset | **Missing dedicated unit** | **Yes — P3** |
| Certora delta tracking | `usage_*` rules seed one `SPOKE_ID` | Does not prove cross-spoke non-interference; out of scope for those rules |

Recommend (P3): unit test `Cache::ensure_spoke_context(1)` then `ensure_spoke_context(2)` → expect `#310`; and optionally twin-spoke same hub-asset supply to show independent usage rows.

---

## 10. Structural dependency to preserve

The untagged memos (`spoke_assets` by hub only; `spoke_config` as bare `Option`) are **correct if and only if**:

1. Every spoke-scoped read/write calls `ensure_spoke_context` first, and
2. Cross-spoke work in one Cache uses `reset_spoke_context` (after persist if usage was dirty).

Do not:

- Add a second `SpokeUsageContext` without changing the pin model.
- Key `spoke_assets` by hub alone **and** remove the pin/assert.
- Call `reset_spoke_context` between `apply_spoke_*` and `persist_spoke_usage`.
- Pass a caller `spoke_id` into `apply_leg_usage` that is not `account.spoke_id` after the match gate.

---

## 11. Verdict

**Cross-spoke usage isolation is defended.** Durable keys include `spoke_id`; the Cache pins a single `SpokeUsageContext` and fails closed on mismatch; resets clear the full spoke memo set and are only used on a usage-clean keeper path; every writer attributes deltas to the account’s immutable spoke; Credit receivers cannot cross spokes.

**Residuals are hygiene-only:** missing Cache-pin unit test; documented mid-flow reset footgun for future multi-spoke mutations. Neither is a present attribution bug or a second medium residual beside A080.

A103’s provisional §7.3 claim is confirmed and upgraded from coverage debt to a filed finding.
