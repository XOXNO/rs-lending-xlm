# A101 — Undefended money-movement gaps (synthesis of A041–A060)

- Agent: A101 (synthesis)
- Theme: T3 / T8
- Severity: medium (highest residual class in corpus; no novel critical)
- Status: partial (corpus mostly defended; residuals clustered and quantified below)
- Paths: synthesis over `docs/audit/controller-defense/findings/A041*.md` … `A059*.md`; `synthesis/PRELIMINARY.md`; threat-model known gaps; INV-ACCT / INV-STRAT / INV-LIQ / INV-FLASH
- Defense: See §3 (defended money surfaces inventory)
- Gap: See §4 (undefended / partial residuals with quantified blast radius)
- Impact: See §4 impact columns and §5 loss-class summary; largest live residuals are **account-local strategy slippage** (≤ swapped notional / excess HF) and **listing-trust non-SAC tokens** (≤ that market’s TVL). No corpus finding demonstrates silent protocol-wide share mint, unmeasured controller custody credit, or stranger redirection of another account’s cash without INV-AUTH-02.
- Evidence: Peer findings A041–A059 (A042/A043/A060 absent); PRELIMINARY leading residuals; threat-model §§slippage / non-standard tokens / aggregator owner; INV-ACCT-03, INV-STRAT-01/02, INV-LIQ-03, INV-FLASH-01/02, ADR-0003/0011/0013/0019/0020
- Opinion: Wave-3 money movement is **substantially defended** at the custody and share-book layers. The remaining undefended surface is not a missing `transfer_amount_measured` call — it is (1) quantitative slippage living only in an out-of-governance aggregator trust root, and (2) governance listing of non-SAC / balance-lying tokens. Treat those as the remediation backlog; do not reopen defended flash pullback, Credit absorb-vs-mint, or delta-only refund designs.

---

## 0. Method and corpus coverage

### 0.1 Method

1. Read `shared/COORDINATION.md`, `SEED.md`, `synthesis/PRELIMINARY.md`, README finding format.
2. Read every finding file under `findings/` whose id is in **A041–A060** and that exists on disk.
3. Extract: status, severity, gaps, impact claims, peer cross-links.
4. Cluster residuals into loss classes; quantify blast radius; cross-link IDs.
5. Prefer **agreement** unless peer evidence conflicts (none found — see §7).
6. Note missing scopes (A042, A043, A060) and what peers already cover by adjacency.

No production Rust edited. No git operations (COORDINATION).

### 0.2 Coverage map (A041–A060)

| ID | File present? | Peer status | Role in money synthesis |
|---|---|---|---|
| A041 | yes | defended (info) | Measured custody at controller inbound; hard rule |
| A042 | **no** | — | Pool withdraw measured-transfer — **coverage hole**; partial inference from A041/A051/A057 |
| A043 | **no** | — | Pool borrow/repay measured amounts — **coverage hole**; partial inference from A047/A054/A058 |
| A044 | yes | defended (info) | Flash loan principal+fee pullback |
| A045 | yes | defended (info) | Flash position mint / collateral Δ / refunds |
| A046 | yes | defended (info) | Multiply borrow→swap→deposit |
| A047 | yes | defended (info) | Swap-debt borrow→swap→repay |
| A048 | yes | **partial** (medium) | Swap-collateral; elevates no-`min_out` residual |
| A049 | yes | defended (info) | Repay-with-collateral net vs swap |
| A050 | yes | defended (info) | Migrate-from-blend leftover repay |
| A051 | yes | defended (info) | Liquidation Transfer seize outflow |
| A052 | yes | defended (info) | Liquidation Credit share credit |
| A053 | yes | defended (low residuals) | Protocol fee skim (bonus-only; mint vs absorb) |
| A054 | yes | defended (info) | Overpay / excess / leftover refunds |
| A055 | yes | **partial** (medium) | Lying / non-SAC token listing residual |
| A056 | yes | **partial** (medium) | Controller slippage inventory (owns F1–F7) |
| A057 | yes | defended (info) | Destination `to` hijack class closed |
| A058 | yes | defended (info) | `balance_delta_since` / `transfer_amount_measured` correctness |
| A059 | yes | defended (info) | ADR-0003 directed rounding on money paths |
| A060 | **no** | — | Cross-asset dust / bad-debt threshold — **coverage hole**; dust notes in A053/A059 only |

**Present:** 17/20 scopes. **Missing:** A042, A043, A060. Synthesis does **not** invent findings for missing scopes; adjacency notes are marked *inferred* and should be re-checked when those agents file.

---

## 1. Executive verdict

**Money-movement defenses in the controller are strong where tokens enter or leave controller custody, where strategy debt is minted, where repay/refund residue is returned, and where liquidation books Transfer vs Credit fees.**

Undefended or only-partially-defended gaps that survive the A041–A060 corpus collapse into **two primary residual classes** (plus smaller accepted/ops residuals):

| Rank | Residual class | Owning IDs | Status | Max quantified impact |
|---|---|---|---|---|
| 1 | No controller quantitative `min_out` on strategy swaps | A048, A056 (also A046/A047/A049 Gap) | partial | **Account-local** loss up to nearly the full authorized swap `amount_in` (clipped by post-gate HF ≥ 1 when debt remains). Not a pool share-mint. |
| 2 | Listed non-SAC / rebasing / balance-lying tokens | A055 (outer bound for A041/A044/A045/A051/A054/A058) | partial | **Market-local** cash↔share desync → bad debt socialized to that market’s suppliers, **≤ that market’s TVL**. Governance listing is the outer control. |
| 3 | Out-of-governance swap-aggregator owner (upgrade / ignore min-out / retain value) | A056 + threat-model; adjacent A009 | known trust root | Same economic class as (1), with stronger adversary; plus router `sweep_balance` of aggregator holdings (ops). |
| 4 | Approved Blend pool trust on migrate | A050 | accepted trust | Migrator self-harm and/or whole-tx revert; protocol books stay consistent with pool mutations. |
| 5 | Post-guard listed-token transfer hooks | A007 shared; noted by A045–A050, A054 | low residual | Reentry against unpersisted RAM mid-strategy; auth + atomicity still apply; listing trust. |
| 6 | Dust / fee-bump / Transfer index liveness | A053, A059 | accepted low | ≤1–2 asset units per liquidator leg; V* ≪ min-collateral floors. |
| 7 | Spoke-usage exit no-op if row missing | A080 (inherited by A049/A052/A053) | partial (T5) | Cap under-count → temporary over-admission; **no direct theft** (PRELIMINARY). |

No A041–A060 finding claims **critical** fund theft under SAC + honest listing assumptions. PRELIMINARY’s money-related leading residuals (A055, A048/A056) are confirmed and refined below.

---

## 2. What “undefended” means here

For this synthesis:

| Label | Meaning |
|---|---|
| **Defended** | A money invariant is enforced in controller and/or owner-gated pool code under stated assumptions; residuals are accepted policy or listing trust. |
| **Partial** | Core measurement / conservation holds, but a documented economic or trust-boundary hole remains (attacker can extract value without breaking share books). |
| **Undefended** | A money path lacks the intended control entirely. **None claimed as novel critical in this corpus.** Closest: controller never enforces quantitative min-out (by design / threat-model known gap) — classified **partial**, not “forgotten assert.” |
| **Coverage hole** | Agent file missing; cannot close the claim from this wave alone. |

---

## 3. Defended money surfaces (do not re-open without new evidence)

These surfaces are consistently judged **defended** across peers. Synthesis agrees.

### 3.1 Measured custody at the controller boundary

| Surface | IDs | Load-bearing controls |
|---|---|---|
| Supply / deposit into pool | A041, A046, A048, A050, A058 | `transfer_amount_measured` → pool action uses **received** |
| Strategy borrow into controller | A045, A046, A047, A050, A058 | `measured == amount_received` + `measured > 0` |
| Flash position collateral / forward | A045 | Baseline Δ ≥ `min_amount`; double measure on mint; measured forward |
| Strategy repay controller→pool | A047, A049, A050, A054, A058 | Measured push; burn follows receipt |
| Liquidation debt repay | A051, A052, A053 | Measured repay → floor-scale seize (INV-LIQ-03) |
| Recapitalize inbound | A054 (via A016) | Measure then `min(received, shortfall)` |
| Router output | A046–A049, A056, A058 | Discard return; `token_out` Δ only; `> 0` |
| Shared primitives | A058 | Centralized `balance_delta_since` / `transfer_amount_measured` |

**Invariant:** Inbound protocol credit equals measured receipt (INV-ACCT-03 / ADR-0013). Regression that credits requested amounts or trusts router/pool returns alone is **Critical**.

### 3.2 Flash cash pullback (pool-enforced)

| Surface | IDs | Controls |
|---|---|---|
| `flash_loan` principal + fee | A044 | Three SAC brackets; allowance-scoped `transfer_from`; fee-only cash book; controller flash guard |

Controller correctly does **not** re-measure pool SAC (trust split). Weakening Bracket B / exact pull / post-pull equality is **Critical** against INV-FLASH-01.

### 3.3 Residue / overpay never becomes free credit

| Family | IDs | Rule |
|---|---|---|
| Ordinary / liq repay overpay | A054 | `credit_cash(net)` only; `transfer_out(payer, overpayment)` |
| Recap excess | A054 | Apply ≤ shortfall; refund rest |
| Strategy repay nested overpay | A047, A049, A050, A054 | Pool → controller → Δ-only forward to caller |
| Router underspend | A046–A049, A054, A056 | `leftover = amount_in − spent` ≤ auth; not gross sweep |
| Flash listed refunds | A045, A054 | Pre-callback baseline; undeclared stranded |
| Migrate leftover borrow | A050 | Leftover **repaid into hub debt**, not paid to caller as free cash |

**Invariant:** Overpay / leftover is uncredited residue to the rightful funder (INV-ACCT-02, INV-STRAT-02). Gross controller-balance sweeps would be Critical.

### 3.4 Liquidation Transfer vs Credit booking

| Mode | IDs | Money identity |
|---|---|---|
| Transfer | A051, A053 | Burn gross; withhold fee cash; mint revenue against retained cash; pay `gross − fee` |
| Credit | A052, A053 | Move RAY shares; fee = ceil(bonus); absorb (not mint); cash/`supplied` unchanged |

Critical ADR-0019 failure mode (Credit fee via mint) is **absent**. Bonus-only fee base pinned (A053).

### 3.5 Rounding direction

| Surface | IDs | Result |
|---|---|---|
| Share mint/burn pairs, risk gates, liq under-delivery, net settle | A059 (+ A049/A051–A054) | ADR-0003 directed; no free-share / debt-erasure pairing found |

Dust residuals deferred to A053 / numeric-bounds — not open free-mint bugs.

### 3.6 Destination / refinance / netting branch safety

| Surface | IDs | Result |
|---|---|---|
| Public `borrow`/`withdraw` `to` | A057 | Stranger hijack closed; pool/controller denylist (GH-17); debt stays on `account_id` |
| Same-market vs cross-market RDWC | A049 | Predicate is `HubAssetKey`; net settle moves no cash; passthrough requires empty swap |
| Multiply / swap_debt / swap_collateral measurement chains | A046–A048 | Defended custody; see §4.1 for slippage residual only |

### 3.7 Strategy open/refinance confinement

| Path | IDs | Confinement |
|---|---|---|
| `flash_position` | A045 | INV-STRAT-04 still-open; measured collateral; zero fee justified |
| `multiply` | A046 | Fee-charging borrow; router confinement; solvency finalize |
| `swap_debt` | A047 | Borrow→swap→repay; HF gate |
| `migrate_from_blend` | A050 | Allowlisted Blend; leftover debt burn; HF gate |

---

## 4. Undefended / partial gap register (quantified)

### 4.1 G-SLIP — Controller does not enforce quantitative swap min-out

| Field | Value |
|---|---|
| **Status** | partial |
| **Severity** | medium |
| **Primary owners** | A056 (inventory + F1–F4), A048 (elevates on withdraw→swap) |
| **Agreeing peers** | A046 Gap(1), A047 Gap(1), A049 outscopes to A056, PRELIMINARY A048/A056 row |
| **Code locus** | `strategies/swap.rs::verify_router_output` — only `received > 0` |
| **What is defended** | Exact `amount_in` auth; discard router return; `RouterOverspend`; leftover refund; `NoSwapOutput`; post-strategy HF gates; honest aggregator `total_min_out` |
| **What is missing** | Controller-owned floor on measured `token_out` Δ (contrast: `flash_position` `min_amount`, A045/A056 §5) |

#### Impact quantification

| Scenario | Who loses | Upper bound | Who is safe |
|---|---|---|---|
| Compromised / malicious router returns dust `token_out`, keeps residual `token_in` | Account owner (or delegate-controlled account) | ≈ **authorized `amount_in` − dust**, subject to `require_post_pool_risk_gates` still passing | Other accounts; pool share books (measured legs stay consistent) |
| Honest router + caller embeds `min_out = 1` | Same (self-authorized) | Same class | Same |
| Dust-out on `swap_collateral` with spare HF | Often sticks | Withdrawn collateral leg notional | Protocol TVL not minted away |
| Dust-out on bare `multiply` (new debt, no spare collateral) | Usually reverts (HF) | Attempt fails atomically | — |
| Dust-out on `swap_debt` / cross-asset RDWC | Usually reverts | Attempt fails | — |
| Router `OverPull` | Blocked | 0 beyond `amount_in` | — |

**Blast-radius class:** account-local / in-flight strategy notional — threat-model “unbounded-loss” relative to **that strategy’s swapped value**, not protocol-wide insolvency.

**Cross-links:** A046, A047, A048, A049, A056; remediation backlog A110; max-loss scenarios A106; missing controller dust-vs-min test A108.

---

### 4.2 G-LIST — Non-SAC / rebasing / balance-lying listed tokens

| Field | Value |
|---|---|
| **Status** | partial |
| **Severity** | medium |
| **Primary owner** | A055 |
| **Agreeing peers** | A041, A044, A045, A051, A054, A057, A058 (all treat SAC/listing as outer bound) |
| **What is defended** | Measure at custody boundaries; equality asserts on strategy borrow; zero-share rejection; flash FOT fail-closed; outbound FOT haircuts recipient not protocol credit |
| **What is missing** | Code cannot make a token that lies about `balance` / rebases mid-tx safe; listing governance must exclude them |

#### Impact quantification

| Failure mode | Upper bound | Socialization |
|---|---|---|
| Cash book vs SAC desync on a listed market | **≤ that market’s TVL** (supplier claims vs recoverable cash) | Bad debt / shortfall socialized to **that market’s** suppliers |
| FoT on measured inbound | User under-credit; no share inflation | User / tax token |
| FoT on unmeasured pool→user payout (borrow/withdraw/Transfer seize) | Recipient short; books already debited | Recipient incentive loss; extreme lies → market desync (same TVL cap) |
| Flash on non-exact asset | Fail closed if flashloanable wrongly set | Ops: never set `is_flashloanable` (A044) |

**Blast-radius class:** market-local under listing compromise — PRELIMINARY A055 row confirmed.

**Cross-links:** A041, A044, A045, A051, A054, A055, A057, A058; threat-model Tamper.3 / non-standard tokens.

---

### 4.3 G-ROUTER-OWNER — Aggregator standalone owner (trust root amplifying G-SLIP)

| Field | Value |
|---|---|
| **Status** | known / accepted architecture residual |
| **Severity** | medium (deployment gate) |
| **Owners** | Threat-model trust roots; A056 §6.3; PRELIMINARY; adjacent A009 |
| **Money effect** | Immediate `upgrade` can drop `total_min_out`; retain strategy `token_in`; `sweep_balance` router holdings |

Controller still caps pull at `amount_in` and rejects zero out. Economic class collapses into **G-SLIP** with a stronger adversary. Not a separate measurement bug.

**Impact:** Same account-local unbounded-loss class as G-SLIP for in-flight strategies; plus aggregator-held fee/referral balances (ops asset, not lending-pool TVL).

---

### 4.4 G-BLEND — Approved Blend pool behavior

| Field | Value |
|---|---|
| **Status** | accepted trust (INV-STRAT-03) |
| **Severity** | low–medium (governance allowlist) |
| **Owner** | A050 |
| **What is defended** | Allowlist before borrow; pull caps; measured leftover **debt repay**; measured sweep deposits; HF finalize; baselines protect stuck inventory |
| **Residual** | Hostile approved pool can consume pulls / under-deliver sweeps |

#### Impact quantification

| Outcome | Bound |
|---|---|
| Cap too low / unhealthy end state | Full tx revert; Blend + hub positions restored |
| Hostile consume without matching liability | Migrator over-indebted → HF revert **or** migrator self-harm inside one tx |
| Protocol silent theft from other users | **Not found** — baselines + measured Δ |

**Blast-radius:** caller’s own migration attempt.

---

### 4.5 G-HOOK — Post-guard transfer-hook reentrancy (shared)

| Field | Value |
|---|---|
| **Status** | residual (shared with A007) |
| **Severity** | low (rises if listed token has arbitrary hooks) |
| **Noted by** | A045–A050, A054, A056, A057 |
| **Windows** | Leftover refund, deposit after router, strategy repay transfer, ordinary borrow/withdraw `transfer_out`, Transfer seize payout |

Not a missing measurement step. Money integrity still relies on listing trust + auth + Soroban atomicity. Synthesis keeps this as **amplifier of G-LIST**, not a standalone theft path.

---

### 4.6 G-DUST — Liquidation / rounding dust economics

| Field | Value |
|---|---|
| **Status** | accepted |
| **Severity** | low / info |
| **Owners** | A053 (dust fee bump), A059 (≤1–2 units/leg, ideal-trim dust) |
| **Impact** | Liquidator PnL haircut ≤ dust units; mitigated by min-collateral / bad-debt USD floors (`numeric-bounds` §6). No third-party account drain. |

A060 (cross-asset dust ↔ bad-debt threshold) is **unfiled** — do not treat G-DUST as a complete answer to A060’s intended scope.

---

### 4.7 G-USAGE — Missing spoke-usage row exit no-op (adjacent T5)

| Field | Value |
|---|---|
| **Status** | partial (spoke-usage theme) |
| **Severity** | low for money; medium for caps |
| **Inherited in** | A049, A051, A052, A053 → A080 |
| **Impact** | Spoke caps can under-count → temporary **over-admission** up to that spoke’s cap headroom; **no direct theft**; supplier risk only if over-admission later goes bad (PRELIMINARY) |

Money books (shares/cash) still move correctly on liquidation/strategy exits; this is capacity accounting, not unmeasured credit.

---

### 4.8 Accepted policy / non-theft residuals (inventory)

Not “undefended holes,” but frequently mistaken for them:

| Residual | IDs | Why not fund-theft |
|---|---|---|
| Delegate / owner `to` drains account | A057, A003 | Documented complete economic control |
| Leftover / refund to `caller` (delegate) | A046–A049, A054 | Same |
| Outbound refunds unmeasured (FOT haircut recipient) | A054, A045, A058 | Cannot inflate protocol credit |
| Pool→user payouts unmeasured | A041, A051, A057 | Intentional under SAC |
| Transfer seize needs cash; Credit escape | A051 | ADR-0019 liveness |
| Transfer vs Credit fee magnitudes differ | A052, A053 | Dual representation; one mode per call |
| `flash_loan` lacks `require_external_recipient` | A044 | Fail-closed today (no `execute_flash_loan`); footgun if surface added |
| Zero flash fee when configured 0 | A044 | Principal still pulled |
| Migrate `debt_caps` temporary full mint | A050 | Reconciled by leftover repay; sizing footgun |
| RDWC doc “assets” vs `HubAssetKey` | A049 | Code correct |
| Stranded undeclared flash / controller dust | A045, A054 | Unstealable by baseline discipline |
| Delisted hub blocks new Credit slot | A052 | Liveness, not share inflation |
| Event requested vs measured (multiply payment) | A046 | Observability only |

---

## 5. Loss-class summary (for A106)

| Class | Mechanism | Max loss unit | Contagion | Corpus IDs |
|---|---|---|---|---|
| **L1 Account strategy slippage** | Dust `token_out` under G-SLIP / G-ROUTER-OWNER | Single account’s swapped notional / excess HF | None across accounts | A048, A056 (+ A046/A047) |
| **L2 Market listing desync** | Non-SAC / lying token under G-LIST | That market’s TVL (supplier claims) | Market suppliers | A055 (+ A041/A051/A058) |
| **L3 Migrator self-harm** | Hostile approved Blend | Migrator’s pulled caps / positions in one tx | None if revert; self if settles unhealthy then somehow didn’t — HF blocks | A050 |
| **L4 Cap over-admission** | A080 usage exit no-op | Spoke cap headroom (indirect) | Future borrowers in spoke | A080 via A049/A052 |
| **L5 Dust liquidator tax** | Fee bump / floor residuals | ≤ few stroops per leg | Liquidator only | A053, A059 |
| **L6 Stranger payout hijack** | Forge `to` / Credit receiver | **0** (defended) | — | A057, A013 |
| **L7 Unbacked Credit fee mint** | Wrong absorb→mint | **0** (absent) | — | A052, A053 |
| **L8 Flash under-repay** | Skip pullback | **0** while INV-FLASH-01 holds | — | A044 |
| **L9 Overpay → free shares/cash** | Book overpay | **0** (defended) | — | A054 |
| **L10 Unmeasured custody credit** | Requested amount mint | **0** under SAC (defended) | — | A041, A058 |

**Highest actionable money residuals for remediation:** L1 then L2. L3–L5 are ops/governance/dust. L6–L10 are closed defenses that must not regress.

---

## 6. Path-by-path residual matrix

| Path | Measurement / conservation | Primary residual | Impact class |
|---|---|---|---|
| User supply | Defended (A041) | G-LIST | L2 |
| User withdraw/borrow to `to` | Unmeasured outbound (accepted) | G-LIST + G-HOOK; `to` auth defended (A057) | L2 / L6=0 |
| User repay / recap | Defended (A054) | Outbound FOT UX | — |
| `flash_loan` | Defended (A044) | Listing + optional denylist hygiene | L8=0 |
| `flash_position` | Defended (A045); **has** controller min floors | G-LIST / G-HOOK | L2 |
| `multiply` | Defended custody (A046) | **G-SLIP** | L1 |
| `swap_debt` | Defended (A047) | **G-SLIP** (often HF-blocked) | L1 |
| `swap_collateral` | Defended custody (A048) | **G-SLIP** (highest stickiness) | L1 |
| `repay_debt_with_collateral` | Defended (A049) | G-SLIP on cross-asset; G-USAGE | L1 / L4 |
| `migrate_from_blend` | Defended (A050) | **G-BLEND** | L3 |
| Liquidate Transfer | Defended (A051) | G-LIST outbound; cash liveness | L2 |
| Liquidate Credit | Defended (A052) | G-USAGE; delist liveness | L4 |
| Liq protocol fee | Defended (A053) | G-DUST | L5 |
| Rounding | Defended (A059) | G-DUST | L5 |
| Δ primitives | Defended (A058) | G-LIST if balance lies | L2 |

---

## 7. Cross-agent agreement and disagreements

### 7.1 Agreement (no disagreement file warranted)

| Topic | Consensus IDs |
|---|---|
| Measure at controller custody | A041, A045–A050, A054, A058 |
| Router return discarded; positivity-only out | A046–A049, A056 |
| Flash pullback at pool SAC | A044 (+ A007/A019) |
| Credit absorb ≠ Transfer mint | A051, A052, A053 |
| Delta-only refunds / no gross sweep | A045, A047, A050, A054, A058 |
| Destination stranger hijack closed | A057 |
| Rounding protocol-favouring | A059 |
| Listing is outer bound for lying tokens | A055 + nearly all T3 peers |
| Migrate leftover must repay debt not refund caller | A050 vs swap leftover pattern |

### 7.2 Framing difference (not evidence conflict)

| Topic | Positions | Synthesis reconciliation |
|---|---|---|
| Slippage residual severity | A046/A047: defended money-flow + known Gap(1) as residual; A048/A056: **partial / medium** primary residual | **Agree with A048/A056 for gap ranking:** custody is defended; quantitative slippage is the leading **undefended economic** gap. A046/A047 “defended” refers to share/cash conservation, not price fairness. |
| A041 “Gap” on unmeasured user payouts | A041 notes it; A051/A057 accept under SAC | **Accepted design**, not an open Critical, unless G-LIST applies |

No peer pair asserts contradictory facts about the same code path (e.g. none claim Credit mints fees while another proves absorb-only). **No `disagreements/` file created.**

---

## 8. Coverage holes (missing A042 / A043 / A060)

### 8.1 A042 — Pool withdraw measured-transfer pattern

**Not filed.** Adjacent evidence:

- Controller strategy withdraw-to-controller **does** measure controller Δ (A048, A049, A058).
- Ordinary / Transfer payouts pool→user are **intentionally unmeasured** at recipient (A041, A051, A057).
- Pool `require_reserves` / cash debit / `transfer_out` gates exist (A051).

**Inference (non-authoritative):** User withdraw money safety is “pool cash book + SAC token + auth,” not recipient Δ. A042 should confirm whether any withdraw path incorrectly credits based on requested amount without pool mutation outputs (A082 adjacency). Until filed, do **not** claim a novel withdraw measurement hole.

### 8.2 A043 — Pool borrow / repay measured amounts

**Not filed.** Adjacent evidence:

- Strategy borrow equality `measured == amount_received` (A045–A047, A050, A058).
- Repay always feeds pool with measured receipt (A047, A054, A058).
- Ordinary borrow pays `to` without recipient measure (A057/A041).

**Inference:** Borrow/repay money integrity for **accounting** is pool mutation + measured inbound repay; outbound borrow mirrors withdraw. A043 should pin pool-side amount vs cash debit identities. No contradiction in present peers.

### 8.3 A060 — Cross-asset dust / dust-threshold bad-debt interaction

**Not filed.** Adjacent evidence only covers per-leg dust fee bump and rounding (A053, A059), not cross-asset dust aggregation into bad-debt thresholds.

**Action:** Treat A060 as an open wave-3 item; A101 must not claim G-DUST closes bad-debt dust interaction.

---

## 9. Regression watchlist (defended → Critical if weakened)

Synthesized from peer “do not remove” opinions:

1. `transfer_amount_measured` / `balance_delta_since` as sole custody credit oracles (A041, A058).
2. `measured == amount_received` on `borrow_into_controller` (A045–A047, A050, A082).
3. Router return discard + `actual_spent ≤ amount_in` + leftover ≤ auth (A046–A048, A056).
4. Flash SAC brackets + exact `transfer_from` repay (A044).
5. Liquidation `scale_seizures_to_received` before seize (A051–A053).
6. Credit fee via absorb only — never `withhold_liquidation_fee` mint (A052, A053).
7. Overpay excluded from `credit_cash`; refunds Δ-only (A054).
8. Migrate leftover → `repay_debt_from_controller`, not caller cash refund (A050).
9. `require_external_recipient` on public borrow/withdraw (A057).
10. ADR-0003 mint/burn direction pairs (A059).

---

## 10. Inputs to later synthesis agents

| Agent | What to take from A101 |
|---|---|
| A105 | G-SLIP and G-LIST match threat-model known gaps; G-ROUTER-OWNER matches trust-root table |
| A106 | Use §5 L1–L5 as max-loss scenario seeds; L6–L10 as closed |
| A108 | Highest missing tests: controller dust-out vs large payload `min_out` (A056 F7/F1); A042/A043/A060 when filed |
| A109 | No disagreement file; only framing note §7.2 |
| A110 | Prioritize: (1) controller `min_out` on strategy swaps; (2) SAC-only listing runbooks / gates; (3) optional flash_loan denylist; (4) A080 usage; (5) fill A042/A043/A060 |

---

## 11. Verdict

**Status: partial** at the wave-3 synthesis layer — not because measured money movement is broken, but because the corpus’s remaining undefended economic surface is real and shared:

1. **G-SLIP / G-ROUTER-OWNER** — no controller quantitative slippage floor; loss ≤ in-flight strategy notional / excess HF (A048, A056; confirmed by A046/A047 residuals; PRELIMINARY).
2. **G-LIST** — non-SAC listing can desync a market up to that market’s TVL (A055; outer bound for A041/A044/A045/A051/A054/A058).

Everything else in A041–A059 is either **defended** under SAC + auth assumptions, an **accepted trust/policy residual** with account- or ops-local blast radius, or a **coverage hole** (A042, A043, A060) that peers do not contradict.

**No novel critical money-theft gap** beyond those documented trust-boundary classes was synthesized from the present A041–A060 findings.
)
