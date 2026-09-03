# A042 — Pool withdraw measured-transfer pattern

- Agent: A042
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/positions/supply.rs:161-417` (`process_withdraw`, `settle_withdraw`, `execute_withdrawal`, `apply_withdraw_batch`, `merge_withdraw_leg`)
  - `contracts/controller/src/external/pool.rs:54-65` (`pool_withdraw_call`)
  - `contracts/controller/src/strategies/legs.rs:94-148` (`withdraw_collateral_to_controller`, `execute_withdraw_all`)
  - `contracts/controller/src/strategies/mod.rs:85-111` (`withdraw_and_swap_from_supply`)
  - `contracts/controller/src/positions/liquidation/apply.rs:88-121` (`apply_liquidation_seizures`)
  - `contracts/controller/src/positions/mod.rs:52-67,112-141` (`LegOutcome`, `apply_leg_usage`)
  - `contracts/controller/src/payments.rs:14-24` (`balance_delta_since`)
  - `contracts/pool/src/ops/withdraw.rs` (gross vs `net_transfer`, `transfer_out`)
  - `contracts/pool/src/cache/cash.rs:39-48` (`transfer_out`)
  - `common/src/rates/scaling.rs:103-119` (`resolve_withdrawal`)
- Defense: Withdraw never mints protocol credit from a caller-requested transfer amount. Controller share/usage books follow pool mutation outputs (`new_scaled`, indexes, gross `actual_amount`). Where the controller custodies the payout (strategy withdraw-to-controller), cash for the next leg is `balance_delta_since`, not the request and not the pool return alone. Ordinary user / Transfer-liquidation / close-all paths are intentional pool→external outbound under ADR-0013 — pool cash debit + `require_reserves` + owner-gated FFI, no recipient Δ.
- Gap: (1) Accepted — user/liquidation/close-all outbound is unmeasured at the recipient; FoT haircuts the receiver while pool cash already debited full `net_transfer` (listing residual / A055). (2) Accepted — strategy measures controller Δ but does **not** equality-assert `measured == pool.actual_amount` (unlike `borrow_into_controller`); FoT mid-strategy shrinks subsequent swap/deposit capital after shares already burned for gross. (3) Observational — events and `process_withdraw`’s returned `paid` use pool **gross** `actual_amount`; on liquidation that is not tokens the liquidator received (fee withheld). (4) Pool `load_leg` trusts controller-supplied `PoolAction.position` scaled (owner-only); not an external measurement hole.
- Impact: No path found where requested withdraw amount alone credits shares, usage, or inbound cash. Blast radius of outbound FoT / lying balance is the affected market’s TVL under a compromised listing (A055), not a missing controller custody measure on the credit side. Strategy FoT loss is account-local (fewer redeposited units after a full gross burn).
- Evidence: INV-ACCT-02/03/05; ADR-0013; pool README withdraw gross vs net note; threat-model “Non-standard tokens” + outbound recipient auth; harness `tests/test-harness/tests/controller/withdraw.rs` (`test_withdraw_returns_actual_amounts_on_full_close`); peers A041, A051, A057, A058, A082, A024, A101 §8.1.
- Opinion: Pattern is correct and asymmetric with deposit on purpose. Keep “measure only at controller custody boundaries” — do not add recipient Δ on pool→user withdraw (would not strengthen share books and would poison baselines if `to` were controller). Optional hardening only: strategy equality `measured ≤ gross` / `measured > 0` after `withdraw_collateral_to_controller` (fail-closed FoT), document that `actual_amount` must never be read as liquidator receipt.

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README format; confirmed `A042-*.md` absent (A101 listed it as a coverage hole).
2. Traced every controller caller of `pool_withdraw_call` / `apply_withdraw_batch` / `execute_withdrawal` / `merge_withdraw_leg`.
3. Compared deposit measured-receipt (`settle_supply`) vs withdraw outbound; read pool `ops/withdraw.rs` gross/`net_transfer` split and `transfer_out`.
4. Classified each pool return field as trusted-for-books vs observational vs must-not-mean-receipt.
5. Cross-checked A041/A051/A057/A058/A082 and A101 §8.1 inference — no disagreement filed.

---

## 1. Verdict in one sentence

**Share books trust pool mutation outputs; cash for further controller credit trusts measured controller Δ; pool→external payouts trust pool cash accounting + SAC listing — never the caller’s requested amount as a credit oracle.**

---

## 2. Call graph (money + measure)

```
Controller::withdraw
  └─ process_withdraw
       ├─ require_external_recipient(to|caller)     # reject pool/controller
       ├─ aggregate_payments(..., MeansAll)         # size only; 0 → sticky all
       └─ settle_withdraw
            ├─ per leg: flags + make_pool_action(pos, requested|MAX, hub)
            └─ apply_withdraw_batch(Normal)
                 ├─ pool_withdraw_call(receiver, false, entries)   # FFI only
                 │    └─ pool.withdraw → accounting → transfer_out(net)
                 └─ merge_withdraw_leg ← LegOutcome{new_scaled, idx, actual_amount=gross}
            └─ paid[] = result.actual_amount        # return/API; not a credit mint

Strategy (swap_collateral / repay_with_collateral / …)
  └─ withdraw_collateral_to_controller
       ├─ balance_before = token.balance(controller)
       ├─ with_flash_guard { execute_withdrawal(..., counterparty=controller) }
       │    └─ apply_withdraw_batch → merge (same as above)
       └─ return balance_delta_since(controller, balance_before)  # MEASURED
            └─ swap / passthrough / later transfer_amount_measured deposit|repay

Liquidation SeizeMode::Transfer
  └─ apply_liquidation_seizures
       └─ apply_withdraw_batch(Liquidation, protocol_fee>0 possible)
            └─ pool: gross burn; net = gross−fee → liquidator; fee→revenue

Close / migrate helpers
  └─ execute_withdraw_all → execute_withdrawal(..., destination)  # unmeasured outbound
```

`external/pool.rs::pool_withdraw_call` is a pure client wrapper: no snapshot, no Δ, no field rewrite.

---

## 3. What is measured vs trusted

### 3.1 Field matrix (pool `PoolPositionMutation` → controller)

| Field | Produced by pool | Controller use on withdraw | Measured at controller? |
|---|---|---|---|
| `position.scaled_amount` → `LegOutcome.new_scaled` | Remaining after burn | Overwrites account supply scaled; usage exit Δ = `old − new` | **Trusted output** (A082) |
| `market_index` | Post-accrual commit | Cache + events; usage entry paths elsewhere | **Trusted output** |
| `actual_amount` | **Gross** asset units resolved (`resolve_withdrawal` / full floor) | Events (`outcome.amount`); `process_withdraw` return `paid`; **not** cash-in for next credit | **Trusted for books/events**; **≠ recipient receipt** when `protocol_fee > 0` |
| `asset_decimals` | Params | Unused on withdraw merge (exit usage needs no decimals) | Trusted |
| `net_transfer` | Internal only (not in mutation) | Never seen by controller | N/A — pool pays this |
| Caller `PoolAction.amount` | Request / `i128::MAX` all-sentinel | Sizes resolve only | **Not** a credit oracle |
| Controller custody Δ | — | Strategy next-leg input only | **Measured** (`balance_delta_since`) |

Contrast deposit (`settle_supply`): inbound hop is `transfer_amount_measured` → `PoolSupplyEntry.action.amount = received` before `pool_supply_call`. Withdraw has no symmetric pre-call measure because tokens leave the pool, not the controller.

### 3.2 ADR-0013 split (load-bearing)

ADR-0013: inbound credit uses measured receipt; **“The pool performs controlled outbound transfers.”** Requested amounts are never sufficient evidence of **received** value. Withdraw is the outbound half of that split:

- Inbound verbs (supply/repay/recap/strategy deposit): measure → credit.
- Outbound verbs (user withdraw/borrow, Transfer seize, claim forward raw hop): pool (or controller) pushes; recipient Δ is not protocol credit.

INV-ACCT-03 lists measured primitives on supply/keepers/liquidation **repay**/strategy legs — not on user withdraw payout.

---

## 4. Path-by-path

### 4.1 User `process_withdraw` — unmeasured outbound (defended)

1. Auth: owner/delegate; `require_external_recipient` (A057).
2. Amount fold: non-negative; `ZeroLeg::MeansAll` sticky zero → `WITHDRAW_ALL_SENTINEL` (`i128::MAX`).
3. Pool resolves burn/pay via `resolve_withdrawal` (partial: ceil shares + pay `amount`; full: burn all + pay floor).
4. Cash: `require_reserves(net)` → utilization/solvency gates → `debit_cash(net)` → `transfer_out(receiver, net)` with `protocol_fee == 0` ⇒ `net == gross`.
5. Controller merge: `scaled = new_scaled`; usage exit from scaled Δ; event amount = **gross** `actual_amount`.
6. Post-pool solvency then finalize (A024/A072).

**Does requested amount mint credit?** No. Shares only decrease to pool `new_scaled`. If the user receives fewer tokens than `actual_amount` (hostile FoT), the account still lost the full share burn — user loss, books match pool cash debit.

Harness pin: `test_withdraw_returns_actual_amounts_on_full_close` asserts returned `paid` equals wallet Δ under SAC.

### 4.2 Strategy `withdraw_collateral_to_controller` — measured custody (defended)

```
balance_before → flash-guarded execute_withdrawal(to=controller) → Δ
```

- Merge still trusts pool `new_scaled` / gross `actual_amount` for share + event books (same `merge_withdraw_leg`).
- **Subsequent** economic use (`withdraw_and_swap_from_supply` → swap/passthrough → measured deposit/repay) sizes off **Δ**, not `req.amount` and not (unchecked) pool return.
- Flash guard covers the pool withdraw + token transfer window (INV-FLASH / A007).
- No equality assert vs `actual_amount` (gap #2). Under SAC, Δ equals net (= gross). Under FoT, Δ < gross after shares already burned — account-local haircut on the strategy mid-flight; fail-closed only when a later leg requires `amount > 0` and Δ is 0.

### 4.3 `execute_withdraw_all` — unmeasured outbound

Pool→`destination` per supply key; merge from pool outputs; no controller Δ. Same class as user withdraw (close/migrate helper).

### 4.4 Liquidation Transfer seize — unmeasured outbound + gross semantics

`apply_liquidation_seizures` builds entries with planned `amount` + `protocol_fee`, `WithdrawKind::Liquidation`.

Pool:

- `actual_amount` mutation field = **gross**.
- `net_transfer` = gross − fee (fee minted as revenue shares; cash retained).
- Liquidator receives **net** via `transfer_out`.

Controller merge still sets shares from `new_scaled` and events from **gross**. Correct (A051): do not treat `actual_amount` as liquidator wallet credit. Seizure USD already floor-scaled to **measured repay receipt** before withdraw entries are built (INV-LIQ-03) — that measurement is on the debt hop, not on the seize payout.

### 4.5 `net_settle` (sibling, no withdraw transfer)

`net_settle_collateral_against_debt` also calls `merge_withdraw_leg` but moves **no** tokens; amounts are `settled_amount`. Out of money-transfer scope; share merge still pool-output-driven.

---

## 5. Pool-side amount identity (what “actual” means)

From `contracts/pool/src/ops/withdraw.rs`:

1. `(burned, gross_amount) = resolve_withdrawal(request, position)`.
2. `net_transfer = withhold_liquidation_fee(..., gross, fee)` (no-op if not liquidation / fee 0).
3. Burn shares; gate; `debit_cash(net_transfer)`.
4. `mutation = position_mutation(remaining, gross_amount)` — **gross** in `actual_amount`.
5. `transfer_out(receiver, net_transfer)` — raw SAC transfer; **no** recipient balance measure; no-op if `amount <= 0`.

Pool README: *“Do not read `actual_amount` as tokens received.”*

`transfer_out` does not adjust cash (already debited). FoT on the push: cash book −net, recipient +less — INV-ACCT-02 cash book remains the reserve truth; recipient under-delivery is listing-trust.

`ops::load_leg` uses controller-provided `action.position.scaled_amount` (owner-only pool). Controllers that pass stale/wrong scaled positions would desync market aggregates vs accounts — mitigated by single owner + controller being the sole writer of those positions, not by withdraw measurement.

---

## 6. Failure modes

| Scenario | User withdraw | Strategy → controller | Liq Transfer |
|---|---|---|---|
| SAC exact transfer | paid == wallet Δ | Δ == gross | liquidator += net; gross in events |
| FoT outbound | User short; shares/cash full burn/debit | Δ short; later legs use Δ; shares already −gross | Liquidator short; fee/cash as designed |
| Request > position (display) | Full close path; pay floor | Same via resolve | Plan amounts clamped earlier |
| Request 0 (MeansAll) | Sentinel MAX → full | N/A (explicit amount) | N/A |
| `to` = pool/controller | Rejected pre-call | Intentional controller | Liquidator is auth’d party |
| Pool reports false `new_scaled` | Requires compromised pool WASM/owner | Same | Same |
| Credit from request without pool out | **Does not occur** | **Does not occur** | **Does not occur** |

Zero-share rejection: pool panics `WithdrawRoundsToZeroShares` if gross > 0 but burned == 0 (INV-ACCT-05).

---

## 7. Consistency with peers

| Peer | Relationship |
|---|---|
| A041 | Deposit measured; explicitly left outbound withdraw unmeasured — this file owns that outbound half |
| A058 | `balance_delta_since` / legs composition; agrees strategy withdraw measures controller Δ |
| A082 | Usage/shares from pool `new_scaled` Δ — confirmed on `merge_withdraw_leg` |
| A051 | Liquidation gross vs net; unmeasured payout class |
| A057 | `to` auth + stranding; not a measure bug |
| A024 | Storage follows pool outputs; money measure deferred to A042 — closed here |
| A055 | Lying/FoT listing residual remains the outer bound |
| A101 §8.1 | Inference matched: no novel withdraw credit-from-request hole |

No `disagreements/` file needed.

---

## 8. Regression criteria (treat as Critical if introduced)

1. Crediting supply/usage/events from **caller request** instead of pool `new_scaled` / resolved gross.
2. Strategy next-leg sizing from **request** or unchecked pool return while ignoring controller Δ after custody withdraw.
3. Interpreting liquidation `actual_amount` as liquidator receipt for further credit or fee math.
4. Removing `require_external_recipient` on user withdraw (measurement poison / stranding).
5. Adding a gross `balance(controller)` sweep “to recover” withdraw dust (breaks INV-STRAT delta-only refunds).

Optional non-blocking hardenings: assert `measured > 0` (or `≥ 0` with explicit policy) after strategy withdraw; document API that `withdraw`’s returned `HubPayment` amounts are pool gross under SAC equality with wallet Δ.
