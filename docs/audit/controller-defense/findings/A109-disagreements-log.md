# A109 — Cross-agent disagreements log

- Agent: A109 (synthesis)
- Theme: T8
- Severity: info (meta — no material Severity/Status conflict found)
- Status: defended (corpus agreement); process note — `disagreements/` remains empty by design of this wave
- Paths: `docs/audit/controller-defense/disagreements/` (empty); all filed `findings/A*.md`; `synthesis/PRELIMINARY.md`; peer syntheses A101–A106
- Defense: Agents explicitly scoped residuals to owners; syntheses (A101 §7, A103 §6.3, A104 §8, A105 §6/§A109 note, A106) already preferred framing reconciliation over disagreement files
- Gap: Header Severity/Status labels sometimes describe **scope defense** while Gap(n) lines describe a **shared residual owned elsewhere** — readers can misread A046/A047 vs A048/A056 as conflict. No peer asserts contradictory facts about the same code path
- Impact: None on funds. Risk is synthesis noise / double-counting in A110 if header labels are treated as competing rankings of the same residual
- Evidence: Full header extract of Severity/Status for all filed findings; deep compare of flagged pairs (A048↔A056, A080↔A084, A007 residuals); empty `disagreements/` directory; PRELIMINARY leading-residuals table
- Opinion: **No material cross-agent disagreements.** Log only nuance / framing / ownership splits. For A110 ranking, follow A048/A056 (slippage), A080 (usage exit), A007 (post-guard hooks = low listing residual), not the “defended” headers of sibling money-flow agents that still list the same Gap(1)

## Scope and method

1. Read `shared/COORDINATION.md`, `SEED.md`, `AGENT_MANIFEST.md` (A109 = cross-agent disagreements log).
2. Listed `docs/audit/controller-defense/disagreements/` — **empty** (directory exists; zero `AXXX-vs-AYYY.md` files).
3. Extracted header `Severity` / `Status` from every filed finding under `findings/`.
4. Deep-read flagged pairs: **A048 vs A056**, **A080 vs A084**, **A007 residuals** and every peer that cites them.
5. Scanned peer “Agreements / disagreements” sections (A101 §7, A102 §5, A103 §6, A104 §8, A105, A106) and PRELIMINARY.
6. Flagged any pair that (a) names the same mechanism and (b) assigns incompatible Status (`defended` vs `undefended` / contradictory `partial`) or incompatible Severity band for that **same** claim — not merely different headers for different scopes.

Out of scope: inventing novel Critical theft paths; writing production Rust; git ops; filling unfiled agent scopes (listed as coverage debt only).

---

## Verdict

| Class | Result |
|---|---|
| Peer-authored `disagreements/*.md` | **None** |
| Fact conflicts (same path, opposite claim) | **None** |
| Severity/Status conflicts on the **same issue** | **None material** |
| Nuance / framing / label differences | **Present** — catalogued below |
| Docs-of-record vs code framing drift | **Present** (STRIDE / threat-model wording) — agent corpus agrees on code |

**Bottom line:** Concurrent agents did not split into camps. Apparent tension is almost always **scope framing** (custody defended vs residual elevated) or **intentional design vs accidental under-count** (A084 vs A080), already reconciled by wave syntheses.

---

## 1. Empty disagreements directory

| Path | State |
|---|---|
| `docs/audit/controller-defense/disagreements/` | Exists, **zero files** |
| SEED instruction | Create `disagreements/AXXX-vs-AYYY.md` when peers conflict |
| Wave syntheses | Explicitly declined: A101 §7.2, A103 §6.3, A104 §8.2, A105 (“No Known-gap vs audit fact conflict”), A106 (“No disagreement file”) |

This file is the A109 consolidation. It does **not** create per-pair disagreement files because none meet the SEED threshold (contradictory evidence about the same code path).

---

## 2. Priority pair: A048 vs A056 (slippage / min-out)

### 2.1 Headers

| ID | Scope | Severity | Status |
|---|---|---|---|
| A048 | `swap_collateral` money-flow legs | **medium** (residual; primary path defended) | **partial** |
| A056 | Controller-side min-out / slippage across all strategy swap callers | **medium** (documented trust-root residual) | **partial** |

### 2.2 Same issue?

**Yes — shared residual:** controller `verify_router_output` enforces only `received > 0`; quantitative `total_min_out` lives in opaque aggregator `Bytes` (untrusted trust root). Both cite INV-STRAT-02 / threat-model Known gap K1.

### 2.3 Conflict?

**No.** They agree on mechanism, blast radius (account-local ≤ swapped / withdrawn notional subject to post-gate HF when debt remains; debt-free collateral swap can lose nearly all withdrawn notional), and remediation shape (controller `min_out` or decode-and-check payload vs measured Δ).

| Dimension | A048 | A056 | Reconciliation |
|---|---|---|---|
| Custody / measurement on swap_collateral | Defended in Defense/Opinion | Out of scope as primary; agrees peers | Complementary |
| Quantitative slippage | Gap(1) = **primary residual**; Status partial | Entire file owns inventory; Status partial | **Same ranking** |
| Cross-caller coverage | swap_collateral (+ shared helpers) | multiply, swap_debt, swap_collateral, repay_with_collateral, convert_swap; contrast flash_position floors | A056 supersets callers |
| Impact wording | “nearly all withdrawn collateral… still-healthy account” | Same; sticky on swap_collateral + spare-HF multiply | Agree (A056 §6 / A048 Impact) |

**Synthesis rule (already in A101 §7.2, A105 K1, A106, PRELIMINARY):** treat **A048∪A056** as one medium/partial residual class (G-SLIP). Do not average with A046/A047 headers (see §4).

---

## 3. Priority pair: A080 vs A084 (spoke usage under-count)

### 3.1 Headers

| ID | Scope | Severity | Status |
|---|---|---|---|
| A080 | `apply_exit` no-op when usage row missing | **medium** | **partial** |
| A084 | Liquidation / strategy usage skip or double-count | **low** | **defended** |

### 3.2 Same issue?

**No — related but different mechanisms.**

| | A080 | A084 |
|---|---|---|
| Mechanism | Missing storage row → exit returns without decrement | Credit seize: debit+credit cancel; **only fee** `apply_spoke_exit`; Transfer uses normal withdraw usage; strategy single `finalize_position_flow` |
| Intent | Tolerance for never-recorded / cleared rows (legacy/migration) | Intentional so liquidations are not blocked by supply cap |
| Capacity direction if “wrong” | Under-count occupancy → **over-admission** | Broken double-count would **tighten** caps (DoS); fee-only is deliberate net under-count of gross seize |
| Direct theft? | No | No |

### 3.3 Conflict?

**No.** A103 §6.3: “A084’s fee-only path is intentional under-count of gross seize, not the same as A080’s missing-row under-count (different mechanism, **same capacity-direction when fee exit no-ops**).”

Compounding (not disagreement): Credit fee / bad-debt exits call `apply_spoke_exit`; if the row is missing, those exits silently skip — **A080 bites on A084’s call sites**. Peers A027/A052/A053/A078 note this. Remediation must not “fix” A084 by double-exiting gross seize (would break liquidations / double-exit).

**Synthesis rule:** Rank **A080** as T5 leading residual (medium/partial). Keep **A084** defended/low. A099 correctly escalates A080 as the real “skipped check” residual outside Cache memo maps.

### 3.4 Internal clarity note (A080 Gap prose)

A080’s Gap bullet self-corrects mid-sentence (“wait: missing row means…”). Later A103 §4.1 states the cleaned model: missing row ≡ usage treated as zero → exit no-ops → **overstates available capacity iff positions still exist without a usage row**. This is authorial clarification, not a peer disagreement. Prefer A103 wording when citing.

---

## 4. Priority cluster: A007 residuals (and peers)

### 4.1 Primary finding

| ID | Severity | Status | Residual of interest |
|---|---|---|---|
| A007 | **low** | **defended** | Post-guard listed-token transfer hooks during strategy settlement (deposit / leftover / refund) while in-memory state unpersisted; intentional ungated TTL/delegate; matrix coverage holes for three already-gated entrypoints |

Primary defense claim: flash / router / Blend / debt-forward windows cannot reenter monetary position or strategy entrypoints — **defended**. Strategies are setters + checkers, not bypasses.

### 4.2 How peers rate the residual

| Peer | How A007 residual is framed | Elevates above low? |
|---|---|---|
| A045 / A046 / A047 / A048 / A049 / A050 / A054 / A070 | Shared Gap / residual; listing trust + measurement | **No** |
| A043 | Ordinary borrow/repay lack `with_flash_guard` (by design per A007 §2); strategy borrow-into-controller holds flag | **No** — contrasts ordinary vs strategy |
| A101 G-FLASH-POST | low residual; rises if listed token has arbitrary hooks | **No** (conditional escalate documented) |
| A105 | “A007 hook residual — low (listing)” | **No** |
| PRELIMINARY | Flash reentrancy guard listed under “defenses that look strong” (A007, A019) | Treats primary as defended |

### 4.3 Nuance (not conflict)

1. **Primary Status = defended** while Gap(3) is a real residual — same pattern as A046 “defended + Gap(1)”. Readers must not treat “defended” as “no residual.”
2. **Ordinary pool legs** (bare borrow/repay/supply/withdraw) intentionally do not set the flash flag; the flag is for untrusted callback/hook windows. A043 naming this as Gap(3) does not contradict A007 §2 design note.
3. **A049** notes repay transfer leg unguarded after guarded withdraw — same post-guard class, not a novel Critical.
4. Coverage gaps (`migrate_from_blend` / `recapitalize` / `force_socialize_bad_debt` missing from `reentrancy_matrix`) are **test debt**; code already gates — A007 does not claim undefended entrypoints.

**Synthesis rule:** A007 residual stays **low / listing-trust**, owned by A007; amplify only via A055 if non-SAC / hookable tokens are listed. Do not invent a medium “flash guard broken” finding from peer Gap citations.

---

## 5. Adjacent framing cluster: A046 / A047 vs A048 / A056

This is the **largest labeling nuance** in the corpus (called out by A101 §7.2 and A106).

| ID | Header Severity | Header Status | Gap(1) content |
|---|---|---|---|
| A046 multiply | info | defended | Controller slippage only `received > 0` |
| A047 swap_debt | info | defended | Same |
| A048 swap_collateral | medium | partial | Same (elevated as primary residual) |
| A056 slippage inventory | medium | partial | Same (owns cross-caller inventory) |

**Why headers differ without a fact conflict:**

- A046/A047 judge **custody / share-cash conservation** → defended; list slippage as known policy residual inside Gap(1).
- A048/A056 judge **economic fairness / quantitative floor** → partial/medium because that residual is the leading undefended economic surface on those scopes (especially debt-free / spare-HF swap_collateral).

No peer claims “controller enforces min_out.” No peer claims “custody measurement is broken on multiply/swap_debt.”

**Reconciliation (mandatory for A110):** Rank the residual with **A048/A056 / A101 G-SLIP**, not by averaging to info because A046/A047 headers say defended.

---

## 6. Other nuance differences (non-material)

### 6.1 Hybrid Status headers (intentional, consistent)

| ID | Hybrid Status | Meaning |
|---|---|---|
| A009 | defended (ownership wiring) / partial (Sensitive delay 12) | Code gates present; deployment Known gap open |
| A015 | defended (recipient/health) / partial (Vec bounds) | Same agent, two surfaces |
| A064 | defended (core flags) / partial (ADR-0008 `no_seize` Option C) | Core matrix OK; governance footgun residual |
| A065 | defended (mutations) / partial (non-pricing paths + config) | Fail-closed pricing OK; plant-stale / windows residual |
| A067 | defended (gates) / partial (grandfathering + BAD_DEBT desync) | Floor chokepoint OK; ops residual |

Peers and A105 treat these as **scoped hybrids**, not contradictory votes.

### 6.2 A086 vs A094 (Cache) — tension, not disagreement

| ID | Severity | Status | Claim |
|---|---|---|---|
| A086 | info | defended | Cache inventory / memo rules sound; sync-data fill-once incomplete |
| A094 | low | partial | Future/missed `put_market_index` footgun |

A104 §8.3: both are residuals under a defended architecture; A094 higher priority because risk/cap math re-reads indexes. **No disagreement file.**

### 6.3 A076 vs A080

A076 = defended core entry/exit semantics; Gap line defers missing-row to A080. Consistent ownership split (same pattern as A046→A056).

### 6.4 A078 vs A080

A078 defended persist-after-pool; explicitly orthogonal to A080 under-accounting. Agree.

### 6.5 A099 vs A080 / A084

A099 defended for Cache/opt short-circuits; names A080 as main skipped-check residual and A084 fee-only as intentional. Consistent with §3.

### 6.6 A026 vs A036 (cleanup shorthand)

A026 corrects: liquidation finalize does **not** set `remove_if_empty: true`; cleanup is a dedicated post step. A036 Defense says strategies/liquidation set `remove_if_empty` “where appropriate” (looser shorthand). **Factual nuance on which call sets the flag** — not a Severity fight; A026’s precision wins for liquidation path docs. No disagreement file was filed (threshold: shorthand vs precise path map).

### 6.7 A015 vs A062 (Vec bounds)

Both low/partial on uncapped mutator Vecs / keeper lists; PRELIMINARY groups A062/A015. Complementary, not conflicting.

### 6.8 A055 consensus

medium / partial everywhere it is cited (A101 G-LIST, A105, A106, PRELIMINARY). No down-rank to info by money-flow peers (they inherit listing trust).

### 6.9 A064 / A102 ranking of `no_seize`

A064 medium residual; A102 elevates as worst in-wave validation residual. Agree. Not a conflict with A006 flag ratchet (ratchet defended; Option C coupling still open).

### 6.10 Threat-model / STRIDE framing vs agents (docs drift)

| Topic | Drift | Agent consensus |
|---|---|---|
| “Unbounded-loss” strategy slippage | Easy to misread as protocol insolvency | Account-local ≤ notional / excess HF (A101/A105/A106) |
| STRIDE Tamper.4 “meet minimums” | Overclaims **controller** layer | Only positivity at controller (A056 F6); threat-model accurate |
| INV-LIQ-04 citation in Known gaps | Conflates liquidate post-HF vs bad-debt seize post-guards | Both absences real; split citations (A105 K4) |
| A080 under-highlighted in Known gaps | A020 / A105 | Newly surfaced backlog item — not agent-vs-agent |

These are **document vs audit framing** notes, not agent disagreements. A105 already owns the compare.

### 6.11 A010 partial vs strong T1 peers

A010 low/partial (access-control checker / declaration hygiene) sits beside many info/defended auth findings. Different scope (gate script vs runtime auth). No conflict.

### 6.12 Synthesis Severity headers

A101–A106 use **portfolio** Severity (highest residual in wave / corpus). That can look “higher” than many leaf info findings without contradicting them. A104 explicitly keeps wave-6 header **low** while noting inherited A080 medium owned by T5 — correct non-double-count.

---

## 7. Severity/Status matrix — flagged issues only

| Issue | Claiming IDs | Severities | Statuses | Material conflict? |
|---|---|---|---|---|
| No controller quantitative `min_out` | A046 Gap(1), A047 Gap(1), A048, A056, A101 G-SLIP, PRELIMINARY | info headers vs **medium** owners | defended headers vs **partial** owners | **No** — see §5 reconciliation |
| `apply_exit` missing-row no-op | A080, A076 defer, A078 ortho, A099 escalate, A103 | **medium** | **partial** | **No** dissent |
| Credit fee-only usage / double finalize | A084, A052/A053, A078 | **low** | **defended** | **No** dissent |
| Post-guard listed-token hooks | A007 + money peers | **low** | residual under defended primary | **No** dissent |
| Non-SAC / lying listed tokens | A055 + inherit | **medium** | **partial** | **No** dissent |
| `no_seize` uncoupled | A064, A102 | **medium** | hybrid / partial residual | **No** dissent |
| Sensitive delay = 12 | A009, A105, A106 | medium / critical-if-miswired | hybrid partial | **No** dissent |
| `put_market_index` footgun | A094, A087, A104 | **low** | **partial** | **No** dissent |

---

## 8. Coverage debt (absence ≠ disagreement)

Unfiled scopes at A109 write time (from disk inventory; not a conflict log):

A035, A037–A039, A060, A069, A071, A073–A075, A081, A083, A085, A089–A093, A095–A098, A100, A107, A108, A110 (+ this file was the missing A109).

Peer syntheses already mark provisional inferences (e.g. A101 on A042/A043/A060 before those files landed; A103 on A079/A081/A083/A085; A104 on most of Wave 6). Where later files appeared (A042, A043, A079, A088, …), peers reported **agreement** with prior inference — still no disagreement files.

---

## 9. Rules for downstream agents

| Consumer | Instruction |
|---|---|
| **A110 backlog** | One row for G-SLIP (cite A048+A056); one for A080; one for A007 residual only if listing/hooks in scope; do not create competing rows from A046/A047 headers |
| **A108 tests** | Prefer A056 F7 (dust-out vs large payload min) and A080 usage↔Σ positions — peers already align |
| **Future deep-dives** | File `disagreements/AXXX-vs-AYYY.md` only if evidence contradicts a peer’s claim about the **same** path (e.g. “Credit mints fee” vs “absorb-only”). Label-only severity spread is **not** enough |
| **Readers of leaf findings** | Read Gap/Impact/Opinion before treating header Status as a global verdict on shared residuals |

---

## 10. Catalogue of nuance-only differences (complete for filed corpus)

1. **A046/A047 defended vs A048/A056 partial** on shared slippage residual (§5).
2. **A080 medium/partial vs A084 low/defended** — different mechanisms; compounding call sites only (§3).
3. **A007 defended primary + low residual** vs peers citing the residual in Gaps — consistent (§4).
4. **A086 defended vs A094 partial** Cache footgun priority (§6.2).
5. **Hybrid Status** on A009 / A015 / A064 / A065 / A067 (§6.1).
6. **A026 precision vs A036 shorthand** on liquidation `remove_if_empty` (§6.6).
7. **Threat-model / STRIDE wording** vs agent impact framing (§6.10).
8. **Synthesis portfolio Severity** vs leaf info findings (§6.12).
9. **A099 “defended” opt hunt** while escalating A080 (§6.5).
10. **A076 defended** deferring missing-row to A080 (§6.3).

Nothing in (1)–(10) warrants a SEED disagreement file.

---

## 11. Verdict (repeated for synthesis skimmers)

**No material cross-agent disagreements.** The `disagreements/` directory is empty; wave syntheses already reconciled the only sharp labeling tension (strategy money-flow “defended” headers vs slippage “partial/medium” owners). Priority pairs **A048↔A056** agree; **A080↔A084** are different issues; **A007** residuals are unanimously low/listing-trust under a defended primary flash guard.

A109 recommends **zero** disagreement files and **one** labeling convention for A110: rank shared residuals by the agent that **owns** them (A056/A048, A080, A007, A055, A064, A009), not by sibling headers that correctly call their narrower custody scope defended.
)