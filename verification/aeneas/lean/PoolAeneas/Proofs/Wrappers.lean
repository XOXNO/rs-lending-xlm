/-
Specs of the typed fixed-point wrappers of `common/src/math/fp.rs` that the
pool code reaches since #224 ("enforce typed fixed-point math boundaries"),
when `fp_core` became private: `Bps::apply_to_ceil` (liquidation buffer),
`Ray::try_mul` (`rates::simulate::market_value`, `guards::fits_entry_ceiling`), `Ray::mul_int`
(`rates::compound::compound_interest`), and the `i128::from(u64)` model those
callers use. `Ray::div_floor_saturating` (`protocol_fee_shares`,
`calculate_scaled_cap`) is specified next to `Ray::div_floor` in
`FloorCeilScaling.lean` (`ray_div_floor_saturating_spec`).
-/
import PoolAeneas.Proofs.Basic

/- Plain (non-`module`) file: unfolds `I128.checked_mul`, sealed under the
module system (WRITING-PROOFS.md). -/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- `BPS` is 10000. -/
theorem bps_val : (constants.shared.BPS).val = 10000 := by
  unfold constants.shared.BPS; simp

/-- `i128::from(u64)` (the trusted model in `FunsExternal.lean`) never aborts
and returns the same integer: every `u64` fits `i128`. -/
theorem i128_from_u64_spec (x : Std.U64) :
    I128.Insts.CoreConvertFromU64.from x ⦃ r => r.val = x.val ⦄ := by
  unfold I128.Insts.CoreConvertFromU64.from IScalar.tryMk
  have hxb := x.hBounds
  simp only [UScalarTy.numBits] at hxb
  generalize hv : (x.val : Int) = v
  have hv0 : 0 ≤ v ∧ v < 2^64 := by omega
  have h := IScalar.tryMkOpt_eq .I128 v
  cases ho : IScalar.tryMkOpt .I128 v with
  | none =>
    rw [ho] at h; exfalso; apply h
    simp only [IScalar.inBounds, IScalarTy.numBits]
    omega
  | some y =>
    rw [ho] at h
    simp [Result.ofOption, h.1]

/-- `Bps::from(v)` for an `i128` wraps `v` unchanged. -/
theorem bps_from_same (v : Std.I128) :
    math.fp.Bps.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) v = ok v := by
  simp [math.fp.Bps.from, core.convert.IntoFrom.into, core.convert.FromSame.from]

/-- **`Bps::apply_to_ceil` is the exact ceiling** (wrapper added in #224;
`fp_core::mul_div_ceil(amount, bps, BPS)`). Whenever `⌈amount * bps / 10000⌉`
fits `i128` it returns exactly that ceiling, for every sign. Used by
`guards::require_liquidation_buffer` with `bps = LIQUIDATION_BUFFER_BPS`. -/
theorem bps_apply_to_ceil_spec (env : Env) (k amount : Std.I128)
    (hfit : ∃ q : Int, IsCeil (amount.val * k.val) 10000 q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Bps.apply_to_ceil k env amount ⦃ r => IsCeil (amount.val * k.val) 10000 r.val ⦄ := by
  unfold math.fp.Bps.apply_to_ceil
  have h := mul_div_ceil_spec env amount k constants.shared.BPS (by rw [bps_val]; norm_num)
    (by rw [bps_val]; exact hfit)
  rw [bps_val] at h
  exact h

/-- **`Ray::try_mul` is the checked half-up product, and total**
(`fp_core::try_mul_div_half_up(a, b, RAY)` mapped into `Ray`). Never aborts.
`None` for a negative operand; for `a ≥ 0`, `b ≥ 0` it returns
`Some(⌊a*b/RAY + 1/2⌋)` when that fits `i128` and `None` exactly when it does
not (`HalfUpResult`). Reached through `rates::simulate::market_value`
(`accrue_step`) and `guards::fits_entry_ceiling`: an unrepresentable market
total holds accrual, or fails the entry check, instead of aborting. -/
theorem ray_try_mul_spec (env : Env) (a b : Std.I128) :
    math.fp.Ray.try_mul a env b ⦃ o =>
      ((a.val < 0 ∨ b.val < 0) → o = none) ∧
      ((0 ≤ a.val ∧ 0 ≤ b.val) → HalfUpResult (a.val * b.val) (10^27) o) ⦄ := by
  unfold math.fp.Ray.try_mul
  step with try_mul_div_half_up_spec env a b constants.shared.RAY as ⟨o, h1, h2⟩
  rw [ray_val] at h1 h2
  have hR : (0 : Int) < 10^27 := by norm_num
  cases o with
  | none =>
    rw [core.option.Option.map_none, WP.spec_ok]
    exact ⟨fun _ => rfl, fun ha hb => h2 ha hb hR⟩
  | some v =>
    rw [core.option.Option.map_some]
    simp only [P.Insts.CoreOpsFunctionFnOnceTupleI128Ray.call_once, math.fp.Ray.constructor]
    apply WP.spec_bind ((WP.spec_ok _).mpr (rfl : v = v))
    rintro _ rfl
    rw [WP.spec_ok]
    refine ⟨fun h => ?_, fun ha hb => h2 ha hb hR⟩
    have := h1 (by rcases h with h | h <;> simp [h])
    simp at this

/-- **`Ray::mul_int` is exact integer multiplication** (wrapper added in #224;
`checked_mul`, panicking on overflow). When `a * m` fits `i128` it returns
exactly `a * m`. Reached through `rates::compound::compound_interest`
(`rate.mul_int(i128::from(delta_ms))`). -/
theorem ray_mul_int_spec (env : Env) (a m : Std.I128)
    (hfit : -2^127 ≤ a.val * m.val ∧ a.val * m.val < 2^127) :
    math.fp.Ray.mul_int a env m ⦃ r => r.val = a.val * m.val ⦄ := by
  unfold math.fp.Ray.mul_int
  have hmul := I128.checked_mul_bv_spec a m
  rw [show I128.checked_mul a m = core.num.checked_mul_IScalar a m from rfl]
  simp only [lift, bind_ok]
  cases hc : core.num.checked_mul_IScalar a m with
  | none =>
    rw [hc] at hmul
    exfalso; apply hmul
    simp only [I128.min, I128.max, I128.numBits, IScalarTy.numBits]
    omega
  | some z =>
    rw [hc] at hmul
    simp only [core.option.Option.unwrap_or_else, bind_ok, WP.spec_ok]
    exact hmul.2.2.1

/-- **`Ray::mul_int` aborts on overflow** (the `MathOverflow` panic): when
`a * m` does not fit `i128`, it never returns. Together with
`ray_mul_int_spec` this is the exact pass condition. -/
theorem ray_mul_int_overflow (env : Env) (a m : Std.I128)
    (hov : ¬ (-2^127 ≤ a.val * m.val ∧ a.val * m.val < 2^127)) :
    ¬ ∃ r, math.fp.Ray.mul_int a env m = ok r := by
  unfold math.fp.Ray.mul_int
  have hmul := I128.checked_mul_bv_spec a m
  rw [show I128.checked_mul a m = core.num.checked_mul_IScalar a m from rfl]
  simp only [lift, bind_ok]
  cases hc : core.num.checked_mul_IScalar a m with
  | none =>
    simp [core.option.Option.unwrap_or_else,
      math.fp.Ray.mul_int.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]
  | some z =>
    rw [hc] at hmul
    exfalso; apply hov
    simp only [I128.min, I128.max, I128.numBits, IScalarTy.numBits] at hmul
    omega

/-- `Ray::from(v)` for an `i128` wraps `v` unchanged. -/
theorem ray_from_same (v : Std.I128) :
    math.fp.Ray.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) v = ok v := by
  simp [math.fp.Ray.from, core.convert.IntoFrom, core.convert.IntoFrom.into,
    core.convert.FromSame, core.convert.FromSame.from]

end PoolAeneas.Proofs
