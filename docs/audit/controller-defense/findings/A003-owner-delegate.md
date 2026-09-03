# A003 — INV-AUTH-02 owner-or-delegate path map

- Agent: A003
- Theme: T1 (defensive protections inventory — auth)
- Severity: info
- Status: defended
- Paths: see table below
- Defense: Triple gate in `is_owner_or_delegate` — (1) caller is live NFT owner, or (2a) globally active position manager **and** (2b) listed on the account’s `DelegateGrant` stamped by that live owner. NFT transfer lazily kills prior grants via `granted_by` filter.
- Gap: none on the eight scoped mutators. Formal Certora coverage for this invariant is supply-slot only (`supply_new_slot_requires_owner_or_delegate`), not borrow/withdraw/strategies.
- Impact: a stranger cannot borrow, withdraw collateral, open/extend leverage, refinance debt/collateral, force-close via repay-with-collateral, or migrate Blend debt onto another account. Blast radius if the gate failed: full collateral drain + uncapped debt mint on any account id the attacker knows.
- Evidence: INV-AUTH-02; Spoof.3 in `STRIDE.md`; `scripts/permissionless_entrypoints.txt` lines for the eight entrypoints; unit + harness tests listed per path
- Opinion: All eight risk-mutating surfaces in scope are gated before any pool/strategy money move. No path mutates foreign account risk without `require_owner_or_delegate` / `AccountGuard::{Migrate,Multiply}`. Residual notes are verification-coverage and design observations, not bypasses.

## Gate primitive

```118:143:contracts/controller/src/account.rs
pub(crate) fn is_owner_or_delegate(
    env: &Env,
    account_id: u64,
    caller: &Address,
    owner: &Address,
) -> bool {
    if caller == owner {
        return true;
    }
    let active_manager =
        storage::get_position_manager(env, caller).is_some_and(|config| config.is_active);
    active_manager && storage::get_delegates(env, account_id, owner).contains(caller)
}

pub(crate) fn require_owner_or_delegate(
    env: &Env,
    account_id: u64,
    caller: &Address,
    owner: &Address,
) {
    if is_owner_or_delegate(env, account_id, caller, owner) {
        return;
    }
    panic_with_error!(env, GenericError::NotAuthorized);
}
```

Owner identity is **live NFT `owner_of`**, not a cached controller field:

```148:157:contracts/controller/src/storage/account.rs
pub(crate) fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    let meta = try_get_account_meta(env, account_id)?;
    let owner = try_account_owner(env, account_id)?;
    Some(account_from_parts(
        owner,
        meta,
        get_supply_positions(env, account_id),
        get_debt_positions(env, account_id),
    ))
}
```

Delegate list is stamped and filtered by granting owner (NFT transfer = lazy revoke):

```170:177:contracts/controller/src/storage/account.rs
/// Reads the delegate list for `account_id`, treating any grant stamped by a previous owner
/// as empty. NFT transfer therefore revokes delegates lazily.
pub(crate) fn get_delegates(env: &Env, account_id: u64, owner: &Address) -> Vec<Address> {
    get_user::<DelegateGrant>(env, &ControllerKey::Delegates(account_id))
        .filter(|grant| grant.granted_by == *owner)
        .map(|grant| grant.delegates)
        .unwrap_or_else(|| Vec::new(env))
}
```

`load_or_create_account` applies the gate for existing ids under `Migrate` / `Multiply`; `account_id == 0` creates a new account owned by `caller` (no foreign-risk path):

```97:114:contracts/controller/src/account.rs
    if account_id == 0 {
        return create_account(env, caller, spoke_id, mode, cache);
    }
    let account = storage::get_account(env, account_id);
    match guard {
        AccountGuard::Supply => require_spoke_match(env, &account, spoke_id),
        AccountGuard::Migrate => {
            require_owner_or_delegate(env, account_id, caller, &account.owner);
            require_spoke_match(env, &account, spoke_id);
        }
        AccountGuard::Multiply => {
            require_owner_or_delegate(env, account_id, caller, &account.owner);
            require_spoke_match(env, &account, spoke_id);
            assert_with_error!(env, account.mode == mode, GenericError::AccountModeMismatch);
        }
    }
```

Every scoped process also calls `require_authorized_caller` (`caller.require_auth()` + flash-loan reentrancy ban) **before** the owner/delegate check.

Delegates cannot self-grant: `add_delegate` / `remove_delegate` use `require_account_owner` (NFT owner only), not owner-or-delegate. Out of primary scope (A005) but closes the INV-AUTH-02 sentence “Delegates cannot grant or renew their own authority.”

---

## Path matrix (scoped mutators)

| Entrypoint (`lib.rs`) | Process | Gate site | When relative to mutation | Risk effect | Verdict |
|---|---|---|---|---|---|
| `borrow` | `positions::process_borrow` | `require_owner_or_delegate` after `get_account` | Before `settle_debt` / pool borrow | Increases debt; optional `to` recipient | **gated** |
| `withdraw` | `positions::process_withdraw` | `require_owner_or_delegate` after `get_account` | Before `settle_withdraw` / pool withdraw | Removes collateral | **gated** |
| `multiply` | `strategies::multiply::process_multiply` | `AccountGuard::Multiply` via `prepare_multiply_account` → `load_or_create_account` | Before `borrow_into_controller` / deposit | Opens/extends leverage debt + supply | **gated** |
| `flash_position` | `strategies::flash_position::process_flash_position` | `AccountGuard::Multiply` via `load_or_create_account` | Before mint/forward/callback/deposit | Opens/extends leverage via callback | **gated** |
| `swap_debt` | `strategies::swap_debt::process_swap_debt` | `require_owner_or_delegate` after `get_account` | Before `borrow_into_controller` | Refinances debt (new borrow + repay) | **gated** |
| `swap_collateral` | `strategies::swap_collateral::process_swap_collateral` | `require_owner_or_delegate` after `get_account` | Before `withdraw_and_swap_from_supply` | Replaces collateral asset | **gated** |
| `repay_debt_with_collateral` | `strategies::repay_debt_with_collateral::process_repay_debt_with_collateral` | `require_owner_or_delegate` after `get_account` | Before net-settle / withdraw+swap / optional close | Risk-reducing for account, but force-close of foreign positions blocked | **gated** |
| `migrate_from_blend` | `strategies::migrate_blend::process_migrate_blend` | `AccountGuard::Migrate` via `load_or_create_account` | Before `borrow_into_controller` / Blend sweep | Mints hub debt + deposits Blend collateral onto account | **gated** |

### Line citations (gate → first money move)

1. **borrow** — `contracts/controller/src/positions/debt.rs:42–45` (`require_authorized_caller` then `require_owner_or_delegate`); pool mutation in `settle_debt` afterward. Entrypoint: `lib.rs:112–120`.

2. **withdraw** — `contracts/controller/src/positions/supply.rs:168–171`; pool mutation in `settle_withdraw` at `:178`. Entrypoint: `lib.rs:127–134`. Note: no `#[when_not_paused]` on withdraw (pause surface is A006); auth gate is independent and present.

3. **multiply** — `prepare_multiply_account` → `AccountGuard::Multiply` at `strategies/multiply.rs:133–140`; first risk mutation `borrow_into_controller` at `:75–83`. Entrypoint: `lib.rs:230–258`.

4. **flash_position** — `AccountGuard::Multiply` at `strategies/flash_position.rs:96–104`; first risk mutation inside `with_flash_guard` / `mint_and_forward` → `borrow_into_controller` (~`:285`). Entrypoint: `lib.rs:194–222`.

5. **swap_debt** — `strategies/swap_debt.rs:41–52`; `borrow_into_controller` at `:60–68`. Entrypoint: `lib.rs:264–284`.

6. **swap_collateral** — `strategies/swap_collateral.rs:43–50`; `withdraw_and_swap_from_supply` at `:58–68`. Entrypoint: `lib.rs:290–310`.

7. **repay_debt_with_collateral** — `strategies/repay_debt_with_collateral.rs:48–55`; first position mutation at net-settle (`:65`) or swap repay (`:74`). Entrypoint: `lib.rs:317–339`.

8. **migrate_from_blend** — `AccountGuard::Migrate` at `strategies/migrate_blend.rs:68–76`; first risk mutation `execute_migration_debt_leg` → `borrow_into_controller` at `:165`. Entrypoint: `lib.rs:347–371`.

---

## Internal helpers (no independent gate — callers must have gated)

These `pub(crate)` helpers mutate positions / move funds and do **not** re-check owner-or-delegate. Audit confirms every production caller is behind one of the gates above (or is liquidation/supply under a different INV):

| Helper | File | Callers in scope |
|---|---|---|
| `borrow_into_controller` | `positions/debt.rs` | multiply, flash_position, swap_debt, migrate_blend |
| `withdraw_and_swap_from_supply` / `withdraw_collateral_to_controller` / `execute_withdraw_all` / `net_settle_collateral_against_debt` / `repay_debt_from_controller` | `strategies/{mod,legs}.rs` | swap_collateral, repay_debt_with_collateral, swap_debt, migrate |
| `supply::process_deposit` | `positions/supply.rs` | multiply / flash / swap_collateral / migrate (controller as token payer after gate) |

No alternate entrypoint reaches these helpers without an upstream owner-or-delegate (or create-self) check.

---

## Related callers of the same gate (out of A003 primary scope, recorded for completeness)

| Path | Gate use | INV |
|---|---|---|
| Third-party `process_supply` new slot | `is_owner_or_delegate` soft check — strangers may only top up existing hubs | INV-AUTH-03 (A012) |
| Liquidation `resolve_seize_receiver` Credit mode | `require_owner_or_delegate` on **receiver** account | INV-AUTH-02 / INV-LIQ-02 (A013) |
| `add_delegate` / `remove_delegate` / `renew_account` | `require_account_owner` (stricter than delegate) | INV-AUTH-02 grant sentence / INV-STOR-02 |

`process_repay` is intentionally **ungated** beyond `require_authorized_caller` — permissionless risk reduction (INV-AUTH-03), not INV-AUTH-02.

---

## Paths that mutate risk without this gate?

**None among the eight scoped entrypoints.** Exhaustive check:

- Existing `account_id`: every path calls `require_owner_or_delegate` or `AccountGuard::{Migrate,Multiply}` (which embeds it) before debt mint, collateral withdraw, or Blend migration debt.
- `account_id == 0` on multiply / flash_position / migrate: `create_account(env, caller, …)` — caller becomes owner; no foreign account is touched. borrow/withdraw/swap_* reject missing accounts via `get_account` (`AccountNotFound`) before any mutation.
- Stranger targeting another account: panics `#44 NotAuthorized`.

Design observation (not a bypass): on `migrate_from_blend`, Blend repay/sweep is keyed to **`caller`**, while hub debt/supply is applied to **`account_id`**. A listed active delegate can therefore move *their own* Blend book into the owner’s account (minting debt there). That still requires INV-AUTH-02 authority on the destination account.

Residual reentrancy note (A007/A011): `flash_position` checks the gate once at entry. A self-owned receiver may transfer the NFT mid-callback; the current call still finalizes under the entry-time authorization, and subsequent owner-gated verbs fail for the old owner (`tests/test-harness/tests/strategy/flash_position_callback_ownership_transfer.rs`). That is intentional finalize-under-opened-id behavior, not an open stranger gate.

---

## Evidence — tests & docs

### Unit (`contracts/controller/tests/helpers/account.rs`)

- `owner_passes`, `stranger_rejected`, `active_registered_opted_in_delegate_passes`
- `registered_but_not_opted_in_rejected`, `opted_in_but_manager_inactive_rejected`
- `transfer_revokes_prior_owner_and_delegates`
- `require_account_owner_rejects_active_delegate` (delegates cannot grant)

### Harness

| Concern | Test |
|---|---|
| Stranger borrow | `controller/security_audit_extended.rs::refutation_third_party_cannot_borrow_on_victim` |
| NFT transfer kills withdraw / delegate borrow | `controller/position_nft.rs::{old_owner_is_rejected_after_transfer,transfer_revokes_old_owners_delegates}` |
| Delegated borrow happy path | `controller/borrow.rs::{test_delegated_borrow_routes_funds_to_owner,…}` |
| Stranger multiply | `strategy/edge/multiply.rs` (BOB on Alice Multiply account → `#44`) |
| Stranger flash_position | `strategy/flash_position.rs::test_flash_position_rejects_non_owner` |
| Stranger swap_debt | `strategy/edge/rejections.rs::test_swap_debt_wrong_account_owner` |
| Stranger repay_debt_with_collateral | `strategy/edge/rejections.rs` (bob on alice_account_id → `#44`) |
| Stranger swap_collateral | `strategy/edge/rejections.rs::test_swap_collateral_wrong_account_owner` |
| Mid-callback NFT handoff | `strategy/flash_position_callback_ownership_transfer.rs` |
| Missing-auth strategy entrypoints | `strategy/edge/rejections.rs::test_strategy_entrypoints_reject_missing_owner_auth` |

### Docs / permissionless inventory

- `docs/reference/invariants.md` INV-AUTH-02
- `STRIDE.md` Spoof.3 / R.1 triple gate
- `scripts/permissionless_entrypoints.txt` — all eight listed as `caller-auth` with INV-AUTH-02 and body text naming `require_owner_or_delegate` or Multiply/Migrate guards

### Formal verification gap (info)

INV-AUTH-02’s “VERIFIED” line cites Certora rule `supply_new_slot_requires_owner_or_delegate` (`certora/controller/spec/market_guard_rules.rs`), which proves the **supply new-slot** stranger case (INV-AUTH-03 adjacent), not borrow/withdraw/strategy owner gates. Runtime/harness evidence for the eight paths is strong; prover coverage for those paths is thin. Remediation would be new revert rules (stranger caller ≠ owner, no active manager) on `borrow` / `withdraw` / strategy entrypoints — not a code fix.

---

## Call-site inventory of `require_owner_or_delegate` / `is_owner_or_delegate`

| Location | Symbol | Role |
|---|---|---|
| `account.rs:104,108` | `require_owner_or_delegate` | `AccountGuard::Migrate` / `Multiply` |
| `account.rs:118–143` | definitions | Gate implementation |
| `positions/debt.rs:45` | `require_owner_or_delegate` | `process_borrow` |
| `positions/supply.rs:92` | `is_owner_or_delegate` | Third-party supply slot rule |
| `positions/supply.rs:171` | `require_owner_or_delegate` | `process_withdraw` |
| `strategies/swap_debt.rs:52` | `require_owner_or_delegate` | `process_swap_debt` |
| `strategies/swap_collateral.rs:50` | `require_owner_or_delegate` | `process_swap_collateral` |
| `strategies/repay_debt_with_collateral.rs:55` | `require_owner_or_delegate` | `process_repay_debt_with_collateral` |
| `positions/liquidation/mod.rs:203` | `require_owner_or_delegate` | Credit seize receiver |

No additional production call sites exist under `contracts/controller/src/`.

---

## Summary judgment

**Status: defended.** INV-AUTH-02 holds on borrow, withdraw, multiply, flash_position, swap_debt, swap_collateral, repay_debt_with_collateral, and migrate_from_blend. No scoped path mutates another account’s risk without the owner-or-delegate gate (or self-create on `account_id == 0`). Highest residual is Certora coverage skew toward supply-slot rather than these eight mutators — tracking item for A108, not an undefended runtime hole.
