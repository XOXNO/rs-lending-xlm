# A053 — Protocol fee skim on liquidation

- Agent: A053
- Theme: T3 (money movement — liquidation protocol fee skim)
- Severity: low
- Status: defended (accepted residuals documented)
- Paths:
  - `contracts/controller/src/positions/liquidation/math.rs:241-367` (`calculate_seized_collateral` — bonus-only fee base, asset fee + dust bump)
  - `contracts/controller/src/positions/liquidation/math.rs:369-404` (`split_seized_shares` — Credit conservation)
  - `contracts/controller/src/positions/liquidation/math.rs:455-491` (`scale_seizures_to_received`)
  - `contracts/controller/src/positions/liquidation/math.rs:44-67` (`LiquidationPlan::validate` fee bounds)
  - `contracts/controller/src/positions/liquidation/apply.rs:85-121` (`apply_liquidation_seizures` — Transfer)
  - `contracts/controller/src/positions/liquidation/apply.rs:123-221` (`apply_liquidation_share_credit` — Credit absorb)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-153` (`process_liquidation` order)
  - `contracts/controller/src/positions/supply.rs:283-301` (`WithdrawKind::Liquidation` → pool)
  - `contracts/controller/src/views.rs:199-233` (estimate mode-aware fee units)
  - `contracts/pool/src/ops/withdraw.rs:42-131` (`withhold_liquidation_fee` + mint)
  - `contracts/pool/src/ops/seize.rs:18-35` / `contracts/pool/src/cache/shares.rs:41-48` (`absorb_supply_as_revenue`)
  - `contracts/pool/src/interest.rs:57-68` (`add_protocol_revenue` → `protocol_fee_shares` floor)
  - `common/src/validation.rs:85-93` (`validate_liquidation_fees` — fees `< BPS`)
  - `common/src/types/controller.rs:250-281` (`SeizeEntry` dual representation contract)
- Defense: Fee is charged only on the realised bonus, never on principal. Transfer withholds asset units from the liquidator payout and **mints** revenue shares against cash kept in the pool. Credit re-derives a share split with protocol-favourable ceil, asserts `S = fee + liquidator`, and **reclassifies** existing shares via absorb (must not mint). Under-delivery floor-scales Transfer’s fee asset field and Credit’s fee *base*; Credit re-splits after scale so conservation still holds exactly. Pool fee/withdraw/seize entrypoints are owner-only (controller). Config rejects `liquidation_fees >= BPS`. Plan validate enforces `0 ≤ protocol_fee ≤ amount` and `bonus_scaled ≤ scaled_amount`.
- Gap: (a) Known dust-fee bump can charge one asset unit when the realised excess floors to zero — liquidator can be one unit underwater on dust legs (`the_dust_fee_bump_charges_more_than_the_realised_excess`; mitigated by min-collateral floors in `numeric-bounds.md` §6). (b) Transfer fee is plan-time asset units and is index-sensitive at apply (possible `WithdrawLessThanFee` / reserve revert — liveness, Credit exists for cash-starved markets). (c) Transfer vs Credit fee magnitudes are not bit-identical for the same seizure (half-up→floor→bump in asset space vs `mul_div_ceil` in share space) — intentional dual representation. (d) Stamped `liquidation_fees` freeze on liquidation restamp (A026) — old stamps can diverge from live listing. None mint unbacked Credit revenue or skim principal as fee by construction.
- Impact: Successful skim moves value from liquidator proceeds (bonus slice) into pool `revenue`. Cannot invent supplier claims on the Credit path; Transfer mint is cash-backed by the withheld net. Cannot charge fee on debt principal / repayment. Dust bump can slightly worsen liquidator PnL on sub-unit bonuses; does not drain third-party accounts or inflate victim collateral. Blast radius of a wrong mint-vs-absorb swap would be market underbacking — that swap is explicitly rejected and test/Certora-pinned.
- Evidence: formulas.md Liquidation; INV-LIQ-02/03; ADR-0019; ADR-0003; pool README mint vs absorb table; Certora `protocol_fee_bonus_math`, `liquidation_withdraw_books_protocol_fee`, spoke usage Credit/Transfer fee rules; unit `liquidation_math.rs` / `liquidation_seize_modes.rs`; harness `liquidation_seize_modes.rs`, `liquidation.rs::test_liquidation_protocol_fee_on_bonus_only`; events TOB-AAVE-4 gross/net pins; peers A013, A026, A084, A080.
- Opinion: The load-bearing defense is mode-split booking (Transfer mint-withhold vs Credit absorb) plus bonus-only base with conservation on Credit. Residuals are economic dust / Transfer liveness / stamp lag — not a missing skim gate or double-charge.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git ops).
2. Trace fee from stamped `liquidation_fees` → plan (`calculate_seized_collateral`) → under-delivery scale → Transfer withhold / Credit `split_seized_shares` + absorb → usage + events + estimate view.
3. Cross-check pool mint vs absorb invariants, Certora fee rules, harness/unit pins, and peer findings A013/A026/A084/A080.
4. Out of scope as primary claims: receiver identity (A013), bad-debt socialization body (A027), Transfer token measurement depth (A051), spoke-usage missing-row no-op beyond fee exit (A080), curve close-factor sizing except where it feeds the bonus base.

---

## Verdict

**Defended.** Protocol fee skim on liquidation is bonus-only, mode-correctly booked, conservation-checked on Credit, and owner-gated at the pool. The only material residual is the documented one-unit dust bump (and Transfer’s known index/cash liveness), not silent principal skim or unbacked revenue mint.

---

## 1. Economics — fee on bonus only

Configured rate `liquidation_fees` (BPS, stamped on the supply position) must be **strictly below** `BPS` (`validate_liquidation_fees`). Risk docs require the fee to come **out of** the bonus, not on top of it (`docs/reference/formulas.md`).

In `calculate_seized_collateral`:

1. Target seizure USD = `repay_usd × (1 + bonus)`.
2. Per collateral leg, pro-rata share → `seizure_ray`, clamped to held value `actual_ray`.
3. **Fee base is not** `capped / (1+bonus)` after clamp. Base is the pre-cap repayment share:
   - `base_ray = seizure_ray.div_floor(one_plus_bonus)`
   - `bonus_ray = capped_ray − base_ray` if positive, else **zero**
4. So a bad-debt close that clamps at or below the repayment share realises **no** excess → **no** fee. Certora `protocol_fee_bonus_math` pins this; unit tests `seizure_at_exactly_the_debt_value_charges_no_protocol_fee`, `a_zero_bonus_never_charges_a_protocol_fee`.
5. Transfer asset fee: `protocol_fee_ray = position.liquidation_fees.apply_to_ray(bonus_ray)` (half-up), then `to_asset_floor`, optional **dust bump** to `1` when `protocol_fee_ray > 0 && fee_asset == 0`, then `min(pool_gross)`.
6. Credit share fields: `bonus_scaled = floor(bonus_ray / supply_index).min(seized_scaled)` carried on the entry; fee itself is **not** stored as shares — re-derived at apply.

Harness pin: `the_protocol_fee_is_charged_on_the_bonus_not_the_gross_seizure` — `fee / seized` is strictly below the fee rate; fee matches bonus portion within 1 unit. Integration: `test_liquidation_protocol_fee_on_bonus_only`.

**Non-finding:** charging the gross seizure would take ~fee_rate of principal; that shape is refuted by tests and the base_ray construction.

---

## 2. Dual representation on `SeizeEntry`

| Field | Transfer use | Credit use |
|---|---|---|
| `amount`, `protocol_fee` | Gross asset seize; fee withheld from payout | Ignored for split (estimate still can report Transfer units if mode=Transfer) |
| `scaled_amount`, `bonus_scaled`, `liquidation_fees` | Carried / scaled for consistency | Gross shares `S`; fee base; stamped BPS → `split_seized_shares` |

Documented on the type (`common/src/types/controller.rs`). Estimate view selects units by `SeizeMode` so Credit reports RAY shares for both seized and fee (`views.rs:213-223`).

`LiquidationPlan::validate` requires:

- `amount > 0`, `0 ≤ protocol_fee ≤ amount`
- `scaled_amount > 0`, `0 ≤ bonus_scaled ≤ scaled_amount`
- `liquidation_fees < BPS`

---

## 3. Transfer path — withhold + mint (cash-backed)

```
apply_liquidation_seizures
  → PoolWithdrawEntry { amount: gross, protocol_fee }
  → apply_withdraw_batch(WithdrawKind::Liquidation)
  → pool.withdraw(..., is_liquidation=true)   # #[only_owner]
```

Pool `withdraw::accounting` order:

1. Resolve burn / gross from live position.
2. **`withhold_liquidation_fee`**: if liquidation and fee ≠ 0, require `gross ≥ protocol_fee`, convert fee asset → Ray, `add_protocol_revenue` → `protocol_fee_shares` (**floor**) → `accrue_revenue` (`revenue += s`, **`supplied += s`**), return `gross − fee`.
3. Burn supply shares for the **gross**.
4. Debit cash for the **net** only; skip utilization cap; still require reserves + solvent withdraw.

Economic identity:

- Cash kept in pool ≈ fee (exact asset units withheld).
- Minted revenue shares floor so claim value ≤ fee Ray (`liquidation_withdraw_books_protocol_fee` asserts floored share value ≤ fee).
- User loses gross shares; liquidator receives net tokens; `actual_amount` on the mutation is **gross** (pool README trap).

Controller usage: full burned Δ exits spoke supply (fee portion included). Booking a separate fee exit here would **double-count** (Certora comment on `usage_liq_transfer_seize_leg_tracks_scaled_delta`).

Events: `LiqSeize` amount = pool gross (fee still inside). Unit `transfer_mode_seizure_delta_is_gross_of_the_protocol_fee` pins TOB-AAVE-4 shape: liquidator tokens = gross − fee.

**Auth:** pool `withdraw` / `seize_positions` are `#[only_owner]` — external parties cannot set `is_liquidation` + arbitrary `protocol_fee` without controller ownership.

---

## 4. Credit path — split + absorb (must not mint)

```
apply_liquidation_share_credit
  → split_seized_shares(S, bonus_scaled, fees)  # fee = ceil(bonus × fees / BPS)
  → assert fee + liquidator == S
  → debit victim by S; credit receiver by S − fee
  → apply_spoke_exit(Supply, fee) only
  → pool_seize_positions(Deposit, fee) → absorb_supply_as_revenue
```

`split_seized_shares`:

- Panics on negative legs, `bonus > seized`, or `fees >= BPS`.
- `mul_div_ceil` on the bonus base (protocol-favourable).
- Asserts conservation identity in production (not only tests).

**Why absorb:** Transfer’s withhold mint is correct only because cash equal to the fee stayed in the pool. Credit moves no cash; minting would create supplier claims with nothing behind them (ADR-0019; apply.rs comments). Absorb: `revenue += fee`, **`supplied` unchanged**, then `require_revenue_backed`.

Harness: conservation `credited + fee == seized`; `cash`/`supplied` unchanged aside from revenue reclass; usage falls by exactly `fee` (`credit_mode_moves_spoke_usage_by_exactly_the_protocol_fee`). Unit sweep: `split_conserves_shares_across_the_decimal_range`.

One-share edge: any positive fee rate on a 1-share bonus-full seizure rounds fee to 1, liquidator gets 0 — still conserves (`one_scaled_unit_seizure_conserves`).

---

## 5. Pipeline order and under-delivery

`process_liquidation`:

1. Auth + flash guard + resolve receiver (before token moves).
2. Build/validate plan (fee fields included).
3. Apply repayments with **measured** receipt; floor-scale leg USD on shortfall.
4. `scale_seizures_to_received(received_usd, planned_usd)` — floor-scales `amount`, `protocol_fee`, `scaled_amount`, `bonus_scaled` by the same ratio.
5. Transfer seize **or** Credit share credit.
6. Observational `LiquidationEvent` (repaid USD = measured; **no** fee field — fees live in position batches).
7. Finalize victim (Both); Credit finalize receiver (Supply); optional bad-debt cleanup.

Under-delivery contract (INV-LIQ-03):

- Transfer: fee asset scales with seizure; `fee ≤ amount` preserved when it held pre-scale.
- Credit: **fee is re-derived** from scaled `bonus_scaled` at every use site (`credited_shares`, apply loop) so conservation is exact after scale rather than carried across independent floors (math.rs comment on `scale_seizures_to_received`). Unit tests assert post-scale split conservation.

Zeroed legs after scale become no-ops (gross 0 / fee 0 / no share move).

---

## 6. Rounding map (protocol-favourable where it matters)

| Site | Direction | Effect on fee skim |
|---|---|---|
| Bonus base after clamp | excess only if `capped > base` | No invented fee on bad-debt close |
| Transfer `apply_to_ray` | half-up | BPS × bonus |
| Transfer asset conversion | floor + optional bump to 1 | Dust may over-charge ≤ 1 unit |
| Transfer fee vs gross | `min(pool_gross)` + pool `gross ≥ fee` | Fail closed if inconsistent |
| Credit split | `mul_div_ceil` on bonus shares | Protocol gets round-up; liquidator exact remainder |
| Transfer revenue mint shares | `protocol_fee_shares` floor | Mint claim ≤ withheld cash value |
| Under-delivery scale | floor on all scaled fields | Never inflates seizure/fee vs receipt |

Certora `protocol_fee_bonus_math` allows `fee_final ≤ bonus + 1` precisely because of the bump; `liquidator_net_is_non_negative_for_any_clamp` **excludes** the bump from the non-negativity claim.

---

## 7. Accepted residuals

### 7.1 Dust fee bump (known, recorded)

When `protocol_fee_ray > 0` but floors to 0 asset units, fee becomes `1`, capped by `pool_gross`. Fixture with 1-stroop repayment: realised excess floors to 0, bump still charges 1 → liquidator net −1 stroop vs repayment (`the_dust_fee_bump_charges_more_than_the_realised_excess`).

Mitigation: protocol min-borrow / bad-debt USD floors make such legs economically irrelevant for listed assets (`numeric-bounds.md` §6; V* ≪ $5 floor). Not a third-party theft vector — liquidator chooses the repay size.

### 7.2 Transfer index / cash liveness

Plan-time `protocol_fee` is applied against live pool resolve. Index drift or thin cash can revert (`WithdrawLessThanFee`, `require_reserves`). Credit mode is the intentional escape hatch (ADR-0019). Index sensitivity of Transfer seize (not Credit) is also noted by A026.

### 7.3 Mode fee asymmetry

Same economic seizure can yield different numeric fees in Transfer asset units vs Credit share ceil. Integrators must use the mode-aware estimate. Not a double-skim: only one mode runs per call.

### 7.4 Stamped fee rate

Fee BPS are position-stamped; liquidation withdraw/Credit debit **do not restamp** risk tuples. Governance changing live `liquidation_fees` does not rewrite in-flight liquidations’ stamps. Can make fee higher or lower than current listing — consistent with other stamped risk fields (peer risk/params coverage).

### 7.5 Spoke usage exit no-op (inherited A080)

Credit `apply_spoke_exit(fee)` no-ops if no usage row exists — cap/governance hygiene, not fee under-booking in the pool (absorb still runs when `fee_scaled > 0`).

---

## 8. What would be undefended (checked absent)

| Failure mode | Status |
|---|---|
| Fee charged on gross seizure / debt principal | Absent — bonus-only base + harness |
| Credit fee via `withhold_liquidation_fee` / `accrue_revenue` mint | Absent — absorb only; ADR-0019 |
| Transfer fee without withholding cash | Absent — net debit = gross − fee |
| Shares invented on Credit split | Absent — asserted conservation |
| Double usage exit of fee on Transfer | Absent — full Δ once |
| Missing usage exit of fee on Credit | Absent — explicit exit; harness |
| Permissionless pool fee injection | Absent — `#[only_owner]` |
| `liquidation_fees == BPS` at config | Rejected — `InvalidLiqThreshold` |
| Fee credited back to victim account | Receiver gates (A013); split never credits victim |
| Estimate reporting net as gross | Mode-aware; events LiqSeize gross / LiqCredit net |

---

## 9. Evidence matrix

| Claim | Evidence |
|---|---|
| Bonus-only fee | `math.rs:299-310`; harness bonus-vs-gross; formulas.md |
| Transfer mint + cash withhold | `withdraw.rs:110-131`; Certora `liquidation_withdraw_books_protocol_fee` |
| Credit absorb not mint | `apply.rs:130-135`; `shares.rs:41-48`; ADR-0019 |
| Credit conservation | `split_seized_shares`; unit decimal sweep; harness e2e |
| Under-delivery fee scale | `scale_seizures_to_received`; Credit re-split |
| Gross/net events | `events.rs` TOB-AAVE-4 tests; STRIDE Repudiate.2 |
| Usage Transfer full / Credit fee | Certora spoke rules; A084; harness fee-only usage |
| Config fee `< BPS` | `validate_liquidation_fees` |
| Dust bump known | unit + numeric-bounds §6 |

---

## 10. Peer cross-links

| Peer | Relation |
|---|---|
| A026 | Storage write set for fee mint vs absorb; agrees mode split is load-bearing |
| A084 | Credit fee-only usage; Transfer full exit — no double-count |
| A013 | Receiver identity; fee path does not undo seizure via credit-back |
| A080 | Fee exit may no-op without usage row |
| A027 | Bad-debt absorb is a sibling reclassify primitive, not this skim |
| A033 | Event order; fee visible as LiqSeize−LiqCredit (Credit) or withhold (Transfer) |

---

## Opinion

Liquidation protocol fee skim is one of the easier places to silently underback a market (Credit mint) or to over-tax liquidators (gross base). The implementation separates the two booking primitives correctly, charges only realised bonus, and pins conservation and gross/net reporting in production asserts plus a dense test/Certora surface. Treat the dust bump and Transfer liveness as accepted residuals already owned by numeric-bounds / ADR-0019 — not as open skim bugs.
