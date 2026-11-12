/-
`estimate_liquidation_amount` / `liquidation_at_target`: when they abort.

* `liquidation_at_target` aborts exactly when the target-solve quotient
  `hu((τ − Wc)·W/(T − δ))` does not fit `i128` (all other steps fit on the
  stated domain): `liquidation_at_target_abort`.
* The consistency hypothesis `hf·D ≤ Wc·W` of `EstDomain` cannot be dropped:
  with every other field of `EstDomain` satisfied, an inconsistent snapshot
  makes `estimate_liquidation_amount` abort (`estimate_aborts_inconsistent`).
  In production `hf = ⌊Wc·W/D⌋`, so this input cannot occur.

A plain (non-`module`) file, like `Basic.lean`.
-/
import ControllerAeneas.Proofs.EstimateTarget

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- `lift x` (Aeneas' wrapper for a pure call) is `ok x`. -/
theorem est_lift_ok {α : Type} (x : α) : Aeneas.Std.lift x = ok x := rfl

unseal Result in
/-- A bind that returns `ok` started with a computation that returned `ok`. -/
theorem est_bind_eq_ok_inv {α β : Type} {m : Result α} {k : α → Result β} {r : β}
    (h : Std.bind m k = ok r) : ∃ v, m = ok v := by
  cases m using Aeneas.Data.Coinductive.ITree.cases with
  | ret v => exact ⟨v, rfl⟩
  | div =>
    have h' : Std.bind (Result.div : Result α) k = ok r := h
    simp at h'
  | vis e k' =>
    simp only [Std.bind, ok] at h
    rw [show (Aeneas.Data.Coinductive.ITree.vis e k').bind k =
      Aeneas.Data.Coinductive.ITree.vis e (fun o => (k' o).bind k) from
      Aeneas.Data.Coinductive.vis_bind e k' k] at h
    exact absurd h.symm Aeneas.Data.Coinductive.not_vis_ret

/-- `mul_div_half_up` fails (returns `fail`) when the half-up quotient does not
fit `i128`, on otherwise valid operands. -/
theorem mul_div_half_up_overflow (env : Env) (x y d : Std.I128) (hx : 0 ≤ x.val)
    (hy : 0 ≤ y.val) (hd : 0 < d.val) (q : Int) (hq : IsHalfUp (x.val * y.val) d.val q)
    (hbig : 2^127 ≤ q) : ∀ r, math.fp_core.mul_div_half_up env x y d ≠ ok r := by
  intro r hr
  unfold math.fp_core.mul_div_half_up math.fp_core.require_nonzero_divisor at hr
  have hd0 : ¬ (d = 0#i128) := by intro h; rw [h] at hd; simp at hd
  have hx0 : x >= 0#i128 := by simp [IScalar.le_equiv]; omega
  have hy0 : y >= 0#i128 := by simp [IScalar.le_equiv]; omega
  have hdp : d > 0#i128 := by simp [IScalar.lt_equiv]; omega
  simp only [massert, hd0, hx0, hy0, hdp, not_false_eq_true, ↓reduceIte, bind_ok] at hr
  obtain ⟨o, ho, hP⟩ := spec_imp_eq_ok (try_mul_div_half_up_spec env x y d)
  have hres := hP.2 ⟨hx, hy, hd⟩
  rw [ho] at hr
  cases o with
  | none =>
    simp [core.option.Option.unwrap_or_else,
      math.fp_core.mul_div_half_up.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once] at hr
  | some v =>
    have hv : v.val = q := IsHalfUp.unique hd hres hq
    have := v.hBounds; simp only [IScalarTy.numBits] at this; omega

/-- **When `liquidation_at_target` aborts.** On the basic domain
(`0 ≤ b ≤ 10^8`, `C, Wc ≥ 0`, `0 ≤ p ≤ 10^22`, `0 ≤ D ≤ 10^36`,
`0 < T ≤ 10 WAD`, where every other step fits), if the target-solve branch is
taken (`δ < T`, `Wc < τ`) and its quotient `q = hu((τ − Wc)·W/(T − δ))` is at
least `2^127`, the call aborts. Together with `liquidation_at_target_spec`
(which shows `q ≤ D + W` under the rate cap) this pins the only abort. -/
theorem liquidation_at_target_abort (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot) (bonus : math.fp.Bps)
    (T : math.fp.Wad)
    (hb0 : 0 ≤ bonus.val) (hb : bonus.val ≤ 10^8)
    (hC : 0 ≤ snap.total_collateral.val) (hD0 : 0 ≤ snap.total_debt.val)
    (hD : snap.total_debt.val ≤ 10^36) (hWc : 0 ≤ snap.weighted_collateral.val)
    (hp : 0 ≤ snap.proportion_seized.val) (hp1 : snap.proportion_seized.val ≤ 10^22)
    (hT0 : 0 < T.val) (hT : T.val ≤ 10 * 10^18)
    (δ τ q : Int)
    (hδ : IsHalfUp (snap.proportion_seized.val * (10^18 + bonus.val * 10^14)) (10^18) δ)
    (hτ : IsHalfUp (T.val * snap.total_debt.val) (10^18) τ)
    (hδT : δ < T.val) (hτW : snap.weighted_collateral.val < τ)
    (hq : IsHalfUp ((τ - snap.weighted_collateral.val) * 10^18) (T.val - δ) q)
    (hbig : 2^127 ≤ q) :
    ¬ (positions.liquidation.curve.liquidation_at_target env snap bonus T ⦃ _ => True ⦄) := by
  intro hspec
  obtain ⟨r, hr, -⟩ := spec_imp_eq_ok hspec
  unfold positions.liquidation.curve.liquidation_at_target at hr
  obtain ⟨w, hw, hwv⟩ := spec_imp_eq_ok (bps_to_wad_spec env bonus hb0 hb)
  rw [hw, bind_ok] at hr
  unfold math.fp.Wad.checked_add at hr
  obtain ⟨opb, hopb, hopbv⟩ := spec_imp_eq_ok (checked_add_raw_spec env math.fp.Wad.ONE w
    (by rw [est_WAD_ONE_val, hwv]; constructor <;> nlinarith))
  rw [hopb, bind_ok, bind_ok] at hr
  rw [est_WAD_ONE_val, hwv] at hopbv
  have hCb := snap.total_collateral.hBounds
  simp only [IScalarTy.numBits] at hCb
  obtain ⟨dm, hdm⟩ := IsHalfUp.exists (snap.total_collateral.val * 10^18) opb.val (by omega)
  have hdmC : dm ≤ snap.total_collateral.val := IsHalfUp.le_of (by omega) hdm (by nlinarith)
  unfold math.fp.Wad.div math.fp.Wad.mul at hr
  obtain ⟨dmax, hdmax, -⟩ := spec_imp_eq_ok (mul_div_half_up_spec env snap.total_collateral
    constants.production.shared.WAD opb hC (by rw [WAD_val]; norm_num) (by omega)
    ⟨dm, by rw [WAD_val]; exact hdm, by omega⟩)
  rw [hdmax, bind_ok, bind_ok] at hr
  have hδ' : IsHalfUp (snap.proportion_seized.val * opb.val) (10^18) δ := by rw [hopbv]; exact hδ
  have hδb : δ ≤ snap.proportion_seized.val * 10^5 :=
    IsHalfUp.le_of (by norm_num) hδ' (by rw [hopbv]; nlinarith)
  obtain ⟨dt, hdt, hdtv⟩ := spec_imp_eq_ok (mul_div_half_up_spec env snap.proportion_seized opb
    constants.production.shared.WAD hp (by omega) (by rw [WAD_val]; norm_num)
    ⟨δ, by rw [WAD_val]; exact hδ', by omega⟩)
  rw [hdt, bind_ok, bind_ok] at hr
  rw [WAD_val] at hdtv
  have hdte : dt.val = δ := IsHalfUp.unique (by norm_num) hdtv hδ'
  have hδ0 : 0 ≤ δ := IsHalfUp.nonneg (by norm_num) (by rw [hopbv]; positivity) hδ'
  have hτb : τ ≤ 10 * snap.total_debt.val := IsHalfUp.le_of (by norm_num) hτ (by nlinarith)
  obtain ⟨td, htd, htdv⟩ := spec_imp_eq_ok (mul_div_half_up_spec env T snap.total_debt
    constants.production.shared.WAD (by omega) hD0 (by rw [WAD_val]; norm_num)
    ⟨τ, by rw [WAD_val]; exact hτ, by omega⟩)
  rw [htd, bind_ok] at hr
  rw [WAD_val] at htdv
  have htde : td.val = τ := IsHalfUp.unique (by norm_num) htdv hτ
  have c1 : ¬ T.val ≤ dt.val := by omega
  have c2 : ¬ td.val ≤ snap.weighted_collateral.val := by omega
  simp only [wad_le, bind_ok, c1, c2, decide_false, Bool.false_eq_true, ↓reduceIte] at hr
  unfold math.fp.Wad.checked_sub at hr
  obtain ⟨num, hnum, hnumv⟩ := spec_imp_eq_ok (checked_sub_nonneg_spec env td
    snap.weighted_collateral hWc (by omega))
  rw [hnum, bind_ok, bind_ok] at hr
  obtain ⟨den, hden, hdenv⟩ := spec_imp_eq_ok (checked_sub_nonneg_spec env T dt
    (by omega) (by omega))
  rw [hden, bind_ok, bind_ok] at hr
  rw [htde] at hnumv; rw [hdte] at hdenv
  have hfail := mul_div_half_up_overflow env num constants.production.shared.WAD den
    (by omega) (by rw [WAD_val]; norm_num) (by omega) q
    (by rw [WAD_val, hnumv, hdenv]; exact hq) hbig
  -- the remaining program starts with that failing call
  generalize hm : math.fp_core.mul_div_half_up env num constants.production.shared.WAD den = m
    at hr hfail
  obtain ⟨v, hv⟩ := est_bind_eq_ok_inv hr
  obtain ⟨v', hv'⟩ := est_bind_eq_ok_inv hv
  exact hfail v' hv'

/-- **The consistency hypothesis is necessary (counterexample).** Take the
validated curve `target = W + 1`, `hf_for_max_bonus = W/2`, `factor = BPS`,
bounds `base = max = 0`, and the snapshot `C = D = 10^21` (USD 1000),
`Wc = 0`, `p = hf = W − 1`. Every field of `EstDomain` holds except
`hf·D ≤ Wc·W`. Then `max_hf_preserving_bonus_bps` gives `cap = 0`, the bonus is
`0`, `δ = W − 1`, so the divisor `T − δ` is `2`, and the quotient
`hu((10^21 + 10^3)·W/2) ≈ 5·10^38` overflows `i128`:
`estimate_liquidation_amount` aborts. Production cannot build this snapshot
(`hf = ⌊Wc·W/D⌋ = 0` there). -/
theorem estimate_aborts_inconsistent (env : Env)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bounds : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (hT : curve.target_hf.val = 10^18 + 1) (hm : curve.hf_for_max_bonus.val = 5 * 10^17)
    (hfac : curve.bonus_factor.val = 10^4)
    (hbase : bounds.base.val = 0) (hmax : bounds.max.val = 0)
    (hC : snap.total_collateral.val = 10^21) (hD : snap.total_debt.val = 10^21)
    (hWc : snap.weighted_collateral.val = 0)
    (hp : snap.proportion_seized.val = 10^18 - 1) (hhf : snap.hf.val = 10^18 - 1) :
    ¬ (positions.liquidation.curve.estimate_liquidation_amount env snap bounds curve
        ⦃ _ => True ⦄) := by
  intro hspec
  obtain ⟨res, hr, -⟩ := spec_imp_eq_ok hspec
  unfold positions.liquidation.curve.estimate_liquidation_amount at hr
  obtain ⟨sb, hsbeq, -, -, sblo, sbhi⟩ := spec_imp_eq_ok
    (calculate_linear_bonus_spec env snap.hf bounds.base bounds.max curve curve.target_hf
      (by omega) (by omega) (by omega) (by omega) (by omega) (by omega) (by omega) (by omega))
  have hsb : sb.val = 0 := by omega
  rw [hsbeq, bind_ok] at hr
  obtain ⟨o, ho, -, hsome⟩ := spec_imp_eq_ok (max_hf_preserving_bonus_bps_spec snap (by omega))
  obtain ⟨cap, rfl, hfl⟩ := hsome ⟨by omega, by omega⟩
  have hcap : cap.val = 0 := by
    rw [hhf, hp] at hfl
    have := IsFloor.unique (by norm_num) hfl
      (show IsFloor ((10^18 - 1) * 10^4) (10^18 - 1) (0 + 10^4) by unfold IsFloor; norm_num)
    omega
  rw [ho, bind_ok] at hr
  simp only [wad_lt, bind_ok, math.fp.Bps.raw] at hr
  have c1 : ¬ snap.total_collateral.val < snap.total_debt.val := by omega
  have c2 : ¬ cap < bounds.base := by simp [IScalar.lt_equiv]; omega
  simp only [c1, decide_false, Bool.false_eq_true, ↓reduceIte, c2, est_lift_ok, bind_ok,
    bps_from_same] at hr
  generalize hbn : core.cmp.impls.OrdI128.min sb cap = bn at hr
  have hbnv : bn.val = 0 := by rw [← hbn]; simp [hsb, hcap]
  obtain ⟨v, hv⟩ := est_bind_eq_ok_inv hr
  refine liquidation_at_target_abort env snap bn curve.target_hf (by omega) (by omega)
    (by omega) (by omega) (by omega) (by omega) (by omega) (by omega) (by omega) (by omega)
    (10^18 - 1) (10^21 + 10^3) ((10^21 + 10^3) * (5 * 10^17)) ?_ ?_ (by omega) (by omega) ?_
    (by norm_num) (by rw [hv]; simp)
  · rw [hp, hbnv]
    have := IsHalfUp.mul_self (10^18 - 1) (10^18) (by norm_num)
    convert this using 1; ring
  · rw [hT, hD]
    have := IsHalfUp.mul_self (10^21 + 10^3) (10^18) (by norm_num)
    convert this using 1; ring
  · rw [hWc, hT]
    have := IsHalfUp.mul_self ((10^21 + 10^3) * (5 * 10^17)) 2 (by norm_num)
    convert this using 1 <;> ring

end ControllerAeneas.Proofs
