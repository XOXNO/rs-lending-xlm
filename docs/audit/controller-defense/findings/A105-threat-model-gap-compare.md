# A105 — Threat-model Known gaps vs live code and audit corpus

- Agent: A105 (synthesis)
- Theme: T8
- Severity: medium (highest confirmed open Known-gap classes: router slippage + standalone trust roots + temporary Sensitive delay; highest newly surfaced: A080 / A064 G1)
- Status: partial (Known-gap catalogue mostly accurate; one closed item still closed; several audit residuals missing from Known gaps; a few framings need tightening)
- Paths: `docs/explanation/threat-model.md` §Known gaps (+ Availability / Accepted residual); live code cited per row; corpus A001–A104 / A106 where present; `STRIDE.md` excerpts; `synthesis/PRELIMINARY.md`
- Defense: See §3 (controls that still match the document) and §4 (confirmed Known gaps)
- Gap: See §4–§6 — confirmed open, newly surfaced (should consider adding to Known gaps), overstated / nuance, and closed
- Impact: See per-row blast radii; no novel critical fund-theft class beyond documented trust-boundary residuals. Largest account-local money residual remains strategy slippage under aggregator compromise (Known gap). Largest availability/governance residuals newly elevated by this audit: spoke-usage missing-row over-admission (A080) and `no_seize` uncoupled from freeze/supply (A064)
- Evidence: Live spot-checks listed in §2; peer syntheses A020, A101–A104; deep-dives A003, A005, A009, A014, A030, A048, A055, A056, A057, A064, A065, A067, A072, A080; INV-LIQ-04 / INV-STRAT / INV-HALT / INV-ORACLE
- Opinion: Treat `threat-model.md` Known gaps as **largely confirmed and still live**. Do not reopen the closed router-input-measurement item. Prioritize documenting newly surfaced residuals (A080, A064 Option C, Vec caps, plant-stale liq DoS, SAC listing) into the next threat-model revision. Soften “unbounded-loss” and INV-LIQ-04 citation wording so operators do not confuse account-local strategy drain with protocol-wide insolvency, or ordinary-liquidate HF post-gates with bad-debt seize post-guards.

> **Corpus-complete addendum:** All A001–A110 files are now present. Later
> Wave-3/5/6 filings did not refute §3–§4. Ranking: `synthesis/FINAL.md`.

---

## 1. Mission and method

**Mission:** Compare every item under `docs/explanation/threat-model.md` → **Known gaps** to (a) live code and (b) this controller-defense audit’s findings A001–A104 (plus adjacent A106 if present). Classify each as **confirmed**, **newly surfaced** (audit residual absent or under-emphasized in Known gaps), **overstated** (doc stronger than code/evidence), or **closed / still closed**.

**Method:**

1. Read `shared/COORDINATION.md`, `SEED.md`, `synthesis/PRELIMINARY.md`, README finding format.
2. Extract the full Known-gaps catalogue (Deployment gates + named subsections through Single-source price keys). Also note Availability trade-offs / Accepted residual risks when findings map there.
3. For each Known-gap claim, spot-check the cited live symbols and cross-link agreeing peer findings.
4. Inventory audit residuals that PRELIMINARY / A101–A104 rank as leading but that Known gaps omit or bury.
5. Flag framing mismatches (severity language, INV citations) without inventing novel Critical theft paths.

No production Rust edited. No git operations.

**Corpus note:** ~80 finding files present under `findings/` at write time. Wave coverage is incomplete (missing e.g. A035, A037–A039, A060, A069, A071, A073–A075, A079, A081, A083, A085, A088–A093, A095–A098, A100). Synthesis uses what is filed; unfiled scopes are not treated as proofs of absence.

---

## 2. Live code spot-checks (Known-gap anchors)

| Claim locus | Live check | Result |
|---|---|---|
| `verify_router_output` positivity only | `strategies/swap.rs` — `assert_with_error!(…, received > 0, NoSwapOutput)` | **Holds** |
| Aggregator `total_min_out` inside untrusted router | `swap-aggregator` execute path; controller never decodes min-out (A056) | **Holds** |
| Router owner immediate powers | `swap-aggregator/src/lib.rs` — `#[only_owner]` `upgrade`, `sweep_balance`, referral setters; `renounce_ownership` | **Holds** |
| Sensitive delay floor = 12 | `governance/src/constants.rs` `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` + TEMPORARY comment → 120_960 | **Holds** |
| Delegate borrow/withdraw `to` | Owner-or-delegate gate (A003/A005/A057); optional recipient | **Holds** |
| Sanity tighten-only | `price-aggregator/src/admin.rs` `SanityBandMustTighten` | **Holds** |
| Liquidation skips `require_post_pool_risk_gates` | `liquidation/mod.rs` → `finalize_position_flow` (persist only; no solvency call). A072 gate is for risk-increasing / strategy finalize | **Holds** |
| Pool seize no `guards::` | `pool/src/ops/seize.rs` — no post-guard assert | **Holds** |
| Router input measured (closed) | `swap-aggregator/.../execute/mod.rs` `transfer_amount_measured` credits delta not `total_in` | **Still closed** |
| Skew clamp to ledger time | `xoxno-oracle/.../aggregation.rs` `newest_ts.min(now * MS_PER_SECOND)` | **Holds** |
| `BAD_DEBT_USD_THRESHOLD` vs live floor | `controller/src/constants.rs` alias of `DEFAULT_MIN_BORROW…`; live floor in instance storage (A067) | **Holds** |
| Governance events | `governance/src/events.rs` — only deploy controller / deploy price-aggregator | **Holds** |
| Risk views unguarded | `lib.rs` `get_health_factor` / `is_liquidatable` → `views.rs` via `Cache::new_view`; no `require_not_flash_loaning` (A030) | **Holds** |
| DeFindex no admin/rescue | `defindex-strategy` surface: asset/deposit/harvest/balance/withdraw; no owner | **Holds** |
| `flash_position` fee / flashloanable | No origination fee; requires `is_flashloanable` (A045/A046) | **Holds** |

---

## 3. Executive verdict

| Bucket | Count (Known-gap items) | Headline |
|---|---|---|
| **Confirmed open** | 13 subsections + 3 deployment gates | Still accurate vs code and audit |
| **Confirmed closed** | 1 (`router input pull measured`) | Remains closed; do not re-open |
| **Overstated / needs nuance** | 2 primary framings | “Unbounded-loss” (account-local); INV-LIQ-04 citation conflates two post-condition absences |
| **Newly surfaced (audit → Known gaps backlog)** | ≥6 material residuals | A080, A064 G1, A062/A015, A065 plant-stale, A055 SAC listing elevation, A094 footgun; STRIDE Tamper.10 still outside Known gaps |

**Overall:** The Known-gaps section is a **high-quality map of intentional and deployment residuals**. This audit **confirms** rather than refutes it for money and trust-root items, and **extends** it with capacity/availability residuals the narrative section does not yet name at the same prominence as slippage / owners / delay.

A020’s earlier cross-check (“external trust roots correct; A080 under-highlighted”) is **upheld** by A101–A104.

---

## 4. Item-by-item: Known gaps catalogue

Legend: **C** = confirmed open · **CL** = closed/still closed · **O** = overstated or needs nuance · **N** = newly surfaced relative to Known gaps (may exist elsewhere in threat-model / STRIDE).

### 4.1 Deployment gates

#### D1 — Swap-aggregator owner is a trust root outside governance

| Field | Value |
|---|---|
| Classification | **C** |
| Live | Immediate `#[only_owner]` upgrade / sweep / referral / fee / whitelist; `renounce_ownership` irreversible |
| Audit | A009 (trust-root adjacency), A056 §6.3 / A101 G-ROUTER-OWNER, PRELIMINARY row, A020 |
| Impact | Amplifies G-SLIP: malicious upgrade can drop payload min-out; `sweep_balance` moves aggregator holdings. Strategy loss remains **account-local** for in-flight swaps; aggregator fee/referral balances are ops assets |
| Verdict | Accurate. Still a **mainnet release gate** to verify multisig owner |

#### D2 — XOXNO oracle owner is a trust root outside governance

| Field | Value |
|---|---|
| Classification | **C** |
| Live | Immediate upgrade / signer / threshold (threat-model trust table; STRIDE Elevation.7) |
| Audit | A009, A065 (aggregator trusts feed legs; single-source bound = sanity band), PRELIMINARY |
| Impact | Fabricated feeds for XOXNO-backed assets; dual-source + sanity bound blast radius when configured; single-source markets trust one operator |
| Verdict | Accurate. Do not enable dependent markets under an individual key |

#### D3 — Sensitive timelock delay temporary (12 ledgers)

| Field | Value |
|---|---|
| Classification | **C** |
| Live | `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12`; comment targets `120_960` |
| Audit | A009 G2 — **partial** until restored via `UpgradeGov`; release blocker per threat-model |
| Impact | Sensitive ops (upgrades, aggregator/oracle pointer moves, force socialize, manager registry, …) effectively ~1 minute after propose+execute path constraints |
| Verdict | Accurate. Highest-priority **deployment** Known gap |

---

### 4.2 Named Known-gap subsections

#### K1 — The controller does not bound slippage

| Field | Value |
|---|---|
| Classification | **C** (+ mild **O** on “unbounded-loss” wording) |
| Live | `verify_router_output` → `received > 0` only |
| Audit | A048, A056 (primary), A046/A047 Gap(1), A101 G-SLIP — **partial / medium**; PRELIMINARY leading residual |
| What holds | Exact `amount_in` auth; discard router return; overspend reject; leftover refund; post-strategy HF ≥ 1 when debt remains; honest aggregator enforces payload `total_min_out` |
| What fails | Quantitative floor at controller; compromised router or `min_out=1` payload can settle dust out |
| Impact refinement | Loss ≤ authorized swapped notional / excess HF (**account-local**). Not protocol share-mint. Sticky especially on `swap_collateral` |
| Overstatement note | “Unbounded-loss path for in-flight strategies” is correct **relative to that strategy’s notional**, easy to misread as protocol insolvency. Prefer A101 L1 framing in future doc edits |
| STRIDE drift | Tamper.4 “meet minimums” overclaims **controller** layer (A056 F6); threat-model itself is accurate |

#### K2 — A delegate has complete economic control of the account

| Field | Value |
|---|---|
| Classification | **C** (accepted design) |
| Live | `is_owner_or_delegate` + borrow/withdraw optional `to` |
| Audit | A003 defended gate; A005 blast-radius note; A057 accepted residual; A101 §4.8 |
| Impact | Delegate can drain credit line and collateral to self within post-pool HF |
| Verdict | Accurate design disclosure. Not a defect. User docs must stay plain |

#### K3 — The sanity band tightens only

| Field | Value |
|---|---|
| Classification | **C** (accepted / ops) |
| Live | `SanityBandMustTighten` |
| Audit | A006 ratchet peer; A065; INV-AUTH-04 |
| Impact | Compromised ORACLE → instant per-asset fail-closed kill; availability not mispricing widen |
| Verdict | Accurate |

#### K4 — Liquidation has no post-condition check

| Field | Value |
|---|---|
| Classification | **C** (+ **O** on INV-LIQ-04 citation precision) |
| Live | Liquidate path: plan HF pre-check → repay/seize → `finalize_position_flow` **without** `require_post_pool_risk_gates`. Pool `seize.rs` has no `guards::` post-assert |
| Audit | A013/A026/A051/A052: money/seize defenses hold; concrete theft attacks refuted. A072 documents post-pool gates on **other** paths only |
| INV nuance | INV-LIQ-04 **NOT ENFORCED** note targets **bad-debt cleanup / seize writedown** lacking utilization/solvency guards (like revenue path has). Threat-model folds that together with “no HF post-gate on liquidate.” Both absences are real; they are **related but not identical** |
| Impact | Structural residual: bonus/seizure scaling edge cases rely on plan math + measurement rather than a final HF assertion. Audit agrees “every concrete attack refuted; structural gap remains” |
| Verdict | Confirmed missing post-condition(s). Cite both ordinary-liquidate HF and bad-debt/seize market guards separately in a doc refresh |

#### K5 — The router input pull is measured (closed)

| Field | Value |
|---|---|
| Classification | **CL** |
| Live | Measured credit of input delta (not declared `total_in`) |
| Audit | A101 treats G-SLIP as output/min-out problem, not input shortfall against fee backing; measurement theme A041/A058 agrees measured custody |
| Verdict | **Still closed.** Correctly marked historical. No audit finding reopens it |

#### K6 — The oracle skew anchor is clamped to ledger time

| Field | Value |
|---|---|
| Classification | **C** (mitigation present; config residual remains) |
| Live | `newest_ts.min(now * MS…)` in `recompute_aggregate` |
| Audit | A065 notes skew/stale windows as config residual; feed outage blocks liquidation (Availability trade-offs) |
| Verdict | Clamp claim accurate. Residual is **window width ops hazard**, correctly stated |

#### K7 — The dust gate and the configured floor can drift apart

| Field | Value |
|---|---|
| Classification | **C** |
| Live | `BAD_DEBT_USD_THRESHOLD = DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD` (compile-time); live floor = instance `get_min_borrow_collateral_usd_wad` |
| Audit | A014 parameter residual; A067 §6.1 confirms threat-model; A102 adjacency |
| Impact | Band where permissionless `clean_bad_debt` won’t admit but debt is “should-be-dust” vs raised floor — owner `force_socialize` only |
| Verdict | Accurate ops/governance footgun |

#### K8 — Governance actions are not observable at the governance contract

| Field | Value |
|---|---|
| Classification | **C** |
| Live | Only `DeployControllerEvent` / `DeployPriceAggregatorEvent` |
| Audit | A009 notes monitoring must read storage; A033 covers controller event order (different contract) |
| Impact | Role grant/revoke and immediate vs delayed execute indistinguishable on events |
| Verdict | Accurate. Monitoring requirement stands |

#### K9 — Risk views are not flash-guarded

| Field | Value |
|---|---|
| Classification | **C** (accepted composability residual) |
| Live | Views use `Cache::new_view`; no flash check (A001, A030) |
| Audit | Explicitly intentional; not protocol self-accounting risk |
| Verdict | Accurate |

#### K10 — The DeFindex adapter has no rescue path

| Field | Value |
|---|---|
| Classification | **C** |
| Live | No owner / sweep / upgrade on adapter |
| Audit | A057 notes stranding if `to` = adapter; construction binds controller/hub/spoke |
| Verdict | Accurate |

#### K11 — Flash position pays no origination fee

| Field | Value |
|---|---|
| Classification | **C** (accepted economic asymmetry) |
| Live | Fee-free debt mint; `is_flashloanable` required (unlike `multiply`) |
| Audit | A045/A046; ADR-0020 |
| Verdict | Accurate. Fee optional in practice when markets are flashloanable |

#### K12 — An approved Blend pool can be upgraded by its own owner

| Field | Value |
|---|---|
| Classification | **C** (accepted trust) |
| Live | Allowlist + measured migrate + HF finalize |
| Audit | A050 / A101 G-BLEND — migrator-local harm; no silent third-party theft found |
| Verdict | Accurate |

#### K13 — Single-source price keys trust one operator completely

| Field | Value |
|---|---|
| Classification | **C** |
| Live | Dual-source tolerance only when second source configured; sanity band backstop (A065) |
| Audit | A065; STRIDE Elevation.7 / Tamper.1 residuals |
| Verdict | Accurate. Ops must enumerate single-source keys |

---

## 5. Newly surfaced residuals (audit → Known-gaps backlog)

These are **not** refutations of existing Known gaps. They are material residuals this audit elevates that the Known-gaps section either omits or only implies via Availability / STRIDE.

| ID | Residual | Severity (audit) | Why it belongs near Known gaps | Blast radius |
|---|---|---|---|---|
| **A080** | `apply_exit` no-op if `SpokeUsage` row missing | medium | Soft cap integrity; PRELIMINARY + A103 leading T5 residual; A020 already flagged under-highlight | Over-admission ≤ spoke asset cap headroom; no direct theft |
| **A064 G1** | `no_seize` ̸⇒ `frozen` / still allows supply (ADR-0008 Option C unshipped) | medium | STRIDE DoS.2 / Availability mention listing flags, but Known gaps never names the coupling hole | Liquidation halt for holders of that collateral; bad-debt latency; hatch = `force_socialize` |
| **A062 ∪ A015** | No hard length cap on mutator payment / keeper Vecs (views use 256) | low | DoS.5 class; PRELIMINARY residual | Fee/CPU grief only |
| **A065 plant-stale** | Supply/repay skip pricing → dust stale collateral leg can block later liq pricing | low | Availability / liquidation liveness; related to oracle outage trade-off but mechanism is distinct | Account-scoped recovery DoS until feed refresh |
| **A055 / G-LIST** | Non-SAC / rebasing / balance-lying if listed | medium | Appears under Accepted residual / Tamper.3 more than Known gaps; A101 elevates to primary money residual #2 | ≤ that market’s TVL desync → supplier socialization |
| **A094** | Future pool merge omitting `put_market_index` | low (footgun) | Engineering hazard, not current path bug; PRELIMINARY residual | Wrong in-tx HF/caps if regresses |
| **STRIDE Tamper.10** | Admission attestation of XOXNO `max_submission_age` is point-in-time | medium (STRIDE ⚙) | In STRIDE, not Known gaps; A065 cross-ref | Silent degradation of admission invariant if oracle owner widens window |
| **A007 hook residual** | Listed-token transfer hooks after flash flag clear | low (listing) | Amplifies A055; not a Named Known gap | Reentry vs unpersisted RAM; atomicity + auth bound |

**Recommended threat-model additions (doc-only backlog for maintainers):**

1. Spoke-usage reconciliation / missing-row exit tolerance (A080).
2. `no_seize` without freeze as liquidation-availability footgun (A064 / ADR-0008 Option C).
3. Explicit “SAC-only listing” Known gap or promotion from Accepted residual (A055).
4. Optional: uncapped keeper/mutator Vecs; plant-stale supply shield; Tamper.10 attestation drift.

---

## 6. Overstated or easy-to-misread claims

| Claim | Where | Audit judgment | Suggested reading |
|---|---|---|---|
| Router compromise = **unbounded-loss** for in-flight strategies | K1 | Mechanism real; adjective unbounded relative to **strategy notional / excess HF**, not protocol TVL | Prefer “account-local loss up to swapped notional subject to post-gate solvency” (A101 L1) |
| INV-LIQ-04 “records this exact gap” for missing liquidate post-HF | K4 | INV-LIQ-04 NOT ENFORCED is about **bad-debt/seize market post-guards**; liquidate also lacks `require_post_pool_risk_gates` — both true, citation conflates | Split: (a) no post-HF on `liquidate`; (b) no post-guards on bad-debt seize writedown |
| “Nothing here is a theoretical concern” (Known gaps intro) | Header | Mostly true; some items are **accepted design** with no open exploit (delegate power, view flash, flash fee asymmetry) | Fine if read as “concrete residual,” not “active Critical bug” |
| STRIDE Tamper.4 “positive and meet minimums” | STRIDE (not Known gaps) | Overclaims controller | Threat-model K1 is the accurate statement (A056) |
| STRIDE Elevation.6 controller-routed aggregator upgrade | STRIDE (A009 G7) | Stale vs tree; threat-model trust-root table is correct | Prefer threat-model over that STRIDE sentence |

No Known-gap item was found to be **false** in live code except the already-marked closed K5 (still closed).

---

## 7. Map: Known gaps ↔ audit syntheses

| Known gap | Primary audit owners | Synthesis bucket |
|---|---|---|
| D1 Aggregator owner | A009, A056, A101 | Confirmed trust root |
| D2 XOXNO owner | A009, A065 | Confirmed trust root |
| D3 Sensitive delay = 12 | A009 | Confirmed release blocker |
| K1 No controller min-out | A048, A056, A101 | Confirmed partial medium |
| K2 Delegate economic control | A003, A005, A057 | Confirmed accepted design |
| K3 Sanity tighten-only | A006, A065 | Confirmed |
| K4 Liq no post-condition | A013, A026, A051–A053, A072 | Confirmed structural; attacks refuted |
| K5 Router input measured | A041, A058, A101 | Still closed |
| K6 Skew clamp + window ops | A065 | Confirmed |
| K7 Dust vs floor drift | A014, A067 | Confirmed |
| K8 Gov events missing | A009 | Confirmed |
| K9 Views not flash-guarded | A001, A007, A030 | Confirmed accepted |
| K10 DeFindex no rescue | A057 adjacency | Confirmed |
| K11 Flash position fee asymmetry | A045, A046 | Confirmed accepted |
| K12 Blend pool upgrade trust | A050, A101 | Confirmed accepted |
| K13 Single-source keys | A065 | Confirmed |
| *(missing)* A080 usage | A080, A103, A020 | **Newly surfaced** |
| *(missing)* A064 no_seize coupling | A064, A102 | **Newly surfaced** |
| *(missing)* Vec caps | A062, A015, A102 | **Newly surfaced** |
| *(under-emphasized)* SAC listing | A055, A101 | **Newly surfaced elevation** |

---

## 8. Availability trade-offs & Accepted residuals (secondary compare)

Not every Availability / Accepted bullet needs a Known-gap promotion. Audit alignment:

| Doc area | Audit stance |
|---|---|
| Oracle outage blocks liquidation | Confirmed (A065; Availability) |
| Global pause vs listing pause (INV-HALT-01/02) | Confirmed (A001, A064) |
| Exact flash balance / never mark non-exact flashloanable | Confirmed (A044) |
| Route quality not verified on-chain | Same class as K1 / Accepted residual — confirmed |
| Bad debt socializes over current suppliers | Accepted; A014 authority split defended |
| Formal models have assumptions | Out of scope for A105 depth |

---

## 9. Inputs for later wave-7 agents

| Agent | Takeaway from A105 |
|---|---|
| **A106** | Seed max-loss from confirmed K1 (L1 account slippage) + A055 (L2 market listing) + D1/D2 trust roots; do not invent protocol-wide unbounded insolvency from K1 wording alone |
| **A107** | Keep STRIDE Medium residuals for Tamper.10, DoS.2 (`no_seize`), Elevation.7; downgrade confidence in Tamper.4 “meet minimums” at controller; trust-model Known gaps > stale Elevation.6 |
| **A108** | Highest missing tests already named by peers: controller dust-out vs payload min-out (A056); usage↔Σ positions invariant (A080); Option C setter pins (A064) |
| **A109** | No Known-gap vs audit **fact conflict** warranting a disagreement file; only framing notes (§6) |
| **A110** | Remediation priority aligned with threat-model Audit priorities + new backlog: (1) restore Sensitive delay; (2) verify aggregator/XOXNO owners; (3) controller `min_out` or equivalent; (4) ADR-0008 Option C; (5) usage reconcile; (6) SAC listing runbooks; (7) Vec caps |

---

## 10. Verdict

**Confirmed:** Nearly the entire Known-gaps catalogue still matches live code and is reinforced by this audit — especially deployment gates (D1–D3), controller slippage positivity-only (K1), delegate power (K2), liquidation missing post-gates (K4), dust/floor drift (K7), governance event blindness (K8), and external trust / single-source / Blend / DeFindex / flash-fee items.

**Still closed:** Router input measurement (K5).

**Overstated / tighten wording:** “Unbounded-loss” without account-local qualifier; INV-LIQ-04 as the sole citation for all liquidation post-condition absence.

**Newly surfaced for the next threat-model revision:** A080 spoke-usage exit no-op; A064 `no_seize` coupling footgun; uncapped Vec hygiene; plant-stale liquidation DoS; elevate SAC-only listing; consider Tamper.10 attestation drift.

**No novel critical theft class** appeared that contradicts the threat-model’s trust-boundary story. The audit’s leading money residuals **are** the document’s K1 + listing/token trust; the audit’s leading non-money residuals **extend** the document into capacity and liquidation-availability governance footguns.
