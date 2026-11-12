/-
`liquidation::math::split_seized_shares`: the credit-mode split of seized
supply shares into the protocol fee and the liquidator's part.
-/
import ControllerAeneas.Proofs.Value
import ControllerAeneas.Proofs.Basic
import ControllerAeneas.Proofs.Wrappers

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- `i128::from(u32)` is the same number. -/
theorem i128_from_u32_spec (x : Std.U32) :
    I128.Insts.CoreConvertFromU32.from x ⦃ r => r.val = x.val ⦄ := by
  unfold I128.Insts.CoreConvertFromU32.from IScalar.tryMk
  have hxb := x.hBounds
  simp only [UScalarTy.numBits] at hxb
  generalize hv : (x.val : Int) = v
  have hv0 : 0 ≤ v ∧ v < 2^32 := by omega
  have h := IScalar.tryMkOpt_eq .I128 v
  cases ho : IScalar.tryMkOpt .I128 v with
  | none =>
    rw [ho] at h; exfalso; apply h
    simp only [IScalar.inBounds, IScalarTy.numBits]
    omega
  | some y =>
    rw [ho] at h
    simp [Result.ofOption, h.1]

/-- **`split_seized_shares` conserves shares and rounds the fee up.** For
seized shares `S`, bonus shares `0 ≤ b ≤ S` and a fee rate `f < BPS` (bps),
the call returns (does not abort) `(fee, liq)` with
* `fee = ⌈b·f / BPS⌉` (the protocol fee rounds up, in the protocol's favour);
* `fee + liq = S` exactly: no share is created or destroyed;
* `0 ≤ fee ≤ b` and `0 ≤ liq ≤ S`: the fee never exceeds the bonus, so the
  liquidator always keeps at least the non-bonus part `S − b`.

Hypotheses: `0 ≤ b ≤ S` (a `SeizeEntry` with `0 ≤ bonus_scaled ≤
scaled_amount`, checked by `LiquidationPlan::validate`) and `f < BPS` (the
position's stamped `liquidation_fees`, validated below 100%). The two internal
re-checks (`fee ≤ S`, `fee + liq == S`) are therefore dead code: they can
never fire. -/
theorem split_seized_shares_spec (env : Env) (S b : math.fp.Ray) (f : Std.U32)
    (hb0 : 0 ≤ b.val) (hbS : b.val ≤ S.val) (hf : (f.val : Int) < 10^4) :
    positions.liquidation.math.split_seized_shares env S b f ⦃ r =>
      IsCeil (b.val * f.val) (10^4) r.1.val ∧
      r.1.val + r.2.val = S.val ∧
      0 ≤ r.1.val ∧ r.1.val ≤ b.val ∧ 0 ≤ r.2.val ∧ r.2.val ≤ S.val ⦄ := by
  unfold positions.liquidation.math.split_seized_shares
  step with i128_from_u32_spec f as ⟨fees, hfees⟩
  have hz : (math.fp.Ray.ZERO : Std.I128).val = 0 := by unfold math.fp.Ray.ZERO; rfl
  simp only [ray_lt, ray_gt, hz, bind_ok]
  have hS0 : 0 ≤ S.val := by omega
  have c1 : ¬ (decide (S.val < 0) = true) := by simp; omega
  have c2 : ¬ (decide (b.val < 0) = true) := by simp; omega
  have c3 : ¬ (decide (S.val < b.val) = true) := by simp; omega
  have c4 : ¬ (fees >= constants.production.shared.BPS) := by
    simp only [ge_iff_le, IScalar.le_equiv, BPS_val, hfees]; omega
  simp only [massert, c1, c2, c3, c4, not_false_eq_true, ↓reduceIte, bind_ok]
  simp only [Bool.false_eq_true, not_false_eq_true, ↓reduceIte, bind_ok, decide_eq_true_eq]
  have hf0 : 0 ≤ (f.val : Int) := by omega
  obtain ⟨c, hc⟩ := isCeil_exists (b.val * f.val) (10^4) (by norm_num)
  have hc' := (isCeil_pos (by norm_num)).mp hc
  have hc0 : 0 ≤ c := IsCeil.ge_of (by norm_num) hc (by simpa using mul_nonneg hb0 hf0)
  have hcb : c ≤ b.val := by
    by_contra h
    have : (b.val + 1) * 10^4 ≤ c * 10^4 := mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    have : b.val * f.val ≤ b.val * 10^4 := mul_le_mul_of_nonneg_left (by omega) hb0
    nlinarith [hc'.2]
  have hbb := b.hBounds
  simp only [IScalarTy.numBits] at hbb
  try simp only [bps_from_same, bind_ok]
  rw [apply_to_ray_ceil_eq]
  step with mul_div_ceil_spec env b fees constants.production.shared.BPS
    (by rw [BPS_val]; norm_num)
    ⟨c, by rw [hfees, BPS_val]; exact hc, by omega, by omega⟩ as ⟨fee, hfee⟩
  rw [hfees, BPS_val] at hfee
  have hfe : fee.val = c := IsCeil.unique (by norm_num) hfee hc
  try simp only [ray_from_same, ray_gt, bind_ok]
  have c5 : ¬ (decide (S.val < fee.val) = true) := by simp; omega
  have c5' : ¬ (S.val < fee.val) := by omega
  simp only [massert, c5, c5', not_false_eq_true, ↓reduceIte, bind_ok, math.fp.Ray.checked_sub]
  step with checked_sub_nonneg_spec env S fee (by omega) (by omega) as ⟨liq, hliq⟩
  unfold math.fp.Ray.checked_add
  have hSb := S.hBounds
  simp only [IScalarTy.numBits] at hSb
  step with checked_add_raw_spec env fee liq (by rw [hliq]; omega) as ⟨t, ht⟩
  simp only [bind_ok, ray_ne]
  have c6 : ¬ (decide (t ≠ S) = true) := by
    simp only [decide_eq_true_eq, ne_eq, not_not]
    apply IScalar.eq_of_val_eq; rw [ht, hliq]; ring
  have c6' : ¬ (t ≠ S) := by simpa using c6
  simp only [massert, c6, c6', not_false_eq_true, ↓reduceIte, bind_ok]
  simp only [decide_false, Bool.false_eq_true, not_false_eq_true, ↓reduceIte, bind_ok]
  refine (WP.spec_ok _).mpr ?_
  dsimp only
  refine ⟨hfee, by rw [hliq]; ring, by omega, by omega, by omega, by omega⟩

end ControllerAeneas.Proofs
