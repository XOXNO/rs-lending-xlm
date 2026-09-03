# A060 — Cross-asset dust / dust-threshold bad-debt interaction

- Agent: A060
- Theme: T3 / T8 (money movement + undefended-gap scan)
- Severity: low (design / ops residuals); core gates **defended**
- Status: defended (aggregate value-based dust gate + pro-rata seize + debt-dust escalation + force-socialize hatch); partial (straddle band, floor↔threshold desync, sub-unit leg residue, missing multi-asset end-to-end pin)
- Paths:
  - `contracts/controller/src/constants.rs:3` (`BAD_DEBT_USD_THRESHOLD` = compile-time alias of `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`)
  - `common/src/constants/shared.rs:35–42` (`DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD = 5 * WAD`, `POSITION_LIMIT_MAX = 5`)
  - `contracts/controller/src/positions/liquidation/curve.rs:23–28` (`is_socializable_bad_debt`); `:117–149` (`estimate_liquidation_amount` residual-debt promotion)
  - `contracts/controller/src/positions/liquidation/mod.rs:218–275` (`BadDebtGate::{DustCapped, InsolventOnly}`, `process_clean_bad_debt`, `process_force_socialize_bad_debt`)
  - `contracts/controller/src/positions/liquidation/apply.rs:323–340` (`check_bad_debt_after_liquidation`)
  - `contracts/controller/src/positions/liquidation/bad_debt.rs` (`execute_bad_debt_cleanup` — seizes **every** remaining supply + debt hub)
  - `contracts/controller/src/positions/liquidation/math.rs:167–218` (`normalize_repayment_plan` / `FullCloseRequired`); `:249–367` (`calculate_seized_collateral` pro-rata + sub-unit drop + dust fee bump)
  - `contracts/controller/src/positions/liquidation/plan.rs` (plan builds from **aggregate** risk totals; seize over **all** supply hubs)
  - `contracts/controller/src/risk/totals.rs:169–229` (half-up `total_collateral` for dust/share; floor for LTV/weighted; ceil debt)
  - `contracts/controller/src/risk/validation.rs` (min-borrow floor on LTV collateral — paired economically, not wired to dust constant)
  - `contracts/controller/src/storage/protocol.rs` (live `MinBorrowCollateralUsd`)
- Defense: Dust eligibility is **account-level USD WAD**, not per-asset wei counts. Permissionless cleanup requires insolvency **and** `total_collateral ≤ BAD_DEBT_USD_THRESHOLD`. Liquidation sizing promotes leftover **debt** `< threshold` to a full-close *ideal*. Seizure is pro-rata across every collateral hub (Aave-style dust-collateral griefing closed). Post-liq auto-clean uses the same dust predicate. Owner `force_socialize_bad_debt` covers the intentional straddle. Cross-market socialization only touches markets the account still holds (INV-IDX-03).
- Gap: (1) **Straddle band** — insolvent with `collateral > $5` blocks `clean_bad_debt` until further liquidations or owner force (Certora V-7 / runbook; intentional). (2) **`BAD_DEBT_USD_THRESHOLD` vs live floor desync** when governance raises `MinBorrowCollateralUsd` (threat-model; A067). (3) **Operator asymmetry** — debt promotion uses `< threshold`; collateral gate uses `≤ threshold`. (4) **Sub-unit seize legs dropped** while still counting in half-up `total_collateral` — multi-asset residue can tip a post-liq book into the straddle or leave economically idle stubs until cleanup. (5) Liquidators may repay **below** ideal in the safe region and leave residual debt `≥ $5` with residual collateral `> $5` (straddle by underpayment). (6) No dedicated harness proving multi-asset dust aggregation cannot *open* the gate while solvent (A108 names `test_cross_asset_dust_does_not_open_bad_debt_gate`; existing `dust_threshold_and_decimal_floor.rs` is single-collateral price-walk). (7) Listing residual: expensive low-decimal collateral can make floor-sized seizures seize nothing (`numeric-bounds` §6.4).
- Impact: No path found where cross-asset dust **silently socializes solvent books**, **mints free shares**, or **griefs cleanup with 1 wei of a second asset** (value gate requires real USD above $5). Residual blast radius is **liveness / delayed socialization**: straddled bad debt grows with interest until a liquidator finishes the book or owner force-socializes; supplier loss then ≤ residual unpaid debt on **touched** markets (INV-LIQ-04 / INV-IDX-03). Floor desync widens the permissionless-blind band. Plant-stale dust collateral legs (A065) can brick `liquidate` / `clean_bad_debt` until the feed recovers — availability, not mis-socialization.
- Evidence: INV-LIQ-04, INV-IDX-03, INV-ACCT-05; formulas.md Liquidation / Bad debt; numeric-bounds.md §6; threat-model “dust gate and configured floor can drift”; ADR-0008 / ADR-0012; STRIDE DoS.9 / I6; Certora `bad_debt_socialization_threshold_boundary`, `bad_debt_straddle_*`, `estimate_leaves_no_sub_threshold_dust`, `clean_bad_debt_zeros_positions`; unit `liquidation_curve.rs` (threshold pin, residual promotion exclusive at exact $5, overshoot bound), `liquidation_math.rs` (sub-unit drop, floor profitability, dust fee bump), `liquidation_zero_threshold.rs` (auto-promote residual to cleanup); harness `bad_debt_index.rs` (force above dust; cross-market bit-identical), `dust_threshold_and_decimal_floor.rs` (oracle price-walk never opens gate before liquidatable); peers A014, A027, A051–A053, A059, A065, A067, A101 §4.6/§8.3, A102 G-VAL-13, A108.
- Opinion: Cross-asset dust ↔ bad-debt is a **designed dual-threshold system**, not an accidental hole. The important property vs Aave ToB-AAVE-1 / Blackthorn L-3 holds: the gate is **value-based and aggregate**, and seizure is **pro-rata**, so multi-asset books cannot cheaply brick socialization or cherry-pick collateral. Treat the straddle + floor desync + sub-unit residue as **accepted ops/design residuals** with a documented owner hatch — not as undefended theft. Do **not** fold this into A053/A059’s per-leg fee/rounding G-DUST (A101 §8.3 correctly left A060 open).

---

## 1. Scope and method

**Mission:** Audit how **cross-asset / multi-hub dust** interacts with liquidation sizing and bad-debt cleanup — specifically whether multi-asset residue can (a) open socialization incorrectly, (b) block cleanup permanently without an escape hatch, (c) leave economically unliquidatable stubs that grow protocol loss, or (d) recreate Aave-style dust-collateral griefing.

**In scope:**

1. `BAD_DEBT_USD_THRESHOLD` definition and relationship to the min-borrow floor.
2. `is_socializable_bad_debt` and both `BadDebtGate` arms.
3. Residual-**debt** promotion inside `estimate_liquidation_amount`.
4. Post-liquidation `check_bad_debt_after_liquidation` vs standalone `clean_bad_debt`.
5. Pro-rata multi-collateral seizure, sub-unit leg drops, dust fee bump.
6. Risk-total rounding that feeds the dust test (`total_collateral` half-up vs gated floors).
7. Cross-market socialization scope when multiple debt/collateral hubs remain.

**Out of primary claim (cross-linked):** authority split depth (A014), cleanup storage body (A027), Transfer/Credit custody (A051/A052), protocol fee product (A053), directed rounding inventory (A059), plant-stale oracle DoS (A065), floor gate call graph (A067).

**Method:**

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format; confirmed `A060-*.md` absent.
2. Traced every production consumer of `BAD_DEBT_USD_THRESHOLD` / `is_socializable_bad_debt`.
3. Contrasted debt-dust escalation vs collateral-dust socialization (different sides of the book).
4. Walked multi-asset seizure arithmetic for dropped legs and residual valuation.
5. Cross-checked formulas.md, numeric-bounds §6, threat-model, force-socialize runbook, Certora boundary/liquidation rules, unit + harness pins, peers A014/A027/A053/A059/A067/A101.

No production Rust edited. No git operations.

---

## 2. Verdict

**Defended** at the load-bearing security properties:

| Property | Result |
|---|---|
| Dust gate is aggregate USD, not per-asset count | **Holds** |
| 1 wei of a second collateral cannot block cleanup | **Holds** (must post > $5 real value) |
| Solvent accounts cannot be socialized via dust gate | **Holds** (`debt > collateral` required) |
| Multi-asset cleanup seizes every remaining hub atomically | **Holds** |
| Untouched markets stay bit-identical under socialization | **Holds** (INV-IDX-03 pins) |
| Straddle above $5 has an owner escape hatch | **Holds** (`force_socialize`) |
| Liquidation ideal cannot *plan* leftover debt in `(0, $5)` | **Holds** (`remaining == 0 \|\| remaining ≥ $5`) |

**Partial / accepted residuals:** straddle liveness, compile-time threshold vs live floor, exclusive vs inclusive boundary asymmetry, sub-unit seize drops leaving valued residue, liquidator underpayment into the straddle, listing unit-value profitability, missing multi-asset harness named by A108.

No novel **critical** money-theft or silent wrong-socialization bug was found in this scope.

---

## 3. Dual thresholds (do not conflate)

Two related but **different** dollar floors share the same default constant `$5 WAD`:

| Mechanism | Metric | Operator | Effect |
|---|---|---|---|
| Min-borrow collateral floor | `ltv_collateral` (floored gate value × effective LTV) | `< floor` reverts on risk-increasing / strategy finalize | Prevents originating / leaving undersized **healthy** books |
| Bad-debt dust gate | `total_collateral` (half-up portfolio value) | `≤ BAD_DEBT_USD_THRESHOLD` **and** `total_debt > total_collateral` | Admits permissionless socialization |
| Residual-debt promotion | leftover `total_debt − ideal` | `> 0 && < BAD_DEBT_USD_THRESHOLD` | Raises **ideal repay cap** to full debt close |

```23:28:contracts/controller/src/positions/liquidation/curve.rs
/// Returns whether an account's residual position is eligible for dust-threshold bad-debt
/// socialization: debt exceeds collateral and collateral is at or below
/// `BAD_DEBT_USD_THRESHOLD`.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}
```

```144:148:contracts/controller/src/positions/liquidation/curve.rs
    let remaining_debt = snap.total_debt.checked_sub(env, ideal);
    if remaining_debt > Wad::ZERO && remaining_debt < Wad::from(BAD_DEBT_USD_THRESHOLD) {
        return (snap.total_debt, bonus);
    }
```

**Cross-asset implication:** both the dust gate and the promotion operate on **sums over all hubs**, not on any single asset’s raw balance. Adding a second hub only matters through its USD contribution to those sums.

**Boundary asymmetry (pinned):** at exactly `$5` leftover debt, promotion does **not** fire (`residual_debt_promotion_is_exclusive_at_exactly_the_dust_threshold`). At exactly `$5` leftover collateral, socialization **does** fire (`<=`). Certora `estimate_leaves_no_sub_threshold_dust` allows `remaining == 0 || remaining >= THRESHOLD`, which is true under both `<` and `<=` for the debt side — the exclusive edge is a unit/harness pin, not a Certora discriminator.

---

## 4. Call graph: where dust meets liquidation and cleanup

### 4.1 Permissionless cleanup

```
clean_bad_debt(caller, id)
  → require_auth + require_not_flash_loaning
  → clean_bad_debt_standalone
  → socialize_bad_debt(..., BadDebtGate::DustCapped)
       assert !borrow_positions.empty
       totals = calculate_account_risk_totals(...)
       assert is_socializable_bad_debt(debt, coll)
       → execute_bad_debt_cleanup  // all hubs
```

Not pause-gated (INV-HALT-01 exit surface). Caller auth is presence-only (A002/A014).

### 4.2 Owner force path

```
force_socialize_bad_debt(id)  // #[only_owner]
  → BadDebtGate::InsolventOnly  // debt > coll, no dust cap
  → execute_bad_debt_cleanup
```

Runbook: `docs/reference/runbooks/force-socialize-bad-debt.md`. Explicitly warns **not** to raise the dust threshold to paper over straddles.

### 4.3 In-call post-liquidation auto-clean

```
process_liquidation
  → plan / repay / seize / event / finalize (victim [+ credit receiver])
  → check_bad_debt_after_liquidation(post_totals)
       if no debt left → cleanup_account_if_empty
       else if is_socializable_bad_debt → execute_bad_debt_cleanup
       else → leave residual book (may be healthy, unhealthy, or straddled insolvent)
```

`post_totals` are computed from the **in-memory** post-seize account **before** finalize, then reused — consistent with the positions that cleanup would seize (A027 notes finalize-then-delete rent write).

### 4.4 Liquidation sizing path (debt dust)

```
build_liquidation_plan
  → aggregate totals (all supply + debt hubs)
  → estimate_liquidation_amount  // may promote ideal to full debt
  → normalize_repayment_plan     // min(offered, ideal); FullCloseRequired in toxic band
  → calculate_seized_collateral  // pro-rata over ALL supply hubs
```

Promotion raises the **cap**, not a mandatory repay. In the safe region a liquidator may still underpay and leave residual debt `≥ $5`.

---

## 5. Cross-asset defenses (vs classical griefing)

### 5.1 Value gate, not collateral-count gate

Certora `bad_debt_straddle_blocks_dust_gate` documents the Aave comparison (V-7): topping `total_collateral` to `BAD_DEBT_USD_THRESHOLD + 1` blocks permissionless cleanup while staying insolvent — but that requires **real USD value**, not `activeCollateralCount++` via 1 wei of a second asset.

**Attack that fails:** supply 1 stroop of a second listed asset worth ≪ $5 while insolvent with dust collateral → aggregate collateral still ≤ $5 → `clean_bad_debt` still admits (subject to pricing).

**Attack that works only with capital:** post > $5 of real collateral while remaining insolvent → straddle → needs further liquidation or `force_socialize`. Capital at risk is the posted collateral (which liquidators can still seize while HF < 1).

### 5.2 Pro-rata seizure over the whole collateral set

`calculate_seized_collateral` allocates `repay_usd * (1 + bonus)` across **every** supply position by half-up USD share of `total_collateral`. Liquidators choose **which debts** to repay; they do **not** choose which collateral to take.

Effects relevant to dust:

- Cannot cherry-pick a single high-bonus leg and leave a second “grief dust” collateral untouched by design of the split (ADR-0008 notes the same pro-rata property for pause policy).
- `L_round` (floor + dust fee bump) scales with leg count `N ≤ POSITION_LIMIT_MAX = 5` (`numeric-bounds` §6) — a cost of the anti-griefing design.
- A leg whose seizure floors below one asset unit is **dropped** from the seize vector (`capped_amount <= 0 => continue`) while siblings still seize (`a_sub_unit_leg_is_dropped_while_its_siblings_are_still_seized`).

### 5.3 Cleanup is total across remaining hubs

`execute_bad_debt_cleanup` iterates **all** remaining supply then **all** remaining debt positions into one `pool_seize_positions` batch, then deletes the account. Multi-asset residue cannot partially socialize one market and leave another debt leg on a live account (INV-LIQ-04). Cross-market isolation: markets the account never held stay bit-identical (`test_socialization_leaves_an_untouched_market_bit_identical` / force twin).

---

## 6. Rounding that feeds the dust test

| Quantity | Rounding | Used for |
|---|---|---|
| `total_collateral` | half-up `position_value` | Dust gate, seizure share, bonus weighting |
| `ltv_collateral` / `weighted_collateral` | floor `position_value_floor` + floor BPS | Min-borrow floor, HF, LTV gates |
| `total_debt` | ceil `position_value_ceil` | Insolvency (`debt > coll`), HF denominator, dust gate |

**Admission bias for socialization:**

- Ceil debt makes `debt > coll` **easier** (protocol-eager to treat as insolvent).
- Half-up collateral makes `coll ≤ $5` **slightly harder** than a floored reading (delays cleanup at the margin).

This matches formulas.md: collateral total for the dust test is explicitly half-up because it is a portfolio/share metric, not a solvency bound. Paired with ceil debt, the insolvency conjunct stays borrower-conservative.

**Cross-asset dust accumulation:** each hub’s half-up contribution adds independently. Many sub-unit positions can sum to a collateral total that sits just above `$5` while each individual seize leg would drop — classic path into the **straddle** after a large repayment that cleared the economically seizable mass.

---

## 7. Scenario analysis (cross-asset)

### S1 — Multi-asset insolvent dust → permissionless clean / auto-clean

**Setup:** Account holds collaterals \(C_1..C_n\) and debts \(D_1..D_m\) with \(\sum V(C) ≤ \$5\) and \(\sum V(D) > \sum V(C)\).

**Result:** `is_socializable_bad_debt` true. `clean_bad_debt` or post-liq auto-clean seizes every hub, writes down each debt market’s supply index, burns NFT. **Defended.**

Pinned: unit gate tests; `policy_zero_threshold_account_is_still_socializable_as_bad_debt`; harness keeper clean + index drop.

### S2 — Post-liq drained collateral, leftover multi-debt

**Setup:** Liquidation seizes all collateral (or leaves ≤ $5). Debt remains across one or more hubs.

**Result:** Auto-clean socializes remaining debt hubs. Zero-threshold fixture `policy_zero_threshold_liquidation_promotes_its_own_residual_to_bad_debt` pins the single-call path. **Defended.**

### S3 — Straddle: insolvent, collateral just above $5 (multi-asset friendly)

**Setup:** \(\sum V(C) = \$5 + \varepsilon\), \(\sum V(D) > \sum V(C)\). Easy to reach with two assets each ~$2.50+ after a partial liquidation, or by underpaying ideal.

**Result:** Dust gate **closed**; force gate **open**. Account remains liquidatable while HF < 1. Interest accrues; supplier loss deferred until further liq or owner force. **Accepted residual** (Certora V-7; runbook).

**Not griefable with free wei:** \(\varepsilon\) must be real oracle USD.

### S4 — Debt-dust promotion vs liquidator underpayment

**Setup:** Ideal partial would leave leftover debt in `(0, $5)` → promotion sets ideal = full debt. Liquidator offers less than ideal in the **safe** HF-preserving band.

**Result:** Plan accepts `min(offered, ideal)`. Residual debt can remain `≥ $5`. If leftover collateral also `> $5` and still insolvent → **S3 straddle**. If collateral `≤ $5` → auto-clean. Promotion overshoot on the borrower (extra collateral seized when they *do* take the full ideal) is bounded by `remaining * (1+bonus)` with `remaining < $5` (`residual_debt_promotion_overshoot_is_bounded_by_the_threshold`).

**Toxic band:** `FullCloseRequired` when HF-preserving bonus cap sits in `[0, base)` and payment is short of ideal — forces full close rather than HF-reducing partials.

### S5 — Sub-unit legs dropped, siblings seized

**Setup:** Three collateral hubs; one dust-sized. Repayment large enough to seize the book.

**Result:** Dust leg omitted from `SeizeEntry` vec; siblings seize. Dust position **remains** on the account and still contributes half-up USD to post-totals. If that residue alone is ≤ $5 under leftover debt → auto-clean removes it. If combined residue > $5 and insolvent → S3. **Partial residue / liveness**, not theft.

### S6 — Cross-asset plant-stale dust collateral (A065 adjacency)

**Setup:** Debted account supplies 1 unit of an asset whose feed later goes stale. `supply` skipped pricing; later `liquidate` / `clean_bad_debt` must price **all** legs.

**Result:** Fail-closed revert until refresh. Blocks both profit liquidation and dust cleanup. Owner force also needs prices. **Availability residual** (prefer stuck over wrong — ADR-0005). Not an A060 logic bug; amplifies multi-asset dust surface.

### S7 — Floor / threshold desync after governance raises min-borrow

**Setup:** Live `MinBorrowCollateralUsd` raised above `$5`; `BAD_DEBT_USD_THRESHOLD` stays compile-time `$5`.

**Result:** Band of positions that the economic floor considers “should not exist” but permissionless cleanup still only admits at ≤ $5 collateral. Threat-model known gap; A067/A102 G-VAL-13. Escape: force-socialize or further liquidation. **Ops residual.**

### S8 — Expensive low-decimal listing (numeric-bounds §6.4)

**Setup:** Governance lists 3-decimal collateral at very high unit value. Floor-sized repayment’s seizure floors to 0 units → empty seize vector; debt still burns.

**Result:** Liquidator may repay and receive nothing (self-inflicted / listing hazard). Residual collateral accounting depends on whether positions still exist. Today’s listed set clears the profitability margin (≥33×). **Listing residual**, not live-config theft.

### S9 — Attempt to open dust gate while solvent via multi-asset dust

**Setup:** Many tiny collateral hubs; debt ≤ aggregate collateral (or HF ≥ 1 with debt ≤ weighted path).

**Result:** Gate requires `total_debt > total_collateral`. Harness `clean_bad_debt_gate_never_opens_before_liquidation_does` walks price on a two-market book and asserts the dust conjunct never opens while not liquidatable. Certora boundary rule asserts the predicate. **Defended.** (Dedicated “dust spam cannot open gate” multi-supply pin still missing per A108 — evidence density, not a counterexample.)

---

## 8. Interaction with per-leg G-DUST (A053 / A059)

A101 §4.6 / §8.3 correctly separates:

| Class | Owners | What it covers |
|---|---|---|
| G-DUST (accepted low) | A053, A059 | Dust **fee bump**, ≤1–2 unit/leg liquidator haircut, ideal-trim refund dust |
| A060 (this file) | A060 | Aggregate **USD threshold** ↔ liquidation close / `clean_bad_debt` / multi-asset residue |

The fee bump makes small multi-leg seizures slightly worse for liquidators (`L_round` scales with `N`) and is why the **same** `$5` floor is used as the profitability comparator (`numeric-bounds` §6.2–§6.3). That pairing is intentional: promotion and socialization keep liquidators from being asked to clear sub-floor stubs at a loss, and give a permissionless path when stubs are dust-insolvent.

**Do not** treat A053’s bump as closing A060’s threshold interaction.

---

## 9. Evidence matrix

| Claim | Evidence |
|---|---|
| Threshold = $5 WAD | `bad_debt_threshold_is_five_usd_wad`; equals `DEFAULT_MIN_BORROW…` |
| Predicate insolvency ∧ coll ≤ $5 | unit + Certora `bad_debt_socialization_threshold_boundary` |
| Straddle blocks dust, force admits | Certora `bad_debt_straddle_*`; harness `test_force_socialize_bad_debt_above_dust_threshold` |
| Ideal leaves no sub-$5 debt dust | Certora `estimate_leaves_no_sub_threshold_dust`; unit escalate / leave-above / exclusive-at-exact |
| Promotion overshoot bounded by threshold | `residual_debt_promotion_overshoot_is_bounded_by_the_threshold` |
| Sub-unit multi-leg drop | `a_sub_unit_leg_is_dropped_while_its_siblings_are_still_seized` |
| Post-liq auto-socialize dust residual | `policy_zero_threshold_liquidation_promotes_its_own_residual_to_bad_debt` |
| Price walk never opens gate before liquidatable | harness `clean_bad_debt_gate_never_opens_before_liquidation_does` |
| Cross-market isolation on cleanup | harness socialization bit-identical twins |
| Cleanup zeros all positions | Certora `clean_bad_debt_zeros_positions` |
| Floor clears listed-set unprofitability incl. N=5 | `the_min_borrow_collateral_floor_clears_…` |
| Floor↔threshold desync known | threat-model; A067; A102 G-VAL-13; A105 K7 |

---

## 10. Gaps and non-gaps

### 10.1 Not gaps (closed in this audit)

1. Aave-style multi-collateral **count** griefing of socialization.
2. Silent socialization of solvent accounts via dust math.
3. Partial multi-asset socialization leaving orphan debt on a live account.
4. Cross-market index contagion from cleanup.
5. Missing owner hatch for straddles.

### 10.2 Residuals (ranked)

| ID | Residual | Severity | Status | Remediation lean |
|---|---|---|---|---|
| R1 | Straddle band above $5 | low (liveness) | accepted design | Keep force-socialize runbook; do not raise dust threshold to paper over |
| R2 | Compile-time `BAD_DEBT_USD_THRESHOLD` vs live min-borrow floor | low (ops) | partial | When raising floor, realign constant / cleanup band (A067/A110) |
| R3 | Debt promotion `<` vs coll gate `≤` | info | pinned | Document only; changing either operator needs boundary re-proof |
| R4 | Sub-unit seize drop leaving valued multi-asset residue | low | accepted | Auto-clean if ≤ $5; else S3; optional future: fold dust legs into cleanup heuristic |
| R5 | Safe-region underpayment → straddle | low | accepted | Liquidator incentive + force hatch; FullCloseRequired already covers toxic band |
| R6 | Missing multi-asset “dust does not open gate” harness | info | evidence hole | A108 catalog item |
| R7 | Expensive low-decimal listing unit value | low (listing) | accepted | Preflight listing checklist (`numeric-bounds` §6.4) |
| R8 | Plant-stale dust leg bricks cleanup | low–medium avail. | A065 | Ops: avoid dust supply of fragile feeds into debted accounts |

### 10.3 What would make this Critical

- Dust gate admitting `debt ≤ collateral`, or
- Cleanup seizing only a subset of remaining hubs, or
- Permissionless path socializing straddles **without** insolvency, or
- Wei-count second collateral blocking cleanup.

None observed.

---

## 11. Cross-links for synthesis agents

| Agent | Take from A060 |
|---|---|
| A101 | Fill §8.3 coverage hole: status **defended** + residuals R1–R8; G-DUST does **not** subsume A060 |
| A102 | R2 aligns G-VAL-13; R8 aligns G-VAL-9 plant-stale |
| A105 | Confirms threat-model K7 (floor drift) and value-based anti-griefing vs Aave narrative |
| A106 | Straddle / delayed socialize → contingent market loss ≤ \(D_{\mathrm{bad}}\) on touched markets (same class as S7 force path), not account theft |
| A108 | Add `test_cross_asset_dust_does_not_open_bad_debt_gate`; optional multi-asset post-liq residue → auto-clean vs straddle twin |
| A109 | No disagreement with A014/A027/A053/A059/A067 — complementary scopes |
| A110 | Ops: realign threshold when raising floor; keep force-socialize; listing unit-value check |

---

## 12. Opinion

The protocol’s answer to cross-asset dust is coherent: **aggregate USD thresholds**, **pro-rata seizure**, **debt-dust close promotion**, **collateral-dust socialization**, and an **owner force hatch** for the intentional straddle. That combination closes the historically expensive “dust collateral griefing” class without inventing a per-asset dust floor that would brick legitimate multi-asset books.

The remaining risk is not that dust “breaks” bad debt — it is that **insolvent value above $5** (or a desynced floor band, or an unpriceable dust leg) must wait for liquidators or governance. That is the same operational surface the force-socialize runbook already owns. Remediations should stay in ops/config/tests, not in weakening `is_socializable_bad_debt` or removing the dust cap.
