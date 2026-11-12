# Controller liquidation repayment plan (Aeneas + Lean)

Machine-checked Lean 4 proofs about the repayment side of a liquidation in
`contracts/controller/src/positions/liquidation/math.rs`:
`process_excess_payment`, `release_unbacked_repayment`,
`one_unit_per_leg_usd`, `one_unit_per_debt_leg_usd`, `whole_unit_repayment`
and `normalize_repayment_plan`, composed into one pipeline theorem
(`repayment_plan_spec`). Every theorem is about the generated
`lean/ControllerAeneas/Funs.lean` (library `ControllerAeneas`). The
extraction, trusted base and the proofs these build on
(`calculate_repayment_amounts_spec`, `sum_repaid_usd_nonneg`, the Map/Context
model) are in [controller.md](controller.md); the curve quote
(`estimate_liquidation_amount_contract`) is in
[controller-estimate.md](controller-estimate.md). Conventions:
[../WRITING-PROOFS.md](../WRITING-PROOFS.md).

## Scope

| Rust function | Status |
|---|---|
| `process_excess_payment` | total correctness: loop invariant, exact token refunds, value bounds in both modes, output a pointwise-smaller sublist, never aborts on its domain |
| `release_unbacked_repayment` | releases at most the unbacked value, exact refunds, flags cleared |
| `one_unit_per_leg_usd`, `one_unit_per_debt_leg_usd` | exact sums, never abort on their domain |
| `whole_unit_repayment` | total correctness, `quote ≤ r ≤ D`, exact characterisation of both raised outcomes and their rounding direction |
| `normalize_repayment_plan` | total correctness (given that the opaque `repays_every_debt_leg` returns), plan facts (`NormSpec`) |
| pipeline | `repayment_plan_spec`: per leg `repaid ≤ debt` and `≤ offered`, exact refunds, `repay_usd = Σ usd_wad` |
| `repays_every_debt_leg` | opaque (see controller.md, Aeneas issue 5): its flag is arbitrary in the model |

Files (plain, non-`module` Lean files under `lean/ControllerAeneas/Proofs/`):

* `RepayPlanArith.lean`: exact specs of `Wad::{div_floor, div_ceil, mul_floor, mul, to_token_floor}`.
* `RepayPlanExcess.lean`: `process_excess_payment` (fates, shape, invariant, consequences).
* `RepayPlanUnit.lean`: `one_unit_per_*`, `whole_unit_repayment`.
* `RepayPlan.lean`: `release_unbacked_repayment`, `normalize_repayment_plan`, `repayment_plan_spec`, `whole_unit_returns`.

Build: `lake build ControllerAeneas.Proofs.RepayPlan` (imports the other three).

## Notation

`W = 10^18`. A repayment leg (`RepayEntry`) has `amount`, recorded value
`usd` (`usd_wad`), feed price `p` (`price_wad`) and decimals `dec`;
`S = 10^(18−dec)`. `hu(n/d)` is rounding to nearest, halves up (`huD`).
`tokUsd(a) = hu(a·S·p/W)` (the value `PriceFeed::usd_value_wad` records),
`unitUsd(dec, p) = hu(S·p/W)` (one native unit), `opb = W + bonus·10^14`
(`1 + bonus`). For a merged payment leg `l = (k, offered)`:
`D(k)` = ceiling-rounded debt in tokens (`debtTokens`),
`legPay(l) = min(offered, D)`, `legUsd(l)` = the recorded value
(controller.md, `calculate_repayment_amounts_spec`). `Σ` over a list of legs:
`usdSum` (values), `amtSum` (amounts), `refSum` (refund amounts).

## The loop invariant of `process_excess_payment`

The loop walks the legs from the last one to the first, holding the
remaining excess `rem` (initially `E`). In plain words, at index `k`
(the next leg to visit is `L[k−1]`):

1. **Shape** (`ExcessShape`): the first `k` legs are exactly the input's;
   every leg from `k` on has been visited and has a *fate* (`LegFate`):
   * *kept*: unchanged, no refund (amount `≤ 0` or value `0` is skipped);
   * *trimmed*: amount lowered to `a ∈ (0, amount]`, value lowered to
     `u ∈ [0, usd]`, hub asset, feed and index snapshot unchanged, one refund
     of exactly `amount − a` of the leg's asset;
   * *removed*: one refund of exactly its (positive) amount.

   The current list is the untouched prefix followed by what is left of the
   visited legs, in their original order; the refunds are the initial ones
   followed by the visited legs' refunds, last leg first.
2. `rem ≥ 0`, and the recorded total never grew: `usdSum(cur) ≤ usdSum(L)`.
3. Without `keep_within_quote`: the value removed so far is at most
   `E − rem` (`usdSum L − E + rem ≤ usdSum cur`): a removed leg takes its
   value off `rem`; a trim keeps the *ceiling* of the backed tokens, so its
   kept value is at least `usd − rem`, and then `rem` becomes `0`.
4. With `keep_within_quote`: the value removed so far is at least `E − rem`
   (`usdSum cur ≤ usdSum L − E + rem`): a trim keeps the *floor* of the backed
   tokens, so its kept value is at most `usd − rem`.
5. While excess remains (`rem > 0`) every visited leg still present was
   skipped (non-positive amount or zero value).

The loop stops when `rem = 0` or `k = 0`; `excess_done` turns the invariant
into `ExcessPost`.

## Theorems

| Lean name | File | Rust function | Claim | Hypotheses | Invariants | Certora |
|---|---|---|---|---|---|---|
| `rp_div_floor_spec`, `rp_div_ceil_spec` | `RepayPlanArith` | `fp.rs::Wad::div_floor`, `div_ceil` | `⌊a·W/b⌋`, `⌈a·W/b⌉` | `a ≥ 0`, `b > 0`, quotient fits | — | — |
| `rp_mul_floor_spec`, `rp_mul_spec` | `RepayPlanArith` | `Wad::mul_floor`, `Wad::mul` | `⌊a·b/W⌋`, `hu(a·b/W)` | `a, b ≥ 0`, result fits | — | — |
| `rp_to_token_floor_spec` | `RepayPlanArith` | `Wad::to_token_floor` | `⌊a/10^(18−dec)⌋`, never aborts | `dec ≤ 18`, `a ≥ 0` | — | — |
| `keep_amount_spec`, `keep_amount_arith` | `RepayPlanExcess` | `process_excess_payment` (kept amount, `keep_within_quote`) | `a = min(⌊⌊kept·W/p⌋/S⌋, amount) ∈ [0, amount]` and `a·S·p ≤ kept·W` | `0 ≤ rem < usd`, `p > 0`, `dec ≤ 18`, `usd·W ≤ (2^127−1)·p` | INV-LIQ-02 | — |
| `ceil_amount_spec`, `ceil_covered_arith`, `ceil_uncovered_arith` | `RepayPlanExcess` | same (kept amount, otherwise) | `a ∈ [1, amount]` and `a = amount ∨ kept·W ≤ a·S·p` (the ceiling covers the kept value) | as above, `rem > 0` | INV-LIQ-02 | — |
| `excess_loop_spec` | `RepayPlanExcess` | the generated loop | the invariant above is preserved; the loop returns `ExcessPost` | `ExcessLegOK` per leg | INV-LIQ-02 | — |
| `process_excess_payment_spec` | `RepayPlanExcess` | `math.rs::process_excess_payment` | never aborts; `ExcessPost`: shape (fates), `usdSum out ≤ usdSum L`; without `keep`: `usdSum out ≥ usdSum L − E`; with `keep`: `usdSum out ≤ usdSum L − E`, or every leg was visited and every leg left has amount `≤ 0` or value `0` | `|L| < 2^32`, `E ≥ 0`, `ExcessLegOK` | INV-LIQ-02 | — |
| `excess_tokens_conserved` | `RepayPlanExcess` | same | `amtSum out + refSum new = amtSum L`, new refunds appended after the old ones | a shape | INV-LIQ-02 | — |
| `legFate_removed_iff` | `RepayPlanExcess` | same (per leg) | a leg is removed iff its refund is its whole amount; a trimmed leg's refund is below its amount | a fate | INV-LIQ-02 | — |
| `excess_nonneg` | `RepayPlanExcess` | same | non-negative inputs give non-negative output amounts and values and non-negative new refunds | a shape | INV-LIQ-02 | — |
| `excess_length_le`, `excess_sublist` | `RepayPlanExcess` | same | `|out| ≤ |L|`; `out` is `LegLe`-paired with a sublist of `L` (same asset, feed, index; amount and value only lowered) | a shape | INV-LIQ-02 | — |
| `one_unit_per_leg_usd_spec` | `RepayPlanUnit` | `math.rs::one_unit_per_leg_usd` | returns `Σ unitUsd(dec, p)` | `dec ≤ 18`, `p ≥ 0`, sum `< 2^127` | INV-LIQ-02 | — |
| `one_unit_per_debt_leg_usd_spec` | `RepayPlanUnit` | `math.rs::one_unit_per_debt_leg_usd` | returns `Σ` over the borrow keys of the cached feed's `unitUsd`, context unchanged | every debt asset priced (`UnitKeyOK`), sum `< 2^127` | INV-LIQ-02 | — |
| `whole_unit_repayment_spec` | `RepayPlanUnit` | `math.rs::whole_unit_repayment` | never aborts, context unchanged, `quote ≤ r ≤ D`; if `r ≠ quote`: one supply leg with `dec < 3`, `hu(quote·opb/W) < m`, and either `r = D` with `(D + U)·opb ≤ u·W` (floor test) or `r = ⌈m·W/opb⌉ < D` (so `r·opb ≥ m·W`) with the full-close test failed (`WholeUnitRaised`) | `WholeUnitDom` | INV-LIQ-02 (whole-unit legs) | — |
| `raised_ge_quote` | `RepayPlanUnit` | same | the raised repayment is at least the quote | `opb > 0`, `0 ≤ q < D` | INV-LIQ-02 | — |
| `legUsd_le_tokUsd` | `RepayPlan` | `calculate_repayment_amounts` (per leg) | `legUsd ≤ tokUsd(legPay)`: a capped leg's floor debt value is at most the value of the ceiling-rounded `D` | `RepayLegOK` | INV-LIQ-02 | — |
| `legUsd_zero_of_pay_nonpos`, `legPay_nonneg` | `RepayPlan` | same | `legPay ≥ 0`; `legPay ≤ 0 → legUsd = 0` | `RepayLegOK` | INV-LIQ-02 | — |
| `calc_tokens_conserved` | `RepayPlan` | same | `amtSum entries + refSum refunds = Σ offered` | the spec's entries and refunds | INV-LIQ-02 | — |
| `release_unbacked_repayment_spec` | `RepayPlan` | `math.rs::release_unbacked_repayment` | `E ≤ 0`: unchanged. `E > 0`: legs and refunds from `process_excess_payment` (`keep = false`, `ExcessPost`), `repay_usd = Σ usd_wad`, `Σ usd(before) − E ≤ repay_usd ≤ Σ usd(before)` (releases at most `E`), `full_close = repays_all_debt = false`, bonus and `seize_all` unchanged | `|legs| < 2^32`, `ExcessLegOK false` per leg, `Σ usd < 2^127` | INV-LIQ-02 | — |
| `normalize_tail_spec` | `RepayPlan` | `normalize_repayment_plan` after the ideal is known | trimming, total, `seize_all`, record (see `NormSpec`) | as stated | INV-LIQ-02 | — |
| `normalize_repayment_plan_spec` | `RepayPlan` | `math.rs::normalize_repayment_plan` | never aborts; `NormSpec`: planned entries/refunds, plan legs a `LegLe`-sublist, exact tokens, `repay_usd = Σ usd_wad ≤ Σ legUsd`, full close not trimmed, `seize_all → C < D`, bonus = the estimate's, quote `q ∈ [0, D]`; insolvent: full close iff `q ≥ D`, else `repay_usd ≤ q`; solvent partial: `repay_usd ≥ min(Σ legUsd, I)` for the ideal `I < D` | REP + `RepayLegTrimOK` per leg, `|P| < 2^32`, `Σ legUsd + Σ unit < 2^127`, `EstDomain`, `hwu`, `hrep` | INV-LIQ-02 | `liquidation_does_not_increase_repaid_debt`; the `split_liq_*` rules assume this acceptance |
| `repayment_plan_spec` | `RepayPlan` | the pipeline | per repaid leg `x` there is a merged leg `l = (k, offered)` with the same hub asset, `0 ≤ x.amount ≤ D(k)`, `x.amount ≤ offered`, `0 ≤ x.usd_wad ≤ legUsd(l)`, feed and borrow-index snapshot of `k`; `Σ repaid + Σ refunds = Σ offered`; `repay_usd = Σ usd_wad`, `0 ≤ repay_usd ≤ Σ legUsd`; `|repaid| ≤ |P|`; and `NormSpec` | as for `normalize_repayment_plan_spec` | INV-LIQ-02 | `liquidation_does_not_increase_repaid_debt` (per leg, unbounded) |
| `whole_unit_returns` | `RepayPlan` | `whole_unit_repayment` | discharges `hwu` | `WholeUnitAcctDom`, `max ≤ BPS·(BPS−1)` | INV-LIQ-02 | — |

Helpers (all proved, doc comments in the files): `rp_huD_*`, `shape_*`,
`inv_skip`, `inv_partial`, `inv_remove`, `excess_loop_unfold`,
`excess_done`, `legFate_amount`, `fates_amount`, `LegLe.*`, `unit_value_spec`,
`unitUsd_le`, `rp_unit_fit`, `one_unit_*_loop_*`, `rp_cached_*_spec`,
`rp_ray_mul_spec`, `entry_excess_ok`, `rp_forall2_*`, `rp_lsum_sublist_le`.

### Hypotheses and why they hold in production

* **`ExcessLegOK keep e`** (per leg of `process_excess_payment`): `usd ≥ 0`;
  a leg with positive amount and value has a positive price, `dec ≤ 18`,
  `usd·W ≤ (2^127−1)·p`, `amount·S < 2^127`, `tokUsd(amount) < 2^127`, and
  (without `keep`) `usd ≤ tokUsd(amount)`. For the legs
  `calculate_repayment_amounts` records, `entry_excess_ok` derives all of it
  from REP (controller.md) plus `RepayLegTrimOK` (positive price, the
  quotient and the kept amount's value fit `i128`); the backing condition is
  `legUsd_le_tokUsd`. Oracle prices are positive; `usd·W/p` is about the
  token amount scaled to 18 decimals, far below `2^127`.
* **`|L| < 2^32`**: the host `Vec` length is a `u32`.
* **`E ≥ 0`**: `normalize_repayment_plan` passes `total − ideal` only when
  `total > ideal`; `release_unbacked_repayment` returns early on `E ≤ 0`.
* **`EstDomain`**: see controller-estimate.md (validated curve, bonus bounds,
  consistent snapshot, `D ≤ 10^36`).
* **`hwu`** (the whole-unit raise returns a non-negative value): discharged
  by `whole_unit_returns` on `WholeUnitAcctDom`: `D ≤ 10^33` WAD (10^15 USD,
  so that `quote·(1 + bonus)` fits for every bonus up to `BPS·(BPS−1)`),
  fewer than `2^32` supply legs, each served by the context (`load_markets`
  runs first) with `dec ≤ 18`, price `≤ 10^36` and a RAY value that fits,
  every debt asset priced, `D + Σ unit < 2^127`.
* **`hrep`**: `repays_every_debt_leg` is opaque (uninterpreted); the theorem
  assumes it returns. Its Rust body is a lookup loop that can abort only where
  `cached_market_index` (a pool fetch for an uncached key) or
  `unscale_borrow_ceil` (overflow) can.
* **`Σ legUsd + Σ unit < 2^127`**: the repaid value plus one native unit per
  leg, in WAD USD.

### Certora claims checked

* `liquidation_does_not_increase_repaid_debt` (liquidation_rules, bounded,
  one leg): the repaid amount per leg never exceeds the ceiling-rounded debt
  — proved for every leg and every input (`repayment_plan_spec`, item 1).
* `split_liq_*` (liquidation_rules) assume that a partial plan accepts at
  most the ideal amount on an insolvent account and trims the excess:
  `normalize_repayment_plan_spec` proves `repay_usd ≤ q` on an insolvent
  partial plan. On a solvent partial plan the kept amount is rounded *up* to
  whole token units, so the kept value may exceed the ideal by less than one
  token unit of the trimmed leg (the theorem proves the lower bound
  `repay_usd ≥ min(Σ legUsd, I)`).
* boundary_rules, consistency_rules and position_rules have no claim about
  these functions (they cover bad-debt gates and the supply/borrow paths).

## Findings

No counterexample to an intended property was found. Observations:

* **Zero refunds.** Without `keep_within_quote`, when the remaining excess is
  smaller than the value of one native unit of the trimmed leg, the ceiling
  keeps the whole amount and the loop appends a refund of `0` tokens (and
  releases nothing). Example: USDC-like leg, `dec = 6`, `p = 10^18`,
  `amount = 100`, `usd = 10^14`, `rem = 1`: `kept = 10^14 − 1`,
  `floor = 99`, not covered, so `new_amount = min(100, 100) = 100`, refund
  `0`, value unchanged. Refunds of a partial plan are only reported (never
  pulled), so this is cosmetic. It is allowed by the theorems (`trim` with
  `a = amount`).
* **Dead branch with `keep_within_quote`.** The kept value is at most
  `usd − rem` (`keep_amount_arith`), so `removed_usd ≥ remaining_excess_usd`
  always holds and the `else` branch (`remaining − removed`) is never taken;
  the loop proof shows it by contradiction. Harmless.
* **Solvent trim rounds toward the liquidator.** As documented in the Rust
  comment, the solvent trim keeps the ceiling of the backed tokens; the
  released value can be below the excess by up to one native unit's value
  (`release_unbacked_repayment` likewise releases *at most* the unbacked
  part).

## Not covered

* **Exact abort conditions of `process_excess_payment`**: only sufficient
  conditions (`ExcessLegOK`) are proved; outside them (e.g. a zero price on a
  leg with positive value, or a negative recorded value) the call can abort.
* **Lower bound of the released value** (release `≥ E − one unit`) is not
  stated; only `≤ E` (and exact token conservation) is.
* **The `held_units ≥ 1` condition** of `whole_unit_repayment` is not part of
  `WholeUnitRaised` (a raise implies it, but the statement does not record
  it), nor are the values of the `quote`-returning branches characterised
  beyond `r = quote`.
* **`repays_every_debt_leg`** and therefore the `repays_all_debt` flag
  (opaque, see controller.md).
* **`seize_all`**: only `seize_all → C < D` is proved, not its exact value.
* **Consistency with the seizure budget** passed to
  `release_unbacked_repayment` (the `unseized_usd` of
  `calculate_seized_collateral`): proved only that at most that value is
  released; the per-leg seizure loop is the seizure-side proof's subject.
* **`aggregate_positive_payments`** is taken as given (a hypothesis naming
  the merged list), as in controller.md.

## Axioms

`#print axioms` was run on all 76 theorems of the four files (together with
the whole library and the seizure/health-factor files, to check that no name
collides): every one depends on a subset of `propext`, `Classical.choice`,
`Quot.sound`. No `sorry`, `admit` or `axiom` in these files.
