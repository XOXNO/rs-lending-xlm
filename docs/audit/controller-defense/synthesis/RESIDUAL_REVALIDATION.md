# Residual revalidation (post–A080 challenge)

Re-checked every **leading residual** in `synthesis/FINAL.md` against live
code and Soroban persistent-storage semantics after the A080 archive challenge.

| Verdict | Meaning |
|---|---|
| **INVALID** | Not a live production hole under current code + storage model |
| **VALID — deploy/ops** | Real only if ownership / config / listing policy fails |
| **VALID — known design** | Documented intentional residual (threat-model / ADR); not a silent bug |
| **VALID — hygiene / footgun** | Low; fee DoS or future-merge checklist only |

---

## A080 — `apply_exit` missing-row no-op → **INVALID** as live issue

### Why it looked real

Unit/Certora/harness can **plant** `None` usage while positions exist. Exit
no-ops; entry admits from zero. That demonstrates the *tolerance*, not a
reachable production creator.

### Why it is not a live hole

1. **First positive supply/borrow always creates** the row
   (`apply_entry` → `persist` → `set_spoke_usage`).
2. **Persistent ≠ temporary.** Archived persistent keys are **restored** (or
   the tx fails `ENTRY_ARCHIVED`). They do **not** become `None`. A080’s
   `load_usage_row` → `None` path is “never written / pruned to both-zero,”
   not “TTL expired.”
3. **Money mutators update usage and positions together** on the same merge
   (`apply_leg_usage` + position write → one finalize). Idle TTL renew of
   *user* keys without touching usage does not wipe the usage key.

### What remains

Intentional carve-out for an absent key (INV-HALT-03 exit-safe / Certora pin).
Keep the no-op; do **not** rank A080 as P1 capacity risk. Status → **defended
(info)**. Artificial plant tests are PIN of the carve-out, not proof of a bug.

---

## Remaining FINAL residuals

### 1. A009 — Sensitive floor = 12 / owner = governance → **VALID — deploy/ops**

**Live code:** `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` with TEMPORARY
comment targeting `120_960` (`governance/src/constants.rs`). Controller has
no native delay; safety is ownership composition.

**Class:** Release blocker / deploy checklist — **not** a missing
`only_owner` in controller logic. Remains **P0** until restored + on-chain
owner attestation.

### 2. Swap-aggregator + XOXNO owners → **VALID — deploy/ops**

**Live code / docs:** Threat-model Known gaps: aggregator owner outside
governance by design; XOXNO owner is a price trust root.

**Class:** Trust-root / ops attestation. Remains **P0** as deploy gates.
Not a controller arithmetic bug.

### 3. A048 / A056 — controller `received > 0` only → **VALID — known design**

**Live code:**

```105:105:contracts/controller/src/strategies/swap.rs
    assert_with_error!(env, received > 0, StrategyError::NoSwapOutput);
```

Threat-model §“The controller does not bound slippage” states the same:
`total_min_out` lives in the opaque aggregator payload.

**Class:** Documented economic residual (account-local; HF-clipped if debted).
**Still the highest live-code extractable residual** if the router is hostile
or the caller embeds dust `min_out`. Remains **P0/P1 product** until
controller-enforced `min_out` ships — but it is **known**, not undiscovered.

### 4. A055 — non-SAC / lying tokens if listed → **VALID — known design / listing**

**Live code:** `transfer_amount_measured` credits balance deltas (FoT-safe under
SAC). Rebasing / balance-liars break cash↔share if governance lists them.

**Class:** Listing policy / ops gate, not a missing measure on SAC paths.
Keep as **P1 listing**; not a silent code defect under SAC-only markets.

### 5. A064 — `no_seize` ̸⇒ `frozen` → **VALID — known design (ADR-0008)**

**Live code:** `FreezePolicy::SeizureLeg` rejects `no_seize` only; entry still
allows supply unless `paused`/`frozen`. Guardian ratchet can set `no_seize`
independently (`set_spoke_asset_flags` / `require_flag_ratchet`).

**Class:** Intentional ADR-0008 / INV-HALT-02 split. Availability residual
(liquidation stranding) until Option C. Keep **P1 product**, not a bug.

### 6. A062 / A015 — uncapped mutator/keeper Vecs → **VALID — hygiene**

**Live code:** Views cap at `MAX_VIEW_INPUTS = 256`; mutator payment Vecs and
keeper lists have no hard length cap. Duplicates on money paths are
**aggregated** (by design), not double-credited.

**Class:** Fee/CPU DoS only. Keep **P2 hygiene**; not fund risk.

### 7. A094 — forgotten `put_market_index` → **VALID — footgun only**

**Live code:** Current merges (`merge_supply_leg`, `merge_withdraw_leg`,
`merge_debt_leg`) call `put_market_index`. A098: ledger time frozen in-tx;
overlay is sound today.

**Class:** Review checklist for **future** merges — not a live omitted put.
Demote to **P3 process**; remove from “leading live residuals.”

### 8. A085 / A108 — missing PIN/CLOSE tests → **VALID — evidence debt**

Does not create theft. Still useful for A048/A056/A064 once product changes.
**P3**.

---

## Recalibrated leading list

| Rank | Band | Residual | Verdict |
|---:|---|---|---|
| 1 | P0 | A009 Sensitive floor + owner=gov | VALID — deploy |
| 2 | P0 | Aggregator / XOXNO owners | VALID — deploy |
| 3 | P0 | A048/A056 no controller `min_out` | VALID — known design |
| 4 | P1 | A055 SAC-only listing | VALID — listing policy |
| 5 | P1 | A064 Option C (`no_seize`⇒freeze) | VALID — known design |
| — | ~~P1~~ | ~~A080 missing usage~~ | **INVALID** as live issue |
| 6 | P2 | A062/A015 Vec length caps | VALID — hygiene |
| 7 | P3 | A094 put-index checklist | VALID — footgun only |
| 8 | P3 | A108 evidence density | VALID — tests |

**No new Critical live-code hole** found among the FINAL leaders. After A080
drop, the actionable set is deploy trust roots + documented design residuals
(slippage, listing, ADR-0008) + low hygiene.
