# A046 — Multiply legs money flow (borrow → swap → deposit)

- Agent: A046
- Theme: T3 (custody / measured settlement), T4 (post-money risk gates)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/strategies/multiply.rs` (`process_multiply`, `collect_initial_multiply_payment`, `prepare_multiply_account`, `validate_multiply_request`)
  - `contracts/controller/src/strategies/legs.rs` (custody primitives — **not called by multiply**; pattern contrast only)
  - `contracts/controller/src/strategies/swap.rs` (`swap_tokens` / `swap_tokens_or_passthrough` / `verify_router_output`)
  - `contracts/controller/src/positions/debt.rs:248-298` (`borrow_into_controller`, `charge_fee = true`)
  - `contracts/controller/src/positions/supply.rs:106-155` (`process_deposit` / `settle_supply` measured controller→pool)
  - `contracts/controller/src/strategies/mod.rs:71-80` (`strategy_finalize`)
  - `contracts/controller/src/payments.rs` (`balance_delta_since`); `common/src/token.rs` (`transfer_amount_measured`)
  - `contracts/pool/src/ops/strategy.rs` (`create_strategy` / fee withhold)
  - `contracts/controller/src/lib.rs:230-258` (`multiply` entrypoint)
- Defense: Single atomic tx opens/extends leverage by (optional measured initial payment) → fee-charging strategy borrow into controller (pool report == balance delta) → router or same-asset passthrough (output = balance delta; unspent `token_in` ≤ authorized amount refunded to `caller`) → measured controller→pool supply of collateral → `strategy_finalize` solvency. Debt shares follow pool **gross** mint; swap/deposit cash follows **net** receipts. Pre-existing controller balances are not credited or swept.
- Gap: (1) Controller-side slippage is only `received > 0`; min-out lives in aggregator payload (INV-STRAT-01 / threat-model residual). (2) Leftover `token_in` and deposit transfers run with flash flag clear — listed-token hook residual (A007/A055). (3) Fee is debt-financed (`charge_fee: true`): less cash for collateral than debt minted. (4) `InitialMultiplyPaymentEvent` records requested amount, not measured receipt (observability only). (5) Payment routing matches on `asset` Address only (hub id ignored for fold-in) — integrator footgun, not theft. (6) `legs.rs` refund/repay helpers unused here; no end-of-flow controller excess sweep beyond swap leftover (success path clears strategy cash by construction).
- Impact: No path found that credits supply/debt without measured custody, double-counts initial payment into both swap and deposit, sweeps stranded controller balances into the caller, or leaves strategy proceeds as stealable free cash without matching account debt + solvency. Router compromise can economically worsen the caller’s own HF down to the post-gate floor (documented unbounded-loss class). Delegate leftover refunds go to `caller` — accepted under threat-model “delegate has complete economic control.”
- Evidence: INV-ACCT-03; INV-STRAT-01/02; ADR-0011; ADR-0020 (fee vs `flash_position`; router confinement vs `is_flashloanable`); threat-model multiply / router / delegate rows; endpoints.md `multiply`; harness `strategy/edge/multiply.rs`, `poc_multiply_reentrancy.rs`, `fuzz/strategy_multiply_budget.rs`; unit `contracts/controller/tests/strategies/mod.rs`; Certora `multiply_sanity`, `post_gate_multiply_observes_gate_witness`. Cross-ref A007, A018, A032, A041, A045, A047, A055, A072, A082, A003.
- Opinion: Multiply’s money-flow core is defended and is the fee-charging sibling of flash_position’s mint→collateral shape. Keep measured borrow equality, router discard-return + leftover ≤ `amount_in`, and deposit-from-controller measured push. Do not add a blind controller balance sweep; do not treat leftover-to-caller as a bug — it is INV-STRAT-02 residue return, gated by solvency.

## Scope

Audit of **token and share money movement** on `multiply`: optional initial payment → borrow debt into controller → swap (or passthrough) into collateral → deposit → finalize.

`legs.rs` is in the agent scope title because it holds the shared controller-custody vocabulary used by other strategies. **Multiply does not call any `legs.rs` function.** This finding inventories that absence and contrasts patterns (swap leftover vs `refund_controller_balance_delta`; no repay/withdraw/net-settle on the open path).

Out of scope for depth (peer agents): auth/pause/mode (A001/A003/A018), flash-guard lifecycle (A007/A030), finalize batching (A032), lying-token listing trust (A055), cache/spoke-usage (A076/A082/A094), TTL (A034).

## Verdict

**Defended.** Cash and share flows are measured at every controller custody boundary, directionally closed under Soroban tx atomicity, and finished behind ordinary post-pool risk gates. Residuals are policy/trust-boundary items (router min-out, listing hooks, fee asymmetry vs flash_position, event requested-vs-measured), not silent share/cash desync or third-party fund theft on the happy path.

## Method

1. Read `COORDINATION.md`, `SEED.md`, README format, INV-ACCT-03, INV-STRAT-01/02, ADR-0020, endpoints `multiply`, threat-model fee/delegate/router rows.
2. Traced `lib.rs::multiply` → `process_multiply` end-to-end: gates → initial payment → borrow → swap → deposit → finalize → event.
3. Decomposed `borrow_into_controller` (`charge_fee=true`) against pool `ops/strategy.rs`; swap measurement/leftover; `process_deposit` as controller payer.
4. Confirmed multiply does not invoke `legs.rs`; contrasted repay/withdraw/refund helpers used by swap_debt / flash_position / repay_with_collateral.
5. Cross-checked peer A007, A018, A032, A041, A045, A047, A055, A082; harness edge multiply + reentrancy POC; unit strategy tests. No novel critical gap beyond accepted residuals.

---

## 1. Entrypoint → orchestration

```230:258:contracts/controller/src/lib.rs
    #[when_not_paused]
    fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        collateral: HubAssetKey,
        debt_to_flash_loan: i128,
        debt: HubAssetKey,
        mode: PositionMode,
        swap: Bytes,
        initial_payment: Option<(HubAssetKey, i128)>,
        convert_swap: Option<Bytes>,
    ) -> u64 {
        strategies::multiply::process_multiply(/* MultiplyParams */);
    }
```

Body (`process_multiply`):

| Step | Code | Money effect |
|---|---|---|
| Auth | `require_authorized_caller` | No tokens yet |
| Mode / distinctness | `validate_multiply_request` | Multiply: full `HubAssetKey` inequality; Long/Short: underlying `asset` inequality; `debt_to_flash_loan > 0` |
| Account | `prepare_multiply_account` → `AccountGuard::Multiply` | Load/create; `require_can_supply(collateral)`; price prefetch |
| Initial payment | `collect_initial_multiply_payment` | Caller → controller (measured); optional convert swap → collateral |
| Borrow | `borrow_into_controller(..., charge_fee: true, Multiply)` | Pool → controller net of fee; debt entry (gross) |
| Compose swap in | `amount_received + debt_extra` | Debt-denominated payment folded into router input |
| Swap / passthrough | `swap_tokens_or_passthrough(..., refund_to: caller)` | Debt asset → collateral asset; leftover debt asset → caller |
| Sum collateral | `collateral_amount + swapped_collateral` | Payment-collateral + swap out |
| Deposit | `supply::process_deposit(controller, …)` | Controller → pool measured supply |
| Finalize | `strategy_finalize` | Restamp LTV + post-pool HF/solvency + persist both sides |
| Event | `emit_multiply_initial_payment` | Observability only (requested amount) |

Persistence of account maps is deferred until finalize (A032). Pool mint/supply commit inside the same tx and roll back together on later panic.

**Not on this path:** `repay_debt_from_controller`, `withdraw_collateral_to_controller`, `execute_withdraw_all`, `net_settle_collateral_against_debt`, `refund_controller_balance_delta` (`legs.rs`). Multiply is open/extend only; unwind strategies own those legs.

---

## 2. Optional initial payment (caller → controller)

```185:227:contracts/controller/src/strategies/multiply.rs
fn collect_initial_multiply_payment(...) -> (i128, i128) {
    // transfer_amount_measured(payment.asset, caller → controller, payment_amount)
    // collateral.asset match → (received, 0)
    // debt.asset match → (0, received)
    // else require convert_swap → swap_tokens → (collateral_out, 0)
}
```

### 2.1 Measurement

- Pull uses `transfer_amount_measured` → credited fold-in is **recipient delta**, not the requested `payment_amount` (INV-ACCT-03).
- `require_positive_amount` on the request before transfer.
- Absence → `(0, 0)`; no phantom credit.

### 2.2 Three denominations

| Payment asset | Fold into | Later use |
|---|---|---|
| `== collateral.asset` | `collateral_amount` | Added to deposit total (never enters debt swap) |
| `== debt.asset` | `debt_extra` | Added to `swap_amount_in` with borrow proceeds |
| Third asset | `collateral_amount` via `convert_swap` | Must supply `convert_swap` or `ConvertStepsRequired` |

When `collateral.asset == debt.asset` (allowed in `PositionMode::Multiply` with distinct hub keys), the **collateral branch wins first**. Same-underlying initial capital is never double-routed as both `collateral_amount` and `debt_extra`.

### 2.3 Convert swap

`swap_tokens(env, caller, payment.asset, received, collateral.asset, convert)` — same router defenses as the main leg (§4): discard return, measure out, leftover payment asset → `caller`. Runs **before** the strategy borrow, so convert residue cannot include borrowed funds.

### 2.4 Prefetch / listing

`prepare_multiply_account` prefetches prices for collateral, debt, and payment asset. Unlisted third-party payment fails at oracle prefetch **before** transfer (harness `test_multiply_rejects_unlisted_third_token_payment_before_transfer`).

### 2.5 Observability gap

`InitialMultiplyPaymentEvent.amount` is the **requested** `payment_amount`, not `received`. FoT under-delivery would understate event vs vaulted cash. Positions still use measured amounts. Severity: info / observability only.

---

## 3. Borrow leg — `borrow_into_controller` (pool → controller, fee on)

Multiply always passes `charge_fee: true` (contrast flash_position `false` — ADR-0020 / A045).

```248:297:contracts/controller/src/positions/debt.rs
// validate_position_entry_gates (Borrow)
// snapshot controller debt.asset balance
// with_flash_guard → pool_create_strategy_call(..., charge_fee)
// measured = balance_delta_since; assert measured == result.amount_received; measured > 0
// merge_debt_leg(Entry) from PoolPositionMutation::from(strategy mutation)
// return measured  // net cash, NOT gross principal
```

### 3.1 Pool economics (`ops/strategy.rs`)

| Quantity | Value | Lands where |
|---|---|---|
| Debt minted / `actual_amount` / `LegOutcome.amount` | gross `debt_to_flash_loan` | Account borrow shares + usage |
| Controller SAC increase / swap input | `amount_received` = gross − fee | Controller balance |
| Protocol fee | `flashloan_fee` bps | Stays in pool cash; booked as revenue |

Fee is debt-financed: user owes gross while only net proceeds can become collateral. Same shape as `swap_debt` (A047). Dust fee that drives `amount_received == 0` fails closed at controller (`AmountMustBePositive`); pool unit test documents the zero-receive edge.

### 3.2 Equality assert (do not remove)

`measured == result.amount_received` is defense-in-depth against FoT / short-delivery / lying pool report on the custody receive (A082/A041/A055). Flash guard spans the pool transfer so a listed token hook cannot reenter monetary verbs mid-payout (A007).

### 3.3 `is_flashloanable` intentionally omitted

Unlike `flash_position`, multiply does **not** require the debt market’s flash-loanable flag. ADR-0020 / threat-model: proceeds reach only the governance-owned aggregator, not a caller-chosen Wasm receiver. Confinement is the router allowlist + measured settlement, not the flash-loan market bit.

---

## 4. Swap / passthrough — router trust boundary

```85:96:contracts/controller/src/strategies/multiply.rs
let swap_amount_in = amount_received.checked_add(debt_extra)...;
let swapped_collateral = swap_tokens_or_passthrough(
    env, caller, &debt.asset, swap_amount_in, &collateral.asset, swap,
);
```

### 4.1 Distinct assets → `swap_tokens`

1. Snapshot controller `token_in` / `token_out`.
2. `authorize_transfer_as_current` exactly `amount_in` to `storage::get_swap_aggregator`.
3. `with_flash_guard` → `execute_strategy` (router return **discarded** — INV-STRAT-01).
4. Assert no `token_in` balance increase (`RouterOverspend`); `actual_spent ≤ amount_in`.
5. `leftover = amount_in - actual_spent` → transfer to `refund_to` (**caller**).
6. `verify_router_output`: `token_out` delta must be `> 0` (`NoSwapOutput`).

Pre-existing controller balances of `token_in` / `token_out` are **not** credited: spend cap is `amount_in` (assert vs baseline), output credit is delta only. Router cannot spend stranded pre-balance beyond `amount_in` without `RouterOverspend`.

### 4.2 Same asset → passthrough

When `debt.asset == collateral.asset` (Multiply cross-hub same underlying; Long/Short forbidden by validation):

- Requires empty `swap` (`InvalidPayments` otherwise).
- Returns `amount_in` unchanged — no router call, no leftover path.
- Net borrow (+ optional debt_extra) becomes the deposit amount on the **collateral** `HubAssetKey`.

### 4.3 Economic meaning of leftover `token_in`

Unspent authorized input (borrow net + debt_extra) refunds to `caller` while account debt remains **gross** minted. Combined with `strategy_finalize`, this is economically similar to borrowing toward self and posting only the swapped fraction as collateral — allowed iff HF/caps clear. INV-STRAT-02 explicitly requires residue return to the rightful caller.

Delegate as `caller` receives leftovers while debt sits on the owner’s account — accepted (threat-model: delegate has complete economic control; A003).

Controller-side slippage floor is only `received > 0`. Real min-out is inside the aggregator `StrategySwap` payload (caller + router trust).

### 4.4 Flash flag after router

Leftover `transfer` and later `process_deposit` run with the flag **clear**. Listed-token transfer-hook reentrancy against still-unpersisted in-memory strategy state is the shared residual tracked by A007 §5 / A055 — mitigated by listing trust + measurement, not by latching the flag for the whole entrypoint.

---

## 5. Deposit leg — controller → pool

```98:109:contracts/controller/src/strategies/multiply.rs
let total_collateral = collateral_amount.checked_add(swapped_collateral)...;
let deposit_assets = vec![env, (collateral.clone(), total_collateral)];
supply::process_deposit(
    env,
    &env.current_contract_address(),  // payer = controller
    &mut account,
    &deposit_assets,
    &mut cache,
);
```

### 5.1 Balance identity (success path)

Controller should hold of `collateral.asset`:

- `collateral_amount` (direct payment and/or convert-swap out), plus
- `swapped_collateral` (main swap out or passthrough of debt proceeds),

and no other multiply-credited source. Deposit requests exactly that sum.

`settle_supply` then `transfer_amount_measured(controller → pool, amount_in)` and builds pool supply entries from **measured pool receipt**, merging supply legs from pool mutations (A041). FoT on the push credits shares for what the pool actually got.

### 5.2 Entry gates

`process_deposit` still runs `validate_position_entry_gates` (Deposit) — caps, position limits, spoke flags — even though the outer caller already passed `require_can_supply`. Internal helper correctly omits a second owner auth; tokens leave controller custody the entrypoint already gated.

### 5.3 No `legs.rs` withdraw mirror

Open path never pulls supply back. Contaminated / unrelated controller collateral balances above `total_collateral` remain stranded (unstealable by later callers’ deltas — same baseline discipline as flash_position undeclared leftovers / endpoints.md). Multiply does not call `refund_controller_balance_delta`.

---

## 6. Finalize and events

`strategy_finalize`: restamp listed supply LTV → `require_post_pool_risk_gates` → `finalize_position_flow(..., Both, true)` (A032 / A072).

Debt event tag: `PositionAction::Multiply`. Supply from this deposit is ordinary supply merge tagging (batch event). Optional `InitialMultiplyPaymentEvent` after finalize (see §2.5).

Unlike INV-STRAT-04 on flash_position, multiply has **no** “still open” dual assert — a multiply that somehow cleared all debt in-call is not in this code path (no repay leg). Closing is a separate user flow.

---

## 7. `legs.rs` inventory (scope adjacency)

| Helper | Used by multiply? | Role elsewhere |
|---|---|---|
| `repay_debt_from_controller` | no | swap_debt, repay_with_collateral — measured controller→pool repay + overpay refund |
| `withdraw_collateral_to_controller` | no | swap_collateral / repay_with_collateral — flash-guarded withdraw, balance-delta return |
| `execute_withdraw_all` | no | close paths |
| `net_settle_collateral_against_debt` | no | same-asset net without token move |
| `refund_controller_balance_delta` | no | flash_position / repay overpay — positive Δ since snapshot → `refund_to` |

Multiply’s residue return is **`swap_tokens` leftover**, not `refund_controller_balance_delta`. Both honor “do not sweep pre-existing balance”: leftover is `amount_in - spent`; refund helper is `max(0, balance - before)`.

Shared measurement vocabulary still applies via `payments::` / `borrow_into_controller` / `process_deposit` — multiply is not a second, weaker custody dialect.

---

## 8. End-to-end money sequence

```
optional:
  P1. transfer_amount_measured(payment → controller) → received
  P2a. collateral denom → collateral_amount = received
  P2b. debt denom → debt_extra = received
  P2c. third → swap_tokens(convert) → collateral_amount; leftover payment → caller

borrow (flash guard):
  B1. pool create_strategy(charge_fee=true): mint debt gross G; send G-fee
  B2. assert controller Δ(debt.asset) == amount_received == G-fee > 0
  B3. merge_debt_leg(Entry) from actual_amount = G

swap:
  S1. amount_in = amount_received + debt_extra
  S2. if debt.asset == collateral.asset: passthrough amount_in (empty swap)
      else: authorize amount_in; router under flash guard; leftover → caller;
            swapped = Δ(collateral.asset) > 0

deposit:
  D1. total = collateral_amount + swapped
  D2. process_deposit(payer=controller): measured push to pool; merge supply

finalize:
  F1. restamp LTV; post-pool risk gates; persist Both + batch event
  F2. optional InitialMultiplyPaymentEvent(requested amount)
```

Atomicity: any panic after pool mutation reverts the whole tx — no durable half-open leverage with stranded strategy cash attributed to this call.

---

## 9. Contamination / theft checklist

| Hypothesis | Result |
|---|---|
| Credit supply from requested not measured payment | Denied — measured pull + measured deposit |
| Credit debt cash without pool mint | Denied — equality assert + merge from pool mutation |
| Double-count payment as collateral_amount and debt_extra | Denied — exclusive if/else on asset |
| Router return value inflates collateral | Denied — return discarded; balance delta only |
| Router spends stranded controller `token_in` | Denied — `actual_spent ≤ amount_in` |
| Leftover refund sweeps pre-existing controller balance | Denied — leftover = `amount_in - spent` only |
| Deposit credits pre-existing collateral on controller | Denied — deposit amount = sum of this call’s measured receipts |
| Fee bypass (gross cash out, no debt) | Denied — `charge_fee=true`; mint gross, send net |
| Free cash flash via multiply | Denied — debt remains; solvency required; no same-call repay |
| Delegate steals via leftover | Accepted policy — delegate economic control (A003) |
| Third party drains via reentry mid-deposit | Residual listing trust (A007/A055), not measurement hole |

---

## 10. Peer cross-links

| Peer | Relation |
|---|---|
| A045 | Sibling open path with `charge_fee=false` + Wasm receiver; multiply uses router + fee |
| A047 | Same borrow→swap measured shape; swap_debt continues to `legs.rs` repay |
| A041 / A082 | Measured receipt / pool-output-not-input pattern multiply instantiates |
| A032 / A072 | Finalize batch + post-pool gates after money movement |
| A007 / A030 | Flash windows on borrow transfer + router; clear on leftover/deposit |
| A018 / A003 / A004 | Mode guard, owner/delegate, account create on `account_id==0` |
| A055 | Non-SAC / FoT outer bound; equality asserts are the inner bound |

---

## 11. Residual tracking (non-blocking)

1. **Router min-out** — only `> 0` at controller; economic loss bounded by caller’s own HF.
2. **Post-guard token hooks** on leftover / deposit transfers (A007).
3. **Fee optional in practice** if users prefer `flash_position` on flashloanable markets (threat-model recorded asymmetry).
4. **Event requested vs measured** for initial payment.
5. **Payment `HubAssetKey.hub_id` ignored** for fold-in routing (asset Address only).
6. **No multiply-specific Certora money-flow rule beyond sanity / post-gate witness** — tracking for formal coverage, not a runtime hole.

**Status: defended.** Treat removal of `measured == amount_received`, leftover `≤ amount_in`, router return discard, or deposit measurement as **Critical** regressions against INV-ACCT-03 / INV-STRAT-01/02.
