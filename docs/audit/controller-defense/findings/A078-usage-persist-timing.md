# A078 — Spoke usage persist timing vs pool mutation success

- Agent: A078
- Theme: T5
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/positions/mod.rs:112-141,238-252` (`apply_leg_usage`, `finalize_position_flow`)
  - `contracts/controller/src/context/spoke.rs:103-143` (`apply_spoke_entry` / `apply_spoke_exit` / `persist_spoke_usage`)
  - `contracts/controller/src/spoke_usage.rs:77-141` (`SpokeUsageContext::persist` / `apply_entry` / `apply_exit`)
  - `contracts/controller/src/positions/supply.rs:126-155,283-301,308-417` (settle → merge → finalize)
  - `contracts/controller/src/positions/debt.rs:128-211,248-297` (settle / repay / borrow_into_controller)
  - `contracts/controller/src/strategies/mod.rs:68-79` (`strategy_finalize`)
  - `contracts/controller/src/strategies/legs.rs:154-217` (`net_settle_collateral_against_debt`)
  - `contracts/controller/src/positions/liquidation/mod.rs:75-150` (liq finalize ordering)
  - `contracts/controller/src/positions/liquidation/apply.rs:139-220` (Credit fee exit before pool seize)
  - `contracts/controller/src/positions/liquidation/bad_debt.rs:15-61` (exit before seize; direct `persist_spoke_usage`)
  - `contracts/controller/src/storage/spoke.rs:56-78` (`set_spoke_usage` prune-on-zero)
- Defense: On every ordinary and strategy account path, durable `SpokeUsage` writes occur only in `finalize_position_flow` **after** the pool mutation has returned and in-memory `apply_leg_usage` has buffered the scaled delta. Cap checks run at apply-entry time (post-pool, using pool-returned indexes — A077/A082). Post-pool solvency (where required) runs **before** finalize. Soroban transaction atomicity rolls back pool + controller together if any later step panics. Bad-debt / Credit-fee paths may buffer exits **before** `pool_seize_positions_call`, but still call `persist_spoke_usage` only **after** that call returns.
- Gap: none that leave durable usage ahead of a failed pool mutation. Residuals (not novel critical): (1) INV-RISK-01 prose says caps are checked “before the pool action,” but spoke-usage caps are enforced **after** pool success in `apply_entry` — safe under atomicity, wording imprecise. (2) Liquidation Credit finalize can rewrite the same usage map twice in one tx (victim + receiver `finalize_position_flow`); bad-debt after liq can persist a third time — idempotent amplification, not a desync. (3) Cap/solvency failures after a successful pool call waste the tx (pool work discarded) — accepted DoS/fee cost, not inconsistent books.
- Impact: Successful txs never commit controller spoke usage without a completed pool leg that produced the delta (or an intentional fee/bad-debt seize that completed). Failed txs never leave half-applied usage. Blast radius of a hypothetical “persist before pool” bug would be false cap occupancy or under-counted capacity across an entire spoke market; current ordering prevents that class.
- Evidence: INV-HALT-03, INV-RISK-01 (post-pool gates), INV-STOR-01; peers A032, A033, A076, A077, A080, A082, A084, A022–A025, A027; Certora `usage_supply_tracks_scaled_delta`, `usage_withdraw_tracks_scaled_delta`; unit `contracts/controller/tests/spoke.rs`; harness `tests/test-harness/tests/controller/spoke_caps.rs`.
- Opinion: Persist-after-pool-success is the load-bearing T5 ordering invariant. Keep `finalize_position_flow` as the sole ordinary durability chokepoint; keep bad-debt’s direct persist **after** `pool_seize_positions_call`. Do not move `persist_spoke_usage` earlier to “save a revert after cap” — that would create the inconsistency this design avoids.

---

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git).
2. Inventory every production caller of `persist_spoke_usage` / `SpokeUsageContext::persist` / `finalize_position_flow`.
3. For each monetary verb and strategy tail, order: pre-pool gates → pool FFI → in-memory usage apply → post-pool risk → durable persist.
4. Special-case paths that apply usage **before** a pool call (bad debt, Credit fee) and confirm persist still trails pool success.
5. Cross-check peers A032/A033 (finalize batch + event order), A076/A077/A080/A082/A084 (usage semantics), A022–A025/A027 (storage write sets), and INV-HALT-03 / INV-RISK-01.

Out of scope as primary claims: missing-row exit no-op (A080), cap index choice (A077/A081), Credit fee-only accounting intent (A084), event vs storage order beyond noting usage is first in finalize (A033).

---

## Production persist call sites (complete)

| Site | When | Preceding pool? |
|---|---|---|
| `finalize_position_flow` → `cache.persist_spoke_usage()` | Tail of supply / withdraw / borrow / repay / strategies / liquidation account batches | Yes — all pool legs for that flow already returned (or no usage context → no-op) |
| `execute_bad_debt_cleanup` → `cache.persist_spoke_usage()` | After `pool_seize_positions_call` | Yes — seize completed; usage exits were buffered **before** the call |

No other production path writes `SpokeUsage` keys. Certora/unit tests call `persist` directly for harness isolation only.

`Cache::persist_spoke_usage` is a no-op when `spoke_usage` was never loaded (no apply this invocation). `SpokeUsageContext::persist` writes **every** row in the in-memory map via `storage::set_spoke_usage` (zero both sides → key remove).

---

## Canonical ordering (ordinary verbs)

```
pre-pool gates (auth, pause/flash, listing, flags, position limits, measured transfers)
    → pool_*_call  (cross-contract mutation SUCCESS or panic)
    → for_each_leg / merge_*_leg
         → apply_leg_usage → apply_spoke_entry|exit   [RAM only; entry enforces cap]
         → put_market_index / position map / event buffers
    → enforce_post_pool_solvency?  (borrow, withdraw; strategies via strategy_finalize)
    → finalize_position_flow
         1. persist_spoke_usage          ← first durable controller usage write
         2. persist_account_positions
         3. emit_position_batch
```

### Per-verb detail

#### Supply (`process_supply`)

1. Entry gates (`validate_position_entry_gates` / `require_can_supply`) — **no** spoke-usage cap yet.
2. `transfer_amount_measured` → `pool_supply_call`.
3. `merge_supply_leg` → `apply_leg_usage(Entry)` → `apply_spoke_entry` (cap vs pool index + decimals).
4. `finalize_position_flow(..., Supply, false)` — persist usage then supply map.

If step 3 panics on cap after step 2 succeeded: whole tx reverts; pool supply and usage write never commit.

#### Borrow (`process_borrow`)

1. Entry gates including `require_can_borrow`.
2. `pool_borrow_call` → `merge_debt_leg(Entry)` → usage buffer + cap.
3. `enforce_post_pool_solvency` (restamp + HF/LTV/min collateral).
4. `finalize_position_flow` (Debt or Both if restamped).

Usage is buffered before solvency; durable only after solvency passes. Solvency failure after pool → full rollback (including buffered usage never written).

#### Withdraw (`process_withdraw`)

1. Exit flags → `pool_withdraw_call` → `merge_withdraw_leg` → `apply_leg_usage(Exit)`.
2. `enforce_post_pool_solvency`.
3. `finalize_position_flow(..., Supply, true)`.

Exit has no cap check; underflow/`next >= 0` can still panic post-pool and roll back.

#### Repay (`process_repay`)

1. Measured transfer → `pool_repay_call` → `merge_debt_leg(Exit)`.
2. `finalize_position_flow(..., Debt, false)` — **no** post-pool solvency (risk-reducing).

Same persist-after-pool pattern.

---

## Strategy paths

Strategies accumulate many pool legs (borrow-into-controller, withdraw, repay, net_settle, deposits) with **in-memory** usage only. A single `strategy_finalize`:

1. `restamp_listed_supply_ltv`
2. `require_post_pool_risk_gates`
3. `finalize_position_flow(..., Both, true)` → one usage persist for the whole batch

`net_settle_collateral_against_debt`: `pool_net_settle_call` then `merge_withdraw_leg` + `merge_debt_leg` (both exits) — still no durable usage until strategy finalize.

`flash_loan`: pool-only; never opens usage / finalize (A008/A032 class).

Mid-leg durable account/usage writes are intentionally absent (A032). Pool legs that succeed before a later strategy panic still roll back with the tx.

---

## Liquidation ordering

```
apply_liquidation_repayments     → pool repay → merge_debt Exit (usage RAM)
apply_liquidation_seizures       → pool withdraw → merge_withdraw Exit (usage RAM)
  OR apply_liquidation_share_credit:
       for each leg:
         debit/credit account maps (RAM)
         if fee > 0: apply_spoke_exit(fee)     ← usage RAM BEFORE pool
       pool_seize_positions_call(fee_entries) ← pool SUCCESS for fees
LiquidationEvent.publish
finalize_position_flow(victim, Both, false)   ← persist usage (#1)
[Credit] record_share_credit_updates
         finalize_position_flow(receiver, Supply, false)  ← persist usage (#2, same Cache map)
check_bad_debt_after_liquidation
  → maybe execute_bad_debt_cleanup
```

### Credit fee vs pool

Fee usage exit is applied in RAM **before** `pool_seize_positions_call`. Durable write still waits for finalize after seize returns. If seize panics, fee exit never persists. Intentional: fee is the only net spoke-usage delta; account↔account share move cancels (A084).

### Bad debt (`execute_bad_debt_cleanup`)

```
for each supply/debt position: apply_spoke_exit(full scaled)   ← RAM first
pool_seize_positions_call(all entries)
persist_spoke_usage()                                         ← durable after seize
CleanBadDebtEvent → remove_account_and_burn_nft
```

Does **not** use `finalize_position_flow` (no position-batch emit; positions deleted). Persist still trails pool success.

When cleanup runs after liquidation finalize: usage was already persisted for post-liq residuals; cleanup applies further exits and persists again, then deletes account keys. Same-tx amplification; final durable usage reflects seized residual (A027).

---

## Layered timeline (what is safe when)

| Layer | State | Survives panic later in same tx? |
|---|---|---|
| Pre-pool gates | Reads + auth | N/A (no mutation) |
| Token transfers into pool | Token balances | No — tx abort restores |
| Pool mutation return | Pool persistent | No — abort restores |
| `apply_spoke_*` / Account maps / event buffers | Controller RAM / Cache | Never durable alone |
| Cap assert in `apply_entry` | Panic → abort | Pool+tokens rolled back |
| Post-pool solvency | Panic → abort | Same |
| `persist_spoke_usage` | Controller `SpokeUsage` keys | Only if tx commits |
| `persist_account_positions` / emit | Positions + events | After usage in finalize (A033) |

**Invariant (A078):** There is no committed controller `SpokeUsage` write that corresponds to a pool mutation that did not succeed in the same committed transaction.

---

## INV-RISK-01 wording vs implementation

`docs/reference/invariants.md` INV-RISK-01: “Caps and listing rules are checked **before** the pool action.”

Observed:

| Check | Timing |
|---|---|
| Listing / halt / collateralizable / borrowable | Before pool (`require_can_*`, `FreezePolicy`) |
| Position-count limits | Before pool |
| **Spoke supply/borrow usage caps** (`enforce_spoke_cap`) | **After** pool, in `apply_spoke_entry` |

Post-pool cap is deliberate: delta and index come from `PoolPositionMutation` (A077, A082). Atomicity makes “pool then cap fail” safe for books. The invariant sentence over-compresses listing vs usage-cap timing — documentation residual only, not an undefended gap.

INV-HALT-03 (“Entry paths enforce usage at the live index”) matches post-pool live indexes from the mutation.

---

## Double / triple persist (idempotency)

1. Credit liquidation: victim finalize then receiver finalize both call `persist_spoke_usage` on the **same** `Cache.spoke_usage` map (same spoke). Second write repeats the same rows after fee exit already applied — no double-count of deltas.
2. Auto bad-debt after liq: additional exits then third persist.
3. `persist` iterates the whole cached map each time — O(touched hubs), not O(legs × accounts) wrong accumulation.

Risk if someone inserted `reset_spoke_context` between applies without persisting: later persist would drop buffered rows. Current liq/strategy code does not reset mid-flow while unpaid usage sits in Cache (A084). Footgun for future multi-spoke work, not a present bug.

---

## Failure-mode matrix

| Failure point | Pool already mutated? | Usage durable? | Outcome |
|---|---|---|---|
| Cap panic in `apply_entry` after supply/borrow pool | Yes (in-tx) | No | Full abort |
| Solvency panic after borrow/withdraw/strategy pool | Yes (in-tx) | No | Full abort |
| Exit underflow / missing assert in `apply_exit` | Yes (in-tx) | No | Full abort |
| `pool_*` panics | No successful mutation | No | Abort |
| Bad-debt seize panics after RAM exits | No durable seize | No persist reached | Abort |
| Credit fee seize panics after RAM fee exit | No | Finalize not reached | Abort |
| Panic after `persist_spoke_usage` but before tx end | Yes | Would-be yes | Abort restores usage + pool |
| Happy path commit | Yes | Yes, matching deltas | Consistent |

---

## What would be undefended (not observed)

- Calling `persist_spoke_usage` / `set_spoke_usage` **before** `pool_*_call` returns on a path that still expects that call to define the delta.
- Persisting usage in a merge helper mid-leg (would force mid-strategy durability and complicate rollback assumptions beyond host atomicity narratives).
- Skipping finalize after pool success on an entrypoint that applied usage (would desync on commit — ordinary verbs and `strategy_finalize` always finalize).

None of these appear in production call graphs audited above.

---

## Cross-links

| Peer | Relation |
|---|---|
| A032 | Strategy single finalize after all pool legs — agrees |
| A033 | Finalize order usage → positions → events — agrees; this file focuses on usage vs **pool** |
| A076 | Apply semantics; persist writes every touched row — agrees |
| A077 / A082 | Cap/delta from pool outputs — explains why apply is post-pool |
| A080 | Exit missing-row no-op — orthogonal under-accounting, not timing |
| A084 | Credit fee-only exit + double finalize — agrees on idempotent re-persist |
| A022–A025 | Verb storage maps — same finalize-after-pool story |
| A027 | Bad-debt persist-after-seize + write-then-delete — agrees |

---

## Verdict

**Defended.** Spoke usage durability is strictly after pool mutation success on every production path. In-memory apply may precede seize on bad-debt and Credit-fee legs, but `persist_spoke_usage` does not. Cap and solvency gates that run between pool success and persist rely on Soroban all-or-nothing commit; they cannot leave usage written without the matching pool state. Residual: tighten INV-RISK-01 language to distinguish pre-pool listing from post-pool usage caps; treat multi-persist on liquidation/bad-debt as intentional amplification, not a timing flaw.
