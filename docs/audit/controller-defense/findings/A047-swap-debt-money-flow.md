# A047 — Swap-debt legs money flow

- Agent: A047
- Theme: T3 (custody / measured settlement), T4 (risk gates after money movement)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/strategies/swap_debt.rs` (`process_swap_debt`)
  - `contracts/controller/src/strategies/legs.rs` (`repay_debt_from_controller`, `refund_controller_balance_delta`; siblings `withdraw_collateral_to_controller`, `execute_withdraw_all`, `net_settle_collateral_against_debt` — not on the swap-debt path)
  - `contracts/controller/src/strategies/swap.rs` (`swap_tokens` / `swap_tokens_or_passthrough` / `verify_router_output`)
  - `contracts/controller/src/positions/debt.rs` (`borrow_into_controller`, `execute_repayment`, `apply_repay_batch`)
  - `contracts/controller/src/payments.rs` (`balance_delta_since`); `common/src/token.rs` (`transfer_amount_measured`)
  - `contracts/pool/src/ops/strategy.rs` (`create_strategy`); `contracts/pool/src/ops/repay.rs` (pre-funded repay + overpay refund)
  - `contracts/controller/src/lib.rs:265-284` (`swap_debt` entrypoint)
- Defense: Refinance is a single atomic tx with three measured custody hops (pool→controller borrow net of fee → router swap → controller→pool repay). Debt shares follow pool mutations; token credit never trusts router/pool return figures alone. Residue (unspent `token_in`, repay overpay) returns to `caller` without sweeping unrelated controller balances. Post-leg `strategy_finalize` enforces the same solvency gates as ordinary borrows.
- Gap: (1) Controller-side slippage is only `received > 0`; min-out lives in the untrusted aggregator payload (threat-model / INV-STRAT-01 residual). (2) Leftover `token_in` and overpay refund transfers run with the flash flag clear — listed-token hook reentrancy residual already tracked by A007/A055. (3) `require_hub_active` on **existing** debt blocks refinance when that hub is deactivated; plain `repay` still works (availability, not fund theft). (4) Fee is debt-financed (`charge_fee: true`): cash available to repay is `amount - fee` while debt mints gross `amount`.
- Impact: No path found that strands strategy cash in the controller, credits debt without matching pool mutation, double-pulls repay tokens, or sweeps pre-existing controller balances to the caller. Router compromise can still economically drain an in-flight refinance down to the HF floor (documented unbounded-loss class).
- Evidence: INV-ACCT-03, INV-STRAT-01/02; ADR-0003/0011; threat-model “Controller to router and tokens”; harness `strategy/edge/swap.rs`, `strategy/happy.rs`, `controller/multi_hub.rs`, `controller/outbound_transfer_measurement.rs`; Certora `swap_debt_preserves_directional_bounds`
- Opinion: Money-flow shape matches the protocol’s custody rule (“measure at the controller boundary”). Do not weaken measured transfer / equality asserts / overpay delta snapshot. Cross-ref A032 (finalize batch), A041/A082 (measurement), A007 (flash windows), A072 (post-pool gates), A003 (delegate economic control of refunds).

## Scope

Audit of **token and debt-share money movement** on `swap_debt`: borrow new asset into controller custody, optional router swap into the existing debt asset, repay existing debt from controller, refund excess, finalize.

In scope from `legs.rs`: primarily `repay_debt_from_controller` + `refund_controller_balance_delta` (the only legs used by swap-debt). Sibling withdraw / withdraw-all / net-settle helpers are inventoried for contamination and pattern consistency only.

Out of scope for depth (peer agents): auth/pause/flash entry gates (A001/A003/A007), strategy finalize batching (A032), lying-token / listing trust (A055), cache staleness (A094), spoke usage deltas (A076/A082).

## Verdict

**Defended.** End-to-end cash and share flows are measured, directionally correct, and closed under Soroban transaction atomicity. Residuals are known policy/trust-boundary items (router min-out, listing hooks, inactive-hub refinance liveness), not silent controller-balance theft or share/cash desync on the happy path.

---

## 1. Entrypoint → orchestration

```265:284:contracts/controller/src/lib.rs
    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    ) {
        strategies::swap_debt::process_swap_debt(
            &env,
            &caller,
            SwapDebtParams { /* ... new_debt_amount: amount ... */ },
        );
    }
```

Pause-gated (`#[when_not_paused]`). Body (`process_swap_debt`):

| Step | Code | Money effect |
|---|---|---|
| Auth | `require_authorized_caller` + `require_owner_or_delegate` | No tokens yet |
| Distinct keys | `existing_debt != new_debt` | Blocks no-op / self-refinance on same `HubAssetKey` |
| Hub gate | `require_hub_active(existing_debt.hub_id)` only | Exit hub must be live at entry (see §7) |
| Positive amount | `require_positive_amount(new_debt_amount)` | Gross borrow request |
| Prefetch | prices for account + both assets | Risk inputs only |
| Borrow leg | `borrow_into_controller(..., charge_fee: true, SwDebtR)` | Pool → controller; debt entry |
| Swap / passthrough | `swap_tokens_or_passthrough(..., refund_to: caller)` | Controller `token_in` → router → `token_out`; leftover `token_in` → caller |
| Repay leg | `repay_debt_from_controller(..., debt_available: repay_amount)` | Controller → pool; debt exit; overpay → caller |
| Finalize | `strategy_finalize` | Restamp LTV + post-pool HF/solvency + persist both sides |

Both borrow and repay legs tag `PositionAction::SwDebtR` (events.md). Persistence is deferred until finalize (A032); pool legs commit inside the same tx and roll back together on later panic.

---

## 2. Borrow leg — `borrow_into_controller` (pool → controller)

```248:297:contracts/controller/src/positions/debt.rs
pub(crate) fn borrow_into_controller(..., charge_fee: bool, ...) -> i128 {
    // validate_position_entry_gates → require_can_borrow
    //   (new_debt hub active, listed, BlockOnEntry, is_borrowable, position limits)
    let before = token::Client::balance(controller);
    let result = storage::with_flash_guard(env, || {
        pool_create_strategy_call(..., charge_fee)
    });
    let measured = balance_delta_since(..., before);
    assert!(measured == result.amount_received);
    assert!(measured > 0);
    merge_debt_leg(... Entry ..., LegOutcome from actual_amount/gross ...);
    measured
}
```

### 2.1 Pool economics (`ops/strategy.rs`)

- Mints debt shares for **gross** `action.amount` (`new_debt_amount`).
- Computes `fee = flashloan_fee` bps when `charge_fee` (swap_debt always `true`).
- Debits pool cash / transfers **`amount - fee`** to the controller.
- Fee stays in the pool and is booked as protocol revenue (never sent out).

So:

| Quantity | Value | Lands where |
|---|---|---|
| Debt minted / `LegOutcome.amount` | `actual_amount` = gross | Account borrow shares + events |
| Controller SAC increase / swap input | `amount_received` = gross − fee | Controller balance |
| Protocol fee | `fee` | Pool revenue (cash never left) |

Equality `measured == amount_received` plus `measured > 0` closes FOT / short-delivery / dust-fee-zero-payout on the custody receive (dust-fee→0 panics here; pool unit test documents the zero-receive edge). Flash guard spans the pool transfer so a listed token hook cannot reenter position verbs before the strategy continues (A007).

### 2.2 New-debt allowlisting

`validate_position_entry_gates` → `require_can_borrow` → `cache.require_hub_active(new_debt.hub_id)` plus spoke listing / pause / freeze / `is_borrowable`. Asymmetry vs the explicit `require_hub_active(existing_debt)` at the top is intentional layering: **entry** for the new asset is fully gated inside the borrow helper; **existing** is gated early so a dead exit hub fails before opening new debt.

---

## 3. Swap / passthrough — router trust boundary

### 3.1 Distinct assets → `swap_tokens`

1. Snapshot controller `token_in` and `token_out` balances.
2. `authorize_transfer_as_current` exactly `amount_in` (net borrow proceeds) to the configured aggregator.
3. `with_flash_guard` → `execute_strategy` (router return value **discarded** — INV-STRAT-01).
4. Assert no balance increase on `token_in` (`RouterOverspend`); `actual_spent ≤ amount_in`.
5. `leftover = amount_in - actual_spent` transferred to `refund_to` (**caller**).
6. `verify_router_output`: `balance_delta_since(token_out) > 0` or `NoSwapOutput`.

Empty swap payload fails `InvalidPayments` before/without durable new debt (harness `test_swap_debt_empty_swap_payload_rolls_back_new_debt`).

### 3.2 Same asset address (cross-hub refinance) → passthrough

If `new_debt.asset == existing_debt.asset` but `HubAssetKey` differs, `swap_tokens_or_passthrough` requires an **empty** swap and returns `amount_in` unchanged. No router call. Covered by `multi_hub.rs::swap_debt_refinances_debt_across_hubs` (borrow hub-2 USDC, repay hub-1 USDC). Caller must size gross borrow to cover fee + target repay (test uses 305 raw vs 300 debt).

### 3.3 Economic meaning of leftover `token_in`

Unspent borrowed `token_in` is sent to **caller**, while the account still carries **gross** new debt. That is equivalent to “borrow + pocket residual + repay old debt with swapped proceeds,” bounded by post-op HF. Aligns with INV-STRAT-02 residue return and A003 delegate economic control (delegate as `caller` receives residue).

### 3.4 Slippage residual (known)

Only `received > 0` is enforced in-controller. `total_min_out` is inside the aggregator payload. Malicious/compromised router can return dust and keep value; HF gate is the remaining bound. Threat-model §“controller does not bound slippage” — **not a novel critical gap**; tracked as unbounded-loss under router trust.

---

## 4. Repay leg — `repay_debt_from_controller` (controller → pool → optional refund)

```49:89:contracts/controller/src/strategies/legs.rs
pub(crate) fn repay_debt_from_controller(...) {
    let received = transfer_amount_measured(
        env, &req.debt.asset, &controller, &debt_pool_addr,
        req.debt_available, InternalError,
    );
    let controller_balance_before_repay = debt_tok.balance(&controller);
    execute_repayment(... amount: received ...);
    refund_controller_balance_delta(env, &req.debt.asset, controller_balance_before_repay, caller);
}
```

### 4.1 Pre-funded pool repay (no double pull)

Pool `ops/repay.rs` documents: hub must have **already transferred** tokens into the pool; accounting credits `net_repay`, burns shares, `transfer_out(payer, overpayment)`. Controller pattern matches plain `process_repay` (measure transfer, then `pool_repay_call`). Counterparty / payer is the **controller** (`controller_event_context`), so overpay returns to controller custody first.

### 4.2 Measured receipt drives share burn

`transfer_amount_measured` snapshots **pool** balance delta. FOT / short delivery reduces `received`; `execute_repayment` / `make_pool_action` use that measured amount. Share burn cannot exceed tokens the pool actually saw (INV-ACCT-03).

`debt_available` comes from swap output (or passthrough net borrow). `verify_router_output` / borrow `measured > 0` ensure it is positive before transfer.

### 4.3 Overpay refund isolation

Snapshot is taken **after** the outbound transfer and **before** `execute_repayment`. Only a **post-repay increase** (pool overpay refund) is forwarded to `caller`. Pre-existing controller balances of the debt asset are untouched.

Pinned by `test_swap_debt_refund_only_uses_strategy_excess`: mint unrelated ETH onto the controller, overpay via swap, assert caller receives only strategy excess and controller retains the seeded balance. Layer commentary also in `outbound_transfer_measurement.rs::repay_overpayment_is_refunded_to_the_payer_not_stranded`.

### 4.4 Partial vs full close

If swap output &lt; outstanding existing debt → partial repay; residual old debt + full new debt remain; `strategy_finalize` HF gate decides success (`test_swap_debt_partial`, `test_swap_debt_health_factor_guard_after_swap`).

If output &gt; debt → full close + overpay to caller.

Frozen (`is_borrowable = false`) existing asset still exits: repay uses `FreezePolicy::AllowOnExit` (`test_swap_debt_closes_existing_debt_even_if_existing_asset_disabled`). `paused` still blocks user exits (AllowOnExit).

### 4.5 Stale `existing_pos` snapshot

`get_debt_position_or_panic` runs **before** the new borrow. Keys differ (`AssetsAreTheSame` otherwise), so the snapshot’s scaled amount still matches the in-memory existing leg at repay time. `merge_debt_leg` Exit re-reads current account scaled for usage baselines; pool action carries the snapshot position the owner-gated pool trusts from the controller.

---

## 5. End-to-end token conservation (single tx)

Happy path (distinct assets):

```
Pool[new]  --(gross-fee)-->  Controller[new]
Controller[new] --(spent)--> Router; leftover[new] --> caller
Router --(out)>0--> Controller[old]
Controller[old] --(measured)--> Pool[old]
Pool[old] --(overpay?)--> Controller[old] --(delta)--> caller
```

Account shares:

```
+ debt[new]  (gross, via create_strategy mint)
- debt[old]  (net repay excluding overpay)
```

Protocol:

```
+ revenue[new] (strategy fee remaining in pool cash)
```

Failure after borrow (bad route, HF, etc.) rolls back pool + controller SAC + storage together — no stranded mid-strategy debt (`empty_swap_payload_rolls_back_new_debt`).

Controller must not retain strategy dust on success: leftover `token_in` and repay overpay both leave; `token_out` is fully offered to repay (excess refunded). Unrelated balances preserved (§4.3).

---

## 6. Other `legs.rs` primitives (not on swap-debt path)

| Helper | Custody pattern | Used by |
|---|---|---|
| `withdraw_collateral_to_controller` | Flash-guarded withdraw; return `balance_delta_since` | swap_collateral / repay_with_collateral via `withdraw_and_swap_from_supply` |
| `execute_withdraw_all` | Per-asset withdraw to `destination` (not controller) | migrate / close paths |
| `net_settle_collateral_against_debt` | No token move; pool burns matched scaled supply+debt | repay_debt_with_collateral same-asset net |
| `refund_controller_balance_delta` | Shared excess forwarder | swap_debt repay; any caller |

Shared invariant: controller-custody legs measure deltas; they do not trust callee-reported amounts alone. Net-settle is share-only and out of swap-debt’s money path.

---

## 7. Residuals and non-findings

| Item | Severity | Status | Notes |
|---|---|---|---|
| Router min-out only in aggregator | medium (trust) | known / partial | Threat-model; HF floor only after dust out |
| Flash flag clear on leftover / overpay token transfers | low–medium | residual A007/A055 | SAC listing assumption; in-memory vs storage mid-strategy |
| `require_hub_active(existing)` blocks refinance of deactivated hub | low (liveness) | accepted | Plain `repay` does not require hub active; users can repay with own funds |
| `charge_fee: true` shrinks repay cash vs debt minted | info | by design | Caller must oversize gross borrow (multi-hub test pattern) |
| Refunds to `caller` (delegate) not owner | info | by design | A003 complete economic control |
| Both legs event tag `SwDebtR` | info | intentional | events.md |
| Same-asset passthrough cross-hub | info | defended | Empty swap required; no router |

**Non-findings checked and rejected as fund-safety bugs:**

- Double-pull on repay (pre-fund + accounting-only pool repay).
- Sweeping donated controller balances into caller refund.
- Crediting repay shares above measured pool receipt.
- Trusting router’s returned amount for `token_out`.
- Persisting new debt if swap payload empty / HF fails (tx revert).
- Using borrowed proceeds without recording debt (merge before return of `measured`).

---

## 8. Method

1. Read `COORDINATION.md` / `SEED.md`; confirmed `A047-*.md` absent.
2. Traced `lib.rs::swap_debt` → `process_swap_debt` → borrow / swap / `repay_debt_from_controller` / finalize.
3. Read pool `create_strategy` and `repay` for fee, cash debit, pre-fund, overpay refund.
4. Compared measurement sites to INV-ACCT-03 / INV-STRAT-01/02 and peers A041/A082/A032/A007/A072/A055.
5. Cross-checked harness: happy/partial HF, refund isolation, empty route rollback, disabled existing asset, cross-hub passthrough, adversarial router reentry.
6. Inventoried unused `legs.rs` helpers for pattern consistency only.

---

## 9. Opinion / review checklist

Keep these intact on any future swap-debt or legs edit:

1. `measured == amount_received` on strategy borrow into controller.
2. Router return discarded; output = controller balance delta; leftover ≤ authorized `amount_in`.
3. Repay uses `transfer_amount_measured` into pool **before** `pool_repay_call`.
4. Overpay refund snapshot **after** that transfer, **before** repay FFI; forward only positive delta.
5. `strategy_finalize` after both legs — never skip post-pool gates on a path that opened debt.
