/-
Exact-rounding specs of the `Wad` operations used by the repayment-plan
functions of `liquidation/math.rs` (`process_excess_payment`,
`whole_unit_repayment`, `one_unit_per_*`): `Wad::div_floor`,
`Wad::div_ceil`, `Wad::mul_floor`, `Wad::mul`, `Wad::to_token_floor`.
Each is one raw `fp_core` call; the specs restate the raw specs
(`mul_div_floor_spec`, `mul_div_ceil_spec`, `mul_div_half_up_spec`,
`rescale_floor_down_spec`) over the typed arguments.
-/
import ControllerAeneas.Proofs.Repayment
import ControllerAeneas.Proofs.Sum

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 2000000

namespace ControllerAeneas.Proofs

/-- `WAD = 10^18`. -/
theorem rp_wad_val : (constants.production.shared.WAD : Std.I128).val = 10^18 := WAD_val

/-- `Wad::ZERO = 0`. -/
theorem rp_zero_val : (math.fp.Wad.ZERO : Std.I128).val = 0 := by
  unfold math.fp.Wad.ZERO; rfl

/-- `Wad::ONE = 10^18`. -/
theorem rp_one_val : (math.fp.Wad.ONE : Std.I128).val = 10^18 := by
  unfold math.fp.Wad.ONE; exact WAD_val

/-- `Wad::div_floor(a, b)` is `⌊a·W/b⌋` (one floor) for `a ≥ 0`, `b > 0`
whenever `a·W ≤ (2^127 − 1)·b` (the quotient fits). -/
theorem rp_div_floor_spec (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 < b.val)
    (hfit : a.val * 10^18 < 2^127 * b.val) :
    math.fp.Wad.div_floor a env b ⦃ q => IsFloor (a.val * 10^18) b.val q.val ⦄ := by
  unfold math.fp.Wad.div_floor
  obtain ⟨q, hq⟩ := isFloor_exists (a.val * 10^18) b.val hb
  have hq' := (isFloor_pos hb).mp hq
  have hq0 : 0 ≤ q := IsFloor.nonneg_of hb hq (by positivity)
  have hqhi : q < 2^127 := by
    by_contra h; replace h := not_lt.mp h
    have : 2^127 * b.val ≤ q * b.val := mul_le_mul_of_nonneg_right h hb.le
    linarith [hq'.1]
  have h := mul_div_floor_spec env a constants.production.shared.WAD b (by omega)
    ⟨q, by rw [rp_wad_val]; exact hq, by omega, hqhi⟩
  rw [rp_wad_val] at h
  step with h as ⟨r, hr⟩
  simpa using hr

/-- `Wad::div_ceil(a, b)` is `⌈a·W/b⌉` (one ceiling) for `a ≥ 0`, `b > 0`
whenever `a·W + b ≤ 2^127·b`... stated as: the ceiling fits. -/
theorem rp_div_ceil_spec (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 < b.val)
    (hfit : a.val * 10^18 < (2^127 - 1) * b.val) :
    math.fp.Wad.div_ceil a env b ⦃ q => IsCeil (a.val * 10^18) b.val q.val ⦄ := by
  unfold math.fp.Wad.div_ceil
  obtain ⟨q, hq⟩ := isCeil_exists (a.val * 10^18) b.val hb
  have hq' := (isCeil_pos hb).mp hq
  have hq0 : 0 ≤ q := by
    by_contra h; replace h := not_le.mp h
    have : q * b.val < 0 := mul_neg_of_neg_of_pos h hb
    have : 0 ≤ a.val * 10^18 := by positivity
    linarith [hq'.1]
  have hqhi : q < 2^127 := by
    by_contra h; replace h := not_lt.mp h
    have : 2^127 * b.val ≤ q * b.val := mul_le_mul_of_nonneg_right h hb.le
    linarith [hq'.2]
  have h := mul_div_ceil_spec env a constants.production.shared.WAD b (by omega)
    ⟨q, by rw [rp_wad_val]; exact hq, by omega, hqhi⟩
  rw [rp_wad_val] at h
  step with h as ⟨r, hr⟩
  simpa using hr

/-- `Wad::mul_floor(a, b)` is `⌊a·b/W⌋` for `a, b ≥ 0` whenever that fits. -/
theorem rp_mul_floor_spec (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 ≤ b.val)
    (hfit : a.val * b.val < 2^127 * 10^18) :
    math.fp.Wad.mul_floor a env b ⦃ q => IsFloor (a.val * b.val) (10^18) q.val ⦄ := by
  unfold math.fp.Wad.mul_floor
  obtain ⟨q, hq⟩ := isFloor_exists (a.val * b.val) (10^18) (by norm_num)
  have hq' := (isFloor_pos (by norm_num : (0:Int) < 10^18)).mp hq
  have hq0 : 0 ≤ q := IsFloor.nonneg_of (by norm_num) hq (mul_nonneg ha hb)
  have hqhi : q < 2^127 := by
    by_contra h; replace h := not_lt.mp h
    have : 2^127 * 10^18 ≤ q * (10:Int)^18 := mul_le_mul_of_nonneg_right h (by norm_num)
    linarith [hq'.1]
  have h := mul_div_floor_spec env a b constants.production.shared.WAD (by rw [rp_wad_val]; norm_num)
    ⟨q, by rw [rp_wad_val]; exact hq, by omega, hqhi⟩
  rw [rp_wad_val] at h
  step with h as ⟨r, hr⟩
  simpa using hr

/-- `Wad::mul(a, b)` is `hu(a·b/W)` for `a, b ≥ 0` whenever that fits. -/
theorem rp_mul_spec (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 ≤ b.val)
    (hfit : huD (a.val * b.val) (10^18) < 2^127) :
    math.fp.Wad.mul a env b ⦃ q => q.val = huD (a.val * b.val) (10^18) ⦄ := by
  unfold math.fp.Wad.mul
  obtain ⟨q, hq⟩ := IsHalfUp.exists (a.val * b.val) (10^18) (by norm_num)
  have hqe : q = huD (a.val * b.val) (10^18) := isHalfUp_eq (by norm_num) hq
  have h := mul_div_half_up_spec env a b constants.production.shared.WAD ha hb
    (by rw [rp_wad_val]; norm_num) ⟨q, by rw [rp_wad_val]; exact hq, by rw [hqe]; exact hfit⟩
  rw [rp_wad_val] at h
  step with h as ⟨r, hr⟩
  rw [isHalfUp_eq (by norm_num) hr]

/-- `Wad::to_token_floor(a, dec)` for `dec ≤ 18`, `a ≥ 0` is `⌊a / 10^(18−dec)⌋`.
Never aborts. -/
theorem rp_to_token_floor_spec (env : Env) (a : math.fp.Wad) (dec : Std.U32) (hdec : dec.val ≤ 18)
    (ha : 0 ≤ a.val) :
    math.fp.Wad.to_token_floor a env dec ⦃ r => IsFloor a.val (10^(18 - dec.val)) r.val ⦄ := by
  unfold math.fp.Wad.to_token_floor
  rw [WAD_DECIMALS_eq]
  by_cases h18 : dec.val = 18
  · have he : (18#u32 : Std.U32) = dec := by apply UScalar.eq_of_val_eq; simp [h18]
    rw [rescale_floor_eq env a _ _ he, h18]
    simp only [Nat.sub_self, pow_zero, WP.spec_ok]
    exact isFloor_one _
  · have := rescale_floor_down_spec env a 18#u32 dec (by simp; omega)
    apply WP.spec_mono this
    intro r hr
    have := hr.1 ha
    simpa using this

end ControllerAeneas.Proofs
