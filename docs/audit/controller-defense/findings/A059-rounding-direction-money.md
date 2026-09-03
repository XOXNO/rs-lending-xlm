# A059 — Rounding direction favours protocol on money paths
- Agent: A059
- Theme: T3 / T8 (money movement + undefended-gap scan)
- Severity: info
- Status: defended
- Paths:
  - `docs/explanation/decisions/0003-ray-scaled-shares-directed-rounding.md`
  - `docs/reference/formulas.md` (Rounding vocabulary; Scaled balances; Rounding review table)
  - `common/src/rates/scaling.rs` (`calculate_scaled_*`, `unscale_*`, `resolve_withdrawal`, `resolve_repay`, `resolve_net_settle`, `calculate_scaled_cap`)
  - `common/src/rates/value.rs` (`position_value` / `_floor` / `_ceil`)
  - `common/src/rates/index.rs` / `simulate.rs` (accrual shortfall → revenue)
  - `common/src/math/fp.rs` (`Bps::apply_to*`, `flash_loan_fee_on`)
  - `contracts/pool/src/ops/{supply,borrow,withdraw,repay,net_settle,seize,recapitalize,revenue}.rs`
  - `contracts/pool/src/cache/{scale,shares}.rs`, `guards.rs`, `interest.rs`
  - `contracts/controller/src/risk/totals.rs`
  - `contracts/controller/src/positions/liquidation/{math,apply,curve}.rs`
  - `contracts/controller/src/spoke_usage.rs` (cap floor)
- Defense: Share mint/burn and risk valuation follow ADR-0003 directed rounding end-to-end. Pool rejects positive value that would change zero shares. Controller risk gates floor collateral / ceil debt / floor HF. Liquidation under-delivery floor-scales seizure; Credit fee ceils; Transfer fee floors with optional dust bump. Accrual shortfall and bad-debt write-downs are protocol/supplier-conservative. Controller books pool mutation outputs — it does not re-round share mints independently.
- Gap: No novel critical mis-direction found. Documented residuals only: (a) Transfer liq fee half-up→floor+bump vs Credit ceil (A053); (b) liquidation collateral-leg ≤2 units/leg liquidator loss + min-collateral floor (numeric-bounds §6); (c) half-up used for non-solvency readouts (views, seizure portfolio share, display unscale) — intentional; (d) `process_excess_payment` floor-refund can leave ≤1 unit of ideal-trim dust as repay (liquidator pays slightly more, victim loses slightly more collateral — bounded).
- Impact: A wrong rounding pair on mint/burn would mint free supply, erase unpaid debt, or overpay withdrawals. Current directed pairs close those classes. Residual bias is at most dust units per leg, mitigated by INV-ACCT-05 and min-collateral / bad-debt floors.
- Evidence: INV-ACCT-05, INV-RISK-02, INV-LIQ-03, INV-IDX-02; formulas.md rounding review table; ADR-0003; Certora `hf_division_rounds_against_borrower`, `position_value_ceil_ge_floor`, `supply_dust_amount_sanity`, `liquidation_does_not_increase_seized_collateral`; unit `common/tests/rates/scaling.rs`, `contracts/controller/tests/positions/liquidation_math.rs`; peers A049, A051–A054, A044.
- Opinion: Money-path rounding is a defended, policy-driven system rather than an ad-hoc mix. Paired readouts (mint floor vs burn ceil, debt ceil vs repay floor, gate floor/ceil vs view half-up) are consistent. Treat dual Transfer/Credit fee representations and the dust fee bump as accepted residuals already owned by A053 / numeric-bounds — not open free-share bugs.

---

## 1. Scope and method

Audit **whether every share↔asset and USD valuation conversion on money paths rounds in the protocol’s favour**, per ADR-0003 and `docs/reference/formulas.md`.

In scope:
1. Core converters in `common/src/rates/scaling.rs` and `value.rs`.
2. Pool mutators that mint/burn shares or move cash (`supply`, `borrow`, `withdraw`, `repay`, `net_settle`, `seize`, `recapitalize`, `claim_revenue`, accrual).
3. Controller risk totals used as solvency gates.
4. Liquidation plan/apply arithmetic (repay, seize, under-delivery, fees, excess trim).
5. Caps, flash fees, and related BPS products that touch cash.

Out of scope as primary claims (cross-linked): measured receipts (A041/A055), Transfer/Credit seize custody (A051/A052), fee skim product (A053), refunds (A054), flash pullback (A044).

Method: map each conversion site to `{floor|ceil|half-up}`, name the favoured party, and check for **paired readout** hazards (ADR-0003 auditor focus: a locally fine rule that is unsafe when paired with a different readout).

---

## 2. Verdict

**Defended.** ADR-0003’s policy is implemented as named helpers and consumed uniformly by pool ops and controller liquidation/risk code. No money path was found that mints free supply shares, under-records debt, under-burns on withdraw, over-burns on repay, or overstates gated collateral / understates gated debt.

Residual dust bias is explicit, tested, and bounded.

---

## 3. Policy (source of truth)

ADR-0003 / formulas.md:

| Operation | Share conversion | Direction | Safety effect |
|---|---|---|---|
| Supply mint | amount → shares / supply_index | **floor** | no over-credit |
| Partial withdraw burn | amount → shares / supply_index | **ceil** | no under-burn |
| Full withdraw | burn all shares; pay | **floor** asset | no overpay |
| Borrow mint | amount → shares / borrow_index | **ceil** | no under-record debt |
| Partial repay burn | amount → shares / borrow_index | **floor** | no erase unpaid debt |
| Full repay | burn all; excess refund | debt valued **ceil** | close only when paid |
| Net settle overlap | min(req, floor supply, ceil debt) | conservative | no half-up promotion to full close |
| Gated collateral | position_value_floor (+ LTV/LT floor) | **floor** | no overstate collateral |
| Risk debt | position_value_ceil | **ceil** | no understate debt |
| Health factor | weighted / debt | **floor** (saturating) | no overstate HF |
| Cap check | calculate_scaled_cap | **floor** (saturating) | slightly tighter |
| Positive Δ → 0 shares | — | **revert** | INV-ACCT-05 |

Half-up is reserved for: interest display/utilization/rate views, non-gated collateral totals (seizure share / dust test), user-facing unscale views, and some BPS products that then apply a dust floor (flash fee, Transfer liq fee bump).

---

## 4. Core converter inventory

### 4.1 `common/src/rates/scaling.rs`

| API | Direction | Used by |
|---|---|---|
| `calculate_scaled_supply` | floor | pool supply mint |
| `calculate_scaled_supply_ceil` | ceil | partial withdraw / net-settle supply burn |
| `calculate_scaled_borrow` | ceil | pool borrow mint |
| `calculate_scaled_borrow_floor` | floor | partial repay / net-settle debt burn |
| `unscale_supply` | half-up | views; full-withdraw threshold compare |
| `unscale_supply_floor` | floor | full withdraw payout; backing claims; revenue claim size |
| `unscale_borrow` | half-up | views only |
| `unscale_borrow_ceil` | ceil | full repay threshold; liq debt cap; backing debt |
| `resolve_withdrawal` | full→(all, floor pay); partial→(ceil burn, amount) | pool withdraw; liq plan `pool_gross` |
| `resolve_repay` | full→(all, excess); partial→(floor burn, 0) | pool repay |
| `resolve_net_settle` | overlap min; directed burns; full close only on conservative exhaust | pool net_settle / repay-with-collateral |
| `calculate_scaled_cap` | floor saturating | spoke usage cap (fail-open on overflow, not over-admit) |

**Paired readout checks (pass):**
- Supply: mint floor + withdraw ceil burn → cannot mint then withdraw more value than deposited at fixed index.
- Debt: mint ceil + repay floor burn → cannot mint then repay-erase more debt than paid.
- Full withdraw compares against **half-up** display value but **pays floor** — user must request ≥ half-up to close, receives ≤ exact; pool-conservative.
- Full repay requires payment ≥ **ceil** debt; burns all; refunds excess — no stranded dust debt from rounding alone when paid in full.
- Net settle refuses half-up promotion to full close (`resolve_net_settle` comments + formulas table). Confirmed in A049.

### 4.2 `common/src/rates/value.rs` + `risk/totals.rs`

| Consumer | Collateral | Debt | HF |
|---|---|---|---|
| `calculate_account_risk_totals` (gates) | half-up **total** (seizure/dust); **floor** gate_value for LTV & LT weights | **ceil** | **floor** saturating |
| `calculate_ltv_collateral_wad` | floor | — | — |
| `sum_debt_usd` | — | half-up | **view only** (`views.rs`) |

INV-RISK-02 holds on the gate path. View half-up debt is not a solvency input — correct separation.

### 4.3 Accrual / indexes (`index.rs`, `simulate.rs`, `interest.rs`)

- Borrow index: half-up compound, never decreases, capped.
- Supply index growth: **floor** (`mul_div_floor_saturating`); shortfall → `supply_index_reward_shortfall` → booked as protocol revenue with fee (`accrue_step` docs: residual favours protocol).
- `protocol_fee_shares`: **floor** (claim value ≤ fee Ray).
- Bad debt: `reduction_factor = div_floor`, `new_index = mul_floor`, clamp `SUPPLY_INDEX_FLOOR_RAW` (INV-IDX-02). Loss on suppliers, never invents claims.
- Seize borrow side: socializes **ceil** debt Ray then burns exact shares (`seize.rs`) — does not understate socialized loss.

### 4.4 BPS / fees (`fp.rs` Bps)

| Product | Direction | Protocol favour? |
|---|---|---|
| LTV/LT on gate collateral | `apply_to_wad_floor` | yes |
| Reserve factor / Transfer liq fee on Ray | `apply_to_ray` half-up | ~neutral; Transfer then floor+bump |
| Flash loan fee | half-up + bump to 1 if bps>0 | yes (A044) |
| Credit liq fee on shares | `mul_div_ceil` | yes (A052/A053) |
| Liquidation buffer reserve | half-up of floored supply | slightly larger buffer |

---

## 5. Pool money mutators (share/cash)

| Op | Rounding at book | Zero-share guard | Cash |
|---|---|---|---|
| `supply::apply` | floor mint | `SupplyRoundsToZeroShares` | credit measured amount |
| `borrow::mint_debt` | ceil mint | `BorrowRoundsToZeroShares` | debit amount |
| `withdraw::resolve_close_or_partial` | ceil burn / floor full pay | `WithdrawRoundsToZeroShares` | debit **net** (after fee) |
| `repay::accounting` | floor burn / full on ceil | `RepayRoundsToZeroShares` | credit **net**; refund overpay |
| `net_settle::apply` | directed pair | `NetSettleRoundsToZeroShares` | none |
| `recapitalize` | no shares; `min(amount, shortfall)` | n/a | credit applied only |
| `burn_claimable_revenue` | payout floor; partial burn **ceil** shares | InternalError if amount>0 & burn 0 | debit ≤ cash |
| Liq fee withhold | fee asset → Ray; revenue shares **floor** | — | retain fee cash |

Controller merge paths (`LegOutcome::from(PoolPositionMutation)`) adopt pool `scaled_amount` / `actual_amount` — **no second independent share rounding** on the hub (A082 class). Strategies have no local `calculate_scaled_*` (grep empty under `strategies/`).

---

## 6. Liquidation money path

### 6.1 Repay sizing (`calculate_repayment_amounts` / `normalize_repayment_plan`)

1. Cap each leg at `unscale_borrow_ceil` — cannot plan more than conservative debt.
2. Price payment with half-up `usd_value_wad` — liquidator credited for every asset unit including ceil dust (numeric-bounds §6.1: **0 debt-leg loss sites**).
3. Ideal-cap trim via `process_excess_payment`: floor ratio / floor token refund → ≤1 unit of excess may remain as repay (liquidator keeps less unpulled dust; more debt cleared). Protocol/victim-favouring at liquidator cost; not free collateral.
4. `FullCloseRequired` tolerance uses `sum_repaid_usd_ceil` so pure rounding shortfalls do not force full close incorrectly.

### 6.2 Seizure sizing (`calculate_seized_collateral`)

| Step | Direction | Favours |
|---|---|---|
| Portfolio share / bonus weights | half-up | neutral share-of-book |
| `base_ray = seizure / (1+bonus)` | **floor** | no invented bonus after clamp |
| Bonus = max(0, capped − base) | guarded sub | no negative |
| Partial `seized_scaled` | **floor**(ray/index) | no share inflation |
| Full close shares | verbatim position | no round-trip invent/strand |
| `bonus_scaled` | floor + min(seized) | split precondition |
| Partial asset amount | **floor** | liquidator ≤ plan |
| Full asset amount | half-up request; pool pays via `resolve_withdrawal` floor on full | pool payout still floor |
| Transfer fee | half-up Ray → floor asset → bump 1 → min(pool_gross) | protocol ≥ fair fee; dust bump known |
| Credit fee (apply) | **ceil** on bonus shares | protocol (A052) |

### 6.3 Under-delivery (`apply_liquidation_repayments` + `scale_seizures_to_received`)

- Leg USD: `mul_div_floor(planned_usd, received, planned_amount)` on shortfall.
- All seizure fields floor-scaled by `received_usd/planned_usd`.
- Never inflates seize vs measured repay (INV-LIQ-03). Credit re-splits after scale so conservation stays exact (A052/A053).

### 6.4 Curve / HF after seize

`curve.rs` uses floor saturating HF and ceil on effective threshold BPS clamp — conservative for “does this close improve HF” math.

---

## 7. Paired-readout hazard scan (ADR-0003 focus)

| Pair | Risk if mismatched | Actual pairing | Result |
|---|---|---|---|
| Supply mint vs withdraw burn | free yield cycle | floor mint / ceil burn | OK |
| Borrow mint vs repay burn | debt erasure | ceil mint / floor burn | OK |
| Full withdraw threshold vs payout | overpay on close | half-up threshold / floor pay | OK |
| Full repay threshold vs burn | leftover dust debt | ceil threshold / burn all | OK |
| Gate valuation vs view | false liquidations / false healthy | gates floor/ceil; views half-up | OK |
| Liq plan shares vs Transfer assets | over-seize tokens | plan floor assets; pool resolve_withdrawal | OK |
| Liq plan shares vs Credit move | invent shares | verbatim / floor; absorb not mint | OK (A052) |
| Fee Transfer vs Credit | double skim / unbacked | single mode per call; different reps | OK intentional (A053) |
| Accrual reward vs index | silent drop | shortfall → revenue | OK |
| Cap scaled vs usage | over-admit | floor saturating cap | OK (slightly tight) |
| Revenue claim burn vs payout | overpay treasury | floor payout / ceil share burn | OK |

No unpaired critical mismatch found.

---

## 8. Gaps and residuals (non-critical)

| ID | Residual | Severity | Owner |
|---|---|---|---|
| R1 | Transfer fee half-up then floor+bump vs Credit `mul_div_ceil` — not bit-identical | low / accepted | A053 |
| R2 | Dust fee bump can charge 1 unit when fair fee floors to 0 | low; V* ≪ $5 floor | numeric-bounds §6; A053 |
| R3 | Partial seizure floor + bump ⇒ ≤2 collateral units/leg liquidator loss | info | numeric-bounds §6 |
| R4 | Ideal-excess floor-refund leaves ≤1 unit repay dust | info | this file §6.1 |
| R5 | Half-up on non-gated totals / views | none if not used as gate | formulas.md |
| R6 | Index drift Transfer plan→apply | liveness / size change | A026/A094 — not rounding direction |

None reverse the ADR-0003 bias into a free-share or unpaid-debt-erasure bug.

---

## 9. What would constitute a real finding (not observed)

- Supply mint using ceil, or withdraw burn using floor, without compensating readout.
- Borrow mint floor or repay burn ceil on the money path.
- Risk gates using half-up collateral or floor debt.
- Credit fee using floor/half-up without conservation absorb.
- Under-delivery using ceil scale (inflating seize).
- Controller re-deriving minted shares with a looser rule than the pool.
- Positive amount accepted with zero share delta (INV-ACCT-05 bypass).

None of the above appear on production money paths reviewed.

---

## 10. Evidence map

| Claim | Evidence |
|---|---|
| Policy | ADR-0003; formulas.md Rounding review table |
| INV-ACCT-05 zero-share | pool ops `*RoundsToZeroShares`; errors.md 47–52 |
| INV-RISK-02 | `risk/totals.rs`; Certora `hf_division_rounds_against_borrower`, `position_value_ceil_ge_floor` |
| INV-LIQ-03 | `scale_seizures_to_received` floor; apply measured receipt |
| INV-IDX-02 | `apply_bad_debt_to_supply_index` two floors + floor clamp |
| Scaling unit tests | `common/tests/rates/scaling.rs` (incl. `resolve_net_settle_partial_keeps_directed_rounding`) |
| Liq rounding economics | `contracts/controller/tests/positions/liquidation_math.rs` (§6 profitability pins) |
| Peer agreement | A049 net settle; A051 Transfer; A052 Credit ceil; A053 fee map; A054 repay ceil/floor; A044 flash bump |

---

## 11. Opinion

Rounding-on-money is one of the stronger defended surfaces in this audit wave: the vocabulary is centralized in `common`, the pool is the sole share minter/burner, and the controller’s dangerous arithmetic (liquidation + risk gates) reuses the same helpers with explicit floor/ceil choices. Half-up appears where policy allows (views, portfolio share, BPS with dust bump), not where it would loosen solvency or share conservation.

No novel critical gap for A059. Synthesis should treat rounding direction as **defended**, with residuals deferred to A053 and numeric-bounds §6 rather than open T8 money bugs.
