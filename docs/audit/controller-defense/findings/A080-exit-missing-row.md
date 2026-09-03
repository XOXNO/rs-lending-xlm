# A080 — apply_exit no-op on missing usage row

- Agent: A080 (deep dive + reproduction)
- Theme: T5
- Severity: medium
- Status: partial
- Paths:
  - `contracts/controller/src/spoke_usage.rs` (`apply_exit` missing-row early return)
  - `contracts/controller/src/context/spoke.rs` (`apply_spoke_exit`)
  - Exit callers: withdraw / repay / liquidation fee exit / bad-debt cleanup
- Defense: Exit with zero delta returns early. Exit with **missing storage row** also returns without writing — intentional tolerance for legacy/migration / already-cleared rows. Pinned by unit `exit_without_usage_row_is_noop_and_does_not_persist` and Certora `usage_exit_without_usage_row_is_a_noop`.
- Gap: Cap enforcement reads **recorded usage**, not Σ live account positions. If a `SpokeUsage` row is absent while positions remain live, recorded occupancy under-counts → new entries admit from a zero baseline up to the full configured cap (**over-admission**). `apply_exit` does not heal that hole: a non-zero withdraw/repay against a missing row is a silent no-op and does not invent a row.
- Impact: Soft governance capacity only. Over-admission ≤ spoke `(supply|borrow)_cap` headroom relative to true live occupancy; no direct theft. Realized loss only if later defaults socialize into the over-filled market (≤ market TVL). Symmetric twin: **over-recorded** usage shrinks headroom (false cap hits / availability).
- Evidence: Reproduction tests below; A103 §4.1; A108 PIN catalog; INV-HALT-03 exit-safe rule set.
- Opinion: Confirmed reproducible. Keep the no-op pin until product ships a reconcile / usage↔Σ-positions invariant. Highest-value follow-up is an admin/keeper reconcile, not changing exit to invent zero rows.

---

## 1. Mechanism

```text
apply_exit(delta > 0):
  load_usage_row(hub)?
    None  → return          # A080: no-op, no insert
    Some  → usage -= delta  # panics if next < 0
```

`apply_entry` defaults a missing row to zero and then enforces `usage + delta ≤ scaled_cap`. So:

| State | Cap behavior |
|---|---|
| Healthy: usage ≈ Σ positions | Entries stop at configured cap |
| **Under-count:** row missing / too low, positions live | Entries see spare headroom → **over-admission** |
| **Over-count:** row higher than positions | Exits leave residual usage → **false rejects** |

Missing-row under-count is the A080 residual. Exit no-op prevents “healing” via organic withdraw/repay.

How a missing row can appear in production (not proven as a live bug today): legacy migration, storage prune races, or any future path that mutates positions without a matching usage write. The plant in tests is a durable `ControllerKey::SpokeUsage` delete while account maps stay intact — the same durable shape the no-op tolerates.

---

## 2. Reproduction (passing)

### 2.1 Unit — context layer

| Test | Shows |
|---|---|
| `exit_without_usage_row_is_noop_and_does_not_persist` | Missing row + exit → still `None` |
| `missing_usage_row_entry_admits_full_cap_from_zero_baseline` | Drop row → second full-cap `apply_entry` succeeds |
| `over_recorded_usage_survives_smaller_exit` | Exit smaller than booked usage leaves residual |
| `over_recorded_usage_residual_blocks_reentry_to_cap` | Residual blocks re-entry (`#311`) |

Commands:

```bash
cargo test -p controller --lib usage_row
cargo test -p controller --lib over_recorded_usage
```

### 2.2 Harness — end-to-end over-admission (PIN)

Plant: after Alice fills the spoke cap, delete the durable usage key; Alice’s supply/debt positions remain.

| Test | Result |
|---|---|
| `test_missing_usage_row_allows_supply_fill_to_full_cap_while_positions_live` | Healthy: Bob +1 reverts `#311`. After plant: Bob supplies **full cap** again while Alice still holds supply. Post-plant usage equals Bob only. |
| `test_missing_usage_row_withdraw_then_supply_fills_to_configured_cap` | After plant, Alice partial withdraw leaves usage **absent** (exit no-op). Bob then fills **full configured cap** while Alice residual remains → over-admission. |
| `test_missing_usage_row_repay_then_borrow_fills_to_configured_borrow_cap` | Borrow-side twin: plant → partial repay no-ops usage → Bob borrows full borrow cap while Alice residual debt remains. |

Command (requires `make build` WASMs):

```bash
make test-match PATTERN=missing_usage_row
# 3 passed
```

Numeric shape (supply, cap = 1000 USDC units): after plant + Bob full fill, live supply ≈ Alice residual + Bob 1000 while recorded usage ≈ 1000 (Bob only). Governance intended occupancy was ≤ 1000.

---

## 3. What this is not

- Not share mint / theft: pool books stay consistent with measured legs.
- Not automatic on healthy paths: ordinary supply→withdraw tracks usage (existing `spoke_caps` / Certora `usage_*_tracks_scaled_delta`).
- Not fixed by inventing a zero row on exit — that would fight the intentional INV-HALT-03 pin until a reconcile model exists.

---

## 4. Remediation pointers (A110 RB-06)

1. Keeper/assert: `recorded usage ≈ Σ scaled positions` per `(spoke, hub, side)`.
2. Permissioned rewrite path for planted holes.
3. Keep `usage_exit_without_usage_row_is_a_noop` until (1)+(2) change the product rule; then replace PIN + CLOSE twins in the same PR.
