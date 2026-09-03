# A106 — Max-loss scenarios (single account / market / protocol)

- Agent: A106 (synthesis)
- Theme: T8
- Severity: medium (largest *live code* residuals); critical **only** under failed deployment trust roots (A009 / oracle / router ownership)
- Status: synthesis — quantifies leading residuals; does not claim novel critical code holes beyond peers
- Paths: synthesis over `synthesis/PRELIMINARY.md`; findings A080, A055, A009, A048, A056, A064, A101–A103 (A104 **absent**); threat-model Known gaps; INV-STRAT-02, INV-LIQ-04, INV-HALT-03, INV-AUTH-01/04/05
- Defense: Measured custody, flash guards, post-pool HF ≥ 1 WAD, owner=governance delay (when wired), listing + FreezePolicy, pool-truth spoke usage on healthy entry paths — see A101 §3, A102 §2, A103 §3
- Gap: This file does **not** invent new gaps; it **bounds blast radius** for residuals already ranked by PRELIMINARY and refined by A101–A103
- Impact: See §3–§6. Headline: **account-local** ≤ strategy swap notional / excess HF; **market-local** ≤ that market’s supplier TVL (listing desync or socialized bad debt); **protocol-total** only via admin/oracle/router trust-root failure — not via a missing measured-transfer assert in the present corpus
- Evidence: Peer impact columns cited inline; threat-model §§slippage / aggregator owner / XOXNO owner / Sensitive=12; A009 §10; A056 §6; A101 §5 L1–L10; A102 §4; A103 §5
- Opinion: Treat “unbounded loss” in the threat-model as **unbounded relative to the in-flight strategy notional** (and, under trust-root failure, relative to the whole book). Do not conflate account-local slippage with protocol insolvency. Highest remediation leverage remains controller `min_out` (L1) and SAC-only listing / ops gates (L2); deployment ownership and Sensitive floor dominate absolute max loss.

---

## 1. Mission and method

**Mission:** Quantify maximum loss scenarios at three blast-radius tiers — **single account**, **single market**, **protocol** — from the audit’s **leading residuals**, using filed peer findings only.

**Method:**

1. Read `shared/COORDINATION.md`, `SEED.md`, `synthesis/PRELIMINARY.md`, README finding format, `AGENT_MANIFEST.md` (A106 scope).
2. Read required peers: A080, A055, A009, A048, A056, A064, A101, A102, A103; note A104 missing.
3. Read threat-model Known gaps (slippage, aggregator/XOXNO owners, Sensitive delay) for official “unbounded” language.
4. Build a loss taxonomy (direct vs contingent vs availability), then scenario cards with explicit upper bounds and contagion.
5. Cross-check A101 §5 L-classes, A102 §4, A103 §5 so synthesis agents do not double-count incompatible mechanisms.

No production Rust edited. No git operations (COORDINATION).

### 1.1 Corpus note — A104 absent

| ID | Manifest scope | Status for A106 |
|---|---|---|
| A104 | Cache/optimization hazards (A086–A100) | **Not filed.** A094 (forgotten `put_market_index`) and PRELIMINARY’s A094 row are used as the only cache-adjacent residual with a loss sketch. Do **not** treat cache hazards as exhaustively quantified until A104 lands. |

---

## 2. Loss taxonomy

### 2.1 Blast-radius tiers

| Tier | Definition used here | Typical unit of max loss |
|---|---|---|
| **Account** | One position NFT / `account_id` (owner + delegates) | That account’s collateral, debt credit line, or in-flight strategy notional |
| **Market** | One hub liquidity pool asset (and its suppliers / borrowers) | Supplier claims vs recoverable cash for **that** asset — ≤ market TVL |
| **Protocol** | Cross-market / cross-spoke control plane (Wasm, price pointer, ownership) | Entire pool book + all account authority (NFT) + future admissions |

### 2.2 Loss kinds (do not mix)

| Kind | Meaning | Example residual |
|---|---|---|
| **D — Direct extractable** | Adversary (or compromised trust root) can move value out of the victim envelope in one successful tx | A048/A056 dust-out swap; hot-owner `upgrade` |
| **C — Contingent / socialized** | No immediate theft; loss appears only if later insolvency + socialization | A080 over-admission → later bad debt; A055 cash↔share desync |
| **A — Availability / latency** | Prevents liquidation or entry; increases time-to-recovery; may enlarge eventual C | A064 `no_seize`; A065 plant-stale; A062 fee DoS |
| **0 — Closed** | Peer corpus shows defense; max loss **0** under stated assumptions | A101 L6–L10 |

### 2.3 Notation

| Symbol | Meaning |
|---|---|
| \(N_{\mathrm{in}}\) | Authorized strategy swap `amount_in` (token units of `token_in`) |
| \(V(N)\) | Oracle/USD value of amount \(N\) at settlement prices (WAD) |
| \(E_{\mathrm{HF}}\) | Excess health: value that can be lost while keeping HF ≥ 1 WAD and LTV collateral ≥ debt (when debt remains) |
| \(\mathrm{TVL}_m\) | Market \(m\) supplier claims / recoverable cash ceiling for that asset |
| \(C_s, C_b\) | Spoke `supply_cap` / `borrow_cap` (asset units) for one `(spoke, hub_asset)` |
| \(U, P\) | Recorded spoke usage vs conceptual Σ live scaled positions (A080/A103) |
| \(D_{\mathrm{bad}}\) | Residual unpaid debt of accounts that eventually socialize on market \(m\) |

**Hard rule from peers:** Post-strategy / post-borrow gates require HF ≥ 1 when debt remains (A072, A056). Debt-free accounts skip HF — so a debt-free `swap_collateral` can lose nearly all withdrawn collateral value to a dust `token_out` without an HF clip (A048, A056 §6.2).

---

## 3. Leading residuals → scenario map

PRELIMINARY leading residuals, enriched by A101–A103:

| Residual | Primary IDs | Kind | Primary tier | Max-loss unit (summary) |
|---|---|---|---|---|
| No controller `min_out` / router trust | A048, A056, A101 G-SLIP / G-ROUTER-OWNER | D | Account | ≈ \(V(N_{\mathrm{in}})\) clipped by \(E_{\mathrm{HF}}\) if debted; ≈ full withdrawn notional if debt-free |
| Non-SAC / lying listed token | A055, A101 G-LIST | C (→ market) | Market | ≤ \(\mathrm{TVL}_m\) |
| Owner ≠ governance / Sensitive=12 | A009 | D | Protocol | Entire book + NFT authority + pointers |
| Standalone aggregator / XOXNO owners | Threat-model; A056; A065/A009 adjacency | D | Account (router) / Protocol (oracle) | Router: same as slippage class + router holdings; Oracle: protocol-wide wrong valuation |
| `apply_exit` missing-row no-op | A080, A103 | C (indirect) | Market (soft) | Over-admission ≤ cap headroom; loss only if later bad debt ≤ \(\mathrm{TVL}_m\) |
| `no_seize` ̸⇒ `frozen` | A064, A102 G-VAL-1 | A → C | Market (holders of that collateral) | Liquidation halt; eventual socialize ≤ \(D_{\mathrm{bad}}\) of stranded set |
| Uncapped mutator/keeper Vecs | A062, A015, A102 G-VAL-2 | A | — (fees) | Attacker’s own fees / budget; **not** inventory |
| Forgotten `put_market_index` | A094 (A104 absent) | D/C if shipped | Account / tx-local | Wrong HF/caps **within one tx**; footgun for new code |

---

## 4. Scenario cards (quantified)

### S1 — Strategy dust-out under compromised or self-authorized slippage (Account)

| Field | Value |
|---|---|
| **IDs** | A048, A056, A101 L1 / G-SLIP; threat-model “controller does not bound slippage” |
| **Kind** | **D** (direct to account equity) |
| **Adversary** | Malicious / upgraded swap aggregator; **or** caller who embeds `total_min_out = 1` against an honest router |
| **Mechanism** | `verify_router_output` requires only `received > 0`. Router spends up to \(N_{\mathrm{in}}\), returns dust `token_out`. Leftover unspent `token_in` refunds to `caller`; spent residual retained by router/venues. |
| **What cannot happen** | Over-pull beyond \(N_{\mathrm{in}}\) (`RouterOverspend`); zero out (`NoSwapOutput`); silent pool share mint (measured legs — A101 §3) |

#### Upper bound by path

| Path | Sticks when | Account max loss | Protocol / other accounts |
|---|---|---|---|
| `swap_collateral` | Spare HF **or** debt-free | ≈ \(V(\text{withdrawn}) - V(\text{dust out})\); debt-free ≈ full withdrawn leg | **0** contagion; books consistent |
| `multiply` (bare new debt) | Usually **reverts** (HF) | 0 (atomic fail) | 0 |
| `multiply` (large initial collateral / spare HF) | Possible | ≤ debt-leg \(N_{\mathrm{in}}\) value clipped by \(E_{\mathrm{HF}}\) | 0 |
| `swap_debt` | Usually reverts | 0 typical | 0 |
| `repay_debt_with_collateral` (cross-asset) | Often reverts | 0 typical | 0 |

**Formal account bound (debted):**

\[
L_{\mathrm{acct}} \le \min\bigl(V(N_{\mathrm{in}}) - V(\varepsilon),\; E_{\mathrm{HF}}\bigr)
\]

where \(\varepsilon\) is the dust `token_out` (at least 1 unit of the chosen asset).

**Formal account bound (debt-free `swap_collateral`):**

\[
L_{\mathrm{acct}} \le V(N_{\mathrm{withdrawn}}) - V(\varepsilon)
\]

(no HF clip — A056 §6.2, A048 Impact).

**Threat-model wording:** “unbounded-loss path for **in-flight strategies**” = unbounded in \(N_{\mathrm{in}}\) / withdrawn notional for that call, **not** unbounded mint against protocol TVL.

**Single-account concentration:** An account may hold ≤ `POSITION_LIMIT_MAX` (= 5) supply and borrow slots. Worst single-tx strategy loss is still one (or few) authorized legs’ \(N_{\mathrm{in}}\), not “all protocol markets,” unless the user repeatedly rotates many legs across txs (self-harm / delegate complete control — threat-model accepted residual).

---

### S2 — Router owner compromise amplifies S1 + sweeps router treasury (Account + ops)

| Field | Value |
|---|---|
| **IDs** | Threat-model aggregator trust root; A056 §6.3; A101 G-ROUTER-OWNER; PRELIMINARY |
| **Kind** | **D** |
| **Adversary** | Standalone swap-aggregator Ownable key |
| **Immediate powers** | `upgrade` (drop `total_min_out`), `sweep_balance`, referral reassignment, `renounce_ownership` |
| **Lending-pool bound** | Same as S1 per in-flight strategy: ≤ authorized \(N_{\mathrm{in}}\) / \(E_{\mathrm{HF}}\) per successful dust settlement |
| **Extra (not pool TVL)** | All non-reserved balances held **on the router** (fees/referrals) — ops asset, not supplier claims |

**Protocol tier?** **No** for pool share books under measured settlement. **Yes** for “every strategy user who swaps while the malicious Wasm is live” as a **sum of account-local losses** over time — still not a single share-mint event.

---

### S3 — Listed non-SAC / rebasing / balance-lying token (Market)

| Field | Value |
|---|---|
| **IDs** | A055; A101 L2 / G-LIST; PRELIMINARY A055 |
| **Kind** | **C** (market desync → supplier shortfall) |
| **Adversary** | Governance listing + hostile/nonstandard token behavior |
| **Mechanism** | Measurement and equality asserts assume SAC-like `balance` truth. Rebase / lie / hidden hooks can desync pool cash vs shares on paths that cannot be made safe in controller code alone. |

#### Upper bound

| Failure mode | Max loss | Who pays |
|---|---|---|
| Cash book vs claims desync on market \(m\) | **≤ \(\mathrm{TVL}_m\)** | Suppliers of \(m\) (socialized shortfall / bad debt class) |
| FoT on measured inbound | User under-credit; no share inflation | Caller |
| FoT on unmeasured pool→user payout | Recipient short | Recipient (extreme → same \(\mathrm{TVL}_m\) class if books already debited) |

**Contagion:** **Market-local.** Untouched markets stay bit-identical under socialization pins (threat-model / INV-LIQ-04 peer language; A014).

**Not protocol-total** unless the same non-SAC pattern is listed on every market (governance failure, not a single-tx exploit).

---

### S4 — Controller owner mis-wire or Sensitive delay unrestored (Protocol)

| Field | Value |
|---|---|
| **IDs** | A009 §10; threat-model deployment gates; PRELIMINARY A009 |
| **Kind** | **D** |
| **Adversary** | Hot EOA/multisig as controller owner; **or** correct owner with Sensitive floor = 12 ledgers (~1 min) after key compromise |

#### Scenario matrix (from A009)

| Wiring | Max loss | Reaction window |
|---|---|---|
| Owner = governance + production delays | Delayed Standard/Sensitive; canceller can drop ops; guardian tighten-only | Full policy delays |
| Owner = governance + Sensitive = 12 | Near-immediate Wasm / oracle pointer / ownership / `force_socialize` | ~1 minute — treat as **near protocol-total** for MEV/response |
| Owner = hot key | Immediate `upgrade`, `upgrade_pool`, `upgrade_position_nft`, `set_price_aggregator`, `set_swap_aggregator`, unpause, flag clear via `edit_asset_in_spoke`, `force_socialize_bad_debt` | **None** |
| Guardian only compromised | Pause + ratchet flags + empty hub/spoke | **Cannot** unpause, clear flags, upgrade, move pointers (INV-AUTH-04) — loss kind **A**, not D |
| PROPOSER only | Can schedule; cannot skip delay | Delay + canceller |

**Protocol max loss (hot owner):**

\[
L_{\mathrm{protocol}} \le \sum_m \mathrm{TVL}_m + \text{all account NFT authority} + \text{future admissions under hostile Wasm/prices}
\]

i.e. **unbounded relative to deployed value** — the only residual in this synthesis that honestly reaches “protocol-total” as a single control failure.

**Code status:** `#[only_owner]` placement **defended**; property is **deployment composition** (A009 Opinion).

---

### S5 — XOXNO oracle standalone owner / mis-pointed price aggregator (Protocol valuation)

| Field | Value |
|---|---|
| **IDs** | Threat-model XOXNO owner; A065 G-VAL-11 / Impact; A009 `set_price_aggregator`; A102 §4 G-VAL-8/11 |
| **Kind** | **D** via wrong HF/LTV/liq sizing (not a missing controller stale assert) |
| **Mechanism** | Controller consumes aggregator `prices()` / `feed.price` only. Compromised oracle Wasm or Sensitive `SetPriceAggregator` to a hostile feed makes every solvency decision lie. |

#### Upper bound

| Subcase | Bound |
|---|---|
| Hostile prices admit undercollateralized borrow / block honest liq | Can drain **borrowable liquidity** and leave bad debt up to **affected markets’ \(\mathrm{TVL}_m\)** as loans go unpaid — effectively **protocol-wide** if the feed covers all listed assets |
| Fail-closed stale/sanity (honest aggregator) | Kind **A** (mutations revert); plant-stale leg (A065/A102 G-VAL-9) blocks liq for **that account** until refresh |
| Dual-source skew inside windows | Config-bounded mis-admission (~fixture skew order; A065) — ops residual |

**Distinction from S1:** S1 loses value **inside** an account’s swap without corrupting prices. S5 corrupts the **risk meter** for everyone using those assets.

---

### S6 — Spoke usage missing-row exit → soft-cap over-admission (Market, contingent)

| Field | Value |
|---|---|
| **IDs** | A080; A103 §4.1 / §5.1; A101 L4; PRELIMINARY A080 |
| **Kind** | **C** (indirect); capacity integrity, not theft |
| **Mechanism** | `apply_exit` no-ops if usage row missing → \(U = 0\) while \(P > 0\) → new entries fill from 0 up to \(C_s\) / \(C_b\) again |

#### Upper bound

| Quantity | Bound |
|---|---|
| Immediate extractable theft | **0** |
| Extra admissions | ≤ remaining configured cap headroom evaluated from recorded \(U\) (≈ full \(C_s\) or \(C_b\) if \(U=0\)) |
| True economic exposure after fill | Prior unrecorded \(P\) **plus** new fill up to \(C\) |
| Realized supplier loss | Only if over-admitted loans later default / socialize → **≤ \(\mathrm{TVL}_m\)** for that asset (same socialization ceiling as any bad debt) |

**False-cap (over-count) dual:** Kind **A** only — entries blocked; no fund seizure (A103 §5.2).

**Not** a bypass of HF/LTV (A072 still gates risk-increasing paths).

---

### S7 — `no_seize` without freeze / without blocking supply (Market availability → contingent loss)

| Field | Value |
|---|---|
| **IDs** | A064 G1; A102 G-VAL-1; STRIDE DoS.2; ADR-0008 Option C unshipped |
| **Kind** | **A → C** |
| **Mechanism** | One `no_seize` collateral leg reverts whole liquidation. Users may still **supply** that asset → unliquidatable set grows. Hatch: owner `force_socialize_bad_debt`. |

#### Upper bound

| Phase | Bound |
|---|---|
| While flag set | **0** direct theft; liquidations that need that seize leg fail |
| Growth | Unliquidatable collateral can increase via continued supply |
| After force socialize | Supplier loss **≤ \(D_{\mathrm{bad}}\)** of stranded insolvent accounts on affected markets — **not** an infinite mint from the flag alone (A102 §4.1) |
| Compare S3 | S3 is desync from token lies; S7 is **delayed recovery** of already-economic bad debt |

**Account tier:** Individual underwater holders cannot be rescued via seize until flag clear / socialize — their shortfall socializes rather than being liquidated in market.

---

### S8 — Fee / CPU DoS via uncapped Vecs (Non-inventory)

| Field | Value |
|---|---|
| **IDs** | A062, A015; A102 G-VAL-2; PRELIMINARY |
| **Kind** | **A** |
| **Max “loss”** | Transaction fees / Soroban budget of attacker or griefed keeper caller |
| **Protocol inventory** | **0** — cannot double-apply after aggregate; cannot redirect `claim_revenue`; position cardinality still ≤ limits |

---

### S9 — Delegate / owner self-drain (Account, accepted design)

| Field | Value |
|---|---|
| **IDs** | Threat-model “delegate has complete economic control”; A057; A003; A101 §4.8 |
| **Kind** | **D** (authorized) |
| **Bound** | Entire account credit line + withdrawable collateral, subject to post-op HF when debt remains |
| **Protocol** | **0** stranger theft — INV-AUTH-02 |

Not a “gap” for remediation in the same class as S1; user-doc / UX residual.

---

### S10 — Cache footgun: forgotten `put_market_index` (Account / tx-local; A104 pending)

| Field | Value |
|---|---|
| **IDs** | A094; PRELIMINARY; A103 adjacency; **A104 unfiled** |
| **Kind** | Potential **D/C** if a new code path ships without refresh |
| **Today** | Current merges call `put_market_index` — residual is **future-leg footgun**, not a demonstrated live drain |
| **Hypothetical bound if broken** | Wrong HF/caps **within that transaction** — could under/over-admit vs live indexes; durable state still atomic with pool mutation; severity depends on the broken leg |

A106 does **not** elevate S10 to a live leading residual equal to S1–S7 until A104 synthesizes A086–A100.

---

### S11 — Closed money paths (max loss = 0 under assumptions)

From A101 §5 L6–L10 (do not re-open without new evidence):

| Class | Mechanism | Max loss |
|---|---|---|
| L6 Stranger `to` / Credit hijack | A057, A013 | **0** |
| L7 Unbacked Credit fee mint | A052, A053 | **0** (absorb-only) |
| L8 Flash under-repay | A044 | **0** while INV-FLASH-01 |
| L9 Overpay → free credit | A054 | **0** |
| L10 Unmeasured custody credit | A041, A058 | **0** under SAC |

Dust liquidator tax (A101 L5 / A053/A059): ≤ few asset units per leg — liquidator PnL only.

---

## 5. Tiered max-loss summary tables

### 5.1 Single account

| Rank | Scenario | Max loss (order) | Kind | Requires |
|---|---|---|---|---|
| 1 | S4 hot owner / hostile Wasm targeting one account | Entire account + more | D | Deploy trust fail |
| 2 | S5 hostile prices | Borrow max / liquidation mis-size vs that account | D | Oracle/aggregator trust fail |
| 3 | S1/S2 strategy dust-out | ≈ \(V(N_{\mathrm{in}})\) or withdrawn notional, HF-clipped if debted | D | Router malice **or** `min_out=1` |
| 4 | S9 delegate drain | Full economic control of account | D | Authorized delegate |
| 5 | S7 unliquidatable | Interest growth until socialize; no immediate seize | A→C | `no_seize` set |
| — | S11 closed paths | 0 | 0 | — |

**Practical “code residual” max for a single healthy user under correct deploy:** **S1** — loss of nearly a full collateral rotation (debt-free) or excess HF on a levered account.

### 5.2 Single market

| Rank | Scenario | Max loss (order) | Kind | Requires |
|---|---|---|---|---|
| 1 | S4/S5 protocol control over that market’s params/prices/Wasm | ≤ \(\mathrm{TVL}_m\) (and freeze/drain dynamics) | D | Trust-root fail |
| 2 | S3 non-SAC listing desync | ≤ \(\mathrm{TVL}_m\) | C | Listing governance |
| 3 | S6 over-admission then defaults | ≤ \(\mathrm{TVL}_m\) (indirect) | C | Usage under-count + later insolvency |
| 4 | S7 stranded liq → force socialize | ≤ \(D_{\mathrm{bad}}\) of affected accounts | A→C | Mis-set `no_seize` |
| 5 | S8 Vec DoS | 0 inventory | A | — |

**Practical “code residual” max for one market under correct deploy + SAC listing:** contingent only (S6/S7), not a silent mint. **Listing non-SAC (S3)** is the largest **market** residual that does not require stealing the admin key.

### 5.3 Protocol

| Rank | Scenario | Max loss (order) | Kind | Requires |
|---|---|---|---|---|
| 1 | S4 owner ≠ timelock (or Sensitive≈0 after compromise) | **All deployed value + authority** | D | Config / delay gate |
| 2 | S5 oracle/price pointer compromise | All markets priced by that feed | D | Trust root |
| 3 | Sum of S1 over all strategy users while malicious router live | Σ account losses | D | Aggregator owner |
| 4 | S3 on every listed market | Σ \(\mathrm{TVL}_m\) | C | Catastrophic listing policy |
| — | S1 alone (honest listing, correct owner) | **Not** protocol-total | D account-local | — |
| — | S6/S7/S8 alone | Not protocol-total | C/A | — |

**Verdict line:** Under the **intended deployment** (owner=governance, production delays, SAC-only listings, intended router/oracle owners), **no leading residual in A080/A055/A048/A056/A064 yields protocol-total loss by itself.** Protocol-total requires **S4 or S5** (or their Sensitive-floor near-equivalent).

---

## 6. Interaction / stacking (do not understate)

| Stack | Effect on bound |
|---|---|
| S2 + S1 | Same account bound per tx; **frequency** of loss rises (every strategy swap) |
| S5 + S1 | Hostile prices can **inflate \(E_{\mathrm{HF}}\)**, raising the HF clip on dust-out — account loss can approach full \(V(N_{\mathrm{in}})\) more often |
| S7 + S5 | Unliquidatable **and** wrong prices — recovery DoS compounds; socialized \(D_{\mathrm{bad}}\) can grow with interest |
| S6 + organic demand | Soft cap fails → utilization/concentration risk; still needs default for supplier haircut |
| S4 after any of the above | Dominates — hostile Wasm can ignore prior defenses |
| S3 + measured paths | Measurement cannot save a token that lies about `balance` mid-tx (A055) |

**Non-stack:** S8 does not increase inventory loss of S1–S7.

---

## 7. Alignment with peer syntheses

| Source | What A106 takes as authoritative |
|---|---|
| PRELIMINARY | Leading residual set; “unbounded” slippage = strategy notional; A080 no direct theft |
| A101 §5 | L1–L5 open loss classes; L6–L10 closed |
| A102 §4 | A064 = availability / delayed socialize; A062 = fee DoS; oracle compromise = trust root |
| A103 §5 | A080 ≤ cap headroom; contingent ≤ market TVL |
| A009 §10 | Protocol-total only on owner mis-wire / near-zero Sensitive |
| A048 / A056 | Dust-out stickiness ranking by strategy path |
| A055 | Market TVL ceiling for listing desync |
| A064 | No silent share mint from `no_seize` |
| A104 | **Missing** — cache portfolio incomplete |

**No disagreement file:** Peers agree on bounds; framing differences (A046 “defended money-flow” vs A048 “partial slippage”) already reconciled in A101 §7.2 — A106 follows A101/A048/A056 for economic loss ranking.

---

## 8. Inputs for A108 / A110

| Priority | Action | Shrinks scenario |
|---|---|---|
| P0 | Restore Sensitive floor + verify owner=governance + router/oracle owners | S4, S5 deploy face |
| P0 | Controller-enforced `min_out` on strategy swaps (mirror `flash_position`) | S1, S2 lending-pool face |
| P1 | SAC-only listing runbooks / gates; never `is_flashloanable` on non-exact assets | S3 |
| P1 | ADR-0008 Option C (`no_seize ⇒ frozen`) | S7 growth |
| P2 | Spoke usage ↔ Σ positions invariant / reconcile admin | S6 |
| P2 | Cap keeper/mutator Vecs | S8 |
| P3 | File A104; checklist `put_market_index` on new merges | S10 |
| P3 | Tests: adversarial router pays `1` vs large payload min (A056/A108) | S1 evidence |

---

## 9. Verdict

**Max loss under correct deployment and SAC listing**

| Tier | Leading live residual | Quantified ceiling |
|---|---|---|
| Account | S1 strategy slippage (A048/A056) | ≈ swapped / withdrawn notional, HF-clipped if debt remains; debt-free collateral swap ≈ full leg |
| Market | S3 listing desync (A055) if non-SAC listed; else contingent S6/S7 | ≤ \(\mathrm{TVL}_m\) or ≤ \(D_{\mathrm{bad}}\) |
| Protocol | **None from controller money-path residuals alone** | Protocol-total needs S4/S5 trust-root failure |

**Max loss if deployment gates fail**

| Failure | Ceiling |
|---|---|
| Controller owner hot / Sensitive unrestored (A009) | **Protocol-total** |
| Price aggregator / XOXNO owner hostile (threat-model, A065) | **Protocol-wide valuation** → market TVLs as bad debt materializes |
| Swap aggregator owner hostile (threat-model, A056) | **Σ account strategy notionals** + router treasury; not a share mint |

**Closed at 0 (corpus):** stranger payout hijack, Credit fee mint, flash under-repay, overpay free credit, unmeasured custody credit under SAC (A101 L6–L10).

A106 therefore ranks remediation by **absolute ceiling first** (deploy/oracle/router ownership and Sensitive floor), then **highest probability live code residual** (controller `min_out`), then **market listing and liquidation liveness** (SAC policy, `no_seize` coupling, usage reconcile).
)
