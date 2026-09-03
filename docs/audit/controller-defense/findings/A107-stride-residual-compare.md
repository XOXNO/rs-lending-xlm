# A107 — Residual STRIDE likelihood vs agent findings

- Agent: A107 (synthesis)
- Theme: T8
- Severity: medium (highest **raised** residuals: Tamper.4 controller slippage; DoS.2 `no_seize` coupling; new capacity class A080; trust-root Mediums confirmed)
- Status: partial (corpus incomplete vs A001–A110; ratings use filed findings + A020/A105/A106; unfiled scopes are not treated as proofs of absence)
- Paths: `STRIDE.md` (threat table + severity summary); `docs/explanation/threat-model.md` Known gaps; `synthesis/PRELIMINARY.md`; findings A020, A105, A106; supporting deep-dives cited per row
- Defense: See §3 (STRIDE residuals the audit **affirms** as correctly Low / correctly Medium)
- Gap: See §4–§6 (residuals this audit **raises**, **lowers**, **reframes**, or **adds** relative to STRIDE)
- Impact: Likelihood shifts change residual severity only where noted (matrix in STRIDE §Severity rubric). No novel Critical fund-theft class beyond documented trust-root / deployment failures (A106 S4/S5). Largest **code-path** likelihood raise is Tamper.4 at the **controller** layer (positivity-only `min_out`). Largest **new** Medium-class residual absent from STRIDE’s numbered threats is spoke-usage missing-row exit (A080).
- Evidence: STRIDE threat IDs Spoof.1–Elevation.10; A020; A105 §6 / §9 A107 guidance; A106 scenario map; PRELIMINARY leading residuals; A048/A056/A101 (slippage); A055/A101 (listing); A064/A102 (DoS.2); A080/A103 (capacity); A009 (Elevation.1/6/7, Tamper.7); A065 (Tamper.1/10, DoS.1); A007/A019/A030 (Tamper.5, Spoof.4); A001–A006/A012 (auth/pause/ratchet)
- Opinion: Treat STRIDE’s **controller-facing code-enforced Low residuals as largely correct** for auth, flash guard, measured custody under SAC, pause matrix, and flag ratchet. **Do not** treat Tamper.4’s “meet minimums → Residual Low” as describing the controller — that sentence overclaims; residual likelihood for strategy dust-out under reachable router malice or self-authorized `min_out=1` should be **Medium** at the controller trust boundary (still account-local impact). Keep Mediums for Spoof.1, Tamper.7/9/10, DoS.1/2/10, Elevation.1/6/7/10. **Add** an explicit capacity/integrity residual for A080 (not covered by any numbered STRIDE row at equal prominence). Prefer threat-model Known gaps + this audit over stale STRIDE Elevation.6 wording about a controller-owned aggregator upgrade path.

> **Corpus-complete addendum:** A001–A110 are complete. RAISE Tamper.4 / ADD
> A080 stand. Ranking: `synthesis/FINAL.md`.

---

## 1. Mission and method

**Mission:** Compare every **controller-relevant** STRIDE residual likelihood (and derived residual severity) to what this controller-defense audit actually found. Explicitly mark where the audit **raises**, **lowers**, or **confirms** residual ratings, and where STRIDE is silent on a leading audit residual.

**Method:**

1. Read `shared/COORDINATION.md`, `SEED.md`, `synthesis/PRELIMINARY.md`, README finding format, `AGENT_MANIFEST.md` (A107 = residual STRIDE likelihood vs agent findings).
2. Read `STRIDE.md` threat table + severity summary (controller / TB1–TB3 / TB6–TB8 / TB11–TB13 rows; governance/oracle/router rows that bound controller risk).
3. Read A020 (early STRIDE cross-check), A105 (Known-gaps compare; A107 guidance), A106 (max-loss bounds).
4. Cross-walk PRELIMINARY leading residuals and A101–A104 syntheses onto STRIDE IDs.
5. Apply STRIDE’s own likelihood definitions (High / Medium / Low) and severity matrix; do **not** invent a parallel scale.
6. Note corpus holes; do not claim “defended forever” for unfiled scopes.

No production Rust edited. No git operations (COORDINATION).

### 1.1 Likelihood vocabulary (from STRIDE)

| Likelihood | STRIDE definition (verbatim sense) |
|---|---|
| **High** | Reachable by any unprivileged actor under normal market conditions |
| **Medium** | Needs one attainable precondition: single dependency failure, single misconfiguration, or thin-liquidity market state |
| **Low** | Needs privileged-key compromise (hardened custody assumed), multi-party collusion, or engineered extreme state |

**Residual** = after code-enforced controls **plus** documented operational controls. This audit can raise residual likelihood when (a) a claimed ✅ control is weaker than stated, (b) an attainable ops misconfiguration is more reachable than STRIDE implies, or (c) a new mechanism is missing from STRIDE entirely.

### 1.2 Rating-change notation used below

| Tag | Meaning |
|---|---|
| **CONFIRM** | Audit agrees with STRIDE residual likelihood (and usually severity) |
| **RAISE** | Audit evidence implies higher residual likelihood and/or residual severity than STRIDE states |
| **LOWER** | Audit evidence implies lower residual likelihood/severity (rare; usually “attacks refuted” while structural gap stays) |
| **REFRAME** | Likelihood band may stay, but STRIDE **wording / layer / blast radius** is wrong or easy to misread |
| **ADD** | Leading audit residual with **no** matching numbered STRIDE threat at comparable prominence |
| **N/A-CTRL** | Primarily non-controller contract; included only for controller blast-radius coupling |

### 1.3 Inputs hierarchy when docs conflict

| Priority | Source | Why |
|---|---|---|
| 1 | Live code + deep-dive findings (A001–A100) | Ground truth for ✅ claims |
| 2 | Threat-model Known gaps + A105 | Explicitly tracks intentional residuals; A105 says prefer it over stale STRIDE Elevation.6 |
| 3 | STRIDE.md residual ratings | Authoritative baseline this file revises |
| 4 | PRELIMINARY / A101–A106 | Cross-wave ranking and quantified bounds |

---

## 2. Executive scoreboard

### 2.1 Where this audit changes residual judgment

| STRIDE ID | STRIDE residual (stated) | Audit judgment | Change | Driver findings |
|---|---|---|---|---|
| **Tamper.4** | Low (✅, route quality ◻) — claims “positive **and meet minimums**” | Controller enforces **positivity only**; quantitative min-out lives in untrusted aggregator payload | **RAISE** likelihood **Low → Medium** at controller/TB6; impact stays account-local (Medium severity under matrix if Impact Medium) | A048, A056, A101 G-SLIP, A105 K1, A106 S1, PRELIMINARY |
| **Tamper.3** | Low (✅) under measured receipt | Measured paths **defended for SAC/FoT inbound**; listing non-SAC/rebasing/lying tokens remains attainable governance precondition → market TVL | **RAISE** residual **ops likelihood** for **listing class** Low→**Medium** when non-SAC listing is considered “one misconfiguration”; keep Low for FoT-on-SAC measured inbound | A055, A101 G-LIST, A041/A058 |
| **DoS.2** | Medium (◻) — paused debt / `no_seize` can trap liq | Confirmed; **elevated prominence**: `no_seize` ̸⇒ `frozen` and still allows **supply** (ADR-0008 Option C unshipped) | **CONFIRM** Medium; **RAISE confidence / operator attention** (not a new severity band) | A064 G1, A102 G-VAL-1, A105, A106 S7 |
| **DoS.5** | Low (✅) — position caps + payload bounds | Position cardinality defended; **mutator/keeper Vec lengths uncapped** (views use 256) → fee/CPU grief | **RAISE** slightly within Low, or treat as **Low→Low+** hygiene; **not** inventory loss | A062, A015, A102 G-VAL-2 |
| **DoS.1** | Medium (◻+⚙) fail-closed prices block liq | Confirmed; plus **plant-stale** supply/repay path can strand an account’s later liq pricing | **CONFIRM** Medium; **ADD mechanism detail** (plant-stale) under same band | A065, A102 G-VAL-9 |
| **Elevation.6** | Medium (⚙) router owner immediate upgrade | Confirmed Medium; STRIDE text about governance→controller→router upgrade path is **stale** vs tree / threat-model | **CONFIRM** likelihood; **REFRAME** ownership story (threat-model / A009 > STRIDE sentence) | A009 G7, A056, A105 §6 |
| **Tamper.4 / route quality ◻** | Accepted route-quality residual | Distinct from missing **controller** min-out — STRIDE conflates “route quality” with “minimums met” | **REFRAME** — split: (a) economic route quality ◻; (b) controller quantitative floor **missing** | A056 F6, A105 |
| **A080 (no STRIDE ID)** | — | `apply_exit` no-op if usage row missing → soft-cap under-count | **ADD** — residual likelihood **Medium** (single misconfiguration / state anomaly + organic demand); impact Medium contingent (market TVL if later bad debt) | A080, A103, A020, PRELIMINARY, A106 S6 |
| **A094 (no STRIDE ID)** | — | Future merge omitting `put_market_index` | **ADD** — residual likelihood **Low** today (footgun, not live drain); engineering Medium if shipped broken | A094, A104, PRELIMINARY |

### 2.2 Where STRIDE Medium residuals stay Medium (confirmed)

| ID | Why audit confirms |
|---|---|
| Spoof.1 | Privileged-key residual; ops custody (A009 adjacency) |
| Tamper.7 | Sensitive floor still 12 ledgers (A009 G2 / Known gap D3) |
| Tamper.9 | FeedNature::Fundamental operator-asserted (ops; not refuted) |
| Tamper.10 | Point-in-time admission attestation; still outside Known gaps (A105) |
| DoS.10 | `renounce_ownership` on router/oracle (ops freeze) |
| Elevation.1 | Owner + delay floors; Sensitive=12 makes near-immediate after compromise (A009, A106 S4) |
| Elevation.7 | XOXNO owner immediate upgrade / signer control (A065, A105 D2) |
| Elevation.10 | Sanity-band kill switch (A065, A006 peer) |
| DoS.1 / DoS.2 | Availability trade-offs still live (above) |

### 2.3 Where STRIDE Low residuals stay Low (confirmed — dense defenses)

| Cluster | STRIDE IDs | Audit owners |
|---|---|---|
| Auth / delegate / third-party | Spoof.3, Elevation.4, Elevation.5 | A003, A005, A012 |
| Flash Wasm + reentrancy | Spoof.4, Tamper.5 | A019, A007, A030 |
| Pause / ratchet | Elevation.2 (+ pause matrix) | A001, A006 |
| Measured custody (SAC) | Tamper.3 inbound FoT class | A041, A044, A058, A101 §3 |
| Flash pullback / refunds / Credit fee | (supports Tamper.5 / money) | A044, A054, A052–A053 |
| Destination hijack closed | (Elevation.5 adjacent) | A057 |
| Post-pool HF on gated paths | (supports Elevation.5 / Tamper) | A072 |
| Position limits / min borrow floor core | DoS.5/9 core | A066, A067 |
| Spoke usage healthy entry / persist | (new A080 is the exception) | A076–A078, A082, A103 |
| Cache memo under current call sites | — | A086, A087, A099, A104 |

### 2.4 Net effect on STRIDE severity summary buckets

STRIDE partitions ~42 threats into Medium operational / Medium accepted / Low code / Low accepted.

| Bucket movement | Items |
|---|---|
| **Should move Low → Medium (controller-relevant)** | **Tamper.4** (at least for the “controller meets minimums” claim — residual Medium ops/dependency: router compromise **or** caller `min_out=1`) |
| **Should gain a new Medium accepted/ops row** | **Capacity.1 / A080** spoke-usage missing-row exit (proposed ID below) |
| **Stay Medium; raise operator priority** | DoS.2 (Option C), Tamper.7 (Sensitive=12), Elevation.6/7, Elevation.1 |
| **Stay Low; do not reopen** | Spoof.3–5, Tamper.5, Tamper.8 (migrate allowlist + measure), Elevation.2–5, Elevation.8–9 (within scope), most money closed classes (A101 L6–L10) |
| **Tamper.3** | Split rating: **Low** for measured SAC FoT; **Medium** residual if “non-SAC listed” counts as attainable misconfiguration (A055) — aligns with A101 ranking listing as money residual #2 |

---

## 3. Threat-by-threat residual compare (controller-relevant)

Each row: STRIDE residual → audit evidence → change tag → recommended residual likelihood.

### 3.1 Spoofing

#### Spoof.1 — Compromised privileged key acts as its role

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** |
| Audit | A009 maps owner/timelock assumptions; A106 S4 protocol-total under hot owner; custody is ops |
| Change | **CONFIRM** Medium |
| Notes | Likelihood remains Medium (key compromise precondition). Impact High → residual severity Medium on matrix. Highest absolute max-loss class when wiring fails (A106). |

#### Spoof.2 — Deployment identity confusion

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅+⚙)** |
| Audit | Constructor atomicity affirmed by threat-model / A009 deploy narrative; residual is standalone owner args |
| Change | **CONFIRM** Low |
| Notes | Controller/pool via governance path reduces front-run class; router/oracle/adapter remain manual (Known gaps D1/D2). |

#### Spoof.3 — Former / unlisted delegate acts

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅+⚙)** |
| Audit | A003 owner-or-delegate defended; A005 manager + NFT `granted_by` kill switches |
| Change | **CONFIRM** Low |
| Notes | Accepted **economic** power of a *live* delegate is Elevation/K2, not Spoof.3. |

#### Spoof.4 — Non-Wasm flash receiver

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A019 Wasm requirement defended on flash_loan / flash_position |
| Change | **CONFIRM** Low |

#### Spoof.5 — DeFindex vault impersonation

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | Adapter caller-keyed accounts (A105 K10 adjacency); no contrary finding |
| Change | **CONFIRM** Low |
| Scope | **N/A-CTRL** primary; controller sees ordinary caller |

---

### 3.2 Tampering

#### Tamper.1 — Manipulated / stale / partial external prices

| Field | Value |
|---|---|
| STRIDE residual | Table: **Low (✅)**; severity summary footnotes Medium operational (source independence config) |
| Audit | A065: hard `prices()` fail-closed on valuation mutations; dual-source / sanity / single-source = config residual; plant-stale is availability |
| Change | **CONFIRM** Low for *integrity under honest aggregator config*; **CONFIRM** Medium *ops* residual for source-independence / single-source (matches summary footnote, Elevation.7/K13) |
| Notes | Do not raise integrity residual to High — fail-closed holds (A072/A065). |

#### Tamper.2 — XOXNO signer subset bad prices

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅, ⚙ threshold > n/2)** |
| Audit | Outside controller deep-dive; A065 consumes aggregates; threshold majority is ops-only |
| Change | **CONFIRM** Low (code) / Medium ops if threshold mis-set — same as STRIDE ⚙ |
| Scope | **N/A-CTRL** primary |

#### Tamper.3 — Fee-on-transfer / rebasing / donation rewrite

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | Measured receipt defended (A041, A058, A101 §3). **A055** elevates **listed non-SAC / lying / rebasing** as market-TVL desync class — outer control is listing governance |
| Change | **REFRAME + conditional RAISE**: keep **Low** for FoT on SAC measured inbound; **RAISE to Medium** residual likelihood for “governance lists hostile token” (one misconfiguration) |
| Impact | ≤ market TVL (A106 S3) — Impact Medium → residual Medium if likelihood Medium |

#### Tamper.4 — Malicious router / venue overspend, retain, or lie about output

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅, route quality residual ◻)** — text claims exact-input auth, ignore returns, deltas, residue, “**positive and meet minimums**” |
| Audit | Exact-input, ignore returns, deltas, residue, positivity: **defended** (A046–A049, A056, A101). **Quantitative minimums at controller: absent** (`received > 0` only). Min-out in opaque aggregator payload / owner-upgradeable Wasm (A048, A056, A105 K1) |
| Change | **RAISE** residual likelihood **Low → Medium** for economic drain of in-flight strategy notional under (a) aggregator compromise **or** (b) caller-authorized dust min-out. **REFRAME** STRIDE “meet minimums” as **false at controller layer**. Keep ◻ route-quality as separate accepted residual |
| Impact | Account-local ≤ \(V(N_{in})\) / excess HF (A106 S1) — Impact Medium (account) not protocol High. Residual severity: **Medium** (matches A048/A056/A101 medium) |
| A020 link | A020 flagged aggregator trust roots; did not yet fully demote Tamper.4’s “minimums” claim — A105/A056 close that loop |

#### Tamper.5 — Reentrancy via flash / router callback

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A007 / A030 flash guard on monetary reentry; views intentionally unguarded (Known gap K9 — accepted) |
| Change | **CONFIRM** Low for monetary nested entry |
| Notes | Post-guard listed-token hooks are listing residual (A101 #5), not guard absence |

#### Tamper.6 — Interest-index manipulation

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | No contrary controller finding in filed corpus; accrual mostly pool/common |
| Change | **CONFIRM** Low (provisional; A073 unfiled) |

#### Tamper.7 — Malicious / accidental Wasm upgrade

| Field | Value |
|---|---|
| STRIDE residual | **Medium (✅+⚙)** — Sensitive currently ~1 minute |
| Audit | A009 G2 / Known gap D3: `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` confirmed live |
| Change | **CONFIRM** Medium; if anything, treat as **near-Elevation.1** until 120_960 restored |
| Notes | A106: Sensitive=12 after key compromise ≈ protocol-total reaction window |

#### Tamper.8 — Hostile Blend migration pool

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A050 / A101 G-BLEND: allowlist + measured + HF; migrator-local harm |
| Change | **CONFIRM** Low for protocol books; accepted trust on approved pool upgrade (Known gap K12) |

#### Tamper.9 — FeedNature::Fundamental operator-asserted

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** |
| Audit | Not refuted; A065 config residuals; A105 notes still outside Known gaps prominence |
| Change | **CONFIRM** Medium |
| Scope | Price-aggregator / ops; controller consumes result |

#### Tamper.10 — Admission attestation drift (`max_submission_age`)

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** |
| Audit | A105: still in STRIDE, not Known gaps; A065 cross-ref |
| Change | **CONFIRM** Medium; **RAISE documentation priority** (should appear near Known gaps) |
| Scope | Aggregator admin + oracle owner; controller fail-closed on stale reads still applies |

---

### 3.3 Repudiation

#### Repudiate.1 / .2 — Admin / user action deniability

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | Controller typed events; A033 event-after-persist defended; liquidation gross/net tags per STRIDE |
| Change | **CONFIRM** Low for controller mutations |
| Notes | Governance event blindness is Known gap K8 (ops) — related to Repudiate.1 at **governance** contract, not controller |

#### Repudiate.3 — XOXNO oracle emits no events

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙+◻)** |
| Audit | Outside controller; monitoring implication for price trust root |
| Change | **CONFIRM** Medium |
| Scope | **N/A-CTRL** |

---

### 3.4 Information disclosure

#### Info.1–.3 — Public state / MEV / timelock visibility

| Field | Value |
|---|---|
| STRIDE residual | **Low (◻)** |
| Audit | Accepted; liquidation MEV bounded by curve (no contrary finding) |
| Change | **CONFIRM** Low |

#### Info.4 — `quotes()` midpoint when `valid:false`

| Field | Value |
|---|---|
| STRIDE residual | **Low (◻)** — controller uses `prices()` |
| Audit | A065 / views narrative agrees mutating paths fail-closed via `prices()` |
| Change | **CONFIRM** Low for on-chain solvency; off-chain integrator footgun remains |

---

### 3.5 Denial of service

#### DoS.1 — Price outage blocks liquidation

| Field | Value |
|---|---|
| STRIDE residual | **Medium (◻+⚙)** |
| Audit | A065 fail-closed; Availability trade-off confirmed (A105 §8) |
| Change | **CONFIRM** Medium |
| Add-on | Plant-stale dust collateral leg (A065/A102) — same severity band, distinct mechanism |

#### DoS.2 — Paused debt / `no_seize` traps liquidation

| Field | Value |
|---|---|
| STRIDE residual | **Medium (◻)** |
| Audit | A064 G1: `no_seize` does **not** freeze or block supply; one seize leg fails whole liq; hatch `force_socialize` (A014) |
| Change | **CONFIRM** Medium likelihood; **RAISE** residual **importance** vs STRIDE’s brief mention — PRELIMINARY + A102 rank as leading validation residual |
| Impact | Availability → contingent socialization ≤ \(D_{bad}\) (A106 S7), not silent mint |

#### DoS.3 — False-alarm pause recovery delay

| Field | Value |
|---|---|
| STRIDE residual | **Low (◻+⚙)** |
| Audit | A001/A006 pause matrix; intentional ratchet |
| Change | **CONFIRM** Low |

#### DoS.4 — Governance deadlock

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | Mostly governance; A009 notes recovery paths exist |
| Change | **CONFIRM** Low |
| Scope | Mostly **N/A-CTRL** |

#### DoS.5 — Resource exhaustion / unbounded iteration

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** — cites position caps, payload bounds, chunked accrual |
| Audit | Position caps **defended** (A066). **Gap:** mutator payment Vecs and keeper asset Vecs lack hard length caps; views use 256 (A062, A015, A102) |
| Change | **RAISE within band** — residual still Low impact (fees only, A106 S8), but STRIDE overstates “payload bounds” completeness for **keeper/mutator** vectors |
| Recommended text | Residual Low (✅ positions; **partial** Vec caps ⚙/hygiene) |

#### DoS.6 — TTL expiry

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅+⚙)** |
| Audit | A017 / A034 renewal paths; views skip instance renew (rent-grief defense) |
| Change | **CONFIRM** Low |

#### DoS.7 — Oracle signer liveness → stale fail-closed

| Field | Value |
|---|---|
| STRIDE residual | Table **Low**; summary lists under Medium accepted |
| Audit | Couples to DoS.1; ops monitoring |
| Change | **CONFIRM** Medium *availability* residual when read as “prolonged signer outage” (align table with summary) — **REFRAME** doc inconsistency |
| Scope | Oracle + aggregator; controller fails closed |

#### DoS.8 — Utilization / cash limits

| Field | Value |
|---|---|
| STRIDE residual | **Low (◻)** |
| Audit | INV-HALT-03 / caps; no contrary theft finding |
| Change | **CONFIRM** Low |

#### DoS.9 — Dust griefing

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A067 floor defended on gated paths; BAD_DEBT vs live floor drift is Known gap K7 (ops) |
| Change | **CONFIRM** Low for dust origination; **CONFIRM** Medium ops footgun only for K7 cleanup band (not DoS.9 theft) |

#### DoS.10 — `renounce_ownership` freeze (router / XOXNO)

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** |
| Audit | A105 D1/D2 adjacency; A056 notes renounce |
| Change | **CONFIRM** Medium |
| Scope | **N/A-CTRL** primary; starves controller pricing / strategies |

---

### 3.6 Elevation of privilege

#### Elevation.1 — Governance-owner / delay floors

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** |
| Audit | A009; Sensitive=12; A106 S4 protocol-total if owner hot or delay unrestored |
| Change | **CONFIRM** Medium (likelihood); impact High → severity Medium on matrix — **dominates** remediation priority with Elevation.6/7 |

#### Elevation.2 — Guardian relaxes protection

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A006 flag ratchet; no immediate unpause |
| Change | **CONFIRM** Low |

#### Elevation.3 — Executor+Canceller / role erosion

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | Governance; no contrary controller finding |
| Change | **CONFIRM** Low |
| Scope | **N/A-CTRL** |

#### Elevation.4 — Delegate escalation (add delegates / outlive mandate)

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A003/A005 owner-only delegate mgmt + manager gate |
| Change | **CONFIRM** Low |
| Notes | Complete *economic* control of a live delegate is accepted (K2), not Elevation.4 |

#### Elevation.5 — Permissionless foreign risk

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | A012 third-party supply; A002 permissionless auth map; A057 `to` hijack closed |
| Change | **CONFIRM** Low |

#### Elevation.6 — Router-owner powers (immediate upgrade)

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** — some STRIDE text implies controller-owned upgrade route |
| Audit | Confirmed standalone trust root (A056, A105 D1). A009 G7 / A105 §6: prefer threat-model — **no** delayed governance path under default standalone owner |
| Change | **CONFIRM** Medium likelihood; **REFRAME** upgrade-path prose |
| Amplifies | Tamper.4 / A106 S2 |

#### Elevation.7 — XOXNO oracle owner

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙)** |
| Audit | A065, A105 D2, A106 S5 — protocol-wide valuation risk |
| Change | **CONFIRM** Medium |

#### Elevation.8 — Test-only entrypoints in release artifact

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅+⚙)** — ABI grep covers governance + price-aggregator only |
| Audit | Out of controller primary; wasm-testing-abi-check scope note stands |
| Change | **CONFIRM** Low with STRIDE’s own caveat on other artifacts |
| Scope | Release process |

#### Elevation.9 — DeFindex adapter authority

| Field | Value |
|---|---|
| STRIDE residual | **Low (✅)** |
| Audit | Supply-only isolation; A105 K10 no rescue |
| Change | **CONFIRM** Low |

#### Elevation.10 — Sanity-band kill switch (ORACLE role)

| Field | Value |
|---|---|
| STRIDE residual | **Medium (⚙, fail-closed ✅)** |
| Audit | A065 / A006 peer; Known gap K3 |
| Change | **CONFIRM** Medium (availability abuse, not mispricing widen) |

---

## 4. Residuals STRIDE under-specifies or omits (ADD)

These are **leading** in PRELIMINARY / A101–A106 but lack a first-class STRIDE threat ID at matching prominence.

### 4.1 Capacity.1 (proposed) — Spoke-usage missing-row exit (A080)

| Field | Value |
|---|---|
| Mechanism | `apply_exit` no-ops when usage row missing → recorded usage under-counts live positions → caps admit again from low baseline |
| Suggested residual likelihood | **Medium** (state anomaly / reconcile failure + organic demand — one attainable precondition) |
| Impact | No direct theft; over-admission ≤ cap headroom; realized loss only if later bad debt ≤ market TVL (A103, A106 S6) |
| STRIDE nearest neighbor | DoS.8 / INV-HALT-03 (caps) — but those assume usage integrity; **Tamper** on capacity accounting is closer than DoS |
| A020 | Already flagged under-highlight vs token measurement — **upheld** |
| Doc action | Add to STRIDE threat table + Known gaps (A105 backlog) |

### 4.2 Eng.1 (proposed) — Cache index refresh footgun (A094)

| Field | Value |
|---|---|
| Mechanism | New pool-merge path omits `put_market_index` → stale simulated index for same-tx HF/caps |
| Suggested residual likelihood | **Low** today (not demonstrated on live paths; A104) |
| Impact | Tx-local wrong gates if shipped; atomic with pool mutation |
| STRIDE nearest neighbor | Tamper.6 / cache not modeled |
| Doc action | Engineering checklist; optional STRIDE “implementation hazard” note — not a live Medium |

### 4.3 Liq.Post (structural) — Liquidation / bad-debt missing post-gates (Known gap K4)

| Field | Value |
|---|---|
| STRIDE | Not a numbered threat; partially implied under liquidation interaction notes |
| Audit | A105 K4 confirmed; concrete theft attacks refuted (A013/A051–A053); structural residual remains |
| Change | **ADD** as accepted/structural Medium-Low: likelihood Low for *exploit*, Medium for *missing defense depth* |
| Recommendation | Keep as Known gap; optional STRIDE Tamper/Elevation structural row — do not inflate to Critical |

### 4.4 Validation hygiene already touching DoS.5 / DoS.2

Covered in §3 (Vec caps, `no_seize` coupling, plant-stale). No separate IDs required if DoS.2/DoS.5 text is expanded.

---

## 5. Crosswalk: PRELIMINARY leading residuals → STRIDE

| PRELIMINARY / A106 residual | STRIDE mapping | Audit effect on residual rating |
|---|---|---|
| A048/A056 no controller `min_out` | Tamper.4 (+ Elevation.6 amplifier) | **RAISE** Tamper.4 residual likelihood |
| A055 non-SAC listing | Tamper.3 (listing face) | **RAISE** listing-class residual |
| A009 owner / Sensitive=12 | Elevation.1, Tamper.7, Spoof.1 | **CONFIRM** Medium; max-loss protocol-total (A106) |
| Aggregator / XOXNO standalone owners | Elevation.6, Elevation.7, DoS.10 | **CONFIRM** Medium |
| A080 usage missing-row | *(missing)* → Capacity.1 | **ADD** Medium |
| A064 `no_seize` coupling | DoS.2 | **CONFIRM** Medium; raise prominence |
| A062/A015 Vec caps | DoS.5 | **Partial RAISE** within Low |
| A094 cache footgun | *(missing)* → Eng.1 | **ADD** Low today |

---

## 6. Alignment with A020 / A105 / A106

| Source | Claim relevant to A107 | A107 stance |
|---|---|---|
| **A020** | Live code matches claimed auth/measure/flash/ratchet/pause; residuals in trust roots + A080 under-highlighted; cache footguns engineering | **Upheld**; this file quantifies likelihood shifts A020 only sketched |
| **A105** | Keep Medium for Tamper.10, DoS.2, Elevation.7; downgrade confidence in Tamper.4 “minimums”; prefer threat-model over stale Elevation.6 | **Executed** in §2–§3 |
| **A105** | No Known-gap vs audit fact conflict warranting disagreement file | **Agree** — A107 records rating **updates**, not peer disagreements |
| **A106** | Account max = S1 slippage; market = S3 listing / contingent S6–S7; protocol-total only S4/S5 | **Constrains impact** when raising Tamper.4 — raise **likelihood**, do **not** raise impact to protocol High |
| **A101** | Money mostly defended; G-SLIP + G-LIST lead | Feeds Tamper.4 / Tamper.3 raises |
| **A102** | Validation mostly defended; A064 medium | Feeds DoS.2 |
| **A103** | A080 only medium in T5 | Feeds Capacity.1 ADD |
| **A104** | Cache defended; A094 footgun | Feeds Eng.1 Low |

**No `disagreements/` file:** Peers agree on mechanisms; A107’s job is STRIDE residual **calibration**, not agent-vs-agent conflict.

---

## 7. Recommended STRIDE residual table edits (doc-only backlog)

For maintainers updating `STRIDE.md` after this audit (not applied here — findings-only write):

1. **Tamper.4 remediation / residual text:** Split controls into (a) overspend / delta / positivity ✅, (b) **controller quantitative min-out ❌** (Known gap K1), (c) route quality ◻. Set residual likelihood **Medium** until controller `min_out` (or equivalent) ships.
2. **Tamper.3:** Note SAC listing as ⚙ residual; measured FoT remains Low.
3. **DoS.2:** Explicitly cite `no_seize` ̸⇒ `frozen` / supply still open (ADR-0008 Option C).
4. **DoS.5:** Qualify Vec bounds — positions capped; keeper/mutator Vecs not.
5. **DoS.7:** Align table residual with summary (Medium availability under prolonged outage).
6. **Elevation.6:** Remove/fix controller-mediated delayed upgrade implication; match threat-model standalone owner.
7. **New Capacity.1 (A080):** Add under Tampering or a Capacity subsection; residual Medium (ops/state), impact Medium contingent.
8. **Severity summary counts:** After Tamper.4 move and Capacity.1 add, recount Medium operational / accepted buckets.
9. **Checklist “additional issues after threat model”:** Mark yes — this audit (A080, Option C prominence, Tamper.4 controller gap, Vec caps, plant-stale, A094).

---

## 8. Likelihood raise/lower summary (strict)

### 8.1 Raises (residual likelihood)

| ID | From | To | Condition |
|---|---|---|---|
| Tamper.4 | Low | **Medium** | Controller-layer settlement without quantitative min-out; reachable via router trust failure **or** dust payload min-out |
| Tamper.3 (listing face) | Low | **Medium** | Non-SAC / lying token listed (governance misconfiguration) |
| DoS.5 (completeness) | Low (absolute) | Low with **partial** control | Uncapped mutator/keeper Vecs — impact stays fee-only |
| DoS.2 | Medium | Medium (**↑ prominence**) | Option C unshipped — likelihood band unchanged |
| *(new)* Capacity.1 / A080 | — | **Medium** | Missing usage row + demand |
| Tamper.10 / Sensitive delay | Medium | Medium (**↑ priority**) | Still open; release blockers |

### 8.2 Lowers

| ID | From | To | Notes |
|---|---|---|---|
| — | — | — | **No STRIDE Medium was lowered to Low** by this audit for controller-relevant rows |
| Concrete liq theft under K4 | (implied fear) | **Low exploit likelihood** | Structural gap remains; attacks refuted — **LOWER exploit confidence**, not the “missing post-condition” fact |
| Tamper.4 *overspend / zero-out / share mint* subcases | (if misread as open) | **Low / closed** | Those sub-controls are ✅ — only min-out/economic drain raises |

### 8.3 Confirmed unchanged (high-value)

Spoof.3–5 Low; Tamper.5 Low; Tamper.7/9/10 Medium; DoS.1 Medium; Elevation.1/2/4/5/6/7/10 as rated; Info.* Low; Repudiate.1–2 Low.

---

## 9. Corpus / coverage caveats

| Hole | Effect on A107 |
|---|---|
| Unfiled Wave-4/5/6 IDs (see A102/A103/A104) | Do not invent STRIDE shifts from absence |
| A106 earlier noted A104 absent — **A104 now filed** | Eng.1 / A094 treated as Low footgun per A104 |
| Governance/oracle-internal threats lightly sampled | Confirm-only via A009/A065/A105 |
| STRIDE Tamper.1 table vs summary inconsistency | Called out; not “solved” |

Re-run A107 when A108–A110 land if they change leading residual ranking.

---

## 10. Verdict

**STRIDE’s residual model is directionally right for the controller:** code-enforced Lows on auth, flash reentrancy, pause/ratchet, and measured SAC custody hold under this audit. Medium residuals correctly cluster on **ops trust roots** (keys, Sensitive delay, router/oracle owners, attestation/FeedNature, fail-closed availability).

**This audit’s material corrections to residual likelihood are few but important:**

1. **Raise Tamper.4** — STRIDE’s “meet minimums → Residual Low” overclaims the **controller**; treat strategy dust-out as **Medium** residual likelihood (account-local impact), consistent with A048/A056/A101/A105/A106.
2. **Raise Tamper.3’s listing face** — non-SAC listing is a Medium ops residual (market TVL), not fully discharged by measured-receipt ✅.
3. **Add Capacity.1 (A080)** — Medium residual missing from the numbered STRIDE catalogue.
4. **Confirm and elevate attention on DoS.2** (`no_seize` coupling) without changing the Medium band.
5. **Do not raise protocol-total likelihood** from money-path residuals alone — A106 still requires Elevation.1 / Tamper.7 / Elevation.7-class trust failure for protocol-total loss.

**Net:** The audit **raises** a small set of residuals (Tamper.4, listing Tamper.3, A080) and **confirms** STRIDE’s Medium trust-root story; it does **not** broadly invalidate STRIDE’s Low code-enforced bucket for controller defenses.
)
