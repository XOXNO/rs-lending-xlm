# A022 — Supply path storage writes (`process_supply` / `merge_supply_leg` / finalize)

- Agent: A022
- Theme: T2
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/lib.rs:98-107`; `contracts/controller/src/positions/supply.rs:44-155,303-357`; `contracts/controller/src/positions/mod.rs:52-90,112-141,206-252,289-327,358-368`; `contracts/controller/src/account.rs:47-114,172-207`; `contracts/controller/src/risk/params.rs:17-91`; `contracts/controller/src/risk/validation.rs:12-61`; `contracts/controller/src/context/{mod,spoke,market_index,events}.rs`; `contracts/controller/src/spoke_usage.rs:61-160`; `contracts/controller/src/storage/{account,spoke}.rs`; `contracts/controller/src/payments.rs`; `common/src/token.rs:19-34`; `common/src/types/controller.rs:326-360`; `scripts/permissionless_entrypoints.txt:63`
- Defense: Ordinary supply mutates durable controller position/usage state only at `finalize_position_flow` after measured token→pool transfer + pool settle + in-memory merge. Create path writes `AccountMeta` (+ NFT mint) early; shares, spoke usage, and events wait for the shared tail. Finalize order is usage → `SupplyPositions` → TTL renew → emit. Debt / delegates / protocol config are never rewritten. Scaled usage deltas and persisted shares come from pool mutation outputs; event token amount uses the measured receipt credited into the pool action.
- Gap: none on ordinary `process_supply` durable accounting. Residuals (cross-ref, not novel critical): (1) listed-token transfer hooks can re-enter monetary entrypoints while outer merge is still buffered — same class as A007/A023; flash flag is not set on plain supply. (2) Event `amount` is measured transfer (`action.amount`), not `outcome.amount` / pool `actual_amount` — observational only if those ever diverge (no equality assert on this verb; shares still follow pool `new_scaled`). (3) `remove_if_empty: false` is correct here; empty-shell rent is an A036 concern on other verbs, not supply.
- Impact: Successful supply can (1) create meta+NFT when `account_id==0`, (2) increase/open per-account supply shares and restamp FullTuple risk params on touched hubs, (3) increase `SpokeUsage` supply RAY (cap-gated) for touched hubs, (4) extend instance + existing user TTLs, (5) emit a post-persist position batch. Cannot mint debt, rewrite foreign slots (INV-AUTH-03), change spoke/mode after create, retarget pool/oracle/NFT addresses, or persist another account’s maps. Blast radius if finalize were skipped after pool success would desync controller books vs pool — prevented by single-tx atomicity + mandatory finalize on this entrypoint.
- Evidence: INV-AUTH-03, INV-ACCT-03, INV-RISK-04, INV-HALT-02 (`BlockOnEntry`), INV-STOR-01/03; Certora `supply_new_slot_requires_owner_or_delegate`, `bulk_supply_two_assets_both_persisted`, `usage_supply_tracks_scaled_delta`; harness `tests/test-harness/tests/controller/supply.rs`; peers A004, A012, A021, A023, A024, A032, A033, A036, A040, A041, A055, A076, A077, A082, A086, A094.
- Opinion: Supply’s storage story is sound and deliberately asymmetric with borrow/withdraw: always `PositionSides::Supply` + `remove_if_empty: false`; no post-pool solvency restamp (risk-reducing); entry-side usage always creates/updates a row (no A080 exit no-op). Do not add `Both` or solvency gates “for symmetry” — they are load-bearing absences. Keep measured receipt → pool action amount → pool `new_scaled` → usage delta as the credit chain.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only).
2. Trace `Controller::supply` → `process_supply` → `load_or_create_account` / third-party gate → `process_deposit` → `settle_supply` → `merge_supply_leg` → `finalize_position_flow`.
3. Enumerate every durable write (`persistent` / instance TTL / cross-contract NFT) reachable on that path; separate Cache buffers from storage keys.
4. Cross-check peer findings A004, A007, A012, A021, A023, A024, A032, A033, A036, A040, A041, A055, A076, A077, A082, A086, A094 and invariants INV-AUTH-03 / INV-ACCT-03 / INV-STOR-01/03.
5. Note shared helper `process_deposit` / `merge_supply_leg` used by strategies — those callers own finalize (A032); this file judges ordinary `process_supply` finalize coupling.

Out of scope as primary claims: withdraw (`process_withdraw` — A024), token lying beyond measured-receipt note (A055), strategy batching beyond shared merge (A032), pool-internal cash books.

---

## Call graph (ordinary supply)

```
Controller::supply                            #[when_not_paused]  lib.rs:98-107
  └─ process_supply                           supply.rs:44-78
       ├─ require_authorized_caller           auth + !flash_loaning
       ├─ aggregate_positive_payments         dedupe hubs; reject ≤0 / empty
       ├─ Cache::new                          renews controller instance TTL
       ├─ load_or_create_account(..., Supply)
       │    ├─ account_id==0 → create_account
       │    │    ├─ active_spoke check
       │    │    ├─ nft_mint_call(caller)     CROSS-CONTRACT NFT
       │    │    └─ set_account_meta          EARLY durable write (spoke, Normal)
       │    └─ else get_account + require_spoke_match
       ├─ require_third_party_existing_supply INV-AUTH-03 (skip if create)
       ├─ process_deposit
       │    ├─ validate_position_entry_gates  limits + require_can_supply
       │    └─ settle_supply
       │         ├─ transfer_amount_measured  token → pool (measured Δ)
       │         ├─ get_or_create_supply_position  RAM seed only (no map insert)
       │         ├─ pool_supply_call          pool durable writes (other contract)
       │         └─ for_each_leg → merge_supply_leg
       │              ├─ refresh_supply_risk_params FullTuple   memory
       │              ├─ position.scaled_amount = outcome.new_scaled
       │              ├─ apply_leg_usage Entry → apply_spoke_entry  BUFFER usage+cap
       │              ├─ put_market_index     CACHE only
       │              ├─ record_supply_position_update  event buffer
       │              └─ update_or_remove_supply_position  Account RAM only
       └─ finalize_position_flow(..., Supply, remove_if_empty=false)
            ├─ persist_spoke_usage            SpokeUsage keys
            ├─ persist_account_positions
            │    ├─ set_supply_positions      WRITE/REMOVE SupplyPositions
            │    ├─ (skip) set_debt_positions
            │    ├─ renew_user_account        TTL live account keys
            │    └─ cleanup skipped           remove_if_empty=false
            └─ emit_position_batch            after durable writes (A033)
```

---

## Durable writes inventory

| Phase | Storage class | Key / target | Writer | Notes |
|---|---|---|---|---|
| Entry | Instance | controller instance TTL | `Cache::new` → `renew_controller_instance` | Every mutating path; INV-STOR-01 |
| Create only | Persistent user | `AccountMeta(account_id)` | `create_account` → `set_account_meta` | `{spoke_id, mode: Normal}` — **before** pool/finalize |
| Create only | Position NFT | token mint to caller | `nft_mint_call` | Cross-contract; id == account_id (A004/A031) |
| Mid (token) | Token contract | balances caller→pool | `transfer_amount_measured` | Not controller storage |
| Mid (pool FFI) | Pool contract | pool market/user books | `pool_supply_call` | Trusted sibling; rolls back with tx on later panic |
| Mid (controller) | — | none | — | `merge_supply_leg` is **not** a durable write |
| Finalize | Persistent shared | `ControllerKey::SpokeUsage(spoke_id, hub)` | `SpokeUsageContext::persist` → `set_spoke_usage` | Removes key if both scaled sides are 0 |
| Finalize | Persistent user | `SupplyPositions(account_id)` | `set_supply_positions` → `write_side_map` | Full map snapshot; remove if empty map |
| Finalize | Persistent user TTL | AccountMeta / Supply / Borrow / Delegates if present | `renew_user_account` | Extends TTL; no value rewrite |
| Finalize | Events | `UpdatePositionBatchEvent` | `emit_position_batch` | After persists; observational |
| Never on this path | Persistent user | `BorrowPositions`, `Delegates` | — | Debt loaded for gated LT refresh HF math only |
| Never on this path (top-up) | Persistent user | `AccountMeta` | — | Spoke/mode immutable after create (A021) |
| Never durable | Cache maps | `market_indexes`, event vecs, spoke_assets, … | `put_market_index`, buffers | Dropped at end of invocation |

Temporary `FlashLoanOngoing` is **not** set on ordinary supply (only flash/strategy windows — A007/A030).

### Explicit non-writes (important)

- **No `set_debt_positions`**: `PositionSides::Supply` skips the debt map. In-memory borrow map may be read for `apply_gated_liquidation_params` HF hypothetics but is never persisted back — intentional; avoids clobbering debt.
- **No `set_account_meta` on top-up**: spoke/mode unchanged; create already wrote meta.
- **No delegate map mutation**.
- **No spoke asset / spoke config / hub / pool / oracle / accumulator writes**.
- **No controller-side market-index persistence** — `put_market_index` is Cache-only; pool remains SoT (A086/A094).
- **No `enforce_post_pool_solvency` / `restamp_listed_supply_ltv`**: supply does not walk sibling collateral LTVs. Touched legs get `refresh_supply_risk_params(FullTuple)` in merge; those stamps land because `Supply` side is always written.
- **No empty-account cleanup**: `remove_if_empty: false` — supply cannot empty an account; cleanup belongs on withdraw/strategy/liq (A024/A036).

---

## Phase analysis

### 1. Gates before position/usage writes

1. **Pause** — `#[when_not_paused]` on `supply` (risk-increasing / new exposure).
2. **Auth** — `require_authorized_caller` (`caller.require_auth` + flash-loan gate).
3. **Inputs** — `aggregate_positive_payments` rejects empty/non-positive legs and collapses duplicate hubs so each hub appears once in the pool batch.
4. **Account resolve** — create (`account_id==0`) or load + `require_spoke_match` (INV-AUTH-06 spoke immutability).
5. **Third-party slot rule** — non-owner/non-delegate may only top up hubs already in `supply_positions` (INV-AUTH-03 / A012). Create skips (caller becomes owner).
6. **Listing / halt / collateralizable** — `validate_position_entry_gates` → `require_can_supply` → hub active, listed on account spoke, `BlockOnEntry` (not paused/frozen), `is_collateralizable` (A040).
7. **Position count** — `validate_bulk_position_limits` (INV-RISK-04).

No position/usage `set_*` runs in this phase. Create may write meta+NFT; `Cache::new` only renews instance TTL otherwise.

### 2. Create-time meta vs finalize positions (ordering)

```47:76:contracts/controller/src/account.rs
pub(crate) fn create_account_with(...) -> (u64, Account) {
    // ...
    let account_id = nft_mint_call(env, &nft, owner);
    // ... empty in-memory maps ...
    storage::set_account_meta(env, account_id, &AccountMeta { spoke_id, mode });
    (account_id, account)
}
```

| Fact | Implication |
|---|---|
| Meta is the existence sentinel (A021) | Account “exists” before first share is persisted |
| NFT mint is paired at create (INV-STOR-03) | Ownership never stored on controller |
| Shares wait for finalize | Mid-tx panic after mint/meta reverts entire tx (Soroban atomicity) — no durable orphan on failure |
| Success path always finalizes | Live create leaves meta + non-empty `SupplyPositions` after positive supply |

Residual: within the same successful invocation, a listed-token transfer hook after meta mint but before finalize can re-enter against empty-on-disk supply maps while outer RAM state is mid-merge (A007 residual class). Listing trust + measured settlement bound that; not unique to supply.

### 3. `settle_supply` — measure, pool, then in-memory merge

#### 3.1 Measured receipt into pool action

```135:154:contracts/controller/src/positions/supply.rs
for (hub_asset, amount_in) in aggregated.iter() {
    let asset_config: AssetConfig = cache.require_spoke_asset(account.spoke_id, &hub_asset);
    let received = payments::transfer_amount_measured(
        env, &hub_asset.asset, caller, &pool_addr, amount_in, ...
    );
    let position = account.get_or_create_supply_position(&hub_asset, &asset_config);
    entries.push_back(PoolSupplyEntry {
        action: make_pool_action(&position, received, hub_asset.clone()),
    });
}
let results = pool_supply_call(...);
for_each_leg(..., |entry, result| merge_supply_leg(...));
```

- Transfer is **caller → pool** with balance delta measured at the pool (INV-ACCT-03 / A041). Fee-on-transfer shrinks `received`; pool is asked to credit that measured amount.
- `get_or_create_supply_position` returns existing or a **zero** seed with config risk stamps — **does not insert** into `account.supply_positions` (`common/.../controller.rs:329-344`). Map insert only later in `update_or_remove_supply_position`.
- `for_each_leg` asserts `entries.len() == results.len()` then merges in order; aggregation already deduped hubs.

#### 3.2 `merge_supply_leg` — buffer only

```308:357:contracts/controller/src/positions/supply.rs
pub(crate) fn merge_supply_leg(...) {
    let mut position = account.get_or_create_supply_position(hub_asset, &asset_config);
    let old_scaled = position.scaled_amount;
    refresh_supply_risk_params(..., RiskRefreshScope::FullTuple);
    let outcome = LegOutcome::from(result);
    position.scaled_amount = outcome.new_scaled;
    apply_leg_usage(..., LegDirection::Entry { asset_decimals: result.asset_decimals }, ...);
    cache.put_market_index(...);
    cache.record_supply_position_update(
        PositionAction::Supply, hub_asset, outcome.market_index.supply_index,
        action.amount,  // measured receipt, not outcome.amount
        &position,
    );
    update_or_remove_supply_position(account, hub_asset, &position);
}
```

| Step | Effect | Durable? |
|---|---|---|
| `old_scaled` | Missing hub → `Ray::ZERO` | No |
| `refresh_supply_risk_params(FullTuple)` | Always restamps LTV; LT/bonus/fees via `apply_gated_liquidation_params` (debt + HF gate) | No until finalize |
| `LegOutcome` | Pool `new_scaled`, index, `actual_amount` | No (pool SoT — A082) |
| `apply_leg_usage` Entry | `delta = new_scaled - old_scaled`; cap via `calculate_scaled_cap` on **returned** supply index + decimals | No until finalize |
| `put_market_index` | Cache refresh for later math | No |
| `record_supply_position_update` | Queue `EventDepositDelta` with post-merge stamps | No until emit |
| `update_or_remove_supply_position` | Upsert raw map; remove if `scaled_amount == 0` | No until finalize |

Cap failure or math panic aborts before finalize → no controller usage/position writes; Soroban rolls back pool + token movements.

**Entry vs exit usage:** supply entry always materializes a usage row (`unwrap_or_default` then write). The A080 missing-row no-op is exit-only — not on this verb’s happy path (A076).

### 4. Finalize — single commit point

```241:252:contracts/controller/src/positions/mod.rs
pub(crate) fn finalize_position_flow(...) {
    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
    cache.emit_position_batch(account_id, account);
}
```

```218:236:contracts/controller/src/positions/mod.rs
pub(crate) fn persist_account_positions(...) {
    if sides != PositionSides::Debt {
        storage::set_supply_positions(...);
    }
    if sides != PositionSides::Supply {
        storage::set_debt_positions(...);
    }
    storage::renew_user_account(env, account_id);
    if remove_if_empty {
        account::cleanup_account_if_empty(...);
    }
}
```

For `process_supply`: `sides = PositionSides::Supply`, `remove_if_empty = false`.

| Commit step | Behavior |
|---|---|
| `persist_spoke_usage` | Writes every cached `SpokeUsage` row for the invocation’s spoke; zero/zero removes key |
| `set_supply_positions` | Persists full in-memory supply map (shares + stamped BPS params); empty map removes key |
| Skip debt | Preserves on-disk borrow map unchanged |
| `renew_user_account` | TTL bump on existing meta/supply/debt/delegates keys (create also has meta) |
| Skip cleanup | Never burns NFT / deletes meta on this verb |
| `emit_position_batch` | Publishes buffered deposit deltas after durable writes (A033) |

### 5. Why no `PositionSides::Both` / no post-pool solvency

Borrow must widen to `Both` when `restamp_listed_supply_ltv` mutates sibling collateral LTVs used by the solvency gate (A023 / TOB-AAVE-7 class). Supply:

- Does **not** call `enforce_post_pool_solvency`.
- Only mutates supply hubs present in the deposit batch (plus their FullTuple stamps).
- Always persists the supply map, so those stamps cannot stay RAM-only.

Adding a borrow-style solvency gate on supply would be incorrect product behavior (blocking collateral top-ups that improve health). Skipping `Both` is safe because debt bytes are untouched in memory for persistence purposes.

### 6. Third-party top-up and stamped params

A stranger who passes INV-AUTH-03 can still:

- Increase `scaled_amount` on an existing hub (risk-reducing / neutral for foreign leverage).
- Trigger `FullTuple` refresh on that hub: LTV always follows listing; LT/bonus/fees update unless liquidator-favoring change is gated by debt + HF floor (`risk/params.rs:66-91`).

Persisted effect is on the victim’s `SupplyPositions` row for that hub only — cannot open new slots, change spoke/mode, or touch debt map. Agrees with A012.

### 7. Shared `process_deposit` (strategy callers)

`multiply`, `flash_position`, `swap_collateral`, and `migrate_blend` call `process_deposit` / `merge_supply_leg` but **not** `process_supply`’s finalize. They must reach `finalize_position_flow` (often `PositionSides::Both`, `remove_if_empty: true`) via strategy tails (A032). Judging those write sets is out of scope here; the merge buffer semantics are the same.

---

## Threat / invariant cross-check

| Claim | Status on this path |
|---|---|
| INV-AUTH-03 third-party cannot open slots | Enforced before deposit |
| INV-ACCT-03 credit = measured receipt | Measured Δ drives pool action amount; shares from pool `new_scaled` |
| INV-RISK-04 position limits | Pre-pool gate |
| INV-HALT-02 BlockOnEntry | `require_can_supply` |
| INV-STOR-01 lifecycle | Instance renew at Cache::new; user renew at finalize; empty map delete |
| INV-STOR-03 NFT↔account pair | Mint on create; supply never burns |
| STRIDE Tamper (storage) | Stranger cannot rewrite foreign maps; residual listed-token reentrancy |
| Cap / usage integrity | Entry cap on scaled delta; persist after all legs; fail → full revert |

---

## Comparison table (supply vs sibling verbs)

| Property | Supply (A022) | Withdraw (A024) | Borrow (A023) |
|---|---|---|---|
| `PositionSides` | Always `Supply` | Always `Supply` | `Debt` or `Both` if LTV restamp |
| `remove_if_empty` | `false` | `true` | `false` |
| Post-pool solvency | No | Yes | Yes |
| Usage direction | Entry (+cap) | Exit (A080 residual) | Entry (+cap) |
| Early meta write | Yes on create | No | No |
| Pause macro | Yes | No (exit liveness) | Yes |
| Third-party | Top-up existing only | Owner/delegate | Owner/delegate |

---

## Residuals (non-blocking)

1. **Listed-token reentrancy during `transfer_amount_measured`** — flash guard unset; outer create may already have meta+NFT; inner monetary calls possible until listing policy excludes hooks (A007/A055). Not a missing finalize bug.
2. **Event amount source** — `action.amount` (measured) vs unused `outcome.amount` on this verb. Indexers should treat scaled/index fields as authoritative for shares; token amount is the measured inflow. Optional hardening: equality assert `received == result.actual_amount` (present on some strategy paths per A082).
3. **Empty-shell rent** — not introduced by supply; supply with `remove_if_empty: false` is correct because success leaves non-empty supply (or reverts).

---

## Verdict

Ordinary supply’s durable write surface is narrow, ordered, and correctly sided. Shares and spoke usage commit once at finalize from pool outcomes; meta is create-only and spoke-immutable thereafter; events follow storage. No novel controller write-set bug found on this verb. Treat listing-trust reentrancy and event-amount observational asymmetry as shared residuals, not supply-specific storage failures.
