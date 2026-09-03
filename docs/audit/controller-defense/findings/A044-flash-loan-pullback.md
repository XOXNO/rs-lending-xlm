# A044 — Flash loan principal+fee pullback

- Agent: A044
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs:178-188` (`#[when_not_paused] flash_loan`)
  - `contracts/controller/src/strategies/flash_loan.rs:18-48` (`process_flash_loan`)
  - `contracts/controller/src/external/pool.rs:101-112` (`pool_flash_loan_call`)
  - `contracts/pool/src/lib.rs:190-203` (`#[only_owner] flash_loan`)
  - `contracts/pool/src/ops/flash.rs` (`apply`, `prepare`, `terms`, `invoke_receiver`, `collect_repayment`, `require_balance`, `book_fee`, `finalize`)
  - `contracts/pool/src/cache/cash.rs:14-19` (`require_reserves`)
  - `common/src/math/fp.rs:298-307` (`Bps::flash_loan_fee_on`)
  - `common/src/validation.rs:75-80` (`require_wasm_receiver`)
  - `contracts/controller/src/storage/account.rs` (`with_flash_guard`)
  - `contracts/controller/src/risk/validation.rs:12-24`
- Defense: Controller gates pause, caller auth, nested-flash refusal, positive amount, hub activity, and Wasm receiver, then wraps the entire pool FFI (payout + callback + pullback) in `with_flash_guard`. Pool is owner-only: accrues, requires `is_flashloanable` and accounting-cash reserves for principal, pays SAC principal out, brackets SAC exactly after payout, invokes `execute_flash_loan`, brackets SAC again (must be unchanged through the callback), allowance-checks then `transfer_from`s exact `principal + fee`, brackets SAC to `pre_balance + fee`, then books **only the fee** into cash and protocol revenue. Push repayments, under-approve, fee-on-transfer, and extra-credit tokens fail closed (`InvalidFlashloanRepay` #402) with full tx rollback.
- Gap: (1) Controller does not re-measure pool SAC after return — INV-FLASH-01 is enforced at the owner-gated pool (correct trust split; cf. A041/A082). (2) `flash_loan` has no `require_external_recipient` ban on controller/pool as `receiver` (unlike `flash_position`); those addresses fail closed today because they lack `execute_flash_loan` (adversarial tests). (3) Non-exact-delivery / rebasing assets cannot complete a flash — accepted listing residual; never set `is_flashloanable` (threat-model; A055). (4) Controller does not pre-check `is_flashloanable` (pool #401). (5) Intentional policy: flash skips `require_liquidation_buffer` and `require_utilization_below_max` (temporary SAC draw; no debt mint; atomic restore).
- Impact: Successful path always leaves pool SAC at `pre + fee` and credits accounting cash by exactly `fee`; principal never enters the cash book and never mints borrow shares. Failed repayment or any balance-bracket violation rolls back SAC, cash, and revenue atomically. A malicious Wasm receiver cannot leave the pool underpaid, inflate protocol cash via donations mid-callback, or steal via over-approve. Market-wide theft via flash underpayment is unavailable while INV-FLASH-01 holds. Mid-callback temporary `cash > SAC` cannot be exploited: pool mutations are owner-only and controller monetary verbs are flash-guarded.
- Evidence: INV-FLASH-01; INV-FLASH-02 (guard around the pullback window); ADR-0010; STRIDE Spoof.4 / Tamper.5 / I7 / TB8; threat-model flash-loan row; `errors.md` #400/#401/#402/#412; `pool/README.md` three SAC equality checks; Certora `flash_repayment_terms_recover_principal_and_fee`, `flash_fee_booking_is_exact`, `flash_apply_accounting_books_fee_without_principal_cash`, `flash_apply_accounting_zero_fee_is_cash_noop`, controller `flash_loan_guard_*`; pool `flows.rs` flash tests; harness `controller/flash_loan.rs`, `controller/flash_loan_adversarial.rs`; `scripts/permissionless_entrypoints.txt` justification for `controller::flash_loan`
- Opinion: Primary money-movement defense for permissionless cash flash loans holds. Pullback is enforced at the pool SAC boundary with three exact-balance brackets plus allowance-scoped `transfer_from`; the controller’s job is auth + pause + Wasm + flash-guard envelope, not a second cash measurement. Treat any weakening of post-callback `require_balance(..., balance_after_payout)`, the allowance check, the exact `transfer_from` amount, or the post-pull `balance_after_repayment` assert as **Critical** against INV-FLASH-01. Cross-ref A007 (reentrancy), A019 (Wasm receiver), A030 (guard lifecycle), A055 (lying / non-SAC tokens).

## Method

1. Traced controller `flash_loan` → `process_flash_loan` → `with_flash_guard` → `pool_flash_loan_call` → pool `ops::flash::apply`.
2. Decomposed pool sequence step-by-step: `prepare` → `terms` → payout → Bracket A → callback → Bracket B → `collect_repayment` → Bracket C → `finalize`/`book_fee`.
3. Checked fee math (`flash_loan_fee_on`), cash vs SAC accounting (principal not booked), owner-only pool surface, and intentional omission of liquidation-buffer / max-utilization guards.
4. Analysed mid-callback `cash > SAC` window and whether any reachable path can drain the phantom cash.
5. Cross-checked INV-FLASH-01/02, ADR-0010, STRIDE Spoof.4 / I7, threat-model exact-balance residual, Certora pool accounting rules, pool unit tests, harness success / underpay / push / FOT / extra-credit / reentry suites, and peer findings A007 / A019 / A030 / A041 / A055.

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

    FlashLoanEvent { /* hub_id, asset, receiver, caller, amount, fee */ }
        .publish(env);
}
```

| Gate | Where | Role for pullback |
|---|---|---|
| `#[when_not_paused]` | `lib.rs:178` | Closes flash while halted (INV-HALT-01) |
| `require_authorized_caller` | `:26` | `caller.require_auth()` + `require_not_flash_loaning` (blocks nested flash) |
| `require_positive_amount` | `:27` | No zero/negative principal |
| `require_hub_active` | `:28` | Dead hubs cannot open flash |
| `require_wasm_receiver` | `:30` | EOA receivers cannot play approve games (Spoof.4); pool repeats the check |
| `with_flash_guard` | `:35-37` | Entire pool payout + callback + pullback; nesting-safe clear (A007/A030) |
| Event after success | `:39-47` | Fee taken from pool return value (owner-gated callee) |

Controller does **not**: measure SAC deltas, compute the fee locally for enforcement, check `is_flashloanable`, pull tokens, or run account solvency / borrowable / spoke-usage gates. Money integrity for principal+fee is entirely the pool’s `apply` path. That matches the controller↔pool ownership boundary (STRIDE I24 / I7 / TB8).

FFI is a thin client returning the fee:

```101:112:contracts/controller/src/external/pool.rs
pub(crate) fn pool_flash_loan_call(...) -> i128 {
    LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(hub_asset, initiator, receiver, &amount, data)
}
```

Permissionless justification (`scripts/permissionless_entrypoints.txt`):

> Anyone may borrow within a single call; the pool verifies principal plus fee is back before returning, and the flash-loan flag blocks monetary reentrancy into position flows.

---

## 2. Pool pullback sequence (INV-FLASH-01)

Pool entrypoint is `#[only_owner]` — only the controller may call it. Direct attacker calls fail (`flows.rs::test_flash_loan_rejects_direct_non_owner_pool_call`). Nested `ReenterPoolFlashLoan` from a callback therefore cannot open a second pool flash without controller ownership.

### 2.1 Ordered steps in `ops::flash::apply`

| Step | Code | Defense |
|---|---|---|
| 1. Accrue + enable + cash | `prepare` | `require_positive_amount`; `is_flashloanable` else #401; `require_reserves(amount)` on **accounting cash** |
| 2. Wasm again | `require_wasm_receiver` | Defense in depth vs controller |
| 3. Snapshot terms | `terms(amount, fee_bps, pre_balance)` | Fee, `total_repayment`, expected SAC after payout, expected SAC after repay |
| 4. Pay principal | `asset.transfer(pool → receiver, amount)` | Live SAC outflow |
| 5. Bracket A | `require_balance(..., balance_after_payout)` | FOT / short delivery on outbound fails #402 |
| 6. Callback | `invoke_contract(execute_flash_loan)` | Untrusted code; args: initiator, asset, amount, fee, pool, data |
| 7. Bracket B | `require_balance(..., balance_after_payout)` **again** | Push-to-pool / donation / mid-callback SAC change fails #402 |
| 8. Collect | `collect_repayment` | `allowance(receiver, pool) >= total_repayment` then `transfer_from(pool, receiver, pool, total)` |
| 9. Bracket C | `require_balance(..., balance_after_repayment)` | Post-pull SAC must equal `pre + fee` |
| 10. Book | `finalize` → `book_fee` | Credit cash by `fee` only; mint revenue shares; commit + market event |

Exact identities from `terms`:

```text
fee                     = flash_loan_fee_on(fee_bps, amount)  // half-up; min 1 if bps > 0
total_repayment         = amount + fee                        // checked_add → MathOverflow
balance_after_payout    = pre_balance - amount
balance_after_repayment = pre_balance + fee
```

Certora `flash_repayment_terms_recover_principal_and_fee` proves these identities and `after_repayment - after_payout == total`.

### 2.2 Why push repayment cannot impersonate (ADR-0010 / Spoof.4)

Repayment is **allowance-pulled**, not “balance somehow increased.” Bracket B requires SAC still equal `pre - amount` **before** pull. A receiver that transfers dust into the pool during the callback trips Bracket B (`PushToPool` / pool `test_flash_loan_rejects_callback_balance_change`). Only after that exact bracket does `transfer_from` move `amount + fee`.

| Receiver behavior | Outcome |
|---|---|
| No approve / `NoRepay` | #402 allowance assert |
| Approve `< principal+fee` / `UnderRepay` | #402 |
| Approve `>` owed / `OverRepay` | Success; pull still exactly `total_repayment`; cash/revenue `+= fee` only |
| Push tokens mid-callback / `PushToPool` | #402 Bracket B |
| Callback panic | Full rollback |
| Missing fee prefund | `transfer_from` fails / #402; reserves unchanged |

### 2.3 Cash book vs SAC (principal is not borrowed in accounting)

```127:132:contracts/pool/src/ops/flash.rs
pub(crate) fn book_fee(cache: &mut Cache, fee: i128) {
    let protocol_fee = Ray::from_asset(cache.env(), fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, protocol_fee);
    cache.credit_cash(fee);
}
```

- Principal out/in never calls `debit_cash` / `credit_cash`.
- Successful path: accounting cash `+= fee`; SAC `+= fee` vs pre-loan snapshot.
- No borrow shares minted; utilization and debt indexes unchanged by the flash itself.
- Certora `flash_apply_accounting_books_fee_without_principal_cash` / `flash_fee_booking_is_exact` pin cash/revenue/supplied deltas; borrowed and indexes unchanged.
- Zero-fee market: cash/revenue noop (`flash_apply_accounting_zero_fee_is_cash_noop`; harness `test_flash_loan_allows_zero_fee_when_configured_zero`) — intentional free flash when governance sets `flashloan_fee = 0`; principal still fully pulled back.

Liquidity gate uses accounting cash (`require_reserves`), not raw SAC surplus. Donated untracked SAC cannot be flash-loaned beyond booked cash. If SAC `<` cash (desync), outbound transfer or Bracket A fails closed.

`pool/README.md` states explicitly that flash_loan is the **only** pool path that reconciles against live `token.balance()` — three strict equality checks.

### 2.4 Mid-callback temporary `cash > SAC`

During the callback, accounting cash still includes the principal that has left SAC. That window is safe because:

1. All pool mutating entrypoints are `#[only_owner]` (controller).
2. Controller monetary entrypoints refuse with `#400` while `with_flash_guard` holds (INV-FLASH-02 / A007).
3. Token transfer hooks during payout or `transfer_from` also run under the same guard (adversarial `test_flash_loan_transfer_hook_cannot_reenter`).
4. Success restores SAC to `pre + fee` and then credits cash by `fee`; failure aborts the tx and rolls both ledgers back.

So the temporary book/SAC skew cannot be turned into a second draw or a free exit in the same invocation.

### 2.5 Intentional guard asymmetries (not pullback gaps)

From `contracts/pool/README.md` guards table:

| Guard | On flash_loan? | Rationale |
|---|---|---|
| `require_reserves` | yes | Cannot flash more cash than booked |
| `require_liquidation_buffer` | **no** | Temporary SAC draw; no lasting cash debit; liquidators cannot run under the flash guard anyway |
| `require_utilization_below_max` | **no** | Flash mints no debt; utilization unchanged |
| `require_backed_market` | no | Entry/exit policy elsewhere |

These omissions do not weaken INV-FLASH-01; they are liquidity-policy choices for an atomic temporary draw.

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

- Half-up BPS; positive bps never round to a free loan (min 1 unit) — harness `test_flash_loan_tiny_amount_charges_min_fee_when_bps_positive`.
- `flashloan_fee` capped at `MAX_FLASHLOAN_FEE_BPS = 500` (5%) at market param validation ⇒ for `amount >= 1`, fee ≤ amount under the cap (Certora assumes this bound).
- Overflow on `amount + fee` / balance arithmetic → `MathOverflow` (fail closed).
- Note: `StrategyFeeExceeds` (#409) applies to `create_strategy`, not cash `flash_loan`; flash relies on the fee-cap + checked add instead.

Callback is told the exact `fee` and `amount` so honest receivers can approve `amount + fee` without re-deriving bps.

---

## 4. Adversarial / regression evidence map

| Attack / behavior | Expected | Evidence |
|---|---|---|
| No approve / no repay | #402 or transfer fail; rollback | pool `test_flash_loan_callback_failure_rolls_back_pool_state`; harness `test_flash_loan_rejects_bad_repayment`, `…_no_repay_rejects` |
| Under-approve | #402 | pool `test_flash_loan_rejects_under_repay_*`; harness under-repay; mock `UnderRepay` |
| Push tokens mid-callback | #402 Bracket B | pool `test_flash_loan_rejects_callback_balance_change`; harness `test_flash_loan_push_to_pool_fails_closed` |
| Over-approve | Success; pull exact; fee only | harness `test_flash_loan_over_repay_still_charges_exact_fee` |
| Fee-on-transfer token | Fail closed; reserves unchanged | harness `test_flash_loan_fee_on_transfer_fails_closed` |
| Extra-credit transfer_from | Fail exact post-repay bracket | harness `test_flash_loan_extra_credit_is_not_pool_theft` |
| Transfer-hook reentry | Fail / blocked | harness `test_flash_loan_transfer_hook_cannot_reenter` (+ A007) |
| Nested controller monetary verbs | #400 | `flash_loan_adversarial.rs` reenter matrix; A007 |
| Nested pool flash | Owner auth fails | mock `ReenterPoolFlashLoan` |
| EOA receiver | #412 | pool + harness `test_flash_loan_eoa_receiver_still_rejected` |
| Controller / pool as receiver | Err (no `execute_flash_loan`) | harness `test_flash_loan_*_receiver_rejects` |
| Market not flashloanable | #401 | pool + harness |
| Insufficient cash | #112 | pool + harness |
| Zero amount | #14 | controller + pool |
| Paused controller | `ContractPaused` | harness `test_flash_loan_rejects_when_paused` |
| Happy path fee to reserves/revenue | `reserves += fee` | pool `test_flash_loan`; harness strict success |
| Min fee on tiny amount | fee = 1 when bps > 0 | harness tiny-amount test |
| Zero configured fee | free flash; principal returned | harness zero-fee test |

Atomicity: any assert/panic after payout rolls back SAC and storage; harness asserts reserves and flash-guard cleared after failure (Soroban tx abort — A030).

---

## 5. Trust boundary and residual gaps

**In scope for A044 (pullback): defended.**

The money invariant is enforced where the tokens live (pool SAC + cash book), with the controller supplying auth, pause, Wasm, and the reentrancy envelope. A second controller-side SAC check would duplicate owner-trusted pool logic without enlarging the trust set.

**Residuals (not pullback breaks):**

1. **Listing trust** — Assets that do not deliver exact amounts can never complete a flash; governance must not enable `is_flashloanable` on them (threat-model; A055). FOT/extra-credit fail closed rather than silently desync cash vs SAC.
2. **No `require_external_recipient` on cash `flash_loan`** — `flash_position` bans controller/pool recipients to protect measurement; cash flash relies on missing `execute_flash_loan`. Fail-closed today; a future controller method named `execute_flash_loan` would be a footgun — consider an explicit reject if that surface ever appears (defense-in-depth, not a current fund-loss path).
3. **Controller trusts pool fee return for events** — same owner trust as all other pool FFI; fee is not independently recomputed for enforcement.
4. **Zero-fee config** — free flash loans when `flashloan_fee = 0`; liquidity still gated by reserves; principal still fully pulled back.
5. **Docs drift (non-fund)** — `errors.md` lists controller `flash_loan` under #100 / #107 / #127 in places; `process_flash_loan` does not run HF / borrowable / utilization checks. Operational confusion only; pullback path unaffected.

**Not this agent’s primary scope (covered elsewhere):** INV-FLASH-02 setter/checker inventory (A007); flash-guard storage lifecycle (A030); Wasm gate alone (A019); measured custody on other legs (A041/A045); lying-token listing trust (A055).

---

## 6. Verdict

**Defended** for flash-loan principal+fee pullback. INV-FLASH-01 is enforced by allowance-scoped `transfer_from` plus three exact SAC brackets around payout, callback, and collection, with fee-only cash booking afterward. Controller defenses correctly wrap that window rather than re-implement it. No undefended pullback gap with fund-theft impact was found under the SAC / exact-delivery listing assumption.

Critical regression watchlist: remove or loosen Bracket B, skip the allowance pre-check without keeping exact post-pull equality, book principal into cash, or clear the flash guard before pool return completes.
