/-
`liquidation::math::sum_repaid_usd`: the WAD USD total of a repayment plan is
exactly the sum of its legs' recorded `usd_wad` values. This is the quantity
`NormalizedRepaymentPlan::validate` compares against `repay_usd`, and the one
`calculate_seized_collateral` multiplies by `1 + bonus` (INV-LIQ-02).
-/
import ControllerAeneas.Proofs.Basic

open Aeneas Aeneas.Std Result
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- Exact sum of the recorded USD values of a list of repayment legs. -/
def usdSum (l : List types.RepayEntry) : Int := (l.map (fun e => e.usd_wad.val)).sum

/-- A value fits `i128`. -/
abbrev FitsI128 (v : Int) : Prop := -2^127 ≤ v ∧ v < 2^127

theorem usdSum_cons (e : types.RepayEntry) (l : List types.RepayEntry) :
    usdSum (e :: l) = e.usd_wad.val + usdSum l := by
  simp [usdSum]

/-- One iteration of the generated loop: consume the front leg and add its
recorded value to the accumulator. -/
theorem sum_repaid_usd_loop_cons (env : Env) (e : types.RepayEntry) (l : List types.RepayEntry)
    (acc : math.fp.Wad) :
    positions.liquidation.math.sum_repaid_usd_loop (e :: l) env acc = (do
      let t ← math.fp.checked_add_raw env acc e.usd_wad
      positions.liquidation.math.sum_repaid_usd_loop l env t) := by
  conv => lhs; unfold positions.liquidation.math.sum_repaid_usd_loop; rw [loop]
  simp [positions.liquidation.math.sum_repaid_usd_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next,
    math.fp.Wad.checked_add, positions.liquidation.math.sum_repaid_usd_loop]

/-- The loop stops on an exhausted iterator and returns the accumulator. -/
theorem sum_repaid_usd_loop_nil (env : Env) (acc : math.fp.Wad) :
    positions.liquidation.math.sum_repaid_usd_loop [] env acc = ok acc := by
  unfold positions.liquidation.math.sum_repaid_usd_loop
  rw [loop]
  simp [positions.liquidation.math.sum_repaid_usd_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The generated loop of `sum_repaid_usd`, from accumulator `acc` over the
remaining legs `l`, returns `acc + Σ usd_wad` whenever every running total
fits `i128`. -/
theorem sum_repaid_usd_loop_spec (env : Env) :
    ∀ (l : List types.RepayEntry) (acc : math.fp.Wad),
      (∀ k, k ≤ l.length → FitsI128 (acc.val + usdSum (l.take k))) →
      positions.liquidation.math.sum_repaid_usd_loop l env acc
        ⦃ r => r.val = acc.val + usdSum l ⦄ := by
  intro l
  induction l with
  | nil =>
    intro acc _
    simp [sum_repaid_usd_loop_nil, usdSum]
  | cons e l ih =>
    intro acc hfit
    have h1 := hfit 1 (by simp)
    have h1' : FitsI128 (acc.val + e.usd_wad.val) := by
      simpa [usdSum] using h1
    rw [sum_repaid_usd_loop_cons]
    step with checked_add_raw_spec env acc e.usd_wad h1' as ⟨t, ht⟩
    have := ih t (by
      intro k hk
      have h2 := hfit (k + 1) (by simp; omega)
      simp only [List.take_succ_cons, usdSum_cons, FitsI128] at h2
      simp only [FitsI128]
      rw [ht]; constructor <;> linarith [h2.1, h2.2])
    apply WP.spec_mono this
    intro r hr
    rw [hr, ht, usdSum_cons]; ring

/-- **`sum_repaid_usd` is the exact sum.** For any list of repayment legs
whose running totals all fit `i128`, `sum_repaid_usd` returns (does not abort)
and its value is exactly `Σ usd_wad`. No rounding happens: the per-leg values
are added with `Wad::checked_add`.

Hypothesis: every prefix sum fits `i128` (otherwise `checked_add` aborts with
`MathOverflow`; see `sum_repaid_usd_nonneg` for the production form). -/
theorem sum_repaid_usd_spec (env : Env) (l : List types.RepayEntry)
    (hfit : ∀ k, k ≤ l.length → FitsI128 (usdSum (l.take k))) :
    positions.liquidation.math.sum_repaid_usd env l ⦃ r => r.val = usdSum l ⦄ := by
  unfold positions.liquidation.math.sum_repaid_usd
  simp only [aeneas_soroban_shim.Vec.iter, bind_ok]
  have hz : (math.fp.Wad.ZERO : Std.I128).val = 0 := by
    unfold math.fp.Wad.ZERO; rfl
  have := sum_repaid_usd_loop_spec env l math.fp.Wad.ZERO (by
    intro k hk; rw [hz, zero_add]; exact hfit k hk)
  apply WP.spec_mono this
  intro r hr; rw [hr, hz, zero_add]

/-- Prefix sums of non-negative values are bounded by the total. -/
theorem usdSum_take_bounds (l : List types.RepayEntry) (hnn : ∀ e ∈ l, 0 ≤ e.usd_wad.val) :
    ∀ k, 0 ≤ usdSum (l.take k) ∧ usdSum (l.take k) ≤ usdSum l := by
  induction l with
  | nil => intro k; simp [usdSum]
  | cons e l ih =>
    intro k
    have he := hnn e (by simp)
    have ih' := ih (fun x hx => hnn x (by simp [hx]))
    cases k with
    | zero =>
      have := ih' 0
      simp only [List.take, usdSum_cons]
      simp [usdSum] at this ⊢; linarith
    | succ k =>
      have := ih' k
      simp only [List.take_succ_cons, usdSum_cons]
      constructor <;> linarith [this.1, this.2]

/-- **Production form.** Every recorded leg value is non-negative (each is a
floor or half-up USD valuation of a non-negative amount at a non-negative
price, see `calculate_repayment_amounts` and `process_excess_payment`), so the
only condition is that the total fits `i128`: then `sum_repaid_usd` returns
exactly `Σ usd_wad`, which is non-negative. -/
theorem sum_repaid_usd_nonneg (env : Env) (l : List types.RepayEntry)
    (hnn : ∀ e ∈ l, 0 ≤ e.usd_wad.val) (htot : usdSum l < 2^127) :
    positions.liquidation.math.sum_repaid_usd env l ⦃ r => r.val = usdSum l ∧ 0 ≤ r.val ⦄ := by
  have hb := usdSum_take_bounds l hnn
  have := sum_repaid_usd_spec env l (fun k _ => ⟨by linarith [(hb k).1], by linarith [(hb k).2]⟩)
  apply WP.spec_mono this
  intro r hr
  exact ⟨hr, by rw [hr]; simpa using (hb l.length).1⟩

end ControllerAeneas.Proofs
