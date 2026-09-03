# A021 — Account storage layout (meta / supply / debt / delegates)

- Agent: A021
- Theme: T2
- Severity: low
- Status: defended
- Paths:
  - `common/src/types/controller.rs:53-74` (`AccountMeta`, `DelegateGrant`)
  - `common/src/types/controller.rs:311-365` (`Account`, `is_empty` / `debt_free`)
  - `common/src/types/controller.rs:534-565` (`ControllerKey` account variants)
  - `common/src/types/pool.rs:226-332` (`AccountPositionRaw` / `DebtPositionRaw`)
  - `common/src/types/pool.rs:465-468` (`HubAssetKey` map key)
  - `contracts/controller/src/storage/account.rs:11-312` (full account storage surface)
  - `contracts/controller/src/storage/protocol.rs:159-205` (`get_user` / `set_user` / `renew_user_key`)
  - `contracts/controller/src/account.rs:47-176` (create, owner gates, in-memory upsert, cleanup)
  - `contracts/controller/src/account.rs:223-291` (renew / delegate entrypoints)
  - `contracts/controller/src/positions/mod.rs:206-252` (`PositionSides`, `persist_account_positions`, `finalize_position_flow`)
  - `contracts/controller/src/positions/supply.rs:44-77`, `:161-189` (supply / withdraw persist)
  - `contracts/controller/src/positions/debt.rs:35-112` (borrow / repay persist + borrow-only load)
  - `contracts/controller/src/positions/liquidation/mod.rs:126-147` (victim Both / receiver Supply)
  - `contracts/controller/src/positions/liquidation/apply.rs:323-340` (post-liq empty cleanup)
  - `contracts/controller/src/positions/liquidation/bad_debt.rs:61` (forced full remove)
  - `contracts/controller/src/strategies/mod.rs:79` (strategy finalize Both + remove_if_empty)
  - `contracts/controller/src/keepers.rs:148-219` (threshold restamp: supply write only)
  - `contracts/controller/src/views.rs:100-128` (existence = meta present)
  - `contracts/controller/src/constants.rs:17` (`MAX_DELEGATES = 16`)
- Defense: Four persistent keys keyed by `u64` account id; ownership never stored on controller (NFT `owner_of`); empty side maps deleted; meta is the existence sentinel; spoke/mode written once at create; side-selective persist paired with load shape; cleanup burns NFT only via `remove_account_and_burn_nft`
- Gap: residual empty-shell rent when positions become empty on paths with `remove_if_empty = false` (notably full repay with no remaining supply) — coincides with A036; not a fund-control gap. Documented NFT-transfer delegate re-arm if prior owner regains NFT before any delegate write (A005)
- Impact: Layout compromise would mean lost/stolen shares or debt forgery per account. Observed residual is rent + stranded NFT/meta for emptied shells until TTL archive or a later cleanup path — not cross-account fund theft. Blast radius of a hypothetical side-wipe bug (`PositionSides::Both` after borrow-only load) would be full collateral erasure for that account; current call sites avoid that coupling
- Evidence: INV-STOR-01, INV-STOR-03, INV-AUTH-02, INV-AUTH-06; unit `contracts/controller/tests/storage/account.rs`; harness `test_withdraw_cleans_up_empty_account`, `test_account_auto_removed_after_full_repay_withdraw`, `emptying_account_burns_nft_and_resupply_mints_fresh_id`; peers A004, A005, A017, A031, A036
- Opinion: The split-key layout is coherent and defensive. Meta-as-existence, NFT-as-owner, empty-map deletion, and `PositionSides` + load-shape pairing are the load-bearing defenses. No undefended mutation path lets a stranger rewrite another account's four keys. Residual cleanup asymmetry is operational (rent), not accounting integrity.

---

## 1. Key family (persistent, per `account_id: u64`)

| `ControllerKey` variant | Value type | Role |
|---|---|---|
| `AccountMeta(u64)` | `AccountMeta { spoke_id, mode }` | Existence sentinel + immutable spoke/mode |
| `SupplyPositions(u64)` | `Map<HubAssetKey, AccountPositionRaw>` | Collateral shares + stamped risk params |
| `BorrowPositions(u64)` | `Map<HubAssetKey, DebtPositionRaw>` | Debt shares (scaled amount only) |
| `Delegates(u64)` | `DelegateGrant { granted_by, delegates }` | Owner-stamped manager list |

Declared at `common/src/types/controller.rs:561-564`. All four live in **persistent** storage under the user TTL class (`TTL_THRESHOLD_USER` / `TTL_BUMP_USER`), not instance storage (contrast protocol singletons — A029).

**Not stored on the controller:** account owner. `Account.owner` is assembled at read time from `position_nft.owner_of(account_id)` (`storage/account.rs:28-40`, `:148-157`). Matches INV-STOR-03 / A004 / A031.

**Adjacent, same file, not account layout:** temporary `SessionKey::FlashLoanOngoing` (`storage/account.rs:276-312`) — tx-scoped reentrancy flag (A007 / A030).

## 2. Value shapes

### 2.1 `AccountMeta`

```58:61:common/src/types/controller.rs
pub struct AccountMeta {
    pub spoke_id: u32,
    pub mode: PositionMode,
}
```

Production write sites of `set_account_meta`: **only** `create_account_with` (`account.rs:74`). Spoke binding and mode are therefore storage-immutable after mint (INV-AUTH-06 for spoke; mode admission for strategies is A018). No admin/user path restamps meta.

### 2.2 Supply map values

`AccountPositionRaw`: `scaled_amount` (RAY i128) plus stamped `liquidation_threshold` / `liquidation_bonus` / `loan_to_value` / `liquidation_fees` (u32 BPS). Zero-scaled entries are removed in memory before persist (`update_or_remove_supply_position`, `account.rs:197-207`).

### 2.3 Debt map values

`DebtPositionRaw`: `scaled_amount` only. Zero-scaled entries removed via `update_or_remove_debt_position` (`account.rs:211-221`).

### 2.4 Delegates

`DelegateGrant` stamps `granted_by`. `get_delegates` returns empty unless `granted_by ==` current NFT owner (`storage/account.rs:170-177`) — lazy revoke on transfer. Cap `MAX_DELEGATES = 16` (`constants.rs:17`). Detail: A005 / A037.

### 2.5 Assembled `Account`

```315:324:common/src/types/controller.rs
pub struct Account {
    pub owner: Address,
    pub spoke_id: u32,
    pub mode: PositionMode,
    pub supply_positions: Map<HubAssetKey, AccountPositionRaw>,
    pub borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
}
```

`is_empty` ⇔ both maps empty; `debt_free` ⇔ borrow map empty (`controller.rs:357-365`). Cleanup uses `is_empty`, not meta absence alone.

## 3. Read / write primitives

| Op | Behavior |
|---|---|
| `try_get_account_meta` | `get_user` → renews TTL if present; `None` = no account |
| `get_account_meta` | panics `AccountNotInMarket` (#13) |
| `set_account_meta` | `set_user` (write + user TTL renew) |
| `get_supply_positions` / `get_debt_positions` | missing key → **empty `Map`**, not panic |
| `set_supply_positions` / `set_debt_positions` | `write_side_map`: empty → `persistent.remove`; else raw `persistent.set` **without** `set_user` renew |
| `get_delegates` | stale grant filtered to empty vec |
| `set_delegates` (private) | empty → remove; else `set_user` with new `DelegateGrant` |
| `remove_account_entry` | removes all four keys unconditionally |
| `renew_user_account` | `extend_ttl` on each key that `has` |

Existence for views: `account_exists` ≡ meta present (`views.rs:100-103`). Missing side maps do not make the account “not exist.”

**TTL design:** side writers deliberately do not co-renew siblings (`tests/storage/account.rs:302-357`). Callers (`persist_account_positions`, keeper sync, `renew_account`) call `renew_user_account` so a Both-side write renews once. Meta/delegate writes via `set_user` renew only their own key — sibling isolation is intentional (A017).

## 4. How user interactions mutate the maps

### 4.1 Create (`account_id == 0`)

1. Mint NFT to caller (`nft_mint_call`).
2. Write **only** `AccountMeta` (`account.rs:74`).
3. Supply / debt / delegates keys absent (reads as empty).
4. Same tx continues into position flow, which then writes the relevant side map(s).

No owner field write. Third-party cannot create foreign-owned meta (A004).

### 4.2 Common persist tail

```218:235:contracts/controller/src/positions/mod.rs
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

`finalize_position_flow` order: spoke-usage persist → position maps → batch event (`positions/mod.rs:241-252`). Account keys are never written mid-leg inside a multi-leg strategy (A032).

### 4.3 Per-path mutation matrix

| User / system path | Load shape | Sides written | Meta | Delegates | `remove_if_empty` |
|---|---|---|---|---|---|
| `process_supply` | load_or_create (full) | Supply | create only if id=0 | untouched | false |
| `process_withdraw` | full | Supply | untouched | untouched | **true** |
| `process_borrow` | full | Debt, or Both if LTV restamp | untouched | untouched | false |
| `process_repay` | **borrow-only** | Debt | untouched | untouched | false |
| Strategies (`strategy_finalize`) | full (in memory) | Both | create if id=0 | untouched | **true** |
| Liquidation victim | full | Both | untouched | untouched | false; then `check_bad_debt` may cleanup |
| Liquidation Credit receiver | create or full | Supply | create if Credit(0) | untouched | false |
| Bad-debt winddown | n/a | `remove_account_entry` all four | removed | removed | forced burn |
| `update_account_threshold` | supply (+ optional debt for HF) | Supply only (if changed) | untouched | untouched | n/a |
| `add_delegate` / `remove_delegate` | meta+NFT owner gate | none | untouched | write/purge | n/a |
| `renew_account` | meta+owner gate | none (TTL only) | TTL | TTL if present | n/a |

### 4.4 In-memory → durable consistency

Positions are mutated on an in-memory `Account`, then whole-map replaced. Zero-scaled slots are dropped from the map before write, so durable maps never retain zero dust rows. Empty whole maps delete the storage key (`write_side_map`, `storage/account.rs:89-103`) — verified by unit tests `:263-299`.

### 4.5 Borrow-only load safety

`process_repay` uses `get_account_borrow_only` (empty supply map in memory) and persists `PositionSides::Debt` only (`debt.rs:90-111`). That pairing is mandatory: persisting `Both` after a borrow-only load would wipe live supply. Borrow restamp correctly loads full account before optional `Both` write (`debt.rs:44-75`). Keeper threshold path never calls `persist_account_positions`; it calls `set_supply_positions` only (`keepers.rs:217-218`).

### 4.6 Destruction pairing

`cleanup_account_if_empty` → `remove_account_and_burn_nft` → `remove_account_entry` (all four keys) then NFT burn (`account.rs:164-176`). Only production removers: empty cleanup, liquidation empty-debt branch (`apply.rs:333-335`), bad-debt path. No public `remove_account` entrypoint (harness `remove_account` is test storage surgery).

## 5. Defenses (layout-level)

1. **Existence vs balances separated** — meta can exist without side keys; side keys cannot authorize an account alone (`try_get_account` requires meta + NFT owner).
2. **Owner not in controller storage** — transfer cannot leave a stale controller owner field.
3. **Empty-map deletion** — no durable zero maps accruing rent under supply/debt keys.
4. **Side-selective writes** — avoid rewriting untouched maps (gas/TTL); coupled to load shape.
5. **Immutable spoke/mode in meta** — no user path rewrites binding under a live id.
6. **Delegate stamp + lazy revoke** — NFT transfer deactivates grants without a storage write.
7. **Atomic create** — mint then meta in one tx; failure rolls both back (INV-STOR-03).
8. **Flash flag isolated** — temporary storage; cannot masquerade as account state.

## 6. Gaps / residuals

| Residual | Severity | Notes |
|---|---|---|
| Empty shell after full repay with no supply (`remove_if_empty=false`) | low | Meta (+ optional Delegates) + NFT remain until TTL or a later cleanup-capable path. Withdraw/strategy/liquidation empty paths do clean. Aligns with A036. Rent only. |
| Delegate grant re-arm if NFT returns to `granted_by` before any delegate write | info | Documented design (`DelegateGrant` docs; A005). |
| `account_exists` ignores NFT | info | Meta-only existence for views; mutators that need owner fail closed via `try_account_owner`. |
| Side writer skips `set_user` renew | info | Compensated by `renew_user_account` on persist; intentional. |

No path found where an unauthorized caller can set/remove another account’s meta, supply, debt, or delegates keys.

## 7. Cross-links

- A004 / A031 — create/burn ↔ NFT pairing; meta-only on create.
- A005 / A037 — delegate map integrity and manager gating.
- A017 / A034 — TTL renew surface over these four keys.
- A022–A027 — per-path write detail (this note is the layout contract those paths must obey).
- A036 — empty-position cleanup residual.
- A029 — non-account key families (instance vs shared persistent).

## 8. Tests anchoring the layout

| Claim | Evidence |
|---|---|
| Empty supply/debt maps remove keys | `set_supply_positions_empty_map_removes_key`, `set_debt_positions_empty_map_removes_key` |
| Side write does not renew siblings | `set_supply_positions_does_not_renew_sibling_ttls` |
| `renew_user_account` co-renews live siblings | `renew_user_account_co_renews_all_live_siblings` |
| Stale delegate purge / lazy empty | `remove_delegate_purges_stale_grant_preventing_resurrection` |
| Withdraw empties → burn | harness `test_withdraw_cleans_up_empty_account` |
| Repay then withdraw empties → burn | `test_repay_cleans_up_empty_account`, `test_account_auto_removed_after_full_repay_withdraw` |
| NFT id == account id; resupply gets new id | `emptying_account_burns_nft_and_resupply_mints_fresh_id` |
