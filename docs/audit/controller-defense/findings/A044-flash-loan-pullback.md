# A044 — Flash loan principal+fee pullback

- Agent: A044
- Theme: T3
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/strategies/flash_loan.rs:18-48` (`process_flash_loan`); `contracts/controller/src/external/pool.rs:101-112` (`pool_flash_loan_call`); `contracts/pool/src/ops/flash.rs` (full `apply` / `terms` / `collect_repayment` / `book_fee` / `finalize`); `contracts/pool/src/lib.rs:190-203` (`#[only_owner] flash_loan`); `common/src/math/fp.rs:298-307` (`Bps::flash_loan_fee_on`); `common/src/validation.rs:75-80` (`require_wasm_receiver`); `contracts/controller/src/storage/account.rs` (`with_flash_guard`); `contracts/controller/src/risk/validation.rs:12-24`
- Defense: Controller gates auth, pause, hub activity, positive amount, and Wasm receiver, then wraps the entire pool FFI in `with_flash_guard`. Pool (owner-only) accrues, requires `is_flashloanable` + cash reserves, pays principal out of SAC, invokes `execute_flash_loan`, refuses any mid-callback SAC drift, allowance-pulls exact `principal + fee` via `transfer_from`, re-asserts SAC equals `pre_balance + fee`, then books **only the fee** into cash/revenue. Push repayments, under-approve, FOT, and extra-credit tokens fail closed (`InvalidFlashloanRepay` #402).
- Gap: (1) Controller does not re-measure pool SAC after return — trusts the owner-gated pool for INV-FLASH-01 (by design; cross-ref A041/A082 custody vs spoke trust). (2) Unlike `flash_position`, `flash_loan` has no `require_external_recipient` ban on controller/pool as `receiver`; rejection is fail-closed because those contracts lack `execute_flash_loan` (pinned by adversarial tests). (3) Non-exact-delivery / rebasing listed assets cannot flash — accepted residual; never set `is_flashloanable` (threat-model). (4) Controller does not pre-check `is_flashloanable` (pool does); fail-closed at pool with #401.
- Impact: Successful path always returns pool SAC to `pre + fee` and credits accounting cash by exactly `fee`; principal never enters the cash book. Failed repayment / balance-bracket violation rolls back SAC, cash, and revenue atomically. Blast radius of a malicious Wasm receiver is limited to their own callback funds plus any prefunded fee — they cannot leave the pool underpaid or inflate protocol cash with donations. Market-wide theft via flash underpayment is not available while INV-FLASH-01 holds.
- Evidence: INV-FLASH-01; INV-FLASH-02 (guard window around pullback); ADR-0010; STRIDE Spoof.4 / Tamper.5 / I7; threat-model flash-loan row; `errors.md` #400/#401/#402/#412; Certora `flash_repayment_terms_recover_principal_and_fee`, `flash_fee_booking_is_exact`, `flash_apply_accounting_books_fee_without_principal_cash`, `flash_apply_accounting_zero_fee_is_cash_noop`, `flash_loan_guard_*`; pool `flows.rs` flash tests; harness `controller/flash_loan.rs`, `controller/flash_loan_adversarial.rs`; permissionless justification line for `controller::flash_loan`
- Opinion: Primary money-movement defense for permissionless cash flash loans holds. Pullback is enforced at the pool SAC boundary with three exact-balance brackets plus allowance scope; controller’s job is auth + flash guard + Wasm gate, not a second cash measurement. Treat any weakening of the post-callback `require_balance(..., balance_after_payout)`, the allowance check, or the post-`transfer_from` `balance_after_repayment` assert as Critical against INV-FLASH-01. Cross-ref A007 (reentrancy), A019 (Wasm receiver), A055 (lying tokens / listing trust).

## Method

1. Traced controller `flash_loan` entrypoint → `process_flash_loan` → `with_flash_guard` → `pool_flash_loan_call` → pool `ops::flash::apply`.
2. Decomposed pool sequence: `prepare` → `terms` → payout → balance bracket → callback → balance bracket → `collect_repayment` → `finalize`/`book_fee`.
3. Checked fee math (`flash_loan_fee_on`), cash vs SAC accounting (principal not booked), and owner-only pool surface.
4. Cross-checked INV-FLASH-01/02, ADR-0010, STRIDE Spoof.4, threat-model exact-balance residual, Certora pool accounting rules, pool unit tests, and harness success / underpay / push / FOT / extra-credit / reentry suites.
5. Compared to peer scopes A007, A019, A041, A055 for overlap vs novel pullback claims.

---

## 1. Controller surface (orchestration only)

```18:48:contracts/controller/src/strategies/flash_loan.rs
pub(crate) fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    require_authorized_caller(env, caller);
    require_positive_amount(env, amount);
    config::require_hub_active(env, hub_asset.hub_id);

    require_wasm_receiver(env, receiver);

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();

    let fee = storage::with_flash_guard(env, || {
        pool_flash_loan_call(env, &pool_addr, hub_asset, caller, receiver, amount, data)
    });

    FlashLoanEvent { /* ... amount, fee ... */ }.publish(env);
}
```

| Gate | Where | Role for pullback |
|---|---|---|
| `#[when_not_paused]` | `lib.rs:178` | Closes flash while halted (INV-HALT-01) |
| `require_authorized_caller` | `:26` | `caller.require_auth()` + `require_not_flash_loaning` (blocks nested flash) |
| `require_positive_amount` | `:27` | No zero/negative principal |
| `require_hub_active` | `:28` | Dead hubs cannot open flash |
| `require_wasm_receiver` | `:30` | EOA receivers cannot play approve games (Spoof.4); pool repeats the check |
| `with_flash_guard` | `:35-37` | Entire pool payout+callback+pullback window; clears only if outer was clear |
| Event after success | `:39-47` | Fee taken from pool return value (trusted owner-gated callee) |

Controller does **not**: measure SAC deltas, compute the fee locally for enforcement, check `is_flashloanable`, or pull tokens itself. Money integrity for principal+fee is entirely the pool’s `apply` path. That matches the controller/pool ownership boundary (STRIDE I24 / I7).

FFI is a thin client:

```101:112:contracts/controller/src/external/pool.rs
pub(crate) fn pool_flash_loan_call(...) -> i128 {
    LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(hub_asset, initiator, receiver, &amount, data)
}
```

Permissionless justification (`scripts/permissionless_entrypoints.txt`): anyone may initiate within one call; pool verifies principal+fee back before return; flash flag blocks monetary reentrancy.

---

## 2. Pool pullback sequence (INV-FLASH-01)

Pool entrypoint is `#[only_owner]` — only the controller may call it. Direct attacker calls fail (`flows.rs::test_flash_loan_rejects_direct_non_owner_pool_call`).

### 2.1 Ordered steps in `ops::flash::apply`

| Step | Code | Defense |
|---|---|---|
| 1. Accrue + enable + cash | `prepare` | `require_positive_amount`; `is_flashloanable` else #401; `require_reserves(amount)` on **accounting cash** |
| 2. Wasm again | `require_wasm_receiver` | Defense in depth vs controller |
| 3. Snapshot terms | `terms(amount, fee_bps, pre_balance)` | Fee, `total_repayment`, expected SAC after payout, expected SAC after repay |
| 4. Pay principal | `asset.transfer(pool → receiver, amount)` | Live SAC outflow |
| 5. Bracket A | `require_balance(..., balance_after_payout)` | FOT / short delivery on outbound fails #402 |
| 6. Callback | `invoke_contract(execute_flash_loan)` | Untrusted code; fee+principal+pool passed in |
| 7. Bracket B | `require_balance(..., balance_after_payout)` **again** | Push-to-pool / donation / mid-callback SAC change fails #402 |
| 8. Collect | `collect_repayment` | `allowance >= total_repayment` then `transfer_from(pool, receiver, pool, total)` |
| 9. Bracket C | `require_balance(..., balance_after_repayment)` | Post-pull SAC must equal `pre + fee` |
| 10. Book | `finalize` → `book_fee` | Credit cash by `fee` only; mint revenue shares; commit + market event |

Exact identities from `terms`:

```
fee                    = flash_loan_fee_on(fee_bps, amount)   // half-up; min 1 if bps > 0
total_repayment        = amount + fee
balance_after_payout   = pre_balance - amount
balance_after_repayment = pre_balance + fee
```

Certora `flash_repayment_terms_recover_principal_and_fee` proves these identities and `after_repayment - after_payout == total`.

### 2.2 Why push repayment cannot impersonate

ADR-0010 / Spoof.4: repayment is **allowance-pulled**, not “balance increased somehow.” Bracket B requires SAC still equal `pre - amount` **before** pull. A receiver that transfers dust into the pool during the callback trips Bracket B (`PushToPool` / `PoolCallbackOverpayReceiver`). Only after that exact bracket does `transfer_from` move `amount + fee`.

Over-approve (`OverRepay`) still pulls exactly `total_repayment`; cash/revenue increase by `fee` only (`flash_loan_adversarial.rs::test_flash_loan_over_repay_still_charges_exact_fee`).

Under-approve fails the allowance assert (`PoolUnderRepayReceiver`, harness underpay / no-fee-prefund cases).

### 2.3 Cash book vs SAC (principal is not “borrowed” in accounting)

```127:132:contracts/pool/src/ops/flash.rs
pub(crate) fn book_fee(cache: &mut Cache, fee: i128) {
    let protocol_fee = Ray::from_asset(cache.env(), fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, protocol_fee);
    cache.credit_cash(fee);
}
```

- Principal out/in never calls `debit_cash` / `credit_cash`.
- Successful path: accounting cash `+= fee`; SAC `+= fee` vs pre-loan snapshot.
- Certora `flash_apply_accounting_books_fee_without_principal_cash` and `flash_fee_booking_is_exact` pin cash/revenue/supplied deltas; borrowed and indexes unchanged.
- Zero-fee market: cash/revenue noop (`flash_apply_accounting_zero_fee_is_cash_noop`; harness `test_flash_loan_allows_zero_fee_when_configured_zero`) — intentional free flash when governance sets `flashloan_fee = 0`.

Liquidity gate uses accounting cash (`require_reserves`), not raw SAC surplus. Donated untracked SAC cannot be flash-loaned beyond booked cash. If SAC < cash (desync), outbound transfer or Bracket A fails closed.

---

## 3. Fee construction (protocol-favoring)

```298:307:common/src/math/fp.rs
pub fn flash_loan_fee_on(self, env: &Env, amount: i128) -> i128 {
    let fee_amount = self.apply_to(env, amount); // half_up BPS
    if self.raw() > 0 && fee_amount == 0 {
        1
    } else {
        fee_amount
    }
}
```

- Half-up BPS (`formulas.md`); positive bps never rounds to a free loan (min 1 unit) — harness `test_flash_loan_tiny_amount_charges_min_fee_when_bps_positive`.
- `flashloan_fee` capped at `MAX_FLASHLOAN_FEE_BPS = 500` at market param validation.
- Overflow on `amount + fee` / balance arithmetic → `MathOverflow` (fail closed).

---

## 4. Adversarial / regression evidence map

| Attack / behavior | Expected | Evidence |
|---|---|---|
| No approve / no repay | #402 or transfer fail; rollback | pool `test_flash_loan_callback_failure_rolls_back_pool_state`; harness `test_flash_loan_rejects_bad_repayment` |
| Under-approve | #402 | pool `test_flash_loan_rejects_under_repay_*`; mock `UnderRepay` |
| Push tokens mid-callback | #402 Bracket B | pool `test_flash_loan_rejects_callback_balance_change`; harness `test_flash_loan_push_to_pool_fails_closed` |
| Over-approve | Success; pull exact; fee only | harness `test_flash_loan_over_repay_still_charges_exact_fee` |
| Fee-on-transfer token | Fail closed; reserves unchanged | harness `test_flash_loan_fee_on_transfer_fails_closed` |
| Extra-credit transfer_from | Fail exact post-repay bracket | harness `test_flash_loan_extra_credit_is_not_pool_theft` |
| Transfer-hook reentry | Fail / blocked | harness `test_flash_loan_transfer_hook_cannot_reenter` (+ A007) |
| Nested controller monetary verbs | #400 | `flash_loan_adversarial.rs` reenter matrix; A007 |
| Nested pool flash | Owner auth / guard | mock `ReenterPoolFlashLoan`; owner-only |
| EOA receiver | #412 | pool + harness |
| Controller / pool as receiver | Err (no callback) | harness `test_flash_loan_*_receiver_rejects` |
| Market not flashloanable | #401 | pool + harness |
| Insufficient cash | #112 | pool + harness |
| Paused controller | `ContractPaused` | harness `test_flash_loan_rejects_when_paused` |
| Happy path fee to reserves/revenue | `reserves += fee` | pool `test_flash_loan`; harness strict success |

Atomicity: any assert/panic after payout rolls back SAC and storage; harness asserts reserves and flash-guard cleared after failure.

---

## 5. Trust boundary and residual gaps

**In scope for A044 (pullback): defended.**

The money invariant is enforced where the tokens live (pool SAC + cash book), with the controller supplying auth, pause, Wasm, and the reentrancy envelope. That split is correct: a second controller-side SAC check would duplicate owner-trusted pool logic without enlarging the trust set.

**Residuals (not pullback breaks):**

1. **Listing trust** — Assets that do not deliver exact amounts can never complete a flash; governance must not enable `is_flashloanable` on them (threat-model; A055). FOT/extra-credit fail closed rather than silently desync.
2. **No `require_external_recipient` on flash_loan** — `flash_position` explicitly bans controller/pool recipients to protect measurement; cash flash relies on missing `execute_flash_loan`. Fail-closed today; a future controller method named `execute_flash_loan` would be a footgun — consider aligning with an explicit reject if the surface grows (defense-in-depth, not a current fund-loss path).
3. **Controller trusts pool fee return for events** — same owner trust as all other pool FFI; not independently recomputed for enforcement.
4. **Zero-fee config** — free flash loans when `flashloan_fee = 0`; liquidity still gated by reserves; principal still fully pulled back.

**Not this agent’s primary scope (covered elsewhere):** INV-FLASH-02 reentrancy inventory (A007); Wasm gate alone (A019); measured custody on other legs (A041/A045).

---

## 6. Verdict

**Defended** for flash-loan principal+fee pullback. INV-FLASH-01 is enforced by allowance-scoped `transfer_from` plus three exact SAC brackets around payout, callback, and collection, with fee-only cash booking afterward. Controller defenses correctly wrap that window rather than re-implement it. No undefended pullback gap with fund-theft impact was found under the SAC / exact-delivery listing assumption.
