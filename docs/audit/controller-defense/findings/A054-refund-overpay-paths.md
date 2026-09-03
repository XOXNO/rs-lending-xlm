# A054 — Refund paths after overpay / excess recap

- Agent: A054
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - Debt repay overpay: `contracts/controller/src/positions/debt.rs` (`settle_debt` Repay / `apply_repay_batch` / `execute_repayment`); `contracts/pool/src/ops/repay.rs`; `common/src/rates/scaling.rs` (`resolve_repay`); `contracts/pool/src/cache/cash.rs` (`transfer_out`)
  - Excess recap: `contracts/controller/src/keepers.rs:43-66`; `contracts/pool/src/ops/recapitalize.rs`; `contracts/controller/src/external/pool.rs:136-144`
  - Strategy leftovers / overpay forward: `contracts/controller/src/strategies/legs.rs` (`repay_debt_from_controller`, `refund_controller_balance_delta`); `strategies/swap.rs` (router underspend leftover); `strategies/{swap_debt,repay_debt_with_collateral,multiply,migrate_blend,flash_position,mod}.rs`
- Defense: Three refund families share one money rule — **only uncredited residue moves outbound; position/cash books never absorb overpay**. (1) Ordinary / liquidation repay: measured pull into the pool, then pool burns ≤ ceil debt, credits **net** cash, `transfer_out(payer, overpayment)`. (2) Recapitalize: measured pull, pool applies `min(received, shortfall)`, refunds the rest to `payer`. (3) Strategy custody: pool overpay refunds to the controller as `payer`, then `refund_controller_balance_delta` forwards only the post-repay **measured** balance increase to `caller`; router leftovers use `amount_in − actual_spent` with overspend asserts; flash_position refunds only pre-listed positive deltas vs a pre-callback baseline.
- Gap: (1) Accepted — pool/strategy outbound refunds use raw `transfer` / `transfer_out`, not recipient-measured delivery; FOT haircuts the refund recipient, never inflates protocol credit (pinned by harness). (2) Shared A007/A045 — leftover/refund transfers after the flash-guard window clear; mitigated by listing/router trust. (3) Accepted — `flash_position` undeclared leftovers stay stranded and unstealable (baseline discipline). (4) Adjacent, not a money hole — liquidation “refunds” are plan trims reported by the estimate view; no refund transfer on `liquidate` (formulas.md).
- Impact: Successful overpay / excess-recap / strategy-leftover paths return residue to the rightful payer/caller and leave controller inventory unchanged except for intentional flash stranding. Failure modes revert the whole tx. No path found that converts overpay into minted debt, inflated cash, or stealable controller dust.
- Evidence: INV-ACCT-02/03/04/05, INV-STRAT-01/02/04; ADR-0003 repay ceil/floor; harness `test_repay_overpayment_refunded`, `repay_overpayment_is_refunded_to_the_payer_not_stranded`, `recapitalize_refund_is_unmeasured_but_strands_nothing`, `test_swap_debt_refund_only_uses_strategy_excess`, router underspend refunds, migrate leftover fuzz; pool `test_repay_overpayment_*`, `test_recapitalize_caps_to_shortfall_and_refunds_every_excess_unit`; peers A016, A025, A041, A045, A007, A055.
- Opinion: Refund surfaces are coherently designed. Keep the dual-layer story explicit: pool books **exclude** overpay from cash; controller custody refunds are **delta-only** so pre-existing dust cannot be swept. Do not “measure” pool→payer refunds into credit — that would invent a second source of truth. Optional hygiene only: document FOT double-haircut on recap/repay refunds in ops notes (already tested).

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git ops).
2. Inventory every controller/pool path that can return tokens after an overpay, excess recap, or strategy leftover.
3. Trace payer/recipient identity, measurement vs raw transfer, cash-book treatment, and interaction with pre-existing controller balances.
4. Cross-check peers A016 (recap measure), A025 (repay storage), A041 (measured deposit), A045 (flash refunds), A007 (post-guard leftovers), A055 (lying tokens); invariants INV-ACCT-02..05, INV-STRAT-01/02/04.
5. Out of primary claim: liquidation seize outflows (A051/A052), flash-loan pullback (A044), migrate Blend pull details beyond leftover repay (A050 when filed). Liquidation estimate `refunds` noted only to prevent false “missing transfer” findings.

---

## Refund family map

| Family | Trigger | Who holds residue mid-call | Refund recipient | Cash / shares effect of residue |
|---|---|---|---|---|
| Ordinary `repay` overpay | `amount_in > ceil(debt)` | Pool token balance (uncredited) | `payer` = caller | None — `credit_cash(net_repay)` only |
| Liquidation repay | Planned `RepayEntry.amount` (already trimmed) | Same as repay if still > ceil debt | `liquidator` | Same; ideal-close excess never pulled |
| `recapitalize` excess | `received > backing_shortfall` | Pool token balance (uncredited) | `payer` | None — `credit_cash(applied)` only |
| Strategy `repay_debt_from_controller` | Same as repay, payer = controller | Pool → controller, then controller | `caller` (owner/delegate) | Pool net only; controller forwards measured Δ |
| Swap router underspend | `actual_spent < amount_in` | Controller `token_in` | `refund_to` (= caller) | N/A (never entered pool as repay) |
| `migrate_from_blend` leftover borrow | Cap > Blend liability | Controller debt asset | Nested via `repay_debt_from_controller` → caller | Debt shares burned for consumed portion |
| `flash_position` listed refund | Callback push of listed non-collateral | Controller | `caller` | No repay; debt stays open (INV-STRAT-04) |

---

## 1. Debt repay overpayment

### 1.1 Controller orchestration

```
process_repay / execute_repayment / apply_liquidation_repayments
  → transfer_amount_measured(payer → pool, requested)   # INV-ACCT-03
  → PoolAction.amount = received                        # never requested
  → pool_repay_call(payer, actions)
  → merge_debt_leg(Exit) from mutation.actual_amount    # = net_repay
```

Ordinary repay (`debt.rs` Repay arm) passes `payer = caller`. Third-party repay therefore refunds the **payer**, not the account owner — harness `test_repay_by_third_party` / overpay comment: Bob is debited net of refund; Alice’s wallet untouched.

Liquidation (`apply_liquidation_repayments`) passes `payer = liquidator` and only transfers **planned** amounts after math already trimmed ideal-close excess into estimate-only `refunds` (formulas.md). Any residual pool-level overpay (e.g. ceil rounding) still refunds the liquidator via the same pool path.

Strategy repay (`execute_repayment` + `legs.rs` event context) sets `counterparty = controller`, so the pool’s `payer` is the controller. That is intentional: tokens already sit in controller custody.

### 1.2 Pool accounting (`ops/repay.rs` + `resolve_repay`)

```
(burned, overpayment) = resolve_repay(amount, pos_scaled, borrow_index, decimals)
net_repay = amount - overpayment
assert net_repay == 0 || burned > 0          # INV-ACCT-05
burn_debt(burned); credit_cash(net_repay)    # overpay NEVER credited
commit; transfer_out(payer, overpayment)     # no-op if ≤ 0
mutation.actual_amount = net_repay
```

`resolve_repay` (ADR-0003):

- If `amount >= ceil(debt)` → burn **all** scaled debt; excess = `amount − ceil(debt)`.
- Else → floor-burn shares for `amount`; excess = 0.

So partial overpay relative to ceil never quietly pads cash. Controller merges `actual_amount = net_repay`, so debt shares and spoke-usage exits track burned debt, not the gross transfer (A025 / A082).

### 1.3 Properties checked

| Property | Mechanism | Evidence |
|---|---|---|
| Overpay does not inflate cash | `credit_cash(net_repay)` | pool flows + `outbound_transfer_measurement` comment |
| Overpay returns to payer | `transfer_out(payer, overpayment)` | `test_repay_overpayment_refunded`, `repay_overpayment_is_refunded_to_the_payer_not_stranded` |
| Controller retains nothing on plain repay | Tokens never touch controller | same harness |
| FOT inbound shrinks burn, not refund inflation | measured `amount_in` | INV-ACCT-03; A055 |
| Zero overpay is silent no-op | `transfer_out` if `amount <= 0` | `cash.rs:42-45`; router test `test_repay_without_excess_skips_zero_value_refund` (strategy) |
| Batch legs independent | per-action `apply` | `test_bulk_repay_overpayment_refunds_second_entry_surplus` |

### 1.4 Residual (not undefended)

Outbound refund is unmeasured at the recipient (same as recap). Under FOT, payer receives less than `overpayment`; pool’s live balance drops by the transfer amount; cash book never held the overpay, so books stay aligned. Protocol cannot keep the haircut as spendable cash. Severity: info / token-tax UX.

---

## 2. Recapitalize excess

### 2.1 Controller (`keepers.rs`)

```
payer.require_auth(); require_not_flash_loaning; require_positive_amount
received = transfer_amount_measured(payer → pool, amount)
return pool_recapitalize_call(..., received).actual_amount
```

Ordering matches A016: measure first, pass **received**, never the caller request. Pause-exempt / permissionless (INV-HALT-01 / INV-AUTH-03 donation path). Flash-loan flag still blocks reentry during another flash.

### 2.2 Pool (`ops/recapitalize.rs`)

```
applied = min(amount, backing_shortfall(cache))
refund  = amount - applied
credit_cash(applied); commit
transfer_out(payer, refund)
return actual_amount = applied
```

`backing_shortfall` = max(0, floor(supply claims) − (cash + ceil debt)). Excess never mints shares (INV-ACCT-04). Healthy market → `applied = 0`, full receipt refunded (`test_recapitalize_refunds_everything_when_market_is_already_backed`, harness FOT variant).

### 2.3 FOT double-haircut (documented, accepted)

Harness `recapitalize_refund_is_unmeasured_but_strands_nothing`: inbound FOT then outbound FOT; pool balance and cash book restore to pre-call; controller balance unchanged; payer absorbs two token taxes. Explicitly answers “unmeasured refund — where does the difference go?”: nowhere in protocol custody.

### 2.4 Properties

| Property | Status |
|---|---|
| Cannot credit more than delivered | defended (measured `received`) |
| Cannot apply more than shortfall | defended (`min` clamp) |
| Excess returns to payer | defended |
| No share mint / no foreign risk | defended (donation-only) |
| Return value = applied, not refund | defended (`actual_amount`) |

---

## 3. Strategy leftovers and nested overpay

### 3.1 Shared primitive: `repay_debt_from_controller`

```
received = transfer_amount_measured(controller → pool, debt_available)
balance_before = controller.balance(debt asset)          # AFTER pull
execute_repayment(..., amount=received, payer=controller)
refund_controller_balance_delta(asset, balance_before, caller)
```

Two-stage refund:

1. Pool `transfer_out(controller, overpayment)` if `received > ceil(debt)`.
2. Controller forwards `max(0, balance_now − balance_before)` to `caller`.

Baseline is post-pull, so:

- Pre-existing controller inventory of that asset is **inside** the baseline and is not swept (`test_swap_debt_refund_only_uses_strategy_excess` mints 50 ETH dust; only strategy overpay reaches Alice).
- If pool refunds nothing, delta ≤ 0 → no-op (no zero-value transfer spam).

Used by: `swap_debt`, `repay_debt_with_collateral` (cross-asset arm), `migrate_from_blend` (`reconcile_debt_refunds`).

### 3.2 Swap leftover (`swap.rs`)

After router call under flash guard:

```
assert in_after ≤ in_before
actual_spent = in_before − in_after
assert actual_spent ≤ amount_in
leftover = amount_in − actual_spent
if leftover > 0: transfer(controller → refund_to, leftover)
verify_router_output(token_out)  # measured Δ > 0
```

- Residue recipient is always the strategy `caller` (passed as `refund_to`).
- Pre-authorization is exact `amount_in` (INV-STRAT-01); overspend panics.
- Leftover size is **authorized unused**, not gross balance — preserves stranded dust of `token_in` the same way delta refunds do.
- Raw transfer of `leftover`: FOT under-delivers to caller only (A045 parallel).

Consumers: `multiply`, `swap_debt`, `withdraw_and_swap_from_supply` (collateral / repay-with-collateral), convert-swap on multiply initial payment.

Harness: `test_swap_collateral_refunds_router_underspend_to_caller`, `test_repay_debt_with_collateral_refunds_router_underspend_to_caller`.

### 3.3 Migrate leftover borrow

```
snapshot controller balances for debt assets
borrow_into_controller(max) for each cap
blend_repay_all(...)
reconcile_debt_refunds: for each debt asset with Δ>0
    repay_debt_from_controller(debt_available=Δ)
```

Borrowed buffer above Blend liability returns to the controller as token balance, then is repaid into hub debt; any still-excess vs hub ceil debt refunds to `caller` via §3.1. Fuzz `prop_migrate_blend_reconciles_same_asset` bounds controller leftover ≤ slack.

### 3.4 Flash position listed refunds (A045; residual only)

`refund_listed_assets` → `refund_controller_balance_delta` with pre-callback baselines. Undeclared assets stranded and unstealable. Debt token in `refund_assets` refunds cash **without** repay (INV-STRAT-04). Not re-litigated here; money-safe under A045.

### 3.5 Multiply

No repay leg. Leftover handling is solely swap underspend → caller (§3.2), then full measured collateral deposit. No post-deposit refund of collateral asset (deposit consumes Δ). Controllers should not hold debt-asset residue after a successful swap that spends `amount_received + debt_extra`.

---

## 4. What is *not* a refund-transfer bug

### 4.1 Liquidation estimate `refunds`

`calculate_repayment_amounts` / `process_excess_payment` build `PaymentTuple` refunds for the **view**. Apply path transfers only trimmed `RepayEntry` amounts. formulas.md: “no refund transfer occurs.” Treating missing `transfer` of `plan.refunds` as a hole would be wrong.

### 4.2 Cash conservation vs live balance

Overpay sits in the pool’s **token** balance without entering `cache.cash`. `transfer_out` deliberately does not debit cash (`cash.rs` comment). Live balance and cash book diverge only for this uncredited buffer, then reconverge after refund. Donations similarly do not create lendable cash (INV-ACCT-02).

### 4.3 Controller as pool owner

Pool `repay` / `recapitalize` are `#[only_owner]`. External users cannot call pool refund APIs directly; they must enter through controller measured pulls. Spoofing `payer` on a direct pool call is gated by owner auth.

---

## 5. Threat / failure matrix

| Scenario | Outcome |
|---|---|
| User repays 2× debt | Debt cleared; ~1× refunded to payer; cash += net only |
| Third party overpays | Refund to third party; borrower wallet untouched |
| Recap on healthy market | `actual_amount=0`; full measured receipt refunded |
| Recap offer > shortfall | Apply shortfall; refund excess |
| Strategy repay with excess swap output | Pool refunds controller; controller forwards Δ to caller; dust untouched |
| Router pulls less than `amount_in` | Leftover `token_in` → caller; output still measured |
| Router tries to pull more | `RouterOverspend` / auth bound |
| FOT on refund hop | Recipient short; protocol books unchanged; nothing stranded in controller/pool cash |
| Flash callback returns unlisted asset | Stranded at controller; next caller’s baseline hides it |
| Mid-path panic after pool refund | Whole tx rolls back (Soroban atomicity) |
| Reenter via refund transfer hook | Shared A007 residual; listing trust |

---

## 6. Invariant cross-walk

| Invariant | How refunds uphold it |
|---|---|
| INV-ACCT-02 | Overpay/excess never credited to cash |
| INV-ACCT-03 | Inbound credit uses measured receipt before pool call |
| INV-ACCT-04 | Recap cap + excess refund; no shares |
| INV-ACCT-05 | Positive net repay must burn shares; dust repay reverts |
| INV-STRAT-01 | Exact auth; leftover = unused auth |
| INV-STRAT-02 | Residue to caller; measured swap out; solvency finalize |
| INV-STRAT-04 | Flash refund ≠ repay |

---

## 7. Peer alignment

| Peer | Agreement |
|---|---|
| A016 | Recap measure-then-clamp; excess refund defended |
| A025 | Pool refunds overpay to payer; controller merges net |
| A041 | Custody legs measure inbound; refunds are the outbound counterpart |
| A045 | Flash listed delta refunds; undeclared stranded; raw transfer OK |
| A007 | Post-guard leftover/refund transfers = listing-trust residual |
| A055 | Lying/FOT tokens: measured inbound prevents credit inflation; outbound refund under-delivery is tax |

No disagreement file needed.

---

## 8. Verdict

**Status: defended** for A054 scope (debt overpay refunds, excess recapitalize refunds, strategy leftover / nested overpay forwards).

The protocol consistently treats overpay and excess as **uncredited residue** returned to the party that funded the excess (payer or strategy caller), with controller custody paths using **post-action balance deltas** so unrelated inventory cannot be stolen. Liquidation “refunds” are estimate-side trims, not a missing transfer. Residuals (FOT outbound haircuts, post-guard hooks, flash stranding) are accepted or shared with other agents and do not create a fund-theft or share-inflation path.

Remediation from this audit alone: none required on production Rust. Optional docs hygiene: keep the dual-layer repay refund explanation next to INV-ACCT-02/03 (already present in harness comments).
