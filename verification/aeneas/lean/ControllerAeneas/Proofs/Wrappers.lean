/-
`common/src/math/fp.rs`: the typed fixed-point wrappers the controller
liquidation math goes through since #224 (typed fixed-point boundary), when
`fp_core` became private.

| Wrapper | Body |
|---|---|
| `Ray::mul_wad_ratio_floor(x, n, d)` | `mul_div_floor(x, n, d)` |
| `Wad::apply_ratio_to_amount_floor(n, d, a)` | `mul_div_floor(a, n, d)` |
| `Bps::from_fraction_ceil(n, d)` | `mul_div_ceil(n, BPS, d)` |
| `Bps::apply_to_ray_ceil(b, v)` | `mul_div_ceil(v, b, BPS)` |

Each wrapper is a single call of the raw primitive (no extra rounding, no
quantisation of the ratio), so every spec below is the raw `mul_div_*` spec
restated over the typed arguments, plus a domain corollary (non-negative
inputs, ratio at most one) under which the call never aborts.
-/
import ControllerAeneas.Proofs.Value
import ControllerAeneas.Proofs.Basic

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## Unfolding equations -/

/-- `Ray::mul_wad_ratio_floor` is exactly one `mul_div_floor(x, n, d)`. -/
theorem mul_wad_ratio_floor_eq (env : Env) (x : math.fp.Ray) (n d : math.fp.Wad) :
    math.fp.Ray.mul_wad_ratio_floor x env n d = math.fp_core.mul_div_floor env x n d := by
  unfold math.fp.Ray.mul_wad_ratio_floor; simp

/-- `Wad::apply_ratio_to_amount_floor` is exactly one `mul_div_floor(a, n, d)`. -/
theorem apply_ratio_to_amount_floor_eq (env : Env) (n d : math.fp.Wad) (a : Std.I128) :
    math.fp.Wad.apply_ratio_to_amount_floor n env d a = math.fp_core.mul_div_floor env a n d := by
  unfold math.fp.Wad.apply_ratio_to_amount_floor; rfl

/-- `Bps::from_fraction_ceil` is exactly one `mul_div_ceil(n, BPS, d)`. -/
theorem from_fraction_ceil_eq (env : Env) (n d : math.fp.Wad) :
    math.fp.Bps.from_fraction_ceil env n d =
      math.fp_core.mul_div_ceil env n constants.production.shared.BPS d := by
  unfold math.fp.Bps.from_fraction_ceil; simp

/-- `Bps::apply_to_ray_ceil` is exactly one `mul_div_ceil(v, b, BPS)`. -/
theorem apply_to_ray_ceil_eq (env : Env) (b : math.fp.Bps) (v : math.fp.Ray) :
    math.fp.Bps.apply_to_ray_ceil b env v =
      math.fp_core.mul_div_ceil env v b constants.production.shared.BPS := by
  unfold math.fp.Bps.apply_to_ray_ceil; simp

/-! ## A ratio at most one keeps the floor in range -/

/-- A floor `⌊x·num/den⌋` with `0 ≤ num < den` lies between `x` and 0, so it
fits `i128`. -/
theorem floor_ratio_fits (x num den : Int) (hn : 0 ≤ num) (hnd : num < den)
    (hx : -2^127 ≤ x ∧ x < 2^127) :
    ∃ q, IsFloor (x * num) den q ∧ -2^127 ≤ q ∧ q < 2^127 := by
  obtain ⟨q, hq⟩ := isFloor_exists (x * num) den (by omega)
  have hq' := (isFloor_pos (by omega)).mp hq
  refine ⟨q, hq, ?_, ?_⟩
  · by_contra h
    have : q + 1 ≤ -2^127 := by omega
    have h1 : (q + 1) * den ≤ -2^127 * den := mul_le_mul_of_nonneg_right this (by omega)
    rcases le_or_gt 0 x with hx0 | hx0
    · nlinarith
    · have : x * den ≤ x * num := mul_le_mul_of_nonpos_left hnd.le hx0.le
      nlinarith
  · by_contra h
    have h1 : 2^127 * den ≤ q * den := mul_le_mul_of_nonneg_right (by omega) (by omega)
    rcases le_or_gt 0 x with hx0 | hx0
    · have : x * num ≤ x * den := mul_le_mul_of_nonneg_left hnd.le hx0
      nlinarith
    · have : x * num ≤ 0 := mul_nonpos_of_nonpos_of_nonneg hx0.le hn
      nlinarith

/-- The same with `0 ≤ num ≤ den`, `0 < den`, for a non-negative `x`: the
floor is in `[0, x]`. -/
theorem floor_ratio_le (x num den : Int) (hx : 0 ≤ x) (hn : 0 ≤ num) (hnd : num ≤ den)
    (hd : 0 < den) {q : Int} (hq : IsFloor (x * num) den q) : 0 ≤ q ∧ q ≤ x := by
  have hq' := (isFloor_pos hd).mp hq
  have h1 : x * num ≤ x * den := mul_le_mul_of_nonneg_left hnd hx
  have h2 : 0 ≤ x * num := mul_nonneg hx hn
  constructor
  · by_contra h
    have : (q + 1) * den ≤ 0 := mul_nonpos_of_nonpos_of_nonneg (by omega) hd.le
    nlinarith
  · by_contra h
    have : (x + 1) * den ≤ q * den := mul_le_mul_of_nonneg_right (by omega) hd.le
    nlinarith

/-- Ceiling counterpart: for `0 ≤ x`, `0 ≤ num ≤ den`, `0 < den`, the
ceiling `⌈x·num/den⌉` exists and lies in `[0, x]`. -/
theorem ceil_ratio_le (x num den : Int) (hx : 0 ≤ x) (hn : 0 ≤ num) (hnd : num ≤ den)
    (hd : 0 < den) : ∃ c, IsCeil (x * num) den c ∧ 0 ≤ c ∧ c ≤ x := by
  obtain ⟨c, hc⟩ := isCeil_exists (x * num) den (by omega)
  have hc' := (isCeil_pos hd).mp hc
  have h1 : x * num ≤ x * den := mul_le_mul_of_nonneg_left hnd hx
  have h2 : 0 ≤ x * num := mul_nonneg hx hn
  refine ⟨c, hc, ?_, ?_⟩
  · by_contra h
    have : c * den ≤ (-1) * den := mul_le_mul_of_nonneg_right (by omega) hd.le
    nlinarith
  · by_contra h
    have : (x + 1) * den ≤ c * den := mul_le_mul_of_nonneg_right (by omega) hd.le
    nlinarith

/-! ## `Ray::mul_wad_ratio_floor` -/

/-- **`Ray::mul_wad_ratio_floor` applies a WAD ratio with one floor.** For any
ray `x` and WAD values `n`, `d ≠ 0` whose exact floor `⌊x·n/d⌋` fits `i128`,
the call returns (does not abort) that floor: the ratio `n/d` is not
quantised first, so there is a single downward rounding. -/
theorem mul_wad_ratio_floor_spec (env : Env) (x : math.fp.Ray) (n d : math.fp.Wad)
    (hd : d.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (x.val * n.val) d.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Ray.mul_wad_ratio_floor x env n d ⦃ r => IsFloor (x.val * n.val) d.val r.val ⦄ := by
  rw [mul_wad_ratio_floor_eq]
  exact mul_div_floor_spec env x n d hd hfit

/-- **`Ray::mul_wad_ratio_floor` with a ratio at most one never aborts and
never increases.** For `x ≥ 0` and `0 ≤ n ≤ d`, `d > 0` (the controller's
`received_usd ≤ planned_usd` scaling) the call returns `⌊x·n/d⌋ ∈ [0, x]`. -/
theorem mul_wad_ratio_floor_le (env : Env) (x : math.fp.Ray) (n d : math.fp.Wad)
    (hx : 0 ≤ x.val) (hn : 0 ≤ n.val) (hnd : n.val ≤ d.val) (hd : 0 < d.val) :
    math.fp.Ray.mul_wad_ratio_floor x env n d ⦃ r =>
      IsFloor (x.val * n.val) d.val r.val ∧ 0 ≤ r.val ∧ r.val ≤ x.val ⦄ := by
  obtain ⟨q, hq⟩ := isFloor_exists (x.val * n.val) d.val (by omega)
  have hb := floor_ratio_le _ _ _ hx hn hnd hd hq
  have hxb := x.hBounds
  simp only [IScalarTy.numBits] at hxb
  apply WP.spec_mono (mul_wad_ratio_floor_spec env x n d (by omega) ⟨q, hq, by omega, by omega⟩)
  intro r hr
  exact ⟨hr, floor_ratio_le _ _ _ hx hn hnd hd hr⟩

/-! ## `Wad::apply_ratio_to_amount_floor` -/

/-- **`Wad::apply_ratio_to_amount_floor` scales an amount by `self/d` with
one floor.** For a token amount `a` and WAD values `n` (the receiver,
`self`) and `d ≠ 0` whose exact floor `⌊a·n/d⌋` fits `i128`, the call returns
(does not abort) that floor. -/
theorem apply_ratio_to_amount_floor_spec (env : Env) (n d : math.fp.Wad) (a : Std.I128)
    (hd : d.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (a.val * n.val) d.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Wad.apply_ratio_to_amount_floor n env d a ⦃ r =>
      IsFloor (a.val * n.val) d.val r.val ⦄ := by
  rw [apply_ratio_to_amount_floor_eq]
  exact mul_div_floor_spec env a n d hd hfit

/-- **`Wad::apply_ratio_to_amount_floor` with a ratio at most one never aborts
and never increases.** For `a ≥ 0` and `0 ≤ n ≤ d`, `d > 0`, the call returns
`⌊a·n/d⌋ ∈ [0, a]`. -/
theorem apply_ratio_to_amount_floor_le (env : Env) (n d : math.fp.Wad) (a : Std.I128)
    (ha : 0 ≤ a.val) (hn : 0 ≤ n.val) (hnd : n.val ≤ d.val) (hd : 0 < d.val) :
    math.fp.Wad.apply_ratio_to_amount_floor n env d a ⦃ r =>
      IsFloor (a.val * n.val) d.val r.val ∧ 0 ≤ r.val ∧ r.val ≤ a.val ⦄ := by
  obtain ⟨q, hq⟩ := isFloor_exists (a.val * n.val) d.val (by omega)
  have hb := floor_ratio_le _ _ _ ha hn hnd hd hq
  have hab := a.hBounds
  simp only [IScalarTy.numBits] at hab
  apply WP.spec_mono
    (apply_ratio_to_amount_floor_spec env n d a (by omega) ⟨q, hq, by omega, by omega⟩)
  intro r hr
  exact ⟨hr, floor_ratio_le _ _ _ ha hn hnd hd hr⟩

/-! ## `Bps::from_fraction_ceil` -/

/-- **`Bps::from_fraction_ceil` forms a BPS ratio rounding up.** For WAD
values `n` and `d ≠ 0` whose exact ceiling `⌈n·BPS/d⌉` fits `i128`, the call
returns (does not abort) that ceiling. -/
theorem from_fraction_ceil_spec (env : Env) (n d : math.fp.Wad) (hd : d.val ≠ 0)
    (hfit : ∃ q : Int, IsCeil (n.val * 10^4) d.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Bps.from_fraction_ceil env n d ⦃ r => IsCeil (n.val * 10^4) d.val r.val ⦄ := by
  rw [from_fraction_ceil_eq]
  have h := mul_div_ceil_spec env n constants.production.shared.BPS d hd
    (by rw [BPS_val]; exact hfit)
  rw [BPS_val] at h
  exact h

/-- **`Bps::from_fraction_ceil` never aborts for a non-negative numerator and a
positive denominator, when the ratio fits.** For `0 ≤ n`, `0 < d` and
`n·BPS ≤ (2^127 − 1)·d` (the ceiling fits `i128`; it always does when
`d ≥ BPS`, e.g. `d = Wad::ONE` as in `max_bonus_for_threshold`) the call
returns `⌈n·BPS/d⌉ ≥ 0`. -/
theorem from_fraction_ceil_nonneg (env : Env) (n d : math.fp.Wad)
    (hn : 0 ≤ n.val) (hd : 0 < d.val) (hfit : n.val * 10^4 ≤ (2^127 - 1) * d.val) :
    math.fp.Bps.from_fraction_ceil env n d ⦃ r =>
      IsCeil (n.val * 10^4) d.val r.val ∧ 0 ≤ r.val ⦄ := by
  obtain ⟨c, hc⟩ := isCeil_exists (n.val * 10^4) d.val (by omega)
  have hc' := (isCeil_pos hd).mp hc
  have hc0 : 0 ≤ c := by
    by_contra h
    have : c * d.val ≤ (-1) * d.val := mul_le_mul_of_nonneg_right (by omega) hd.le
    nlinarith
  have hcM : c < 2^127 := by
    by_contra h
    have : 2^127 * d.val ≤ c * d.val := mul_le_mul_of_nonneg_right (by omega) hd.le
    nlinarith
  apply WP.spec_mono (from_fraction_ceil_spec env n d (by omega) ⟨c, hc, by omega, hcM⟩)
  intro r hr
  exact ⟨hr, by rw [IsCeil.unique (by omega) hr hc]; exact hc0⟩

/-! ## `Bps::apply_to_ray_ceil` -/

/-- **`Bps::apply_to_ray_ceil` applies a BPS rate to a ray rounding up.** For
a rate `b` and a ray `v` whose exact ceiling `⌈v·b/BPS⌉` fits `i128`, the
call returns (does not abort) that ceiling. -/
theorem apply_to_ray_ceil_spec (env : Env) (b : math.fp.Bps) (v : math.fp.Ray)
    (hfit : ∃ q : Int, IsCeil (v.val * b.val) (10^4) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Bps.apply_to_ray_ceil b env v ⦃ r => IsCeil (v.val * b.val) (10^4) r.val ⦄ := by
  rw [apply_to_ray_ceil_eq]
  have h := mul_div_ceil_spec env v b constants.production.shared.BPS
    (by rw [BPS_val]; norm_num) (by rw [BPS_val]; exact hfit)
  rw [BPS_val] at h
  exact h

/-- **`Bps::apply_to_ray_ceil` with a rate at most 100% never aborts and never
exceeds its input.** For `v ≥ 0` and `0 ≤ b ≤ BPS` the call returns
`⌈v·b/BPS⌉ ∈ [0, v]`. -/
theorem apply_to_ray_ceil_le (env : Env) (b : math.fp.Bps) (v : math.fp.Ray)
    (hv : 0 ≤ v.val) (hb : 0 ≤ b.val) (hbB : b.val ≤ 10^4) :
    math.fp.Bps.apply_to_ray_ceil b env v ⦃ r =>
      IsCeil (v.val * b.val) (10^4) r.val ∧ 0 ≤ r.val ∧ r.val ≤ v.val ⦄ := by
  obtain ⟨c, hc, hc0, hcv⟩ := ceil_ratio_le _ _ _ hv hb hbB (by norm_num)
  have hvb := v.hBounds
  simp only [IScalarTy.numBits] at hvb
  apply WP.spec_mono (apply_to_ray_ceil_spec env b v ⟨c, hc, by omega, by omega⟩)
  intro r hr
  have he := IsCeil.unique (by norm_num) hr hc
  exact ⟨hr, by omega, by omega⟩

end ControllerAeneas.Proofs
