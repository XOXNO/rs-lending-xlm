# A026 — Liquidation apply storage writes (debt burn, seize, fees)

- Agent: A026
- Theme: T2 (storage mutations on liquidation apply)
- Severity: low
- Status: defended (accepted residuals documented)
- Paths:
  - `contracts/controller/src/lib.rs` (`liquidate`)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-153` (`process_liquidation`), `:170-216` (`resolve_seize_receiver`)
  - `contracts/controller/src/positions/liquidation/apply.rs` (full file: repay / Transfer seize / Credit seize / fee split / post-liq cleanup hook)
  - `contracts/controller/src/positions/liquidation/math.rs:455-491` (`scale_seizures_to_received`), `:369-404` (`split_seized_shares`)
  - `contracts/controller/src/positions/debt.rs:190-212` (`apply_repay_batch`)
  - `contracts/controller/src/positions/supply.rs:283-435` (`apply_withdraw_batch` / `merge_withdraw_leg` / liquidation restamp freeze)
  - `contracts/controller/src/positions/mod.rs:112-252` (`apply_leg_usage` / `merge_debt_leg` / `persist_account_positions` / `finalize_position_flow`)
  - `contracts/controller/src/account.rs:47-76,172-221` (Credit(0) create; upsert/remove; empty cleanup)
  - `contracts/controller/src/spoke_usage.rs:77-141`; `context/spoke.rs:104-143`; `context/events.rs:11-62`
  - `contracts/controller/src/storage/account.rs:71-103,247-270`
  - Cross-contract (not controller keys, but books the fee/debt burn): `contracts/pool/src/ops/{repay,withdraw,seize}.rs`, `pool/src/interest.rs:57-68`, `pool/src/cache/shares.rs:41-48`
- Defense: All controller durable mutations for a successful liquidation commit through explicit finalize tails after in-memory merges. Debt burns follow measured pool repay outcomes; Transfer seizures follow measured pool withdraw outcomes (gross burn + withheld fee); Credit seizures move scaled shares account-to-account with conservation `fee + liquidator = seized`, booking only the fee via pool `absorb_supply_as_revenue` and only the fee as spoke-usage exit. Victim persists `PositionSides::Both`; Credit receiver persists `Supply` only. Empty-account / bad-debt cleanup is a separate post-finalize step (A027). Events for position batches emit after persists (A033); `LiquidationEvent` is observational and still tx-atomic.
- Gap: (a) Inherited A080 — repay Exit, Transfer seize Exit, and Credit fee Exit no-op when no spoke-usage row exists. (b) Finalize uses `remove_if_empty: false`, then `check_bad_debt_after_liquidation` may delete — can rewrite then delete residual maps in-tx (rent; A027/A036). (c) Credit mode opening a **new** receiver supply slot calls `require_spoke_asset` — delisted collateral cannot be Credit-seized onto a receiver that does not already hold that hub (Transfer still works). (d) Transfer seize is index-sensitive between plan and pool apply; Credit is share-denominated and immune (documented). None are silent cross-key corruption or double-mint of shares inside controller storage.
- Impact: Successful liquidation can only decrease victim debt/supply shares (and Credit-increase receiver supply by `S − fee`), decrease spoke usage by repaid debt scaled delta + (Transfer: full seize delta | Credit: fee only), optionally create receiver `AccountMeta`/NFT on `Credit(0)`, renew TTLs, and optionally cascade into bad-debt full delete (A027). Cannot inflate victim collateral, mint unbacked controller debt, rewrite foreign accounts outside the resolved receiver, or leave pool revenue unbacked on the Credit fee path (absorb, not mint). Cap distortion from A080 is availability/governance, not direct theft.
- Evidence: INV-LIQ-01..04, INV-HALT seizure/`AllowOnExit` policies, INV-STOR-01/03; ADR-0003 (rounding), ADR-0008 (seizure halt), ADR-0019 (Credit mode); Certora `liquidation_does_not_increase_repaid_debt`, `liquidation_does_not_increase_seized_collateral`; harness `tests/test-harness/tests/controller/liquidation_seize_modes.rs` (fee-only usage, two position batches, cash-untouched Credit); unit `contracts/controller/tests/positions/liquidation_seize_modes.rs` (split conservation); peers A013, A023–A025, A027, A033, A036, A080, A084.
- Opinion: Liquidation’s write set is the most complex user path in Wave 2, but the buffer-then-finalize discipline holds. The load-bearing subtleties are mode-split fee booking (Transfer mint-withhold vs Credit absorb) and Credit’s fee-only usage delta — both are intentional and test-pinned. Treat A080 and Credit-delist new-slot liveness as residuals, not as missing debt-burn or seize writes.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only).
2. Trace `Controller::liquidate` → `process_liquidation` → `apply_liquidation_repayments` / `apply_liquidation_seizures` | `apply_liquidation_share_credit` → `finalize_position_flow` (+ Credit second finalize) → `check_bad_debt_after_liquidation`.
3. Enumerate every durable controller write and every cross-contract pool write that backs debt burn, seize, and protocol fees; separate Cache buffers from persistent keys.
4. Cross-check peer findings A013 (receiver gates), A023–A025 (shared merge/finalize primitives), A027 (post-liq socialization), A033 (event order), A036 (cleanup flags), A080/A084 (usage), and INV-LIQ-*.
5. Out of scope as primary claims: plan/curve arithmetic correctness (except where it feeds stored amounts), bad-debt body internals (A027), auth/self-liq (A013), Transfer token outflow measurement depth (A051), Credit share math product claims beyond storage conservation (A052).

---

## Call graph (storage-relevant)

```
Controller::liquidate                         # permissionless; no #[when_not_paused]
  └─ process_liquidation                      liquidation/mod.rs
       ├─ liquidator.require_auth
       ├─ require_not_flash_loaning           # READ temp flash flag
       ├─ storage::get_account(victim)        # READ meta+supply+debt
       ├─ Cache::new                          # renews controller instance TTL
       ├─ resolve_seize_receiver
       │    ├─ Transfer → None
       │    ├─ Credit(0) → create_account_with  # WRITE AccountMeta + NFT mint
       │    └─ Credit(id) → load + owner/delegate + spoke + Normal
       ├─ build_liquidation_plan              # reads only; validates HF / legs
       ├─ apply_liquidation_repayments
       │    ├─ transfer_amount_measured       # token → pool (measured)
       │    ├─ floor-scale leg USD if under-delivery
       │    └─ apply_repay_batch
       │         ├─ pool_repay_call           # POOL: burn debt + credit cash
       │         └─ merge_debt_leg Exit       # RAM debt + usage Borrow Exit + event buf
       ├─ scale_seizures_to_received          # floor-scale seize to received_usd
       ├─ [Transfer] apply_liquidation_seizures
       │    └─ apply_withdraw_batch(Liquidation)
       │         ├─ pool_withdraw_call        # POOL: burn supply, withhold fee→revenue mint,
       │         │                            #        debit cash, pay liquidator net
       │         └─ merge_withdraw_leg        # RAM supply; usage Supply Exit; NO risk restamp
       ├─ [Credit] require_credit_position_limit
       │    apply_liquidation_share_credit
       │         ├─ debit victim supply by S  # RAM; event LiqSeize; NO usage for S
       │         ├─ credit_supply_shares      # RAM receiver += S−fee (stamp listing if new)
       │         ├─ apply_spoke_exit(fee)     # usage −fee only
       │         └─ pool_seize_positions      # POOL: absorb_supply_as_revenue(fee)
       ├─ LiquidationEvent.publish            # observational; before persist
       ├─ calculate_account_risk_totals       # in-memory post-liq victim
       ├─ finalize_position_flow(victim, Both, remove_if_empty=false)
       │    ├─ persist_spoke_usage            # WRITE SpokeUsage keys
       │    ├─ set_supply_positions + set_debt_positions (+ empty remove)
       │    ├─ renew_user_account
       │    └─ emit_position_batch            # clears buffers
       ├─ [Credit] record_share_credit_updates + finalize(receiver, Supply, false)
       └─ check_bad_debt_after_liquidation
            ├─ no debt → cleanup_account_if_empty
            └─ dust insolvent → execute_bad_debt_cleanup (A027)
```

---

## 1. Durable write inventory

### 1.1 Controller persistent keys

| Key / effect | When | Op |
|---|---|---|
| `AccountMeta(receiver)` | `Credit(0)` only, before money | **set** (create) |
| Position NFT mint | `Credit(0)` | external mint |
| `SpokeUsage(spoke, hub)` | finalize (and Credit second finalize re-persist) | set / remove-if-both-zero |
| `SupplyPositions(victim)` | victim finalize `Both` | set / remove-if-empty map |
| `BorrowPositions(victim)` | victim finalize `Both` | set / remove-if-empty map |
| TTL bump victim account keys | victim finalize | renew |
| `SupplyPositions(receiver)` | Credit second finalize `Supply` | set / remove-if-empty map |
| TTL bump receiver account keys | Credit second finalize | renew |
| Full account key set + NFT burn | post-liq empty cleanup or bad-debt | remove (A027/A036) |

**Not written on the happy apply path:** protocol instance config (pool/oracle/NFT addresses), spoke/hub listing config, victim `AccountMeta` spoke/mode, delegates maps (unless cleanup deletes them), flash-loan temp flag, market indexes on controller (Cache-only; pool remains SoT — A094/A038).

### 1.2 In-memory / Cache only (until finalize)

| Buffer | Mutations on apply |
|---|---|
| `Account` supply/debt maps | repay Exit; Transfer withdraw; Credit debit/credit |
| `SpokeUsageContext` | Borrow Exit (repay); Supply Exit (Transfer full Δ or Credit fee) |
| Event `supply_updates` / `debt_updates` | `LiqRepay`, `LiqSeize`, later `LiqCredit` |
| `put_market_index` | from pool mutation outcomes on repay/Transfer withdraw legs |

### 1.3 Pool durable books (cross-contract)

| Mode / leg | Pool effect |
|---|---|
| Repay | `burn_debt` + `credit_cash` (measured received) |
| Transfer seize | `burn_supply` (gross) + `add_protocol_revenue` / `accrue_revenue` (fee mint) + `debit_cash` (net to liquidator); skips utilization gate |
| Credit fee | `absorb_supply_as_revenue` — **revenue only**, `supplied` unchanged |
| Credit principal (`S − fee`) | **no pool call** — shares reassigned only on controller maps |

The apply.rs comment that Transfer’s withhold mint must not be used for Credit is load-bearing: minting revenue without withholding cash would create an unbacked supplier claim.

---

## 2. Debt burn storage path

`apply_liquidation_repayments`:

1. Per leg: `FreezePolicy::AllowOnExit` (paused blocks repay; frozen OK).
2. `transfer_amount_measured` into the pool; if `received < entry.amount`, floor-scale that leg’s planned USD (`mul_div_floor`) so seizure coupling (INV-LIQ-03) uses **received** value.
3. `apply_repay_batch` → `pool_repay_call` → `merge_debt_leg(..., Exit, pool outcome)`.

`merge_debt_leg` Exit:

- Baseline scaled from existing debt (panic if missing — cannot invent repay slot).
- `apply_leg_usage` Borrow Exit with `old − new` from **pool** outcome (A082).
- Updates RAM debt via `update_or_remove_debt_position` (zero → map remove).
- Buffers `PositionAction::LiqRepay`.

Victim finalize writes `BorrowPositions` with `PositionSides::Both`. Empty debt map deletes the key (INV-STOR-01).

**Contrast with ordinary repay (A025):** liquidation loads full account and always persists **Both** sides (supply also changes on seize). Ordinary repay is borrow-only load + `PositionSides::Debt`. Liquidation correctly widens sides.

---

## 3. Transfer seize + fee storage path

`apply_liquidation_seizures` builds `PoolWithdrawEntry { action, protocol_fee }` and calls `apply_withdraw_batch(..., WithdrawKind::Liquidation, LiqSeize)`.

Pool (`ops/withdraw.rs`):

- Resolves burn from requested amount vs live position.
- `withhold_liquidation_fee`: requires `gross >= protocol_fee`; mints fee as revenue shares (`add_protocol_revenue` → `accrue_revenue`); pays liquidator `gross − fee`.
- Skips max-utilization; still requires reserves + solvent withdraw state.

Controller `merge_withdraw_leg`:

- Sets supply scaled to pool `new_scaled`.
- Supply usage Exit for full burned Δ (fee portion leaves the user-supply system → full exit is correct).
- **Does not restamp** risk params (`leg_may_restamp_risk_params` false for `WithdrawKind::Liquidation`) — matches apply.rs Credit debit policy and avoids mid-liq LTV churn.
- Event amount = pool gross `outcome.amount` (not net paid).

Under-delivery scaling (`scale_seizures_to_received`) floor-scales `amount` and `protocol_fee` by the same ratio, preserving `fee ≤ amount` when it held pre-scale. Zeroed legs become pool no-ops (`gross == 0` allowed) / no share movement.

---

## 4. Credit seize + fee storage path

`apply_liquidation_share_credit` is the divergent write shape:

| Step | Victim | Receiver | Spoke usage | Pool |
|---|---|---|---|---|
| Debit `S` | `scaled −= S` (trap if overshoot) | — | none | — |
| Credit `S − fee` | — | `scaled += liquidator` (or create stamped from **current** listing) | none | — |
| Fee `fee` | — | — | `apply_spoke_exit(Supply, fee)` | `PoolSeizeEntry { Deposit, fee }` → absorb |

Conservation: `split_seized_shares` uses `mul_div_ceil` on `bonus_scaled × fees / BPS` (protocol-favourable) and asserts `fee + liquidator == seized`. Re-checked in-loop with `checked_sub` identity. Same-spoke assert (`SpokeMismatch`) before books move.

**Why fee-only usage (A084):** net spoke supply movement is `−S + (S − fee) = −fee`. Routing the credit through `apply_spoke_entry` would enforce the supply cap and could brick liquidations at cap — explicitly rejected in comments. Harness `credit_mode_moves_spoke_usage_by_exactly_the_protocol_fee` pins this.

**Receiver position limits:** `require_credit_position_limit` runs **before** mutate, using credited hubs only. Liquidator can fall back to `Credit(0)`.

**Risk stamp import blocked:** `credit_supply_shares` never copies the victim’s LTV/threshold/bonus/fees tuple onto the receiver. New slots stamp from live listing; existing slots keep their own stamps and only grow scaled amount.

**Event ordering:** Victim `LiqSeize` (gross) buffers during apply; victim finalize emits first batch; then `record_share_credit_updates` buffers `LiqCredit` (net of fee, asset amount floor from scaled×index); receiver finalize emits second batch. Harness `credit_mode_emits_two_position_batches_liquidated_account_first`.

**Double `persist_spoke_usage`:** Second finalize re-writes the same buffered usage rows (persist does not clear the map). Idempotent; no double-decrement because exits are not re-applied.

---

## 5. Finalize, cleanup, and side selection

```126:150:contracts/controller/src/positions/liquidation/mod.rs
    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::Both,
        false,
    );
    // ...
    apply::check_bad_debt_after_liquidation(env, &mut cache, account_id, &account, &post_totals);
```

| Flag | Value | Rationale |
|---|---|---|
| Victim `sides` | `Both` | Repay touched debt; seize touched supply |
| Victim `remove_if_empty` | `false` | Defer delete to post-liq gate (empty cleanup vs dust bad-debt) |
| Receiver `sides` | `Supply` | Credit never opens receiver debt |
| Receiver `remove_if_empty` | `false` | Receiver just received shares |

`check_bad_debt_after_liquidation`:

- Empty borrows → `cleanup_account_if_empty` (both sides empty ⇒ delete keys + burn NFT).
- Else if `is_socializable_bad_debt` → `execute_bad_debt_cleanup` (A027).
- Else leave residual undercollateralized (non-dust) account as-is.

**Residual (b):** Finalize may persist empty or near-empty maps, then cleanup/bad-debt removes them in the same transaction — extra rent write only (agrees with A027). Not a fund-safety hole.

**Correction vs A036 shorthand:** liquidation finalize itself does **not** set `remove_if_empty: true`; cleanup is the dedicated post step. That split is correct so dust socialization can still see the account and remaining positions.

No `enforce_post_pool_solvency` on this path — correct: the account starts unhealthy; requiring HF ≥ 1 would break partial liquidations.

---

## 6. Coupling defenses (INV-LIQ)

| Invariant | Storage implication |
|---|---|
| INV-LIQ-02 repay/seize coupling | Only planned `repaid` / scaled `seized` enter apply; refunds never transfer (estimate-only) |
| INV-LIQ-03 under-delivery | Measured repay shrinks USD → `scale_seizures_to_received` before any seize write |
| INV-LIQ-01 Credit not-self | Receiver ≠ victim before money; second spoke assert in apply |
| Fee conservation (Credit) | Asserted split; pool absorb amount = fee only |
| Halt policy | Repay `AllowOnExit`; seize `SeizureLeg` (`no_seize` only) — ADR-0008 |

---

## 7. Residuals and non-findings

### 7.1 Inherited A080 (usage exit no-op)

Repay Borrow Exit, Transfer Supply Exit, and Credit fee Exit all call `apply_spoke_exit`. Missing row → silent no-op. After a full clear, capacity can remain overstated until reconcile. Cap distortion only; positions and pool books still move. Cross-ref A080, A084.

### 7.2 Credit + delisted hub, new receiver slot (liveness)

`credit_supply_shares` uses `cache.require_spoke_asset` when the receiver has no existing position. A delisted hub (no spoke-asset row) therefore **cannot** open a new Credit slot, while Transfer seize still works (victim already holds the position; withdraw merge does not require listing). Workarounds: Transfer mode, or Credit to a receiver that already holds that hub. Not a storage corruption bug; Credit liveness hole for delisted collateral on empty receivers.

### 7.3 Transfer index drift vs Credit immunity

Plan stamps indexes at planning time; Transfer apply trusts pool mutation indexes/burns after live accrual on withdraw. Controller storage follows pool outcomes (correct SoT). Credit moves precomputed scaled shares and documents immunity to index drift. Residual is economic sizing for Transfer under intra-tx accrual, not double-write of controller keys.

### 7.4 `LiquidationEvent` before persist

Published before `finalize_position_flow`. On Soroban, failure after the publish still aborts the transaction (event + pool + controller roll back together). Position-batch events remain persist-before-emit (A033). Observational ordering quirk only.

### 7.5 Explicit non-findings

- No path credits seized shares back onto the liquidated account (A013).
- No path mints Credit fee revenue via withhold (would unback supply) — absorb only.
- No path persists receiver debt on Credit.
- No path skips victim debt map persist after repay (Both).
- Zero-fee / zero-bonus splits leave `fee = 0` and skip pool seize + usage exit (unit-tested).
- `scale_seizures_to_received` does not drop zero legs, but zero legs are storage no-ops.

---

## 8. Evidence map

| Claim | Anchor |
|---|---|
| Measured repay → pool → merge Exit | `apply.rs:31-83`, `debt.rs:190-212`, `mod.rs:148-188` |
| Transfer fee withhold + no restamp | `supply.rs:283-435`, `pool/ops/withdraw.rs:106-131` |
| Credit conservation + fee-only usage | `apply.rs:139-221`, `math.rs:369-404` |
| Fee absorb not mint | `apply.rs:130-135`, `pool/cache/shares.rs:41-48` |
| Finalize Both / Supply + post cleanup | `liquidation/mod.rs:122-150`, `apply.rs:326-340` |
| Two Credit batches ordered | harness `liquidation_seize_modes.rs` |
| Usage fee-only | harness `credit_mode_moves_spoke_usage_by_exactly_the_protocol_fee` |

---

## 9. Verdict

**Defended** for controller storage integrity on liquidation apply: debt burns, Transfer seizures, Credit share moves, and protocol fees commit through a single buffered account model and explicit finalize writes, with mode-appropriate pool fee booking. Highest residuals are shared spoke-usage exit tolerance (A080), in-tx rewrite-then-delete on cleanup (rent), and Credit liveness for delisted hubs on new receiver slots — none demonstrated as silent share inflation, foreign-account writes, or unbacked revenue mint on the Credit path.
