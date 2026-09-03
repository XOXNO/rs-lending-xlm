# A043 — Pool borrow / repay measured amounts (`positions/debt.rs`)

- Agent: A043
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/positions/debt.rs` (full: `process_borrow`, `process_repay`, `settle_debt`, `apply_repay_batch`, `execute_repayment`, `borrow_into_controller`)
  - `contracts/controller/src/positions/mod.rs:52-67,148-188,358-368` (`LegOutcome`, `merge_debt_leg`, `make_pool_action`, `require_external_recipient`)
  - `contracts/controller/src/payments.rs` / `common/src/token.rs:19-34` (`transfer_amount_measured`, `balance_delta_since`)
  - `contracts/controller/src/strategies/legs.rs:49-89` (`repay_debt_from_controller`)
  - `contracts/controller/src/positions/liquidation/apply.rs:31-83` (shared `apply_repay_batch` consumer)
  - `contracts/controller/src/external/pool.rs:33-76` (`pool_borrow_call`, `pool_create_strategy_call`, `pool_repay_call`)
  - `contracts/pool/src/ops/borrow.rs`, `ops/repay.rs`, `ops/strategy.rs`, `cache/cash.rs`
  - `common/src/rates/scaling.rs:169-190` (`resolve_repay`); `common/src/types/pool.rs:396-418` (mutation fields)
- Defense: Three distinct money identities are kept consistent end-to-end. (1) **Inbound repay** always pushes tokens with `transfer_amount_measured` and feeds the pool `PoolAction.amount` from the **pool-balance Δ**, never the caller request — then pool `credit_cash(net_repay)` / share burn follow that measured figure (INV-ACCT-03 / ADR-0013). (2) **Ordinary outbound borrow** intentionally does **not** re-measure at the recipient; the pool mints ceil debt, debits cash, and `transfer_out`s the **same** `actual_amount` used for books (INV-ACCT-02 cash ≡ debit ≡ transfer intent under SAC). (3) **Strategy borrow into controller** double-checks the custody boundary: `balance_delta_since == result.amount_received` and `measured > 0`, while debt shares merge on **gross** `actual_amount` (fee withheld stays in pool as revenue). Overpay on repay is excluded from cash credit and refunded to `payer` (A054).
- Gap: (1) Accepted — user `borrow(..., to)` / pool→EOA payout is unmeasured at recipient (same class as withdraw; ADR-0013 “pool performs controlled outbound”). FoT under-delivers to `to` only; pool cash book and SAC sender debit stay matched. (2) Shared A055 / listing trust — rebasing or balance-lying tokens defeat Δ oracles. (3) Shared A007/A023/A025 — ordinary borrow/repay do not wrap the token/`pool_*` window in `with_flash_guard` (strategy `borrow_into_controller` does). (4) Hygiene — `transfer_amount_measured` may return `0`/`negative` without a primitive-level gate; zero receipt → pool repay no-op (debt unchanged); negative requires a hostile listed token (A058 §4.3). (5) `execute_repayment` does not re-measure — safe only because every in-tree caller already measured (`repay_debt_from_controller`).
- Impact: No path found where repay credits debt burn / cash above tokens the pool actually received, where strategy custody credits net cash without equality to the pool report, or where ordinary borrow mints debt without a matching cash debit and outbound transfer intent. Blast radius of outbound FoT on user borrow is **recipient shortfall** (account-local incentive loss), not share inflation. Market-wide desync requires governance-listed non-SAC behavior (A055), capped by that market’s TVL.
- Evidence: INV-ACCT-02/03/05, INV-ACCT-07/08 (borrow reserve/util gates), INV-LIQ-03 (liq repay adjacency), INV-STRAT-01/02, INV-FLASH-02 (strategy borrow guard), ADR-0003/0013; STRIDE Tamper.3 / TB7; threat-model token actor; Certora `controller_borrow_persists_pool_returned_position`, `usage_repay_tracks_scaled_delta`; harness `controller/repay.rs` (overpay), `outbound_transfer_measurement.rs`, strategy FoT suites (`multiply`/`flash_position` fail-closed on FoT debt); peers A023, A025, A041, A045–A050, A054, A055, A057, A058, A059, A082, A101 §8.2.
- Opinion: A043 closes the coverage hole called out in A101. Borrow/repay money integrity is **defended** under the custody split: measure every inbound credit; trust pool cash+transfer identity on outbound user payouts; equality-assert strategy mints into controller. Do not “fix” ordinary borrow by crediting recipient Δ into debt books, and do not remove `measured == amount_received` on `borrow_into_controller`.

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format; confirmed `A043-*.md` absent.
2. Traced every `debt.rs` money path: ordinary borrow/repay, `apply_repay_batch` / `execute_repayment`, `borrow_into_controller`, plus strategy/`liquidation` consumers of the shared repay primitive.
3. Pinned pool-side identities in `ops/borrow.rs`, `ops/repay.rs`, `ops/strategy.rs`, `cache/cash.rs`, and `resolve_repay` against controller merge inputs (`LegOutcome.amount = mutation.actual_amount`).
4. Cross-checked ADR-0013 outbound policy, INV-ACCT-02/03/05, peers A041/A054/A055/A057/A058/A082/A101 §8.2, harness FoT/overpay coverage.
5. Searched for novel critical gaps (credit-from-request, cash/debit skew, unmeasured strategy mint, repay without measure). None beyond accepted listing/outbound residuals.

Out of scope as primary claims (cross-linked): withdraw outbound (A042 hole), flash_loan pullback (A044), full strategy product flows (A045–A050), liq seize (A051–A053), destination auth (A057), rounding direction table (A059).

---

## 1. Verdict

**Defended.** `positions/debt.rs` implements the correct split of measurement duties:

| Flow | What is measured at controller | What books follow |
|---|---|---|
| User repay | Pool recipient Δ before `pool_repay_call` | Measured `amount_in` → pool net cash / share burn → `merge_debt_leg(Exit)` |
| Strategy repay | Same, payer = controller | Measured `received` → `execute_repayment` → optional Δ refund to caller |
| Liq repay (shared batch) | Same + USD floor-scale | INV-LIQ-03 (A051/A053) |
| User borrow | **Nothing at `to`** (by design) | Pool `amount` mint+debit+`transfer_out`; controller merges pool mutation |
| Strategy borrow | Controller Δ vs `amount_received` | Gross debt from `actual_amount`; net cash returned to caller |

A101’s inference (“accounting = pool mutation + measured inbound repay; outbound borrow mirrors withdraw”) is confirmed against source.

---

## 2. Call graphs

### 2.1 Ordinary borrow

```
Controller::borrow
  └─ process_borrow                          debt.rs:35-76
       ├─ require_authorized_caller
       ├─ get_account + require_owner_or_delegate
       ├─ require_external_recipient         reject pool/controller as `to`
       ├─ aggregate_positive_payments        requested > 0 per hub
       ├─ validate_position_entry_gates      BlockOnEntry + borrowable
       ├─ settle_debt(Borrow)                debt.rs:138-160
       │    ├─ make_pool_action(pos, amount) amount = REQUESTED
       │    ├─ pool_borrow_call(receiver=to)
       │    │    pool: mint_debt(amount) → debit_cash(amount)
       │    │         → transfer_out(to, actual_amount=amount)
       │    └─ for_each_leg → merge_debt_leg(Entry)
       │         LegOutcome.amount = result.actual_amount  (book, not recipient Δ)
       ├─ enforce_post_pool_solvency
       └─ finalize_position_flow(Debt|Both)
```

No controller balance snapshot. Recipient receipt is not an accounting input.

### 2.2 Ordinary repay

```
Controller::repay
  └─ process_repay                           debt.rs:81-112
       ├─ require_authorized_caller          permissionless third-party OK
       ├─ aggregate_positive_payments
       ├─ get_account_borrow_only
       └─ settle_debt(Repay { payer: caller }) debt.rs:161-183
            for each hub:
              ├─ AllowOnExit flags
              ├─ get_debt_position_or_panic
              ├─ amount_in = transfer_amount_measured(payer → pool, requested)
              └─ make_pool_action(pos, amount_in)   ← MEASURED, not requested
            apply_repay_batch → pool_repay_call(payer)
              pool: resolve_repay(amount) → burn / credit_cash(net) / refund overpay
            merge_debt_leg(Exit) from mutation.actual_amount (= net_repay)
```

### 2.3 Strategy borrow into controller

```
borrow_into_controller                       debt.rs:248-298
  ├─ require_positive_amount(amount)
  ├─ validate_position_entry_gates
  ├─ before = balance(controller, debt.asset)
  ├─ with_flash_guard {
  │     pool_create_strategy_call(controller, action, charge_fee)
  │       mint_debt(gross) ; fee → revenue ; debit/transfer amount_received
  │   }
  ├─ measured = balance_delta_since(controller, before)
  ├─ assert measured == result.amount_received
  ├─ assert measured > 0
  ├─ merge_debt_leg(Entry) from PoolPositionMutation::from(strategy)
  │     actual_amount = GROSS principal (debt book)
  └─ return measured                         NET cash for strategy legs
```

### 2.4 Strategy repay from controller

```
repay_debt_from_controller                   legs.rs:49-89
  ├─ received = transfer_amount_measured(controller → pool, debt_available)
  ├─ snapshot controller balance AFTER push
  ├─ execute_repayment(..., amount: received)  debt.rs:217-242
  │     → apply_repay_batch (no second measure)
  └─ refund_controller_balance_delta(snapshot → caller)
```

---

## 3. Identity matrix (pool amount vs cash vs transfer)

### 3.1 Ordinary borrow (`ops/borrow.rs`)

| Step | Value | Symbol |
|---|---|---|
| Request / entry | `entry.action.amount` | `A` |
| Scaled mint | `calculate_scaled_borrow(A)` ceil | debt ↑ |
| Cash debit | `debit_cash(A)` | cash ↓ by `A` |
| Mutation report | `actual_amount = A` | |
| Token out | `transfer_out(receiver, A)` | SAC sender −`A` |

**Identity:** `mint_basis = cash_debit = transfer_intent = actual_amount = A`.

Controller merge stores pool `new_scaled` and records event amount `A`. It does **not** introduce a second figure from recipient balance. Under SEP-41/SAC, pool token balance falls by `A` with the debit — INV-ACCT-02 cash book stays aligned with the pool’s own SAC movement. If a listed FoT token under-delivers to `receiver`, the shortfall is outside the pool (fee sink / recipient), not silent share inflation.

### 3.2 Strategy borrow (`ops/strategy.rs`)

| Step | Value | Symbol |
|---|---|---|
| Request | `action.amount` | `G` (gross) |
| Fee | `flash_loan_fee_on(G)` if `charge_fee` else 0 | `F` |
| Scaled mint | ceil on `G` | debt ↑ by gross |
| Cash debit / transfer | `G − F` | `N = amount_received` |
| Mutation | `actual_amount=G`, `amount_received=N` | |

Controller enforces `Δ_controller == N` and `N > 0`. Debt books merge on `G`. Subsequent strategy cash uses `N`. Fee `F` remains as protocol revenue backed by retained cash (never debited). FoT that breaks `Δ == N` **fails closed** (`InternalError`) — harness `test_multiply_fee_on_transfer_debt_fails_closed`, `test_flash_position_fee_on_transfer_debt_fails_closed`.

### 3.3 Repay (`ops/repay.rs` + measured push)

| Step | Value | Symbol |
|---|---|---|
| Caller request | aggregated payment | `R` |
| Measured push | `transfer_amount_measured` → pool Δ | `M` (`≤ R` on FoT) |
| Pool action amount | `M` | |
| `resolve_repay(M)` | burned shares; overpay `O` | full if `M ≥ ceil(debt)` |
| Net cash credit | `M − O` | `net_repay` |
| Mutation `actual_amount` | `net_repay` | |
| Refund | `transfer_out(payer, O)` | uncredited residue |

**Identity:** `credit_cash = net_repay = M − O`; shares burn from `resolve_repay(M)` (floor partial / full position); controller Exit merge uses `net_repay`, not `R` or `M` alone when overpay exists.

Pre-condition (pool comment): hub **already transferred** tokens; pool never pulls. Donations above `M` do not rewrite cash (INV-ACCT-02).

### 3.4 Zero / dust measured repay

If FoT delivers `M = 0` after a positive request:

- `make_pool_action(..., 0)` still submitted.
- `resolve_repay(0)` → burn 0, overpay 0, `net_repay == 0` passes `RepayRoundsToZeroShares` guard (`net_repay == 0 || burned > 0`).
- Debt unchanged; payer absorbed the token tax; protocol did not erase liability against missing cash.

If `0 < M` but floor shares round to 0 with positive net → `RepayRoundsToZeroShares` (INV-ACCT-05).

---

## 4. Controller merge semantics

```52:67:contracts/controller/src/positions/mod.rs
pub(crate) struct LegOutcome {
    pub new_scaled: Ray,
    pub market_index: MarketIndexRaw,
    pub amount: i128,
}
// amount ← mutation.actual_amount
```

| Path | `LegOutcome.amount` meaning | Usage / events |
|---|---|---|
| Borrow (user or strategy) | Gross borrowed principal applied | Debt ↑; spoke entry uses **scaled** Δ, not this token amount |
| Repay | Net repay credited to cash | Debt ↓; spoke exit uses scaled Δ |

Spoke usage always derives from `new_scaled − old_scaled` (A076/A082) — never from the caller’s requested `R`. That closes “usage tracks request while cash tracks measure” desync for repay.

`for_each_leg` length-asserts entries vs results (`InternalError`) so a truncated pool return cannot silently skip a merge.

---

## 5. Path-by-path defenses and residuals

### 5.1 `settle_debt` Borrow — unmeasured outbound (accepted)

Mirrors A041’s withdraw note and ADR-0013: outbound is the pool’s controlled transfer. Defenses that still apply before cash moves:

- `require_external_recipient` — blocks `to ∈ {controller, pool}` (stranded funds / measurement poison; A057 / GH-17).
- Entry gates + post-pool solvency + utilization/liquidation buffer on pool mint.
- Books follow pool mutation outputs only (Certora `controller_borrow_persists_pool_returned_position`).

**Not a bug:** omitting recipient Δ from debt accounting. Crediting debt from recipient receipt would under-record liabilities when FoT short-pays `to` while cash already debited `A` — worse for suppliers.

### 5.2 `settle_debt` Repay — measured inbound (load-bearing)

```172:181:contracts/controller/src/positions/debt.rs
let amount_in = payments::transfer_amount_measured(
    env, &hub_asset.asset, payer, &pool_addr, amount,
    GenericError::AmountMustBePositive,
);
actions.push_back(make_pool_action(&position, amount_in, hub_asset.clone()));
```

Regression that passed `amount` (requested) into `make_pool_action` would be **Critical** against INV-ACCT-03 (share burn / cash credit above receipt). Current code feeds `amount_in`. Batching measures all legs before a single `pool_repay_call` — atomic with the tx.

Third-party repay: payer = `caller`; overpay refunds to payer (A054; harness third-party overpay).

### 5.3 `apply_repay_batch` / `execute_repayment`

Shared primitive for user repay, strategy repay, and liquidation repay. Does **not** transfer tokens. Contract: caller supplies `PoolAction.amount` already equal to tokens sitting in the pool for that credit.

In-tree callers:

| Caller | Measures? |
|---|---|
| `settle_debt(Repay)` | Yes, per leg |
| `repay_debt_from_controller` | Yes, then `execute_repayment` |
| `apply_liquidation_repayments` | Yes, plus USD floor-scale |

No unmeasured production caller found. A future direct `execute_repayment` with request-sized `amount` and no prior push would over-credit cash relative to tokens — treat as Critical in review checklists.

### 5.4 `borrow_into_controller` — equality assert (do not remove)

```278:284:contracts/controller/src/positions/debt.rs
let measured = payments::balance_delta_since(env, &hub_debt.asset, &controller, before);
assert_with_error!(env, measured == result.amount_received, GenericError::InternalError);
assert_with_error!(env, measured > 0, GenericError::AmountMustBePositive);
```

Closes: trusting pool report alone while a hook drains controller mid-transfer; FoT under-delivery into controller custody; zero-net mint used as strategy budget. Flash guard held across the pool transfer (INV-FLASH-02 / A007). A082/A101 list this assert on the regression watchlist.

Gross vs net split is intentional: debt liability tracks financed principal; strategies spend only delivered cash (multiply fee-on-debt, flash_position fee-free).

---

## 6. Failure-mode table

| Scenario | Borrow (user) | Borrow (strategy) | Repay (user/strategy/liq) |
|---|---|---|---|
| FoT short on outbound | Recipient short; books use `A` | `Δ != N` → revert | N/A (inbound) |
| FoT short on inbound | N/A | N/A | Books use `M`; liq seizes scale down |
| FoT delivers 0 inbound | N/A | N/A | Pool no-op; debt unchanged |
| Extra donation into pool | Ignored by cash book | Ignored | Ignored beyond `M` |
| Overpay repay | N/A | N/A | `O` uncredited; refunded to payer |
| `to` = controller/pool | Rejected pre-call | Intentional controller `to` | N/A |
| Requested amount in pool action | Used (outbound identity) | Gross `G` for mint | **Forbidden** — must be `M` |
| Lying `balance()` token | Listing residual A055 | Equality may false-pass/fail | Δ oracle compromised A055 |
| Reentrancy mid-transfer | A007 residual (no flash flag) | Flash flag held | A007 residual (ordinary) |

---

## 7. Consistency with peers / A101 hole fill

| Peer | Relation |
|---|---|
| A041 | Deposit measure pattern; outbound user borrow/withdraw unmeasured — **same policy**, confirmed here for borrow |
| A023 / A025 | Storage finalize; note unmeasured vs measured money legs without contradicting |
| A045–A050 | All strategy opens use `borrow_into_controller` equality; closes use measured repay |
| A054 | Overpay refund layer on top of measured `M` |
| A055 | Outer listing residual for non-SAC |
| A057 | `to` hijack defended; outbound measure not required for that class |
| A058 | Owns primitives; this file owns debt.rs composition |
| A059 | Ceil mint / floor repay pairing supports these cash identities |
| A082 | Usage from pool scaled outputs; strategy equality assert |
| A101 §8.2 | Coverage hole — **filled**; no contradiction with adjacency inference |

No disagreement file needed.

---

## 8. Tests / formal anchors

| Anchor | What it pins |
|---|---|
| `tests/test-harness/tests/controller/repay.rs` (`test_repay_overpayment_refunded`, third-party) | Overpay → payer; net debt clear |
| `outbound_transfer_measurement.rs::repay_overpayment_is_refunded_to_the_payer_not_stranded` | Dual-layer cash vs refund commentary |
| Strategy FoT debt fail-closed (`strategy/adversarial.rs`, `flash_position_adversarial.rs`) | `measured == amount_received` |
| Certora `controller_borrow_persists_pool_returned_position` | Controller debt follows pool mutation |
| Certora `usage_repay_tracks_scaled_delta` | Repay usage from scaled Δ |
| Unit FoT liquidation events (`contracts/controller/tests/events.rs`) | Measured receipt vocabulary |

Optional hardening (non-blocking): assert `amount_in >= 0` explicitly after measure on repay (reject hostile negative Δ earlier than pool math); document in runbooks that user borrow outbound FoT is recipient-tax, not a controller bug.

---

## 9. Remediation

**None required on production Rust for A043 scope.**

Preserve:

1. Repay: `PoolAction.amount = transfer_amount_measured(...)` only.
2. Strategy borrow: `measured == amount_received && measured > 0`.
3. User borrow: books from pool mutation; do not invent recipient-Δ debt credits.
4. `execute_repayment` callers must remain measure-first.

Treat any PR that credits repay/borrow books from raw request amounts, or drops the strategy equality assert as redundant, as **Critical** against INV-ACCT-03 / ADR-0013.
