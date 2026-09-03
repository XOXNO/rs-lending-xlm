# A081 — Supply vs borrow index selection for scaled spoke caps

- Agent: A081
- Theme: T5
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/spoke_usage.rs:14-59,100-160` (`UsageSide::{cap,index,cap_error}`, `apply_entry`, `enforce_spoke_cap`)
  - `contracts/controller/src/context/spoke.rs:103-124` (`Cache::apply_spoke_entry` — sole production selector)
  - `contracts/controller/src/positions/mod.rs:112-188` (`apply_leg_usage`, `merge_debt_leg`)
  - `contracts/controller/src/positions/supply.rs:334-345` (`merge_supply_leg` → `UsageSide::Supply`)
  - `common/src/rates/scaling.rs:17-31` (`calculate_scaled_cap`)
  - `common/src/types/pool.rs:337-341` (`MarketIndexRaw` dual fields)
  - `contracts/pool/src/cache/report.rs:25-62` (pool returns both indexes post-accrual)
- Defense: Every production entry that grows spoke usage passes a full `MarketIndexRaw` from the pool mutation into `apply_spoke_entry`, which selects **exactly one** field via `UsageSide::index` — `supply_index` for supply entries, `borrow_index` for borrow entries — then `calculate_scaled_cap` floors the asset-unit cap into RAY shares at that index. Cap config is also side-correct (`UsageSide::cap`). Exits never rescale and never touch the cap. Zero asset-unit cap yields `cap_scaled = 0` at any positive index (INV-HALT-03).
- Gap: (1) Unit/harness cap tests almost always run with `supply_index == borrow_index == RAY`, so a swapped-side selector would be **silent** in those suites — coverage debt for A085, not a live bug. (2) No dedicated Certora rule that `enforce_spoke_cap` uses `I_supply` vs `I_borrow` by side (cap rules run under unconstrained caps). (3) Hypothetical wrong-side selection after supply-index write-down would over-admit borrow shares; current code does not take that path. Sibling footgun remains A094 (forgotten `put_market_index` for *later* Cache readers) — **not** this entry’s cap path, which uses the mutation DTO directly.
- Impact: Wrong-side index would systematically mis-scale asset-unit caps into share space (false rejects or over-admission up to per-spoke-asset headroom × index-ratio). Current selection prevents that class on all inventoried entry paths. No direct theft; soft governance capacity only.
- Evidence: ADR-0015; INV-HALT-03; formulas.md “Caps and fees”; peers A076, A077, A078, A082, A094, A103 §7.2 provisional; unit `spoke.rs` (`usage_side_cap_reads_matching_field`, zero-cap); harness `spoke_caps.rs`; pool `market_index()` packing both sides.
- Opinion: A103’s provisional read is confirmed. Close A081 as **defended**. Keep “side-matched index” on the review checklist next to A077/A094: any new entry that grows usage must go through `apply_spoke_entry` / `UsageSide`, never hard-code one index field for both sides. Add a divergent-index unit test so swap bugs cannot hide behind `I_s = I_b = RAY`.

---

## 1. Scope and method

Mission: prove or refute that scaled spoke-cap enforcement picks the **matching** market index for the side whose shares are being counted — supply shares ↔ `supply_index`, borrow shares ↔ `borrow_index` — inside `spoke_usage` / `apply_spoke_entry`.

Method:

1. Read `shared/COORDINATION.md`, `SEED.md`, `AGENT_MANIFEST.md` (A081), and peers A076 / A077 / A103 (A081 was coverage debt in A103 §7.2); skim A078 / A082 / A085 / A094 for boundaries.
2. Trace every production call into `apply_spoke_entry` and `enforce_spoke_cap`.
3. Pin the conversion formula, zero-cap behavior, exit path (no index), and impact if sides were swapped.
4. Inventory tests/Certora for divergent-index sensitivity.

Out of scope as primary claims: persist-vs-pool timing (A078), missing-row exit (A080), pool-vs-caller amounts (A082), Credit fee-only intent (A084), multi-asset zip (A079).

No production Rust edited. No git operations (coordination protocol).

---

## 2. Why side-matched index is load-bearing

Spoke usage stores **RAY-scaled shares**, not asset units:

```text
SpokeUsageRaw { supplied_scaled_ray, borrowed_scaled_ray }
```

Configured caps are **native asset units** (`supply_cap` / `borrow_cap` on `SpokeAssetConfig`). Entry converts the configured ceiling into share space before comparing:

```text
cap_scaled = floor( rescale(cap, asset_decimals → RAY) / index )
next_scaled = usage_side + delta_scaled
require next_scaled ≤ cap_scaled
```

Share ↔ asset conversion for positions uses the **same** side’s index (formulas.md / ADR-0003):

| Side | Position math | Cap must use |
|---|---|---|
| Supply | `value ≈ scaled × supply_index / RAY` | `supply_index` |
| Borrow | `debt ≈ scaled × borrow_index / RAY` | `borrow_index` |

If entry used the opposite index, `cap_scaled` would no longer mean “≤ C asset units of this side’s shares.” ADR-0015’s guarantee that “interest-index changes cannot make a configured cap ambiguous” assumes the **correct** live index for that side.

Indexes diverge in production: borrow index grows with borrower interest; supply index grows slower (reserve factor) and can **fall** on bad-debt socialization (floor `SUPPLY_INDEX_FLOOR_RAW = RAY/1000`). Equality at genesis (`RAY`) is the special case, not the steady state.

---

## 3. Selection mechanism (code)

### 3.1 Single choke point: `UsageSide::index`

```44:50:contracts/controller/src/spoke_usage.rs
    /// Returns this side's index (supply or borrow) from `market_index`.
    pub(crate) fn index(self, market_index: &MarketIndexRaw) -> Ray {
        match self {
            Self::Supply => Ray::from(market_index.supply_index),
            Self::Borrow => Ray::from(market_index.borrow_index),
        }
    }
```

Symmetric helpers keep config and errors aligned:

| Helper | Supply | Borrow |
|---|---|---|
| `scaled` / `set_scaled` | `supplied_scaled_ray` | `borrowed_scaled_ray` |
| `cap` | `cfg.supply_cap` | `cfg.borrow_cap` |
| `index` | `market_index.supply_index` | `market_index.borrow_index` |
| `cap_error` | `#311 SpokeSupplyCapReached` | `#312 SpokeBorrowCapReached` |

There is no third path that picks an index without going through this enum.

### 3.2 `apply_spoke_entry` wires side → cap + index

```103:124:contracts/controller/src/context/spoke.rs
    pub(crate) fn apply_spoke_entry(
        &mut self,
        spoke_id: u32,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let spoke_config = self.require_spoke_asset_config(spoke_id, hub_asset);
        let cap = side.cap(&spoke_config);
        let index = side.index(market_index);
        self.require_spoke_usage_context(spoke_id).apply_entry(
            side,
            hub_asset,
            delta_scaled,
            cap,
            index,
            decimals,
        );
    }
```

Production WASM: **only** `apply_leg_usage` (Entry arm) calls `apply_spoke_entry`. Unit tests may call it directly; they still pass a `UsageSide`. Lower-level `SpokeUsageContext::apply_entry` takes an already-selected `Ray` index (test seam); production never bypasses the selector.

### 3.3 Cap math

```147:159:contracts/controller/src/spoke_usage.rs
fn enforce_spoke_cap(...) -> Ray {
    let cap_scaled = calculate_scaled_cap(env, cap, decimals, index);
    let next_scaled = Ray::from(side.scaled(usage)).checked_add(env, delta_scaled);
    assert_with_error!(env, next_scaled <= cap_scaled, side.cap_error());
    next_scaled
}
```

```24:31:common/src/rates/scaling.rs
pub fn calculate_scaled_cap(env: &Env, cap: i128, decimals: u32, index: Ray) -> Ray {
    Ray::from(fp_core::mul_div_floor_saturating(
        env,
        Ray::from_asset(env, cap, decimals).raw(),
        RAY,
        index.raw(),
    ))
}
```

Properties relevant to A081:

| Property | Behavior |
|---|---|
| Rounding | **Floor** → slightly tighter than exact asset-unit ceiling (formulas.md; A059) |
| Overflow | Saturates to `i128::MAX` → fail-open on overflow, not trap (numeric-bounds.md) |
| `cap = 0` | `cap_scaled = 0` for any positive `index` → INV-HALT-03 “zero admits nothing” |
| `index = 0` | `DivisionByZero` panic — unreachable for live markets (indexes init at `RAY`, supply floored ≥ `RAY/1000`) |

Exits (`apply_exit` / `apply_spoke_exit`) subtract scaled shares and **never** call `calculate_scaled_cap` — index selection is entry-only by design (ADR-0015 exit-safe).

---

## 4. Call-graph: side and index stay paired

### 4.1 Ordinary position merges

| Merge | `UsageSide` | Cap index field | Source of `MarketIndexRaw` |
|---|---|---|---|
| `merge_supply_leg` (entry) | `Supply` | `supply_index` | `LegOutcome` from `PoolPositionMutation` |
| `merge_withdraw_leg` (exit) | `Supply` | n/a (exit) | index still refreshed via `put_market_index` for events/risk |
| `merge_debt_leg` borrow (entry) | `Borrow` | `borrow_index` | same |
| `merge_debt_leg` repay (exit) | `Borrow` | n/a | same |

`apply_leg_usage` Entry always forwards `&outcome.market_index` plus the caller-chosen `side` — it does not re-derive side from the DTO:

```125:133:contracts/controller/src/positions/mod.rs
        LegDirection::Entry { asset_decimals } => cache.apply_spoke_entry(
            spoke_id,
            side,
            hub_asset,
            outcome.new_scaled.checked_sub(env, old_scaled),
            &outcome.market_index,
            asset_decimals,
        ),
```

Callers hard-pin the side at the merge site (`UsageSide::Supply` in supply.rs; `UsageSide::Borrow` in `merge_debt_leg`). A mistaken caller side would also mis-book the wrong usage field — still not a silent “right field, wrong index” mix-up inside `UsageSide::index`.

### 4.2 Pool always returns both indexes together

```25:31:contracts/pool/src/cache/report.rs
    pub(crate) fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }
```

Every `PoolPositionMutation` / strategy / net-settle result embeds this pair after the pool’s accrual for that market. The controller does not stitch a supply-only or borrow-only wire type for caps.

### 4.3 Cap uses mutation DTO, not Cache memo (vs A094)

Entry cap reads `outcome.market_index` **passed by value into** `apply_spoke_entry`. It does **not** call `cached_market_index` for the check.

`put_market_index` runs around the same merge (supply entry: after usage; debt entry: after usage; withdraw: before exit usage). A094’s footgun is “later Cache readers see stale index if `put` is forgotten.” **This entry’s cap check remains correct even if `put` were omitted**, because selection is from the mutation argument. A077 and A081 are complementary: A077 = trust pool-returned indexes; A081 = pick the correct field of that return.

### 4.4 Liquidation / strategies / net settle

| Path | Usage entry? | Index concern |
|---|---|---|
| Credit seize | No `apply_spoke_entry` for liquidator credit (cap exemption); fee uses `apply_spoke_exit` Supply | Exit — no cap index |
| Transfer seize | Withdraw-batch exits | Exit |
| Bad-debt cleanup | Exits both sides | Exit |
| Strategy legs | Via `merge_*` / `apply_leg_usage` | Same side pairing as ordinary |
| Net settle | Withdraw + debt **exits** | Exit |

No liquidation or strategy path grows spoke usage with a hard-coded opposite index.

---

## 5. Counterfactual: wrong-side index impact

Let `I_s`, `I_b` be live supply/borrow indexes; `C` the configured asset-unit cap; `U` current scaled usage on that side.

Correct: `cap_scaled = floor(C_ray / I_side)`.

| Mis-selection | When | Effect on headroom |
|---|---|---|
| Supply entry uses `I_b` while `I_b > I_s` (typical accrual) | Steady interest | `cap_scaled` **smaller** → false `SpokeSupplyCapReached` (availability) |
| Borrow entry uses `I_s` while `I_s < I_b` | Steady interest | `cap_scaled` **larger** → **over-admission** of borrow shares |
| Borrow entry uses `I_s` after supply-index write-down | Bad-debt socialized market | `I_s` can approach floor while `I_b` stays high → over-admission factor up to ~`I_b / I_s` (bounded by index domain, not by C alone in share space) |
| Supply entry uses `I_b` after write-down | Same | Tighter supply cap (false reject) |

Blast radius if broken: **per `(spoke, hub_asset, side)` soft cap** — same class as A080 capacity distortion; no HF bypass and no direct mint/theft. The dangerous direction for governance intent is **borrow over-admission after supply-index drop**. Current `UsageSide::index` prevents it.

When `I_s = I_b` (fresh markets / many unit tests), wrong-side selection has **zero** numerical effect — hence the coverage note in §7.

---

## 6. Zero-cap and INV-HALT-03 (pinned)

INV-HALT-03: “Zero cap admits nothing. Entry paths enforce usage at the live index; exits do not consume a cap.”

| Claim | Status under A081 |
|---|---|
| `cap = 0` → `cap_scaled = 0` independent of which positive index is chosen | Holds (`calculate_scaled_cap(0, …) = 0`) |
| Live index is post-mutation pool index | Holds (A077 + this file) |
| Side-correct live index | Holds (`UsageSide::index`) |
| Exit uncapped | Holds (no index in exit path) |

Unit: `zero_supply_cap_rejects_entry` / `zero_borrow_cap_rejects_entry` in `contracts/controller/tests/spoke.rs`. Harness: `test_zero_supply_cap_rejects_every_supply` / `test_zero_borrow_cap_rejects_every_borrow` in `spoke_caps.rs`.

---

## 7. Evidence and coverage debt

### 7.1 What exists

| Layer | What it pins |
|---|---|
| Unit `usage_side_cap_reads_matching_field` | `UsageSide::cap` field mapping (not index) |
| Unit zero / exact / +1 cap / saturate-at-floor | Cap math with caller-supplied index (usually `RAY`) |
| Harness `spoke_caps.rs` | End-to-end supply/borrow cap at live market indexes |
| Certora `usage_*` | Δusage = Δscaled; **not** which index field fed the cap |
| Peers A076/A077/A103 | Semantics + “mutation index” trust boundary; A103 §7.2 provisional = this deep-dive |

### 7.2 Residual coverage (not a production defect)

1. **No `UsageSide::index` unit test** with `supply_index ≠ borrow_index` asserting which field is selected for each side.
2. Cap unit tests at `I = RAY` for both fields cannot catch a swapped match arm.
3. Certora does not assert side↔index coupling under constrained caps (fixtures use unconstrained caps for usage delta rules — A085).

Recommended cheap pin (for A085 / follow-up, not blocking A081 status):

```text
assert UsageSide::Supply.index(MarketIndexRaw { supply: A, borrow: B }) == A
assert UsageSide::Borrow.index(MarketIndexRaw { supply: A, borrow: B }) == B
with A ≠ B
```

Optional: one harness case after interest accrual (or forced index skew) that fills a borrow cap and shows the bound tracks `borrow_index` asset value, not `supply_index`.

---

## 8. Cross-links

| Peer | Relation |
|---|---|
| **A077** | Cap uses pool **mutation** indexes — complementary to A081’s **which field** |
| **A076** | Entry semantics; cap via `calculate_scaled_cap` |
| **A082** | Deltas from pool scaled amounts; same trust boundary |
| **A094** | Stale Cache index after forgotten `put` — affects later readers, not this entry’s DTO path |
| **A078** | Lists A081 as sibling out-of-scope; timing orthogonal |
| **A080** | Leading T5 residual (missing-row exit); orthogonal to index selection |
| **A103 §7.2** | Provisional “unlikely novel critical” — **confirmed** |
| **A085** | Owns divergent-index / Certora cap-index coverage debt |
| **A059** | Floor rounding of `calculate_scaled_cap` |

No disagreement file required: consistent with A077 and A103 provisional.

---

## 9. Verdict

**Defended.** Scaled spoke-cap enforcement selects `supply_index` for supply entries and `borrow_index` for borrow entries through a single `UsageSide::index` choke point in `apply_spoke_entry`, fed by the pool’s post-mutation `MarketIndexRaw`. Zero caps remain literal; exits never rescale. Wrong-side selection would be a soft-cap integrity bug (especially borrow over-admission after supply-index write-down); it is not present on inventoried paths.

Close wave-5 coverage debt for A081. Keep side-matched index on the merge checklist with A077/A094; prefer a divergent-index unit pin so future refactors cannot hide behind equal indexes.
