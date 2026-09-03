# A103 — Spoke-usage gap synthesis (A076–A085)

- Agent: A103 (synthesis)
- Theme: T5 → T7 gap hunt
- Severity: medium
- Status: partial (portfolio: one medium residual; core paths defended)
- Paths: synthesis over `spoke_usage.rs`, `context/spoke.rs`, `positions/{mod,supply,debt,liquidation}/`, `strategies/`; primary evidence in peer findings A076–A085 (where present)
- Defense: see §3 defended surfaces
- Gap: see §4 residual portfolio; **leading residual A080**
- Impact: see §5 quantified blast radii — no direct theft from spoke-usage alone; capacity distortion up to per-spoke `supply_cap` / `borrow_cap` headroom
- Evidence: peers A076–A078, A080, A082, A084; supporting A022–A028, A032–A033, A041, A052–A053, A072, A086, A094; INV-HALT-03, INV-RISK-01, ADR-0015; Certora `usage_*` / `usage_liq_*` suite; PRELIMINARY.md leading-residuals table
- Opinion: T5’s load-bearing defenses (pool-truth deltas, post-pool cap, persist-after-pool, one-spoke Cache pin) hold. The only medium-severity residual in the wave is **A080** (exit no-op on missing usage row). Persist/timing (**A078**) is defended and must stay that way. Missing peer files A079/A081/A083/A085 leave coverage holes, not proven new criticals — provisional inferences from peers are below.

---

## 1. Mission and method

Synthesize spoke-usage gaps from wave-5 findings **A076–A085 that exist**, quantify impact, cross-link IDs, and emphasize **A080** plus **persist/timing (A078)**.

Method:

1. Read `shared/COORDINATION.md`, `SEED.md`, `synthesis/PRELIMINARY.md`, and every present A076–A085 finding.
2. Inventory wave-5 coverage vs `AGENT_MANIFEST.md` scopes.
3. Collapse peer claims into defended vs residual portfolios with blast-radius quantification.
4. Cross-link storage/liq/cache peers that load-bear on usage (A022–A028, A032–A033, A052–A053, A086, A094).
5. Flag missing A079/A081/A083/A085 as **coverage debt**, with provisional status only where peers already constrain the claim.

No production Rust edited. No git operations (coordination protocol).

---

## 2. Wave-5 coverage inventory

| ID | Manifest scope | Finding file | Headline status |
|---|---|---|---|
| A076 | `SpokeUsageContext` apply_entry/exit semantics | **present** | defended (info) |
| A077 | Cap enforcement using pool output indexes | **present** | defended (info) |
| A078 | Persist timing vs pool mutation success | **present** | defended (info) — **timing pillar** |
| A079 | Multi-asset batch usage aggregation | **absent** | coverage debt (§7.1) |
| A080 | Exit no-op on missing row | **present** | **partial (medium)** — **leading residual** |
| A081 | Supply vs borrow index for scaled caps | **absent** | coverage debt (§7.2); A077 partially covers |
| A082 | Usage from pool returns, not caller inputs | **present** | defended (info) |
| A083 | Cross-spoke isolation of usage maps | **absent** | coverage debt (§7.3); A028/A076 pin isolation |
| A084 | Liq / strategy skip or double-count | **present** | defended (low) — intentional fee-only exit |
| A085 | Tests and Certora covering spoke usage | **absent** | coverage debt (§7.4); peers cite extensive `usage_*` rules |

**Present: 6/10. Absent: 4/10.** Synthesis therefore rests on A076–A078, A080, A082, A084, plus PRELIMINARY’s ranking of A080 as a leading residual across the whole audit.

---

## 3. Defended surfaces (what holds)

### 3.1 Semantics and units — A076

- Lazy row load; absent row on **entry** starts at zero and is written.
- Entry: `delta = new_scaled − old_scaled` (via `apply_leg_usage`), cap via `calculate_scaled_cap(index, decimals)`.
- Exit: subtract; panic on math overflow or `next < 0`; zero delta early-return.
- `persist()` writes every touched in-memory row (zero both sides → storage prune).
- Cache isolates **one spoke per invocation** (`SpokeMismatch` without `reset_spoke_context`).

**Impact if broken:** wrong RAY occupancy → false cap hits or silent over-admission. Current design ties deltas to pool scaled positions, not user token amounts.

### 3.2 Cap index trust boundary — A077 (+ A082, A094)

- Cap check uses the **leg’s** `PoolPositionMutation.market_index` (supply index for supply side, borrow index for borrow side via `UsageSide::index`) and pool-reported decimals — not caller-requested amounts, not a pre-call stale index for the write path.
- Pre-pool / view bulk indexes may differ from post-mutation indexes; that can cause **false rejects on views**, not under-capped durable writes (A077, A094).

**Impact if broken:** using request amounts or wrong-side index would systematically under/over-count scaled usage relative to live share math. Current boundary is correct.

### 3.3 Persist-after-pool-success — A078 (emphasized)

Canonical ordinary ordering:

```
pre-pool gates → pool_* success → apply_leg_usage (RAM; entry enforces cap)
  → post-pool solvency? → finalize_position_flow
       1. persist_spoke_usage
       2. persist_account_positions
       3. emit_position_batch
```

Hard claims from A078:

| Claim | Status |
|---|---|
| No durable `SpokeUsage` write ahead of the pool mutation that defines the delta | **holds** |
| Cap / solvency panic after pool → full tx abort; usage never commits alone | **holds** |
| Bad-debt / Credit-fee: RAM `apply_spoke_exit` may precede seize; `persist_spoke_usage` still trails seize success | **holds** |
| Only production writers: `finalize_position_flow` and `execute_bad_debt_cleanup` (post-seize) | **holds** |
| Multi-persist on Credit liq (+ optional bad-debt) is idempotent re-write of same Cache map | **holds** |

**Invariant (A078):** There is no committed controller `SpokeUsage` write that corresponds to a pool mutation that did not succeed in the same committed transaction.

Residual on A078 is **documentation only**: INV-RISK-01 says caps are checked “before the pool action,” but spoke-usage caps run **after** pool success in `apply_entry`. Safe under Soroban atomicity; wording over-compresses listing vs usage-cap timing. INV-HALT-03 (“live index”) matches the post-pool design.

**Impact if persist were moved earlier:** false cap occupancy or under-counted capacity across an entire spoke market after a failed pool leg — the exact inconsistency class A078 says current ordering prevents. **Do not “optimize” by persisting before pool to save a revert.**

### 3.4 Pool outputs / measured custody — A082

- Scaled usage = pool `new_scaled − old_scaled`.
- Custody legs use `transfer_amount_measured` / balance deltas and equality asserts vs pool `actual_amount` on critical paths (strategy borrow-into-controller, etc.).

**Impact if broken:** fee-on-transfer / lying tokens desync cash from accounting (also A041, A055). Measurement + asserts close custody desync for those legs; usage itself still follows scaled shares.

### 3.5 Liquidation / strategy accounting intent — A084

- Credit seize: account↔account share move cancels on same spoke/asset; **only protocol fee** is `apply_spoke_exit`’d so liquidations are not blocked by supply cap.
- Transfer seize: full withdraw-batch usage via normal merge.
- Strategies: buffer all legs; single `strategy_finalize` → one usage persist.
- Double finalize (victim then receiver) re-persists the same Cache usage map — not double-count of deltas.

**Impact if broken incorrectly:** double-count would **tighten** caps (availability DoS), not enable theft. Fee-only exit is intentional **net** under-count of gross seized shares vs protocol fee — by design under ADR-0019 / INV-LIQ-*.

---

## 4. Residual portfolio (gaps)

### 4.1 A080 — Exit no-op on missing usage row ★ leading residual

| Field | Value |
|---|---|
| Severity | **medium** |
| Status | **partial** |
| Code | `spoke_usage.rs` `apply_exit`: missing storage row → return without write |
| Intent | Tolerance for legacy/migration / already-cleared rows; also pinned by Certora `usage_exit_without_usage_row_is_a_noop` (INV-HALT-03 exit-safe) |

**Mechanics**

1. Positions exist (or existed) but **no** `SpokeUsage` row for `(spoke_id, hub_asset)`.
2. Exit path computes a non-zero scaled delta from the pool/account merge and calls `apply_spoke_exit`.
3. `load_usage_row` returns `None` → **no-op**. Storage stays absent (treated as zero usage).
4. Cap headroom is therefore **overstated** relative to live positions that never contributed to (or were never recorded in) usage.
5. Subsequent **entries** can admit up to the configured `supply_cap` / `borrow_cap` as if those positions did not occupy the spoke.

**Symmetric / related distortions (A080 + A028)**

| Distortion | Cause | Effect on caps |
|---|---|---|
| Under-counted usage (row missing while positions live) | A080 no-op on exit; never-recorded entry history | **Over-admission** (soft cap bypass) |
| Over-counted usage (row > sum of live positions) | Missed exits historically; orphaned usage after account TTL / incomplete cleanup | **False cap hits** (availability) |
| Zombie usage with no admin reconcile | A028: no admin write path to recompute usage from positions | Distortion persists until exits drive row toward truth or TTL death |

**Compounding with A078:** A080 is **orthogonal to timing**. Persist-after-pool is correct; the bug class is **semantic under-decrement**, not premature durability. A078 explicitly scopes A080 out as under-accounting, not a timing flaw.

**Compounding with A084:** Credit fee-only exit and bad-debt full exits both call `apply_spoke_exit`. If the fee/residual row is missing, fee/bad-debt exits silently skip — capacity stays overstated after protocol takes fee shares or wipes an account. Peers A027/A052/A053 note A080 on these paths.

**Compounding with A076:** Entry creates rows from zero; healthy entry→exit cycles that always had a row are fine. A080 bites when history and storage diverge (migration, bug, or path that never entered through `apply_entry`).

### 4.2 A078 residuals (non-critical, timing-adjacent)

| Residual | Severity | Nature |
|---|---|---|
| INV-RISK-01 prose vs post-pool usage-cap enforcement | info / docs | Wording imprecise; books remain consistent under atomicity |
| Double/triple persist on Credit liq + bad-debt | info | Idempotent amplification; not desync |
| Cap/solvency fail after successful pool call wastes tx fees | info | Accepted fee-DoS; not inconsistent state |
| Future `reset_spoke_context` mid-flow while unpaid usage sits in Cache | footgun | Would drop buffered rows on later persist — not observed in current liq/strategy code (A078/A084) |

None of these are novel critical gaps. The **timing defense must not regress**.

### 4.3 A084 intentional “under-count” (not a bug)

Fee-only Credit usage exit under-counts **gross** seized shares vs net receiver credit. That is deliberate so liquidations are not blocked by supply cap. Do not “fix” by applying full seize as usage exit without also modeling the credit — that would double-exit or block liquidations (A053/A084).

### 4.4 Footguns adjacent to T5 (outside A076–A085 but load-bearing)

| Peer | Relation to spoke usage |
|---|---|
| A094 | Forgotten `put_market_index` on a new pool merge → wrong cap index / risk USD in-tx |
| A086 / A091 | Spoke usage embedded in Cache lifecycle; isolation + reset rules |
| A032 / A033 | Single finalize after strategy legs; usage persist before position persist / events |
| A055 / A041 | Lying / rebasing tokens — cash vs scaled; usage follows scaled, not cash |

---

## 5. Impact quantification

Spoke caps are **soft governance limits** (A080, A028, PRELIMINARY): they bound admissions per `(spoke, hub_asset, side)`, not HF/LTV solvency math. Distortion does not by itself mint tokens or bypass post-pool risk gates.

### 5.1 Max over-admission (A080 under-count)

Let:

- `C_s`, `C_b` = configured `supply_cap` / `borrow_cap` in **asset units** for the spoke asset.
- `U_s`, `U_b` = durable spoke usage in **RAY scaled** (or 0 if row absent).
- `P_s`, `P_b` = sum of live account scaled positions for that spoke market (conceptually).

When `U ≪ P` (including `U = 0` with `P > 0`):

| Bound | Meaning |
|---|---|
| **≤ one spoke asset’s remaining configured cap headroom** | New entries can fill from recorded `U` up to `C` at the live index, ignoring unrecorded `P` |
| **Not** protocol-wide TVL | Caps are per spoke × hub-asset × side |
| **Not** direct theft | Over-admission increases concentration / utilization risk for **suppliers in that market** only if later loans go bad — same class as PRELIMINARY’s A080 line |
| Temporary until reconcile | Further correct entry/exit cycles that touch a live row move `U` toward truth; missing-row exits never heal by themselves |

Worst case narrative (governance, not exploit primitive):

1. Historical under-recording leaves `U = 0` while substantial `P` exists.
2. Attackers / organic demand supply or borrow up to full `C_s` / `C_b` again.
3. True economic exposure ≈ prior unrecorded positions **plus** new cap fill.
4. Loss if those loans default is socialized to that market’s suppliers (≤ market TVL for that asset) — **indirect**, requires separate insolvency; usage gap only removed the soft brake.

### 5.2 Max false rejection (over-count)

If `U > P` (orphaned usage):

| Bound | Meaning |
|---|---|
| Entries fail with `SpokeSupplyCapReached` / `SpokeBorrowCapReached` while true occupancy is lower | **Availability / liveness** for that spoke market |
| Liquidations (Credit) still try fee-only exit | May no-op under A080 if row missing; if row **over**-full, exits help heal |
| No fund seizure | Users cannot enter; exits still allowed (INV-HALT-03 exit-safe) |

### 5.3 Persist-timing failure class (not observed — A078)

Hypothetical pre-pool persist blast radius (prevented today):

| Scenario | Blast radius |
|---|---|
| Usage written, pool leg reverts | False occupancy → false cap DoS across spoke market **or** under-count if reverted write somehow asymmetric — host atomicity makes partial commit impossible **unless** design splits durability across txs |
| Usage skipped after pool success | Under-count → same over-admission class as A080, but for **every** successful leg on that path |

Current code prevents both on all inventoried production paths.

### 5.4 Comparison to PRELIMINARY leading residuals

| ID | Issue | Quantified impact (PRELIMINARY + this synthesis) |
|---|---|---|
| **A080** | `apply_exit` no-op if usage row missing | Spoke caps under-count → temporary over-admission up to that spoke’s cap headroom; no direct theft; supplier risk only if over-admission later goes bad |
| A078 | Persist timing | **Defended** — not a residual gap; listed here as the timing pillar that must not regress |
| A055 | Non-SAC / rebasing if listed | Market-wide desync → bad debt ≤ market TVL (orthogonal token trust) |
| A094 | Forgotten `put_market_index` | Wrong HF/caps within a tx — footgun for new code |

A080 remains the **highest T5 residual** and a top audit-wide residual in PRELIMINARY’s table.

---

## 6. Cross-link matrix

### 6.1 Wave-5 peer graph (present findings)

```
A076 semantics ──┬── A077 cap indexes ── A082 pool outputs
                 │         │
                 │         └── A094 put_market_index footgun
                 │
                 ├── A078 persist-after-pool ★ timing
                 │         │
                 │         ├── A032 strategy single finalize
                 │         ├── A033 usage → positions → events
                 │         └── A022–A025 / A027 storage write sets
                 │
                 ├── A080 missing-row exit ★ residual
                 │         │
                 │         ├── A024 / A025 repay-withdraw exit paths
                 │         ├── A027 / A084 bad-debt exits
                 │         ├── A052 / A053 Credit fee exit
                 │         └── A028 zombie usage / no reconcile admin
                 │
                 └── A084 liq/strategy fee-only + double finalize
```

### 6.2 Claim agreement table

| Claim | Primary | Agrees |
|---|---|---|
| Deltas from pool scaled, not request amounts | A076, A082 | A022–A025, A077 |
| Cap uses mutation index + side-correct index | A077 | A076 (`UsageSide::index`), A094 |
| Persist only after pool success | A078 | A032, A033, A022–A025, A027, A084 |
| Missing-row exit is intentional no-op with capacity risk | A080 | A024, A025, A027, A028, A052, A076, A078 |
| Credit fee-only usage exit is intentional | A084 | A052, A053, A026, A078 |
| One spoke usage context per Cache invocation | A076 | A028 (key domain), keepers `reset_spoke_context` |

### 6.3 No disagreement file required

Present peers are consistent: A080 is the named residual; A078 does not contradict A080; A084’s fee-only path is intentional under-count of gross seize, not the same as A080’s missing-row under-count (different mechanism, same capacity-direction when fee exit no-ops).

---

## 7. Coverage debt (absent A079 / A081 / A083 / A085)

These scopes lack dedicated finding files. Synthesis records **provisional** status from peer evidence only — not a substitute for the missing deep-dives.

### 7.1 A079 — Multi-asset batch aggregation (absent)

**Provisional:** `for_each_leg` asserts `entries.len() == results.len()` then zips; each leg calls `apply_leg_usage` independently into one `SpokeUsageContext` map keyed by `HubAssetKey`. Aggregation is **per-asset row**, not a single scalar — multi-asset batches accumulate correctly if each leg’s pool result pairs correctly (A077 length equality). Residual risk would be a path that applies usage once for a batch total or reuses the wrong hub key — not reported by present peers. **Recommend completing A079** before closing wave-5.

### 7.2 A081 — Supply vs borrow index selection (absent)

**Provisional:** A077 + `UsageSide::index` already select supply index for supply side and borrow index for borrow side from the same `MarketIndexRaw`. Cap scaling therefore uses the matching RAY index for that side’s shares. A078 notes A081 as out-of-scope sibling. **Unlikely novel critical** if A077 holds; still want an explicit A081 file pinning both sides and zero-cap behavior (INV-HALT-03).

### 7.3 A083 — Cross-spoke isolation (absent)

**Provisional:** Storage keys are `(spoke_id, HubAssetKey)` (A028). Cache `ensure_spoke_context` panics `SpokeMismatch` on spoke change without `reset_spoke_context` (A076). Liquidation apply asserts spoke match. Accounts bind a single `spoke_id`. **Isolation looks defended**; residual is operational (multi-spoke future + mid-flow reset footgun from A078). Dedicated A083 should confirm no entrypoint can attribute usage to the wrong spoke_id.

### 7.4 A085 — Tests and Certora coverage (absent)

**Provisional inventory from peers (not exhaustive):**

| Area | Evidence cited by peers |
|---|---|
| Entry/exit track scaled delta | `usage_supply_tracks_scaled_delta`, `usage_withdraw_*`, `usage_borrow_*`, `usage_repay_*` |
| Exit without row no-op | `usage_exit_without_usage_row_is_a_noop` (documents A080 behavior as specified) |
| Strategy legs / net settle | `usage_strategy_*`, `usage_strategy_net_settle_tracks_scaled_delta` |
| Liq Transfer / Credit / bad debt | `usage_liq_transfer_*`, `usage_liq_credit_*`, `usage_liq_bad_debt_cleanup_*` |
| Unit / harness | `contracts/controller/tests/spoke.rs`, `tests/test-harness/.../spoke_caps.rs`, liq seize mode suites |

**Gap for A085 to own:** map rules → properties that are **unproven** — especially global `Σ positions ≈ usage` per spoke asset (A080’s recommended invariant), admin reconcile, and multi-asset batch aggregation (A079). Certora currently seeds usage with assumptions (`usage >= position scaled`) and proves **delta tracking**, which does **not** prove initial usage equals sum of positions — consistent with A080 remaining open.

---

## 8. Prioritized remediation backlog (spoke-usage only)

| Priority | Action | Addresses | Impact if done |
|---|---|---|---|
| P0 | Keep persist-after-pool; reject PRs that call `persist_spoke_usage` / `set_spoke_usage` before defining pool success | A078 regression | Prevents market-wide false occupancy / under-count |
| P1 | Invariant or keeper: recompute / assert spoke usage vs Σ account scaled positions per `(spoke, hub, side)` | **A080** | Detects under/over-count; bounds over-admission |
| P1 | Admin or permissioned reconcile tool to rewrite usage from positions (today: none — A028) | A080, A028 | Heals zombie / missing rows without waiting for TTL |
| P2 | Tighten INV-RISK-01 language: listing/halt pre-pool; usage caps post-pool at live mutation index | A078 docs | Removes false “pre-pool cap” reading |
| P2 | Regression: cap-at-limit Credit liquidation (fee-only exit); missing-row exit still no-op but documented | A084, A080 | Prevents “helpful” double-exit fixes |
| P2 | Checklist: every new pool merge → `apply_leg_usage` + `put_market_index` | A094, A077 | Stops stale-index cap footgun |
| P3 | Complete missing findings A079, A081, A083, A085 | coverage debt | Close wave-5 evidence holes |
| P3 | Certora/global rule: usage ↔ position totals (beyond per-leg delta) | A080, A085 | Turns soft tolerance into checked invariant where intended |

---

## 9. Verdict

**Spoke-usage core is defended** (A076, A077, A078, A082, A084): pool-truth scaled deltas, side-correct live indexes at cap check, persist strictly after pool success, measured custody where required, and intentional Credit fee-only exits.

**The wave’s material gap is A080:** missing-row `apply_exit` no-op can leave usage below live positions and allow **temporary over-admission up to that spoke asset’s configured cap headroom**. No direct theft; soft-governance capacity integrity only; supplier risk is contingent on later bad debt in the over-filled market.

**Persist/timing (A078) is not a gap** — it is the ordering invariant that prevents a worse class of durable desync. Synthesis elevates it as a **must-not-regress** control, not as an open defect.

**Four wave-5 scopes (A079, A081, A083, A085) lack finding files**; provisional peer evidence does not surface a second medium residual, but A085 should explicitly track that delta-tracking proofs do not close A080’s global reconciliation hole.

---

## 10. Sources read for this synthesis

- `docs/audit/controller-defense/shared/COORDINATION.md`
- `docs/audit/controller-defense/shared/SEED.md`
- `docs/audit/controller-defense/shared/AGENT_MANIFEST.md` (wave 5 / A103)
- `docs/audit/controller-defense/synthesis/PRELIMINARY.md`
- Findings: A076, A077, A078, A080, A082, A084 (present); supporting A022–A028, A032–A033, A041, A052–A053, A055, A072, A086, A094
- Code spot-checks: `spoke_usage.rs`, `context/spoke.rs`, `positions/mod.rs` `apply_leg_usage`; Certora `usage_*` name inventory in `spoke_rules.rs`
- Invariants: INV-RISK-01, INV-HALT-03
