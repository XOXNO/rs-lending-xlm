/-
`contracts/controller/src/positions/liquidation/curve.rs`: the bad-debt
socialisation predicate, the threshold-derived maximum bonus, and the linear
bonus curve.

Units: health factors and the seizure proportion are WAD (`W = 10^18`);
bonuses and the bonus factor are BPS (`10^4`).
-/
import ControllerAeneas.Proofs.Value
import ControllerAeneas.Proofs.Basic
import ControllerAeneas.Proofs.Wrappers

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## `is_socializable_bad_debt` -/

theorem BAD_DEBT_USD_THRESHOLD_spec :
    constants.controller.BAD_DEBT_USD_THRESHOLD ⦃ v => v.val = 5 * 10^18 ⦄ := by
  unfold constants.controller.BAD_DEBT_USD_THRESHOLD
    constants.production.shared.DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD
  have hw := WAD_val
  step as ⟨v, hv⟩
  rw [hv, hw]

/-- **`is_socializable_bad_debt` is exactly its documented predicate.** It
never aborts and returns true exactly when debt exceeds collateral and the
collateral is at most `BAD_DEBT_USD_THRESHOLD = 5 WAD` (USD 5). No hypotheses. -/
theorem is_socializable_bad_debt_spec (debt coll : math.fp.Wad) :
    positions.liquidation.curve.is_socializable_bad_debt debt coll ⦃ b =>
      (b = true ↔ (coll.val < debt.val ∧ coll.val ≤ 5 * 10^18)) ⦄ := by
  unfold positions.liquidation.curve.is_socializable_bad_debt
  rw [wad_gt]
  simp only [bind_ok]
  by_cases h : coll.val < debt.val
  · rw [show decide (coll.val < debt.val) = true from by simp [h]]
    simp only [↓reduceIte]
    step with BAD_DEBT_USD_THRESHOLD_spec as ⟨t, ht⟩
    simp [ht, h]
  · rw [show decide (coll.val < debt.val) = false from by simp [h]]
    simp [h]

/-! ## `max_bonus_for_threshold` -/

/-- **`max_bonus_for_threshold` exact result.** For a positive seizure
proportion `p` (WAD) the call returns (does not abort)
`⌊BPS·(BPS − e)/e⌋` where `e = clamp(⌈p·BPS/W⌉, 1, BPS)` is the effective
threshold in BPS; for `p ≤ 0` it returns 0. No other hypothesis (every `i128`
proportion is accepted). -/
theorem max_bonus_for_threshold_spec (env : Env) (p : math.fp.Wad) :
    positions.liquidation.curve.max_bonus_for_threshold env p ⦃ r =>
      (p.val ≤ 0 → r.val = 0) ∧
      (0 < p.val → ∃ c e, IsCeil (p.val * 10^4) (10^18) c ∧ 1 ≤ c ∧
        e = min c (10^4) ∧ IsFloor (10^4 * (10^4 - e)) e r.val) ⦄ := by
  unfold positions.liquidation.curve.max_bonus_for_threshold
  simp only [wad_le]
  have hz : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  rw [hz]
  by_cases hp : p.val ≤ 0
  · simp only [hp, decide_true, bind_ok, ↓reduceIte]
    rw [WP.spec_ok]
    have hzB : (math.fp.Bps.ZERO : Std.I128).val = 0 := by
      unfold math.fp.Bps.ZERO
      rfl
    exact ⟨fun _ => hzB, fun h => by omega⟩
  · have hp' : 0 < p.val := by omega
    simp only [hp, decide_false, Bool.false_eq_true, ↓reduceIte, math.fp.Wad.raw, bind_ok]
    have hpb := p.hBounds
    simp only [IScalarTy.numBits] at hpb
    norm_num at hpb
    obtain ⟨c, hc⟩ := isCeil_exists (p.val * 10^4) (10^18) (by norm_num)
    have hc' := (isCeil_pos (by norm_num)).mp hc
    have hc1 : 1 ≤ c := by
      by_contra h
      have : c * 10^18 ≤ 0 := by nlinarith
      nlinarith [hc'.1]
    have hcM : c < 2^127 := by nlinarith [hc'.2]
    rw [from_fraction_ceil_eq, show math.fp.Wad.ONE = constants.production.shared.WAD by unfold math.fp.Wad.ONE; rfl]
    step with mul_div_ceil_spec env p constants.production.shared.BPS
      constants.production.shared.WAD (by rw [WAD_val]; norm_num)
      ⟨c, by rw [BPS_val, WAD_val]; exact hc, by omega, hcM⟩ as ⟨i1, hi1⟩
    rw [BPS_val, WAD_val] at hi1
    have hie : i1.val = c := IsCeil.unique (by norm_num) hi1 hc
    have hB : (constants.production.shared.BPS : Std.I128).val = 10^4 := BPS_val
    norm_num at hB
    rw [show math.fp.Bps.ONE = constants.production.shared.BPS by
      unfold math.fp.Bps.ONE
      rfl]
    -- clamp(i1, 1, BPS)
    simp only [bps_from_same, bind_ok]
    simp only [core.cmp.Ord.clamp.trait_default, core.cmp.Ord.clamp.default,
      core.cmp.Ord.clamp_body, math.fp.Bps.Insts.CoreCmpPartialOrdBps,
      core.cmp.PartialOrd.le.default, core.cmp.PartialOrd.le_body,
      core.cmp.PartialOrd.lt.default, core.cmp.PartialOrd.lt_body,
      core.cmp.PartialOrd.gt.default, core.cmp.PartialOrd.gt_body,
      math.fp.Bps.Insts.CoreCmpPartialOrdBps.partial_cmp,
      math.fp.Bps.Insts.CoreCmpOrd.cmp, core.cmp.impls.OrdI128.cmp,
      bind_ok, Option.some.injEq, compare_lt_iff_lt, compare_eq_iff_eq,
      compare_gt_iff_gt, massert]
    have h1B : (1#i128 : Std.I128).val ≤ constants.production.shared.BPS.val := by
      rw [hB]; simp
    have hn1 : ¬ (i1.val < (1#i128 : Std.I128).val) := by simp; omega
    simp [hB, hn1]
    by_cases hbig : i1.val > constants.production.shared.BPS.val
    · have hn1' : ¬ i1.val < 1 := by omega
      have hbig' : 10000 < i1.val := by rw [← hB]; exact hbig
      simp [compare_lt_iff_lt, compare_gt_iff_gt, hn1']
      simp only [hbig', ↓reduceIte, bind_ok]
      unfold math.fp.Bps.checked_sub
      step with checked_sub_nonneg_spec env constants.production.shared.BPS
        constants.production.shared.BPS (by rw [hB]; norm_num) (le_refl _) as ⟨d, hd⟩
      rw [show constants.production.shared.BPS.val - constants.production.shared.BPS.val = 0
        by ring] at hd
      unfold math.fp.Bps.div_floor
      step with mul_div_floor_spec env d constants.production.shared.BPS
        constants.production.shared.BPS (by rw [hB]; norm_num)
        ⟨0, by rw [hd, hB]; unfold IsFloor; norm_num, by norm_num, by norm_num⟩ as ⟨q, hq⟩
      intro _
      refine ⟨c, hc, hc1, ?_⟩
      have hmin : min c 10000 = 10000 := by rw [← hie]; rw [hB] at hbig; omega
      rw [hmin]
      simpa [hd, hB] using hq
    · have hn1' : ¬ i1.val < 1 := by omega
      have hle : i1.val ≤ 10^4 := by rw [hB] at hbig; omega
      have hbig' : ¬ 10000 < i1.val := by omega
      simp [compare_lt_iff_lt, compare_gt_iff_gt, hn1']
      simp only [hbig', ↓reduceIte, bind_ok]
      unfold math.fp.Bps.checked_sub
      step with checked_sub_nonneg_spec env constants.production.shared.BPS i1
        (by omega) (by rw [hB]; exact hle) as ⟨d, hd⟩
      have hi1pos : 0 < i1.val := by rw [hie]; omega
      obtain ⟨q, hq⟩ := isFloor_exists
        (d.val * constants.production.shared.BPS.val) i1.val hi1pos
      have hq0 : 0 ≤ q := IsFloor.nonneg_of hi1pos hq (by
        rw [hd, hB]
        nlinarith)
      have hqmax : q ≤ 10^8 := IsFloor.le_of hi1pos hq (by
        rw [hd, hB]
        nlinarith)
      unfold math.fp.Bps.div_floor
      step with mul_div_floor_spec env d constants.production.shared.BPS i1 (by omega)
        ⟨q, hq, by omega, by norm_num at hqmax ⊢; omega⟩ as ⟨r, hr⟩
      intro _
      refine ⟨c, hc, hc1, ?_⟩
      have hmin : min c 10000 = c := by rw [← hie]; exact min_eq_left hle
      rw [hmin]
      simpa [hd, hB, hie, mul_comm] using hr

/-- **The maximum bonus keeps a full seizure solvent (INV-LIQ).** For a seizure
proportion `0 < p ≤ 1 WAD` (weighted collateral over total collateral, at
most one because every liquidation threshold is at most 100%),
`max_bonus_for_threshold` returns a bonus `m` with
`0 ≤ m ≤ BPS·(BPS−1)` and `(BPS + m)·p ≤ BPS·W`, that is
`(1 + m/BPS)·p ≤ 1`: seizing collateral worth `(1 + bonus)·repaid` never
removes more weighted collateral than the debt it repays. -/
theorem max_bonus_for_threshold_bound (env : Env) (p : math.fp.Wad)
    (hp0 : 0 < p.val) (hp1 : p.val ≤ 10^18) :
    positions.liquidation.curve.max_bonus_for_threshold env p ⦃ r =>
      0 ≤ r.val ∧ r.val ≤ 10^4 * (10^4 - 1) ∧ (10^4 + r.val) * p.val ≤ 10^4 * 10^18 ⦄ := by
  apply WP.spec_mono (max_bonus_for_threshold_spec env p)
  rintro r ⟨_, h⟩
  obtain ⟨c, e, hc, hc1, he, hf⟩ := h hp0
  have hc' := (isCeil_pos (by norm_num)).mp hc
  have hcB : c ≤ 10^4 := by
    by_contra h
    have : 10^4 * 10^18 < c * 10^18 := by nlinarith
    nlinarith [hc'.2]
  have hee : e = c := by rw [he]; exact min_eq_left hcB
  subst hee
  have hf' := (isFloor_pos (by omega)).mp hf
  have hr0 : 0 ≤ r.val := IsFloor.nonneg_of (by omega) hf (by nlinarith)
  refine ⟨hr0, ?_, ?_⟩
  · have := IsFloor.le_of (by omega) hf (show 10^4 * (10^4 - e) ≤ (10^4 * (10^4 - 1)) * e by nlinarith)
    exact this
  · -- (BPS + r)·e ≤ BPS², and p·BPS ≤ e·W
    have h1 : (10^4 + r.val) * e ≤ 10^4 * 10^4 := by nlinarith [hf'.1]
    have h2 : (10^4 + r.val) * (p.val * 10^4) ≤ (10^4 + r.val) * (e * 10^18) :=
      mul_le_mul_of_nonneg_left hc'.1 (by omega)
    nlinarith

/-- **The maximum bonus is non-increasing in the seizure proportion.** For
`0 < p₁ ≤ p₂`, `max_bonus_for_threshold(p₂) ≤ max_bonus_for_threshold(p₁)`
(both return): a riskier account (higher weighted share of collateral)
never gets a larger bonus ceiling. -/
theorem max_bonus_for_threshold_antitone (env : Env) (p₁ p₂ : math.fp.Wad)
    (hp1 : 0 < p₁.val) (h12 : p₁.val ≤ p₂.val) :
    ∃ r₁ r₂, positions.liquidation.curve.max_bonus_for_threshold env p₁ = ok r₁ ∧
      positions.liquidation.curve.max_bonus_for_threshold env p₂ = ok r₂ ∧
      r₂.val ≤ r₁.val := by
  obtain ⟨r₁, e1, -, h1⟩ := spec_imp_eq_ok (max_bonus_for_threshold_spec env p₁)
  obtain ⟨r₂, e2, -, h2⟩ := spec_imp_eq_ok (max_bonus_for_threshold_spec env p₂)
  refine ⟨r₁, r₂, e1, e2, ?_⟩
  obtain ⟨c1, f1, hc1, hc11, hf1e, hf1⟩ := h1 hp1
  obtain ⟨c2, f2, hc2, hc21, hf2e, hf2⟩ := h2 (by omega)
  have hc1' := (isCeil_pos (by norm_num)).mp hc1
  have hc2' := (isCeil_pos (by norm_num)).mp hc2
  -- ceiling is monotone
  have hcc : c1 ≤ c2 := by
    by_contra hlt
    have : c2 * 10^18 ≤ (c1 - 1) * 10^18 := mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    nlinarith [hc1'.2, hc2'.1]
  have hf12 : f1 ≤ f2 := by rw [hf1e, hf2e]; exact min_le_min_right _ hcc
  have hf1p : 1 ≤ f1 := by rw [hf1e]; exact le_min hc11 (by norm_num)
  have hf2B : f2 ≤ 10^4 := by rw [hf2e]; exact min_le_right _ _
  have hf1' := (isFloor_pos (by omega)).mp hf1
  have hf2' := (isFloor_pos (by omega)).mp hf2
  by_contra h
  have h' : r₁.val + 1 ≤ r₂.val := by omega
  have a : (r₁.val + 1) * f2 ≤ 10^4 * (10^4 - f2) := by nlinarith [hf2'.1]
  have b : 10^4 * (10^4 - f1) < (r₁.val + 1) * f1 := by nlinarith [hf1'.2]
  nlinarith

/-! ## The linear bonus curve -/

/-- **`bonus_scale` exact result.** With a validated curve
(`0 ≤ hf_for_max_bonus < target ≤ 10 WAD`, see `validate_liquidation_curve`:
`0 < hf_for_max_bonus < target_hf ≤ MAX_LIQUIDATION_TARGET_HF_WAD`) and
`0 ≤ hf ≤ target`, `bonus_scale` returns (does not abort)
`min(hu((target − hf)·W / (target − hf_for_max_bonus)), W)`, where `hu` is
half-up rounding: the linear ramp from 0 at `target` to 1 WAD at
`hf_for_max_bonus`, capped at 1 WAD. -/
theorem bonus_scale_spec (env : Env) (curve : positions.liquidation.curve.LiquidationCurve)
    (hf target : math.fp.Wad)
    (hm0 : 0 ≤ curve.hf_for_max_bonus.val) (hmt : curve.hf_for_max_bonus.val < target.val)
    (ht : target.val ≤ 10 * 10^18) (hf0 : 0 ≤ hf.val) (hft : hf.val ≤ target.val) :
    positions.liquidation.curve.LiquidationCurve.bonus_scale curve env hf target ⦃ s =>
      ∃ q, IsHalfUp ((target.val - hf.val) * 10^18) (target.val - curve.hf_for_max_bonus.val) q ∧
        s.val = min q (10^18) ⦄ := by
  unfold positions.liquidation.curve.LiquidationCurve.bonus_scale
  rw [wad_le]
  simp only [bind_ok]
  rw [show decide (target.val ≤ curve.hf_for_max_bonus.val) = false from by simp; omega]
  simp only [Bool.false_eq_true, ↓reduceIte, math.fp.Wad.checked_sub]
  step with checked_sub_nonneg_spec env target hf hf0 hft as ⟨w, hw⟩
  step with checked_sub_nonneg_spec env target curve.hf_for_max_bonus hm0 hmt.le as ⟨w1, hw1⟩
  obtain ⟨q, hq⟩ := IsHalfUp.exists ((target.val - hf.val) * 10^18)
    (target.val - curve.hf_for_max_bonus.val) (by omega)
  have hqle : q ≤ (target.val - hf.val) * 10^18 :=
    IsHalfUp.le_of (by omega) hq (by nlinarith)
  unfold math.fp.Wad.div
  step with mul_div_half_up_spec env w constants.production.shared.WAD w1
    (by omega) (by rw [WAD_val]; norm_num) (by omega)
    ⟨q, by rw [WAD_val, hw, hw1]; exact hq, by nlinarith⟩ as ⟨d, hd⟩
  rw [WAD_val, hw, hw1] at hd
  have hde : d.val = q := IsHalfUp.unique (by omega) hd hq
  rw [wad_min, WP.spec_ok]
  refine ⟨q, hq, ?_⟩
  have hONE : (math.fp.Wad.ONE : Std.I128).val = 10^18 := by
    unfold math.fp.Wad.ONE; exact WAD_val
  split <;> rename_i hlt
  · rw [hONE] at hlt ⊢; rw [hde] at hlt; omega
  · rw [hONE] at hlt; rw [hde] at hlt ⊢; omega

/-- The scale is a WAD fraction in `[0, 1]`. -/
theorem bonus_scale_range {n d q : Int} (hn : 0 ≤ n) (hd : 0 < d) (hq : IsHalfUp n d q) :
    0 ≤ min q (10^18) ∧ min q (10^18) ≤ 10^18 :=
  ⟨le_min (IsHalfUp.nonneg hd hn hq) (by norm_num), min_le_right _ _⟩

/-- The exact value of the linear bonus below the target: `base` plus the
bonus range scaled by the curve position and by the bonus factor, each step
rounded half up. -/
def LinearBonus (hf target hfmax base max factor b : Int) : Prop :=
  ∃ q s i j, IsHalfUp ((target - hf) * 10^18) (target - hfmax) q ∧ s = min q (10^18) ∧
    IsHalfUp ((max - base) * s) (10^18) i ∧ IsHalfUp (i * factor) (10^4) j ∧ b = base + j

/-- **`calculate_linear_bonus_with_target` exact result and bounds.** For a
validated curve (`0 ≤ hf_for_max_bonus < target ≤ 10 WAD`,
`0 ≤ bonus_factor ≤ BPS`), a non-negative health factor and bonus bounds
`0 ≤ base ≤ max`, the call returns (does not abort) a bonus `b` with:
* `b = base` when `hf ≥ target` (healthy enough: no extra bonus);
* otherwise `b = base + hu(hu((max − base)·scale/W)·factor/BPS)` with `scale`
  the `bonus_scale` ramp (`LinearBonus`);
* in every case `base ≤ b ≤ max`.

The hypotheses are the checks `validate_liquidation_curve` enforces on every
stored curve, plus `0 ≤ base ≤ max`, which `get_account_bonus_params`
guarantees (`base = min(weighted bonus, max)`, both non-negative). -/
theorem calculate_linear_bonus_spec (env : Env) (hf : math.fp.Wad) (base max : math.fp.Bps)
    (curve : positions.liquidation.curve.LiquidationCurve) (target : math.fp.Wad)
    (hm0 : 0 ≤ curve.hf_for_max_bonus.val) (hmt : curve.hf_for_max_bonus.val < target.val)
    (ht : target.val ≤ 10 * 10^18) (hfac0 : 0 ≤ curve.bonus_factor.val)
    (hfac : curve.bonus_factor.val ≤ 10^4) (hf0 : 0 ≤ hf.val)
    (hb0 : 0 ≤ base.val) (hbm : base.val ≤ max.val) :
    positions.liquidation.curve.calculate_linear_bonus_with_target env hf base max curve target
      ⦃ b => (target.val ≤ hf.val → b.val = base.val) ∧
        (hf.val < target.val → LinearBonus hf.val target.val curve.hf_for_max_bonus.val
          base.val max.val curve.bonus_factor.val b.val) ∧
        base.val ≤ b.val ∧ b.val ≤ max.val ⦄ := by
  unfold positions.liquidation.curve.calculate_linear_bonus_with_target
  rw [wad_ge]
  simp only [bind_ok]
  by_cases hge : target.val ≤ hf.val
  · rw [show decide (target.val ≤ hf.val) = true from by simp [hge]]
    simp only [↓reduceIte, WP.spec_ok]
    simp_all
  · rw [show decide (target.val ≤ hf.val) = false from by simp [hge]]
    simp only [Bool.false_eq_true, ↓reduceIte]
    step with bonus_scale_spec env curve hf target hm0 hmt ht hf0 (by omega) as ⟨sc, q, hq, hsc⟩
    have hsr := bonus_scale_range (n := (target.val - hf.val) * 10^18) (by nlinarith) (by omega) hq
    rw [← hsc] at hsr
    unfold math.fp.Bps.checked_sub
    step with checked_sub_nonneg_spec env max base hb0 hbm as ⟨r, hr⟩
    obtain ⟨i, hi⟩ := IsHalfUp.exists (r.val * sc.val) (10^18) (by norm_num)
    have hir := IsHalfUp.le_of_ratio_le_one (by norm_num) (by omega) hsr.1 hsr.2 hi
    unfold math.fp.Bps.mul_wad
    step with mul_div_half_up_spec env r sc constants.production.shared.WAD (by omega) hsr.1
      (by rw [WAD_val]; norm_num) ⟨i, by rw [WAD_val]; exact hi, by
        have := r.hBounds; simp only [IScalarTy.numBits] at this; omega⟩ as ⟨iv, hiv⟩
    rw [WAD_val] at hiv
    have hie : iv.val = i := IsHalfUp.unique (by norm_num) hiv hi
    obtain ⟨j, hj⟩ := IsHalfUp.exists (iv.val * curve.bonus_factor.val) (10^4) (by norm_num)
    have hjr := IsHalfUp.le_of_ratio_le_one (by norm_num) (by omega) hfac0 hfac hj
    unfold math.fp.Bps.mul
    step with mul_div_half_up_spec env iv curve.bonus_factor constants.production.shared.BPS
      (by omega) hfac0 (by rw [BPS_val]; norm_num) ⟨j, by rw [BPS_val]; exact hj, by
        have := iv.hBounds; simp only [IScalarTy.numBits] at this; omega⟩ as ⟨jv, hjv⟩
    rw [BPS_val] at hjv
    have hje : jv.val = j := IsHalfUp.unique (by norm_num) hjv hj
    have hsum : base.val + jv.val ≤ max.val := by omega
    unfold math.fp.Bps.checked_add
    step with checked_add_raw_spec env base jv (by
      have hmax := max.hBounds
      simp only [IScalarTy.numBits] at hmax
      constructor <;> omega) as ⟨v, hv⟩
    refine ⟨fun h => absurd h hge, fun _ => ?_, by omega, by omega⟩
    exact ⟨q, sc.val, i, j, hq, hsc, by rw [← hr]; exact hi, by rw [← hie]; exact hj,
      by rw [hv, hje]⟩

/-- **The linear bonus is non-increasing in the health factor.** Under the
hypotheses of `calculate_linear_bonus_spec`, for `0 ≤ hf₁ ≤ hf₂` both calls
return and `bonus(hf₂) ≤ bonus(hf₁)`: a less healthy account never receives a
smaller liquidation bonus, and (with `calculate_linear_bonus_spec`) every
bonus lies in `[base, max]`, equal to `base` from the target upward. -/
theorem calculate_linear_bonus_antitone (env : Env) (hf₁ hf₂ : math.fp.Wad)
    (base max : math.fp.Bps)
    (curve : positions.liquidation.curve.LiquidationCurve) (target : math.fp.Wad)
    (hm0 : 0 ≤ curve.hf_for_max_bonus.val) (hmt : curve.hf_for_max_bonus.val < target.val)
    (ht : target.val ≤ 10 * 10^18) (hfac0 : 0 ≤ curve.bonus_factor.val)
    (hfac : curve.bonus_factor.val ≤ 10^4) (hf0 : 0 ≤ hf₁.val) (h12 : hf₁.val ≤ hf₂.val)
    (hb0 : 0 ≤ base.val) (hbm : base.val ≤ max.val) :
    ∃ b₁ b₂,
      positions.liquidation.curve.calculate_linear_bonus_with_target env hf₁ base max curve
        target = ok b₁ ∧
      positions.liquidation.curve.calculate_linear_bonus_with_target env hf₂ base max curve
        target = ok b₂ ∧
      b₂.val ≤ b₁.val := by
  obtain ⟨b₁, e1, t1, l1, lo1, _⟩ := spec_imp_eq_ok
    (calculate_linear_bonus_spec env hf₁ base max curve target hm0 hmt ht hfac0 hfac hf0 hb0 hbm)
  obtain ⟨b₂, e2, t2, l2, _, _⟩ := spec_imp_eq_ok
    (calculate_linear_bonus_spec env hf₂ base max curve target hm0 hmt ht hfac0 hfac
      (by omega) hb0 hbm)
  refine ⟨b₁, b₂, e1, e2, ?_⟩
  by_cases c2 : target.val ≤ hf₂.val
  · rw [t2 c2]; exact lo1
  · have c1 : hf₁.val < target.val := by omega
    obtain ⟨q1, s1, i1, j1, hq1, hs1, hi1, hj1, hb1⟩ := l1 c1
    obtain ⟨q2, s2, i2, j2, hq2, hs2, hi2, hj2, hb2⟩ := l2 (by omega)
    have hq : q2 ≤ q1 := IsHalfUp.mono (by omega) (by nlinarith) hq2 hq1
    have hs : s2 ≤ s1 := by rw [hs1, hs2]; exact min_le_min_right _ hq
    have hs2n : 0 ≤ s2 := by
      rw [hs2]; exact le_min (IsHalfUp.nonneg (by omega) (by nlinarith) hq2) (by norm_num)
    have hi : i2 ≤ i1 := IsHalfUp.mono (by norm_num)
      (mul_le_mul_of_nonneg_left hs (by omega)) hi2 hi1
    have hj : j2 ≤ j1 := IsHalfUp.mono (by norm_num)
      (mul_le_mul_of_nonneg_right hi hfac0) hj2 hj1
    omega

/-- **Maximum bonus at or below the max-bonus health factor.** Under the
hypotheses of `calculate_linear_bonus_spec`, with the default bonus factor
`BPS` (100%, `DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS`) and
`hf ≤ hf_for_max_bonus`, the bonus is exactly `max`. Generalises the Certora
rule `bonus_is_max_below_curve_floor` from the default curve to every
validated curve and every `0 ≤ base ≤ max`. -/
theorem calculate_linear_bonus_max_below_floor (env : Env) (hf : math.fp.Wad)
    (base max : math.fp.Bps)
    (curve : positions.liquidation.curve.LiquidationCurve) (target : math.fp.Wad)
    (hm0 : 0 ≤ curve.hf_for_max_bonus.val) (hmt : curve.hf_for_max_bonus.val < target.val)
    (ht : target.val ≤ 10 * 10^18) (hfac : curve.bonus_factor.val = 10^4)
    (hf0 : 0 ≤ hf.val) (hfm : hf.val ≤ curve.hf_for_max_bonus.val)
    (hb0 : 0 ≤ base.val) (hbm : base.val ≤ max.val) :
    positions.liquidation.curve.calculate_linear_bonus_with_target env hf base max curve target
      ⦃ b => b.val = max.val ⦄ := by
  apply WP.spec_mono (calculate_linear_bonus_spec env hf base max curve target hm0 hmt ht
    (by omega) (by omega) hf0 hb0 hbm)
  rintro b ⟨-, hl, -, -⟩
  obtain ⟨q, s, i, j, hq, hs, hi, hj, hb⟩ := hl (by omega)
  have hqW : 10^18 ≤ q := IsHalfUp.ge_of (by omega) hq (by nlinarith)
  have hsW : s = 10^18 := by rw [hs]; exact min_eq_right hqW
  rw [hsW] at hi
  have hie : i = max.val - base.val :=
    IsHalfUp.unique (by norm_num) hi (IsHalfUp.mul_self _ _ (by norm_num))
  rw [hie, hfac] at hj
  have hje : j = max.val - base.val :=
    IsHalfUp.unique (by norm_num) hj (IsHalfUp.mul_self _ _ (by norm_num))
  omega

end ControllerAeneas.Proofs
