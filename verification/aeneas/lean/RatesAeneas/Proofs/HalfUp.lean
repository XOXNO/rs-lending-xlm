import RatesAeneas.Proofs.MulDiv
import FixedPoint.HalfUpSpec

/-
Half-up multiply-divide: `fp_core::try_mul_div_half_up` and
`fp_core::mul_div_half_up` (common/src/math/fp_core.rs).

For non-negative `x`, `y` and positive `d`, both compute `x*y/d` rounded to
nearest with exact halves rounded up (`IsHalfUp`), on the native `i128` path
and on the widened `I256` path alike. No other input bound.

Plain (non-`module`) file: it unfolds `I128.checked_mul`/`checked_add`, which
are sealed under the module system (see WRITING-PROOFS.md).
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open rates_aeneas

set_option maxRecDepth 20000

namespace RatesAeneas.Proofs

unseal Result in
/-- A total-correctness triple pins the result: a program that satisfies
`m ⦃ P ⦄` is `ok v` for some `v` with `P v`. (The only effect of `Result` is
failure, whose weakest precondition is `False`.) Used to state abort
theorems as `¬ (m ⦃ _ => True ⦄)`. -/
theorem spec_imp_eq_ok {α : Type} {m : Result α} {P : α → Prop} (h : m ⦃ v => P v ⦄) :
    ∃ v, m = ok v ∧ P v := by
  unfold WP.spec at h
  exact Aeneas.Data.Coinductive.DWP.induction (θ := WP.effectWP)
    (P := fun m _ => ∃ v, (show Result α from m) = ok v ∧ P v)
    (fun v _ hv => ⟨v, rfl, hv⟩) (fun e k s hw => by cases e; exact hw.elim) h

/-- What `try_mul_div_half_up` returns on valid operands: `Some v` with
`v = half_up(n/d)`, or `None` exactly when that value does not fit `i128`. -/
def HalfUpResult (n d : Int) (o : Option Std.I128) : Prop :=
  match o with
  | some v => IsHalfUp n d v.val
  | none => ∀ q, IsHalfUp n d q → ¬ q < 2^127

/-- **`try_mul_div_half_up` is exact half-up rounding, and total.**
It never aborts. For `x < 0`, `y < 0` or `d ≤ 0` it returns `None`. For
`x, y ≥ 0` and `d > 0` it returns `Some q` with `q = x*y/d` rounded half up
whenever that `q` fits `i128`, and `None` exactly when it does not
(`HalfUpResult`). No bound on the inputs. -/
theorem try_mul_div_half_up_spec (env : Env) (x y d : Std.I128) :
    math.fp_core.try_mul_div_half_up env x y d ⦃ o =>
      ((x.val < 0 ∨ y.val < 0 ∨ d.val ≤ 0) → o = none) ∧
      ((0 ≤ x.val ∧ 0 ≤ y.val ∧ 0 < d.val) → HalfUpResult (x.val * y.val) d.val o) ⦄ := by
  unfold math.fp_core.try_mul_div_half_up
  by_cases hx : x.val < 0
  · simp [IScalar.lt_equiv, hx]
  by_cases hy : y.val < 0
  · simp [IScalar.lt_equiv, hx, hy]
  by_cases hd : d.val ≤ 0
  · simp [IScalar.lt_equiv, IScalar.le_equiv, hx, hy, hd]
  have hx0 : ¬ (x < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hy0 : ¬ (y < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hd0 : ¬ (d <= 0#i128) := by simp [IScalar.le_equiv]; omega
  simp only [hx0, hy0, hd0, ↓reduceIte]
  have hxn : 0 ≤ x.val := by omega
  have hyn : 0 ≤ y.val := by omega
  have hdp : 0 < d.val := by omega
  have hpn : 0 ≤ x.val * y.val := Int.mul_nonneg hxn hyn
  have hbounds : x.val * y.val < 2^254 := by
    have hxb := x.hBounds
    have hyb := y.hBounds
    simp only [IScalarTy.numBits] at hxb hyb
    norm_num at hxb hyb
    have h1 : x.val * y.val ≤ x.val * 2^127 := by nlinarith
    have h2 : x.val * 2^127 < 2^127 * 2^127 := by nlinarith
    have : (2:Int)^127 * 2^127 = 2^254 := by norm_num
    omega
  have hdb := d.hBounds
  simp only [IScalarTy.numBits] at hdb
  suffices key : (do
      let half ← d / 2#i128
      let o ← lift (I128.checked_mul x y)
      let o1 ← core.option.Option.and_then
        math.fp_core.try_mul_div_half_up.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128 o half
      match o1 with
      | none => do
        let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
        let i ← aeneas_soroban_shim.I256.mul x256 y256
        let i1 ← aeneas_soroban_shim.I256.from_i128 env half
        let i2 ← aeneas_soroban_shim.I256.add i i1
        let i3 ← aeneas_soroban_shim.I256.div i2 d256
        aeneas_soroban_shim.I256.to_i128 i3
      | some biased => do
        let i ← biased / d
        ok (some i)) ⦃ o => HalfUpResult (x.val * y.val) d.val o ⦄ by
    apply WP.spec_mono key
    intro o ho
    exact ⟨fun h => by omega, fun _ => ho⟩
  have hspec := isHalfUp_tdiv (x.val * y.val) d.val hpn hdp
  step as ⟨half, hhalf⟩
  have hh2 : Int.tdiv d.val 2 = d.val / 2 := Int.tdiv_eq_ediv_of_nonneg (by omega)
  rw [← hhalf] at hspec
  rw [hh2] at hhalf
  -- The widened path.
  have wide : (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let i ← aeneas_soroban_shim.I256.mul x256 y256
      let i1 ← aeneas_soroban_shim.I256.from_i128 env half
      let i2 ← aeneas_soroban_shim.I256.add i i1
      let i3 ← aeneas_soroban_shim.I256.div i2 d256
      aeneas_soroban_shim.I256.to_i128 i3) ⦃ o => HalfUpResult (x.val * y.val) d.val o ⦄ := by
    unfold math.fp_core.to_i256_operands
    step as ⟨a, ha⟩
    step as ⟨b, hb⟩
    step as ⟨c, hc⟩
    step as ⟨i, hi⟩
    · simp only [aeneas_soroban_shim.I256.InRange, ha, hb,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega
    step as ⟨h1, hh1⟩
    step as ⟨s, hs⟩
    · simp only [aeneas_soroban_shim.I256.InRange, hi, hh1, ha, hb,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega
    rw [hi, hh1, ha, hb] at hs
    have hsn : 0 ≤ x.val * y.val + half.val := by omega
    have hq0 : 0 ≤ Int.tdiv (x.val * y.val + half.val) d.val := by
      rw [Int.tdiv_eq_ediv_of_nonneg hsn]; exact Int.ediv_nonneg hsn (by omega)
    have hq1 : Int.tdiv (x.val * y.val + half.val) d.val ≤ x.val * y.val + half.val := by
      rw [Int.tdiv_eq_ediv_of_nonneg hsn]; exact Int.ediv_le_self _ hsn
    step as ⟨t, ht⟩
    · simp only [aeneas_soroban_shim.I256.InRange, hs, hc,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega
    rw [hs, hc] at ht
    step as ⟨o, ho⟩
    have htm := IScalar.tryMkOpt_eq .I128 t
    rw [← ho] at htm
    cases o with
    | none =>
      intro q hq
      have := IsHalfUp.unique hdp hq hspec
      rw [this]
      rw [ht] at htm
      simp only [IScalar.inBounds, IScalarTy.numBits] at htm
      norm_num at htm
      intro hlt
      have := htm (by omega)
      omega
    | some v =>
      obtain ⟨hv, _⟩ := htm
      show IsHalfUp _ _ v.val
      rw [hv, ht]; exact hspec
  have hmul := I128.checked_mul_bv_spec x y
  rw [show I128.checked_mul x y = core.num.checked_mul_IScalar x y from rfl]
  cases hc : core.num.checked_mul_IScalar x y with
  | none =>
    simp only [lift, bind_ok, core.option.Option.and_then]
    exact wide
  | some m =>
    rw [hc] at hmul
    obtain ⟨_, _, hm, _⟩ := hmul
    simp only [lift, bind_ok, core.option.Option.and_then,
      math.fp_core.try_mul_div_half_up.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
    have hadd := I128.checked_add_bv_spec m half
    rw [show I128.checked_add m half = core.num.checked_add_IScalar m half from rfl]
    cases ha : core.num.checked_add_IScalar m half with
    | none => exact wide
    | some b =>
      rw [ha] at hadd
      obtain ⟨_, _, hb, _⟩ := hadd
      step as ⟨q, hq⟩
      show IsHalfUp _ _ q.val
      rw [hq, hb, hm]; exact hspec

/-- **`mul_div_half_up` is exact half-up rounding.** For `x, y ≥ 0`, `d > 0`
whose half-up quotient `q` of `x*y/d` fits `i128`, `mul_div_half_up` returns
(does not abort) exactly `q`. No other bound on the inputs. -/
theorem mul_div_half_up_spec (env : Env) (x y d : Std.I128)
    (hx : 0 ≤ x.val) (hy : 0 ≤ y.val) (hd : 0 < d.val)
    (hfit : ∃ q : Int, IsHalfUp (x.val * y.val) d.val q ∧ q < 2^127) :
    math.fp_core.mul_div_half_up env x y d ⦃ q => IsHalfUp (x.val * y.val) d.val q.val ⦄ := by
  obtain ⟨qf, hqf, hlt⟩ := hfit
  unfold math.fp_core.mul_div_half_up math.fp_core.require_nonzero_divisor
  have hd0 : ¬ (d = 0#i128) := by intro h; rw [h] at hd; simp at hd
  have hx0 : x >= 0#i128 := by simp [IScalar.le_equiv]; omega
  have hy0 : y >= 0#i128 := by simp [IScalar.le_equiv]; omega
  have hdp : d > 0#i128 := by simp [IScalar.lt_equiv]; omega
  simp only [massert, hd0, hx0, hy0, hdp, not_false_eq_true, ↓reduceIte, bind_ok]
  step with try_mul_div_half_up_spec env x y d as ⟨o, _, ho⟩
  have h := ho hx hy hd
  cases o with
  | none => exact absurd hlt (h qf hqf)
  | some v => simpa [core.option.Option.unwrap_or_else, HalfUpResult] using h

/-- **When `mul_div_half_up` aborts.** It returns (does not abort) exactly when
`x ≥ 0`, `y ≥ 0`, `d > 0` and the half-up quotient of `x*y/d` fits `i128`.
So a zero or negative divisor, a negative operand, or an overflowing result
aborts the call (in production: `DivisionByZero`, the `debug_assert` /
`MathOverflow` panic). -/
theorem mul_div_half_up_returns_iff (env : Env) (x y d : Std.I128) :
    (math.fp_core.mul_div_half_up env x y d ⦃ _ => True ⦄) ↔
      (0 ≤ x.val ∧ 0 ≤ y.val ∧ 0 < d.val ∧
        ∃ q : Int, IsHalfUp (x.val * y.val) d.val q ∧ q < 2^127) := by
  constructor
  · intro h
    obtain ⟨r, hr, _⟩ := spec_imp_eq_ok h
    unfold math.fp_core.mul_div_half_up math.fp_core.require_nonzero_divisor at hr
    by_cases hd0 : d = 0#i128
    · simp [massert, hd0] at hr
    by_cases hx : x >= 0#i128
    swap; · simp [massert, hd0, hx] at hr
    by_cases hy : y >= 0#i128
    swap; · simp [massert, hd0, hx, hy] at hr
    by_cases hdp : d > 0#i128
    swap; · simp [massert, hd0, hx, hy, hdp] at hr
    simp only [massert, hd0, hx, hy, hdp, not_false_eq_true, ↓reduceIte, bind_ok] at hr
    simp only [IScalar.le_equiv, IScalar.lt_equiv, GE.ge, GT.gt] at hx hy hdp
    simp at hx hy hdp
    obtain ⟨o, ho, hP⟩ := spec_imp_eq_ok (try_mul_div_half_up_spec env x y d)
    rw [ho] at hr
    have hres := hP.2 ⟨hx, hy, hdp⟩
    refine ⟨hx, hy, hdp, ?_⟩
    cases o with
    | none => simp [core.option.Option.unwrap_or_else,
        math.fp_core.mul_div_half_up.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once] at hr
    | some v =>
      exact ⟨v.val, hres, by have := v.hBounds; simp only [IScalarTy.numBits] at this; omega⟩
  · rintro ⟨hx, hy, hd, hfit⟩
    exact WP.spec_mono (mul_div_half_up_spec env x y d hx hy hd hfit) (fun _ _ => trivial)

end RatesAeneas.Proofs
