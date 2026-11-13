/-
`contracts/controller/src/positions/liquidation/curve.rs`: the building blocks
of `estimate_liquidation_amount`:

* `Bps::to_wad` and `1 + bonus` (exact for every bonus the code sees);
* `max_hf_preserving_bonus_bps` (exact value, maximality, abort condition);
* `liquidation_at_target` (exact value, never aborts under the rate cap).

Units: amounts and health factors are WAD (`W = 10^18`), bonuses BPS (`10^4`).
Notation in the comments: `C` total collateral, `D` total debt, `Wc` weighted
collateral, `p` seizure proportion, `T` target health factor, `b` bonus.

A plain (non-`module`) file, like `Basic.lean`.
-/
import ControllerAeneas.Proofs.Curve

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## Small helpers -/

/-- `Wad::ONE` is `10^18`. -/
theorem est_WAD_ONE_val : (math.fp.Wad.ONE : Std.I128).val = 10^18 := by
  unfold math.fp.Wad.ONE; exact WAD_val

/-- `Wad::ZERO` is `0`. -/
theorem est_WAD_ZERO_val : (math.fp.Wad.ZERO : Std.I128).val = 0 := by
  unfold math.fp.Wad.ZERO; rfl

/-- The value of the `Wad` minimum as an integer minimum. -/
theorem est_wad_min_val (a b : math.fp.Wad) :
    (if b.val < a.val then b else a).val = min a.val b.val := by
  split <;> rename_i h
  · exact (min_eq_right (le_of_lt h)).symm
  · exact (min_eq_left (not_lt.mp h)).symm

/-- **`Bps::to_wad` is exact for a bonus.** For `0 ≤ b ≤ 10^8` BPS
(every bonus is at most `BPS·(BPS−1)`, see `max_bonus_for_threshold_bound`),
`to_wad` returns exactly `b·10^14` (the half-up rounding of `b·W/BPS` has no
remainder). -/
theorem bps_to_wad_spec (env : Env) (b : math.fp.Bps) (hb0 : 0 ≤ b.val) (hb : b.val ≤ 10^8) :
    math.fp.Bps.to_wad b env ⦃ w => w.val = b.val * 10^14 ⦄ := by
  unfold math.fp.Bps.to_wad
  have hx : IsHalfUp (b.val * 10^18) (10^4) (b.val * 10^14) := by
    have := IsHalfUp.mul_self (b.val * 10^14) (10^4) (by norm_num)
    rw [show b.val * 10^14 * 10^4 = b.val * 10^18 by ring] at this
    exact this
  step with mul_div_half_up_spec env b constants.production.shared.WAD
    constants.production.shared.BPS hb0 (by rw [WAD_val]; norm_num) (by rw [BPS_val]; norm_num)
    ⟨b.val * 10^14, by rw [WAD_val, BPS_val]; exact hx, by omega⟩ as ⟨w, hw⟩
  rw [WAD_val, BPS_val] at hw
  exact IsHalfUp.unique (by norm_num) hw hx

/-- `Wad::ONE + bonus.to_wad()` is exactly `W + b·10^14` (`1 + bonus` in WAD). -/
theorem one_plus_bonus_spec (env : Env) (b : math.fp.Bps) (hb0 : 0 ≤ b.val)
    (hb : b.val ≤ 10^8) :
    (do let w ← math.fp.Bps.to_wad b env
        math.fp.Wad.checked_add math.fp.Wad.ONE env w) ⦃ o => o.val = 10^18 + b.val * 10^14 ⦄ := by
  step with bps_to_wad_spec env b hb0 hb as ⟨w, hw⟩
  unfold math.fp.Wad.checked_add
  step with checked_add_raw_spec env math.fp.Wad.ONE w
    (by rw [est_WAD_ONE_val, hw]; constructor <;> nlinarith) as ⟨o, ho⟩
  rw [ho, est_WAD_ONE_val, hw]

/-! ## `max_hf_preserving_bonus_bps` -/

/-- **`max_hf_preserving_bonus_bps` exact result.** For a non-negative health
factor `hf` and any proportion `p`:
* `p ≤ 0` or `hf ≥ 1 WAD` → `None`;
* otherwise `Some cap` with `cap + BPS = ⌊hf·BPS/p⌋`.

It never aborts on this domain (the comment in the code: `hf < 10^18` keeps
`hf·BPS` below `10^22`). -/
theorem max_hf_preserving_bonus_bps_spec (snap : positions.liquidation.curve.LiquidationSnapshot)
    (hhf : 0 ≤ snap.hf.val) :
    positions.liquidation.curve.max_hf_preserving_bonus_bps snap ⦃ o =>
      ((snap.proportion_seized.val ≤ 0 ∨ 10^18 ≤ snap.hf.val) → o = none) ∧
      ((0 < snap.proportion_seized.val ∧ snap.hf.val < 10^18) →
        ∃ cap : Std.I128, o = some cap ∧
          IsFloor (snap.hf.val * 10^4) snap.proportion_seized.val (cap.val + 10^4)) ⦄ := by
  unfold positions.liquidation.curve.max_hf_preserving_bonus_bps
  simp only [math.fp.Wad.raw, bind_ok]
  by_cases hp : snap.proportion_seized.val ≤ 0
  · have : snap.proportion_seized ≤ 0#i128 := by simp [IScalar.le_equiv]; omega
    simp only [this, ↓reduceIte, WP.spec_ok]
    exact ⟨fun _ => trivial, fun h => absurd h.1 (by omega)⟩
  · have : ¬ (snap.proportion_seized ≤ 0#i128) := by simp [IScalar.le_equiv]; omega
    simp only [this, ↓reduceIte]
    by_cases hh : 10^18 ≤ snap.hf.val
    · have : snap.hf ≥ constants.production.shared.WAD := by
        simp [IScalar.le_equiv, WAD_val]; omega
      simp only [this, ↓reduceIte, WP.spec_ok]
      exact ⟨fun _ => trivial, fun h => absurd h.2 (by omega)⟩
    · have : ¬ (snap.hf ≥ constants.production.shared.WAD) := by
        simp [IScalar.le_equiv, WAD_val]; omega
      simp only [this, ↓reduceIte]
      have hW := snap.hf.hBounds
      have hP := snap.proportion_seized.hBounds
      simp only [IScalarTy.numBits] at hW hP
      step as ⟨i1, hi1⟩
      · simp only [I128.min, I128.max, I128.numBits, IScalarTy.numBits, BPS_val]; omega
      · simp only [I128.min, I128.max, I128.numBits, IScalarTy.numBits, BPS_val]; omega
      step as ⟨i2, hi2⟩
      have hn : 0 ≤ i1.val := by rw [hi1, BPS_val]; omega
      have hi2n : 0 ≤ i2.val := by rw [hi2]; exact Int.tdiv_nonneg hn (by omega)
      have hi2b := i2.hBounds
      simp only [IScalarTy.numBits] at hi2b
      step as ⟨i3, hi3⟩
      all_goals try (simp only [I128.min, I128.max, I128.numBits, IScalarTy.numBits, BPS_val]; omega)
      refine ⟨fun h => by omega, fun _ _ => ⟨i3, rfl, ?_⟩⟩
      rw [hi3, BPS_val, show i2.val - 10^4 + 10^4 = i2.val by ring, hi2, hi1, BPS_val]
      exact tdiv_isFloor_of_nonneg _ _ (by omega) (by omega)

/-- **The cap is the largest HF-preserving bonus.** When
`max_hf_preserving_bonus_bps` returns `Some cap`:
`(BPS + cap)·p ≤ hf·BPS < (BPS + cap + 1)·p`, i.e. `cap` is the largest
integer BPS bonus with `(1 + bonus)·p ≤ hf` — exactly the doc comment's
"BPS ceiling satisfying `1 + bonus <= HF / proportion_seized`". -/
theorem max_hf_cap_bounds {hf p cap : Int} (hp : 0 < p)
    (h : IsFloor (hf * 10^4) p (cap + 10^4)) :
    (10^4 + cap) * p ≤ hf * 10^4 ∧ hf * 10^4 < (10^4 + cap + 1) * p := by
  unfold IsFloor at h; rw [if_pos hp] at h
  constructor <;> nlinarith [h.1, h.2]

/-! ## `liquidation_at_target` -/

/-- The exact value of `liquidation_at_target` (`opb = W + b·10^14` is
`1 + bonus` in WAD):
* `dmax = hu(C·W/opb)` (collateral-backed repayment), `δ = hu(p·opb/W)` (the
  weighted collateral removed per unit repaid), `τ = hu(T·D/W)` (target debt);
* if `T ≤ δ` or `τ ≤ Wc`: `r = min(dmax, D)`;
* otherwise `r = min(hu((τ − Wc)·W/(T − δ)), dmax, D)`. -/
def LiqAtTarget (C D Wc p T b r : Int) : Prop :=
  ∃ dmax δ τ, IsHalfUp (C * 10^18) (10^18 + b * 10^14) dmax ∧
    IsHalfUp (p * (10^18 + b * 10^14)) (10^18) δ ∧ IsHalfUp (T * D) (10^18) τ ∧
    ((T ≤ δ ∨ τ ≤ Wc) → r = min dmax D) ∧
    (δ < T → Wc < τ → ∃ q, IsHalfUp ((τ - Wc) * 10^18) (T - δ) q ∧ r = min (min q dmax) D)

/-- **`liquidation_at_target` exact result, general form.** On the basic
domain (`0 ≤ b ≤ 10^8`, `C, Wc ≥ 0`, `0 ≤ p ≤ 10^22`, `0 ≤ D ≤ 10^36`,
`0 < T ≤ 10 WAD`) every step except the target-solve division fits; if that
quotient `q = hu((τ − Wc)·W/(T − δ))` fits `i128` whenever the branch is
taken (`hfit`), the call returns `r` with `LiqAtTarget` and
`0 ≤ r ≤ min(dmax, D)`. With `liquidation_at_target_abort` this is the exact
abort characterisation: on the basic domain the call aborts iff the
target-solve branch is taken and `q ≥ 2^127`. -/
theorem liquidation_at_target_spec_gen (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot) (bonus : math.fp.Bps)
    (T : math.fp.Wad)
    (hb0 : 0 ≤ bonus.val) (hb : bonus.val ≤ 10^8)
    (hC : 0 ≤ snap.total_collateral.val) (hD0 : 0 ≤ snap.total_debt.val)
    (hD : snap.total_debt.val ≤ 10^36) (hWc : 0 ≤ snap.weighted_collateral.val)
    (hp : 0 ≤ snap.proportion_seized.val) (hT0 : 0 < T.val) (hT : T.val ≤ 10 * 10^18)
    (hp1 : snap.proportion_seized.val ≤ 10^22)
    (hfit : ∀ δ τ q : Int,
      IsHalfUp (snap.proportion_seized.val * (10^18 + bonus.val * 10^14)) (10^18) δ →
      IsHalfUp (T.val * snap.total_debt.val) (10^18) τ → δ < T.val →
      snap.weighted_collateral.val < τ →
      IsHalfUp ((τ - snap.weighted_collateral.val) * 10^18) (T.val - δ) q → q < 2^127) :
    positions.liquidation.curve.liquidation_at_target env snap bonus T ⦃ r =>
      LiqAtTarget snap.total_collateral.val snap.total_debt.val snap.weighted_collateral.val
        snap.proportion_seized.val T.val bonus.val r.val ∧
      0 ≤ r.val ∧ r.val ≤ snap.total_debt.val ∧
      (∀ dmax, IsHalfUp (snap.total_collateral.val * 10^18) (10^18 + bonus.val * 10^14) dmax →
        r.val ≤ dmax) ⦄ := by
  unfold positions.liquidation.curve.liquidation_at_target
  -- abbreviations
  generalize hCv : snap.total_collateral.val = C at *
  generalize hDv : snap.total_debt.val = D at *
  generalize hWv : snap.weighted_collateral.val = Wc at *
  generalize hPv : snap.proportion_seized.val = p at *
  generalize hBv : bonus.val = b at *
  step with bps_to_wad_spec env bonus (by omega) (by omega) as ⟨w, hw⟩
  unfold math.fp.Wad.checked_add
  step with checked_add_raw_spec env math.fp.Wad.ONE w
    (by rw [est_WAD_ONE_val, hw]; constructor <;> nlinarith) as ⟨opb, hopb⟩
  rw [est_WAD_ONE_val, hw, hBv] at hopb
  have hopbW : 10^18 ≤ opb.val := by rw [hopb]; nlinarith
  -- d_max
  have hCb := snap.total_collateral.hBounds
  simp only [IScalarTy.numBits, hCv] at hCb
  obtain ⟨dm, hdm⟩ := IsHalfUp.exists (C * 10^18) opb.val (by omega)
  have hdmC : dm ≤ C := IsHalfUp.le_of (by omega) hdm (by nlinarith)
  have hdm0 : 0 ≤ dm := IsHalfUp.nonneg (by omega) (by nlinarith) hdm
  unfold math.fp.Wad.div
  step with mul_div_half_up_spec env snap.total_collateral constants.production.shared.WAD opb
    (by omega) (by rw [WAD_val]; norm_num) (by omega)
    ⟨dm, by rw [WAD_val, hCv]; exact hdm, by omega⟩ as ⟨dmax, hdmax⟩
  rw [WAD_val, hCv] at hdmax
  have hdme : dmax.val = dm := IsHalfUp.unique (by omega) hdmax hdm
  -- denom_term
  obtain ⟨δ, hδ⟩ := IsHalfUp.exists (p * opb.val) (10^18) (by norm_num)
  have hδh : δ ≤ p * 10^5 := IsHalfUp.le_of (by norm_num) hδ (by rw [hopb]; nlinarith)
  have hδ0 : 0 ≤ δ := IsHalfUp.nonneg (by norm_num) (by nlinarith) hδ
  unfold math.fp.Wad.mul
  step with mul_div_half_up_spec env snap.proportion_seized opb constants.production.shared.WAD
    (by omega) (by omega) (by rw [WAD_val]; norm_num)
    ⟨δ, by rw [WAD_val, hPv]; exact hδ, by omega⟩ as ⟨dt, hdt⟩
  rw [WAD_val, hPv] at hdt
  have hdte : dt.val = δ := IsHalfUp.unique (by norm_num) hdt hδ
  -- target_debt
  obtain ⟨τ, hτ⟩ := IsHalfUp.exists (T.val * D) (10^18) (by norm_num)
  have hτD : τ ≤ 10 * D := IsHalfUp.le_of (by norm_num) hτ (by nlinarith)
  have hτ0 : 0 ≤ τ := IsHalfUp.nonneg (by norm_num) (by nlinarith) hτ
  step with mul_div_half_up_spec env T snap.total_debt constants.production.shared.WAD
    (by omega) (by omega) (by rw [WAD_val]; norm_num)
    ⟨τ, by rw [WAD_val, hDv]; exact hτ, by omega⟩ as ⟨td, htd⟩
  rw [WAD_val, hDv] at htd
  have htde : td.val = τ := IsHalfUp.unique (by norm_num) htd hτ
  have hdmax_def : IsHalfUp (C * 10^18) (10^18 + b * 10^14) dmax.val := by
    rw [← hopb]; exact hdmax
  have hδdef : IsHalfUp (p * (10^18 + b * 10^14)) (10^18) δ := by rw [← hopb]; exact hδ
  have hdmaxle : ∀ d', IsHalfUp (C * 10^18) (10^18 + b * 10^14) d' → dmax.val = d' :=
    fun d' hd' => IsHalfUp.unique (by omega) hdmax_def hd'
  simp only [wad_le, bind_ok]
  by_cases c1 : T.val ≤ dt.val
  · simp only [c1, decide_true, ↓reduceIte, wad_min, WP.spec_ok]
    rw [est_wad_min_val, hDv]
    refine ⟨⟨dmax.val, δ, τ, hdmax_def, hδdef, hτ, fun _ => rfl,
      fun h1 _ => by omega⟩, by omega, min_le_right _ _,
      fun d' hd' => by rw [← hdmaxle d' hd']; exact min_le_left _ _⟩
  · simp only [c1, decide_false, Bool.false_eq_true, ↓reduceIte]
    by_cases c2 : td.val ≤ Wc
    · have c2' : td.val ≤ snap.weighted_collateral.val := by rw [hWv]; exact c2
      simp only [c2', decide_true, ↓reduceIte, wad_min, WP.spec_ok]
      rw [est_wad_min_val, hDv]
      refine ⟨⟨dmax.val, δ, τ, hdmax_def, hδdef, hτ, fun _ => rfl,
        fun _ h2 => by omega⟩, by omega, min_le_right _ _,
        fun d' hd' => by rw [← hdmaxle d' hd']; exact min_le_left _ _⟩
    · have c2' : ¬ td.val ≤ snap.weighted_collateral.val := by rw [hWv]; exact c2
      simp only [c2', decide_false, Bool.false_eq_true, ↓reduceIte]
      unfold math.fp.Wad.checked_sub
      step with checked_sub_nonneg_spec env td snap.weighted_collateral
        (by rw [hWv]; omega) (by rw [hWv]; omega) as ⟨num, hnum⟩
      step with checked_sub_nonneg_spec env T dt (by omega) (by omega) as ⟨den, hden⟩
      rw [hWv, htde] at hnum
      rw [hdte] at hden
      obtain ⟨q, hq⟩ := IsHalfUp.exists ((τ - Wc) * 10^18) (T.val - δ) (by omega)
      have hqb : q < 2^127 := hfit δ τ q hδdef hτ (by omega) (by omega) hq
      step with mul_div_half_up_spec env num constants.production.shared.WAD den
        (by omega) (by rw [WAD_val]; norm_num) (by omega)
        ⟨q, by rw [WAD_val, hnum, hden]; exact hq, by omega⟩ as ⟨qv, hqv⟩
      rw [WAD_val, hnum, hden] at hqv
      have hqe : qv.val = q := IsHalfUp.unique (by omega) hqv hq
      simp only [wad_min, bind_ok, WP.spec_ok]
      rw [est_wad_min_val, est_wad_min_val, hDv]
      refine ⟨⟨dmax.val, δ, τ, hdmax_def, hδdef, hτ,
        fun h3 => by rcases h3 with h3 | h3 <;> omega,
        fun _ _ => ⟨q, hq, by rw [hqe]⟩⟩, ?_, ?_, ?_⟩
      · have : 0 ≤ q := IsHalfUp.nonneg (by omega) (by nlinarith) hq
        simp only [hqe]; omega
      · exact min_le_right _ _
      · intro d' hd'; rw [← hdmaxle d' hd']; omega

/-- **`liquidation_at_target` exact result; it never aborts under the rate cap.**
Hypotheses: a bonus `0 ≤ b ≤ 10^8` BPS; `C, Wc, p ≥ 0`; `0 ≤ D ≤ 10^36`
(`10^18` USD); `0 < T ≤ 10 WAD`; and the **rate cap**: some `h` with
`0 ≤ h < T`, `(BPS + b)·p ≤ h·BPS` and `h·D ≤ Wc·W`. (In
`estimate_liquidation_amount`, `h` is the snapshot health factor `hf`,
`⌊Wc·W/D⌋` by construction in `risk/totals.rs`, and the cap is what
`max_hf_preserving_bonus_bps` enforces.) Then the call returns `r` with
`LiqAtTarget` and `0 ≤ r ≤ min(dmax, D)`.

The rate cap is what bounds the divisor: it gives `δ ≤ h`, so
`T − δ ≥ T − h`, while the numerator is at most `D·(T − h)/W + 1/2`; the
quotient is therefore at most `D + W/2 + 1`. -/
theorem liquidation_at_target_spec (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot) (bonus : math.fp.Bps)
    (T : math.fp.Wad) (h : Int)
    (hb0 : 0 ≤ bonus.val) (hb : bonus.val ≤ 10^8)
    (hC : 0 ≤ snap.total_collateral.val) (hD0 : 0 ≤ snap.total_debt.val)
    (hD : snap.total_debt.val ≤ 10^36) (hWc : 0 ≤ snap.weighted_collateral.val)
    (hp : 0 ≤ snap.proportion_seized.val) (hT0 : 0 < T.val) (hT : T.val ≤ 10 * 10^18)
    (hh0 : 0 ≤ h) (hhT : h < T.val)
    (hrate : (10^4 + bonus.val) * snap.proportion_seized.val ≤ h * 10^4)
    (hcons : h * snap.total_debt.val ≤ snap.weighted_collateral.val * 10^18) :
    positions.liquidation.curve.liquidation_at_target env snap bonus T ⦃ r =>
      LiqAtTarget snap.total_collateral.val snap.total_debt.val snap.weighted_collateral.val
        snap.proportion_seized.val T.val bonus.val r.val ∧
      0 ≤ r.val ∧ r.val ≤ snap.total_debt.val ∧
      (∀ dmax, IsHalfUp (snap.total_collateral.val * 10^18) (10^18 + bonus.val * 10^14) dmax →
        r.val ≤ dmax) ⦄ := by
  refine liquidation_at_target_spec_gen env snap bonus T hb0 hb hC hD0 hD hWc hp hT0 hT
    (by nlinarith) ?_
  intro δ τ q hδ hτ hδT hτW hq
  have hδh : δ ≤ h := IsHalfUp.le_of (by norm_num) hδ (by nlinarith)
  have h2τ := IsHalfUp.mul_le hτ
  have hnumb : (τ - snap.weighted_collateral.val) * 10^18 ≤
      (snap.total_debt.val + 10^18) * (T.val - δ) := by
    have e1 : snap.total_debt.val * (T.val - h) ≤ snap.total_debt.val * (T.val - δ) :=
      mul_le_mul_of_nonneg_left (by omega) hD0
    nlinarith
  have hqb : q ≤ snap.total_debt.val + 10^18 := IsHalfUp.le_of (by omega) hq hnumb
  omega

end ControllerAeneas.Proofs
