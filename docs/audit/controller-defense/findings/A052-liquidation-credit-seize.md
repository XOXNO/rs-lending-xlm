# A052 — Liquidation Credit seize (share credit, no token move)

- Agent: A052
- Theme: T3 (money / claim movement without underlying transfer), overlaps T5 (spoke usage) and T2 (storage apply)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs` (`liquidate` → `process_liquidation`)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-153,170-216` (`process_liquidation`, `resolve_seize_receiver`)
  - `contracts/controller/src/positions/liquidation/apply.rs:123-321` (`apply_liquidation_share_credit`, `credit_supply_shares`, `record_share_credit_updates`, `require_credit_position_limit`, `credited_shares`)
  - `contracts/controller/src/positions/liquidation/math.rs:249-367,369-404,455-491` (`calculate_seized_collateral` scaled fields, `split_seized_shares`, `scale_seizures_to_received`)
  - `contracts/controller/src/views.rs:189-242` (Credit estimate units)
  - `contracts/pool/src/ops/seize.rs` (Deposit → `absorb_supply_as_revenue`)
  - `contracts/pool/src/cache/shares.rs:41-48` (`absorb_supply_as_revenue` vs `accrue_revenue` mint)
  - `contracts/pool/src/ops/withdraw.rs:106-131` (`withhold_liquidation_fee` mint path — must **not** be used here)
  - `common/src/types/controller.rs:228-281` (`SeizeMode::Credit`, `SeizeEntry` dual representation)
  - ADR-0019; INV-LIQ-01/02/03; INV-ACCT-01 (revenue backed); INV-HALT-02 (`SeizureLeg`)
- Defense: Credit mode moves RAY supply shares account→account with **no** pool cash debit and **no** `supplied` change. Per-leg identity `seized_scaled == fee_scaled + liquidator_scaled` is asserted in `split_seized_shares` (ceil fee on bonus only) and re-asserted in apply. Protocol fee leaves the account system via `PoolSeizeEntry { Deposit }` → `absorb_supply_as_revenue` (**reclassify** revenue↑ only). Spoke usage nets to **−fee** only (debit+credit cancel; cap entry skipped by design). Receiver never inherits victim risk stamps; new slots take **current** listing. Index-independent apply (scaled fields only).
- Gap: (1) Shared A080 — fee `apply_spoke_exit` no-ops if usage row missing. (2) New Credit slot calls `require_spoke_asset` — **delisted** hub cannot open a fresh receiver position (Transfer still works; A026 §7.2). (3) Transfer vs Credit fee **magnitudes** differ by design (asset floor+bump+mint vs RAY ceil+absorb) — not theft, but keeper economics diverge. (4) Event docs equate fee to `LiqSeize.amount − LiqCredit.amount` in asset units; share conservation is exact in RAY, asset-gap identity is fixture-pinned at unit index. (5) `is_collateralizable=false` does not zero stamped LTV on Credit open — governance must cut LTV separately if that is intended.
- Impact: No demonstrated path to mint unbacked supplier claims, invent/destroy shares, credit the victim, open a foreign-spoke Credit slot, or move pool cash on Credit seize. Residuals are liveness (delist+empty receiver), cap bookkeeping (A080), and integrator/fee-parity nuance.
- Evidence: ADR-0019; unit `liquidation_seize_modes.rs` (conservation, ceil, under-delivery re-split, full-close exact scaled); harness `liquidation_seize_modes.rs` (supplied/cash untouched, usage −fee, risk stamp, non-collateralizable credit, cap bypass, two batches, estimate=execution); Certora `usage_liq_credit_seize_sums_over_two_accounts`, `usage_liq_credit_fee_exits_usage_reachable`; peers A013 (receiver identity), A026 (storage), A084 (usage), A080 (exit no-op).
- Opinion: This is the load-bearing ADR-0019 surface. The critical negative — do not route Credit fees through `withhold_liquidation_fee` / `accrue_revenue` — is correctly implemented and heavily commented. Treat as **defended** for fund safety; ship residuals as known ops/liveness notes.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git ops).
2. Trace Credit seize after repay + `scale_seizures_to_received`: limit check → `apply_liquidation_share_credit` → dual finalize → events.
3. Audit share conservation, fee base (bonus only), rounding direction, pool primitive (absorb vs mint), usage delta, risk-stamp policy, halt gates, under-delivery, estimate view, and Certora/harness pins.
4. Out of scope as primary claims: receiver auth / Credit(0) (A013), Transfer cash path (A051 if present), bad-debt body (A027), fee *curve* / close-factor depth (A053 if present). Cross-ref those peers where they touch Credit.

---

## Call graph (Credit seize only)

```
process_liquidation
  ├─ resolve_seize_receiver → Some(receiver)          # A013; before money
  ├─ build_liquidation_plan → SeizeEntry{scaled,bonus,fees,…}
  ├─ apply_liquidation_repayments                     # measured tokens → pool (only cash move)
  ├─ scale_seizures_to_received                       # floor all four size fields
  ├─ require_credit_position_limit                    # before any Credit mutate
  ├─ apply_liquidation_share_credit
  │    per leg:
  │      SeizureLeg (no_seize only)
  │      split_seized_shares → (fee, liquidator)      # ceil; assert S = fee + L
  │      assert same spoke; assert S − L == fee
  │      debit victim S; LiqSeize event (gross amount)
  │      credit_supply_shares(receiver, L)            # no usage entry; no is_collateralizable
  │      if fee > 0: apply_spoke_exit(fee); queue PoolSeizeEntry{Deposit, fee}
  │    pool_seize_positions_call → absorb_supply_as_revenue
  ├─ finalize(liquidated, Both)
  ├─ record_share_credit_updates → LiqCredit (net)
  └─ finalize(receiver, Supply)
```

No `pool_withdraw_call`, no liquidator wallet credit, no `withhold_liquidation_fee`.

---

## 1. Settlement model (no token move)

| Surface | Credit seize effect |
|---|---|
| Pool `cash` | Unchanged |
| Pool `supplied` | Unchanged |
| Pool `revenue` | ↑ by Σ `fee_scaled` (absorb) |
| Victim supply map | ↓ by Σ `seized_scaled` (gross) |
| Receiver supply map | ↑ by Σ `liquidator_scaled` (= S − fee) |
| Spoke supply usage | ↓ by Σ `fee_scaled` only |
| Underlying ERC-20 / SAC | Only liquidator→pool **repay** transfers earlier in the call |

ADR-0019: pool never tracks per-account positions, so account↔account share moves need no pool mutation except fee reclassification. Solvency of `cash`/`supplied` is preserved by construction.

Harness pin: `credit_mode_leaves_supplied_and_cash_untouched_and_moves_only_revenue` asserts `credited + fee == seized` and unchanged `supplied`/`cash`.

---

## 2. Share conservation (`split_seized_shares`)

```369:404:contracts/controller/src/positions/liquidation/math.rs
/// fee = ceil(liquidation_fees × bonus_scaled) … liquidator takes the exact remainder
pub(crate) fn split_seized_shares(...) -> (Ray, Ray) {
    // rejects: negative inputs, bonus > seized, fees >= BPS
    let fee_scaled = Ray::from(mul_div_ceil(env, bonus_scaled.raw(), fees, BPS));
    if fee_scaled > seized_scaled { panic InternalError }
    let liquidator_scaled = seized_scaled.checked_sub(env, fee_scaled);
    // assert fee + liquidator == seized
    (fee_scaled, liquidator_scaled)
}
```

Properties verified in unit tests:

| Property | Result |
|---|---|
| `S == fee + liquidator` across magnitudes / fee rates | Conserved |
| Fee rounds **ceil** (protocol-favouring), not half-up | `fee_rounds_up_not_half_up` |
| Fee base is **bonus only**; zero bonus → fee 0 | `zero_bonus_gives_the_liquidator_the_whole_seizure` |
| Zero fee rate → liquidator gets all of S | `zero_fee_rate_…` |
| 1-share seizure + any positive rate → fee=1, liquidator=0 | Still conserves; `LiqCredit` leg omitted |
| `bonus > seized` or `fees >= BPS` | `InternalError` (#34) |

Apply re-asserts `seized − liquidator == fee` before booking. `credited_shares` re-derives the same split from the entry so limit checks, events, and apply cannot disagree.

**Fee rate source:** `SeizeEntry.liquidation_fees` is the **victim position’s stamped** rate (planner copies it). Live listing changes do not alter mid-flight Credit fees. Correct for stamped-risk policy; Transfer’s asset `protocol_fee` is planned separately (half-up RAY → asset floor + bump-to-1).

---

## 3. Fee reclassification (absorb, not mint)

### Correct path

```130:135:contracts/controller/src/positions/liquidation/apply.rs
/// … PoolSeizeEntry { side: Deposit } … absorb_supply_as_revenue:
/// *reclassifies* shares that already exist, raising `revenue` alone.
/// Transfer's withhold_liquidation_fee … *mints* new revenue shares …
```

```45:48:contracts/pool/src/cache/shares.rs
pub(crate) fn absorb_supply_as_revenue(&mut self, scaled: Ray) {
    self.revenue = self.revenue.checked_add(&self.env, scaled);
    self.require_revenue_backed();  // revenue ≤ supplied
}
```

Contrast Transfer mint:

```110:128:contracts/pool/src/ops/withdraw.rs
// withhold_liquidation_fee → add_protocol_revenue → accrue_revenue
// mints revenue **and** supplied, backed only because cash was withheld
```

| If Credit used mint | Effect |
|---|---|
| `supplied` ↑ by fee while user books only move S | Extra supplier claims with no new assets |
| Local arithmetic can still “balance” | Silent underbacking (ADR-0019’s named footgun) |

**No path found** that sends Credit fees through `withhold_liquidation_fee` or `accrue_revenue`. Zero fee skips the pool call entirely (`if !fee_entries.is_empty()`).

### Order vs atomicity

Victim debit and receiver credit happen in controller RAM first; pool absorb runs after the loop. Soroban rolls the whole transaction back on any panic — no durable half-credit without matching absorb (or vice versa).

---

## 4. Planner scaled fields (input to Credit)

`calculate_seized_collateral` builds both Transfer asset fields and Credit share fields in one pass:

- Full close: `seized_scaled = position.scaled_amount` **verbatim** (no asset round-trip).
- Partial: `capped_ray.div_floor(supply_index)`.
- `bonus_scaled = floor(bonus_ray / index).min(seized_scaled)` so `bonus ≤ seized` for the split.
- Asset `protocol_fee` (Transfer only) uses half-up on bonus RAY → floor asset + bump; **not** consumed by Credit.

Under-delivery (`scale_seizures_to_received`) floor-scales `amount`, `protocol_fee`, `scaled_amount`, `bonus_scaled` by the same ratio. Credit **re-splits after scaling**, so conservation holds on scaled totals (unit test `under_delivery_scaling_keeps_the_bonus_bounded_and_the_split_exact`). Fee is not carried as a precomputed scaled field across the shrink.

**Index immunity (Credit):** apply moves only `scaled_amount` / derived fee shares. Plan vs apply index drift cannot invent shares on this path. Transfer lacks that property (A026 / A094).

**Dust asymmetry vs Transfer:** after under-delivery, `amount` can floor to 0 while `scaled_amount > 0`. Transfer withdraw becomes a no-op; Credit still moves shares. Favours protocol completeness on Credit / leaves dust on victim for Transfer — not over-seize.

---

## 5. Receiver credit semantics (`credit_supply_shares`)

```223:251:contracts/controller/src/positions/liquidation/apply.rs
/// existing position keeps its own stamped risk tuple …
/// new position stamped from *current* listing …
/// liquidated account's tuple never travels …
/// No entry gate: is_collateralizable / supply cap must not block
```

| Rule | Enforcement |
|---|---|
| Same spoke | `resolve_seize_receiver` + apply assert |
| Not victim id | A013 / `#133` |
| Owner or delegate | A013 |
| `PositionMode::Normal` | A013 |
| `max_supply_positions` | `require_credit_position_limit` **before** apply (actionable via `Credit(0)`) |
| Skip supply-cap entry | Intentional; usage nets −fee (ADR-0015/0019) |
| Skip `is_collateralizable` | Intentional; harness `a_non_collateralizable_asset_can_still_be_credited` |
| Halt | `FreezePolicy::SeizureLeg` → `no_seize` only; paused OK (ADR-0008) |
| `scaled == 0` | Early return (fee-eats-all) |

Harness: current listing stamp on empty receiver; existing receiver keeps its tuple and grows; spoke-at-cap still credits.

### Residual — delisted hub, new slot

`require_spoke_asset` panics with `AssetNotInSpoke` when the listing row is gone. Victim debit path tolerates missing listing (`enforce_spoke_asset_flags` no-op), but opening a **new** receiver slot does not. Workarounds: `Transfer`, or Credit to a receiver that already holds that hub. Documented also in A026 §7.2. Liveness hole, not share inflation.

### Residual — `is_collateralizable=false` + nonzero listing LTV

Credit stamps LTV/threshold from listing even when collateralizable is false. Ordinary supply stays blocked; Credit can still open a borrowing-capable stamp if governance left LTV &gt; 0. Config footgun; ADR explicitly skips the collateralizable gate.

---

## 6. Spoke usage (fee-only exit)

Identity written in apply comments: `-S + (S − fee) = −fee`.

- Account↔account half never calls `apply_spoke_entry` (would put liquidation behind the supply cap).
- Only `fee_scaled > 0` calls `apply_spoke_exit(Supply, fee)`.
- Certora: `usage_liq_credit_seize_sums_over_two_accounts` pins sum-over-accounts Δ = −fee.
- Harness: `credit_mode_moves_spoke_usage_by_exactly_the_protocol_fee`.

**Residual A080:** missing usage row → exit no-op → capacity can stay overstated; does not mint shares or skip absorb.

Dual finalize (victim `Both`, then receiver `Supply`) re-persists the same Cache usage rows; deltas are applied once in memory (A084).

---

## 7. Events and estimate view

| Tag | Account | Amount sense |
|---|---|---|
| `LiqSeize` | Liquidated | Gross (`entry.amount` asset units at plan index) |
| `LiqCredit` | Receiver | Net of fee (`floor(liquidator_scaled × supply_index)` to asset); **omitted** when net is 0 |

Two `UpdatePositionBatchEvent`s, liquidated first. Fee-eats-all omits receiver deposit legs (ADR-0019).

Unit events test (unit index fixture): share conservation exact; asset gap `LiqSeize.amount − LiqCredit.amount == floor(fee_shares → asset)`. Docs state that identity generally; load-bearing safety property is RAY conservation + absorb, not the asset event gap under arbitrary indexes.

Estimate view (`liquidation_estimations_detailed`): Credit reports `seized_collaterals = scaled_amount`, `protocol_fees = fee_scaled` from the same split. Harness: estimate matches execution debit; receiver gets estimate seize − fee.

Does **not** call `resolve_seize_receiver` — view-only; no Credit-back bypass.

---

## 8. Attack / bypass matrix

| Attempt | Outcome |
|---|---|
| Credit fee via mint/withhold | Impossible — Deposit seize → absorb only |
| Invent shares in split | Asserted identity; ceil fee ≤ bonus &lt; BPS |
| Credit more than seized | Debit uses full S; credit = S − fee; `checked_sub` on victim |
| Credit victim account | `#133` (A013) |
| Foreign spoke / strategy mode | SpokeMismatch / AccountModeMismatch |
| Cap-full spoke blocks Credit | Skipped entry; intentional |
| `is_collateralizable=false` blocks Credit | Does not block |
| `no_seize` | Blocks leg |
| Paused collateral blocks Credit | Does not (`SeizureLeg`) |
| Under-delivery keeps full seize | Scaled down before apply (INV-LIQ-03) |
| Index drift inflates Credit shares | Immune — scaled apply |
| Double usage decrement on dual finalize | Cache deltas once; persist twice idempotent |
| Position limit DoS | Revert before mutate; `Credit(0)` fallback |
| Delisted + Credit(0) new slot | Reverts on stamp (`require_spoke_asset`) — liveness residual |
| Swap Transfer fee math onto Credit | Different units by design; both conserve their own books |

---

## 9. Invariant checklist

| Claim | Verdict |
|---|---|
| No pool cash / supplied move on Credit seize | Match |
| `seized == fee + credited` (RAY) | Match (asserted + tested) |
| Fee on bonus only; ceil | Match |
| Fee = absorb reclassify, not mint | Match |
| Usage Δ (sum of accounts) = −fee | Match |
| Same-spoke + Normal + not-self | Match (A013) |
| Victim risk tuple not imported | Match |
| Seizure coupled to measured repay | Match (scale then apply) |
| Full close exact scaled position | Match |
| Missing usage row on fee exit | Residual (A080) |
| Delisted new Credit slot | Residual (liveness) |
| Transfer↔Credit fee size parity | Not required; magnitudes may differ |

---

## 10. Cross-refs

| Peer | Relationship |
|---|---|
| A013 | Receiver identity / Credit(0) / self-credit ban — assumed here |
| A026 | Durable writes / dual finalize / Transfer vs Credit fee primitives |
| A084 | Fee-only usage; dual-finalize Cache story |
| A080 | Fee exit no-op if row missing |
| A051 | Transfer money movement (out of scope) |
| A053 | Fee curve / close arithmetic depth (out of scope beyond split) |
| A007 | Flash reentrancy blocked before plan |

---

## Verdict

Credit-mode liquidation share credit is **defended**. Shares conserve exactly under ceil-on-bonus fees; the only pool mutation is revenue **reclassification**; pool liquidity is irrelevant to completion; usage falls by the fee alone; and receiver risk stamps cannot import the victim’s stale tuple. The critical ADR-0019 failure mode (minting Credit fees) is absent from the call graph.

Residual risk is operational: delisted collateral cannot open a brand-new Credit slot, spoke-usage exits may no-op (A080), and Transfer/Credit fee sizes / event asset gaps are not the same numeric object as the RAY fee. None of these yield unbacked claims or silent share creation under current source.
