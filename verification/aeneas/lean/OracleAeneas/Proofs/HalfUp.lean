/-
`fp_core::try_mul_div_half_up` (as compiled into the oracle crate): for
non-negative `x`, `y` and positive `d` it returns `Some q` with `q` the exact
half-up quotient of `x*y/d`, or `None` exactly when that quotient does not fit
`i128`. It never aborts. Negative inputs and a non-positive divisor give `None`.
-/
import OracleAeneas.Proofs.Host
import OracleAeneas.Proofs.Spec

/- A plain (non-`module`) file: the signed checked operations of the Aeneas
library are only reachable from non-module files (see WRITING-PROOFS.md). -/

open Aeneas Aeneas.Std Result
open OracleAeneas.Spec
open oracle_aeneas

namespace OracleAeneas.Proofs

/-- The product of two `i128` values lies strictly inside the host `I256` range. -/
theorem i128_prod_bounds (x y : Std.I128) :
    -2^254 ≤ x.val * y.val ∧ x.val * y.val ≤ 2^254 := by
  have hx := x.hBounds
  have hy := y.hBounds
  simp only [IScalarTy.numBits] at hx hy
  have hax : x.val.natAbs ≤ 2^127 := by omega
  have hay : y.val.natAbs ≤ 2^127 := by omega
  have hp : (x.val * y.val).natAbs ≤ 2^254 := by
    rw [Int.natAbs_mul]
    calc x.val.natAbs * y.val.natAbs ≤ 2^127 * 2^127 := Nat.mul_le_mul hax hay
      _ = 2^254 := by norm_num
  omega

/-- **`try_mul_div_half_up` is the exact half-up quotient or `None`.** For
`x ≥ 0`, `y ≥ 0`, `d > 0` it returns (never aborts) `Some q` with
`IsHalfUp (x*y) d q`, or `None`, and `None` only when the half-up quotient is
at least `2^127` (does not fit `i128`). Both the native path and the `I256`
fallback are covered. -/
theorem try_mul_div_half_up_spec (env : aeneas_oracle_shim.Env) (x y d : Std.I128)
    (hx : 0 ≤ x.val) (hy : 0 ≤ y.val) (hd : 0 < d.val) :
    math.fp_core.try_mul_div_half_up env x y d ⦃ o =>
      match o with
      | some q => IsHalfUp (x.val * y.val) d.val q.val
      | none => ∀ q : Int, IsHalfUp (x.val * y.val) d.val q → 2^127 ≤ q ⦄ := by
  have hb := i128_prod_bounds x y
  have hdb := d.hBounds
  simp only [IScalarTy.numBits] at hdb
  have hhu := halfUp_of_ediv (x.val * y.val) d.val hd
  unfold math.fp_core.try_mul_div_half_up
  have hx' : ¬ (x < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hy' : ¬ (y < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hd' : ¬ (d ≤ 0#i128) := by simp [IScalar.le_equiv]; omega
  simp only [hx', hy', hd', ↓reduceIte]
  step as ⟨half, hhalf⟩
  have hhalf' : half.val = d.val / 2 := by
    rw [hhalf, Int.tdiv_eq_ediv_of_nonneg (by omega)]
  simp only [lift, bind_ok]
  rw [show I128.checked_mul x y = core.num.checked_mul_IScalar x y from rfl]
  have hmul := core.num.checked_mul_IScalar_bv_spec x y
  -- the widened path
  have wide : (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let i ← aeneas_oracle_shim.I256.mul x256 y256
      let i1 ← aeneas_oracle_shim.I256.from_i128 env half
      let i2 ← aeneas_oracle_shim.I256.add i i1
      let i3 ← aeneas_oracle_shim.I256.div i2 d256
      aeneas_oracle_shim.I256.to_i128 i3) ⦃ o =>
      match o with
      | some q => IsHalfUp (x.val * y.val) d.val q.val
      | none => ∀ q : Int, IsHalfUp (x.val * y.val) d.val q → 2^127 ≤ q ⦄ := by
    unfold math.fp_core.to_i256_operands
    step as ⟨a, ha⟩
    step as ⟨b, hb2⟩
    step as ⟨c, hc⟩
    step as ⟨i, hi⟩
    · simp only [aeneas_oracle_shim.I256.InRange, ha, hb2, aeneas_oracle_shim.I256.MIN_eq,
        aeneas_oracle_shim.I256.MAX_eq]; omega
    step as ⟨i1, hi1⟩
    step as ⟨i2, hi2⟩
    · simp only [aeneas_oracle_shim.I256.InRange, hi, hi1, ha, hb2, hhalf',
        aeneas_oracle_shim.I256.MIN_eq, aeneas_oracle_shim.I256.MAX_eq]; omega
    rw [hi, hi1, ha, hb2, hhalf'] at hi2
    have hnn : 0 ≤ x.val * y.val := Int.mul_nonneg hx hy
    have htd : Int.tdiv (i2 : Int) c = (x.val * y.val + d.val / 2) / d.val := by
      rw [hi2, hc, Int.tdiv_eq_ediv_of_nonneg (by omega)]
    have hqle : (x.val * y.val + d.val / 2) / d.val ≤ x.val * y.val + d.val / 2 :=
      Int.ediv_le_self _ (by omega)
    have hq0 : 0 ≤ (x.val * y.val + d.val / 2) / d.val := Int.ediv_nonneg (by omega) (by omega)
    step as ⟨i3, hi3⟩
    · simp only [aeneas_oracle_shim.I256.InRange, htd,
        aeneas_oracle_shim.I256.MIN_eq, aeneas_oracle_shim.I256.MAX_eq]; omega
    rw [htd] at hi3
    step as ⟨o, ho⟩
    have htm := IScalar.tryMkOpt_eq .I128 (i3 : Int)
    rw [← ho] at htm
    cases o with
    | none =>
      intro q hq
      have := IsHalfUp.unique hd hq hhu
      simp only [IScalar.inBounds, IScalarTy.numBits] at htm
      rw [this]; rw [hi3] at htm; omega
    | some v =>
      obtain ⟨hv, _⟩ := htm
      show IsHalfUp (x.val * y.val) d.val v.val
      rw [hv, hi3]; exact hhu
  cases hc : core.num.checked_mul_IScalar x y with
  | none =>
    simp only [core.option.Option.and_then, bind_ok]
    exact wide
  | some m =>
    rw [hc] at hmul
    obtain ⟨_, _, hm, _⟩ := hmul
    simp only [core.option.Option.and_then,
      math.fp_core.try_mul_div_half_up.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
    rw [show I128.checked_add m half = core.num.checked_add_IScalar m half from rfl]
    have hadd := core.num.checked_add_IScalar_bv_spec m half
    cases ha : core.num.checked_add_IScalar m half with
    | none => simp only [bind_ok]; exact wide
    | some s =>
      rw [ha] at hadd
      obtain ⟨_, _, hs, _⟩ := hadd
      simp only [bind_ok]
      step as ⟨q, hq⟩
      show IsHalfUp (x.val * y.val) d.val q.val
      have hnn : 0 ≤ x.val * y.val + d.val / 2 := by
        have := Int.mul_nonneg hx hy; omega
      rw [hq, hs, hm, hhalf', Int.tdiv_eq_ediv_of_nonneg hnn]
      exact hhu

/-- `try_mul_div_half_up` returns `None` for a negative factor or a
non-positive divisor, without aborting. -/
theorem try_mul_div_half_up_rejects (env : aeneas_oracle_shim.Env) (x y d : Std.I128)
    (h : x.val < 0 ∨ y.val < 0 ∨ d.val ≤ 0) :
    math.fp_core.try_mul_div_half_up env x y d ⦃ o => o = none ⦄ := by
  unfold math.fp_core.try_mul_div_half_up
  rcases h with h | h | h
  · have : x < 0#i128 := by simp [IScalar.lt_equiv]; omega
    simp [this]
  · by_cases hx : x < 0#i128
    · simp [hx]
    · have : y < 0#i128 := by simp [IScalar.lt_equiv]; omega
      simp [hx, this]
  · by_cases hx : x < 0#i128
    · simp [hx]
    · by_cases hy : y < 0#i128
      · simp [hx, hy]
      · have : d ≤ 0#i128 := by simp [IScalar.le_equiv]; omega
        simp [hx, hy, this]

end OracleAeneas.Proofs
