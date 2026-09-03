# A051 — Liquidation Transfer seize token outflow

- Agent: A051
- Theme: T3 (money movement — `SeizeMode::Transfer` collateral payout)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs` (`liquidate` → `SeizeMode`)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-152` (`process_liquidation`; Transfer arm `receiver = None`)
  - `contracts/controller/src/positions/liquidation/apply.rs:31-83` (measured repay; INV-LIQ-03 input)
  - `contracts/controller/src/positions/liquidation/apply.rs:85-121` (`apply_liquidation_seizures`)
  - `contracts/controller/src/positions/liquidation/math.rs:249-366` (`calculate_seized_collateral` fee/gross sizing)
  - `contracts/controller/src/positions/liquidation/math.rs:455-491` (`scale_seizures_to_received`)
  - `contracts/controller/src/positions/supply.rs:283-301` (`apply_withdraw_batch` / `WithdrawKind::Liquidation`)
  - `contracts/controller/src/positions/supply.rs:365-417` (`merge_withdraw_leg`; no risk restamp on liquidation)
  - `contracts/controller/src/external/pool.rs:54-65` (`pool_withdraw_call`)
  - `contracts/pool/src/ops/withdraw.rs` (full: `apply` / `accounting` / `withhold_liquidation_fee` / `gate_and_debit`)
  - `contracts/pool/src/cache/cash.rs:12-48` (`require_reserves` / `debit_cash` / `transfer_out`)
  - `contracts/pool/src/interest.rs:57-68` (`add_protocol_revenue` mint)
  - `contracts/pool/src/cache/shares.rs:35-39` (`accrue_revenue` increases `revenue` + `supplied`)
  - `common/src/rates/scaling.rs:103-119` (`resolve_withdrawal`)
- Defense: Transfer seize pays the authenticated `liquidator` from pool cash only after (1) measured debt receipt has floor-scaled every seize leg, (2) plan validation ensures `0 ≤ protocol_fee ≤ amount`, (3) pool resolves burn/gross against the live position, (4) fee is withheld from the outbound amount and minted as revenue shares backed by retained cash, (5) reserves + solvent-withdraw gates run on the **net** payout, (6) utilization is skipped so cash-thin but solvent markets stay liquidatable, (7) controller supply books follow pool mutation outputs (gross burn). Outbound tokens are not re-measured at the controller — intentional for pool→EOA payout under the SAC listing assumption (same class as ordinary withdraw).
- Gap: (a) No controller-side balance delta on the liquidator receive — FOT/rebasing collateral can short the liquidator or desync cash vs SAC if a non-SAC is listed (A055 / A041 residual; not unique to liquidate). (b) Transfer is index/asset-unit sized at plan time; live pool accrual can change burn vs planned economic size (Credit is share-immune — ADR-0019). (c) Cash starvation reverts Transfer (`#112`); Credit is the designed escape hatch. (d) Dust fee bump / fee==gross can zero a leg’s payout (numeric-bounds known cost against the liquidator). (e) `require_external_recipient` is not called — recipient is the authed liquidator, not a separate `to`. None demonstrated as silent over-payout or unbacked fee mint.
- Impact: Successful Transfer liquidation decreases victim supply shares by the pool-reported burn (gross of fee), pays the liquidator `gross − fee` in underlying, retains `fee` cash in the market while minting matching revenue shares, and exits spoke supply usage by the full burned scaled delta. Cannot pay more underlying than `require_reserves(net)` allows, cannot mint Transfer fee revenue without withholding the same asset units from the outbound transfer, and cannot keep seizure sized to undelivered debt repayment (INV-LIQ-03). Blast radius of a lying outbound collateral token is that market’s TVL / liquidator proceeds under listing governance — not a missing withhold or scale step in this path.
- Evidence: INV-LIQ-01..03, INV-ACCT-02/03 (inbound-only measure), INV-HALT-02 (`SeizureLeg`), ADR-0003, ADR-0008, ADR-0019; Certora `liquidation_does_not_increase_seized_collateral`, `liquidation_does_not_increase_repaid_debt`, `withdraw_never_overdraws_cash`, `withdraw_keeps_revenue_backed`; unit `contracts/controller/tests/events.rs` (`transfer_mode_seizure_delta_is_gross_of_the_protocol_fee`); harness `liquidation_seize_modes.rs` (`cash_starved_market_blocks_transfer_but_not_credit`), `liquidation_accrual_timing.rs`, `deprecated_spoke_liquidation_liveness.rs`; peers A013, A026, A041, A055, A082, A084; A053 owns deeper fee-product claims.
- Opinion: Transfer outflow is the classical “burn shares → withhold fee → pay net” path and is correctly coupled to measured repay before any collateral leaves the pool. Treat as **defended** for fund safety; keep the mint-withhold identity load-bearing and do not “fix” Credit by copying this mint path.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only write).
2. Trace `SeizeMode::Transfer` only: plan seize legs → `scale_seizures_to_received` → `apply_liquidation_seizures` → pool `withdraw(is_liquidation=true)` → controller `merge_withdraw_leg`.
3. Inventory every token leaving the pool, every cash/share book change that backs that outflow, and every place amounts are measured vs trusted.
4. Cross-check peers A013 (receiver identity — N/A for Transfer), A026 (storage), A041/A055/A082 (measurement), A084 (usage), ADR-0019 (why Transfer fee ≠ Credit fee), INV-LIQ-*.
5. Out of scope as primary claims: Credit share credit (A052), fee BPS product beyond withhold identity (A053), bad-debt socialization (A014/A027), auth/self-liq (A013).

---

## Verdict

**Defended.** Collateral tokens leave the pool only as `net_transfer = gross − protocol_fee` to the liquidator who authorized the call. Gross share burn, fee revenue mint, and cash debit are ordered so revenue is never minted without retained cash, and seizure size never exceeds the measured repayment USD (floor-scaled). Residuals are listing-trust outbound measurement, cash liveness, and plan-time index sensitivity — all documented elsewhere and not silent over-seize.

---

## 1. End-to-end Transfer money sequence

```
Controller::liquidate(liquidator, account, debts, SeizeMode::Transfer)
  └─ process_liquidation
       ├─ liquidator.require_auth
       ├─ require_not_flash_loaning
       ├─ resolve_seize_receiver → None          # no account credit path
       ├─ build_liquidation_plan                 # HF<1; seize legs; fee≤amount
       ├─ apply_liquidation_repayments           # MEASURED debt tokens → pool
       │    └─ received_usd floor-scaled per under-delivering leg
       ├─ scale_seizures_to_received             # floor-scale amount+fee+shares
       ├─ apply_liquidation_seizures             # Transfer only
       │    └─ apply_withdraw_batch(Liquidation, liquidator)
       │         └─ pool.withdraw(receiver=liquidator, is_liquidation=true)
       │              per leg:
       │                sync/accrue market
       │                resolve_withdrawal → (burned, gross)
       │                withhold_liquidation_fee → mint revenue; net = gross−fee
       │                burn_supply(burned)
       │                require_reserves(net); skip util; solvent withdraw
       │                debit_cash(net)
       │                transfer_out(liquidator, net)     # SAC; not re-measured
       │         └─ merge_withdraw_leg ← pool mutation (gross actual_amount)
       ├─ LiquidationEvent { repaid_usd = received_usd }
       └─ finalize victim Both; optional bad-debt (A027)
```

Return value is `0` (no Credit receiver id).

---

## 2. Who receives tokens, and why that is enough

| Property | Transfer behavior |
|---|---|
| Recipient | `liquidator` address passed into `apply_liquidation_seizures` / `pool_withdraw_call` |
| Separate `to`? | No — unlike `withdraw` / `borrow` |
| `require_external_recipient`? | **Not called** on this path |
| Auth | `liquidator.require_auth()` at the start of `process_liquidation` |

Ordinary withdraw rejects pool/controller as `to` so users cannot strand funds or poison balance-delta measurements. Transfer has no independent `to`: the payout address **is** the authenticated liquidator. An attacker cannot redirect seizure to the controller/pool without authorizing as those addresses. Self-liquidation in Transfer is allowed by INV-LIQ-01 and pays the owner’s wallet — collateral still leaves the lending account (A013).

---

## 3. Gross burn / fee mint / net pay identity

Pool `ops/withdraw.rs` documents the split explicitly: mutation `actual_amount` is **gross**; `net_transfer` is what leaves.

```
plan:     amount (gross request), protocol_fee ∈ [0, amount]
pool:     (burned, gross) = resolve_withdrawal(amount, position, live index)
          assert gross ≥ protocol_fee          # else #115 WithdrawLessThanFee
          add_protocol_revenue(from_asset(fee)) # mint revenue ⊆ supplied
          burn_supply(burned)                  # user shares exit
          debit_cash(gross − fee)
          transfer_out(liquidator, gross − fee)
controller event / usage: follow pool gross burn (LiqSeize amount = outcome.amount)
```

### Why mint (not absorb) is correct here

`add_protocol_revenue` → `accrue_revenue` increases **both** `revenue` and `supplied`. That would be unbacked if cash equal to the fee left the pool. Transfer **withholds** those units: they stay in `cash` while only `net` is debited and transferred. ADR-0019 / apply.rs comments state the dual: Credit must **not** use this mint path because nothing is withheld there (absorb only).

`protocol_fee_shares` floors. If shares round to zero while `fee > 0` asset units were withheld, cash remains as an unclaimed surplus (protocol/supplier-favourable), not an under-backed revenue mint.

### Plan-time fee bound

`LiquidationPlan::validate` requires `amount > 0` and `0 ≤ protocol_fee ≤ amount` before any money moves. `calculate_seized_collateral` clamps `protocol_fee = bumped_fee.min(pool_gross)`. `scale_seizures_to_received` floor-scales `amount` and `protocol_fee` by the same ratio; because `floor` is monotonic, `fee' ≤ amount'` is preserved. Zeroed legs after under-delivery become pool no-ops (`amount ≥ 0` allowed; `transfer_out` skips `≤ 0`).

---

## 4. Measurement boundary (INV-ACCT-03 vs outbound)

| Leg | Measured? | Mechanism |
|---|---|---|
| Debt repay into pool | **Yes** | `transfer_amount_measured` → floor-scale leg USD → `scale_seizures_to_received` (INV-LIQ-03) |
| Collateral out to liquidator | **No** | Pool `token.transfer(pool → liquidator, net)` after cash debit |
| Controller custody of seize | N/A | Tokens never touch the controller in Transfer mode |

INV-ACCT-03’s liquidation cite is the **repay** measurement in `apply_liquidation_repayments`, not the seize payout. A041 already notes pool→user withdraw/borrow trust pool+token correctness. Transfer seize is the same pattern with an extra withhold.

**Attack if outbound FOT under-delivers without revert:** pool cash book and SAC balance both drop by `net` (sender debited full `net`); liquidator receives less — incentive loss, not supplier insolvency. **Attack if listed token can debit more than requested:** market-wide desync (A055); listing is the outer control.

Flash guard: `require_not_flash_loaning` blocks liquidate during flash; a collateral-token hook during `transfer_out` can still reenter permissionless verbs mid-batch (shared A007/A055 class). Soroban atomicity still rolls back on panic; the residual is listing policy.

---

## 5. Cash, utilization, and solvent-withdraw gates

```96:104:contracts/pool/src/ops/withdraw.rs
fn gate_and_debit(env: &Env, cache: &mut Cache, net_transfer: i128, is_liquidation: bool) {
    cache.require_reserves(net_transfer);

    if !is_liquidation {
        guards::require_utilization_below_max(env, cache);
    }
    guards::require_solvent_withdraw_state(env, cache);
    cache.debit_cash(net_transfer);
}
```

| Gate | Transfer seize | Rationale |
|---|---|---|
| `require_reserves(net)` | Yes | Liquidity for the payout only; fee cash stays |
| Max utilization | **Skipped** | Liquidations must proceed in stressed markets |
| Solvent withdraw state | Yes | Still cannot leave the market accounting-insolvent |
| Liquidation cash buffer (borrow path) | N/A | Not a borrow |

Harness pin: `cash_starved_market_blocks_transfer_but_not_credit` — Transfer hits `#112 InsufficientLiquidity` with cash unchanged; same repay/seize in `Credit(0)` succeeds without moving cash. That is ADR-0019 liveness, not a Transfer hole.

---

## 6. Coupling to repayment (INV-LIQ-02 / INV-LIQ-03)

1. Plan sizes seize from **planned** repay USD (+ bonus).
2. Apply pulls debt with measurement; under-delivery shrinks that leg’s USD.
3. `scale_seizures_to_received` shrinks every seize field before `apply_liquidation_seizures`.
4. Only then does the pool pay collateral.

So a skim/FOT **debt** token cannot buy full planned collateral. Headline `LiquidationEvent.repaid_usd_wad` reports **received** USD, not planned (STRIDE Repudiate.2 / harness).

Seizure never exceeds the live position: plan caps to `actual_ray`; pool `resolve_withdrawal` full-closes when `amount ≥` current supply value.

---

## 7. Controller bookkeeping after outflow

`merge_withdraw_leg` for `WithdrawKind::Liquidation`:

- Sets victim supply scaled to pool `new_scaled` (SoT).
- Spoke usage **Supply Exit** for full burned Δ (fee portion left the user-supply system — full exit correct; A084).
- **Does not restamp** risk params (matches Credit debit policy; avoids mid-liq LTV churn).
- Event amount = pool **gross** `actual_amount` (TOB-AAVE-4 pin in unit tests).

Victim finalize uses `PositionSides::Both` with `remove_if_empty: false`; empty/bad-debt cleanup is post-step (A026/A027).

---

## 8. Index / accrual sensitivity (Transfer-only residual)

Plan stamps asset units and fees from Cache indexes at planning time. Each pool withdraw leg re-syncs/accrues that market before `resolve_withdrawal`. Credit moves precomputed scaled shares and is immune (ADR-0019).

Consequences for Transfer:

- Intra-tx accrual on a collateral market can change shares burned for a given asset `amount` (partial ceil burn vs plan).
- If live `gross < protocol_fee` after resolve, `#115` aborts the **entire** tx (repay included) — fail-closed, not silent under-fee.
- Harness `liquidation_accrual_timing.rs` / `preaccrual_does_not_change_liquidator_payoff` pins that committing accrual before liquidate does not open an extractable payoff gap on the fixture.

Residual is economic sizing / rare liveness (`#115`), not double-pay.

---

## 9. Halt and pause interaction (INV-HALT-02)

Seize legs use `FreezePolicy::SeizureLeg`: reject `no_seize` only; tolerate `paused` / `frozen` (ADR-0008). Enforced in both plan and `apply_liquidation_seizures`. Global pause does **not** gate `liquidate` (`#[when_not_paused]` absent) — exits stay open.

---

## 10. Attack / bypass attempts

| Attempt | Outcome |
|---|---|
| Seize sized to planned repay while debt FOT under-delivers | Blocked — measure + `scale_seizures_to_received` |
| Pay liquidator gross including fee | Blocked — `withhold_liquidation_fee` subtracts before `transfer_out` |
| Mint fee revenue without retaining cash | Blocked — only `net` debited/transferred |
| Overdraw cash | Blocked — `require_reserves(net)`; Certora `withdraw_never_overdraws_cash` |
| Utilization brick at high util | Skipped intentionally for `is_liquidation` |
| Cash-empty market Transfer | `#112`; use Credit |
| Redirect payout via `to=pool/controller` | No separate `to`; would require auth as that address |
| Credit-back undo via Transfer | N/A — tokens leave account to wallet even on self-liq |
| Zero/dust fee leg with `fee == gross` | `net=0` transfer no-op; fee cash retained + revenue minted; liquidator earns 0 on that leg (incentive residual) |
| Post-scale `fee > amount` | Prevented by monotonic floor scale + plan clamp |
| Reenter via flash during liquidate | `require_not_flash_loaning` |
| Partial batch pay then fail | Single Soroban tx; any leg panic rolls back prior repay + withdraws |

---

## 11. Residuals (non-blocking)

1. **Outbound unmeasured (accepted).** Same as user withdraw; SAC listing assumption (A041, A055, threat-model Tamper.3).
2. **Cash liveness.** Transfer needs spare cash for net payout; Credit is the designed alternative (harness + ADR-0019).
3. **Index drift.** Documented Transfer vs Credit asymmetry; fail-closed on `WithdrawLessThanFee`.
4. **Dust fee bump.** `bumped_fee` can charge 1 unit when fair fee floors to 0 — cost to liquidator, bounded in `numeric-bounds.md`; not an over-seize of victim beyond plan clamp.
5. **Event/indexer gross vs net.** `LiqSeize` is gross by design (TOB-AAVE-4 lesson); liquidator proceeds = gross − fee. Unit-tested.
6. **A080 usage exit no-op** on missing spoke-usage row — cap distortion only (A026/A084); shares and cash still move.
7. **Fee product / BPS math** deferred to A053; this finding only requires withhold ≤ gross and mint↔cash identity.

---

## 12. Evidence map

| Claim | Anchor |
|---|---|
| Transfer arm calls seizures not share credit | `liquidation/mod.rs:88-91` |
| Fee carried into pool withdraw entry | `apply.rs:107-110` |
| Gross mutation / net transfer split | `pool/ops/withdraw.rs:28-39,55-61,110-131` |
| Cash gate on net; util skip | `pool/ops/withdraw.rs:96-104` |
| Revenue mint increases supplied | `interest.rs:61-68`, `shares.rs:35-39` |
| Measured repay scales seize | `apply.rs:52-67`, `math.rs:465-491` |
| Plan `fee ≤ amount` | `math.rs:52-57`, `:352` |
| LiqSeize gross vs wallet net | `tests/events.rs` `transfer_mode_seizure_delta_is_gross_of_the_protocol_fee` |
| Cash starve Transfer / Credit escape | harness `cash_starved_market_blocks_transfer_but_not_credit` |
| INV-ACCT-03 inbound-only on liquidate | `docs/reference/invariants.md` INV-ACCT-03 |

---

## 13. Opinion

The Transfer seize outflow is narrow and correctly ordered: measure debt → scale collateral → burn gross → mint fee against withheld cash → pay net under reserve/solvency gates. The dangerous dual (minting fee without withholding) is explicitly confined to this path and forbidden on Credit. Do not add controller-side receive measurement unless listing policy admits non-SAC collateral; do not weaken `require_reserves(net)` or the `gross ≥ fee` assert. **Defended** for T3 Transfer token outflow.
