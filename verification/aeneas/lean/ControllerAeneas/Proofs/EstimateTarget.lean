/-
`estimate_liquidation_amount`, continued: the "curve HF target" property
(the quote brings the health factor to the target within the code's
rounding, or repays everything, or exhausts the collateral), the full-close
branch, and the unbounded generalisation of the Certora rules
`ideal_repayment_targets_curve_hf` and `estimate_leaves_no_sub_threshold_dust`.

A plain (non-`module`) file, like `Basic.lean`.
-/
import ControllerAeneas.Proofs.Estimate

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- The quote `r` hits the target health factor `T` within rounding, with the
seizure rate `δ = hu(p·(1 + b))` the code computes (`denom_term`): writing
`E = W·Wc − δ·r` (`W ×` the weighted collateral left after seizing
`(1 + b)·r` at proportion `δ/W`) and `G = T·(D − r)` (`W ×` target ×
remaining debt), `|2E − 2G| ≤ (T − δ) + W`, i.e. the post-liquidation
weighted collateral equals `target × remaining debt` up to
`((T − δ) + W)/(2W) ≤ 5.5` raw WAD units (`5.5·10^-18` USD). -/
def HitsTarget (D Wc p T b r : Int) : Prop :=
  ∃ δ, IsHalfUp (p * (10^18 + b * 10^14)) (10^18) δ ∧ δ < T ∧
    2 * (T * (D - r)) - (T - δ) - 10^18 < 2 * (10^18 * Wc - δ * r) ∧
    2 * (10^18 * Wc - δ * r) ≤ 2 * (T * (D - r)) + (T - δ) + 10^18

/-- The quote exhausts the collateral at bonus `b`: `r ≈ C/(1 + b)`, within
one unit below and half a unit above (`C·W − opb < r·opb ≤ C·W + opb/2`,
`opb = W + b·10^14`); both the half-up `d_max` and the floored insolvent
quote satisfy it. -/
def Exhausts (C b r : Int) : Prop :=
  C * 10^18 - (10^18 + b * 10^14) < r * (10^18 + b * 10^14) ∧
  2 * (r * (10^18 + b * 10^14)) ≤ 2 * (C * 10^18) + (10^18 + b * 10^14)

/-- The half-up `d_max = hu(C·W/opb)` exhausts the collateral. -/
theorem exhausts_of_halfUp {C b r : Int} (h : IsHalfUp (C * 10^18) (10^18 + b * 10^14) r) :
    Exhausts C b r := by
  have h1 := IsHalfUp.mul_le h
  have h2 := IsHalfUp.lt_mul h
  constructor <;> linarith

/-- The floored insolvent quote `⌊C·W/opb⌋` exhausts the collateral. -/
theorem exhausts_of_floor {C b r : Int} (hb : 0 ≤ b)
    (h : IsFloor (C * 10^18) (10^18 + b * 10^14) r) : Exhausts C b r := by
  have hpos : (0 : Int) < 10^18 + b * 10^14 := by
    have := Int.mul_nonneg hb (show (0:Int) ≤ 10^14 by norm_num); omega
  have h' := (isFloor_pos hpos).mp h
  constructor <;> linarith [h'.1, h'.2]

/-- **The target solve hits the target.** Every `liquidation_at_target` value
is the full debt, the collateral-exhausting `d_max`, or a quote that brings
the health factor to the target within rounding (`HitsTarget`). -/
theorem LiqAtTarget.target {C D Wc p T b r : Int}
    (h : LiqAtTarget C D Wc p T b r) :
    r = D ∨ Exhausts C b r ∨ HitsTarget D Wc p T b r := by
  obtain ⟨dmax, δ, τ, hdm, hδ, hτ, h1, h2⟩ := h
  have hx := exhausts_of_halfUp hdm
  by_cases c : T ≤ δ ∨ τ ≤ Wc
  · rw [h1 c]
    rcases le_total dmax D with hl | hl
    · right; left; rw [min_eq_left hl]; exact hx
    · left; exact min_eq_right hl
  · obtain ⟨q, hq, hr⟩ := h2 (by omega) (by omega)
    rw [hr]
    rcases le_total (min q dmax) D with hl | hl
    · rw [min_eq_left hl]
      rcases le_total q dmax with hl2 | hl2
      · rw [min_eq_left hl2]
        right; right
        refine ⟨δ, hδ, by omega, ?_, ?_⟩
        · have a1 := IsHalfUp.lt_mul hq
          have a2 := IsHalfUp.lt_mul hτ
          nlinarith
        · have a1 := IsHalfUp.mul_le hq
          have a2 := IsHalfUp.mul_le hτ
          nlinarith
      · rw [min_eq_right hl2]; right; left; exact hx
    · left; exact min_eq_right hl

/-- **In production the "seizure rate meets the target" branch of
`liquidation_at_target` is dead.** Under the rate cap
(`(BPS + b)·p ≤ h·BPS`, `h < T`) the rounded rate `δ` is at most `h`, so
`target_hf <= denom_term` never holds; the early return is reached only through
`target_debt <= weighted_collateral`. -/
theorem rate_branch_dead {p b h T δ : Int} (hrate : (10^4 + b) * p ≤ h * 10^4) (hhT : h < T)
    (hδ : IsHalfUp (p * (10^18 + b * 10^14)) (10^18) δ) : δ < T := by
  have : δ ≤ h := IsHalfUp.le_of (by norm_num) hδ (by nlinarith)
  omega

/-- **Curve HF target (`ideal_repayment_targets_curve_hf`, generalised).** Under
`EstDomain`, the repayment `r` and bonus `b` that `estimate_liquidation_amount`
returns satisfy one of:
* `r = D`: the whole debt is repaid (post-liquidation debt 0); this covers
  the full-close branch and every dust adjustment;
* `Exhausts C b r`: the quote seizes (within a unit) all the collateral,
  `r ≈ C/(1 + b)`: the target is not reachable with the collateral
  available (on an insolvent account the rest becomes bad debt);
* `HitsTarget`: after repaying `r` and seizing `(1 + b)·r` at the code's
  rate, the weighted collateral is `target × remaining debt` within
  `5.5·10^-18` USD, i.e. the post-liquidation health factor is the target
  up to that rounding (two-sided). -/
theorem estimate_hits_curve_target (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (hd : EstDomain snap bounds curve) :
    positions.liquidation.curve.estimate_liquidation_amount env snap bounds curve ⦃ res =>
      res.1.val = snap.total_debt.val ∨
      Exhausts snap.total_collateral.val res.2.val res.1.val ∨
      HitsTarget snap.total_debt.val snap.weighted_collateral.val snap.proportion_seized.val
        curve.target_hf.val res.2.val res.1.val ⦄ := by
  apply WP.spec_mono (estimate_liquidation_amount_spec env snap bounds curve hd)
  rintro ⟨r, b⟩ ⟨sb, -, sblo, sbhi, hA, hB⟩
  simp only at hA hB ⊢
  have dust : ∀ ideal : Int, LiqAtTarget snap.total_collateral.val snap.total_debt.val
      snap.weighted_collateral.val snap.proportion_seized.val curve.target_hf.val b.val ideal →
      DustAdj snap.total_debt.val ideal r.val →
      r.val = snap.total_debt.val ∨ Exhausts snap.total_collateral.val b.val r.val ∨
        HitsTarget snap.total_debt.val snap.weighted_collateral.val snap.proportion_seized.val
          curve.target_hf.val b.val r.val := by
    intro ideal hlat hdust
    by_cases c : 0 < snap.total_debt.val - ideal ∧ snap.total_debt.val - ideal < 5 * 10^18
    · left; exact hdust.1 c
    · rw [hdust.2 c]; exact LiqAtTarget.target hlat
  by_cases hc : snap.proportion_seized.val ≤ 0 ∨ 10^18 ≤ snap.hf.val
  · obtain ⟨hb, ideal, hlat, hdust⟩ := hA hc
    rw [← hb] at hlat
    exact dust ideal hlat hdust
  · obtain ⟨cap, -, hC, hcb, hcs⟩ := hB (by omega)
    by_cases cCD : snap.total_collateral.val < snap.total_debt.val
    · obtain ⟨hb, f, hf, hr⟩ := hC cCD
      rw [hr, hb]
      rcases le_total f snap.total_debt.val with hl | hl
      · right; left; rw [min_eq_left hl]; exact exhausts_of_floor hd.base0 hf
      · left; exact min_eq_right hl
    · by_cases ccap : cap < bounds.base.val
      · left; exact (hcb (by omega) ccap).1
      · obtain ⟨-, ideal, hlat, hdust⟩ := hcs (by omega) (by omega)
        exact dust ideal hlat hdust

/-- **Full close when no HF-preserving bonus reaches `base`.** Under
`EstDomain`, for a liquidatable solvent account (`0 < p`, `hf < W`, `C ≥ D`)
with `hf·BPS < (BPS + base)·p` (equivalently `cap < base`), the estimate is
the whole debt at bonus `max(cap, 0)`, `cap = ⌊hf·BPS/p⌋ − BPS`. -/
theorem estimate_full_close_of_low_cap (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (hd : EstDomain snap bounds curve)
    (hp : 0 < snap.proportion_seized.val) (hhf : snap.hf.val < 10^18)
    (hCD : snap.total_debt.val ≤ snap.total_collateral.val)
    (hlow : snap.hf.val * 10^4 < (10^4 + bounds.base.val) * snap.proportion_seized.val) :
    positions.liquidation.curve.estimate_liquidation_amount env snap bounds curve ⦃ res =>
      res.1.val = snap.total_debt.val ∧
      ∃ cap, IsFloor (snap.hf.val * 10^4) snap.proportion_seized.val (cap + 10^4) ∧
        res.2.val = max cap 0 ⦄ := by
  apply WP.spec_mono (estimate_liquidation_amount_spec env snap bounds curve hd)
  rintro ⟨r, b⟩ ⟨sb, -, -, -, -, hB⟩
  simp only at hB ⊢
  obtain ⟨cap, hfl, -, hcb, -⟩ := hB ⟨hp, hhf⟩
  have ⟨hc1, _⟩ := max_hf_cap_bounds hp hfl
  have hcap : cap < bounds.base.val := by
    by_contra hn
    have : (10^4 + bounds.base.val) * snap.proportion_seized.val ≤
        (10^4 + cap) * snap.proportion_seized.val :=
      mul_le_mul_of_nonneg_right (by omega) hd.p0
    linarith
  obtain ⟨hr, hb⟩ := hcb hCD hcap
  exact ⟨hr, cap, hfl, hb⟩

/-- **Certora `ideal_repayment_targets_curve_hf` and
`estimate_leaves_no_sub_threshold_dust`, unbounded.** The Certora rules build
the snapshot as `C = D`, `p = hu(Wc·W/D)`, `hf = ⌊Wc·W/D⌋` with
`0 < hf < W` and `0 < base ≤ max ≤ BPS` (and bound `D ≤ 10^6 WAD`). For every
such snapshot (any `D ≤ 10^36`) and every validated curve the estimate
returns exactly `(D, 0)`: the whole debt at zero bonus. Hence
`ideal > 0`, `ideal ≤ D`, `ideal ≤ hu(C·W/(W + 0)) + 1 = D + 1` and the
remaining debt is `0` — all four Certora assertions.

Note: this also shows the two rules never reach `liquidation_at_target` or
the dust check: with `C = D`, `p ≥ hf`, so no positive bonus preserves the
health factor and the code takes the full-close branch. -/
theorem certora_estimate_rules (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (hm0 : 0 ≤ curve.hf_for_max_bonus.val)
    (hmt : curve.hf_for_max_bonus.val < curve.target_hf.val)
    (tW : 10^18 < curve.target_hf.val) (tmax : curve.target_hf.val ≤ 10 * 10^18)
    (fac0 : 0 ≤ curve.bonus_factor.val) (fac : curve.bonus_factor.val ≤ 10^4)
    (hD : 0 < snap.total_debt.val) (hDmax : snap.total_debt.val ≤ 10^36)
    (hWc : 0 < snap.weighted_collateral.val)
    (hC : snap.total_collateral.val = snap.total_debt.val)
    (hp : IsHalfUp (snap.weighted_collateral.val * 10^18) snap.total_debt.val
      snap.proportion_seized.val)
    (hhf : IsFloor (snap.weighted_collateral.val * 10^18) snap.total_debt.val snap.hf.val)
    (hhf0 : 0 < snap.hf.val) (hhf1 : snap.hf.val < 10^18)
    (hb0 : 0 < bounds.base.val) (hbm : bounds.base.val ≤ bounds.max.val)
    (hmB : bounds.max.val ≤ 10^4) :
    positions.liquidation.curve.estimate_liquidation_amount env snap bounds curve ⦃ res =>
      res.1.val = snap.total_debt.val ∧ res.2.val = 0 ⦄ := by
  have hf' := (isFloor_pos hD).mp hhf
  have hp1 := IsHalfUp.lt_mul hp
  have hp2 := IsHalfUp.mul_le hp
  -- hf ≤ p
  have hfp : snap.hf.val ≤ snap.proportion_seized.val := by
    by_contra hn
    have : (snap.proportion_seized.val + 1) * snap.total_debt.val ≤
        snap.hf.val * snap.total_debt.val :=
      mul_le_mul_of_nonneg_right (by omega) hD.le
    nlinarith [hf'.1]
  have hp0 : 0 < snap.proportion_seized.val := by omega
  have hd : EstDomain snap bounds curve :=
    { hm0, hmt, tW, tmax, fac0, fac, base0 := hb0.le, basemax := hbm,
      maxB := by omega, maxp := fun h => absurd h (by omega), hf0 := hhf0.le, p0 := hp0.le,
      C0 := by omega, Wc0 := hWc.le, D0 := hD.le, Dmax := hDmax, cons := hf'.1 }
  have hlow : snap.hf.val * 10^4 < (10^4 + bounds.base.val) * snap.proportion_seized.val := by
    nlinarith
  apply WP.spec_mono (estimate_full_close_of_low_cap env snap bounds curve hd hp0 hhf1
    (by omega) hlow)
  rintro res ⟨hr, cap, hfl, hb⟩
  refine ⟨hr, ?_⟩
  have ⟨_, hc2⟩ := max_hf_cap_bounds hp0 hfl
  have hcap : cap ≤ 0 := by
    by_contra hn
    have : (10^4 + 1) * snap.proportion_seized.val ≤
        (10^4 + cap) * snap.proportion_seized.val :=
      mul_le_mul_of_nonneg_right (by omega) hp0.le
    have := max_hf_cap_bounds hp0 hfl
    nlinarith
  rw [hb]; exact max_eq_right hcap

end ControllerAeneas.Proofs
