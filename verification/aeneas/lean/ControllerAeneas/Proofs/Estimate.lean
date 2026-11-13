/-
`contracts/controller/src/positions/liquidation/curve.rs::estimate_liquidation_amount`:
exact result, total correctness (no abort) on the production domain, and the
consequences the liquidation plan relies on (repayment at most the debt, no
sub-threshold dust, HF-preserving bonus).

Notation: `C` total collateral, `D` total debt, `Wc` weighted collateral, `p`
seizure proportion, `hf` health factor, `T` target health factor (all WAD,
`W = 10^18`); `base`, `max`, bonuses in BPS (`10^4`).

A plain (non-`module`) file, like `Basic.lean`.
-/
import ControllerAeneas.Proofs.EstimateBasic

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- The production domain of `estimate_liquidation_amount`
(see `docs/controller-estimate.md` for why each field holds at the only call
site, `normalize_repayment_plan` in `build_liquidation_plan`):
* the curve passed `validate_liquidation_curve`
  (`0 < hf_for_max_bonus < target`, `W < target ≤ 10 W`, `0 < factor ≤ BPS`;
  only the weaker forms below are needed);
* `0 ≤ base ≤ max`, `max ≤ BPS·(BPS−1)` and `(BPS + max)·p ≤ BPS·W`: what
  `get_account_bonus_params` produces (`max = max_bonus_for_threshold(p)`,
  `max_bonus_for_threshold_bound`; `base = min(weighted bonus, max)`). The
  last one is only needed when `hf ≥ 1 WAD`, a case the call site excludes;
* non-negative snapshot values, `D ≤ 10^36` (USD 10^18);
* the snapshot is consistent: `hf·D ≤ Wc·W` (production `hf` is
  `⌊Wc·W/D⌋`, computed by `div_floor_saturating` in `risk/totals.rs`). -/
structure EstDomain (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve) : Prop where
  hm0 : 0 ≤ curve.hf_for_max_bonus.val
  hmt : curve.hf_for_max_bonus.val < curve.target_hf.val
  tW : 10^18 < curve.target_hf.val
  tmax : curve.target_hf.val ≤ 10 * 10^18
  fac0 : 0 ≤ curve.bonus_factor.val
  fac : curve.bonus_factor.val ≤ 10^4
  base0 : 0 ≤ bounds.base.val
  basemax : bounds.base.val ≤ bounds.max.val
  maxB : bounds.max.val ≤ 10^4 * (10^4 - 1)
  maxp : 10^18 ≤ snap.hf.val →
    (10^4 + bounds.max.val) * snap.proportion_seized.val ≤ 10^4 * 10^18
  hf0 : 0 ≤ snap.hf.val
  p0 : 0 ≤ snap.proportion_seized.val
  C0 : 0 ≤ snap.total_collateral.val
  Wc0 : 0 ≤ snap.weighted_collateral.val
  D0 : 0 ≤ snap.total_debt.val
  Dmax : snap.total_debt.val ≤ 10^36
  cons : snap.hf.val * snap.total_debt.val ≤ snap.weighted_collateral.val * 10^18

/-- The dust rule: a quote `ideal` leaving positive debt below
`BAD_DEBT_USD_THRESHOLD = 5 WAD` becomes the whole debt `D`. -/
def DustAdj (D ideal r : Int) : Prop :=
  ((0 < D - ideal ∧ D - ideal < 5 * 10^18) → r = D) ∧
  (¬ (0 < D - ideal ∧ D - ideal < 5 * 10^18) → r = ideal)

/-- The exact result `(r, b)` of `estimate_liquidation_amount`, given the curve
bonus `sb` (`calculate_linear_bonus_with_target` at the target HF):
* `p ≤ 0` or `hf ≥ W` (no HF-preserving cap): `b = sb`, `r` = the dust-adjusted
  `liquidation_at_target` quote at `sb`;
* otherwise, with `cap = ⌊hf·BPS/p⌋ − BPS`:
  * `C < D` (insolvent): `b = base`, `r = min(⌊C·W/(W + base·10^14)⌋, D)`;
  * `C ≥ D`, `cap < base`: `r = D`, `b = max(cap, 0)`;
  * `C ≥ D`, `cap ≥ base`: `b = min(sb, cap)`, `r` = the dust-adjusted
    `liquidation_at_target` quote at `b`. -/
def EstimateSpec (C D Wc p hf T base sb r b : Int) : Prop :=
  ((p ≤ 0 ∨ 10^18 ≤ hf) →
      b = sb ∧ ∃ ideal, LiqAtTarget C D Wc p T sb ideal ∧ DustAdj D ideal r) ∧
  ((0 < p ∧ hf < 10^18) → ∃ cap, IsFloor (hf * 10^4) p (cap + 10^4) ∧
    (C < D → b = base ∧ ∃ f, IsFloor (C * 10^18) (10^18 + base * 10^14) f ∧ r = min f D) ∧
    (D ≤ C → cap < base → r = D ∧ b = max cap 0) ∧
    (D ≤ C → base ≤ cap → b = min sb cap ∧
      ∃ ideal, LiqAtTarget C D Wc p T b ideal ∧ DustAdj D ideal r))

/-- The dust check at the end of `estimate_liquidation_amount`. -/
theorem dust_tail_spec (env : Env) (D ideal : math.fp.Wad) (b : math.fp.Bps)
    (h0 : 0 ≤ ideal.val) (hD : ideal.val ≤ D.val) :
    (do
      let remaining_debt ← math.fp.Wad.checked_sub D env ideal
      let c ← core.cmp.PartialOrd.gt.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad
        remaining_debt math.fp.Wad.ZERO
      if c then
        let i ← constants.controller.BAD_DEBT_USD_THRESHOLD
        let w ← math.fp.Wad.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) i
        let c1 ← core.cmp.PartialOrd.lt.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad
          remaining_debt w
        if c1 then ok (D, b) else ok (ideal, b)
      else ok (ideal, b)) ⦃ res => res.2 = b ∧ DustAdj D.val ideal.val res.1.val ⦄ := by
  unfold math.fp.Wad.checked_sub
  step with checked_sub_nonneg_spec env D ideal h0 hD as ⟨rem, hrem⟩
  simp only [wad_gt, est_WAD_ZERO_val, bind_ok]
  by_cases c : 0 < rem.val
  · simp only [c, decide_true, ↓reduceIte]
    step with BAD_DEBT_USD_THRESHOLD_spec as ⟨t, ht⟩
    simp only [wad_from_same, wad_lt, bind_ok, ht]
    by_cases c1 : rem.val < 5 * 10^18
    · simp only [c1, decide_true, ↓reduceIte, WP.spec_ok]
      exact ⟨trivial, fun _ => rfl, fun h => absurd ⟨by omega, by omega⟩ h⟩
    · simp only [c1, decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
      exact ⟨trivial, fun h => absurd h.2 (by omega), fun _ => rfl⟩
  · simp only [c, decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
    exact ⟨trivial, fun h => absurd h.1 (by omega), fun _ => rfl⟩

/-- **`estimate_liquidation_amount` never aborts on the production domain and
returns exactly `EstimateSpec`.** Under `EstDomain` (validated curve,
`0 ≤ base ≤ max ≤ max_bonus_for_threshold(p)`, non-negative snapshot,
`D ≤ 10^36`, `hf·D ≤ Wc·W`) the call returns `(r, b)`, the curve bonus `sb`
returns, and `(r, b)` is the branch value described by `EstimateSpec`.
No bound on `hf` is needed (`hf ≥ 1 WAD` is excluded at the call site, but is
also covered). -/
theorem estimate_liquidation_amount_spec (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (hd : EstDomain snap bounds curve) :
    positions.liquidation.curve.estimate_liquidation_amount env snap bounds curve ⦃ res =>
      ∃ sb : math.fp.Bps,
        positions.liquidation.curve.calculate_linear_bonus_with_target env snap.hf bounds.base
          bounds.max curve curve.target_hf = ok sb ∧
        bounds.base.val ≤ sb.val ∧ sb.val ≤ bounds.max.val ∧
        EstimateSpec snap.total_collateral.val snap.total_debt.val snap.weighted_collateral.val
          snap.proportion_seized.val snap.hf.val curve.target_hf.val bounds.base.val sb.val
          res.1.val res.2.val ⦄ := by
  obtain ⟨hm0, hmt, tW, tmax, fac0, fac, base0, basemax, maxB, maxp, hf0, p0, C0, Wc0, D0,
    Dmax, cons⟩ := hd
  unfold positions.liquidation.curve.estimate_liquidation_amount
  have hsb := calculate_linear_bonus_spec env snap.hf bounds.base bounds.max curve
    curve.target_hf hm0 hmt tmax fac0 fac hf0 base0 basemax
  obtain ⟨sb, hsbeq, -, -, sblo, sbhi⟩ := spec_imp_eq_ok hsb
  rw [hsbeq, bind_ok]
  step with max_hf_preserving_bonus_bps_spec snap hf0 as ⟨o, hnone, hsome⟩
  cases o with
  | none =>
    have hc : snap.proportion_seized.val ≤ 0 ∨ 10^18 ≤ snap.hf.val := by
      by_contra hn
      obtain ⟨cap, hcap, -⟩ := hsome (by omega) (by omega); cases hcap
    simp only
    -- the rate-cap witness
    have hw : ∃ h, 0 ≤ h ∧ h < curve.target_hf.val ∧
        (10^4 + sb.val) * snap.proportion_seized.val ≤ h * 10^4 ∧
        h * snap.total_debt.val ≤ snap.weighted_collateral.val * 10^18 := by
      rcases hc with hc | hc
      · have hp : snap.proportion_seized.val = 0 := by omega
        exact ⟨0, le_refl _, by omega, by rw [hp]; simp, by nlinarith⟩
      · refine ⟨10^18, by norm_num, tW, ?_, ?_⟩
        · have : (10^4 + sb.val) * snap.proportion_seized.val ≤
              (10^4 + bounds.max.val) * snap.proportion_seized.val :=
            mul_le_mul_of_nonneg_right (by omega) p0
          have := maxp hc
          linarith
        · have : 10^18 * snap.total_debt.val ≤ snap.hf.val * snap.total_debt.val :=
            mul_le_mul_of_nonneg_right hc D0
          linarith
    obtain ⟨h, hh0, hhT, hrate, hcons⟩ := hw
    step with liquidation_at_target_spec env snap sb curve.target_hf h (by omega) (by omega)
      C0 D0 Dmax Wc0 p0 (by omega) tmax hh0 hhT hrate hcons as ⟨ideal, hlat, hi0, hiD, _⟩
    apply WP.spec_mono (dust_tail_spec env snap.total_debt ideal sb hi0 hiD)
    rintro ⟨r, b⟩ ⟨hb, hdust⟩
    simp only at hb hdust
    refine ⟨sblo, sbhi, fun _ => ⟨by rw [hb], ideal.val, hlat, hdust⟩, fun h' => ?_⟩
    rcases hc with hc | hc <;> omega
  | some cap =>
    have hc : 0 < snap.proportion_seized.val ∧ snap.hf.val < 10^18 := by
      by_contra hn
      have : snap.proportion_seized.val ≤ 0 ∨ 10^18 ≤ snap.hf.val := by omega
      cases hnone this
    obtain ⟨cap', hcap', hfl⟩ := hsome hc.1 hc.2
    cases hcap'
    have ⟨hcb1, hcb2⟩ := max_hf_cap_bounds hc.1 hfl
    rw [wad_lt]; simp only [bind_ok]
    by_cases cCD : snap.total_collateral.val < snap.total_debt.val
    · simp only [cCD, decide_true, ↓reduceIte]
      step with bps_to_wad_spec env bounds.base base0 (by omega) as ⟨w, hw⟩
      unfold math.fp.Wad.checked_add
      step with checked_add_raw_spec env math.fp.Wad.ONE w
        (by rw [est_WAD_ONE_val, hw]; constructor <;> nlinarith) as ⟨opb, hopb⟩
      rw [est_WAD_ONE_val, hw] at hopb
      obtain ⟨f, hf⟩ := isFloor_exists (snap.total_collateral.val * 10^18) opb.val (by omega)
      have hf' := (isFloor_pos (by omega)).mp hf
      have hfC : f ≤ snap.total_collateral.val := IsFloor.le_of (by omega) hf (by nlinarith)
      have hf0 : 0 ≤ f := IsFloor.nonneg_of (by omega) hf (by nlinarith)
      unfold math.fp.Wad.div_floor
      step with mul_div_floor_spec env snap.total_collateral constants.production.shared.WAD opb
        (by omega) ⟨f, by rw [WAD_val]; exact hf, by omega, by
          have := snap.total_collateral.hBounds; simp only [IScalarTy.numBits] at this; omega⟩
        as ⟨bk, hbk⟩
      rw [WAD_val] at hbk
      have hbke : bk.val = f := IsFloor.unique (by omega) hbk hf
      simp only [wad_min, bind_ok, WP.spec_ok]
      refine ⟨sblo, sbhi, fun h' => by omega, fun _ => ⟨cap, hfl, fun _ => ⟨rfl, f,
        by rw [← hopb]; exact hf, ?_⟩, fun h' => absurd h' (by omega),
        fun h' => absurd h' (by omega)⟩⟩
      rw [est_wad_min_val, hbke]
    · simp only [cCD, decide_false, Bool.false_eq_true, ↓reduceIte, math.fp.Bps.raw, bind_ok]
      by_cases ccap : cap.val < bounds.base.val
      · have : cap < bounds.base := by simp [IScalar.lt_equiv]; omega
        simp only [this, ↓reduceIte]
        step as ⟨i1, hi1⟩
        simp only [bps_from_same, bind_ok, WP.spec_ok]
        refine ⟨sblo, sbhi, fun h' => by omega, fun _ => ⟨cap, hfl,
          fun h' => absurd h' cCD, fun _ _ => ⟨rfl, ?_⟩, fun _ h' => absurd h' (by omega)⟩⟩
        rw [hi1]; simp
      · have : ¬ (cap < bounds.base) := by simp [IScalar.lt_equiv]; omega
        simp only [this, ↓reduceIte]
        step as ⟨bn, hbn⟩
        simp only [bps_from_same, bind_ok]
        have hbnv : bn.val = min sb.val cap.val := by rw [hbn]; simp
        have hrate : (10^4 + bn.val) * snap.proportion_seized.val ≤ snap.hf.val * 10^4 := by
          have : (10^4 + bn.val) * snap.proportion_seized.val ≤
              (10^4 + cap.val) * snap.proportion_seized.val :=
            mul_le_mul_of_nonneg_right (by omega) p0
          linarith
        step with liquidation_at_target_spec env snap bn curve.target_hf snap.hf.val
          (by omega) (by omega) C0 D0 Dmax Wc0 p0 (by omega) tmax hf0 (by omega) hrate cons
          as ⟨ideal, hlat, hi0, hiD, _⟩
        apply WP.spec_mono (dust_tail_spec env snap.total_debt ideal bn hi0 hiD)
        rintro ⟨r, b⟩ ⟨hb, hdust⟩
        simp only at hb hdust
        refine ⟨sblo, sbhi, fun h' => by omega, fun _ => ⟨cap, hfl,
          fun h' => absurd h' cCD, fun _ h' => absurd h' ccap,
          fun _ _ => ⟨by rw [hb, hbnv], ideal.val, by rw [hb]; exact hlat, hdust⟩⟩⟩

/-- A `liquidation_at_target` value is a repayment in `[0, D]`. -/
theorem LiqAtTarget.bounds {C D Wc p T b r : Int} (hC : 0 ≤ C) (hD : 0 ≤ D) (hb : 0 ≤ b)
    (h : LiqAtTarget C D Wc p T b r) : 0 ≤ r ∧ r ≤ D := by
  obtain ⟨dmax, δ, τ, hdm, -, -, h1, h2⟩ := h
  have hdm0 : 0 ≤ dmax := IsHalfUp.nonneg
    (by have := Int.mul_nonneg hb (show (0:Int) ≤ 10^14 by norm_num); omega)
    (Int.mul_nonneg hC (by norm_num)) hdm
  by_cases c : T ≤ δ ∨ τ ≤ Wc
  · rw [h1 c]; exact ⟨le_min hdm0 hD, min_le_right _ _⟩
  · obtain ⟨q, hq, hr⟩ := h2 (by omega) (by omega)
    have hq0 : 0 ≤ q := IsHalfUp.nonneg (by omega)
      (Int.mul_nonneg (by omega) (by norm_num)) hq
    rw [hr]; exact ⟨le_min (le_min hq0 hdm0) hD, min_le_right _ _⟩

/-- The dust rule leaves either no debt or at least `5 WAD`. -/
theorem DustAdj.no_dust {D ideal r : Int} (h0 : 0 ≤ ideal) (hD : ideal ≤ D)
    (h : DustAdj D ideal r) : 0 ≤ r ∧ r ≤ D ∧ (r = D ∨ r + 5 * 10^18 ≤ D) := by
  obtain ⟨h1, h2⟩ := h
  by_cases c : 0 < D - ideal ∧ D - ideal < 5 * 10^18
  · rw [h1 c]; omega
  · rw [h2 c]; omega

/-- **`estimate_liquidation_amount` contract (INV-LIQ-02, INV-LIQ-04).** Under
`EstDomain` the call returns `(r, b)` with:
1. `0 ≤ r ≤ D`: never more than the debt (Certora
   `ideal_repayment_targets_curve_hf`, second assertion);
2. `0 ≤ b ≤ max`: the bonus never exceeds the threshold-derived maximum;
3. **no sub-threshold dust**: unless the account is insolvent with an
   HF-preserving cap (`0 < p`, `hf < W`, `C < D`, where the remainder is bad
   debt), the repayment clears the debt or leaves at least
   `BAD_DEBT_USD_THRESHOLD = 5 WAD` (Certora
   `estimate_leaves_no_sub_threshold_dust`, unbounded);
4. **HF-preserving bonus**: for a liquidatable solvent account
   (`0 < p`, `hf < W`, `C ≥ D`) either `(BPS + b)·p ≤ hf·BPS` (seizing
   `(1 + b)·repaid` of collateral removes weighted collateral at a rate at
   most `hf`, so the health factor cannot fall, see `hf_preserved_of_rate`)
   or the quote is a full close `r = D` at bonus `0`. -/
theorem estimate_liquidation_amount_contract (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (hd : EstDomain snap bounds curve) :
    positions.liquidation.curve.estimate_liquidation_amount env snap bounds curve ⦃ res =>
      0 ≤ res.1.val ∧ res.1.val ≤ snap.total_debt.val ∧
      0 ≤ res.2.val ∧ res.2.val ≤ bounds.max.val ∧
      (¬ (0 < snap.proportion_seized.val ∧ snap.hf.val < 10^18 ∧
          snap.total_collateral.val < snap.total_debt.val) →
        res.1.val = snap.total_debt.val ∨ res.1.val + 5 * 10^18 ≤ snap.total_debt.val) ∧
      ((0 < snap.proportion_seized.val ∧ snap.hf.val < 10^18 ∧
          snap.total_debt.val ≤ snap.total_collateral.val) →
        (10^4 + res.2.val) * snap.proportion_seized.val ≤ snap.hf.val * 10^4 ∨
        (res.1.val = snap.total_debt.val ∧ res.2.val = 0)) ⦄ := by
  apply WP.spec_mono (estimate_liquidation_amount_spec env snap bounds curve hd)
  rintro ⟨r, b⟩ ⟨sb, -, sblo, sbhi, hA, hB⟩
  simp only at hA hB ⊢
  have C0 := hd.C0; have D0 := hd.D0; have b0 := hd.base0; have bm := hd.basemax
  by_cases hc : snap.proportion_seized.val ≤ 0 ∨ 10^18 ≤ snap.hf.val
  · obtain ⟨hb, ideal, hlat, hdust⟩ := hA hc
    have ⟨i0, iD⟩ := LiqAtTarget.bounds C0 D0 (by omega) hlat
    have ⟨r0, rD, rd⟩ := DustAdj.no_dust i0 iD hdust
    exact ⟨r0, rD, by omega, by omega, fun _ => rd, fun h => absurd hc (by omega)⟩
  · obtain ⟨cap, hfl, hC, hcb, hcs⟩ := hB (by omega)
    have ⟨hcb1, hcb2⟩ := max_hf_cap_bounds (by omega) hfl
    by_cases cCD : snap.total_collateral.val < snap.total_debt.val
    · obtain ⟨hb, f, hf, hr⟩ := hC cCD
      have hf0 : 0 ≤ f := IsFloor.nonneg_of (by omega) hf (by nlinarith)
      rw [hr, hb]
      exact ⟨le_min hf0 D0, min_le_right _ _, b0, bm, fun h => absurd ⟨by omega, by omega, cCD⟩ h,
        fun h => absurd h.2.2 (by omega)⟩
    · by_cases ccap : cap < bounds.base.val
      · obtain ⟨hr, hb⟩ := hcb (by omega) ccap
        rw [hr, hb]
        refine ⟨D0, le_refl _, le_max_right _ _, by omega, fun _ => Or.inl rfl, fun _ => ?_⟩
        by_cases c0 : 0 ≤ cap
        · left; rw [max_eq_left c0]; exact hcb1
        · right; exact ⟨rfl, max_eq_right (by omega)⟩
      · obtain ⟨hb, ideal, hlat, hdust⟩ := hcs (by omega) (by omega)
        have hb0 : 0 ≤ b.val := by rw [hb]; exact le_min (by omega) (by omega)
        have ⟨i0, iD⟩ := LiqAtTarget.bounds C0 D0 hb0 hlat
        have ⟨r0, rD, rd⟩ := DustAdj.no_dust i0 iD hdust
        refine ⟨r0, rD, hb0, by rw [hb]; exact le_trans (min_le_left _ _) sbhi,
          fun _ => rd, fun h => Or.inl ?_⟩
        have hp := hd.p0
        have : (10^4 + b.val) * snap.proportion_seized.val ≤
            (10^4 + cap) * snap.proportion_seized.val :=
          mul_le_mul_of_nonneg_right (by rw [hb]; have := min_le_right sb.val cap; omega) hp
        linarith

/-- **The HF-preserving rate really preserves the health factor (exact
arithmetic).** If the seizure rate satisfies `(BPS + b)·p ≤ hf·BPS` and the
snapshot is consistent (`hf·D ≤ Wc·W`), then for every repayment `0 ≤ r`
the exact post-liquidation weighted collateral
`Wc' = Wc − p·(BPS + b)·r/(BPS·W)` keeps `Wc'/(D − r) ≥ Wc/D`; division-free:
`(BPS·W·Wc − p·(BPS + b)·r)·D ≥ BPS·W·Wc·(D − r)`. This is the property
`max_hf_preserving_bonus_bps` exists for. -/
theorem hf_preserved_of_rate (Wc D p hf b r : Int) (hr : 0 ≤ r) (hp : 0 ≤ p)
    (hb : 0 ≤ 10^4 + b) (hD : 0 ≤ D)
    (hrate : (10^4 + b) * p ≤ hf * 10^4) (hcons : hf * D ≤ Wc * 10^18) :
    10^4 * 10^18 * Wc * (D - r) ≤ (10^4 * 10^18 * Wc - p * (10^4 + b) * r) * D := by
  have h1 : (10^4 + b) * p * D ≤ hf * 10^4 * D := mul_le_mul_of_nonneg_right hrate hD
  have h2 : p * (10^4 + b) * D ≤ 10^4 * 10^18 * Wc := by nlinarith
  have h3 : p * (10^4 + b) * D * r ≤ 10^4 * 10^18 * Wc * r := mul_le_mul_of_nonneg_right h2 hr
  nlinarith

end ControllerAeneas.Proofs
