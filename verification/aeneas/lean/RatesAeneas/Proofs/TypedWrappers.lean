/-
Specifications of the typed fixed-point wrappers introduced by #224
("enforce typed fixed-point math boundaries") in `common/src/math/fp.rs`.
`fp_core` became private; callers go through these `Ray`/`Wad`/`Bps` methods.
Each one is a single call into a proved core (`mul_div_floor`,
`mul_div_ceil`, `mul_div_floor_saturating`, `try_mul_div_half_up`,
`mul_div_half_up`, `div_by_int_half_up`) or a checked integer operation, so
each spec is the core spec with the wrapper's constant (`RAY`, `WAD`, `BPS`)
substituted.

The two wrappers the rates code calls are proved next to their users:
`ray_try_div_spec` and `ray_try_mul_spec` (`HalfUpWrappers.lean`; `accrue_step`
values the market totals with `Ray::try_mul`) and
`ray_div_floor_saturating_spec` (`FloorCeilScaling.lean`, used by
`calculate_scaled_cap`, `protocol_fee_shares`, `scaled_to_original`, and,
for non-abort, `update_supply_index`).
-/
import RatesAeneas.Proofs.FloorCeilResolve
import RatesAeneas.Proofs.HalfUpWrappers

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open rates_aeneas

set_option maxRecDepth 20000

namespace RatesAeneas.Proofs

/-- `WAD` is `10^18` (`constants::WAD`). -/
theorem wad_val : IScalar.val (ty := .I128) constants.production.shared.WAD = 10^18 := by
  unfold constants.production.shared.WAD; simp

/-- `WAD_DECIMALS` is `18`. -/
theorem wad_decimals_val : (constants.production.shared.WAD_DECIMALS).val = 18 := by
  unfold constants.production.shared.WAD_DECIMALS; simp

/-! ## Floor, ceiling and saturating wrappers -/

/-- **`Ray::mul_wad_ratio_floor` is one floor rounding.** `a * num / den`
rounded down, with the WAD ratio applied unquantized, for every nonzero
denominator whenever the floor fits `i128`. -/
theorem ray_mul_wad_ratio_floor_spec (env : Env) (a num den : Std.I128) (hd : den.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (a.val * num.val) den.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Ray.mul_wad_ratio_floor a env num den ⦃ r =>
      IsFloor (a.val * num.val) den.val r.val ⦄ := by
  unfold math.fp.Ray.mul_wad_ratio_floor
  step with mul_div_floor_spec env a num den hd hfit as ⟨q, hq⟩
  exact hq

/-- **`Wad::div_ceil` is the ceiling quotient** `⌈a*WAD/b⌉` for every nonzero
`b` whenever it fits `i128`. -/
theorem wad_div_ceil_spec (env : Env) (a b : Std.I128) (hb : b.val ≠ 0)
    (hfit : ∃ q : Int, IsCeil (a.val * 10^18) b.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Wad.div_ceil a env b ⦃ r => IsCeil (a.val * 10^18) b.val r.val ⦄ := by
  unfold math.fp.Wad.div_ceil
  step with mul_div_ceil_spec env a constants.production.shared.WAD b hb
    (by rw [wad_val]; exact hfit) as ⟨q, hq⟩
  rw [wad_val] at hq; exact hq

/-- **`Wad::mul_floor_saturating` is the clamped floor product.** It never
aborts and returns `⌊a*b/WAD⌋` clamped to `[i128::MIN, i128::MAX]`. -/
theorem wad_mul_floor_saturating_spec (env : Env) (a b : Std.I128) :
    math.fp.Wad.mul_floor_saturating a env b ⦃ r => ∀ q : Int,
      IsFloor (a.val * b.val) (10^18) q → r.val = max (-2^127) (min (2^127 - 1) q) ⦄ := by
  unfold math.fp.Wad.mul_floor_saturating
  step with mul_div_floor_saturating_spec env a b constants.production.shared.WAD
    (by rw [wad_val]; norm_num) as ⟨q, hq⟩
  rw [wad_val] at hq; exact hq

/-- **`Wad::mul_ratio_floor` is one floor rounding:** `⌊a*num/den⌋` for every
nonzero denominator whenever it fits `i128`. -/
theorem wad_mul_ratio_floor_spec (env : Env) (a num den : Std.I128) (hd : den.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (a.val * num.val) den.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Wad.mul_ratio_floor a env num den ⦃ r =>
      IsFloor (a.val * num.val) den.val r.val ⦄ := by
  unfold math.fp.Wad.mul_ratio_floor
  step with mul_div_floor_spec env a num den hd hfit as ⟨q, hq⟩
  exact hq

/-- **`Wad::apply_ratio_to_amount_floor` is one floor rounding:**
`⌊amount*self/den⌋` for every nonzero `den` whenever it fits `i128`. -/
theorem wad_apply_ratio_to_amount_floor_spec (env : Env) (self den amount : Std.I128)
    (hd : den.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (amount.val * self.val) den.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Wad.apply_ratio_to_amount_floor self env den amount ⦃ r =>
      IsFloor (amount.val * self.val) den.val r.val ⦄ := by
  unfold math.fp.Wad.apply_ratio_to_amount_floor
  exact mul_div_floor_spec env amount self den hd hfit

/-- **`Bps::from_fraction_floor` is `⌊num*BPS/den⌋`** for every nonzero `den`
whenever it fits `i128`. -/
theorem bps_from_fraction_floor_spec (env : Env) (num den : Std.I128) (hd : den.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (num.val * 10000) den.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Bps.from_fraction_floor env num den ⦃ r => IsFloor (num.val * 10000) den.val r.val ⦄ := by
  unfold math.fp.Bps.from_fraction_floor
  step with mul_div_floor_spec env num constants.production.shared.BPS den hd
    (by rw [bps_val]; exact hfit) as ⟨q, hq⟩
  rw [bps_val] at hq; exact hq

/-- **`Bps::from_fraction_ceil` is `⌈num*BPS/den⌉`** for every nonzero `den`
whenever it fits `i128`. -/
theorem bps_from_fraction_ceil_spec (env : Env) (num den : Std.I128) (hd : den.val ≠ 0)
    (hfit : ∃ q : Int, IsCeil (num.val * 10000) den.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Bps.from_fraction_ceil env num den ⦃ r => IsCeil (num.val * 10000) den.val r.val ⦄ := by
  unfold math.fp.Bps.from_fraction_ceil
  step with mul_div_ceil_spec env num constants.production.shared.BPS den hd
    (by rw [bps_val]; exact hfit) as ⟨q, hq⟩
  rw [bps_val] at hq; exact hq

/-- A ceiling share `⌈x*y/d⌉` with `0 ≤ x`, `0 ≤ y ≤ d` lies in `[0, x]`. -/
theorem isCeil_ratio_bounds {x y d q : Int} (hd : 0 < d) (hx : 0 ≤ x) (hy : 0 ≤ y)
    (hyd : y ≤ d) (h : IsCeil (x * y) d q) : 0 ≤ q ∧ q ≤ x := by
  unfold IsCeil at h; rw [if_pos hd] at h
  obtain ⟨h1, h2⟩ := h
  have hxy : 0 ≤ x * y := Int.mul_nonneg hx hy
  have hxyd : x * y ≤ x * d := Int.mul_le_mul_of_nonneg_left hyd hx
  constructor
  · by_contra hc
    have : q * d < 0 := Int.mul_neg_of_neg_of_pos (by omega) hd
    omega
  · by_contra hc
    have : (x + 1) * d ≤ q * d := Int.mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith

/-- **`Bps::apply_to_ceil` is the exact ceiling share, at most the amount.**
For an amount `≥ 0` and `0 ≤ bps ≤ BPS` it never aborts and returns
`⌈amount*bps/BPS⌉`, which lies in `[0, amount]`. -/
theorem bps_apply_to_ceil_spec (env : Env) (bps amount : Std.I128)
    (hbps : 0 ≤ bps.val ∧ bps.val ≤ 10000) (hamt : 0 ≤ amount.val) :
    math.fp.Bps.apply_to_ceil bps env amount ⦃ q =>
      IsCeil (amount.val * bps.val) 10000 q.val ∧ 0 ≤ q.val ∧ q.val ≤ amount.val ⦄ := by
  unfold math.fp.Bps.apply_to_ceil
  have hab := amount.hBounds
  simp only [IScalarTy.numBits] at hab; norm_num at hab
  obtain ⟨c, hc⟩ := exists_isCeil (amount.val * bps.val) 10000 (by norm_num)
  have hcb := isCeil_ratio_bounds (by norm_num) hamt hbps.1 hbps.2 hc
  have h := mul_div_ceil_spec env amount bps constants.production.shared.BPS
    (by rw [bps_val]; norm_num) (by rw [bps_val]; exact ⟨c, hc, by omega, by omega⟩)
  rw [bps_val] at h
  apply WP.spec_mono h
  intro q hq
  exact ⟨hq, isCeil_ratio_bounds (by norm_num) hamt hbps.1 hbps.2 hq⟩

/-- **`Bps::apply_to_ray_ceil` is the exact ceiling share of a ray, at most
the value.** Same claim as `bps_apply_to_ceil_spec` for a `Ray` value. -/
theorem bps_apply_to_ray_ceil_spec (env : Env) (bps v : Std.I128)
    (hbps : 0 ≤ bps.val ∧ bps.val ≤ 10000) (hv : 0 ≤ v.val) :
    math.fp.Bps.apply_to_ray_ceil bps env v ⦃ q =>
      IsCeil (v.val * bps.val) 10000 q.val ∧ 0 ≤ q.val ∧ q.val ≤ v.val ⦄ := by
  unfold math.fp.Bps.apply_to_ray_ceil
  have := bps_apply_to_ceil_spec env bps v hbps hv
  unfold math.fp.Bps.apply_to_ceil at this
  step with this as ⟨q, hq1, hq2, hq3⟩
  exact ⟨hq1, hq2, hq3⟩

/-! ## Half-up wrappers -/

/-- **`Bps::div` is the half-up ratio** `half_up(a*BPS/b)` for `a ≥ 0`,
`b > 0` whenever it fits `i128`. -/
theorem bps_div_spec (env : Env) (a b : Std.I128) (ha : 0 ≤ a.val) (hb : 0 < b.val)
    (hfit : ∃ q : Int, IsHalfUp (a.val * 10000) b.val q ∧ q < 2^127) :
    math.fp.Bps.div a env b ⦃ q => IsHalfUp (a.val * 10000) b.val q.val ⦄ := by
  unfold math.fp.Bps.div
  have h := mul_div_half_up_spec env a constants.production.shared.BPS b ha
    (by rw [bps_val]; norm_num) hb (by rw [bps_val]; exact hfit)
  rw [bps_val] at h
  step with h as ⟨q, hq⟩
  exact hq

/-- **`Bps::try_from_fraction` is the checked half-up ratio.** It never
aborts. `None` for `num < 0` or `den ≤ 0`; for `num ≥ 0`, `den > 0` it is
`Some(half_up(num*BPS/den))` when that fits `i128` and `None` exactly when it
does not (`HalfUpResult`). -/
theorem bps_try_from_fraction_spec (env : Env) (num den : Std.I128) :
    math.fp.Bps.try_from_fraction env num den ⦃ o =>
      ((num.val < 0 ∨ den.val ≤ 0) → o = none) ∧
      ((0 ≤ num.val ∧ 0 < den.val) → HalfUpResult (num.val * 10000) den.val o) ⦄ := by
  unfold math.fp.Bps.try_from_fraction
  step with try_mul_div_half_up_spec env num constants.production.shared.BPS den as ⟨o, h1, h2⟩
  rw [bps_val] at h1 h2
  have hR : (0 : Int) ≤ 10000 := by norm_num
  cases o with
  | none =>
    rw [Aeneas.Std.core.option.Option.map_none, WP.spec_ok]
    exact ⟨fun _ => rfl, fun ha hb => h2 ha hR hb⟩
  | some v =>
    rw [Aeneas.Std.core.option.Option.map_some]
    simp only [P.Insts.CoreOpsFunctionFnOnceTupleI128Bps.call_once, math.fp.Bps.constructor]
    apply WP.spec_bind ((WP.spec_ok _).mpr (rfl : v = v))
    rintro _ rfl
    rw [WP.spec_ok]
    refine ⟨fun h => ?_, fun ha hb => h2 ha hR hb⟩
    have := h1 (by rcases h with h | h <;> simp [h])
    simp at this

/-- **`Bps::from_weighted_sum` rounds to nearest, ties away from zero.** For a
positive WAD weight it never aborts and returns `q` with
`IsHalfAway weighted_sum weight q`, for every `i128` weighted sum. -/
theorem bps_from_weighted_sum_spec (env : Env) (ws w : Std.I128) (hw : 0 < w.val) :
    math.fp.Bps.from_weighted_sum env ws w ⦃ q => IsHalfAway ws.val w.val q.val ⦄ := by
  unfold math.fp.Bps.from_weighted_sum
  step with div_by_int_half_up_spec env ws w hw as ⟨q, hq⟩
  exact hq

/-- **`Wad::try_value_of_token` is the checked half-up value.** It never
aborts. With `decimals > 38` (`10^decimals` does not fit `i128`) it returns
`None`. Otherwise: `None` for a negative amount or price; for non-negative
operands `Some(half_up(amount*price/10^decimals))` when that fits `i128`, and
`None` exactly when it does not. -/
theorem wad_try_value_of_token_spec (env : Env) (price amount : Std.I128) (dec : Std.U32) :
    math.fp.Wad.try_value_of_token price env amount dec ⦃ o =>
      (38 < dec.val → o = none) ∧
      (dec.val ≤ 38 → (amount.val < 0 ∨ price.val < 0) → o = none) ∧
      (dec.val ≤ 38 → 0 ≤ amount.val → 0 ≤ price.val →
        HalfUpResult (amount.val * price.val) (10^dec.val) o) ⦄ := by
  unfold math.fp.Wad.try_value_of_token
  simp only [core.num.I128.checked_pow, bind_ok]
  have h10 : (10#i128).val = 10 := by simp
  rw [h10]
  have hpos := pow_pos (show (0:Int) < 10 by norm_num) dec.val
  by_cases hd : dec.val ≤ 38
  · have hpl := pow10_lt_i128 _ hd
    generalize hx : (10 : Int) ^ dec.val = x at hpl hpos ⊢
    have hp := IScalar.tryMkOpt_eq .I128 x
    cases hpo : IScalar.tryMkOpt .I128 x with
    | none =>
      exfalso; rw [hpo] at hp; apply hp
      simp only [IScalar.inBounds, IScalarTy.numBits]; constructor <;> omega
    | some f =>
      rw [hpo] at hp; obtain ⟨hfv, _⟩ := hp
      simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok]
      step with try_mul_div_half_up_spec env amount price f as ⟨o, h1, h2⟩
      rw [hfv] at h1 h2
      cases o with
      | none =>
        rw [Aeneas.Std.core.option.Option.map_none, WP.spec_ok]
        exact ⟨fun h => absurd hd (by omega), fun _ _ => rfl,
          fun _ ha hp => h2 ha hp (by omega)⟩
      | some v =>
        rw [Aeneas.Std.core.option.Option.map_some]
        simp only [P.Insts.CoreOpsFunctionFnOnceTupleI128Wad.call_once, math.fp.Wad.constructor]
        apply WP.spec_bind ((WP.spec_ok _).mpr (rfl : v = v))
        rintro _ rfl
        rw [WP.spec_ok]
        refine ⟨fun h => absurd hd (by omega), fun _ h => ?_,
          fun _ ha hp => h2 ha hp (by omega)⟩
        have := h1 (by rcases h with h | h <;> simp [h])
        simp at this
  · have hpg := pow10_ge_i128 _ (by omega : 39 ≤ dec.val)
    generalize hx : (10 : Int) ^ dec.val = x at hpg hpos ⊢
    have hp := IScalar.tryMkOpt_eq .I128 x
    cases hpo : IScalar.tryMkOpt .I128 x with
    | none =>
      simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok,
        core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual,
        WP.spec_ok]
      exact ⟨fun _ => trivial, fun h => absurd h hd, fun h => absurd h hd⟩
    | some f =>
      exfalso; rw [hpo] at hp; obtain ⟨hfv, _⟩ := hp
      have := f.hBounds
      simp only [IScalarTy.numBits] at this
      omega

/-! ## Half-up ratio wrapper (#225) -/

/-- `Ray::mul_ray_ratio` is literally one `mul_div_half_up(self, numerator, denominator)`. -/
theorem ray_mul_ray_ratio_eq (env : Env) (x n d : Std.I128) :
    math.fp.Ray.mul_ray_ratio x env n d = math.fp_core.mul_div_half_up env x n d := by
  unfold math.fp.Ray.mul_ray_ratio
  exact Std.bind_pure _

/-- **`Ray::mul_ray_ratio` is one half-up rounding** (added by #225).
For `x, n ≥ 0` and `d > 0` whose half-up quotient `q` of `x*n/d` fits `i128`,
`x.mul_ray_ratio(n, d)` returns (does not abort) exactly `q`: the ratio `n/d`
is applied unquantized, with no intermediate rounding. -/
theorem ray_mul_ray_ratio_spec (env : Env) (x n d : Std.I128)
    (hx : 0 ≤ x.val) (hn : 0 ≤ n.val) (hd : 0 < d.val)
    (hfit : ∃ q : Int, IsHalfUp (x.val * n.val) d.val q ∧ q < 2^127) :
    math.fp.Ray.mul_ray_ratio x env n d ⦃ r => IsHalfUp (x.val * n.val) d.val r.val ⦄ := by
  rw [ray_mul_ray_ratio_eq]
  exact mul_div_half_up_spec env x n d hx hn hd hfit

/-- **When `Ray::mul_ray_ratio` aborts.** It returns (does not abort) exactly
when `x ≥ 0`, `n ≥ 0`, `d > 0` and the half-up quotient of `x*n/d` fits
`i128`; a negative operand, a zero or negative denominator, or an overflowing
result aborts (`mul_div_half_up_returns_iff`). -/
theorem ray_mul_ray_ratio_returns_iff (env : Env) (x n d : Std.I128) :
    (math.fp.Ray.mul_ray_ratio x env n d ⦃ _ => True ⦄) ↔
      (0 ≤ x.val ∧ 0 ≤ n.val ∧ 0 < d.val ∧
        ∃ q : Int, IsHalfUp (x.val * n.val) d.val q ∧ q < 2^127) := by
  rw [ray_mul_ray_ratio_eq]
  exact mul_div_half_up_returns_iff env x n d

/-- **`Ray::mul_ray_ratio` by a ratio at most one never aborts.** For
`x ≥ 0` and `0 ≤ n ≤ d`, `d > 0`, it returns `q = half_up(x*n/d)` with
`0 ≤ q ≤ x` (so the result always fits). -/
theorem ray_mul_ray_ratio_frac_spec (env : Env) (x n d : Std.I128)
    (hx : 0 ≤ x.val) (hn : 0 ≤ n.val) (hnd : n.val ≤ d.val) (hd : 0 < d.val) :
    math.fp.Ray.mul_ray_ratio x env n d ⦃ r =>
      IsHalfUp (x.val * n.val) d.val r.val ∧ 0 ≤ r.val ∧ r.val ≤ x.val ⦄ := by
  have hxb := x.hBounds
  simp only [IScalarTy.numBits] at hxb; norm_num at hxb
  apply WP.spec_mono (ray_mul_ray_ratio_spec env x n d hx hn hd
    (halfUp_ratio_fits x.val n.val d.val hd hx (by omega) hn hnd))
  intro r hr
  exact ⟨hr, IsHalfUp.le_of_ratio_le_one hd hx hn hnd hr⟩

/-! ## Checked integer wrappers -/

/-- **`Ray::mul_int` is the exact integer multiple** `a * m` when it fits
`i128` (no rescaling). -/
theorem ray_mul_int_spec (env : Env) (a m : Std.I128)
    (h : -2^127 ≤ a.val * m.val ∧ a.val * m.val < 2^127) :
    math.fp.Ray.mul_int a env m ⦃ r => r.val = a.val * m.val ⦄ := by
  unfold math.fp.Ray.mul_int
  have hm := core.num.checked_mul_IScalar_bv_spec a m
  rw [show I128.checked_mul a m = core.num.checked_mul_IScalar a m from rfl]
  cases hc : core.num.checked_mul_IScalar a m with
  | none =>
    rw [hc] at hm; exfalso; apply hm
    simp only [IScalar.min, IScalar.max, IScalarTy.numBits]; omega
  | some c =>
    rw [hc] at hm
    obtain ⟨_, _, hcv, _⟩ := hm
    simp [lift, core.option.Option.unwrap_or_else, hcv]

/-- **`Ray::mul_int` aborts on overflow** (`MathOverflow` in production). -/
theorem ray_mul_int_aborts (env : Env) (a m : Std.I128)
    (h : a.val * m.val < -2^127 ∨ 2^127 ≤ a.val * m.val) :
    ¬ (math.fp.Ray.mul_int a env m ⦃ _ => True ⦄) := by
  unfold math.fp.Ray.mul_int
  have hm := core.num.checked_mul_IScalar_bv_spec a m
  rw [show I128.checked_mul a m = core.num.checked_mul_IScalar a m from rfl]
  cases hc : core.num.checked_mul_IScalar a m with
  | none =>
    simp [lift, core.option.Option.unwrap_or_else,
      math.fp.Ray.mul_int.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]
  | some c =>
    rw [hc] at hm
    obtain ⟨h1, h2, _, _⟩ := hm
    simp only [IScalar.min, IScalar.max, IScalarTy.numBits] at h1 h2
    omega

/-- `10i128.checked_pow(e)` is `Some(10^e)` for `e ≤ 38`. -/
theorem checked_pow10_some (e : Std.U32) (he : e.val ≤ 38) :
    ∃ f : Std.I128, core.num.I128.checked_pow 10#i128 e = ok (some f) ∧ f.val = 10^e.val := by
  simp only [core.num.I128.checked_pow]
  have h10 : (10#i128).val = 10 := by simp
  rw [h10]
  have hpl := pow10_lt_i128 _ he
  have hpos := pow_pos (show (0:Int) < 10 by norm_num) e.val
  generalize hx : (10 : Int) ^ e.val = x at hpl hpos ⊢
  have hp := IScalar.tryMkOpt_eq .I128 x
  cases hpo : IScalar.tryMkOpt .I128 x with
  | none =>
    exfalso; rw [hpo] at hp; apply hp
    simp only [IScalar.inBounds, IScalarTy.numBits]; constructor <;> omega
  | some f =>
    rw [hpo] at hp
    exact ⟨f, rfl, hp.1⟩

/-- **`Wad::try_from_token_upscale` is the exact checked upscale.** It never
aborts. `None` for a negative amount or `decimals > 18`; otherwise
`Some(amount * 10^(18 - decimals))` when that fits `i128`, and `None` exactly
when it does not. -/
theorem wad_try_from_token_upscale_spec (amount : Std.I128) (dec : Std.U32) :
    math.fp.Wad.try_from_token_upscale amount dec ⦃ o =>
      (amount.val < 0 → o = none) ∧
      (18 < dec.val → o = none) ∧
      (0 ≤ amount.val → dec.val ≤ 18 → match o with
        | some w => w.val = amount.val * 10^(18 - dec.val)
        | none => 2^127 ≤ amount.val * 10^(18 - dec.val)) ⦄ := by
  unfold math.fp.Wad.try_from_token_upscale
  by_cases ha : amount.val < 0
  · have : amount < 0#i128 := by simp [IScalar.lt_equiv]; omega
    simp only [this, ↓reduceIte, WP.spec_ok]
    exact ⟨fun _ => trivial, fun _ => trivial, fun h => absurd h (by omega)⟩
  have ha' : ¬ (amount < 0#i128) := by simp [IScalar.lt_equiv]; omega
  simp only [ha', ↓reduceIte, lift, bind_ok]
  rw [show U32.checked_sub constants.production.shared.WAD_DECIMALS dec
      = core.num.checked_sub_UScalar constants.production.shared.WAD_DECIMALS dec from rfl]
  have hs := core.num.checked_sub_UScalar_bv_spec constants.production.shared.WAD_DECIMALS dec
  rw [wad_decimals_val] at hs
  cases hsc : core.num.checked_sub_UScalar constants.production.shared.WAD_DECIMALS dec with
  | none =>
    rw [hsc] at hs
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual, WP.spec_ok]
    exact ⟨fun h => absurd h ha, fun _ => trivial, fun _ h => absurd h (by omega)⟩
  | some k =>
    rw [hsc] at hs
    obtain ⟨hle, hkv, _⟩ := hs
    obtain ⟨f, hf, hfv⟩ := checked_pow10_some k (by omega)
    rw [hkv] at hfv
    have hpos := pow_pos (show (0:Int) < 10 by norm_num) (18 - dec.val)
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok, hf]
    rw [show I128.checked_mul amount f = core.num.checked_mul_IScalar amount f from rfl]
    have hm := core.num.checked_mul_IScalar_bv_spec amount f
    have hprod : 0 ≤ amount.val * 10^(18 - dec.val) := Int.mul_nonneg (by omega) (by omega)
    cases hmo : core.num.checked_mul_IScalar amount f with
    | none =>
      rw [hmo] at hm
      rw [Aeneas.Std.core.option.Option.map_none, WP.spec_ok]
      refine ⟨fun h => absurd h ha, fun h => absurd hle (by omega), fun _ _ => ?_⟩
      rw [hfv] at hm
      simp only [IScalar.min, IScalar.max, IScalarTy.numBits] at hm
      omega
    | some r =>
      rw [hmo] at hm; obtain ⟨_, _, hr, _⟩ := hm
      rw [Aeneas.Std.core.option.Option.map_some]
      simp only [P.Insts.CoreOpsFunctionFnOnceTupleI128Wad.call_once, math.fp.Wad.constructor]
      apply WP.spec_bind ((WP.spec_ok _).mpr (rfl : r = r))
      rintro _ rfl
      rw [WP.spec_ok]
      refine ⟨fun h => absurd h ha, fun h => absurd hle (by omega), fun _ _ => ?_⟩
      show r.val = amount.val * 10^(18 - dec.val)
      rw [hr, hfv]

end RatesAeneas.Proofs
