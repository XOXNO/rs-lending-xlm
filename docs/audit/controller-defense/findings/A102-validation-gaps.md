# A102 — Synthesize validation gaps from A061–A075

- Agent: A102 (synthesis)
- Theme: T4 / T8
- Severity: medium (highest residual in-wave: A064 `no_seize`; otherwise low/info)
- Status: partial (wave incomplete; gaps below are from filed agents only)
- Paths: synthesis over `findings/A061`–`A075` that exist; primary code cited by those agents (`payments.rs`, `risk/validation.rs`, `positions/mod.rs` `FreezePolicy`, `flash_position.rs`, `keepers.rs`, `views.rs`)
- Defense: Amount sign/zero/overflow (A061), spoke/hub entry gates (A063), listing + FreezePolicy matrix (A064 core), flash `refund_assets` uniqueness/allowlist (A070), post-pool HF/LTV/min-borrow floor (A072), and intentional aggregate-and-sum + position-limit cardinality (A062) form a dense T4 stack on risk-increasing money paths.
- Gap: (1) **A064 G1** — `no_seize` uncoupled from `frozen` / supply entry (ADR-0008 Option C not shipped). (2) **A062 / A015** — no hard length cap on mutator payment Vecs or keeper Vecs (views use 256). (3) Hygiene/docs residuals: liquidate vs estimate 256 asymmetry, migrate soft-dedup, hub deactivation latent, refund debt-hub keying, errors.md #43 over-claim. (4) **Unfiled A065–A069, A071, A073–A075** — wave incomplete; adjacent peers already flag A069 Bytes trust (A056) and A066/A067 content inside A062/A072.
- Impact: See §4 quantification. No filed A061–A075 finding demonstrates silent share mint, double-credit via duplicate Vecs, or undercollateralized exit of a gated risk-increasing path. Worst in-wave residual is **liquidation unavailability** for holders of a `no_seize` collateral until owner socializes (bad-debt latency, market-scoped). Next is fee/CPU DoS from uncapped Vecs (account/tx budget only). Quantitative swap slippage remains a **T3** trust-root gap (A056 / PRELIMINARY), not owned by filed validation agents.
- Evidence: Filed findings A061, A062, A063, A064, A070, A072; peers A006, A008, A015, A040, A045, A050, A056; PRELIMINARY leading residuals row A062/A015; INV-HALT-02, INV-RISK-01/04, ADR-0008, STRIDE DoS.2 / DoS.5.
- Opinion: Treat validation as **mostly defended** on money-safety axes (amounts, listing, entry/exit asymmetry, flash refund confinement, post-pool solvency). Prioritize product decision on ADR-0008 Option C (`no_seize ⇒ frozen`) over more call-site checks. Length-cap hygiene is real but Low severity. Re-run A102 when A065–A069 / A071 / A073–A075 land.

---

## 1. Method and coverage

### 1.1 Inputs

| Source | Role |
|---|---|
| `shared/COORDINATION.md` | No git; findings-only write |
| `synthesis/PRELIMINARY.md` | Leading residuals already call out A062/A015 |
| `shared/AGENT_MANIFEST.md` Wave 4 | Scope list A061–A075; A102 = synthesize validation gaps |
| Filed findings | A061, A062, A063, A064, A070, A072 |
| Adjacent peers (not Wave 4 owners) | A006, A008, A015, A040, A045, A050, A056 — only to quantify cross-theme impact |

### 1.2 Wave 4 filing status (snapshot for this synthesis)

| ID | Manifest scope | File present? | Status | Severity |
|---|---|---|---|---|
| A061 | Amount sign / zero / overflow | **yes** | defended | info |
| A062 | Vec length / duplicate hub-asset | **yes** | partial | low |
| A063 | Spoke / hub existence & active | **yes** | defended | info |
| A064 | Listed-in-spoke + FreezePolicy | **yes** | defended / partial | medium (G1) |
| A065 | Oracle freshness / sanity on risk paths | **no** | — | — |
| A066 | Position limits (max supply/debt slots) | **no** | (covered inside A062 §2.2 / INV-RISK-04) | — |
| A067 | Min borrow collateral floor | **no** | (mentioned in A072 defense) | — |
| A068 | Mode / SeizeMode exhaustive handling | **no** | — | — |
| A069 | Callback `data` / swap Bytes size & trust | **no** | (A056 defers Bytes size here) | — |
| A070 | `refund_assets` uniqueness & allowlist | **yes** | defended | info |
| A071 | Blend pool approval on migrate | **no** | — | — |
| A072 | HF / post-pool risk gates | **yes** | defended | info |
| A073 | Interest model / market params read trust | **no** | — | — |
| A074 | Panic vs `assert_with_error` consistency | **no** | — | — |
| A075 | Fuzz/proptest vs validation surface | **no** | — | — |

**6 / 15** Wave 4 agents filed. Synthesis below is authoritative for those six; for unfiled IDs, §7 records only **pointers from peers**, not independent gap claims.

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
Flash refund confinement            A070 — listed Address, unique, ≤ max_supply, ≠ collateral
        ↓
Post-pool solvency                  A072 — LTV coll ≥ debt; HF ≥ 1 WAD; optional min-borrow floor
```

| Concern | Defense owner | Outcome if violated |
|---|---|---|
| Negative / wrap amounts | A061 | Revert; no silent wrap |
| Empty payment batch | A061 / A062 | `InvalidPayments` |
| Duplicate payment legs → double pool apply | A062 | Summed once per `HubAssetKey` |
| Duplicate flash snapshot / refund | A062 / A070 | Hard reject |
| Slot explosion | A062 (+ unfiled A066) | `PositionLimitExceeded` |
| Unknown / deprecated spoke on entry | A063 | `#300` / `#301` |
| Unknown hub on entry | A063 | `#43 HubNotActive` |
| Unlisted / wrong-spoke asset on entry | A064 / A040 | `#307 AssetNotInSpoke` |
| paused / frozen on entry | A064 | `#315` / `#316` |
| Arbitrary refund token Client | A070 | `#307` before callback |
| Undercollateralized borrow/withdraw/strategy | A072 | `InsufficientCollateral` / min-borrow |

**Judgment:** Money-safety validation for filed scopes is strong. Residuals concentrate in **governance/availability** (A064 G1) and **resource hygiene** (A062), not accounting corruption.

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

### G-VAL-8 — Post-pool gate oracle dependency (A072)

| Field | Value |
|---|---|
| Source | A072 gap; INV-ORACLE |
| Status | defended at controller gate; upstream trust |
| Severity | **info** at A072 scope (oracle policy owned by unfiled A065 / oracle theme) |
| Mechanism | `require_post_pool_risk_gates` uses Cache prices; fail-closed oracle is upstream |
| Impact | Gated paths cannot leave HF < 1 / LTV breach when prices resolve; broken-oracle full exit of debt-free accounts is intentional (A024 harness) |

### Explicit non-gaps (filed)

| Claim | Owner | Verdict |
|---|---|---|
| Sign/zero/overflow holes on aggregates | A061 | **none** |
| Double-credit via duplicate payments | A062 | **defended** (sum) |
| Open exposure on unknown spoke/hub | A063 | **defended** |
| Unlisted position entry | A064 / A040 | **defended** |
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
| G-VAL-8 (oracle upstream) | If oracle lies: risk totals wrong — **owned by oracle trust root**, not missing HF assert | Wrong HF within fail-open oracle policy | Market-wide if oracle compromised | Same | Fail-closed preferred (INV-ORACLE) |

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

### 4.3 Out-of-wave but validation-adjacent (do not double-count as A061–A075)

| Peer | Issue | Impact (from peer / PRELIMINARY) |
|---|---|---|
| A056 / A048 | No controller quantitative `min_out` | Swapped notional up to post-swap solvency; account-local |
| A069 (unfiled) | Bytes size / opaque swap payload | A056 already owns slippage; A069 should size/trust Bytes |
| A055 | Non-SAC / rebasing if listed | ≤ market TVL desync |
| A080 | `apply_exit` missing-row no-op | Cap under-count → temporary over-admission |

---

## 5. Cross-link matrix

### 5.1 Among filed Wave 4 agents

| | A061 | A062 | A063 | A064 | A070 | A072 |
|---|---|---|---|---|---|---|
| A061 | — | empty payments / MeansAll order | — | — | amounts not refund allowlist | — |
| A062 | complement amounts | — | — | flash length uses position limits | refund length/dupes | position limits feed risk surface |
| A063 | — | — | — | shared `require_listed_unhalted` stack | refund uses `require_listed_active_config` | entry before post-pool |
| A064 | — | — | hub/spoke before flags | — | refund skips freeze (intentional) | flags before money; HF after |
| A070 | — | sibling Vec hygiene | debt-hub listing | listing ≠ freeze on refunds | — | finalize after refund |
| A072 | — | — | — | — | — | — |

**Agreement:** No disagreement files needed among A061/A062/A063/A064/A070/A072. Intentional asymmetries (entry vs exit hub checks; seize vs pause; refund vs supply flags; sum vs reject duplicates) are consistent across findings.

### 5.2 Peers outside Wave 4

| Peer | Links into validation gaps |
|---|---|
| A006 | Guardian ratchet enables G-VAL-1 (`no_seize` tighten-only) |
| A008 | Contrasts defended view 256-cap vs G-VAL-2 |
| A015 | Same uncapped keeper Vec residual as A062 |
| A040 | High-level listing; A064 owns FreezePolicy depth |
| A045 | Flash money-flow; defers refund allowlist to A070 |
| A050 | Migrate soft-dedup residual shared with A062 |
| A051 / A052 / A026 | Seize Transfer/Credit; Credit listing residual = A064 G2 |
| A056 | Slippage / opaque Bytes — points at unfiled A069 |
| A001 | Global pause orthogonal to per-asset FreezePolicy |
| A007 | Post-guard listed-token residual on refunds (A070 notes) |
| A024 / A023 / A032 | Cite A072 as solvency chokepoint |
| A099 | `verified_hubs` success-only memo (A063) |

### 5.3 PRELIMINARY alignment

PRELIMINARY already lists **A062/A015** as a leading residual (fee-funded compute DoS). This synthesis **elevates A064 G1** as the highest-severity **in-wave validation** residual (medium), which PRELIMINARY’s early table did not yet surface (A064 filed later). Recommend PRELIMINARY / A110 backlog add:

| ID | Issue | Impact quantification |
|---|---|---|
| A064 G1 | `no_seize` ̸⇒ `frozen`; supply still allowed | Liquidation halt for that collateral; bad-debt latency; hatch = force socialize |

Keep A062/A015 as Low hygiene.

---

## 6. Prioritized remediation backlog (validation theme)

Ordered by severity × leverage. Audit-only recommendations; no production edits in this agent.

| Priority | Action | Closes | Effort class |
|---|---|---|---|
| P0 | Ship ADR-0008 **Option C**: setter couples `no_seize ⇒ frozen` (and/or block `require_can_supply` while `no_seize`) | G-VAL-1 | Product + config setter + harness pins |
| P1 | Cap keeper Vecs (and optionally mutator payment Vecs) with `MAX_VIEW_INPUTS` or `MAX_KEEPER_INPUTS` **before** loops | G-VAL-2 | Small controller change + tests |
| P2 | Align `liquidate` raw `debt_payments` with estimate’s 256 cap | G-VAL-3 | API symmetry |
| P3 | Docs: `errors.md` #43 caller list; `endpoints.md` refund `debt.hub_id` keying; hub deactivation story | G-VAL-5, G-VAL-7 | Docs only |
| P4 | Optional: hard-reject migrate coll/supply duplicates; harness over-length refunds / multi-hub refund | G-VAL-4, G-VAL-7 | UX / coverage |
| P5 | Do **not** add `require_hub_active` to withdraw/repay/liquidate | anti-G-VAL-5 | Preserve INV-LIQ-01 |
| P5 | Do **not** change payment aggregate-and-sum without API break | anti-regression | Documented design |

**Explicitly out of P0 for “validation” label but still top protocol residual:** controller `min_out` (A056) — track under money-movement / A101 / A110, not as a missing A061 assert.

---

## 7. Unfiled Wave 4 slots — pointers only

These are **not** synthesized gap claims. They flag follow-up agents and peer hints so A102 is not mistaken for full Wave 4 coverage.

| Unfiled | Manifest intent | Peer / code pointer |
|---|---|---|
| A065 | Oracle freshness / sanity on risk paths | A072 defers to INV-ORACLE; price aggregator trust root in threat-model / PRELIMINARY |
| A066 | Position limits | Largely inventoried in A062 §2.2 (`validate_bulk_position_limits`, `POSITION_LIMIT_MAX=5`, INV-RISK-04) — likely **defended** if filed |
| A067 | Min borrow collateral floor | Implemented inside `require_post_pool_risk_gates` (A072); governance setter in storage/tests — needs dedicated floor semantics / edge cases |
| A068 | Mode / SeizeMode exhaustive | A013 / A018 / A051 / A052 own much of the behavior; A068 should confirm match exhaustiveness / no silent `_` arms |
| A069 | Callback `data` / swap Bytes | A056 explicitly out-of-scopes Bytes size to A069; trust of opaque payload = slippage class |
| A071 | Blend pool approval on migrate | A050 money-flow; approval check needs dedicated gate inventory |
| A073 | Interest / market params read trust | Pool FFI / index trust; adjacent A077/A094 |
| A074 | Panic vs `assert_with_error` | Consistency / error-code surface; A072 already mixes both for min-borrow vs HF |
| A075 | Fuzz/proptest vs validation | Coverage map for A061–A074 negatives; A070 already notes missing over-length harness |

When these land, **re-open A102** (or A110) to fold new residuals into §3–§4.

---

## 8. Test / evidence density (filed)

| Area | Strength | Gaps in evidence |
|---|---|---|
| Amounts / aggregate | Unit + harness duplicate/overflow | — |
| Position limits | INV-RISK-04; Certora; harness top-up after limit cut | Dedicated A066 file missing |
| FreezePolicy matrix | Unit `flags.rs` + harness pause/freeze/`no_seize` | No Certora named `enforce_spoke_asset_flags` (A064: acceptable) |
| Spoke/hub | Unit + harness deprecated liveness | Latent `is_active=false` mostly test-only |
| Flash refunds | Dupes / overlap / unlisted harness | Over-length + multi-hub keying thin |
| Post-pool gates | INV-RISK-01; `solvency_gate_checked`; strategy Certora finals | Oracle band ownership → A065 |

---

## 9. Verdict

**Wave 4 validation (filed subset): mostly defended for fund safety; one medium availability/governance residual; one low DoS hygiene residual.**

1. **Highest actionable validation gap:** A064 / G-VAL-1 — `no_seize` uncoupled from freeze/supply (medium).
2. **Confirmed Low residual:** A062∪A015 / G-VAL-2 — uncapped Vecs (fee DoS only).
3. **Info residuals:** hub docs/latency (A063), refund keying/coverage (A070), liq 256 asymmetry, migrate soft-dedup, oracle upstream (A072).
4. **Incomplete:** 9 of 15 Wave 4 IDs unfiled; do not treat this file as exhaustion of oracle (A065), Bytes (A069), Blend approval (A071), or fuzz coverage (A075).

Cross-links primary: **A061, A062, A063, A064, A070, A072**; supporting **A006, A008, A015, A040, A045, A050, A056**; synthesis peers **A101** (money), **A110** (backlog).
)
