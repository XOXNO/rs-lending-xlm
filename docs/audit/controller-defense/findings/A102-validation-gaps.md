# A102 — Synthesize validation gaps from A061–A075

- Agent: A102 (synthesis)
- Theme: T4 / T8
- Severity: medium (highest residual in-wave: A064 `no_seize`; otherwise low/info)
- Status: partial (wave incomplete; gaps below are from filed agents only)
- Paths: synthesis over `findings/A061`–`A075` that exist; primary code cited by those agents (`payments.rs`, `risk/validation.rs`, `positions/mod.rs` `FreezePolicy`, `context/oracle.rs`, `flash_position.rs`, `keepers.rs`, `views.rs`)
- Defense: Amount sign/zero/overflow (A061), spoke/hub entry gates (A063), listing + FreezePolicy matrix (A064 core), hard aggregator `prices()` freshness/sanity on valuation mutations (A065), position-slot cardinality (A066), min-borrow collateral floor in post-pool gates (A067/A072), flash `refund_assets` uniqueness/allowlist (A070), post-pool HF/LTV (A072), and intentional aggregate-and-sum (A062) form a dense T4 stack on risk-increasing money paths.
- Gap: (1) **A064 G1** — `no_seize` uncoupled from `frozen` / supply entry (ADR-0008 Option C not shipped). (2) **A062 / A015** — no hard length cap on mutator payment Vecs or keeper Vecs (views use 256). (3) **A065** — plant-stale-leg liquidation DoS; dual-source config skew; aggregator-as-SoT. (4) **A067** — keeper/passive grandfathering below floor; `BAD_DEBT_USD_THRESHOLD` desync from live floor. (5) **A066** — `swap_debt` borrow-first blocks full refinance at borrow cap (UX). (6) Hygiene/docs residuals (A062/A063/A070). (7) **Unfiled A068–A069, A071, A073–A075**.
- Impact: See §4 quantification. No filed A061–A075 finding demonstrates silent share mint, durable over-cap position maps, double-credit via duplicate Vecs, or undercollateralized origination on gated paths when the live aggregator enforces `failure`. Worst in-wave residual remains **liquidation unavailability** via `no_seize` (A064, medium) or plant-stale (A065, low). Vec DoS and floor/BAD_DEBT drift are Low. Swap slippage (A056) stays T3.
- Evidence: Filed findings A061–A067, A070, A072; peers A006, A008, A009, A015, A040, A045, A048, A050, A056; PRELIMINARY; INV-HALT-02, INV-ORACLE-01..04, INV-RISK-01/04, ADR-0005/0008, STRIDE DoS.2 / DoS.5 / DoS.9 / TB3.
- Opinion: Validation is **mostly defended** for fund safety. Prioritize ADR-0008 Option C. Treat A065 plant-stale, A062 Vec caps, and A067 BAD_DEBT desync as Low ops/hygiene. Document A066 `swap_debt` cap refinance; do not reorder borrow-first legs lightly. Re-run A102 when A068–A069 / A071 / A073–A075 land.

---

## 1. Method and coverage

### 1.1 Inputs

| Source | Role |
|---|---|
| `shared/COORDINATION.md` | No git; findings-only write |
| `synthesis/PRELIMINARY.md` | Leading residuals already call out A062/A015 |
| `shared/AGENT_MANIFEST.md` Wave 4 | Scope list A061–A075; A102 = synthesize validation gaps |
| Filed findings | A061, A062, A063, A064, A065, A066, A067, A070, A072 |
| Adjacent peers (not Wave 4 owners) | A006, A008, A009, A015, A040, A045, A048, A050, A056 — only to quantify cross-theme impact |

### 1.2 Wave 4 filing status (snapshot for this synthesis)

| ID | Manifest scope | File present? | Status | Severity |
|---|---|---|---|---|
| A061 | Amount sign / zero / overflow | **yes** | defended | info |
| A062 | Vec length / duplicate hub-asset | **yes** | partial | low |
| A063 | Spoke / hub existence & active | **yes** | defended | info |
| A064 | Listed-in-spoke + FreezePolicy | **yes** | defended / partial | medium (G1) |
| A065 | Oracle freshness / sanity on risk paths | **yes** | defended / partial | low (config / availability) |
| A066 | Position limits (max supply/debt slots) | **yes** | defended | info (+ low UX residual) |
| A067 | Min borrow collateral floor | **yes** | defended / partial | low |
| A068 | Mode / SeizeMode exhaustive handling | **no** | — | — |
| A069 | Callback `data` / swap Bytes size & trust | **no** | (A056 defers Bytes size here) | — |
| A070 | `refund_assets` uniqueness & allowlist | **yes** | defended | info |
| A071 | Blend pool approval on migrate | **no** | — | — |
| A072 | HF / post-pool risk gates | **yes** | defended | info |
| A073 | Interest model / market params read trust | **no** | — | — |
| A074 | Panic vs `assert_with_error` consistency | **no** | — | — |
| A075 | Fuzz/proptest vs validation surface | **no** | — | — |

**9 / 15** Wave 4 agents filed. Synthesis below is authoritative for those nine; for unfiled IDs, §7 records only **pointers from peers**, not independent gap claims.

---

## 2. What is defended (validation stack)

Layered fail-closed gates on risk-increasing user paths (compiled from A061–A064, A070, A072):

```text
Auth / pause / flash guard          (T1: A001–A007; out of A102 primary)
        ↓
Amount / empty / overflow           A061 — positive|MeansAll; checked_add; non-empty payments
        ↓
Vec shape                           A062 — aggregate-sum OR hard-reject (flash/migrate debt);
                                    position cardinality ≤ POSITION_LIMIT_MAX (≤5)
        ↓
Spoke + hub identity                A063 — active spoke + hub on entry; exits/liq open
        ↓
Listing + FreezePolicy              A064 — BlockOnEntry / AllowOnExit / SeizureLeg
        ↓
Oracle hard prices (valuation)      A065 — aggregator `prices()` stale/deviation/sanity; Cache snapshot
        ↓
Slot cardinality                    A066 — new keys only; ≤ max_* ∈ 1..=5; top-ups free after limit cut
        ↓
Flash refund confinement            A070 — listed Address, unique, ≤ max_supply, ≠ collateral
        ↓
Post-pool solvency                  A072 / A067 — LTV coll ≥ debt; HF ≥ 1 WAD; optional min-borrow floor
```

| Concern | Defense owner | Outcome if violated |
|---|---|---|
| Negative / wrap amounts | A061 | Revert; no silent wrap |
| Empty payment batch | A061 / A062 | `InvalidPayments` |
| Duplicate payment legs → double pool apply | A062 | Summed once per `HubAssetKey` |
| Duplicate flash snapshot / refund | A062 / A070 | Hard reject |
| Slot explosion | A066 (A062 notes) | `PositionLimitExceeded` `#109` |
| Unknown / deprecated spoke on entry | A063 | `#300` / `#301` |
| Unknown hub on entry | A063 | `#43 HubNotActive` |
| Unlisted / wrong-spoke asset on entry | A064 / A040 | `#307 AssetNotInSpoke` |
| paused / frozen on entry | A064 | `#315` / `#316` |
| Stale / insane price on valuation mutation | A065 | Aggregator `failure` → panic |
| Undersized LTV collateral while indebted | A067 / A072 | `MinBorrowCollateralNotMet` `#126` |
| Arbitrary refund token Client | A070 | `#307` before callback |
| Undercollateralized HF/LTV on gated path | A072 | `InsufficientCollateral` |

**Judgment:** Money-safety validation for filed scopes is strong. Residuals concentrate in **governance/availability** (A064 G1, A065 plant-stale) and **resource/ops hygiene** (A062, A067 BAD_DEBT drift, A066 UX), not accounting corruption.

---

## 3. Gap inventory (filed agents only)

### G-VAL-1 — `no_seize` without `frozen` / without blocking supply (A064 G1)

| Field | Value |
|---|---|
| Source | A064 §6 G1; ADR-0008 draft Option C; STRIDE DoS.2 |
| Status | **partial** |
| Severity | **medium** |
| Mechanism | `SeizureLeg` rejects only `no_seize`. `BlockOnEntry` ignores `no_seize`, so users may still supply that collateral. Liquidation seizes **all** planned collateral legs; one `no_seize` leg reverts the whole tx. |
| Operator hatch | `force_socialize_bad_debt` (owner) — availability recovery, not automatic liquidation |
| Related | A006 (guardian can only tighten — can set `no_seize`); A013/A051/A052 (seize modes); A001 (global pause orthogonal) |

### G-VAL-2 — Uncapped mutator / keeper Vec length (A062 + A015)

| Field | Value |
|---|---|
| Source | A062 gaps (1)(2)(4); A015; PRELIMINARY row A062/A015; STRIDE DoS.5 |
| Status | **partial** |
| Severity | **low** |
| Mechanism | Views enforce `MAX_VIEW_INPUTS = 256`. Mutator payment Vecs walk raw legs before/during aggregate; keepers (`update_indexes`, `claim_revenue`, `update_account_threshold`) have **no** length or empty check. |
| Not a bug | Aggregate-and-sum on payments is intentional (endpoints.md §6); flash/migrate-debt hard-reject duplicates correctly |
| Related | A008 (views defended); A050 (migrate coll/supply soft-dedup — hygiene only) |

### G-VAL-3 — Liquidate / estimate length asymmetry (A062)

| Field | Value |
|---|---|
| Source | A062 §3.1 |
| Status | residual / hygiene |
| Severity | **info → low** |
| Mechanism | `get_liquidation_estimate` caps at 256; `liquidate` does not. Post-aggregate unique hubs still ≤ account debt map (≤ `max_borrow_positions`). |
| Impact | UX / fee DoS only |

### G-VAL-4 — Migrate collateral/supply soft-dedup (A062 + A050)

| Field | Value |
|---|---|
| Source | A062 gap (3); A050 residual |
| Status | residual |
| Severity | **info** |
| Mechanism | Debt caps hard-reject duplicates; collateral/supply soft-dedup into withdraw list |
| Impact | No double mint; wasted Blend sweep work possible |

### G-VAL-5 — Latent hub deactivation / docs drift (A063)

| Field | Value |
|---|---|
| Source | A063 gaps (1)–(4) |
| Status | defended for funds; **info** operational |
| Mechanism | No public `set_hub_active(false)`; `#43` is mostly existence today. Strategy “exit-like” paths require hub active while bare withdraw/repay do not. `errors.md` #43 over-claims “all position … entry points”. |
| Impact | Liveness / docs; market halt today = spoke flags + deprecation (A064/A006) |

### G-VAL-6 — Credit delist new-slot / helper pairing footguns (A064 G2–G3)

| Field | Value |
|---|---|
| Source | A064 G2, G3; A026/A052 |
| Status | residual |
| Severity | **low** (G2 liveness) / **info** (G3) |
| Mechanism | Credit new receiver needs live listing; victim debit tolerates delist. `enforce_spoke_asset_flags(BlockOnEntry)` alone no-ops on missing listing — safe only because production pairs with `require_listed_*`. |
| Impact | Transfer seize still works; future callers must keep pairing |

### G-VAL-7 — Flash refund UX / coverage nits (A070)

| Field | Value |
|---|---|
| Source | A070 residuals |
| Status | defended for funds |
| Severity | **info** |
| Mechanism | Refund listing keyed by `debt.hub_id`; no pause/freeze on refund list (intentional); missing harness for over-length / multi-hub asymmetry |
| Impact | Deny refund declaration only; cannot invent credit or double-pay (delta semantics) |

### G-VAL-8 — Post-pool gate consumes Cache prices (A072 ↔ A065)

| Field | Value |
|---|---|
| Source | A072 gap; A065 §2–§3; INV-ORACLE |
| Status | defended at controller gate when aggregator enforces `failure` |
| Severity | **info** at A072; residual detail owned by **A065** |
| Mechanism | `require_post_pool_risk_gates` uses totals built from hard `prices()` only; debt-free skip intentional |
| Impact | Gated paths cannot leave HF < 1 / LTV breach when prices resolve; broken-oracle full exit of debt-free accounts intentional (A024) |

### G-VAL-9 — Plant-stale / supply-without-oracle liquidation DoS (A065 §7.2)

| Field | Value |
|---|---|
| Source | A065 §7.2; harness `audit_supply_stale_shield` |
| Status | **partial** (intentional non-pricing path with availability cost) |
| Severity | **low** |
| Mechanism | `supply` / `repay` skip pricing. Debted account can add a dust collateral leg while that feed is stale; later `liquidate` / `clean_bad_debt` / collateral withdraw must price **all** legs → `PriceFeedStale` until refresh. |
| Related | A064 G1 is a different liquidation-halt mechanism (flag vs oracle); both raise bad-debt **latency** |
| Impact | Account-scoped recovery DoS; not silent undercollateralized borrow (borrow still fail-closed) |

### G-VAL-10 — Dual-source skew inside configured stale windows (A065 §7.3)

| Field | Value |
|---|---|
| Source | A065 §7.3; STRIDE Tamper.1 residual |
| Status | config residual (not missing panic) |
| Severity | **low** (ops) |
| Mechanism | Wide per-source windows can admit lagging “fresh” legs; blend can inflate collateral within tolerance (~5% fixture skew) |
| Impact | Mis-admission bounded by configured deviation/stale policy; fix is ops sizing / listing, not a new controller assert |

### G-VAL-11 — Controller trusts aggregator pointer; no local timestamp re-check (A065 §7.1)

| Field | Value |
|---|---|
| Source | A065 §7.1; A009 / PRELIMINARY aggregator trust root |
| Status | **accepted design** |
| Severity | critical **if** aggregator compromised — already tracked as external trust root, not a novel controller hole |
| Mechanism | `cached_price` uses `feed.price` only; freshness/sanity live in aggregator `failure` |
| Impact | Compromised / mis-pointed aggregator ⇒ protocol-wide wrong HF/LTV/liq sizing |

### G-VAL-12 — `swap_debt` borrow-first blocks refinance at borrow cap (A066)

| Field | Value |
|---|---|
| Source | A066 gap (1); contrast A048 `swap_collateral` withdraw-first |
| Status | defended for cardinality; **info/low** UX residual |
| Severity | **low** (liveness) |
| Mechanism | New debt hub opens before old repay; at `max_borrow_positions` a full swap into a fresh hub reverts even though repay would free a slot |
| Impact | Workaround: repay then borrow, or refinance into already-held debt asset. No durable over-cap books |

### G-VAL-13 — Min-borrow floor: keeper grandfathering + `BAD_DEBT` desync (A067)

| Field | Value |
|---|---|
| Source | A067 gaps (1)(2); threat-model dust/floor drift; STRIDE DoS.9 |
| Status | **partial** (core gate defended) |
| Severity | **low** |
| Mechanism | (a) Keepers restamp LTV/params without evaluating the floor → indebted accounts can sit below floor until next risk-increasing action. (b) `BAD_DEBT_USD_THRESHOLD` is compile-time default; raising live floor desyncs dust cleanup band. |
| Impact | No silent origination under floor on gated paths. Residuals: temporary below-floor “zombies”, cleanup-band ops drift, Certora fixtures force floor `0` |

### Explicit non-gaps (filed)

| Claim | Owner | Verdict |
|---|---|---|
| Sign/zero/overflow holes on aggregates | A061 | **none** |
| Double-credit via duplicate payments | A062 | **defended** (sum) |
| Open exposure on unknown spoke/hub | A063 | **defended** |
| Unlisted position entry | A064 / A040 | **defended** |
| Stale/insane price silently accepted on borrow/liq/strategy finalize | A065 | **defended** (hard `prices`) |
| Soft `quotes()` used for on-chain HF | A065 | **defended** (views only) |
| Durable over-cap supply/borrow maps | A066 | **defended** |
| Mint debt under positive min-borrow floor on gated paths | A067 | **defended** |
| Arbitrary `refund_assets` Client | A070 | **defended** |
| Persist undercollateralized gated path | A072 | **defended** |

---

## 4. Impact quantification

Blast-radius axes: **funds (theft / silent mint)**, **account**, **market**, **protocol**, **availability**.

| Gap ID | Funds theft / silent mint | Account loss | Market | Protocol | Availability / DoS |
|---|---|---|---|---|---|
| G-VAL-1 (`no_seize`) | **None** (no share mint; socialize hatch exists) | Underwater holders cannot be liquidated via seize; debt grows with interest until socialize | **All accounts holding that collateral** blocked from liquidation while flag set; new supply can **grow** the set | Bad-debt **latency** until owner `force_socialize_bad_debt`; socialized loss ≤ unpaid debt of stranded accounts (not unbounded protocol TVL from this gap alone) | **High** for liquidation of affected collateral; STRIDE DoS.2 |
| G-VAL-2 (uncapped Vecs) | **None** | Caller burns own fees / hits Soroban budget | Keepers can spam accrual/claim loops — fee griefing, not redirect (A015: revenue → accumulator) | Instance TTL renew on keeper `Cache::new` even for empty Vec — rent annoyance, not drain | **Low** fee/CPU; STRIDE DoS.5 |
| G-VAL-3 (liq 256 asym) | None | Liquidator fee burn on huge raw debt Vec | None beyond budget | None | Low UX |
| G-VAL-4 (migrate soft dup) | None | Integrator confusion | None | None | Negligible |
| G-VAL-5 (hub latent) | None | Strategy unwind blocked if hub ever deactivated; bare repay/withdraw OK | Soft-close via flags still works | None | Operational |
| G-VAL-6 (Credit delist) | None | Credit seize to empty receiver fails; Transfer OK | Liquidation mode choice | None | Low liveness |
| G-VAL-7 (refund keying) | None | Refund declaration denied | None | None | UX |
| G-VAL-8 (HF ↔ oracle) | None when aggregator honest | Gate skip if debt-free | — | — | — |
| G-VAL-9 (plant-stale) | **None** (borrow still fail-closed) | Poisoned account: liquidate / clean / collateral exit blocked until feed refreshes | Contagion only if many accounts hold the same unpriceable dust leg | Bad-debt latency until oracle recovers (not force-socialize required) | **Low–medium** account recovery DoS; prefer stuck over wrong (ADR-0005) |
| G-VAL-10 (blend skew) | Economic mis-admission within config | Over-borrow vs honest fresh midpoint | Per-asset window mis-sizing | Valuation bias ≤ configured tolerance / stale envelope | Ops |
| G-VAL-11 (aggregator root) | If aggregator lies: wrong HF protocol-wide | Same | Same | Same as PRELIMINARY trust-root row | Deploy / upgrade discipline (A009) |
| G-VAL-12 (`swap_debt` at cap) | None | Full refinance into new hub blocked at max borrow slots | None | None | Low UX; repay+borrow workaround |
| G-VAL-13 (floor residuals) | None on gated origination | Passive below-floor after LTV restamp / floor raise | Dust cleanup band may not track live floor | Socialize/cleanup ops hygiene | Low; next risk-increasing action re-enforces |

### 4.1 Worst-case numeric framing (G-VAL-1)

- **Trigger cost:** Guardian (or owner via `edit_asset_in_spoke`) sets `no_seize=true` without `frozen` / without stopping supply.
- **Immediate effect:** Any liquidation whose seize plan includes that hub-asset reverts `#318`.
- **Growth:** Users can still `supply` that asset (`BlockOnEntry` ignores `no_seize`) → unliquidatable collateral set can increase.
- **Bound on protocol loss:** Not automatic theft. Loss materializes only if positions go underwater and owner later socializes — then suppliers of the affected market absorb bad debt **≤ unpaid debt of those accounts**, subject to pool socialization rules (see A014 / bad-debt theme). This gap increases **time-to-socialize** and **optional growth of exposure**, not a novel infinite mint.
- **Compare PRELIMINARY:** A055 (lying tokens) can desync ≤ market TVL; A064 G1 is availability / delayed socialization, different mechanism.

### 4.2 Worst-case numeric framing (G-VAL-2)

- **Work units:** O(n) over raw Vec length before aggregation / per keeper asset.
- **Cap after aggregate (mutators):** unique `HubAssetKey` legs still constrained by position limits (≤ 5 new slots) and existing maps.
- **Money effect:** Cannot open > `max_*_positions` slots; cannot double-apply same hub after aggregate; cannot redirect `claim_revenue` (A015).
- **Practical max loss:** Transaction fees paid by attacker (or griefed keeper caller) up to Soroban budget exhaustion — **not** protocol inventory.

### 4.3 Worst-case numeric framing (G-VAL-9 vs G-VAL-1)

| | G-VAL-1 `no_seize` | G-VAL-9 plant-stale |
|---|---|---|
| Scope | All liquidations touching that collateral **market-wide** | Accounts that hold the unpriceable leg |
| Growth | New supply of `no_seize` asset allowed | New supply of stale-priced asset allowed while feed down |
| Clearance | Owner clears flag or socializes | Oracle refresh (or remove position if other paths allow) |
| Severity | **medium** | **low** |

Both raise liquidation latency; G-VAL-1 is worse because a guardian action alone strands an entire collateral class without depending on oracle downtime.

### 4.4 Out-of-wave but validation-adjacent (do not double-count as A061–A075)

| Peer | Issue | Impact (from peer / PRELIMINARY) |
|---|---|---|
| A056 / A048 | No controller quantitative `min_out` | Swapped notional up to post-swap solvency; account-local |
| A069 (unfiled) | Bytes size / opaque swap payload | A056 already owns slippage; A069 should size/trust Bytes |
| A055 | Non-SAC / rebasing if listed | ≤ market TVL desync |
| A080 | `apply_exit` missing-row no-op | Cap under-count → temporary over-admission |
| A009 | Aggregator / oracle owners | Immediate price authority outside governance — outer bound for G-VAL-11 |

---

## 5. Cross-link matrix

### 5.1 Among filed Wave 4 agents

| | A061 | A062 | A063 | A064 | A065 | A066 | A067 | A070 | A072 |
|---|---|---|---|---|---|---|---|---|---|
| A061 | — | empty/MeansAll | — | — | — | — | — | amounts ≠ refund allowlist | — |
| A062 | amounts | — | — | flash len↔limits | — | slot math vs raw Vec | — | refund len/dupes | — |
| A063 | — | — | — | shared entry stack | — | — | — | listed refunds | entry before post-pool |
| A064 | — | — | hub/spoke→flags | — | — | — | — | refund skips freeze | flags before HF |
| A065 | — | — | — | — | — | — | floor uses prices | — | fills A072 oracle deferral |
| A066 | — | agrees dedup | — | — | — | — | — | flash len = hygiene | limits before risk |
| A067 | — | — | — | — | needs live prices | — | — | — | floor inside same chokepoint |
| A070 | — | Vec sibling | debt-hub listing | listing≠freeze | — | len ≤ max_supply | — | — | finalize after refund |
| A072 | — | — | — | — | consumes hard prices | — | owns floor call | — | — |

**Agreement:** No disagreement files among filed Wave 4 agents. A066 owns slot cardinality that A062 only adjacent-noted; A067 owns floor depth that A072 summarized.

### 5.2 Peers outside Wave 4

| Peer | Links into validation gaps |
|---|---|
| A006 | Guardian ratchet enables G-VAL-1 (`no_seize` tighten-only) |
| A008 | Contrasts defended view 256-cap vs G-VAL-2 |
| A009 / A029 | Aggregator pointer — outer trust root for G-VAL-11; protocol storage for floor |
| A012 | Third-party cannot open supply slots (complements A066) |
| A015 | Uncapped keeper Vecs (G-VAL-2); keepers skip min-borrow floor (G-VAL-13) |
| A040 | High-level listing; A064 owns FreezePolicy depth |
| A045 | Flash money-flow; refund allowlist → A070; price snapshot ↔ A065 |
| A048 | `swap_collateral` free-slot order contrasts A066 `swap_debt` |
| A050 | Migrate soft-dedup residual shared with A062 |
| A051 / A052 / A026 | Seize Transfer/Credit; Credit listing = A064 G2; Credit limit = A066 |
| A056 | Slippage / opaque Bytes — points at unfiled A069 |
| A001 | Global pause orthogonal to FreezePolicy |
| A007 | Post-guard listed-token residual on refunds |
| A024 / A023 / A032 | Cite A072 solvency chokepoint |
| A094 / A086 | Index cache ≠ oracle freshness |
| A099 | `verified_hubs` success-only memo (A063) |

### 5.3 PRELIMINARY alignment

Recommend PRELIMINARY / A110 backlog rows:

| ID | Issue | Impact quantification |
|---|---|---|
| A064 G1 | `no_seize` ̸⇒ `frozen`; supply still allowed | Liquidation halt for that collateral; bad-debt latency; hatch = force socialize |
| A065 §7.2 | Supply without oracle → plant stale leg | Account liquidate/clean blocked until feed recovers |
| A062/A015 | Uncapped mutator/keeper Vecs | Fee-funded compute DoS only |
| A067 | Live floor vs `BAD_DEBT_USD_THRESHOLD` drift | Cleanup-band ops; not silent under-floor mint |

---

## 6. Prioritized remediation backlog (validation theme)

Ordered by severity × leverage. Audit-only recommendations; no production edits in this agent.

| Priority | Action | Closes | Effort class |
|---|---|---|---|
| P0 | Ship ADR-0008 **Option C**: setter couples `no_seize ⇒ frozen` (and/or block `require_can_supply` while `no_seize`) | G-VAL-1 | Product + config setter + harness pins |
| P1 | Cap keeper Vecs (and optionally mutator payment Vecs) with `MAX_VIEW_INPUTS` or `MAX_KEEPER_INPUTS` **before** loops | G-VAL-2 | Small controller change + tests |
| P2 | Ops: tighten stale windows / listing dust for G-VAL-9; when raising min-borrow floor, realign `BAD_DEBT_USD_THRESHOLD` / cleanup band | G-VAL-9, G-VAL-10, G-VAL-13 | Ops / runbook |
| P3 | Document `swap_debt` refinance-at-cap UX (do not lightly reorder borrow-first) | G-VAL-12 | Docs |
| P4 | Align `liquidate` raw `debt_payments` with estimate’s 256 cap | G-VAL-3 | API symmetry |
| P5 | Docs: `errors.md` #43 / #126 caller lists; refund `debt.hub_id` keying; hub deactivation | G-VAL-5, G-VAL-7, A067 docs | Docs |
| P5 | Optional: Certora non-zero floor witness; hard-reject migrate coll/supply dupes; refund over-length harness | G-VAL-13, G-VAL-4, G-VAL-7 | Coverage |
| P5 | Do **not** add hub-active to withdraw/repay/liquidate; do **not** change aggregate-and-sum; do **not** duplicate aggregator stale checks; do **not** bolt floor onto keepers/repay without product decision | anti-regressions | Preserve INV-LIQ-01 / INV-ORACLE / dust cleanup |

**Explicitly out of P0 for “validation” label but still top protocol residual:** controller `min_out` (A056) — track under money-movement / A101 / A110, not as a missing A061 assert.

---

## 7. Unfiled Wave 4 slots — pointers only

These are **not** synthesized gap claims. They flag follow-up agents and peer hints so A102 is not mistaken for full Wave 4 coverage.

| Unfiled | Manifest intent | Peer / code pointer |
|---|---|---|
| A066 | Position limits | Largely inventoried in A062 §2.2 (`validate_bulk_position_limits`, `POSITION_LIMIT_MAX=5`, INV-RISK-04) — likely **defended** if filed |
| A067 | Min borrow collateral floor | Implemented inside `require_post_pool_risk_gates` (A072); governance setter in storage/tests — needs dedicated floor semantics / edge cases |
| A068 | Mode / SeizeMode exhaustive | A013 / A018 / A051 / A052 own much of the behavior; A068 should confirm match exhaustiveness / no silent `_` arms |
| A069 | Callback `data` / swap Bytes | A056 explicitly out-of-scopes Bytes size to A069; trust of opaque payload = slippage class |
| A071 | Blend pool approval on migrate | A050 money-flow; approval check needs dedicated gate inventory |
| A073 | Interest / market params read trust | Pool FFI / index trust; adjacent A077/A094 |
| A074 | Panic vs `assert_with_error` | Consistency / error-code surface; A072 already mixes both for min-borrow vs HF |
| A075 | Fuzz/proptest vs validation | Coverage map for A061–A074 negatives; A070 already notes missing over-length harness; A065 has strong oracle harness density |

When these land, **re-open A102** (or A110) to fold new residuals into §3–§4.

---

## 8. Test / evidence density (filed)

| Area | Strength | Gaps in evidence |
|---|---|---|
| Amounts / aggregate | Unit + harness duplicate/overflow | — |
| Position limits | INV-RISK-04; Certora; harness top-up after limit cut | Dedicated A066 file missing |
| FreezePolicy matrix | Unit `flags.rs` + harness pause/freeze/`no_seize` | No Certora named `enforce_spoke_asset_flags` (A064: acceptable) |
| Spoke/hub | Unit + harness deprecated liveness | Latent `is_active=false` mostly test-only |
| Oracle freshness/sanity | Aggregator unit + harness staleness/sanity/supply-stale-shield; Certora freshness* | Controller does not assert timestamps locally (by design) |
| Flash refunds | Dupes / overlap / unlisted harness | Over-length + multi-hub keying thin |
| Post-pool gates | INV-RISK-01; `solvency_gate_checked`; strategy Certora finals | Oracle residual detailed in A065 |

---

## 9. Verdict

**Wave 4 validation (filed subset): mostly defended for fund safety; one medium availability/governance residual; two low availability/hygiene residuals.**

1. **Highest actionable validation gap:** A064 / G-VAL-1 — `no_seize` uncoupled from freeze/supply (**medium**).
2. **Low residuals:** A062∪A015 / G-VAL-2 (uncapped Vecs); A065 / G-VAL-9–10 (plant-stale + config skew).
3. **Info / accepted:** hub docs (A063), refund keying (A070), liq 256 asymmetry, migrate soft-dedup, aggregator-as-SoT (A065/A072).
4. **Incomplete:** 8 of 15 Wave 4 IDs still unfiled; do not treat this file as exhaustion of Bytes (A069), Blend approval (A071), or fuzz coverage (A075). Position limits (A066) and min-borrow (A067) are partially covered by A062/A072 pending dedicated files.

Cross-links primary: **A061, A062, A063, A064, A065, A070, A072**; supporting **A006, A008, A009, A015, A040, A045, A050, A056**; synthesis peers **A101** (money), **A110** (backlog).)
