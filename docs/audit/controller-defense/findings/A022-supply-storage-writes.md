# A022 — Supply path storage writes (`process_supply`)

- Agent: A022
- Theme: T2 (shares / meta / spoke usage / events via `finalize_position_flow`)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs:98-107` (`Controller::supply`)
  - `contracts/controller/src/positions/supply.rs:44-155`, `:308-357` (`process_supply`, `process_deposit`, `settle_supply`, `merge_supply_leg`)
  - `contracts/controller/src/positions/mod.rs:112-141`, `:206-252`, `:292-327` (`apply_leg_usage`, `finalize_position_flow`, `persist_account_positions`, entry gates)
  - `contracts/controller/src/account.rs:47-114`, `:172-207` (`create_account_with` / `load_or_create_account`, `update_or_remove_supply_position`)
  - `contracts/controller/src/storage/account.rs:53-103`, `:247-270` (`set_account_meta`, `set_supply_positions`, `write_side_map`, `renew_user_account`, `remove_account_entry`)
  - `contracts/controller/src/storage/spoke.rs:56-79` (`set_spoke_usage`)
  - `contracts/controller/src/context/{mod,spoke,market_index,events}.rs` (Cache buffers + persist/emit)
  - `contracts/controller/src/spoke_usage.rs:61-160` (`apply_entry` / `persist`)
  - `contracts/controller/src/risk/params.rs:17-91` (`refresh_supply_risk_params` FullTuple)
  - `contracts/controller/src/payments.rs`, `common/src/token.rs:19-34` (measured inbound receipt)
  - `contracts/pool/src/ops/supply.rs:19-42` (pool share mint + cash credit)
  - `scripts/permissionless_entrypoints.txt` (`controller::supply`)
- Defense: Durable controller writes for the bare `supply` verb are batched at one tail — `persist_spoke_usage` → `set_supply_positions` (+ TTL renew) → `emit_position_batch` — after pool mutation returns. Account shares take **pool** `new_scaled`; spoke-usage entry deltas are `new_scaled − old_scaled` with cap check on the **returned** supply index; event `scaled_amount` / risk stamps mirror the in-memory position about to be persisted. `AccountMeta` is written only on `account_id==0` create (spoke/mode), never restamped on top-up. `PositionSides::Supply` + `remove_if_empty=false` skip debt-map writes and empty-account burn. Third-party top-ups cannot open new slots (INV-AUTH-03) but may restamp risk params on an existing slot as a side effect of `merge_supply_leg`.
- Gap: No novel fund-theft or share-inflation write-set bug on this path. Residuals (not unique to supply): (1) third-party top-up triggers `refresh_supply_risk_params(FullTuple)` on the victim’s existing slot — LTV always restamped; liquidator-favoring LT/bonus/fees gated (cross A012 / keepers); (2) shared T5 A080 is exit-side only — supply **creates/updates** usage rows on entry, so missing-row no-op does not apply to this verb’s usage write; (3) `transfer_amount_measured` does not require `delta == requested` — fee-on-transfer under-credits shares (user loss), malicious credit-on-transfer over-credits if governance lists such a token (A041 / A055); (4) strategy reuse of `process_deposit` / `merge_supply_leg` finalizes elsewhere (A032 / A007 residual on post-guard listed-token reentry).
- Impact: Successful supply can (1) mint NFT + write `AccountMeta` on create, (2) increase/create per-account supply shares and stamped risk params under `SupplyPositions`, (3) increase `SpokeUsage` supply RAY (cap-gated) for touched hubs, (4) renew instance + live user TTLs, (5) emit one `UpdatePositionBatchEvent`. Cannot mint debt, rewrite `BorrowPositions` / delegates, retarget protocol instance keys, or delete the account. Blast radius of a hypothetical skipped finalize after pool success would desync controller books vs pool — prevented by Soroban whole-tx atomicity + mandatory finalize on the success path.
- Evidence: INV-AUTH-03, INV-ACCT-03/05, INV-HALT-01/03, INV-STOR-01/03, INV-STOR spoke-cap language; Certora `usage_supply_tracks_scaled_delta`, `supply_new_slot_requires_owner_or_delegate`, spoke Wave0 `SupplyEntry` → `merge_supply_leg`; harness `tests/test-harness/tests/controller/supply.rs`, `spoke_caps.rs`, `position_nft.rs`; unit `contracts/controller/tests/storage/account.rs`, `tests/spoke.rs`, `tests/events.rs`; peers A004, A012, A021, A024, A032, A033, A036, A041, A076, A077, A082, A094.
- Opinion: Supply’s controller write surface is narrow, correctly sided, and ordered (durable before events). Pool outputs drive shares and usage; measured receipt drives the pool credit amount. Treat third-party risk restamp as an intentional maintenance side effect, not a storage integrity failure. Mirror A024’s discipline: any new supply-adjacent pool FFI must still funnel through `merge_supply_leg` + `finalize_position_flow`.

## Scope

Inventory every **controller** storage mutation reachable from `Controller::supply` → `process_supply`, including create-on-zero, in-memory share/risk merges, spoke-usage buffering, and the `finalize_position_flow` tail (persist usage, persist supply map, emit events).

Out of scope as primary claims (peer agents): withdraw legs in the same file (A024), account key layout generally (A021), third-party slot auth predicate detail (A012), strategy finalize batching (A032), event-vs-storage order abstractly (A033), lying/fee-on-transfer token taxonomy (A041/A055), spoke-usage exit missing-row (A080).

---

## Call graph (storage-relevant)

```
Controller::supply                            # #[when_not_paused]
  └─ process_supply
       ├─ require_authorized_caller           # auth + flash-guard READ (temp)
       ├─ aggregate_positive_payments         # no storage; reject empty/zero/neg
       ├─ Cache::new → renew_controller_instance   # TTL instance
       ├─ load_or_create_account(..., Supply)
       │    ├─ account_id==0 → create_account_with
       │    │    ├─ active_spoke READ
       │    │    ├─ nft_mint_call             # CROSS-CONTRACT NFT mint (+ NFT TTLs)
       │    │    └─ set_account_meta          # WRITE AccountMeta {spoke, mode=Normal}
       │    └─ else → get_account READ + require_spoke_match
       ├─ require_third_party_existing_supply # READ-only auth over existing slots
       ├─ process_deposit
       │    ├─ validate_position_entry_gates  # READ hub/spoke/asset; bulk limits
       │    └─ settle_supply
       │         ├─ transfer_amount_measured  # CROSS-CONTRACT token → pool (measured Δ)
       │         ├─ get_or_create_supply_position  # &self; NO map insert if missing
       │         ├─ pool_supply_call          # CROSS-CONTRACT pool shares + cash
       │         └─ merge_supply_leg × N
       │              ├─ refresh_supply_risk_params(FullTuple)  # memory stamps
       │              ├─ position.scaled_amount = outcome.new_scaled
       │              ├─ apply_leg_usage Entry → apply_spoke_entry (+ cap)  # BUFFER
       │              ├─ put_market_index     # CACHE only
       │              ├─ record_supply_position_update  # event BUFFER
       │              └─ update_or_remove_supply_position  # memory map
       └─ finalize_position_flow(..., Supply, remove_if_empty=false)
            ├─ persist_spoke_usage            # WRITE SpokeUsage keys
            ├─ persist_account_positions
            │    ├─ set_supply_positions      # WRITE/REMOVE SupplyPositions
            │    ├─ (skip) set_debt_positions
            │    ├─ renew_user_account        # TTL live account keys
            │    └─ (skip) cleanup_account_if_empty
            └─ emit_position_batch            # events only (after durable)
```

Shared helpers: strategies (`multiply`, `flash_position`, `migrate_blend`, `swap_collateral`) call `process_deposit` / `merge_supply_leg` but finalize via `strategy_finalize` (A032) — same merge write semantics, different outer persist sides/flags.

---

## Durable write inventory

| Step | Key / surface | Mechanism | When | Value mutation? |
|---|---|---|---|---|
| Cache construction | Controller instance | `renew_controller_instance` / `extend_ttl` | Always at `Cache::new` | No (TTL only) |
| Create (`account_id==0`) | Position NFT | `nft_mint_call` → sequential mint to caller | Create only | Cross-contract mint |
| Create | `ControllerKey::AccountMeta(id)` | `set_account_meta` | Create only | Yes — `{spoke_id, PositionMode::Normal}` |
| Per-leg usage buffer | in-memory `SpokeUsageContext` | `apply_entry` (+ cap) | Each successful pool leg | Not durable yet |
| Finalize usage | `ControllerKey::SpokeUsage(spoke, hub)` | `set_spoke_usage` / remove if both sides 0 | If spoke-usage context loaded and rows cached | Yes — supply RAY ↑ (borrow side preserved) |
| Finalize positions | `ControllerKey::SupplyPositions(id)` | `set_supply_positions` / remove if map empty | Always on success path | Yes — scaled shares + stamped LTV/LT/bonus/fees |
| Finalize positions | `BorrowPositions` / `Delegates` | **not written** | — | No |
| Finalize positions | `AccountMeta` | **not rewritten** on top-up | Create already wrote it | No on top-up |
| Finalize TTL | meta / supply / debt / delegates if `has` | `renew_user_account` | Always after position write | No (TTL only) |
| Empty cleanup | account keys + NFT | **skipped** (`remove_if_empty=false`) | — | No |
| Market indexes | controller persistent | none | `put_market_index` is cache-only | No on controller |
| Protocol config | instance / shared | none on this path | — | No |
| Flash flag | temporary `FlashLoanOngoing` | none (read-only check) | — | No |
| Events | ledger events | `UpdatePositionBatchEvent` | After durable writes | Observational |

### Explicit non-writes (important)

- **No `set_debt_positions`**: `PositionSides::Supply` skips the debt map. Account is loaded with both maps; only supply is persisted — avoids clobbering live debt with an untouched in-memory copy policy that matches A021/A024.
- **No `set_account_meta` on top-up**: spoke/mode immutable after create (A021). `AccountGuard::Supply` only enforces spoke match vs the call argument.
- **No delegate mutation**.
- **No account deletion / NFT burn** on this verb (`remove_if_empty=false`). Contrast withdraw (A024) and strategy finalize.
- **No premature durable zero-slot**: `Account::get_or_create_supply_position` takes `&self` and returns an ephemeral zero position without inserting into `supply_positions`; durable insert happens only after pool returns nonzero scaled via `update_or_remove_supply_position`.

---

## 1. Shares (`SupplyPositions`)

### 1.1 Source of truth

```331:356:contracts/controller/src/positions/supply.rs
    let outcome = LegOutcome::from(result);
    position.scaled_amount = outcome.new_scaled;
    // ...
    update_or_remove_supply_position(account, hub_asset, &position);
```

- Pool `PoolPositionMutation.position.scaled_amount` → `LegOutcome.new_scaled` → account share balance.
- Caller-requested amounts and even the measured transfer amount are **not** written as shares; they only seed `PoolAction.amount` for the pool to mint from (`settle_supply` → `make_pool_action(..., received, ...)`).
- Pool enforces INV-ACCT-05: positive amount that mints zero shares panics (`SupplyRoundsToZeroShares`). Zero-amount no-op is allowed only when mint is zero — not reachable from `aggregate_positive_payments` (rejects 0).
- Zero post-leg scaled removes the map entry (`update_or_remove_supply_position`); supply entry legs only increase scaled, so removal on this verb is not a realistic success outcome.

### 1.2 Stamped risk meta colocated with shares

Each `AccountPositionRaw` stores RAY shares plus BPS `liquidation_threshold` / `liquidation_bonus` / `loan_to_value` / `liquidation_fees`. On every supply leg, `merge_supply_leg` calls `refresh_supply_risk_params(..., FullTuple)` **before** assigning `new_scaled`:

- `loan_to_value` always restamped from current listed config.
- LT / bonus / fees go through `apply_gated_liquidation_params` (block liquidator-favoring updates when account has debt and hypothetical HF would fall below 1.05 WAD).

Gating HF uses the **pre-deposit** scaled amount still present in the account map / local position at refresh time — slightly stricter against liquidator-favoring restamps than a post-deposit HF would be (borrower-favorable residual; not a storage integrity bug).

### 1.3 Persist shape

`finalize_position_flow` → `persist_account_positions(..., PositionSides::Supply, false)` → `set_supply_positions` writes the **entire** in-memory supply map (not a per-key patch). Empty map removes the persistent key (`write_side_map`). Then `renew_user_account` extends TTL on every live sibling key that `has` (meta/supply/debt/delegates) — INV-STOR-01.

### 1.4 Multi-leg batching

All legs mutate memory + Cache first; one durable supply-map write at the end. Mid-batch cap failure or pool panic aborts the whole Soroban transaction (token transfer + pool mint + any create meta/NFT roll back together).

---

## 2. Meta (`AccountMeta`)

| Case | Meta write? | Notes |
|---|---|---|
| `account_id == 0` | Yes, once at create | `spoke_id` from args; `mode = PositionMode::Normal` forced |
| Existing account | No | Spoke must match (`AccountGuard::Supply`); mode untouched |
| Third-party top-up | No | Cannot change spoke/mode; cannot open new hub slots (A012) |

Ownership is never written to controller storage (NFT `owner_of` — A004/A021). Create couples mint recipient to authorizing `caller`.

---

## 3. Spoke usage

### 3.1 Entry semantics on supply

```334:345:contracts/controller/src/positions/supply.rs
    apply_leg_usage(
        env,
        cache,
        account.spoke_id,
        UsageSide::Supply,
        hub_asset,
        LegDirection::Entry {
            asset_decimals: result.asset_decimals,
        },
        old_scaled,
        &outcome,
    );
```

- Delta = `outcome.new_scaled.checked_sub(old_scaled)` (pool truth − preimage shares).
- `Cache::apply_spoke_entry` loads listed cap, uses **outcome** market supply index + pool-reported decimals, then `SpokeUsageContext::apply_entry` → `enforce_spoke_cap` (`calculate_scaled_cap`). Cap breach → `SpokeSupplyCapReached`; no partial durable usage write without finalize (and tx reverts).
- Absent usage row starts at zero and is written into the in-memory map unconditionally on entry — supply **does not** share A080’s exit missing-row no-op.

### 3.2 Persist

`cache.persist_spoke_usage()` writes every cached row via `set_spoke_usage`. Both-zero rows are removed from shared persistent storage. Borrow-side RAY on the same key is preserved when only supply changes.

### 3.3 Ordering vs cash movement

Tokens are transferred to the pool and `pool_supply_call` commits **before** usage/cap enforcement in `merge_supply_leg`. A cap miss after the pool leg still reverts the full transaction — no stranded pool mint without controller usage/account commit. INV-HALT-03: exits do not consume caps; this entry path does.

Certora: `usage_supply_tracks_scaled_delta` (endpoint through `process_supply` + finalize); Wave0 `SupplyEntry` wires `merge_supply_leg`.

---

## 4. Events via `finalize_position_flow`

### 4.1 Buffer then emit

`record_supply_position_update` pushes `EventDepositDelta` during `merge_supply_leg`. `emit_position_batch` runs **after** `persist_spoke_usage` and `persist_account_positions` (A033). Empty buffers → no-op publish.

### 4.2 Payload vs storage

| Event field | Source on supply leg | Matches durable state? |
|---|---|---|
| `scaled_amount` | post-merge `position` | Yes — same object persisted |
| risk params (LT/LTV/bonus/fees) | post-refresh `position` | Yes |
| `index_ray` | `outcome.market_index.supply_index` | Cache-updated; pool is SoT |
| `amount` | `action.amount` (= measured transfer into pool) | Pool `actual_amount` equals credited `amount` on supply (`pool/ops/supply.rs` echoes it) |

Contrast withdraw (`merge_withdraw_leg` records `outcome.amount`). For supply, measured receipt and pool `actual_amount` coincide by pool design; observational consistency holds without a separate controller equality assert on this verb (strategy custody legs assert elsewhere — A082).

### 4.3 Account attributes on the batch

`UpdatePositionBatchEvent.account_attributes` comes from the in-memory `Account` (spoke/mode). Those values originated from create meta or load; supply does not mutate them.

---

## 5. Trust boundary: measured receipt → pool credit → controller shares

```
caller token  --transfer_amount_measured-->  pool balance Δ (= received)
received      --PoolAction.amount-------->  pool mint + credit_cash(amount)
pool mutation --new_scaled--------------->  account shares + usage Δ
```

- INV-ACCT-03: inbound credit uses measured receipt, not the caller’s requested figure alone.
- `transfer_amount_measured` does **not** require `Δ == amount`. Under-delivery → fewer shares (user loss). Over-delivery on a malicious listed token → excess shares vs economic intent (governance token trust — A041/A055). Protocol cash book on the pool still tracks what the pool credited.

No post-pool solvency check on supply (collateral-increasing): intentional; contrast withdraw/borrow (A072).

---

## 6. Create vs top-up write-set comparison

| Write | New account (`id=0`) | Owner/delegate top-up | Third-party top-up |
|---|---|---|---|
| NFT mint | Yes (to caller) | No | No |
| `AccountMeta` | Yes | No | No |
| Open new hub supply slot | Yes (caller is owner) | Yes | **No** (`NotAuthorized`) |
| Increase existing slot shares | Yes | Yes | Yes |
| Restamp risk on slot | Yes | Yes | Yes (side effect) |
| Spoke usage ↑ + cap | Yes | Yes | Yes |
| `SupplyPositions` persist | Yes | Yes | Yes |
| `remove_if_empty` cleanup | No | No | No |
| Debt / delegates | No | No | No |

---

## 7. Failure / atomicity

| Failure point | Durable controller state | Pool / token / NFT |
|---|---|---|
| Auth / aggregate / gates | Unchanged | Unchanged |
| Create mint then later panic | Full tx revert | Mint/meta revert with tx |
| Transfer then pool/cap/merge panic | Full tx revert | Transfer + pool revert |
| After successful finalize | Committed write-set above | Committed |

There is no success path that returns from `process_supply` without `finalize_position_flow`. In-memory-only windows exist only until the end of the same invocation (or until a panic aborts).

---

## 8. Cross-links

| Peer | Relation |
|---|---|
| A021 | Account key family / meta-as-existence / empty-map deletion — supply respects `PositionSides::Supply` |
| A024 | Mirror inventory for withdraw; supply uses `remove_if_empty=false` and Entry usage+cap |
| A012 / A004 | Third-party slot rule + create ownership |
| A032 / A033 | Finalize batching and durable-before-events order |
| A036 | Empty cleanup asymmetry — supply intentionally never cleans |
| A076 / A077 / A082 | Usage/cap/pool-output trust boundary |
| A080 | Exit missing-row residual — **not** on supply entry |
| A041 / A055 | Measured deposit / lying token residuals |
| A007 | `process_deposit` reused under strategies after flash-guard windows |
| A094 | `put_market_index` after merge required for later same-tx risk math |

---

## 9. Verdict

Supply-path controller storage writes are **defended**: create meta/NFT are correctly coupled to the authorizing caller; shares and spoke-usage entry track pool-scaled outputs; caps use post-mutation indexes; events are observational and emitted after durable commits; debt/meta/delegates are not clobbered on top-up; empty-account burn is correctly omitted. No undefended write that lets a stranger forge shares, rewrite foreign meta, or skip usage accounting on a successful entry leg. Residual notes are governance-token trust, intentional third-party risk restamp, and shared strategy finalize concerns — not a broken supply write-set.
