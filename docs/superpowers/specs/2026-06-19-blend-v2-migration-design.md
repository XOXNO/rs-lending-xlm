# Blend V2 → XOXNO Lending: One-Click Position Migration — Design Spec

Date: 2026-06-19
Status: Approved (refined during planning)

> **Refinement (planning, 2026-06-19):** The Blend ABI surface is reduced to **`submit` only**.
> Instead of reading Blend `get_positions`/`get_reserve` to compute the exact debt (§5/§8 as
> originally written), the migration borrows the caller's `max_debt` cap, repays Blend with it,
> and reconciles Blend's over-repay refund back into the user's new debt via
> `repay_debt_from_controller` — netting the user's debt to exactly what cleared Blend. This
> eliminates the fragile `Reserve` mirror (risk #2) and shrinks WASM (risk #3) while keeping
> D2's cap semantics (a too-low cap reverts via Blend's post-withdraw health check). See the
> implementation plan `docs/superpowers/plans/2026-06-19-blend-v2-migration.md` for the
> authoritative flow.
Scope: A new migration strategy inside the controller that atomically moves a user's
full Blend V2 position (collateral, non-collateral supply, and debt) into our lending
protocol in a single transaction, at zero flash-loan fee.

---

## 1. Goal

Let a Blend V2 user migrate their position to our protocol in one click:

- **Debt-free users**: atomically withdraw all Blend collateral + supply and deposit it
  into our protocol, creating/crediting their position account here.
- **Users with debt**: same, but first source the repayment via our internal
  `create_strategy` borrow (the "flash loan" the user referred to — *not* the
  `flash_loan` receiver primitive). The controller borrows the debt asset on the user's
  new account (fee = 0), repays Blend, withdraws the freed collateral, deposits it here,
  and a single end-of-flow health check gates the result. Net effect: the user's debt and
  collateral move from Blend to us; Blend is left empty; the new debt equals the old debt.

Both flows are **one unified entrypoint and code path**. The debt-free flow is the
debt flow with an empty debt list (the borrow + repay legs are skipped).

## 2. Confirmed design decisions

| # | Decision | Choice |
|---|----------|--------|
| D1 | Asset scope | **Generic** — works for any markets, not hardcoded XLM/USDC. Reads the user's actual Blend position. |
| D2 | Debt-flow borrow bound | **Explicit `max_debt` cap per debt asset.** Controller borrows only what is needed to clear Blend, and reverts if that exceeds the caller's cap. |
| D3 | Migrate scope | **Collateral + non-collateral supply + debt.** Blend's plain `supply` map is also swept (see §10 for the consequence). |
| D4 | "Flash loan" mechanism | Internal `create_strategy` strategy-borrow with `fee = 0` (controller-chosen fee; no pool change). Confirmed by the user. |
| D5 | Contract location | **In-controller strategy**, no second contract. All required primitives already live in the controller. |

## 3. Why no second contract

Every building block is already a controller-internal primitive (recon citations):

- `supply::process_deposit(env, source, &mut account, assets, cache)` — credits collateral
  to the *user's account* while funds come from a *source* address (the controller).
  Used today by `strategies/multiply.rs:153`.
- `borrow_for_strategy` → `pool_create_strategy_call(receiver = controller, action, fee, borrow_cap)`
  (`positions/borrow.rs:148-185`) — opens debt on the user's account, sends funds to the
  controller, **defers** the health check. Fee is controller-supplied → we pass `0`.
- `strategy_finalize` → `validation::require_post_pool_risk_gates`
  (`strategies/mod.rs:53-64`, `validation.rs:58-93`) — the mandatory tail health/LTV/
  min-collateral gate.
- Re-entrancy guard `validation::require_not_flash_loaning` (`validation.rs:44-50`).
- `authorize_as_current_contract` cross-call auth pattern (`strategies/swap.rs:108-128`).

A second contract would have to re-expose these or call back into the controller, which the
Soroban host blocks as re-entry. In-controller is both simpler and safer.

## 4. Components / files

| File | Change |
|------|--------|
| `contracts/controller/src/external/blend.rs` | **New.** Blend client + minimal type mirrors (`Request`, `Positions`, `Reserve` fields we read) + thin `blend_*_call` wrappers, cfg-swapped for the certora harness like `pool`/`sac`. |
| `contracts/controller/src/external/mod.rs` | Register `mod blend;` (with the `#[cfg(feature="certora")]` harness-stub swap mirroring `pool`/`sac`). |
| `contracts/controller/src/strategies/migrate_blend.rs` | **New.** The `migrate_from_blend` entrypoint + `process_migrate_blend`. |
| `contracts/controller/src/strategies/mod.rs` | Register `mod migrate_blend;`. |
| `contracts/controller/src/strategies/positions.rs` (or `borrow.rs`) | Add a `fee = 0` strategy-borrow helper (`open_migration_borrow`) — thin variant of `open_strategy_borrow`/`borrow_for_strategy` that passes `fee = 0`. |
| `interfaces/controller/src/lib.rs` | Add `migrate_from_blend` to the controller trait (ABI). |
| `common/src/errors.rs` *(or controller errors)* | Add migration-specific error variants (see §11). Reuse existing variants where possible to limit WASM growth (each variant ≈ 57 B). |
| `interfaces/controller/src/types/controller.rs` | Migration-facing param/event types if needed (controller types live here, not in `common`). |
| tests/integration (flows) | A `flows/migrate_blend.sh` E2E lane against a Blend mock (see §12). |
| `contracts/controller/src/events.rs` | A `BlendMigration` event. |

## 5. The Blend interface (`external/blend.rs`)

Mirror only what we use. `#[contracttype]` structs serialize as field-name maps, so **field
names must match Blend exactly**; struct field order is irrelevant to encoding but we keep
Blend's order for readability.

```rust
// Mirrors blend pool/src/pool/actions.rs:13-17
#[contracttype]
pub struct Request {
    pub request_type: u32,   // RequestType discriminant (see below)
    pub address: Address,    // asset address
    pub amount: i128,
}

// RequestType discriminants (blend actions.rs:22-33) — we only emit these three:
pub const REQ_WITHDRAW: u32 = 1;             // sweep non-collateral supply
pub const REQ_WITHDRAW_COLLATERAL: u32 = 3;  // sweep collateral
pub const REQ_REPAY: u32 = 5;                // clear debt

// Mirrors blend pool/src/pool/user.rs:8-15
#[contracttype]
pub struct Positions {
    pub liabilities: Map<u32, i128>, // reserve index -> dToken shares
    pub collateral:  Map<u32, i128>, // reserve index -> bToken shares
    pub supply:      Map<u32, i128>, // reserve index -> bToken shares (non-collateral)
}
```

Client (inline `#[contractclient]`, aggregator-style — `swap.rs:14-22`), covering only:

- `fn submit(env, from: Address, spender: Address, to: Address, requests: Vec<Request>) -> Positions`
- `fn get_positions(env, address: Address) -> Positions`
- `fn get_reserve_list(env) -> Vec<Address>` (index → asset)
- `fn get_reserve(env, asset: Address) -> Reserve` (for live `d_rate` to compute exact debt underlying)

For `Reserve` we mirror only the fields we read (`data.d_rate`, `scalar`/decimals).
If mirroring `Reserve` proves heavy, an alternative is a Blend view that returns the
underlying debt directly; default is to mirror the minimal `Reserve`/`ReserveData` subset.

The Blend pool address is **a caller-supplied argument** (`blend_pool: Address`), validated
to be a deployed Wasm contract (`require_wasm_receiver`). We do **not** hardcode it (generic, D1).

## 6. Entrypoint & signature

```rust
#[when_not_paused]
pub fn migrate_from_blend(
    env: Env,
    caller: Address,
    account_id: u64,                 // 0 => create a new account here
    blend_pool: Address,
    collateral_assets: Vec<Address>, // Blend collateral to sweep (WithdrawCollateral, all)
    supply_assets: Vec<Address>,     // Blend non-collateral supply to sweep (Withdraw, all)
    debt_caps: Vec<(Address, i128)>, // per debt asset: max we may borrow to clear it; empty => debt-free flow
) -> u64                              // the account_id (new or existing)
```

Caller passes the asset lists (built by the UI from a Blend `get_positions` read). The
contract reads exact debt amounts from Blend; collateral/supply use "withdraw all" + measure.

## 7. Data flow (single path; debt legs skipped when `debt_caps` empty)

1. `caller.require_auth()`; `validation::require_not_flash_loaning(env)`.
2. Validate inputs: `blend_pool` is a Wasm contract; lists non-empty in aggregate;
   no asset appears in more than one role.
3. `Cache::new(env, OraclePolicy::RiskIncreasing)` (debt may be opened).
4. Load or create the account (`account_id == 0` → `helpers::create_account(caller, mode = Normal)`;
   else load + `require_account_owner_match`). Mode is **Normal** (a plain migrated position,
   not a leverage mode).
5. Validate every involved asset (`collateral_assets ∪ supply_assets ∪ debt_caps.keys`) is an
   **active, supported market** here, and that supply targets are `can_supply()` (collateralizable).
   Revert otherwise (we cannot value or hold an unsupported asset, and leaving Blend debt while
   pulling Blend collateral would trip Blend's own health check).
6. `prefetch_strategy_oracles(cache, account, all_involved_assets)`.
7. **Debt legs (per `(debt_asset, max)` in `debt_caps`):**
   a. Read Blend debt: `shares = blend.get_positions(caller).liabilities[reserve_index(debt_asset)]`;
      `d_rate = blend.get_reserve(debt_asset).data.d_rate`.
   b. `repay_amt = to_asset_from_d_token(shares, d_rate)` — Blend's own **ceil** rounding
      (`reserve.rs:182`/`174`). Using exactly this value closes the debt with **zero refund**
      (proof in §8).
   c. `require(repay_amt <= max)` else revert (`MigrationDebtCapExceeded`).
   d. `open_migration_borrow(cache, account, debt_asset, repay_amt)` →
      `pool_create_strategy_call(receiver = controller, action = (debt_asset, repay_amt), fee = 0, borrow_cap)`.
      Controller now holds `repay_amt` of `debt_asset`; user debt position = `repay_amt`.
8. Build the Blend request batch (one `submit`):
   `[Repay(d, repay_amt_d) ∀ debt]  ++  [WithdrawCollateral(c, i128::MAX) ∀ collateral]  ++  [Withdraw(s, i128::MAX) ∀ supply]`.
   Repays first ⇒ after them the user has zero Blend liabilities ⇒ Blend's final health check
   is skipped (`submit.rs:188` `has_liabilities()` gate), and the withdraws empty the position.
9. **Authorize the controller's own legs** via `authorize_as_current_contract`, emitted
   *immediately* before the submit (no intervening cross-call — §9): the `submit` context
   (controller as `spender`) with nested `transfer(controller → blend_pool, repay_amt_d)`
   sub-invocations for each debt asset.
10. `balance_before[a]` snapshot for each withdraw asset `a`.
11. `blend.submit(from = caller, spender = controller, to = controller, requests)`.
    Blend repays the user's debt from the controller's funds and sends all withdrawn
    collateral + supply to the controller. The user's `from` auth comes from the tx auth tree
    (§9), not from us.
12. For each withdraw asset `a`: `received[a] = balance_delta(a)` (`swap.rs:64`).
    `process_deposit(env, source = controller, &mut account, [(a, received[a])], cache)` —
    credits the user's account, funds from the controller.
13. `strategy_finalize(env, account_id, &mut account, cache)` — the tail health/LTV/
    min-collateral gate (`require_post_pool_risk_gates`), then persist both sides + emit events.
14. Emit `BlendMigration { caller, account_id, blend_pool, n_collateral, n_supply, n_debt }`.
15. Return `account_id`.

## 8. Exact debt repayment → zero dust (no leftover)

Blend repay (`actions.rs:422-440`) clamps: if `to_d_token_down(amount) > cur_d_tokens` it fully
closes the debt and refunds `amount - to_asset_from_d_token(cur_d_tokens)` to `to`.

Let `repay_amt = to_asset_from_d_token(cur_d_tokens)` (ceil). Then:
- `to_d_token_down(repay_amt) >= cur_d_tokens` ⇒ Blend fully closes the debt.
- refund `= repay_amt - to_asset_from_d_token(cur_d_tokens) = 0`.

So borrowing and repaying exactly `repay_amt` (computed with Blend's own ceil conversion from a
**same-ledger** `get_reserve` read — interest accrual is identical within one ledger) closes the
debt with **no refund and no leftover**, and the user's new debt equals exactly the amount that
cleared Blend. No dust-handling code is required.

(Withdraws use `i128::MAX`; Blend clamps to the real balance — `actions.rs:319-331,370-377` — so
"withdraw all" needs no pre-read and over-withdraw is impossible.)

## 9. Authorization model (the crux)

For the debt flow the controller calls
`blend.submit(from = caller, spender = controller, to = controller, requests)`.
Blend's `submit` (`contract.rs:452-466`) calls `spender.require_auth()` always and
`from.require_auth()` because `from != spender`.

- **User authorizes** (their wallet produces these during tx simulation — standard Soroban
  multi-party auth): the top-level `migrate_from_blend(caller, …)` entrypoint **and** the nested
  `blend.submit(from = caller, …)` (Blend's `from = user` requirement). We do **not** forward a
  user AuthEntry and do **not** call `require_auth_for_args`; the host attributes the unmet
  `user.require_auth()` to the user during simulation and the wallet signs it.
- **Controller authorizes** (via `authorize_as_current_contract`, as itself, emitted right
  before the submit): the `submit` context (controller as `spender`) + nested
  `debt_asset.transfer(controller → blend_pool, repay_amt)` for each debt asset (the repay pull
  from `spender = controller`).

The two authorizations are keyed by different addresses (user vs controller) and coexist on the
same `submit` node — exactly Blend's `from != spender` case. Collateral/supply withdrawals pay
`to = controller` and are authorized by the Blend pool itself (no user/controller auth needed for
those transfers).

**Ordering constraint (the `authorize_as_current_contract` gotcha,
`defindex-strategy-and-auth-fix` memory):** invoker-contract auth covers only the *next*
sub-invocation tree. Therefore the zero-fee borrow (a cross-call to our pool) must happen
*before* the `authorize_as_current_contract`, and **nothing** (no oracle read, no `get_reserve`,
no extra cross-call) may run between `authorize_as_current_contract` and `blend.submit`. All Blend
reads (`get_positions`, `get_reserve`, `get_reserve_list`) happen in step 7 (before the borrow);
the authorize+submit in steps 9–11 are back-to-back.

The debt-free flow only issues `WithdrawCollateral`/`Withdraw` (paid to the controller). Blend
still calls `spender.require_auth()` (controller, via `authorize_as_current_contract`) and
`from.require_auth()` (user, signed). No nested token transfer needs controller auth (withdraws
are pool→controller).

## 10. Non-collateral supply mapping (consequence of D3)

Our protocol has **no non-collateral supply concept**: `can_supply()` == `is_collateralizable`
(`interfaces/controller/src/types/controller.rs:44`). So Blend's plain `supply` balances, when
migrated, are deposited **as collateral** here. Effects:

- The user's health *improves* (more collateral), but this deviates from their Blend intent of
  supplying-without-collateralizing.
- A Blend `supply` asset that is **not** `is_collateralizable` here cannot be migrated as supply —
  `process_deposit` would revert `NotCollateral`. The caller must omit such assets from
  `supply_assets` (or the UI surfaces this). Documented behavior, not a silent change.

## 11. Error handling

Reuse existing variants where possible (`CollateralError::{NotCollateral, InsufficientCollateral}`,
`GenericError::{AccountNotInMarket, …}`, `FlashLoanError::FlashLoanOngoing`,
`validation::require_positive_amount`). Add only what is genuinely new (each `contracterror`
variant ≈ 57 B WASM):

- `MigrationDebtCapExceeded` — exact Blend debt > caller's `max` for a debt asset.
- `MigrationAssetUnsupported` — an involved asset is not an active/listed market here
  (may map onto an existing "market not active"/`AssetNotSupported` variant if one exists — reuse first).
- `MigrationNothingToMigrate` — all asset lists empty.

All failures revert the whole atomic tx (no partial migration). No silent fallbacks.

## 12. Testing strategy

- **Unit (controller, `#[cfg(test)]`)**: with a Blend **mock** pool registered as a Wasm fixture
  (mirroring the `defindex` mock-market lane). Cases:
  - Debt-free: multi-collateral + multi-supply migration → positions credited, health gate trivially
    passes, Blend position emptied.
  - With debt: borrow(fee=0) → repay → withdraw → supply → healthy end state; assert user new debt ==
    Blend debt (exact, zero refund); assert account totals.
  - `max_debt` exceeded → revert `MigrationDebtCapExceeded`.
  - Unsupported / non-collateralizable asset → revert.
  - End-state unhealthy (debt too large vs migrated collateral) → revert at `strategy_finalize`.
  - Auth: `mock_auths` proving the exact required tree — `caller` signs `migrate_from_blend` +
    nested `submit(from=caller,…)`; controller's `authorize_as_current_contract` covers
    `submit`(spender) + repay `transfer`. (Per `defindex` memory, `mock_all_auths` can hide an
    auth-ordering bug; use targeted `mock_auths` for the auth-shape test.)
  - Re-entrancy guard set/respected.
- **Integration E2E (`tests/integration/flows/migrate_blend.sh`)**: deploy a Blend mock + our
  controller/pool, seed a Blend position, run both flows, assert balances/positions/events. Gate
  helpers as in existing lanes.
- **Differential/snapshot**: commit `test_snapshots/` for the new flows.

## 13. Open risks / must-verify

1. **Live auth tree (highest risk).** The user-signed nested `submit(from=user)` + controller
   `authorize_as_current_contract` split is sound by construction and matches Blend's
   `from != spender` tests, but **must be verified on testnet** with a real wallet (simulation must
   produce the user's nested-submit AuthEntry; the controller's invoker-auth must satisfy
   spender + repay transfer). Unit `mock_auths` proves the shape; testnet proves the wallet UX.
2. **`Reserve` mirror fidelity.** `get_reserve` returns a non-trivial `Reserve` struct; we mirror
   only `data.d_rate` (+ scalar). If field-name/shape drift breaks decoding, fall back to a Blend
   view that returns underlying debt, or mirror the full `ReserveData`. Verify against Blend's
   actual ABI at build time.
3. **WASM size.** Controller is near its cap (memory: deploys ~127–134 KB vs 140 KB). The Blend
   mirror + new strategy add code; measure with the size check and apply the strip levers if needed.
4. **Same-ledger rate assumption** for zero-dust repay holds within one tx/ledger (Blend accrues at
   submit time using the same timestamp). Re-confirm no second ledger boundary is crossed.
5. **Multiple debt assets** are supported (per-asset cap + per-asset borrow/repay), but the common
   case is a single debt (USDC). Keep the loop general; test the multi-debt case.

## 14. Non-goals (v1)

- Migrating Blend liquidation auctions, bad-debt, or emissions/BLND claims (the user may have
  unclaimed BLND on Blend; v1 does not sweep it — documented, not silently dropped).
- Partial-position migration that *leaves* Blend debt while pulling collateral (would trip Blend's
  health check); v1 migrates the listed assets as a coherent whole and clears all listed debt.
- A standalone migrator contract (D5).
