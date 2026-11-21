# Controller seizure and post-liquidation health (Aeneas + Lean)

Lean 4 proofs about the seizure side of the liquidation plan
(`contracts/controller/src/positions/liquidation/math.rs`) and the account's
health factor after the executed plan, against the generated
`lean/ControllerAeneas/Funs.lean`. They build on
[controller.md](controller.md) and
[controller-estimate.md](controller-estimate.md); models:
[extraction.md](extraction.md#controller-trusted-base).

## Scope

| Rust function | Status |
|---|---|
| `calculate_seized_collateral` (loop body, loop, function) | exact closed forms, never aborts on the production domain, per-entry bounds, total ≤ budget + rounding |
| `get_account_bonus_params` (and its loop) | exact value, `0 ≤ base ≤ max`, base a weighted average of stamped bonuses |
| `calculate_seizure_proportions` | exact value, `0 ≤ p ≤ W`, meets the bonus fields of the estimate's `EstDomain` |
| `rates::scaling::resolve_withdrawal` | value form re-proved on this extraction (`sz_resolve_withdrawal_val`) |
| `build_liquidation_plan` (`plan.rs`) | every returned plan seizes at most the held shares, per supply position, with no domain hypothesis ([details](#plan-builder-seizure-within-the-held-shares)) |
| post-liquidation health factor | **headline**: post-HF ≥ pre-HF and ≥ curve target, each up to explicit rounding ([details](#the-post-liquidation-health-factor-in-plain-words)) |

Files (plain, non-`module` Lean under `lean/ControllerAeneas/Proofs/`;
`lake build ControllerAeneas.Proofs.HealthAfterPlan ControllerAeneas.Proofs.Plan`
builds all):

| File | Contents |
|---|---|
| `SeizeBasic.lean` | value-form specs of the loop's primitives (`Ray::mul`, `Wad::mul/div/div_floor`, `Ray::div_floor`, `to_asset{,_floor,_ceil}`, `from_asset`, `to_wad`, `saturating_mul`, `Bps::apply_to_ray`, checked add/sub, `Ray` `≤`/`min`/`==`), `sz_resolve_withdrawal_val`; `sz_rescale_half_up_down_spec`, `sz_to_asset_val`, `sz_from_asset_val` copied from `RatesAeneas/Proofs/{HalfUpWrappers,Scaling}.lean` |
| `SeizeLeg.lean` | `SzLeg` (one leg's integer inputs), the [closed forms](#notation) (`A, V, Sh, L, Q, S0, S1, A1, Uns, Capped, Base, Bonus, FeeR, Seized, Bsc, Amt, PG, FeeA, Bumped, Real, PFee, Emit`), `SzLegOK`, per-leg properties |
| `SeizeBody.lean` | `sz_body_spec`, `legOf`, `EntryOf` |
| `SeizeLoop.lean` | `SzOut`, `sz_loop_spec`, `calculate_seized_collateral_spec`, `..._no_collateral` |
| `SeizeBudget.lean` | `seized_total_le_budget`, `calculate_seized_collateral_entries` |
| `SeizeBonus.lean` | `get_account_bonus_params_spec`, `calculate_seizure_proportions_spec`, `weighted_bonus_between` |
| `HealthAfterLegs.lean` | distance of `fv`/`huv`/`cv` from the exact value, per-leg collateral loss, per-leg debt reduction (`DebtStep`) |
| `HealthAfterCombine.lean` | rational-arithmetic combination lemmas |
| `HealthAfter.lean` | `health_after_hf_preserving`, `health_after_target`, `ha_counterexample` |
| `HealthAfterPlan.lean` | `plan_credits_sum`, `health_after_executed_plan` |
| `Plan.lean` | `Rets` (partial correctness), `SeizedFrom`, `calculate_seized_collateral_seized_le_held`, `build_liquidation_plan_seized_computed`, `build_liquidation_plan_seized_le_held`, `seized_le_held_nonvacuous` |

## Notation

`R = 10^27`, `W = 10^18`, `BPS = 10^4`, `K = R·10^9·W = 10^54` (scale of an
exact value `s·i·p`). `huD n d` = `n/d` half up, `n / d` = floor, `clD n d` =
ceiling (`Totals.lean`); `isHalfUp_eq`/`isFloor_eq`/`isCeil_eq` tie them to
`IsHalfUp`/`IsFloor`/`IsCeil`. `fv`, `huv`, `cv`: floor, half-up and ceiling
valuation chains of `position_value*`.

One supply leg (`SzLeg`): shares `s`, supply index `i`, price `p`, decimals
`dec`, fee rate `f`; `Dd = 10^(27−dec)`. Plan: total collateral `C`, budget
`B`, `opb = W + bonus·10^14`, flags `rad` (`repays_all_debt`) and `sa`
(`seize_all`).

| Closed form | Code |
|---|---|
| `A = huD(s·i, R)` | `actual_ray` |
| `V = huv(s, i, p)` | `asset_value` |
| `Sh = huD(V·W, C)` | `share` |
| `L = huD(B·Sh, W)` | `seizure_for_asset_usd` (leg budget) |
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
| `PFee = min(Bumped, Real)`, `Bumped` = 1 if `huD(Bonus·f, BPS) > 0` and its token floor is 0, `Real = ⌊(PG·Dd − Base)/Dd⌋⁺` | `protocol_fee` |
| `Emit = 0 < S1 ∧ 0 < Capped ∧ 0 < Seized ∧ 0 < Amt` | no `continue` fires |

**SzLegOK** (one leg's production domain): `0 ≤ s ≤ 2^126`,
`s·i ≤ 2^126·R`, `VD s i p`; `0 < i`; `0 < p`; `dec ≤ 18`; `0 ≤ f ≤ BPS`;
`0 < C`, `V ≤ C`; `0 ≤ B`, `B·W ≤ 2^126·p`; `W ≤ opb`.

## Theorems

### Seizure loop and budget

All support INV-LIQ-02, except `sz_resolve_withdrawal_val` (INV-ACCT-01
only). "Served" means the context caches the leg's price and index.

| Lean name | File | Rust function | Claim | Hypotheses |
|---|---|---|---|---|
| `sz_body_spec` | `SeizeBody` | `calculate_seized_collateral` loop body | `continue`, context unchanged, `unseized_usd += Uns`; appends `EntryOf` (amount `Amt`, fee `PFee`, shares `Seized`, bonus shares `Bsc`, fee rate `f`, context price/decimals/index) iff `Emit` | leg served, `SzLegOK`, `unseized_usd + Uns < 2^127` |
| `sz_loop_spec` | `SeizeLoop` | generated loop | output `= input ++ out`, `SzOut` (legs in order, one entry per emitting leg), `unseized = Σ Uns`, context unchanged | every leg as above, `Σ Uns` fits |
| `calculate_seized_collateral_spec` | `SeizeLoop` | `calculate_seized_collateral` | `C > 0`: never aborts, context unchanged; seizures exactly `SzOut` over the supply map with `planLeg` (`B = huD(repay·opb, W)`); unbacked USD `= ⌊(Σ Uns)·W/opb⌋` | `0 ≤ repay`, `0 ≤ bonus ≤ 10^8`, `B < 2^127`, legs served and `SzLegOK`, `Σ Uns < 2^127` |
| `calculate_seized_collateral_no_collateral` | `SeizeLoop` | same | `C ≤ 0` → `([], 0)`, never aborts, context unchanged | none |
| `calculate_seized_collateral_entries` | `SeizeBudget` | same | each entry from a supply entry with the same key: `0 < scaled_amount ≤` held shares, `0 ≤ bonus_scaled ≤ scaled_amount`, `0 < amount ≤ huD(huD(s·i, R), Dd)`, `0 ≤ protocol_fee ≤ amount` (also INV-ACCT-01) | as `_spec` |
| `seized_total_le_budget` | `SeizeBudget` | same (cross-leg) | `2W·Σ SeizedVal ≤ K·(2W·B + n·(B + W)) + W·Σ Slack`; `SeizedVal = Seized·i·p` per emitting leg, `Slack = R·p·(10^9 + 1 + 2X)`, `X = Dd − 1` on the whole-unit round-up branch, else 0 | legs `SzLegOK` with common `C`, `B`, not seize-all; `Σ V ≤ C` (production `C = Σ huv`); `B ≥ 0` |
| `SzLegOK.seized_le`, `.bsc_le`, `.fee_le`, `.amt_le` | `SeizeLeg` | one leg | `0 < Seized ≤ s`; `0 ≤ Bsc ≤ Seized`; `0 < Amt`, `0 ≤ PFee ≤ Amt`; `Amt ≤ huD(A, Dd)` | `SzLegOK`, `Emit` |
| `SzLegOK.seized_val`, `.capped_le`, `.S1_le_S0` | `SeizeLeg` | one leg | `2·Seized·i ≤ R·(2·Capped + 1)`; `¬sa → Capped ≤ S1`; `S1 ≤ S0 + X` | `SzLegOK` |
| `SzLegOK.seized_value_le_budget`, `.seizedVal_le` | `SeizeLeg`, `SeizeBudget` | one leg | `2·Seized·i·p ≤ 2K·L + Slack`: budget plus half a token-wei at its price, half a RAY unit, one whole token on the round-up branch | `SzLegOK`, not seize-all |
| `SzLegOK.L_share` | `SeizeBudget` | one leg | `4WC·L ≤ 4BW·V + 2C(B + W)` (pro-rata share of `B`) | `SzLegOK` |
| `sz_resolve_withdrawal_val` | `SeizeBasic` | `rates/scaling.rs::resolve_withdrawal` | `paid = ⌊⌊pos·i/R⌋/Dd⌋` if `huD(huD(pos·i,R),Dd) ≤ amount`, else `amount`; `0 ≤ paid ≤ amount`, `paid·Dd ≤ ⌊pos·i/R⌋` | `dec < 27`, `0 < i`, `0 ≤ pos`, `0 ≤ amount`, `huD(pos·i, R)` fits (exact abort condition) |

Certora: `liquidation_does_not_increase_seized_collateral` (account level,
bounded) for `calculate_seized_collateral_entries`.

### Plan builder: seizure within the held shares

All support INV-LIQ-02. These theorems are partial correctness: each one is
about a call that returned, and holds for every input. `Rets m P` means "if
`m` returns `r`, then `P r`". "Within" a supply entry `e` means: the same hub
asset as `e`, and `0 < scaled_amount ≤ e.scaled_amount`.

| Lean name | File | Rust function | Claim | Hypotheses |
|---|---|---|---|---|
| `pl_body_rets` | `Plan` | `calculate_seized_collateral` loop body | a returning iteration continues with the rest of the legs; the seizure list is unchanged, or gains one entry within this leg | none |
| `pl_loop_rets` | `Plan` | generated loop | output `= input ++ out`, `SeizedFrom`: legs in order, at most one entry per leg, each within its leg | none |
| `calculate_seized_collateral_seized_le_held` | `Plan` | `calculate_seized_collateral` | the returned seizures are `SeizedFrom` the account's supply map | none |
| `build_liquidation_plan_seized_computed` | `Plan` | `build_liquidation_plan` | the plan's `seized` list is the list one `calculate_seized_collateral` call returned, on the same account, with the total collateral of `calculate_account_risk_totals` on the account's maps | none |
| `build_liquidation_plan_seized_within` | `Plan` | same | every seizure entry is within some entry of the account's supply map | the call returned the plan |
| **`build_liquidation_plan_seized_le_held`** | `Plan` | same | every seizure entry names a supplied hub asset (`Map::get` returns a position) with `0 < scaled_amount ≤ position.scaled_amount`; no two entries name the same hub asset | the call returned the plan; distinct supply keys |
| `seized_le_held_nonvacuous` | `Plan` | `calculate_seized_collateral` | on a concrete account (one position of `10^30` shares, a 100 USD plan at 5%), the call returns exactly one entry of `105·10^27` shares | none |
| `SeizedFrom.mem`, `.keys_sublist`, `.transport`, `.nil` | `Plan` | (lists) | entries come from legs; the entries' hub assets are a sub-list of the legs' hub assets; transport along `TypedOf` | as stated |
| `mapLookup_of_mem` | `Plan` | `Map::get` model | with distinct keys, `get` of an entry's key returns that entry's value | distinct keys |
| `pl_ray_mul_ok`, `pl_bind_ok`, `pl_spec_ok`, `Rets.ok`, `.bind`, `.bind_any`, `.ite` | `Plan` | `Ray::mul`; (logic) | a returning `Ray::mul` has non-negative operands and equals `huD(a·b, R)`; rules for `Rets` | as stated |

`calculate_seized_collateral_entries` states the same per-entry bound under
the hypotheses of `calculate_seized_collateral_spec`. Those hypotheses make
the call return; the bound does not need them. The reason, per leg:

* `actual_ray = Ray::mul(s, i)` returns only for `s ≥ 0` and `i ≥ 0`, and
  equals `huD(s·i, R)` (`mul_div_half_up_returns_iff`).
* `capped_ray ≤ actual_ray` on both branches.
* If `capped_ray = actual_ray`, the entry takes exactly `s`.
* Otherwise `0 < capped_ray < actual_ray`, so `i > 0` and
  `⌊capped_ray·R/i⌋ ≤ s`.
* The loop pushes an entry only when `seized_scaled > 0`.

`build_liquidation_plan` passes its account to `calculate_seized_collateral`
and returns that list unchanged. `release_unbacked_repayment`, the
seizure-flag loop and `LiquidationPlan::validate` only read it.

Certora: `liquidation_does_not_increase_seized_collateral` (account level,
bounded).

### Bonus and seizure proportion

All support INV-LIQ-02 and leave the context unchanged.

| Lean name | File | Rust function | Claim | Hypotheses |
|---|---|---|---|---|
| `get_account_bonus_params_spec` | `SeizeBonus` | `get_account_bonus_params` | `max = max_bonus_for_threshold(p)`; `C = 0` → `base = 0`; `C > 0` → `base = min(huD(Σ huv·bonus, C), max)` (one rounding) | `C ≥ 0`, legs served and `VD`, `Σ huv·bonus < 2^127` (else `MathOverflow`) |
| `bonus_loop_spec` | `SeizeBonus` | its loop | exact accumulation `Σ huv·bonus` | as above |
| `weighted_bonus_between` | `SeizeBonus` | (math) | `C = Σ huv > 0`, stamped bonuses in `[lo, hi]` → `lo ≤ huD(Σ huv·bonus, C) ≤ hi` | as stated |
| `calculate_seizure_proportions_spec` | `SeizeBonus` | `calculate_seizure_proportions` | `p = huD(Wc·W, C)` (`C > 0`) or 0, `0 ≤ p ≤ W`; `0 ≤ base ≤ max ≤ BPS(BPS−1)`, `(BPS + max)·p ≤ BPS·W` (the bonus fields of `EstDomain`) | `0 ≤ Wc ≤ C` (`calculate_account_risk_totals_conservative`), legs served and `VD`, weighted sum fits |

Certora: `bonus_bounded` (bounded) for `get_account_bonus_params_spec`;
`derived_bonus_respects_threshold` for `calculate_seizure_proportions_spec`.

### Post-liquidation health factor

| Lean name | File | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|---|
| `ha_fv_le`, `ha_fv_ge`, `ha_cv_ge`, `ha_cv_le`, `ha_huv_le` | `HealthAfterLegs` | `position_value*` closed forms | `fv·K ≤ s·i·p < fv·K + K + 2·10^36·p`; `s·i·p ≤ cv·K < s·i·p + K + 2·10^36·p`; `huv·K < s·i·p + K + 2·10^36·p` | `s·i ≥ 0`, `p ≥ 0` | INV-RISK-02 |
| `ha_coll_leg` | `HealthAfterLegs` | weighted-collateral term | removing `0 ≤ σ ≤ s` shares lowers `⌊fv·LT/BPS⌋` by at most `(LT·(σ·i·p + K + 2·10^36·p) + BPS·K)/(BPS·K)` | as stated, `LT ≥ 0` | INV-RISK-02 |
| `ha_huv_weighted` | `HealthAfterLegs` | same | `K·LT·huv ≤ K·BPS·(⌊fv·LT/BPS⌋ + 1) + LT·(2K + 4·10^36·p)` | | INV-RISK-02 |
| `ha_debt_leg` | `HealthAfterLegs` | debt term + `resolve_repay` | `DebtStep` untouched, partial floor burn with credited USD `≤ huD(a·10^(18−dec)·p, W)`, or full: `0 ≤ t' ≤ t`, `2K·(cv(t) − cv(t')) ≥ 2K·u − 3K − 6·10^36·p` | `t ≥ 0`, `0 < ib ≤ 10^36`, `p ≥ 0`, `dec ≤ 18` | INV-LIQ-02, INV-RISK-02 |
| `ha_combine`, `ha_coll_bound`, `ha_combine_target` | `HealthAfterCombine` | (math) | combination steps ([below](#the-post-liquidation-health-factor-in-plain-words)) | | |
| **`health_after_hf_preserving`** | `HealthAfter` | `calculate_account_risk_totals` before/after | `Wc'·D ≥ Wc·D' − Wc·Ed/(2K) − D·ErrC` | [below](#the-post-liquidation-health-factor-in-plain-words) | INV-LIQ-02, INV-RISK-02 |
| **`health_after_target`** | `HealthAfter` | same | `W·Wc' ≥ T·D' − (T·Ed/(2K) + (T − δ + W)/2 + rep/2 + W·ErrC)` | as above, `HitsTarget` and `r ≤ rep` instead of the cap | INV-LIQ-02 |
| `sum_regroup`, `lsum_ite_key` | `HealthAfterPlan` | (math) | a sum over items keyed by a map equals, per map entry, the sum of its items | `(map keys).Nodup`, every item's key a map key | |
| `plan_credits_sum`, `mapLookup_mem_keys` | `HealthAfterPlan` | `normalize_repayment_plan` | for account `acc`: `0 ≤ repay_usd = Σ planU(k)` over its debt legs (`planU(k)`: `usd_wad` of key `k`'s repaid entries); discharges `hr0`/`hrep`; every repaid entry names a debt leg (from `RepayLegOK`) | those of `repayment_plan_spec` ([controller-repayment.md](controller-repayment.md)); distinct debt keys (Soroban `Map`) | INV-LIQ-02 |
| **`health_after_executed_plan`** | `HealthAfterPlan` | `normalize_repayment_plan` + `calculate_account_risk_totals` | `health_after_hf_preserving` for the production plan: maps `acc.supply_positions` / `acc.borrow_positions`, each debt leg repaid `planA(k)` tokens, credited `planU(k)` USD | as `plan_credits_sum`, totals domain of both accounts, `SzLegOK` per supply leg, `SupplyAfter`/`DebtAfter` (application, modelled), not seize-all, the HF-preserving cap | INV-LIQ-02, INV-RISK-02 |
| `ha_counterexample` | `HealthAfter` | same | a concrete tight-cap account where the HF drops one unit ([below](#the-post-liquidation-health-factor-in-plain-words)) | none | INV-LIQ-02 |

Certora: `position_value_ceil_ge_floor` (valuation bounds);
`liquidation_does_not_increase_repaid_debt` (`ha_debt_leg`);
`split_liq_partial_at_the_cap_never_lowers_collateral_coverage` states
`health_after_hf_preserving`'s idea for `C/D` with a `repay/10^10 + 2`
allowance; `ideal_repayment_targets_curve_hf` states `health_after_target`'s
intent but never reaches the target branch (see controller-estimate.md).

Supporting lemmas, documented in the files: value forms `sz_*`; `SzLegOK.*`
bounds (`AB`, `ShW`, `LB`, `QB`, `S1B`, `A1B`, `wusd_fit`, `part_le`,
`div_fit`, `F_le_H`, `PG_le`, `Uns0`, `UnsL`); `SzOut.mem`,
`SzOut.transport`, `legOf_typed`; list sums `sz_lsum_*`, `ha_lsum_*`,
`ha_forall2_*`.

### Why the hypotheses hold in production

* **Distinct supply keys** (`build_liquidation_plan_seized_le_held`). The
  account's `supply_positions` is a Soroban `Map`, whose keys are unique. The
  host enforces it; no repository code does. The list model of `Map` does not
  record it ([trusted base](extraction.md#controller-trusted-base)).
* **The call returned.** This is the premise of a partial-correctness
  theorem, not a domain. `seized_le_held_nonvacuous` shows a returning call
  of `calculate_seized_collateral` with a non-empty list.
* **`SzLegOK`.** Shares, indexes, prices are non-negative `Ray`/`Wad`; supply
  index ≥ its floor (`0 < i`); oracle prices positive;
  `MAX_ASSET_DECIMALS = 18`; stamped fee rates validated below 100%. `s ≤ 2^126`, `s·i ≤ 2^126·R`:
  held shares and their RAY value fit `i128` with a factor-2 margin (about
  `8.5·10^10` tokens, the order of every valuation's `Ray::mul` limit). `VD`
  is the totals' value domain. `C` is `calculate_account_risk_totals`'
  half-up total over the same legs, so `V ≤ C`. `W ≤ opb`: the bonus is
  non-negative.
* **`B·W ≤ 2^126·p`** is the only non-type/validation hypothesis.
  `Wad::div(seizure_for_asset_usd, price)` aborts exactly when
  `huD(L·W, p) ≥ 2^127` (`mul_div_half_up_returns_iff`), a request above
  about `1.7·10^20` tokens. The request is about `(B/C)·held`, and every
  estimate branch keeps `B ≲ C`: insolvent `repay ≤ C/(1 + base)`; solvent,
  the cap `(1 + b)·p ≤ hf` gives `(1 + b)·D ≤ C`; whole-unit repayment raises
  the quote to one unit of a leg holding at least one. So the request is at
  most the held amount plus rounding. It is stated, not derived, because
  plan-level `B ≲ C` goes through `normalize_repayment_plan`
  ([controller-repayment.md](controller-repayment.md)).
* **`Σ Uns < 2^127`:** each `Uns ≤ L ≤ B` (`UnsL`, `LB`).
* **Bonus `≤ 10^8`:** `max ≤ BPS·(BPS−1)` (`max_bonus_for_threshold_bound`),
  plan bonus `≤ max`.
* **Post-liquidation model.** `apply_liquidation_share_credit` subtracts
  exactly `entry.scaled_amount` (= `Seized`) (credit mode). The pool's repay
  burns `resolve_repay`'s `⌊a·10^(27−dec)·R/ib⌋` shares on a partial repay,
  the whole position on a full one (`RatesAeneas` `resolve_repay_spec`). A
  leg's credited USD is `usd_value_wad(a)` (`calculate_repayment_amounts_spec`),
  or less after trimming (`process_excess_payment` takes `min(…, usd)`).
  `repay_usd = Σ usd_wad` (`LiquidationPlan::validate`,
  `sum_repaid_usd_spec`). `aggregate_positive_payments` gives one repayment
  per debt key. Borrow index in `[RAY, 10^36]`.

## The post-liquidation health factor, in plain words

Totals from `calculate_account_risk_totals_spec`: `C`, `Wc`, `D` before the
liquidation (half-up collateral, threshold-weighted floor collateral,
ceiling debt); `hf = ⌊Wc·W/D⌋`; seizure proportion `p = huD(Wc·W, C)`;
`Wc'`, `D'` after the plan.

**At the HF-preserving bonus** (`(BPS + b)·p ≤ hf·BPS`, guaranteed by
`estimate_liquidation_amount_contract` on a solvent liquidatable account
unless it closes the whole debt at bonus 0):

`Wc'/D' ≥ Wc/D − (Wc·Ed/(2K) + D·ErrC)/(D·D')`.

In exact arithmetic (`hf_preserved_of_rate`) the HF never decreases. The
allowance covers the code's ~fifteen roundings, in raw WAD units (`10^-18`
USD) of weighted collateral:

* debt, `Ed/(2K) = Σ_debt (1.5 + 3·p_j/W)`: floor burn and ceiling valuation,
  at most 1.5 units plus 3 token-wei at the leg's price per debt leg;
* collateral, `ErrC`, five terms:
  * `rep·opb/(2W²) + 1/2`: half-up budget and proportion;
  * `B·(n + Ev/(K·BPS))/C`: half-up vs floor valuation, about
    `(2 + 4·p_i/W)` units per leg scaled by `B/C ≲ 1`;
  * `(B + W)·Σ LT/(2W·BPS)`: half-up share and leg budget, half a unit per
    leg plus a `10^-18` relative share of the budget;
  * `(Es/2 + Ec)/(BPS·K)`: per-leg seizure slack (half a token-wei, half a
    RAY unit, a whole token on the whole-unit round-up branch) plus the
    floor of the post value;
  * `n`: the weighting floor, one unit per supply leg.

For a realistic account this is below `10^-15` USD per leg. The exception is
the whole-unit round-up branch (`repays_all_debt` on a leg below 3
decimals): one whole token per such leg, seized on purpose. The debt is then
fully repaid (post-HF `i128::MAX`), so the bound is moot.

**On the curve-target branch** (`HitsTarget D Wc p T b r` from
`estimate_hits_curve_target`, plan repays at least the quote, `r ≤ rep`):

`W·Wc' ≥ T·D' − (T·Ed/(2K) + (T − δ + W)/2 + rep/2 + W·ErrC)`:

post-HF ≥ target `T/W`, up to the estimate's target slack (`≤ 5.5` units),
half a unit per repaid unit for using `δ = huD(p·opb, W)` instead of
`p·opb/W` (a `5·10^-19` relative error), and the rounding above. Repaying
more than the quote only helps (`δ < T`). Only the lower bound is proved.

**Counterexample: the allowance is needed** (`ha_counterexample`). One supply
leg of USD 1100 at threshold 50% and one debt leg of USD 1000 (18 decimals,
price 1, index `RAY`): `C = 1100 W`, `Wc = 550 W`, `D = 1000 W`,
`hf = 0.55 W`, `p = 0.5 W`. The cap is exactly 1000 BPS:
`(BPS + 1000)·p = hf·BPS`. Repaying 1001 raw units: budget
`huD(1001·1.1) = 1101`, seized shares `1101·10^9`, debt shares burned
`1001·10^9`, so `Wc' = 550 W − 551`, `D' = 1000 W − 1001`, `Wc'·D < Wc·D'`,
and the HF goes from `550000000000000000` to `549999999999999999`. The
half-up budget seizes 1101 instead of 1100.1; the weighting floor loses the
other half unit. Not exploitable (the liquidator pays at least the value it
takes, up to that unit), but "post-HF ≥ pre-HF" holds only up to rounding.

## Findings

No production bug. Observations:

1. **Post-HF can fall one `10^-18` unit at the HF-preserving cap**
   (counterexample above), within `health_after_hf_preserving`'s allowance.
2. **Seizure rounds half up at four steps** (`Wad::mul` budget, `Wad::div`
   share, `Wad::mul` leg budget, `Wad::div` by price), so a leg can exceed
   its exact pro-rata budget by half a token-wei at its price and half a RAY
   unit (`seized_value_le_budget`); the total by `n·(B + W)/(2W)` units
   (`seized_total_le_budget`). The whole-unit round-up branch seizes up to
   one whole token more, as INV-LIQ-02 documents.
3. **The `Wad::div` by the price can abort** above about `1.7·10^20` tokens;
   the code comment mentions only the saturating RAY upscale. Unreachable
   from `build_liquidation_plan` ([why](#why-the-hypotheses-hold-in-production)).
4. **Every emitted entry passes `LiquidationPlan::validate`'s seizure
   checks** (`calculate_seized_collateral_entries`), except
   `liquidation_fees < BPS`, which depends on the stamped rate (an input).
5. **`LiquidationPlan::validate` does not compare a seizure with the held
   shares.** The bound holds by construction
   (`build_liquidation_plan_seized_le_held`). In credit mode the only runtime
   check is the `checked_sub` in `apply_liquidation_share_credit`.

## Not covered

* **`build_liquidation_plan` beyond the seizure bound.** Not proved: that it
  returns on the production domain; the health-factor gate; the spoke-flag
  loops; the effect of `release_unbacked_repayment` on the returned
  repayment; that `LiquidationPlan::validate` passes. The bound covers the
  `scaled_amount` field. The `amount` field is bounded only under `SzLegOK`
  (`calculate_seized_collateral_entries`).
* **A returning call of `build_liquidation_plan` in Lean.** The spoke flags
  and the spoke configuration are uninterpreted, so no concrete call is
  evaluated. `seized_le_held_nonvacuous` evaluates the seizure call only.
* **Exact abort set of the loop.** `SzLegOK` is sufficient, not necessary:
  `B·W ≤ 2^126·p` is the sufficient form of `huD(L·W, p) < 2^127`.
* **Plan-level `B ≲ C` and post-account domain.** `rep = Σ u_j` and
  `0 ≤ rep` come from the production plan (`plan_credits_sum`, used by
  `health_after_executed_plan`). Still hypotheses: every supply leg in
  `SzLegOK` (needs `B ≲ C`), and the post account in the totals domain
  (follows from `s' ≤ s`, `t' ≤ t`; `ha_debt_leg` proves `0 ≤ t' ≤ t`, not
  assembled).
* **`health_after_target` for the production plan** is not restated like
  `health_after_executed_plan`; `plan_credits_sum` discharges its plan
  hypotheses the same way.
* **Transfer-mode seizure.** The model is credit mode (`scaled_amount`
  subtracted exactly). Transfer mode burns `⌈amount·Dd·R/i⌉ ≤ Seized + 1`
  shares via the pool's withdrawal; the extra share is not modelled.
* **Full close at low cap with `cap < 0`** (bonus 0, does not preserve HF;
  finding 4 of controller-estimate.md) and **seize-all** (insolvent, takes
  all collateral) are outside the health theorems.
* **Upper bound on the target branch** (no overshoot beyond rounding). It
  needs lower bounds on every seizure rounding and the held-balance cap.
* **`apply_liquidation_seizures`, `apply_liquidation_share_credit`, the pool
  repay**: modelled by `SupplyAfter`/`DebtAfter`, not proved from the
  external calls.
* **`repays_every_debt_leg`** is opaque
  ([Aeneas issue 5](extraction.md#controller-aeneas-issues)); every theorem
  here holds for both values of `repays_all_debt`.

## Axioms

`#assert_standard_axioms` (end of `ControllerAeneas.lean`) covers every
declaration of the eleven files ([README](../README.md#trusted-base)).
