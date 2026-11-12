# Controller seizure and post-liquidation health (Aeneas + Lean)

Machine-checked Lean 4 proofs about the seizure side of the liquidation plan
in `contracts/controller/src/positions/liquidation/math.rs`
(`calculate_seized_collateral`, `get_account_bonus_params`,
`calculate_seizure_proportions`) and about the health factor of the account
after the executed plan. Every theorem about code is about the generated
`lean/ControllerAeneas/Funs.lean`, library `ControllerAeneas`. Extraction,
trusted base (Map/Context/Vec models) and the theorems these build on:
[controller.md](controller.md), [controller-estimate.md](controller-estimate.md).
Conventions: [../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

| Rust function | Status |
|---|---|
| `calculate_seized_collateral` (loop body, loop, function) | exact closed form of every quantity, never aborts on the production domain, per-entry bounds, total seized value ≤ budget + stated rounding per leg |
| `get_account_bonus_params` (and its loop) | exact value, `0 ≤ base ≤ max`, base is a weighted average of the stamped bonuses |
| `calculate_seizure_proportions` | exact value, `0 ≤ p ≤ W`, output satisfies the bonus fields of the estimate's `EstDomain` |
| `rates::scaling::resolve_withdrawal` | value form re-proved on this extraction (`sz_resolve_withdrawal_val`) |
| post-liquidation health factor | **headline**: post-HF ≥ pre-HF − explicit rounding at the HF-preserving bonus; post-HF ≥ curve target − explicit rounding on the target branch; counterexample that the rounding term is needed |

No extraction was re-run; all functions were already in `Funs.lean`.

Files (all plain, non-`module` Lean files under `lean/ControllerAeneas/Proofs/`):

| File | Contents |
|---|---|
| `SeizeBasic.lean` | value-form specs of the primitives the loop calls (`Ray::mul`, `Wad::mul/div/div_floor`, `Ray::div_floor`, `to_asset{,_floor,_ceil}`, `from_asset`, `to_wad`, `saturating_mul`, `Bps::apply_to_ray`, checked add/sub, `Ray` `≤`/`min`/`==`), `sz_resolve_withdrawal_val`; `sz_rescale_half_up_down_spec`, `sz_to_asset_val`, `sz_from_asset_val` are copies of `RatesAeneas/Proofs/{HalfUpWrappers,Scaling}.lean` (header says so) |
| `SeizeLeg.lean` | `SzLeg` (integer inputs of one leg), the closed forms `A, V, Sh, L, Q, S0, S1, A1, Uns, Capped, Base, Bonus, FeeR, Seized, Bsc, Amt, PG, FeeA, Bumped, Real, PFee, Emit`, the domain `SzLegOK`, and the per-leg properties |
| `SeizeBody.lean` | `sz_body_spec` (one loop iteration), `legOf`, `EntryOf` |
| `SeizeLoop.lean` | `SzOut`, `sz_loop_spec`, `calculate_seized_collateral_spec`, `..._no_collateral` |
| `SeizeBudget.lean` | `seized_total_le_budget`, `calculate_seized_collateral_entries` |
| `SeizeBonus.lean` | `get_account_bonus_params_spec`, `calculate_seizure_proportions_spec`, `weighted_bonus_between` |
| `HealthAfterLegs.lean` | distance of `fv`/`huv`/`cv` from the exact value, per-leg collateral loss, per-leg debt reduction (`DebtStep`) |
| `HealthAfterCombine.lean` | the rational-arithmetic combination lemmas |
| `HealthAfter.lean` | `health_after_hf_preserving`, `health_after_target`, `ha_counterexample` |

Build: `lake build ControllerAeneas.Proofs.HealthAfter` (imports all the others).

## Notation

`R = 10^27`, `W = 10^18`, `BPS = 10^4`, `K = R·10^9·W = 10^54` (the scale of
an exact value `s·i·p`). `huD n d` = `n/d` rounded half up, `n / d` = floor,
`clD n d` = ceiling (`Totals.lean`; specifications are stated with these
closed forms, which `isHalfUp_eq`/`isFloor_eq`/`isCeil_eq` tie to the
division-free `IsHalfUp`/`IsFloor`/`IsCeil`). `fv`, `huv`, `cv`: the floor,
half-up and ceiling valuation chains of `position_value*`.

For one supply leg (`SzLeg`): scaled shares `s`, supply index `i`, price `p`,
decimals `dec`, fee rate `f`; plan: total collateral `C`, budget `B`,
`opb = W + bonus·10^14`, flags `rad` (`repays_all_debt`), `sa`
(`seize_all`). `Dd = 10^(27−dec)`.

| Closed form | Code |
|---|---|
| `A = huD(s·i, R)` | `actual_ray` |
| `V = huv(s, i, p)` | `asset_value` |
| `Sh = huD(V·W, C)` | `share` |
| `L = huD(B·Sh, W)` | `seizure_for_asset_usd` (the leg's budget) |
| `Q = huD(L·W, p)` | `seizure_amount_wad` |
| `S0 = min(Q·10^9, i128::MAX)` | `seizure_ray` (saturating upscale) |
| `WU = ¬sa ∧ dec < 3 ∧ S0 < A` | whole-unit branch |
| `S1` = `min(clD(S0, Dd)·Dd, A)` (WU, rad), `⌊S0/Dd⌋·Dd` (WU, ¬rad), else `S0` | `seizure_ray` after it |
| `A1` = `huD(S1, 10^9)` (WU), else `Q` | `seizure_amount_wad` after it |
| `Uns` = `L − huD(A1·p, W)` if positive (WU, ¬rad), else 0 | contribution to `unseized_usd` |
| `Capped = sa ? A : min(S1, A)` | `capped_ray` |
| `Base = min(⌊A1·W/opb⌋·10^9, i128::MAX)` | `base_ray` |
| `Bonus = Capped − Base` if positive, else 0 | `bonus_ray` |
| `Seized = (Capped = A) ? s : ⌊Capped·R/i⌋` | `seized_scaled` |
| `Bsc = min(⌊Bonus·R/i⌋, Seized)` | `bonus_scaled` |
| `Amt = (Capped = A) ? huD(Capped, Dd) : ⌊Capped/Dd⌋` | `capped_amount` |
| `PG` = `⌊⌊s·i/R⌋/Dd⌋` if `huD(A, Dd) ≤ Amt`, else `Amt` | `pool_gross` |
| `PFee = min(Bumped, Real)` with `Bumped` = 1 if `huD(Bonus·f, BPS) > 0` and its floor in tokens is 0, `Real = ⌊(PG·Dd − Base)/Dd⌋⁺` | `protocol_fee` |
| `Emit = 0 < S1 ∧ 0 < Capped ∧ 0 < Seized ∧ 0 < Amt` | no `continue` fires |

**SzLegOK** (production domain of one leg):
`0 ≤ s ≤ 2^126`, `s·i ≤ 2^126·R`, `VD s i p`; `0 < i`; `0 < p`; `dec ≤ 18`;
`0 ≤ f ≤ BPS`; `0 < C`, `V ≤ C`; `0 ≤ B`, `B·W ≤ 2^126·p`; `W ≤ opb`.

## Theorems

| Lean name | File | Rust function | Claim | Hypotheses (why they hold) | Invariants | Certora |
|---|---|---|---|---|---|---|
| `sz_body_spec` | `SeizeBody` | `calculate_seized_collateral` loop body | one iteration returns `continue` with the context unchanged, `unseized_usd += Uns`, and the list extended by one entry `EntryOf` (amount `Amt`, fee `PFee`, shares `Seized`, bonus shares `Bsc`, fee rate `f`, the context's price/decimals/index) exactly when `Emit` | leg served by the context, `SzLegOK`, running `unseized_usd + Uns < 2^127` | INV-LIQ-02 | — |
| `sz_loop_spec` | `SeizeLoop` | the generated loop | over typed legs `L`: returns, context unchanged, output `= input ++ out` with `SzOut` (legs in order, one entry per emitting leg), `unseized = Σ Uns` | every leg as above, `Σ Uns` fits | INV-LIQ-02 | — |
| `calculate_seized_collateral_spec` | `SeizeLoop` | `calculate_seized_collateral` | for `C > 0`: never aborts; context unchanged; seizures are exactly `SzOut` over the account's supply map with the plan's legs (`planLeg`: budget `B = huD(repay·opb, W)`, `opb = W + bonus·10^14`); unbacked USD `= ⌊(Σ Uns)·W/opb⌋` | `0 ≤ repay`, `0 ≤ bonus ≤ 10^8`, `B < 2^127`, every leg served and `SzLegOK`, `Σ Uns < 2^127` | INV-LIQ-02 | — |
| `calculate_seized_collateral_no_collateral` | `SeizeLoop` | same | `C ≤ 0` → `([], 0)`, context unchanged, never aborts | none | INV-LIQ-02 | — |
| `calculate_seized_collateral_entries` | `SeizeBudget` | same | every entry comes from a supply entry with the same key and has `0 < scaled_amount ≤` held shares, `0 ≤ bonus_scaled ≤ scaled_amount`, `0 < amount ≤ huD(huD(s·i, R), Dd)` (the half-up held amount), `0 ≤ protocol_fee ≤ amount` | as `_spec` | INV-LIQ-02, INV-ACCT-01 | `liquidation_does_not_increase_seized_collateral` (account level, bounded) |
| `seized_total_le_budget` | `SeizeBudget` | same (cross-leg) | `2W·Σ SeizedVal ≤ K·(2W·B + n·(B + W)) + W·Σ Slack`, `SeizedVal = Seized·i·p` of an emitting leg, `Slack = R·p·(10^9 + 1 + 2X)`, `X = Dd − 1` on the whole-unit round-up branch else 0 | legs `SzLegOK` with common `C`, `B`, not seize-all; `Σ V ≤ C` (production `C = Σ huv`); `B ≥ 0` | INV-LIQ-02 | — |
| `SzLegOK.seized_le`, `.bsc_le`, `.fee_le`, `.amt_le` | `SeizeLeg` | one leg | `0 < Seized ≤ s`; `0 ≤ Bsc ≤ Seized`; `0 < Amt`, `0 ≤ PFee ≤ Amt`; `Amt ≤ huD(A, Dd)` | `SzLegOK`, `Emit` | INV-LIQ-02 | — |
| `SzLegOK.seized_val`, `.capped_le`, `.S1_le_S0` | `SeizeLeg` | one leg | `2·Seized·i ≤ R·(2·Capped + 1)`; `¬sa → Capped ≤ S1`; `S1 ≤ S0 + X` (`X` as above) | `SzLegOK` | INV-LIQ-02 | — |
| `SzLegOK.seized_value_le_budget`, `.seizedVal_le` | `SeizeLeg`, `SeizeBudget` | one leg | `2·Seized·i·p ≤ 2K·L + Slack` (the leg seizes its budget plus half a token-wei at its price, half a RAY unit, and on the round-up branch one whole token) | `SzLegOK`, not seize-all | INV-LIQ-02 | — |
| `SzLegOK.L_share` | `SeizeBudget` | one leg | `4WC·L ≤ 4BW·V + 2C(B + W)` (leg budget = pro-rata share of `B` up to rounding) | `SzLegOK` | INV-LIQ-02 | — |
| `sz_resolve_withdrawal_val` | `SeizeBasic` | `rates/scaling.rs::resolve_withdrawal` | returns `paid = ⌊⌊pos·i/R⌋/Dd⌋` if `huD(huD(pos·i,R),Dd) ≤ amount`, else `amount`; `0 ≤ paid ≤ amount`, `paid·Dd ≤ ⌊pos·i/R⌋` | `dec < 27`, `0 < i`, `0 ≤ pos`, `0 ≤ amount`, `huD(pos·i, R)` fits (the exact abort condition) | INV-ACCT-01 | — |
| `get_account_bonus_params_spec` | `SeizeBonus` | `get_account_bonus_params` | returns, context unchanged; `max = max_bonus_for_threshold(p)`; `C = 0` → `base = 0`; `C > 0` → `base = min(huD(Σ huv·bonus, C), max)` (one rounding, no per-leg accumulation) | `C ≥ 0`, legs served and `VD`, `Σ huv·bonus < 2^127` (else `MathOverflow`) | INV-LIQ-02 | `bonus_bounded` (bounded) |
| `bonus_loop_spec` | `SeizeBonus` | its loop | exact accumulation `Σ huv·bonus`, context unchanged | as above | INV-LIQ-02 | — |
| `weighted_bonus_between` | `SeizeBonus` | (math) | with `C = Σ huv > 0` and every stamped bonus in `[lo, hi]`: `lo ≤ huD(Σ huv·bonus, C) ≤ hi` | as stated | INV-LIQ-02 | — |
| `calculate_seizure_proportions_spec` | `SeizeBonus` | `calculate_seizure_proportions` | returns `p = huD(Wc·W, C)` (`C > 0`) or 0, the bounds of `get_account_bonus_params` at `p`; `0 ≤ p ≤ W`; `0 ≤ base ≤ max ≤ BPS(BPS−1)`, `(BPS + max)·p ≤ BPS·W`: the bonus fields of `EstDomain` | `0 ≤ Wc ≤ C` (`calculate_account_risk_totals_conservative`), legs served and `VD`, weighted sum fits | INV-LIQ-02 | `derived_bonus_respects_threshold` |
| `ha_fv_le`, `ha_fv_ge`, `ha_cv_ge`, `ha_cv_le`, `ha_huv_le` | `HealthAfterLegs` | `position_value*` closed forms | `fv·K ≤ s·i·p < fv·K + K + 2·10^36·p`; `s·i·p ≤ cv·K < s·i·p + K + 2·10^36·p`; `huv·K < s·i·p + K + 2·10^36·p` | `s·i ≥ 0`, `p ≥ 0` | INV-RISK-02 | `position_value_ceil_ge_floor` |
| `ha_coll_leg` | `HealthAfterLegs` | weighted-collateral term | removing `0 ≤ σ ≤ s` shares lowers `⌊fv·LT/BPS⌋` by at most `(LT·(σ·i·p + K + 2·10^36·p) + BPS·K)/(BPS·K)` | as stated, `LT ≥ 0` | INV-RISK-02 | — |
| `ha_huv_weighted` | `HealthAfterLegs` | same | `K·LT·huv ≤ K·BPS·(⌊fv·LT/BPS⌋ + 1) + LT·(2K + 4·10^36·p)` | as stated | INV-RISK-02 | — |
| `ha_debt_leg` | `HealthAfterLegs` | debt term + `resolve_repay` | for a `DebtStep` (untouched / partial floor burn with credited USD `≤ huD(a·10^(18−dec)·p, W)` / full): `0 ≤ t' ≤ t` and `2K·(cv(t) − cv(t')) ≥ 2K·u − 3K − 6·10^36·p` | `t ≥ 0`, `0 < ib ≤ 10^36`, `p ≥ 0`, `dec ≤ 18` | INV-LIQ-02, INV-RISK-02 | `liquidation_does_not_increase_repaid_debt` |
| `ha_combine`, `ha_coll_bound`, `ha_combine_target` | `HealthAfterCombine` | (math) | the combination steps, see below | as stated | — | — |
| **`health_after_hf_preserving`** | `HealthAfter` | `calculate_account_risk_totals` before/after the plan | `Wc'·D ≥ Wc·D' − Wc·Ed/(2K) − D·ErrC` | see "The post-liquidation health factor" | INV-LIQ-02, INV-RISK-02 | `split_liq_partial_at_the_cap_never_lowers_collateral_coverage` (same idea for `C/D`, with a `repay/10^10 + 2` allowance) |
| **`health_after_target`** | `HealthAfter` | same | `W·Wc' ≥ T·D' − (T·Ed/(2K) + (T − δ + W)/2 + rep/2 + W·ErrC)` | as above with `HitsTarget` and `r ≤ rep` instead of the cap | INV-LIQ-02 | `ideal_repayment_targets_curve_hf` (intent; the rule itself never reaches the target branch, see controller-estimate.md) |
| `sum_regroup`, `lsum_ite_key` | `HealthAfterPlan` | (math) | summing over items whose keys are keys of a map with distinct keys equals summing, per map entry, the items with that key | `(map keys).Nodup`, every item's key is a map key | — | — |
| `plan_credits_sum` | `HealthAfterPlan` | `normalize_repayment_plan` | for the returned plan, `0 ≤ repay_usd` and `repay_usd = Σ_debt planU(k)`, where `planU(k)` is the `usd_wad` of the repaid entries for key `k`: discharges `hr0`/`hrep` of the health theorems | hypotheses of `repayment_plan_spec` (controller-repayment.md), debt keys distinct (Soroban `Map`), every repaid entry names a debt leg | INV-LIQ-02 | — |
| **`health_after_executed_plan`** | `HealthAfterPlan` | `normalize_repayment_plan` + `calculate_account_risk_totals` | `health_after_hf_preserving` for the production plan: each debt leg repaid `planA(k)` tokens and credited `planU(k)` USD | as `plan_credits_sum`, plus the totals domain of both accounts, `SzLegOK` per supply leg, `SupplyAfter`/`DebtAfter` (application, modelled), not seize-all, the HF-preserving cap | INV-LIQ-02, INV-RISK-02 | — |
| `ha_counterexample` | `HealthAfter` | same | concrete account where the cap is tight and the executed plan lowers the production HF by one unit (below) | none | INV-LIQ-02 | — |

Supporting lemmas (value forms of primitives `sz_*`, `SzLegOK.*` bounds such
as `AB`, `ShW`, `LB`, `QB`, `S1B`, `A1B`, `wusd_fit`, `part_le`, `div_fit`,
`F_le_H`, `PG_le`, `Uns0`, `UnsL`; `SzOut.mem`, `SzOut.transport`,
`legOf_typed`, list-sum lemmas `sz_lsum_*`, `ha_lsum_*`, `ha_forall2_*`)
carry doc comments in the files.

### Why the hypotheses hold in production

* **Leg domain `SzLegOK`.** Shares, indexes, prices are non-negative `Ray`/`Wad`
  values; supply index ≥ its floor (`0 < i`); oracle prices are positive
  (`0 < p`); `MAX_ASSET_DECIMALS = 18`; stamped fee rates are validated below
  100%. `s ≤ 2^126` and `s·i ≤ 2^126·R` ask that held shares and their RAY
  value fit `i128` with a factor-2 margin (about `8.5·10^10` tokens, the same
  order as the `Ray::mul` limit every valuation already has). `VD` is the
  totals' value domain. `C` is `calculate_account_risk_totals`' half-up
  total over the same legs, so `V ≤ C`. `W ≤ opb` since the bonus is
  non-negative.
* **`B·W ≤ 2^126·p` (budget in tokens fits).** This is the only hypothesis
  that is not a plain type/validation fact: `Wad::div(seizure_for_asset_usd, price)`
  aborts exactly when `huD(L·W, p) ≥ 2^127` (`mul_div_half_up_returns_iff`),
  i.e. when the leg's request exceeds about `1.7·10^20` tokens. The request is
  about `(B/C)·held`, and every branch of the estimate keeps `B ≲ C`
  (insolvent: `repay ≤ C/(1 + base)`; solvent: the cap `(1 + b)·p ≤ hf`
  gives `(1 + b)·D ≤ C`; whole-unit repayment raises the quote to one unit of
  a leg that holds at least one unit). So the request is at most the held
  amount plus rounding, far below the limit. The hypothesis is stated rather
  than derived because the plan-level `B ≲ C` goes through
  `normalize_repayment_plan` (proved by another work stream).
* **`Σ Uns < 2^127`.** Each `Uns ≤ L ≤ B` (`UnsL`, `LB`).
* **Bonus `≤ 10^8`.** `max ≤ BPS·(BPS−1)` (`max_bonus_for_threshold_bound`) and
  the plan's bonus is at most `max`.
* **Post-liquidation model.** `apply_liquidation_share_credit` subtracts
  exactly `entry.scaled_amount` (= `Seized`) from the supply position
  (credit mode). The pool's repay burns `resolve_repay`'s
  `⌊a·10^(27−dec)·R/ib⌋` shares on a partial repayment and the whole
  position on a full one (`RatesAeneas` `resolve_repay_spec`). The credited
  USD of a leg is `usd_value_wad(a)` (`calculate_repayment_amounts_spec`) or
  less after trimming (`process_excess_payment` takes `min(…, usd)`), and the
  plan's `repay_usd = Σ usd_wad` (`LiquidationPlan::validate`,
  `sum_repaid_usd_spec`); one repayment per debt key (payments are merged by
  `aggregate_positive_payments`). The borrow index lies in `[RAY, 10^36]`.

## The post-liquidation health factor, in plain words

Let `C`, `Wc`, `D` be the production totals before the liquidation
(half-up collateral, threshold-weighted floor collateral, ceiling debt),
`hf = ⌊Wc·W/D⌋`, `p = huD(Wc·W, C)` the seizure proportion, and `Wc'`, `D'`
the production totals after the plan (both from
`calculate_account_risk_totals_spec`).

**At the HF-preserving bonus** (`(BPS + b)·p ≤ hf·BPS`, which
`estimate_liquidation_amount_contract` guarantees on a solvent liquidatable
account unless it closes the whole debt at bonus 0):

`Wc'/D' ≥ Wc/D − (Wc·Ed/(2K) + D·ErrC)/(D·D')`.

In exact arithmetic (`hf_preserved_of_rate`) the health factor never
decreases. The code rounds at about fifteen places, and the allowance
accounts for each of them, all in raw WAD units (`10^-18` USD) of weighted
collateral:

* debt side, `Ed/(2K) = Σ_debt (1.5 + 3·p_j/W)`: the floor burn and the
  ceiling valuation, at most 1.5 units plus 3 token-wei at the leg's price per
  debt leg;
* collateral side, `ErrC` = `rep·opb/(2W²) + 1/2` (half-up budget and
  proportion) `+ B·(n + Ev/(K·BPS))/C` (half-up vs floor valuation of each leg,
  about `(2 + 4·p_i/W)` units per leg scaled by `B/C ≲ 1`)
  `+ (B + W)·Σ LT/(2W·BPS)` (half-up share and leg budget: half a unit per leg
  plus a `10^-18` relative share of the budget) `+ (Es/2 + Ec)/(BPS·K)`
  (the per-leg seizure slack: half a token-wei and half a RAY unit, a whole
  token on the whole-unit round-up branch, plus the floor of the post value)
  `+ n` (the weighting floor, one unit per supply leg).

For a realistic account all of this is below `10^-15` USD per leg, except the
whole-unit round-up branch (`repays_all_debt` on a leg below 3 decimals),
where the allowance includes one whole token per such leg: the code then
deliberately seizes up to one whole unit more, and the debt is fully repaid
(post-HF is `i128::MAX`), so the bound is irrelevant there.

**On the curve-target branch** (`HitsTarget D Wc p T b r` from
`estimate_hits_curve_target`, and the plan repays at least the quote,
`r ≤ rep`):

`W·Wc' ≥ T·D' − (T·Ed/(2K) + (T − δ + W)/2 + rep/2 + W·ErrC)`,

i.e. the post-liquidation health factor is at least the target `T/W` up to:
the estimate's own target slack (`≤ 5.5` units), half a unit per repaid unit
for using the rounded seizure rate `δ = huD(p·opb, W)` instead of the exact
`p·opb/W` (a `5·10^-19` relative error on the repayment), and the same
rounding as above. Repaying more than the quote only helps, since `δ < T`.
Only the lower bound is proved (no overshoot bound).

**Counterexample: the allowance is needed** (`ha_counterexample`). One
supply leg, USD 1100 at threshold 50% (18 decimals, price 1, index `RAY`),
one debt leg USD 1000 (same), so `C = 1100 W`, `Wc = 550 W`, `D = 1000 W`,
`hf = 0.55 W`, `p = 0.5 W`, and the cap is exactly 1000 BPS with no slack:
`(BPS + 1000)·p = hf·BPS`. Repaying 1001 raw units at that bonus: budget
`huD(1001·1.1) = 1101`, seized shares `1101·10^9`, debt shares burned
`1001·10^9`. Then `Wc' = 550 W − 551` and `D' = 1000 W − 1001`, so
`Wc'·D < Wc·D'`, and the production HF goes from `550000000000000000` to
`549999999999999999`. The half-up budget seizes 1101 instead of the exact
1100.1, and the weighting floor loses the remaining half unit. The drop is
one `10^-18` HF unit, within the allowance, and is not exploitable (the
liquidator pays at least the value it takes, up to that unit), but
"post-HF ≥ pre-HF" holds only up to rounding.

## Findings

No production bug. Observations:

1. **Post-HF can fall by rounding at the HF-preserving cap** (counterexample
   above, one `10^-18` unit). The intended property holds with the explicit
   allowance of `health_after_hf_preserving`.
2. **Seizure rounding is half up at four steps** (`Wad::mul` for the budget,
   `Wad::div` for the share, `Wad::mul` for the leg budget, `Wad::div` by the
   price), so a leg can seize more than its exact pro-rata budget: by at most
   half a token-wei at its price and half a RAY unit
   (`seized_value_le_budget`), and the total by at most `n·(B + W)/(2W)`
   units above `B` (`seized_total_le_budget`). The whole-unit round-up branch
   seizes up to one whole token more, as documented in INV-LIQ-02.
3. **The `Wad::div` by the price can abort** when the requested amount
   exceeds about `1.7·10^20` tokens. The code's comment only mentions the
   saturating RAY upscale. The production plan keeps the request near the held
   amount (see the hypotheses), so this is not reachable from
   `build_liquidation_plan`. It is the only non-type hypothesis of the
   non-abort theorem.
4. **Every emitted seizure entry passes the seizure checks of
   `LiquidationPlan::validate`** (`calculate_seized_collateral_entries`),
   except `liquidation_fees < BPS`, which depends on the stamped rate (an
   input).

## Not covered

* **Exact abort set of the loop.** Non-abort is proved on `SzLegOK`; the
  hypotheses are sufficient, not necessary (`B·W ≤ 2^126·p` is the sufficient
  form of the abort condition `huD(L·W, p) < 2^127`).
* **Plan-level derivation of `B ≲ C` and of the post-account domain.**
  `rep = Σ u_j` and `0 ≤ rep` are derived from the production plan
  (`plan_credits_sum`, used by `health_after_executed_plan`). Still taken as
  hypotheses: that every supply leg is in `SzLegOK` (which needs the budget
  near the collateral, `B ≲ C`) and that the post account is in the totals
  domain (follows from `s' ≤ s`, `t' ≤ t`; `ha_debt_leg` proves
  `0 ≤ t' ≤ t`, not assembled).
* **`health_after_target` for the production plan** is not restated the way
  `health_after_executed_plan` restates the cap theorem; the same
  `plan_credits_sum` discharges its plan hypotheses.
* **Transfer-mode seizure.** The post-account model is credit mode
  (`scaled_amount` subtracted exactly). Transfer mode burns
  `⌈amount·Dd·R/i⌉ ≤ Seized + 1` shares via the pool's withdrawal; the extra
  share is not modelled.
* **The full-close-at-low-cap branch with `cap < 0`** (bonus 0, HF not
  preserved, finding 4 of controller-estimate.md) and **seize-all**
  (insolvent, all collateral taken) are outside the health theorems.
* **Upper bound on the target branch** (post-HF does not overshoot the target
  by more than rounding): would need lower bounds on every seizure rounding
  and the held-balance cap; not attempted.
* **`apply_liquidation_seizures`, `apply_liquidation_share_credit`, the pool
  repay**: their effects are modelled by `SupplyAfter`/`DebtAfter`, not
  proved from the (external) calls.
* **`repays_every_debt_leg`** is opaque (see controller.md); every theorem
  here holds for both values of `repays_all_debt`.

## Axioms

`#print axioms` was run on all 137 theorems of the ten files (the nine above
plus `HealthAfterPlan`), from a generated file outside the Lean library. Each depends only on `propext`,
`Classical.choice` and `Quot.sound` (or a subset). None uses `sorry`,
`admit` or a new `axiom`.
