# A007 — Flash-loan ongoing guard vs reentrancy into position flows

- Agent: A007
- Theme: T1
- Severity: low
- Status: defended
- Paths: `contracts/controller/src/storage/account.rs:282-312` (`is_flash_loan_ongoing` / `set_flash_loan_ongoing` / `with_flash_guard`); `contracts/controller/src/risk/validation.rs:12-24` (`require_authorized_caller` / `require_not_flash_loaning`); six production setters listed below; eighteen monetary entrypoints that check the flag
- Defense: Temporary-storage session flag wraps every untrusted handoff (flash callback, flash-position forward+callback, swap router, pool withdraw-to-controller, strategy borrow transfer, Blend submit). Nesting preserves the outer flag. All monetary position / strategy / liquidation / keeper entrypoints refuse with `FlashLoanOngoing` (#400) while the flag is set. Strategies do not bypass the check: each strategy entrypoint calls `require_authorized_caller` first, then only sets the flag around external windows.
- Gap: (1) Intentional — `renew_account` / `add_delegate` / `remove_delegate` stay reachable under the flag (non-monetary; GH-28 / `reentrancy_matrix`). (2) Intentional — `ControllerAdmin` / views never check the flag. (3) Residual — after a setter window closes, same-invocation legs (`process_deposit`, refund transfers, swap leftover `transfer`) run with the flag clear; a governance-listed token with a transfer hook could re-enter monetary entrypoints against still-unpersisted in-memory strategy state. Mitigated by listing trust + measured settlement, not by the flash flag. (4) Coverage — `reentrancy_matrix` omits `migrate_from_blend`, `recapitalize`, and `force_socialize_bad_debt` (code still gates all three).
- Impact: Callback / router / Blend / debt-forward reentrancy cannot mutate positions mid-window: blocked at every monetary entrypoint. Residual post-guard listed-token hooks could race finalize only if governance lists a reentrant token; blast radius is then one account’s in-flight strategy state (clobber / lost-update vs outer `strategy_finalize`), not unaccounted pool cash theft. Non-monetary delegate/TTL verbs under the flag cannot move funds.
- Evidence: INV-FLASH-02; STRIDE Tamper.5; ADR-0020; `docs/reference/errors.md` #400; Certora `flash_loan_guard_*`; harness `meta/reentrancy_matrix.rs`, `strategy/flash_position_adversarial.rs`, `strategy/flash_position.rs`, `poc_multiply_reentrancy.rs`, `controller/flash_loan.rs`, `strategy/adversarial.rs`
- Opinion: Primary defense holds. Strategies are setters and checkers, not bypasses. Treat any new untrusted handoff without `with_flash_guard`, or any new monetary entrypoint without `require_not_flash_loaning` / `require_authorized_caller`, as a Critical regression against INV-FLASH-02.

## Method

1. Read guard primitive and nesting semantics in `storage/account.rs`.
2. Enumerated every production `with_flash_guard` setter under `contracts/controller/src/`.
3. Enumerated every `require_not_flash_loaning` / `require_authorized_caller` checker and mapped them to `ControllerInterface` entrypoints in `lib.rs`.
4. Traced strategy bodies for internal helpers that skip the entry check (`process_deposit`, `borrow_into_controller`, router leftover transfer, refunds) and whether they run inside or outside the flag.
5. Cross-checked INV-FLASH-02, STRIDE Tamper.5, permissionless list, harness matrix, Certora rules, and adversarial flash-position / multiply reentrancy tests.

---

## 1. Guard primitive

Temporary storage key `SessionKey::FlashLoanOngoing` (`storage/account.rs:278-280`):

| Symbol | Behavior |
|---|---|
| `is_flash_loan_ongoing` | `temporary().get(...).unwrap_or(false)` |
| `set_flash_loan_ongoing(true)` | writes `true` |
| `set_flash_loan_ongoing(false)` | **removes** the key (does not store `false`) |
| `with_flash_guard(f)` | records `prev`, sets true, runs `f`, clears **only if** `!prev` |

Nesting is load-bearing: `flash_position`’s outer guard wraps the receiver; inner `borrow_into_controller` also takes the guard for the pool transfer. Without nesting, the inner clear would open the callback window. Unit coverage: `contracts/controller/tests/storage/account.rs` nested-window test.

Checkers:

```12:24:contracts/controller/src/risk/validation.rs
pub(crate) fn require_authorized_caller(env: &Env, caller: &Address) {
    caller.require_auth();
    require_not_flash_loaning(env);
}

pub(crate) fn require_not_flash_loaning(env: &Env) {
    assert_with_error!(
        env,
        !storage::is_flash_loan_ongoing(env),
        FlashLoanError::FlashLoanOngoing
    );
}
```

Eight literal `require_not_flash_loaning` call sites (one inside the shared wrapper + seven direct) cover **18** monetary entrypoints — matches STRIDE Tamper.5.

---

## 2. Who sets the flag (`with_flash_guard`)

Exactly six production setters (INV-FLASH-02 inventory):

| # | Site | Window covered |
|---|---|---|
| 1 | `strategies/flash_loan.rs:35` | Pool `flash_loan` (funds to receiver + callback + pullback) |
| 2 | `strategies/flash_position.rs:126` | Debt-token forward **and** `execute_flash_position` callback |
| 3 | `strategies/swap.rs:89` | Swap-aggregator `execute_strategy` |
| 4 | `strategies/legs.rs:103` | Pool withdraw of collateral into the controller |
| 5 | `positions/debt.rs:275` | Strategy borrow / create-strategy pool transfer into the controller |
| 6 | `external/blend.rs:91` | Blend `submit` during migrate |

No other production `with_flash_guard` call sites exist under `contracts/` (definition + re-export only).

Design note: ordinary `borrow` / `withdraw` / `supply` / `repay` entrypoints do **not** set the flag around their own pool legs. The flag exists for untrusted **callbacks and hooks** during strategy / flash / external integration windows, not for every pool FFI.

---

## 3. Which entrypoints check the flag

### 3.1 Via `require_authorized_caller` (auth + flash check)

| Entrypoint | Process fn | Check line |
|---|---|---|
| `supply` | `positions/supply.rs::process_supply` | `:51` |
| `withdraw` | `positions/supply.rs::process_withdraw` | `:168` |
| `borrow` | `positions/debt.rs::process_borrow` | `:42` |
| `repay` | `positions/debt.rs::process_repay` | `:87` |
| `flash_loan` | `strategies/flash_loan.rs::process_flash_loan` | `:26` |
| `flash_position` | `strategies/flash_position.rs::process_flash_position` | `:47` |
| `multiply` | `strategies/multiply.rs::process_multiply` | `:39` |
| `swap_debt` | `strategies/swap_debt.rs` | `:41` |
| `swap_collateral` | `strategies/swap_collateral.rs` | `:43` |
| `repay_debt_with_collateral` | `strategies/repay_debt_with_collateral.rs` | `:48` |
| `migrate_from_blend` | `strategies/migrate_blend.rs::process_migrate_blend` | `:44` |

= **4 position + 7 strategy** entrypoints.

### 3.2 Direct `require_not_flash_loaning` (auth separate)

| Entrypoint | Process fn | Check line |
|---|---|---|
| `liquidate` | `positions/liquidation/mod.rs::process_liquidation` | `:54` (after `liquidator.require_auth`) |
| `clean_bad_debt` | `process_clean_bad_debt` | `:222` |
| `force_socialize_bad_debt` | `process_force_socialize_bad_debt` | `:274` (owner-gated at `lib.rs`; no caller auth in body) |
| `update_indexes` | `keepers.rs` | `:19` |
| `claim_revenue` | `keepers.rs` | `:30` |
| `recapitalize` | `keepers.rs` | `:50` |
| `update_account_threshold` | `keepers.rs` | `:77` |

= **3 liquidation/bad-debt + 4 keeper** entrypoints.

**Total monetary surface gated: 18.** Aligns with STRIDE Tamper.5 and `docs/reference/errors.md` #400.

### 3.3 Entrypoints that do **not** check (by design)

| Entrypoint | Why ungated |
|---|---|
| `renew_account` | TTL only; `account.rs:228-237` — auth + owner, no flash check |
| `add_delegate` / `remove_delegate` | Authority map only; `account.rs:241-251` — pinned by harness GH-28 |
| All `ControllerAdmin` mutators | Owner/timelock surface; no flash check |
| Views | Read-only |

Pinned test: `tests/test-harness/tests/meta/reentrancy_matrix.rs::delegate_and_renew_verbs_stay_reachable_under_flash_loan_ongoing`.

---

## 4. Bypass via strategies? — No

Question: can a flash / strategy callback reach position flows by going through another strategy, or by using strategy internals that skip the flag?

### 4.1 Strategy entrypoints cannot be nested under an ongoing flag

Every strategy entry calls `require_authorized_caller` **before** any `with_flash_guard`. Nested `flash_loan` / `flash_position` / `multiply` / `swap_*` / `migrate_from_blend` from a callback therefore hit `FlashLoanOngoing` the same way `supply`/`borrow` do.

Evidence:

- `flash_position_adversarial.rs` — reenter flash_loan, flash_position, borrow, withdraw, repay all fail; guard clears on rollback.
- `flash_position.rs::test_flash_position_reenter_supply_rejects` — callback cannot reenter supply (host may surface `InvalidAction` for same-contract reentry; flag semantics pinned by `test_flash_position_rejects_during_flash_loan`).
- `flash_position.rs::test_flash_position_rejects_during_flash_loan` — entry blocked when flag already set.
- `strategy/adversarial.rs::test_strategy_entries_still_blocked_by_flag` — multiply / swap_debt / swap_collateral / repay_debt_with_collateral blocked.
- Debt-token transfer hook during `flash_position` forward fails closed (`test_flash_position_transfer_hook_cannot_reenter`) — that transfer sits **inside** the outer guard.

### 4.2 Strategies set the flag; they do not clear a path around checkers

During guarded windows, any cross-contract reentry into the controller must hit a `#[contractimpl]` entrypoint. Those monetary entrypoints check the flag. There is no alternate “strategy-only” FFI that mutates positions without going through a checked entrypoint.

Internal helpers used **inside** a strategy (`borrow_into_controller`, `process_deposit`, `swap_tokens`, Blend submit) are same-invocation Rust calls, not reentrancy. They correctly omit a second entry check; the entrypoint already gated the call.

### 4.3 Nested guards during strategy composition

Example `flash_position`:

1. Entry: `require_authorized_caller` (flag must be clear).
2. Outer `with_flash_guard` around mint/forward + callback.
3. Inner `borrow_into_controller` → `with_flash_guard` around pool transfer; nesting leaves flag set for the callback.
4. Guard clears only after the receiver returns.
5. Then `process_deposit` / refunds / `strategy_finalize` run with flag clear (continuation, not callback reentrancy).

Example `multiply`: brief guard on strategy borrow transfer; separate guard on router; deposit after router returns is unguarded continuation.

Blend migrate: entry check + `guarded_submit` during Blend `submit` so a Blend pool hook cannot reenter monetary paths.

---

## 5. Residual windows (not strategy bypasses)

These are same-invocation legs **after** a setter clears, while outer strategy state may still be in-memory only (`strategy_finalize` / `finalize_position_flow` persists later).

| Window | Flag | Concern |
|---|---|---|
| `flash_position` post-callback `process_deposit` (collateral controller→pool) | clear | Listed collateral transfer hook could reenter monetary entrypoints; storage may still lack the newly minted debt |
| `flash_position` `refund_listed_assets` | clear | Documented in `endpoints.md`; mitigated by **listed-only** `refund_assets` |
| `swap_tokens` leftover `token_in.transfer` after router guard | clear | Listed/router path; leftover return to `refund_to` |
| `multiply` / swap strategies `process_deposit` after swap | clear | Same class as flash_position deposit |

Protocol stance (from code comments + `endpoints.md`): post-guard token addresses must be governance-listed; unlisted refund assets are rejected (`test_flash_position_rejects_unlisted_refund_asset`). That is a **listing trust** boundary, not a flash-flag boundary.

Severity for this residual: **low** under the current trust model (governance-approved tokens). Would rise if a listed asset gains an arbitrary reentrant transfer hook — then outer `strategy_finalize` could clobber or race a reentrant mutation of the same `account_id`.

Soroban same-contract reentry may also fail with `InvalidAction` independently; the flash flag is the portable defense for **cross-contract** reentry into the controller.

---

## 6. Evidence matrix

| Claim | Evidence |
|---|---|
| INV-FLASH-02 enforced | Six setters; `require_not_flash_loaning`; nesting in `with_flash_guard` |
| 18 monetary entrypoints gated | Tables §3.1–3.2; STRIDE Tamper.5 |
| Strategies not a bypass | All 7 strategy entries use `require_authorized_caller`; adversarial + matrix tests |
| Nested flash / position reentry blocked | `flash_position_adversarial.rs`, `flash_loan.rs` reentrancy tests |
| Non-monetary verbs intentionally ungated | `reentrancy_matrix.rs` GH-28 test |
| Formal | Certora `flash_loan_guard_blocks_callers`, `_blocks_supply_entrypoint`, `_blocks_liquidation_entrypoint`, `_cleared_after_summarized_pool_return` |
| Docs | INV-FLASH-02; errors #400; ADR-0020; threat-model flash-loan row |

### Coverage gaps (code gated; tests incomplete)

| Entrypoint | Code check | Dedicated flag test in `reentrancy_matrix`? |
|---|---|---|
| `migrate_from_blend` | yes (`:44`) | no (partial elsewhere / adversarial strategy set) |
| `recapitalize` | yes (`keepers.rs:50`) | **no** dedicated `FLASH_LOAN_ONGOING` assert found |
| `force_socialize_bad_debt` | yes (`liquidation/mod.rs:274`) | **no** dedicated assert found |

Recommend extending `reentrancy_matrix` (or sibling) to assert #400 on those three so STRIDE’s “18 entrypoints” claim stays mechanically pinned.

---

## 7. Verdict

**Defended** for the scoped question: flash-loan / flash-position / router / Blend / debt-forward windows cannot reenter position or strategy monetary flows; strategies neither skip nor weaken the check.

Residual **low**: post-guard listed-token transfer hooks during strategy settlement; intentional ungated TTL/delegate verbs; incomplete matrix coverage for three already-gated entrypoints.

No production code change recommended from this audit alone. If remediation is later scoped: (a) extend `reentrancy_matrix` for `migrate_from_blend` / `recapitalize` / `force_socialize_bad_debt`; (b) optionally hold the flash flag through `flash_position`’s post-callback `process_deposit` + refunds until `strategy_finalize` returns, if listing trust is judged insufficient for transfer-hook assets.
