# A063 — Spoke id / hub id existence and active checks on user mutators

- Agent: A063
- Theme: T4 (untrustworthy input validation)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/config/spoke.rs:93-96` (`require_hub_active`)
  - `contracts/controller/src/context/mod.rs:76-83` (`Cache::require_hub_active` / `verified_hubs`)
  - `contracts/controller/src/context/spoke.rs:75-101` (`require_listed_active_config`, `spoke_config`, `active_spoke`)
  - `contracts/controller/src/account.rs:15-77` (`SpokeAdmission`, `create_account_with`), `:154-158` (`require_spoke_match`)
  - `contracts/controller/src/positions/mod.rs:254-327` (`require_listed_unhalted_config`, `require_can_supply` / `require_can_borrow`, `validate_position_entry_gates`)
  - `contracts/controller/src/positions/{supply,debt}.rs`; `positions/liquidation/{mod,plan,apply}.rs`
  - `contracts/controller/src/strategies/{flash_loan,flash_position,multiply,migrate_blend,swap_debt,swap_collateral,repay_debt_with_collateral}.rs`
  - Admin lifecycle: `config/spoke.rs` (`add_spoke` / `remove_spoke` / `create_hub`); `storage/{hub,spoke}.rs`
- Defense: Unknown spoke/hub ids fail closed on risk-increasing paths. Spoke deprecation (`is_deprecated`) and hub activity (`HubConfig.is_active`) gate **entry** (supply/borrow/strategy open/refinance destination) while **exits** (withdraw/repay) and **liquidation** intentionally stay open on deprecated spokes and do not require hub activity. Account spoke binding is immutable (INV-AUTH-06) and re-matched on every spoke-parameterized mutator.
- Gap: (1) No public `deactivate_hub` / `set_hub_active(false)` entrypoint — `HubNotActive` today is primarily an **existence** check for unseeded ids; `is_active=false` is only reachable via direct storage write (tests) or a future upgrade. (2) Strategy “exit-like” verbs (`swap_debt` existing hub, `swap_collateral` source hub, `repay_debt_with_collateral` both hubs) require hub active, so a deactivated hub would block those convenience paths while bare `withdraw`/`repay` still work (liveness residual, accepted — same class as A047). (3) `docs/reference/errors.md` #43 over-claims “all … position and strategy entry points” — withdraw/repay/liquidate do not call `require_hub_active`. (4) Keeper paths (`update_indexes` / `claim_revenue` / `recapitalize`) and `add_asset_to_spoke` do not re-check hub active (pool / listing policy, not user-entry bypass).
- Impact: Callers cannot open new exposure against unknown/deprecated spokes or unknown hubs. Existing positions on a deprecated spoke remain unwindable and liquidatable. No fund-theft vector from missing hub/spoke active checks on user mutators; residual is operational liveness / docs accuracy.
- Evidence: INV-AUTH-06, INV-LIQ-01 (deprecated-spoke liquidation liveness); unit `governance/config.rs` hub gates; `positions/flags.rs::require_can_supply_blocks_inactive_hub`; harness `controller/spoke.rs` deprecated exit matrix; `deprecated_spoke_liquidation_liveness.rs`; integration `flash_position.sh` inactive-hub / deprecated-spoke xfails; peers A004, A013, A040, A047–A050, A099.
- Opinion: The entry/exit asymmetry is deliberate and correctly implemented. Treat hub `is_active` as a latent kill switch until a public deactivator ships; until then market halt is via spoke-asset flags (`paused`/`frozen`/`no_seize`) and spoke deprecation.

## Scope

Audit of **spoke id existence / deprecation** and **hub id existence / activity** checks on **user mutators** (ControllerInterface monetary verbs). Admin create/deprecate paths are in scope only as the writers of the flags users consume. Asset listing / freeze flags are adjacent (A040 / A064) and cited only where they share the same entry gate stack.

Out of scope for depth: pause macros (A001), auth (A002–A005), amount validation (A061), listing flags beyond the shared `require_listed_*` stack (A064), cache memo correctness beyond hub short-circuit (A090/A099).

## Verdict

**Defended.** Risk-increasing user paths require a live spoke and an active (existing) hub before opening or increasing exposure. Risk-reducing and liquidation paths intentionally skip those gates so users and keepers are not stranded after governance soft-closes a spoke or (in the latent design) a hub.

---

## 1. Primitive inventory

### 1.1 Hub activity

```93:96:contracts/controller/src/config/spoke.rs
pub(crate) fn require_hub_active(env: &Env, hub_id: u32) {
    let active = storage::get_hub(env, hub_id).is_some_and(|hub| hub.is_active);
    assert_with_error!(env, active, GenericError::HubNotActive);
}
```

- Missing key **or** `is_active == false` → `#43 HubNotActive`.
- Hub ids are minted by `create_hub` → `increment_hub_id` (instance counter, first id = `1`; id `0` never seeded).
- `create_hub` always writes `HubConfig { is_active: true }`.
- **No** `ControllerAdmin` entrypoint clears `is_active`. The only production writers of `Hub(hub_id)` found under `contracts/controller/src` are `create_hub` and the storage helper used by tests. Deactivation is therefore a **latent** control (upgrade / future API), not an operator path today.

`Cache::require_hub_active` memoizes **success only** into `verified_hubs` (A099): a failed check never records false; a later call for the same id re-runs storage. Same-tx deactivation by another contract is not reachable without reentrancy into this contract’s storage writers (flash guard / no public deactivator).

### 1.2 Spoke existence and deprecation

| Helper | Behavior on unknown id | Behavior on `is_deprecated` |
|---|---|---|
| `storage::get_spoke` | panic `#300 SpokeNotFound` | returns config |
| `Cache::spoke_config` | via `get_spoke` | returns config (caches once) |
| `Cache::active_spoke` | via `spoke_config` | panic `#301 SpokeDeprecated` |
| `Cache::require_listed_active_config` | via `active_spoke` then listing | deprecated blocked before listing |
| `create_account` / `ActiveOnly` | `spoke_id >= 1` + `active_spoke` | blocked |
| `AllowDeprecated` (liq `Credit(0)` only) | `spoke_id >= 1` + `spoke_config` | **allowed** if spoke row exists |

Deprecation is one-way soft-delete: `remove_spoke` flips `is_deprecated`, never removes the `Spoke(id)` row (A028). Live accounts keep their immutable `AccountMeta.spoke_id` (INV-AUTH-06).

### 1.3 Shared entry stack (supply / borrow)

```260:270:contracts/controller/src/positions/mod.rs
fn require_listed_unhalted_config(...) -> AssetConfig {
    cache.require_hub_active(hub_asset.hub_id);
    let asset_config = cache.require_listed_active_config(spoke_id, hub_asset);
    enforce_spoke_asset_flags(..., FreezePolicy::BlockOnEntry);
    asset_config
}
```

Order: **hub active → spoke not deprecated → asset listed on that spoke → not paused/frozen → verb flag** (`is_collateralizable` / `is_borrowable`). This is the single stack behind `require_can_supply` / `require_can_borrow` / `validate_position_entry_gates`.

### 1.4 Exit / seizure flag helper (no hub/spoke-active)

`enforce_spoke_asset_flags` is a **no-op** when the listing row is missing (delisted assets remain exitable/seizable). It never calls `require_hub_active` or `active_spoke`. Used by withdraw, repay, liquidation repay/seize legs.

---

## 2. Per-mutator matrix (user surface)

Legend: **H** = `require_hub_active` (direct or via `require_can_*`); **S+** = spoke must exist and not be deprecated (`active_spoke` / create `ActiveOnly`); **S=** = caller `spoke_id` must match account (INV-AUTH-06); **S?** = spoke existence only (`spoke_config` / `AllowDeprecated`); **—** = not checked (by design).

| Mutator | Spoke id source | Spoke check | Hub check | Notes |
|---|---|---|---|---|
| `supply` (create `account_id=0`) | caller arg | **S+** at create | **H** via entry gates | Unknown/deprecated spoke cannot mint |
| `supply` (existing) | caller arg | **S=** then **S+** on entry gates | **H** via entry gates | Top-up of existing slots still goes through `require_can_supply` → deprecated blocked |
| `borrow` | account meta | **S+** via entry gates | **H** | No caller spoke arg |
| `withdraw` | account meta | — (flags only) | — | Must already hold supply slot; position-not-found for fake hub |
| `repay` | account meta | — (flags only) | — | Must already hold debt slot |
| `liquidate` | victim meta | curve via `spoke_config` (**S?**); Credit create **AllowDeprecated** | — | Intentionally live on deprecated spoke (INV-LIQ-01) |
| `clean_bad_debt` / force socialize | account meta | — | — | Cleanup after liq gates |
| `flash_loan` | n/a (no account) | — | **H** early | Pool flash; no spoke |
| `flash_position` | caller arg | **S+** on create / **S=**+listing | **H** debt early; collaterals via `require_can_supply` | Refund assets also `require_listed_active_config` |
| `multiply` | caller arg | **S+** / **S=** | collateral **H** early via `require_can_supply`; debt **H** via `borrow_into_controller` → entry gates | |
| `swap_debt` | account meta | new debt via entry gates (**S+**) | **H** on **existing** early; **H** on **new** via borrow gates | Existing-hub gate is liveness residual if hub ever deactivated (A047) |
| `swap_collateral` | account meta | dest via `require_can_supply` (**S+**) | **H** on **source** early; **H** on **dest** via `require_can_supply` | Source-hub gate same liveness class (A048) |
| `repay_debt_with_collateral` | account meta | — on unwind legs | **H** on **both** collateral and debt hubs early | Bare withdraw+repay still available if blocked |
| `migrate_from_blend` | caller arg | **S+** / **S=**; withdraw assets `require_can_supply` | **H** on target `hub_id` early; debt legs via `borrow_into_controller` | |
| `add_delegate` / `remove_delegate` / `renew_account` | account meta | — | — | No market id inputs |
| `update_indexes` / `claim_revenue` / `recapitalize` | n/a | — | — | Pool fails closed on unknown market; not entry |

---

## 3. Path detail

### 3.1 Account creation and spoke binding

`create_account_with`:

1. `spoke_id >= 1` else `SpokeNotFound`.
2. `ActiveOnly` → `cache.active_spoke` (existence + not deprecated); `AllowDeprecated` → `spoke_config` (existence only).
3. Mint NFT; persist `AccountMeta { spoke_id, mode }`.

User creates (`supply` / `multiply` / `flash_position` / `migrate_from_blend` with `account_id=0`) always use `ActiveOnly` (A004). Sole `AllowDeprecated` site: liquidation `Credit(0)` receiver in the **victim’s** spoke (A013) — required because deprecation is irreversible and Transfer seize may need pool cash.

Existing-account guards (`load_or_create_account`): `require_spoke_match` only. Existence/deprecation are **not** re-checked at load; they are re-enforced on **entry** gates when the path increases exposure. Exit paths deliberately skip `active_spoke`.

### 3.2 Supply / borrow (risk-increasing)

- Supply: aggregate → load/create → third-party slot rule → `validate_position_entry_gates(Deposit)` → `require_can_supply` per hub.
- Borrow: owner/delegate → `validate_position_entry_gates(Borrow)` → `require_can_borrow` per hub → post-pool solvency.

Unknown hub id → `HubNotActive` before listing. Deprecated spoke → `SpokeDeprecated` inside `require_listed_active_config`. Covered by unit `require_can_supply_blocks_inactive_hub` and harness `test_supply_panics_on_deprecated_spoke_category` / `test_deprecated_spoke_repay_allowed_but_new_borrow_blocked`.

### 3.3 Withdraw / repay (risk-reducing)

Only `enforce_spoke_asset_flags(..., AllowOnExit)` (paused blocks; frozen does not). No hub active, no spoke active. Position must already exist (`CollateralPositionNotFound` / `DebtPositionNotFound`). Harness confirms deprecated spoke still allows withdraw/repay while blocking new borrow.

This is the correct counterpart to spoke soft-close: governance can stop new exposure without trapping open books.

### 3.4 Liquidation

- Plan: `AllowOnExit` on repay assets; `SeizureLeg` (`no_seize`) on seized collateral; liquidation curve from `spoke_config(account.spoke_id)` — **does not** require non-deprecated.
- Apply: same flag policy; Credit new slot uses `require_spoke_asset` (listing), not hub active.
- Receiver: same spoke as victim; `Credit(0)` `AllowDeprecated`.

No `require_hub_active` on liquidate. Inactive/unknown hubs cannot appear on victim books without having passed entry earlier; delisted hubs remain seizable/repayable.

### 3.5 Strategies

| Strategy | Early hub gate | Spoke / dest gate |
|---|---|---|
| `flash_loan` | debt hub | none |
| `flash_position` | debt hub | create/match + `require_can_supply` collaterals + listed refunds |
| `multiply` | via `require_can_supply(collateral)` + borrow gates on debt | create/match |
| `migrate_from_blend` | target `hub_id` | create/match + `require_can_supply` each withdrawn asset; debt via borrow gates |
| `swap_debt` | **existing** debt hub | new debt via `borrow_into_controller` entry gates |
| `swap_collateral` | **current** (source) hub | `require_can_supply(new)` |
| `repay_debt_with_collateral` | **both** hubs | unwind via exit flags / positions |

Asymmetry vs bare repay/withdraw: strategy paths that still open new debt or that insist the exit hub is “live” will fail with `#43` if that hub were deactivated, while the user can still close risk with `repay` / `withdraw`. Documented as accepted liveness residual (A047 §7, A048, A049).

### 3.6 Keepers / admin adjacency (not user entry, but same ids)

- `create_liquidity_pool`: `require_hub_active` — cannot create a pool market under a missing hub.
- `add_asset_to_spoke`: checks spoke not deprecated on Add; **does not** call `require_hub_active`. Governance can list a hub-asset key whose hub row is missing; user entry still fails at `require_can_supply`/`borrow`. Footgun for admins, not a user bypass.
- Keepers: no hub/spoke-active checks; pool load fails for unknown markets (A015).

---

## 4. Attack / misuse scenarios

| Scenario | Outcome | Severity |
|---|---|---|
| `supply` / `multiply` / `flash_position` / `migrate` with `spoke_id=0` | `SpokeNotFound` | defended |
| Same with unknown spoke id | `SpokeNotFound` via `get_spoke` | defended |
| Same with deprecated spoke (new account) | `SpokeDeprecated` | defended |
| Top-up / borrow on existing account after `remove_spoke` | `SpokeDeprecated` on entry gates | defended |
| Withdraw / repay after `remove_spoke` | allowed (flags permitting) | defended (policy) |
| Liquidate after `remove_spoke` | allowed; `Credit(0)` may create receiver there | defended (INV-LIQ-01) |
| Any entry with unseeded `hub_id` (incl. 0) | `HubNotActive` | defended |
| Entry with hub row `is_active=false` (test / future) | `HubNotActive` | defended (latent) |
| Withdraw/repay/liquidate with inactive hub on existing position | allowed | defended (exit policy) |
| `swap_debt` / `swap_collateral` / `repay_with_collateral` when exit hub inactive | blocked early; bare repay/withdraw still work | low liveness |
| Caller passes wrong `spoke_id` for existing account | `SpokeMismatch` | defended |
| Cross-spoke Credit receiver | `SpokeMismatch` | defended (A013) |
| Forge hub id on withdraw without position | `CollateralPositionNotFound` | defended |
| `flash_loan` unknown hub | `HubNotActive` | defended |
| `verified_hubs` memo skip after success | cannot skip a failed check (A099) | defended |

---

## 5. Cross-links and doc drift

Agree with:

- **A004 / A013** — create `ActiveOnly` vs liq `AllowDeprecated`.
- **A040** — listing sits behind hub+spoke-active on entry.
- **A047–A050** — strategy hub-active placement and exit asymmetry.
- **A099** — `verified_hubs` success-only memo.

Doc drift (non-blocking):

- `docs/reference/errors.md` #43 says HubNotActive applies to “All controller position and strategy entry points”. Withdraw, repay, and liquidate are position mutators that **omit** the check by design. Prefer “risk-increasing position/strategy entry and `create_liquidity_pool`”.
- Same error text implies operational “deactivated” hubs; no public deactivator exists yet. Market soft-close today is spoke deprecation + asset flags.

---

## 6. Tests / rules touching this surface

| Coverage | What it shows |
|---|---|
| `contracts/controller/tests/governance/config.rs` | hub 0 / unknown / deactivated → `#43`; create marks active |
| `contracts/controller/tests/positions/flags.rs::require_can_supply_blocks_inactive_hub` | entry stack needs hub row |
| Harness `controller/spoke.rs` | deprecated: block borrow/supply, allow repay/withdraw/liq |
| Harness `deprecated_spoke_liquidation_liveness.rs` | `Credit(0)` on deprecated; other creates closed |
| Harness `admin.rs::test_supply_panics_on_deprecated_spoke_category` | top-up blocked after deprecate |
| Integration `flash_position.sh` | inactive hub `#43`; deprecated spoke new `#301` |

No Certora rule was found that exclusively asserts `require_hub_active` on every mutator; hub activity is covered indirectly by entry-gate / market-guard rules. Optional follow-up (not a gap in runtime defense): a CVL rule that risk-increasing specs assume `HubConfig.is_active` and exit specs do not.

---

## 7. Residual / remediation (none required for funds)

1. **Optional product:** add an owner/timelock `set_hub_active(hub_id, false)` (or rename docs) so the latent `is_active` bit matches operator expectations; until then halt markets with `set_spoke_asset_flags` / `remove_spoke` / listing removal.
2. **Docs:** narrow errors.md #43 caller list; note strategy-vs-bare-exit hub asymmetry next to endpoints.md strategy sections (partially present for `swap_debt`).
3. **Do not** add `require_hub_active` to withdraw/repay/liquidate — that would trap users if hubs ever deactivate and would contradict INV-LIQ-01 / halt design.

**Final judgment:** Spoke and hub existence/active checks on user mutators are correctly layered: fail closed on new exposure, fail open on unwind and liquidation. No undefended critical/high gap in this scope.
