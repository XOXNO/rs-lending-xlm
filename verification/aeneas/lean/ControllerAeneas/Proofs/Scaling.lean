/-
`Ray` floor/ceiling wrappers (`common/src/math/fp.rs`) and the directed
share conversions of `common/src/rates/scaling.rs` that the liquidation math
reaches (`unscale_borrow_ceil`, `unscale_supply_floor`, `Ray::to_asset_*`).

Snapshot of `RatesAeneas/Proofs/FloorCeilScaling.lean` up to
`unscale_borrow_ceil_spec` (without the `calculate_scaled_*` theorems, which
need `from_asset_spec`), taken 2026-11-12, namespace changed (`ray_val`
from `RatesAeneas/Proofs/Scaling.lean` restated here); see WRITING-PROOFS.md,
last section.
-/
import ControllerAeneas.Proofs.FloorCeilRescale
import ControllerAeneas.Proofs.Saturating

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- `RAY = 10^27`. -/
theorem ray_val : IScalar.val (ty := .I128) constants.production.shared.RAY = 10^27 := by
  unfold constants.production.shared.RAY; rfl

/-- `RAY_DECIMALS = 27`. -/
theorem ray_decimals_val : (constants.production.shared.RAY_DECIMALS).val = 27 := by
  unfold constants.production.shared.RAY_DECIMALS; simp

/-! ## Nesting of directed divisions -/

/-- Flooring twice is flooring once by the product of the divisors:
`⌊⌊n/a⌋/b⌋ = ⌊n/(a*b)⌋` for `a, b > 0`. -/
theorem IsFloor.nest {n a b m r : Int} (ha : 0 < a) (hb : 0 < b)
    (h1 : IsFloor n a m) (h2 : IsFloor m b r) : IsFloor n (a * b) r := by
  unfold IsFloor at *
  rw [if_pos ha] at h1; rw [if_pos hb] at h2; rw [if_pos (mul_pos ha hb)]
  obtain ⟨h1a, h1b⟩ := h1; obtain ⟨h2a, h2b⟩ := h2
  constructor
  · have : r * b * a ≤ m * a := mul_le_mul_of_nonneg_right h2a (by omega)
    nlinarith
  · have : (m + 1) * a ≤ (r * b + b) * a := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith

/-- Ceiling twice is ceiling once by the product of the divisors:
`⌈⌈n/a⌉/b⌉ = ⌈n/(a*b)⌉` for `a, b > 0`. -/
theorem IsCeil.nest {n a b m r : Int} (ha : 0 < a) (hb : 0 < b)
    (h1 : IsCeil n a m) (h2 : IsCeil m b r) : IsCeil n (a * b) r := by
  unfold IsCeil at *
  rw [if_pos ha] at h1; rw [if_pos hb] at h2; rw [if_pos (mul_pos ha hb)]
  obtain ⟨h1a, h1b⟩ := h1; obtain ⟨h2a, h2b⟩ := h2
  constructor
  · have : m * a ≤ r * b * a := mul_le_mul_of_nonneg_right h2a (by omega)
    nlinarith
  · have : r * b * a ≤ (m + b - 1) * a := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith

/-- Dividing by one is exact (floor). -/
theorem isFloor_one (a : Int) : IsFloor a 1 a := by unfold IsFloor; simp
/-- Dividing by one is exact (ceiling). -/
theorem isCeil_one (a : Int) : IsCeil a 1 a := by unfold IsCeil; simp

/-! ## `Ray` wrappers -/

/-- `Ray::mul_floor(a, b)` is `⌊a*b/RAY⌋` whenever that fits `i128` (it
returns, does not abort). Hypothesis: the result fits (production aborts with
`MathOverflow` otherwise). -/
theorem ray_mul_floor_spec (env : Env) (a b : Std.I128)
    (hfit : ∃ q : Int, IsFloor (a.val * b.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Ray.mul_floor a env b ⦃ r => IsFloor (a.val * b.val) (10^27) r.val ⦄ := by
  unfold math.fp.Ray.mul_floor
  step with mul_div_floor_spec env a b constants.production.shared.RAY
    (by rw [ray_val]; norm_num) (by rw [ray_val]; exact hfit) as ⟨q, hq⟩
  rw [ray_val] at hq; exact hq

/-- `Ray::mul_ceil(a, b)` is `⌈a*b/RAY⌉` whenever that fits `i128`. -/
theorem ray_mul_ceil_spec (env : Env) (a b : Std.I128)
    (hfit : ∃ q : Int, IsCeil (a.val * b.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Ray.mul_ceil a env b ⦃ r => IsCeil (a.val * b.val) (10^27) r.val ⦄ := by
  unfold math.fp.Ray.mul_ceil
  step with mul_div_ceil_spec env a b constants.production.shared.RAY
    (by rw [ray_val]; norm_num) (by rw [ray_val]; exact hfit) as ⟨q, hq⟩
  rw [ray_val] at hq; exact hq

/-- `Ray::div_floor(a, b)` is `⌊a*RAY/b⌋` for every nonzero `b` whenever that
fits `i128`. -/
theorem ray_div_floor_spec (env : Env) (a b : Std.I128) (hb : b.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (a.val * 10^27) b.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Ray.div_floor a env b ⦃ r => IsFloor (a.val * 10^27) b.val r.val ⦄ := by
  unfold math.fp.Ray.div_floor
  step with mul_div_floor_spec env a constants.production.shared.RAY b hb
    (by rw [ray_val]; exact hfit) as ⟨q, hq⟩
  rw [ray_val] at hq; exact hq

/-- `Ray::div_ceil(a, b)` is `⌈a*RAY/b⌉` for every nonzero `b` whenever that
fits `i128`. -/
theorem ray_div_ceil_spec (env : Env) (a b : Std.I128) (hb : b.val ≠ 0)
    (hfit : ∃ q : Int, IsCeil (a.val * 10^27) b.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp.Ray.div_ceil a env b ⦃ r => IsCeil (a.val * 10^27) b.val r.val ⦄ := by
  unfold math.fp.Ray.div_ceil
  step with mul_div_ceil_spec env a constants.production.shared.RAY b hb
    (by rw [ray_val]; exact hfit) as ⟨q, hq⟩
  rw [ray_val] at hq; exact hq

/-- `Ray::to_asset_floor(a, decimals)` for `decimals ≤ 27` truncates
`a / 10^(27 - decimals)` toward zero: the floor for `a ≥ 0`, the ceiling for
`a < 0`. Never aborts. Hypothesis `decimals ≤ 27`: listed assets have at most
`MAX_ASSET_DECIMALS = 18` decimals. -/
theorem ray_to_asset_floor_spec (env : Env) (a : Std.I128) (dec : Std.U32)
    (hdec : dec.val ≤ 27) :
    math.fp.Ray.to_asset_floor a env dec ⦃ r =>
      (0 ≤ a.val → IsFloor a.val (10 ^ (27 - dec.val)) r.val) ∧
      (a.val < 0 → IsCeil a.val (10 ^ (27 - dec.val)) r.val) ⦄ := by
  unfold math.fp.Ray.to_asset_floor
  by_cases h27 : dec.val = 27
  · have he : constants.production.shared.RAY_DECIMALS = dec := by
      apply UScalar.eq_of_val_eq; rw [ray_decimals_val, h27]
    rw [rescale_floor_eq env a _ _ he, h27]
    simp only [Nat.sub_self, pow_zero, WP.spec_ok]
    exact ⟨fun _ => isFloor_one _, fun _ => isCeil_one _⟩
  · have := rescale_floor_down_spec env a constants.production.shared.RAY_DECIMALS dec
      (by rw [ray_decimals_val]; omega)
    rw [ray_decimals_val] at this
    exact this

/-- `Ray::to_asset_ceil(a, decimals)` for `decimals ≤ 27` is
`⌈a / 10^(27 - decimals)⌉` for every `a`. Never aborts. -/
theorem ray_to_asset_ceil_spec (env : Env) (a : Std.I128) (dec : Std.U32)
    (hdec : dec.val ≤ 27) :
    math.fp.Ray.to_asset_ceil a env dec ⦃ r => IsCeil a.val (10 ^ (27 - dec.val)) r.val ⦄ := by
  unfold math.fp.Ray.to_asset_ceil
  by_cases h27 : dec.val = 27
  · have he : constants.production.shared.RAY_DECIMALS = dec := by
      apply UScalar.eq_of_val_eq; rw [ray_decimals_val, h27]
    rw [rescale_ceil_eq env a _ _ he, h27]
    simp only [Nat.sub_self, pow_zero, WP.spec_ok]
    exact isCeil_one _
  · have := rescale_ceil_down_spec env a constants.production.shared.RAY_DECIMALS dec
      (by rw [ray_decimals_val]; omega)
    rw [ray_decimals_val] at this
    exact this

/-! ## `rates/scaling.rs` -/

/-- **`unscale_supply_floor` is a single floor.** For `scaled * index ≥ 0`
(production: share balances and indexes are non-negative, INV-ACCT-01,
INV-IDX-02) and `decimals ≤ 27`, the asset amount paid for `scaled` shares
is exactly `⌊scaled*index / (RAY * 10^(27-decimals))⌋`: the two floors
(`mul_floor`, then `to_asset_floor`) compose to one, so the claim is never
worth more than the shares. Hypothesis: the intermediate `⌊scaled*index/RAY⌋`
fits `i128` (otherwise production aborts). -/
theorem unscale_supply_floor_isFloor (env : Env) (scaled index : Std.I128) (dec : Std.U32)
    (hdec : dec.val ≤ 27) (hnn : 0 ≤ scaled.val * index.val)
    (hfit : ∃ q : Int, IsFloor (scaled.val * index.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    rates.scaling.unscale_supply_floor env scaled index dec ⦃ r =>
      IsFloor (scaled.val * index.val) (10^27 * 10^(27 - dec.val)) r.val ⦄ := by
  unfold rates.scaling.unscale_supply_floor
  step with ray_mul_floor_spec env scaled index hfit as ⟨m, hm⟩
  step with ray_to_asset_floor_spec env m dec hdec as ⟨r, hr, _⟩
  have hm0 : 0 ≤ m.val := by
    unfold IsFloor at hm; rw [if_pos (by norm_num)] at hm
    by_contra h
    have : (m.val + 1) * 10^27 ≤ 0 := by nlinarith
    omega
  exact IsFloor.nest (by norm_num) (by positivity) hm (hr hm0)

/-- **`unscale_borrow_ceil` is a single ceiling.** For every `scaled` and
`index` and `decimals ≤ 27`, the debt owed for `scaled` debt shares is
exactly `⌈scaled*index / (RAY * 10^(27-decimals))⌉`: `mul_ceil` then
`to_asset_ceil` compose to one ceiling, so the debt is never valued below its
exact amount. Hypothesis: `⌈scaled*index/RAY⌉` fits `i128`. -/
theorem unscale_borrow_ceil_spec (env : Env) (scaled index : Std.I128) (dec : Std.U32)
    (hdec : dec.val ≤ 27)
    (hfit : ∃ q : Int, IsCeil (scaled.val * index.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    rates.scaling.unscale_borrow_ceil env scaled index dec ⦃ r =>
      IsCeil (scaled.val * index.val) (10^27 * 10^(27 - dec.val)) r.val ⦄ := by
  unfold rates.scaling.unscale_borrow_ceil
  step with ray_mul_ceil_spec env scaled index hfit as ⟨m, hm⟩
  step with ray_to_asset_ceil_spec env m dec hdec as ⟨r, hr⟩
  exact IsCeil.nest (by norm_num) (by positivity) hm hr


end ControllerAeneas.Proofs
