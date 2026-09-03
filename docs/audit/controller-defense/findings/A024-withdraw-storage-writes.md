# A024 — Withdraw path storage writes (`process_withdraw`)

- Agent: A024
- Theme: T2
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/lib.rs:123-135`; `contracts/controller/src/positions/supply.rs:161-435`; `contracts/controller/src/positions/mod.rs:92-104,112-141,218-252,329-354`; `contracts/controller/src/account.rs:164-207`; `contracts/controller/src/context/{mod,spoke,market_index,events}.rs`; `contracts/controller/src/spoke_usage.rs:77-141`; `contracts/controller/src/storage/{account,spoke,protocol}.rs`; `contracts/controller/src/risk/{params,validation}.rs`; `scripts/permissionless_entrypoints.txt:46`
- Defense: Owner/delegate-gated exit mutates only in-memory `Account` + `Cache` until a single `finalize_position_flow` tail: persist spoke-usage rows → write `SupplyPositions` (remove key if empty) → renew live account keys → optional NFT-paired cleanup → emit buffered events. Debt/meta/delegate/protocol config keys are not rewritten on the happy path. Scaled deltas and event amounts come from pool mutation outputs, not caller request amounts. Post-pool solvency runs before durable writes; Soroban atomicity rolls back pool + controller together on failure.
- Gap: Shared residual with A080 — `apply_exit` no-ops when no spoke-usage row exists, so a withdraw that reduces/removes a supply position may leave spoke usage unchanged (capacity overstated until reconcile). Not unique to withdraw; not a direct fund-theft path. No novel controller write-set bug found on this verb.
- Impact: Successful withdraw can (1) decrease/remove per-account supply shares and stamped risk params, (2) decrease (or remove-if-both-zero) `SpokeUsage` supply RAY for touched hubs, (3) extend instance + user TTLs, (4) on full empty exit, delete all account keys and burn the position NFT. Cannot mint debt, open foreign supply slots, retarget pool/oracle/NFT, or rewrite another account’s maps. Blast radius if finalize were skipped after pool success would be desynced controller books vs pool — prevented by single-tx atomicity + the mandatory finalize tail.
- Evidence: INV-AUTH-02, INV-RISK-01, INV-HALT-01/03, INV-STOR-01/03, INV-ACCT share-non-negativity via pool; Certora `usage_withdraw_tracks_scaled_delta`, `usage_exit_without_usage_row_is_a_noop`; harness `tests/test-harness/tests/controller/withdraw.rs` (partial/full/cleanup/pause/HF); A003, A031, A032, A033, A036, A072, A076, A080, A082.
- Opinion: Withdraw’s durable write surface is narrow, batched, and correctly sided (`PositionSides::Supply` + `remove_if_empty: true`). Treat A080 as the only material residual on this path’s storage semantics; do not “fix” by writing usage on missing rows without a deliberate reconcile policy.

## Scope

Inventory every **controller** storage mutation reachable from `Controller::withdraw` → `process_withdraw`, classify durable vs buffered vs TTL-only vs cross-contract, and judge whether the write set can inflate risk, strand authority, or desync spoke usage / account maps.

Out of scope as primary claims (peer agents): token transfer measurement (A042/A055), recipient hijack (A057), auth TOCTOU (A011), liquidation/strategy withdraw legs except where they share `merge_withdraw_leg` / `finalize_position_flow`.

---

## Call graph (storage-relevant)

```
Controller::withdraw                          # no #[when_not_paused]
  └─ process_withdraw
       ├─ require_authorized_caller           # auth + flash guard read (temp)
       ├─ storage::get_account                # READ meta+supply+debt; NFT owner
       ├─ require_owner_or_delegate           # READ delegates / position-manager
       ├─ Cache::new → renew_controller_instance   # TTL instance
       ├─ require_external_recipient          # READ pool addr (cached)
       ├─ payments::aggregate_payments        # no storage
       ├─ settle_withdraw
       │    ├─ enforce_spoke_asset_flags      # READ spoke asset (cache)
       │    ├─ get_supply_position_or_panic   # memory
       │    ├─ apply_withdraw_batch
       │    │    ├─ pool_withdraw_call        # CROSS-CONTRACT pool storage + token
       │    │    └─ merge_withdraw_leg × N
       │    │         ├─ put_market_index     # CACHE only
       │    │         ├─ apply_leg_usage Exit → apply_spoke_exit  # BUFFER usage
       │    │         ├─ maybe refresh_supply_risk_params         # memory
       │    │         ├─ update_or_remove_supply_position         # memory
       │    │         └─ record_supply_position_update            # event buffer
       ├─ enforce_post_pool_solvency
       │    ├─ restamp_listed_supply_ltv      # memory LTV only
       │    └─ require_post_pool_risk_gates   # READ prices/indexes; may panic
       └─ finalize_position_flow(..., Supply, remove_if_empty=true)
            ├─ persist_spoke_usage            # WRITE SpokeUsage keys
            ├─ persist_account_positions
            │    ├─ set_supply_positions      # WRITE/REMOVE SupplyPositions
            │    ├─ (skip) set_debt_positions
            │    ├─ renew_user_account        # TTL live account keys
            │    └─ cleanup_account_if_empty  # maybe REMOVE all account keys + NFT burn
            └─ emit_position_batch            # events only
```

---

## Durable write inventory

| Step | Key / surface | Mechanism | When | Value mutation? |
|---|---|---|---|---|
| Cache construction | Controller instance | `renew_controller_instance` / `extend_ttl` | Always at `Cache::new` | No (TTL only) |
| Per-leg usage buffer | in-memory `SpokeUsageContext` | `apply_exit` | Each successful pool leg with nonzero scaled delta **and** existing usage row | Not durable yet |
| Finalize usage | `ControllerKey::SpokeUsage(spoke, hub)` | `set_spoke_usage` via `set_shared` or `remove` if both sides zero | If any spoke-usage context was created and rows cached | Yes — supply RAY ↓ (borrow side preserved) |
| Finalize positions | `ControllerKey::SupplyPositions(id)` | `set_supply_positions` / `persistent.remove` if map empty | Always on success path | Yes — scaled shares + stamped LTV/LT/bonus/fees |
| Finalize positions | `BorrowPositions` / `AccountMeta` / `Delegates` | **not written** on happy path | — | No |
| Finalize TTL | meta / supply / debt / delegates if `has` | `renew_user_account` | Always after position write | No (TTL only) |
| Empty cleanup | meta + supply + debt + delegates | `remove_account_entry` | `remove_if_empty && account.is_empty()` | Yes — delete |
| Empty cleanup | Position NFT | `nft_burn_call` | Same as above | Cross-contract burn |
| Market indexes | controller persistent | none | `put_market_index` is cache-only | No on controller |
| Protocol config (pool, oracle, NFT addr, limits) | instance / shared | none on this path | — | No |
| Flash flag | temporary `FlashLoanOngoing` | none (read-only check) | — | No |
| Events | — | `UpdatePositionBatchEvent` | After durable writes | Observational |

### Explicit non-writes (important)

- **No `set_debt_positions`**: `PositionSides::Supply` skips the debt map. In-memory borrow map is loaded for solvency but never persisted back — intentional; avoids clobbering debt with a stale empty map.
- **No `set_account_meta`**: spoke/mode unchanged.
- **No delegate map mutation** except full-account `remove_account_entry` on empty cleanup.
- **No spoke asset / spoke config / hub / pool / oracle / accumulator writes**.
- **No controller-side market-index persistence** — pool remains source of truth (see A038 theme; agrees with A086 cache inventory).

---

## Phase analysis

### 1. Gates before any durable write

1. `caller.require_auth` + `require_not_flash_loaning` (`risk/validation.rs:12-23`).
2. `storage::get_account` — fail-closed if meta or NFT owner missing.
3. `require_owner_or_delegate` — stranger cannot shrink foreign collateral (INV-AUTH-02 / A003).
4. `require_external_recipient` — rejects pool or controller as `to` (stranding defense; money path peer).
5. Per leg: `FreezePolicy::AllowOnExit` — paused listing blocks; frozen allowed; missing listing tolerated so delisted collateral stays exitable (`enforce_spoke_asset_flags`).
6. `get_supply_position_or_panic` — cannot invent a supply slot to withdraw.

No `#[when_not_paused]` on `withdraw` — intentional INV-HALT-01 exit liveness (A001). Storage may therefore change during global pause; that is policy, not a write bug.

### 2. Pool mutation then memory merge (pre-persist)

`pool_withdraw_call` mutates **pool** books and pays the recipient before controller durable writes. Each result becomes `LegOutcome`; `merge_withdraw_leg`:

1. Decides `may_restamp` **before** mutating the position (liquidation kind / unlisted / full exit → no FullTuple restamp).
2. Sets `position.scaled_amount = outcome.new_scaled` (pool output — A082).
3. Buffers market index in `Cache` only.
4. `apply_leg_usage(..., LegDirection::Exit)` → `old_scaled - new_scaled` into `apply_spoke_exit` (INV-HALT-03: exits do not consume caps).
5. Optionally `refresh_supply_risk_params(..., FullTuple)` into memory.
6. `update_or_remove_supply_position` — drops map entry when `scaled_amount == Ray::ZERO`.
7. Buffers supply event delta (amount = `outcome.amount`).

Multi-asset batches coalesce all legs in memory; a single finalize writes once (same pattern as A032).

### 3. Post-pool solvency before persist

`enforce_post_pool_solvency` restamps listed LTVs in memory, then `require_post_pool_risk_gates` (A072). Debt-free accounts skip numerical gates (full collateral exit allowed even with broken oracle — harness `test_withdraw_full_exit_works_with_broken_oracle`). Any panic here aborts the tx: pool transfer and buffered usage never commit.

### 4. Finalize write batch (the only controller accounting commit)

Order in `finalize_position_flow` (`positions/mod.rs:241-252`):

1. **`persist_spoke_usage`** — every cached usage row for the spoke context.
2. **`set_supply_positions`** — full map replace, or key removal if empty (`storage/account.rs:71-102`).
3. **`renew_user_account`** — TTL on whichever of meta/supply/debt/delegates still exist.
4. **`cleanup_account_if_empty`** when `remove_if_empty` (withdraw passes `true`, unlike supply’s `false` — A036).
5. **`emit_position_batch`** — after durable state (A033).

`set_spoke_usage` removes the key when both supply and borrow RAY are zero; otherwise writes the full row so a supply exit to zero leaves borrow usage intact.

---

## Invariant / defense checklist

| Claim | Verdict | Notes |
|---|---|---|
| Only owner/delegate can trigger these writes | Match | Before pool and before finalize |
| Supply shares only fall / remove on this path | Match | Pool exit + `new_scaled`; no supply entry merge |
| Debt map not silently zeroed | Match | `PositionSides::Supply` skips debt write |
| Spoke usage delta from pool scaled, not request amount | Match | `apply_leg_usage` Exit; Certora `usage_withdraw_tracks_scaled_delta` |
| Exit does not hit supply cap | Match | `apply_exit` never calls `enforce_spoke_cap` |
| Empty account deletes storage + burns NFT together | Match | `remove_account_and_burn_nft` only remover (INV-STOR-03 / A031) |
| Events not source of truth | Match | Emitted after persist |
| Pause still allows exit writes | Match | By design |
| Unlisted asset still exitable | Match | Flag check no-ops; restamp skipped |
| Missing usage row decrements usage | **Residual** | A080 no-op; capacity soft overstatement |

---

## Residual / non-gaps

### Residual (shared) — missing usage row

If `SpokeUsage` was never recorded (or already cleared) while a live supply position exists, `apply_exit` returns without buffering a row (`spoke_usage.rs:130-132`). Finalize then has nothing to decrement. Account `SupplyPositions` still shrink correctly. Spoke cap can admit more entry than true aggregate exposure until governance/reconcile. Severity owned by A080; withdraw is a primary caller of that exit path.

### Non-gaps

- **Debt not rewritten**: loading full account for HF while writing only supply is correct; rewriting debt from the loaded map would be redundant, and a borrow-only load would break solvency.
- **Cache market indexes**: not durable on controller; using pool-returned indexes for the rest of the tx is intended (A086/A094).
- **`let _ = enforce_post_pool_solvency(...)`**: discards the “did LTV change?” bool only; mutations remain on `account` and are persisted.
- **TTL renew before cleanup**: harmless; removed keys are gone after cleanup.
- **Strategy/liquidation sharing `merge_withdraw_leg`**: same memory/buffer semantics; their finalize/`remove_if_empty` flags differ (A026/A032) — not a `process_withdraw` defect.

---

## Tests / formal anchors

| Check | Location |
|---|---|
| Partial / full / multi-asset withdraw | `tests/test-harness/tests/controller/withdraw.rs` |
| Position removed when empty; other hubs remain | `test_withdraw_removes_position_when_empty` |
| Empty account auto-removed | `test_withdraw_cleans_up_empty_account` |
| HF / LTV gates block unsafe exit | `test_withdraw_rejects_exceeding_hf`, `test_withdraw_rejects_when_above_ltv_but_hf_ok` |
| Allowed while paused | `test_withdraw_allowed_when_paused` |
| Flash reentrancy blocked | `test_withdraw_rejects_during_flash_loan` |
| Usage tracks withdraw scaled delta | Certora `usage_withdraw_tracks_scaled_delta` |
| Exit without usage row is noop | Certora `usage_exit_without_usage_row_is_a_noop` |
| Empty supply map removes storage key | `contracts/controller/tests/storage/account.rs` `set_supply_positions_empty_map_removes_key` |
| NFT burn on empty | INV-STOR-03 harness in `position_nft.rs` |

---

## Cross-links

- **A003** — auth gate before collateral leaves.
- **A031 / A036** — cleanup + NFT pairing; withdraw correctly sets `remove_if_empty: true`.
- **A032 / A033** — single finalize batch; events after durable writes.
- **A072** — post-pool solvency before persist.
- **A076 / A082** — exit usage semantics; pool outputs drive deltas.
- **A080** — only open storage-semantics residual on this path.
- **A022** (peer) — supply path contrast: entry usage + caps + `remove_if_empty: false`.

---

## Verdict

`process_withdraw`’s controller storage writes are **defended**: minimal write set, correct side selection, pool-output accounting, solvency-before-persist, empty-account lifecycle paired with NFT burn, and no protocol-config mutation. Record the A080 missing-row exit no-op as a shared soft-cap integrity residual, not as an under-defended withdraw bookkeeping bug.
