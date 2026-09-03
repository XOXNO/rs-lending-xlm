# A110 — Final prioritized remediation backlog (controller defense)

- Agent: A110 (final backlog / synthesis)
- Theme: T8
- Severity: medium (highest *live code* residual class); critical **only** under failed deployment trust roots (A009 / oracle / router ownership)
- Status: synthesis — remediations ranked from documented residuals only; no novel exploit invention
- Paths: synthesis over `synthesis/PRELIMINARY.md`; findings A101–A104, A106; leading residuals A080, A055, A009, A048, A056, A064; supporting A062/A015, A065, A067, A086, A094; threat-model Known gaps; ADR-0008 / INV-* cited by peers
- Defense: Strong stacks already shipped — pause matrix, owner-or-delegate, flash guard, measured custody, Credit absorb-vs-mint, pool-truth spoke usage on healthy entry, Cache pin + post-leg index overwrite — see A101 §3, A102 §2, A103 §3, A104 §3
- Gap: This file does **not** invent gaps. It **orders fixes** for residuals already ranked by PRELIMINARY and refined by A101–A104 / A106
- Impact: Absolute ceiling is protocol-total only on deploy/oracle ownership failure (A009 / S4–S5). Highest-probability live code loss is account-local strategy slippage (A048/A056 / S1). Market ceilings ≤ \(\mathrm{TVL}_m\) (A055) or contingent bad debt after capacity/availability failures (A080, A064)
- Evidence: Peer priority tables A101 §10, A102 §6, A103 §8, A104 §9, A106 §8; PRELIMINARY leading-residuals table; threat-model §§Deployment gates / slippage; no `disagreements/` files present
- Opinion: Remediate by **absolute ceiling first** (deploy + Sensitive floor + router/oracle ownership), then **highest-probability extractable account loss** (controller `min_out`), then **market listing + liquidation liveness** (SAC policy, ADR-0008 Option C), then **soft-cap integrity** (A080 reconcile), then hygiene/footguns. Do not reopen closed money paths (A101 L6–L10) or “fix” intentional designs (aggregate-and-sum, ADR-0005 price snapshot, Credit fee-only usage exit, persist-after-pool).

---

## 0. Mission, method, and ranking model

### 0.1 Mission

Produce the **final prioritized remediation backlog** for controller defense gaps at the end of the audit wave series (A001–A110 intent). Rank every actionable residual by:

\[
\mathrm{Score} \approx \mathrm{Severity} \times \mathrm{Impact\ tier} \times \mathrm{Fixability}
\]

where impact is the **documented blast radius** (account / market / protocol) and fixability is engineering + ops effort to close the residual without inventing new threat models.

### 0.2 Method

1. Read `shared/COORDINATION.md`, `SEED.md`, `AGENT_MANIFEST.md` (A110), `README.md` finding format, `synthesis/PRELIMINARY.md`.
2. Read synthesis peers **A101–A104, A106** (A105 / A107 / A108 / A109 **absent** on disk — noted as coverage debt, not invented findings).
3. Read leading residual primaries: **A080, A055, A009, A048, A056, A064**; supporting A062, A015, A065, A067, A086, A094.
4. Fold peer backlog sections (A101 §10, A102 §6, A103 §8, A104 §9, A106 §8) into one ranked program.
5. Update A101 coverage-hole notes where **A042 / A043 / A068** have since filed as **defended** (do not re-open those as backlog defects).
6. Explicitly separate: **deploy gates**, **live code residuals**, **ops/runbook**, **process/lint**, **docs/tests**, **anti-remediation**.

No production Rust edited. No git operations (COORDINATION).

### 0.3 Scoring rubrics

| Axis | Levels used |
|---|---|
| **Severity** | critical (deploy-total) · medium (documented live residual) · low · info |
| **Impact tier** | P = protocol-total · M = ≤ market \(\mathrm{TVL}_m\) / \(D_{\mathrm{bad}}\) · A = account-local extractable · C = contingent only · Z = fees/availability only · 0 = closed |
| **Fixability** | H = small/config/ops · M = product + harness · L = architectural / trust-root accept |
| **Priority band** | **P0** ship-before-value · **P1** next code/ops sprint · **P2** integrity hygiene · **P3** process/docs/coverage · **P4** optional polish · **R** must-not-regress / accepted |

**Hard rules from peers (do not violate in this backlog):**

- Do **not** invent exploits beyond documented residuals (COORDINATION + user scope).
- Threat-model “unbounded loss” for slippage = unbounded in **in-flight strategy notional**, not protocol share mint (A056, A101, A106).
- A080 / A064 do **not** steal funds by themselves; they distort capacity or delay recovery (PRELIMINARY, A102, A103, A106).
- Protocol-total requires **S4/S5** trust-root failure (A106), not a missing `transfer_amount_measured` in the present corpus.

### 0.4 Corpus snapshot (synthesis completeness)

| Peer | Present? | Role for A110 |
|---|---|---|
| A101 money gaps | yes | Owns G-SLIP / G-LIST / closed L6–L10 |
| A102 validation gaps | yes | Owns A064 Option C, Vec caps, plant-stale |
| A103 spoke-usage gaps | yes | Owns A080 + A078 must-not-regress |
| A104 cache hazards | yes | Owns A094 checklist; A086 sync-data |
| A106 max-loss | yes | Owns S1–S11 bounds; absolute-ceiling ranking |
| A105 threat-model crosswalk | **yes** (filed after this snapshot) | Confirms Known gaps; newly surfaced A080 / A064 |
| A107 STRIDE residual | **yes** (filed after this snapshot) | Raises Tamper.4 at controller; ADD A080 |
| A108 missing tests | **yes** (filed after this snapshot) | Owns PIN/CLOSE names; supersedes inferred RB-12 rows |
| A109 disagreements | **yes** (filed after this snapshot) | No material fact conflicts |

---

## 1. Executive ranked backlog (one-page)

| Rank | Band | Item | Primary IDs | Sev × Impact × Fix | Max documented loss | Recommended fix shape |
|---:|---|---|---|---|---|---|
| 1 | **P0** | Restore Sensitive floor + verify owner=governance | A009, threat-model, A106 S4 | critical×P×H | **Protocol-total** if mis-wired / unrestored | Config/constant + deploy checklist |
| 2 | **P0** | Confirm swap-aggregator + XOXNO oracle intended owners | Threat-model, A056, A065, A106 S2/S5 | critical×P/A×H | Router: Σ account strategy losses + router treasury; Oracle: protocol-wide bad valuation | Ops ownership attestation; no lone EOA on XOXNO |
| 3 | **P0** | Controller-enforced quantitative `min_out` on strategy swaps | A048, A056, A101 G-SLIP, A106 S1 | medium×A×M | ≈ \(V(N_{\mathrm{in}})\) / withdrawn notional; HF-clipped if debted | Explicit `min_out` arg **or** decode+check vs measured Δ (mirror `flash_position`) |
| 4 | **P1** | SAC-only listing runbooks / gates; never flashloanable on non-exact | A055, A101 G-LIST, A106 S3 | medium×M×H | ≤ \(\mathrm{TVL}_m\) | Listing policy + optional pre-list checklist; A044 flash flag discipline |
| 5 | **P1** | Ship ADR-0008 **Option C**: `no_seize ⇒ frozen` (and/or block supply) | A064 G1, A102 G-VAL-1, A106 S7 | medium×M/A×H | 0 direct theft; growth of unliquidatable set; socialize ≤ \(D_{\mathrm{bad}}\) | Setter coupling + harness pins |
| 6 | **P1** | Spoke usage ↔ Σ positions invariant + reconcile admin | A080, A028, A103, A106 S6 | medium×C×M | Over-admission ≤ cap headroom; realized loss only if later bad debt ≤ \(\mathrm{TVL}_m\) | Keeper/assert + permissioned rewrite path |
| 7 | **P2** | Cap keeper / mutator payment Vec lengths | A062, A015, A102 G-VAL-2, A106 S8 | low×Z×H | Fees / budget only | `MAX_KEEPER_INPUTS` / reuse 256 before loops |
| 8 | **P2** | Review checklist / lint: every pool merge → `put_market_index` (+ `apply_leg_usage`) | A094, A104, A077 | low×A(tx)×H | Same-tx wrong HF/caps if future merge regresses | Process + optional static check |
| 9 | **P3** | Ops: oracle stale windows / plant-stale hygiene; floor vs `BAD_DEBT` realign | A065, A067, A102 | low×Z/C×H | Account liq latency; cleanup-band drift | Runbooks; no new silent admit |
| 10 | **P3** | Cache invalidation docs; optional `pool_sync_data` clear if post-leg reads added | A086, A104 | info×Z×H | Negligible today | Docs; code only if call graph changes |
| 11 | **P3** | Tests for dust-out vs large payload `min_out`; usage global reconcile rules | A056, A080, A108* | — | Evidence density | Harness + Certora beyond per-leg delta |
| 12 | **P4** | Docs UX: `swap_debt` refinance-at-cap; liq 256 symmetry; errors.md | A066, A062, A063 | info×Z×H | UX / fee | Docs / API symmetry |
| — | **R** | Must-not-regress defended stacks | A101 §9, A078, A104 | — | Regression → Critical | Checklist in §8 |

A108 later filed; use its PIN/CLOSE names over inferred rows. Authoritative ranking: `synthesis/FINAL.md`.

---

## 2. Tier maps (from A106 — authoritative bounds)

### 2.1 Single account — practical code residual

| Rank | Scenario | Ceiling | Kind |
|---|---|---|---|
| 1 | S1/S2 strategy dust-out (A048/A056) | ≈ swapped / withdrawn notional; HF-clipped if debt remains; debt-free `swap_collateral` ≈ full leg | Direct |
| 2 | S9 delegate drain | Full economic control | Accepted design |
| 3 | S7 `no_seize` strand | Interest growth until socialize | Availability → contingent |
| — | Closed L6–L10 | **0** | Defended |

### 2.2 Single market — practical residual

| Rank | Scenario | Ceiling | Kind |
|---|---|---|---|
| 1 | S3 non-SAC listing (A055) | ≤ \(\mathrm{TVL}_m\) | Contingent desync |
| 2 | S6 usage under-count then defaults (A080) | ≤ \(\mathrm{TVL}_m\) indirect | Contingent |
| 3 | S7 stranded liq → force socialize (A064) | ≤ \(D_{\mathrm{bad}}\) | Availability → contingent |

### 2.3 Protocol — only via trust roots

| Scenario | Ceiling |
|---|---|
| S4 hot owner / Sensitive≈0 (A009) | **All deployed value + NFT authority** |
| S5 hostile price aggregator / XOXNO (threat-model) | Protocol-wide wrong HF → market TVLs as bad debt |
| Malicious router owner alone | Σ account strategy notionals + router holdings — **not** share mint |

**Verdict line (A106):** Under intended deploy + SAC listing, **no** A080/A055/A048/A056/A064 residual alone yields protocol-total loss.

---

## 3. P0 — Deployment and trust-root gates (absolute ceiling)

These are **release blockers** per threat-model Known gaps. They dominate absolute max loss even when controller money paths are defended.

### RB-01 — Sensitive timelock floor + controller owner = governance

| Field | Value |
|---|---|
| **IDs** | A009 G2/G1; threat-model “sensitive timelock delay”; A106 S4; PRELIMINARY A009 |
| **Score drivers** | Severity critical-if-failed · Impact protocol-total · Fixability high (constant + deploy verify) |
| **Problem** | `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` (~1 min). Production target 120_960 (~7 days). Controller has **no** native delay — delay is ownership composition. |
| **Fix** | (1) Restore Sensitive floor via `UpgradeGov` when ready. (2) Deploy checklist: on-chain controller owner == governance; `min_delay` production-sized; no completed `TransferCtrlOwnership` to hot key; canceller/guardian roles known. (3) Optional: controller-side address/Wasm checks (A009 G3) for defense-in-depth if owner ≠ gov — **not** a substitute for (1)(2). |
| **Done when** | Threat-model release-blocker line closed; A009 audit sign-off items §12 verified on-chain. |
| **Does not fix** | Standalone aggregator/XOXNO owners (RB-02). |

### RB-02 — Swap-aggregator and XOXNO oracle ownership attestation

| Field | Value |
|---|---|
| **IDs** | Threat-model trust roots; A056 G-ROUTER-OWNER; A065 G-VAL-11; A101 §4.3; A106 S2/S5 |
| **Score drivers** | Critical deploy · Protocol (oracle) / Σ accounts (router) · Ops-high |
| **Problem** | Router owner: immediate `upgrade` / `sweep_balance` / `renounce_ownership`. XOXNO owner: immediate oracle Wasm/signer control. Both outside governance by design. |
| **Fix** | (1) Confirm aggregator admin is intended multisig (`AGGREGATOR_ADMIN` / post-deploy). (2) Never enable markets on XOXNO feeds while an individual key owns the oracle. (3) Treat router compromise as same economic class as missing controller `min_out` (RB-03) with higher adversary power. |
| **Done when** | Ownership attestation recorded in deploy runbook; no lone EOA on XOXNO for live feeds. |
| **Accepted residual** | Operational trade for router fee/referral admin — **not** closable by controller-only code without product redesign. |

### RB-03 — Controller quantitative `min_out` on strategy swaps ★ highest live code P0

| Field | Value |
|---|---|
| **IDs** | A048, A056 (F1–F7), A101 G-SLIP, A046/A047 Gap(1), A106 S1, PRELIMINARY A048/A056, threat-model “controller does not bound slippage” |
| **Score drivers** | Medium severity · Account direct extractable · Medium fix (API + harness) |
| **Problem** | `verify_router_output` only requires `received > 0`. Quantitative floor lives in opaque aggregator payload. Compromised router **or** `min_out = 1` can dust-out; sticks especially on `swap_collateral` (debt-free or spare HF). |
| **What stays defended** | Exact `amount_in` auth; discard return; `RouterOverspend`; leftover refund; `NoSwapOutput`; post-strategy HF; measured custody (A101 §3). |
| **Fix options (prefer in order)** | **(A)** Add explicit `min_out: i128` (or per-leg) on multiply / swap_debt / swap_collateral / cross-asset RDWC; assert measured Δ ≥ `min_out` in `verify_router_output` / caller. **(B)** Decode payload `total_min_out` in controller and assert vs measured Δ (still keep discard of router *return*). **(C)** Weaker: BPS band vs oracle — product-heavy; not required to close threat-model gap. Mirror pattern: `flash_position` already has controller `min_amount` (A045/A056). |
| **Path stickiness (do not “fix” wrong path first)** | Highest: `swap_collateral`. Often HF-blocked: bare `multiply`, `swap_debt`, cross-asset RDWC. |
| **Tests to ship with fix** | Adversarial router pays `1` with large auth in; payload `min_out` large vs dust; debt-free collateral swap; spare-HF multiply; regression that honest aggregator still passes. (A056 / A101 → A108.) |
| **Done when** | Controller rejects dust-out independent of aggregator honesty; STRIDE Tamper.4 “meet minimums” becomes true at controller boundary. |
| **Does not replace** | RB-02 (router owner can still upgrade to ignore payload-only floors if controller never checks). |

---

## 4. P1 — Market integrity and liquidation liveness

### RB-04 — SAC-only / non-lying token listing policy

| Field | Value |
|---|---|
| **IDs** | A055; A101 G-LIST / L2; A041/A044/A045/A051/A054/A058 outer bound; A106 S3; PRELIMINARY A055 |
| **Score drivers** | Medium · Market ≤ TVL · High (ops) / Medium if code gate |
| **Problem** | Measurement assumes SAC-like `balance` truth. Rebasing / balance-lying / hostile hooks cannot be made fully safe in controller code. |
| **Fix** | (1) Ops runbook: SAC-only listing equal to code review. (2) Never set `is_flashloanable` on non-exact assets (A044). (3) Optional: listing-time metadata checks / allowlist of known SAC wrappers. (4) Optional FoT hardening on strategy withdraw equality (A042 residual) — account-local only. |
| **Done when** | Listing checklist exists and is gate for `add_asset_to_spoke` / pool create; flashloanable policy documented. |
| **Not a code Critical** | Under honest SAC listing, money measurement stack is defended (A101). |

### RB-05 — ADR-0008 Option C: couple `no_seize` to freeze / block supply

| Field | Value |
|---|---|
| **IDs** | A064 G1; A102 G-VAL-1; A006 ratchet; STRIDE DoS.2; A106 S7; PRELIMINARY A064 |
| **Score drivers** | Medium · Market availability → contingent \(D_{\mathrm{bad}}\) · High fixability |
| **Problem** | `SeizureLeg` rejects `no_seize`; `BlockOnEntry` ignores it → users can still supply → unliquidatable set grows. Whole liquidation reverts if any planned seize leg is `no_seize`. Hatch: `force_socialize_bad_debt`. |
| **Fix** | Ship ADR-0008 **Option C**: setter enforces `no_seize ⇒ frozen` and/or `require_can_supply` rejects `no_seize`. Harness: cannot set `no_seize` without freeze; cannot grow supply under `no_seize`; liquidation still respects `SeizureLeg`. |
| **Done when** | Guardian cannot strand an expanding collateral class without also freezing entry. |
| **Anti-fix** | Do **not** gate seizure on `paused` (Aave-class protocol-wide liq halt — A064). |

### RB-06 — Spoke usage missing-row exit: reconcile + invariant

| Field | Value |
|---|---|
| **IDs** | A080; A103 §4.1/§8; A028 no admin rewrite; A099 redirect; A101 L4; A106 S6; PRELIMINARY A080 |
| **Score drivers** | Medium · Contingent market · Medium fixability |
| **Problem** | `apply_exit` no-ops if usage row missing → \(U=0\) while positions live → over-admission up to cap headroom. Intentional tolerance (Certora `usage_exit_without_usage_row_is_a_noop`) with capacity risk. |
| **Fix** | (1) Permissioned reconcile: recompute usage from Σ account scaled positions per `(spoke, hub, side)` (A028 gap). (2) Invariant/keeper assert usage vs position totals (detect under/over-count). (3) Optional: change exit missing-row from silent no-op to create-from-zero-then-decrement **only** if product accepts migration break — prefer reconcile first. (4) Keep persist-after-pool (A078) untouched. |
| **Done when** | Operators can heal \(U \ll P\) without waiting for TTL; monitoring detects divergence. |
| **Impact reminder** | No direct theft; HF/LTV still gate risk-increasing paths (A072). |

---

## 5. P2 — Soft integrity, DoS hygiene, engineering footguns

### RB-07 — Cap uncapped mutator / keeper Vecs

| Field | Value |
|---|---|
| **IDs** | A062 gaps (1)(2)(4); A015; A102 G-VAL-2; A106 S8; PRELIMINARY A062/A015 |
| **Score drivers** | Low · Fee/CPU only · High fixability |
| **Problem** | Views use `MAX_VIEW_INPUTS = 256`; mutator payment Vecs and keeper asset/account Vecs walk uncapped raw length. Aggregate-and-sum still prevents double-apply after collapse; position limits still ≤ 5. |
| **Fix** | Cap keeper Vecs (and optionally mutator payments) with `MAX_VIEW_INPUTS` or `MAX_KEEPER_INPUTS` **before** loops. Optional: align `liquidate` raw debt Vec with estimate’s 256. |
| **Anti-fix** | Do **not** replace aggregate-and-sum with hard-reject on user payments (documented design). |
| **Done when** | Oversized Vec fails closed cheaply; revenue claim still cannot redirect (A015). |

### RB-08 — `put_market_index` after every pool mutation merge

| Field | Value |
|---|---|
| **IDs** | A094; A104 §4.1; A077; A087; PRELIMINARY A094 |
| **Score drivers** | Low (future footgun) · Same-tx account · High process fix |
| **Problem** | Current merges call `put_market_index`. A **future** merge that forgets it leaves simulated index for later HF/caps in the same tx. |
| **Fix** | Code-review checklist + optional lint/static check: every pool mutation merge → `put_market_index` **and** `apply_leg_usage`. Document Cache rules (prices immutable; indexes overwrite). |
| **Today** | Not a demonstrated live drain (A094, A104, A106 S10). |
| **Done when** | New merge PRs fail review/CI without overwrite pair. |

### RB-09 — Optional strategy withdraw equality (FoT fail-closed)

| Field | Value |
|---|---|
| **IDs** | A042 residual (2); A055 adjacency |
| **Score drivers** | Info/low · Account-local · Medium |
| **Problem** | Strategy withdraw measures controller Δ but does not equality-assert vs pool `actual_amount` (unlike `borrow_into_controller`). |
| **Fix** | Optional `measured > 0` and/or `measured ≤ gross` after `withdraw_collateral_to_controller`. |
| **Priority** | Below RB-03/RB-04; only strengthens FoT listing edge. |

---

## 6. P3 — Ops runbooks, docs, tests, cache hygiene

### RB-10 — Oracle / floor ops hygiene

| Item | IDs | Action |
|---|---|---|
| Plant-stale supply shield | A065 §7.2, A102 G-VAL-9 | Ops: avoid dust supply of unpriceable assets into debted accounts; document recovery = refresh feed |
| Dual-source skew | A065 §7.3 | Size stale/deviation windows; listing discipline |
| `BAD_DEBT_USD_THRESHOLD` vs live floor | A067, A102 G-VAL-13 | When raising min-borrow floor, realign compile-time / cleanup band |
| Certora non-zero floor witness | A067 | Optional prover coverage |

### RB-11 — Cache / optimization documentation

| Item | IDs | Action |
|---|---|---|
| Invalidation matrix | A086, A104 P2 | Document: prices freeze; indexes overwrite; sync-data fill-once; spoke reset API |
| Clear `pool_sync_data` | A086 / A088* | Only if new post-leg safety reads appear |
| Keeper index prefetch | A087 | Budget only |
| Do **not** mid-tx refresh oracle | A087, ADR-0005 | Anti-remediation |

### RB-12 — Evidence densification (tests / Certora)

Derived from A056, A080, A103, A102 (A108 unfiled):

| Test / rule | Closes evidence for |
|---|---|
| Adversarial router dust vs large controller `min_out` (after RB-03) | A048/A056 |
| Global `Σ positions ≈ usage` per spoke asset (beyond per-leg delta) | A080 / A085* |
| Cap-at-limit Credit liquidation still succeeds (fee-only exit) | A084 / A080 anti-regression |
| `no_seize` setter coupling harness | A064 Option C |
| Keeper Vec over-length reject | A062/A015 |
| Optional: refund over-length / multi-hub keying | A070 |

### RB-13 — Documentation / UX polish (P4-class, listed under P3 for completeness)

| Item | IDs |
|---|---|
| Document `swap_debt` borrow-first refinance-at-cap | A066, A102 G-VAL-12 |
| `errors.md` #43 / #126 caller accuracy | A063, A067 |
| INV-RISK-01 prose: listing pre-pool; usage caps post-pool | A078 |
| STRIDE Tamper.4 “meet minimums” after RB-03 | A056 |
| Refund listing keyed by `debt.hub_id` | A070 |
| Liquidate raw Vec 256 symmetry with estimate | A062 G-VAL-3 |

---

## 7. Coverage debt (do not invent severity)

### 7.1 Closed since A101 (no longer backlog holes)

| ID | Status now | Implication for backlog |
|---|---|---|
| A042 | **defended** (info) | Do not claim withdraw measurement hole; optional RB-09 only |
| A043 | **defended** (info) | Borrow/repay measurement closed |
| A068 | **defended** (info) | SeizeMode exhaustiveness OK today; maintain before third variant |

### 7.2 Still thin / unfiled (pointers only)

| Gap | Notes |
|---|---|
| A060 cross-asset dust ↔ bad-debt | Unfiled; G-DUST (A053/A059) does **not** close it (A101 §8.3) |
| A069 Bytes size / opaque swap payload | A056 outscopes here; size/trust adjacent to RB-03 |
| A071 Blend approval deep-dive | A050 money-flow accepted; gate inventory still owed |
| A073–A075, A079, A081, A083, A085 | Wave 4/5 coverage debt; provisional peers do not add new mediums |
| A088–A093, A095–A098, A100 | Wave 6 thin; A104 authoritative for filed four only |
| A105, A107, A108, A109 | Wave 7 peers missing; A110 used threat-model + A106 directly |

**Rule:** Completing these files may **reorder low items** or add hygiene; they should not invent Criticals without new evidence that contradicts A101–A104 / A106.

---

## 8. Must-not-regress (anti-remediation checklist)

Removing or “simplifying” any of the following converts a defended surface into a Critical/High. Sourced from A101 §9, A078, A103, A104.

| # | Control | Owning peers | If weakened |
|---|---|---|---|
| 1 | `transfer_amount_measured` / `balance_delta_since` as custody oracles | A041, A058 | Unmeasured share mint / free credit |
| 2 | `measured == amount_received` on `borrow_into_controller` | A045–A047, A050, A082 | Strategy custody desync |
| 3 | Router return discard + `actual_spent ≤ amount_in` + leftover ≤ auth | A046–A048, A056 | Router lie / over-pull |
| 4 | Flash SAC brackets + exact `transfer_from` repay | A044 | Flash under-repay |
| 5 | `scale_seizures_to_received` before seize | A051–A053 | Over-seize |
| 6 | Credit fee via **absorb only** (never Transfer fee mint path) | A052, A053 | Unbacked fee shares |
| 7 | Overpay excluded from `credit_cash`; Δ-only refunds | A054 | Free credit / gross sweep |
| 8 | Migrate leftover → debt repay, not caller cash | A050 | Free cash on migrate |
| 9 | `require_external_recipient` on public borrow/withdraw | A057 | Stranger / stranded `to` |
| 10 | ADR-0003 directed mint/burn pairs | A059 | Free-share / debt-erasure |
| 11 | Persist spoke usage **after** pool success only | A078, A103 | Market-wide false occupancy / under-count |
| 12 | Credit fee-only usage exit (do not “full seize exit” without credit model) | A084, A053 | Liq blocked by supply cap or double-exit |
| 13 | Success-only `verified_hubs`; no mid-tx price refresh | A099, A087, ADR-0005 | Sticky fail / incoherent oracle |
| 14 | Aggregate-and-sum on user payment batches | A062 | Product break / false “dupe bug” fixes |
| 15 | Seizure not gated on `paused` | A064 | Protocol-wide liq halt |

---

## 9. Accepted residuals (document, do not “bugfix” as Critical)

| Residual | IDs | Why accepted |
|---|---|---|
| Delegate complete economic control | Threat-model, A057, A003, A106 S9 | Design; user-doc obligation |
| Router operational Ownable trade | Threat-model, RB-02 | Product ops vs timelock |
| Outbound pool→user unmeasured under SAC | A041, A042, A051, A057 | Intentional custody split |
| Missing-row exit no-op semantics (until RB-06) | A080 Certora pin | Migration tolerance; capacity soft-cap |
| Debt-free skip of post-pool solvency | A072, A099 | No borrow risk |
| Plant-stale prefer stuck over wrong | A065, ADR-0005 | Availability cost intentional |
| Sensitive floor temporary until UpgradeGov | A009 | Known release blocker — track as RB-01, not “missing only_owner” |
| Dust liquidator tax ≤ few units/leg | A053, A059 | Bounded PnL haircut |

---

## 10. Suggested remediation program (ordered work packages)

### WP-A — Pre-mainnet deploy gates (RB-01, RB-02)

1. Owner / delay / Sensitive attestation checklist (A009 §12).
2. Aggregator + XOXNO ownership attestation.
3. Block mainnet if Sensitive still 12 or owner ≠ governance.

**Exit criterion:** Threat-model deployment gates section closable for controller/gov composition.

### WP-B — Slippage control plane (RB-03 + tests)

1. Design `min_out` surface (entrypoint args vs decoded payload).
2. Implement assert on measured Δ in shared `swap.rs`.
3. Harness adversarial dust-out on `swap_collateral` first, then multiply/swap_debt.
4. Update STRIDE/threat-model wording once controller enforces floors.

**Exit criterion:** Dust-out fails closed even with malicious router Wasm **for the measured floor**; router owner residual becomes “cannot settle below controller floor” plus router-treasury ops risk only.

### WP-C — Listing + liquidation liveness (RB-04, RB-05)

1. SAC-only runbook + flashloanable policy.
2. ADR-0008 Option C setter + harness.
3. Operator playbook: clear `no_seize` vs `force_socialize_bad_debt`.

**Exit criterion:** Non-SAC listing is a deliberate exception with sign-off; `no_seize` cannot expand supply exposure.

### WP-D — Capacity integrity (RB-06)

1. Spec usage ↔ position invariant.
2. Admin/keeper reconcile tool.
3. Certora/global rule beyond per-leg delta (A085 debt).

**Exit criterion:** Detect and heal \(U \ll P\) / \(U \gg P\) without relying on silent no-op forever.

### WP-E — Hygiene batch (RB-07, RB-08, RB-10–RB-13)

1. Vec caps; `put_market_index` checklist/lint; ops floor/oracle notes; docs polish.

**Exit criterion:** Fee DoS bounded; future merge footgun process-gated; docs match live gates.

---

## 11. Cross-link matrix (synthesis → backlog)

| Synthesis source | Feeds backlog items |
|---|---|
| PRELIMINARY leading residuals | RB-01, RB-03, RB-04, RB-05, RB-06, RB-07, RB-08 |
| A101 §10 | RB-03, RB-04, optional flash denylist, RB-06; A042/A043 now closed |
| A102 §6 | RB-05 (P0 there), RB-07, RB-10, RB-13; defers min_out to money theme |
| A103 §8 | RB-06; A078 in §8 must-not-regress |
| A104 §9 | RB-08, RB-11; A080 via A099 → RB-06 |
| A106 §8 | Absolute order: deploy → min_out → SAC → no_seize → usage → Vec → A094 |
| Threat-model Known gaps | RB-01, RB-02, RB-03, delegate accepted |

**Framing reconciliation (not disagreement):** A046/A047 call money-flow **defended** while listing slippage as Gap(1); A048/A056 rate slippage **partial/medium**. A110 follows A101 §7.2 / A106: custody defended; quantitative slippage is the leading **economic** code residual (RB-03).

---

## 12. What this backlog explicitly does **not** recommend

1. Inventing Critical fund-theft bugs in measured custody, Credit fee absorb, flash pullback, or stranger `to` hijack without new evidence (A101 L6–L10 = 0).
2. Mid-transaction oracle price refresh “to fix staleness” (breaks ADR-0005).
3. Persisting spoke usage before pool success “to save reverts” (A078).
4. Gating seizure on `paused` to “fix” `no_seize` (A064).
5. Treating A080 as direct theft or A062 as inventory drain.
6. Replacing aggregate-and-sum with duplicate rejection on ordinary payments.
7. Claiming A105–A109 conclusions that were never filed.
8. Estimating calendar effort — only technical shape and dependency order (cloud-agent instruction).

---

## 13. Verdict

**Final remediation order for controller defense gaps:**

1. **Close deploy trust roots** (Sensitive floor, owner=governance, aggregator/XOXNO owners) — only path to protocol-total loss in the corpus.
2. **Enforce controller `min_out`** on strategy swaps — highest-probability direct account loss under correct deploy.
3. **SAC listing discipline + ADR-0008 Option C** — largest market/availability residuals that are not admin-key theft.
4. **Reconcile spoke usage (A080)** — soft-cap integrity; contingent supplier risk only.
5. **Vec caps + `put_market_index` process** — low severity, high fixability hygiene.
6. **Preserve defended stacks** — measurement, flash, Credit absorb, persist-after-pool, aggregate-and-sum, coherent price snapshots.

No novel Critical was synthesized beyond the already-documented trust-boundary classes. The backlog is complete for **actionable** residuals present in PRELIMINARY + A101–A104 + A106 + leading primaries; remaining unfiled agents are **evidence debt**, not license to invent exploits.
)
