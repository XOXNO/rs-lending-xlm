/-
Value-form specifications of the fixed-point primitives that the per-leg loop
of `calculate_seized_collateral` (`contracts/controller/src/positions/liquidation/math.rs`)
calls, each stated as "returns exactly this closed form" with the closed forms
of `Totals.lean` (`huD` half-up, `/` floor, `clD` ceiling).

`sz_rescale_half_up_down_spec`, `sz_ray_to_asset_spec` and `sz_from_asset_spec`
are copies of `RatesAeneas/Proofs/HalfUpWrappers.lean`
(`rescale_half_up_down_spec`, `ray_to_asset_spec`) and
`RatesAeneas/Proofs/Scaling.lean` (`from_asset_spec`), taken 2026-11-12 with
the namespace changed and the names prefixed (`sz_`) so they do not collide
with other files of this library; see WRITING-PROOFS.md, last section.
`sz_resolve_withdrawal_val` restates `RatesAeneas/Proofs/Withdrawal.lean`
(`resolve_withdrawal_spec`) in value form and is re-proved here on the
`ControllerAeneas` extraction.

A plain (non-`module`) file: it uses the sealed checked operations.
-/
import ControllerAeneas.Proofs.Repayment
import ControllerAeneas.Proofs.Seizure
import ControllerAeneas.Proofs.Apply

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 1000000

namespace ControllerAeneas.Proofs

/-! ## Closed forms -/

/-- `huD n d` is the half-up quotient. -/
theorem sz_isHalfUp_huD (n d : Int) (hd : 0 < d) : IsHalfUp n d (huD n d) := by
  obtain ⟨q, hq⟩ := IsHalfUp.exists n d hd
  rw [← isHalfUp_eq hd hq]; exact hq

/-- `n / d` is the floor quotient. -/
theorem sz_isFloor_div (n d : Int) (hd : 0 < d) : IsFloor n d (n / d) := by
  obtain ⟨q, hq⟩ := isFloor_exists n d hd
  rw [← isFloor_eq hd hq]; exact hq

/-- `clD n d` is the ceiling quotient. -/
theorem sz_isCeil_clD (n d : Int) (hd : 0 < d) : IsCeil n d (clD n d) := by
  obtain ⟨q, hq⟩ := isCeil_exists n d hd
  rw [← isCeil_eq hd hq]; exact hq

/-- Bracket of the half-up quotient: `2·q·d ≤ 2n + d < 2·q·d + 2d`. -/
theorem sz_huD_bounds (n d : Int) (hd : 0 < d) :
    2 * (huD n d) * d ≤ 2 * n + d ∧ 2 * n + d < 2 * (huD n d) * d + 2 * d :=
  sz_isHalfUp_huD n d hd

/-- Bracket of the floor quotient. -/
theorem sz_div_bounds (n d : Int) (hd : 0 < d) : (n / d) * d ≤ n ∧ n < (n / d) * d + d :=
  (isFloor_pos hd).mp (sz_isFloor_div n d hd)

/-- Bracket of the ceiling quotient. -/
theorem sz_clD_bounds (n d : Int) (hd : 0 < d) : n ≤ (clD n d) * d ∧ (clD n d) * d < n + d :=
  (isCeil_pos hd).mp (sz_isCeil_clD n d hd)

theorem sz_huD_nonneg (n d : Int) (hd : 0 < d) (hn : 0 ≤ n) : 0 ≤ huD n d :=
  IsHalfUp.nonneg hd hn (sz_isHalfUp_huD n d hd)

theorem sz_div_nonneg (n d : Int) (hd : 0 < d) (hn : 0 ≤ n) : 0 ≤ n / d :=
  Int.ediv_nonneg hn (by omega)

theorem sz_clD_nonneg (n d : Int) (hd : 0 < d) (hn : 0 ≤ n) : 0 ≤ clD n d := by
  have := sz_clD_bounds n d hd
  by_contra h
  have : clD n d * d < 0 := mul_neg_of_neg_of_pos (by omega) hd
  omega

/-- `huD n d ≤ c` whenever `n ≤ c·d`. -/
theorem sz_huD_le_of (n d c : Int) (hd : 0 < d) (h : n ≤ c * d) : huD n d ≤ c :=
  IsHalfUp.le_of hd (sz_isHalfUp_huD n d hd) h

/-- `n / d ≤ c` whenever `n ≤ c·d`. -/
theorem sz_div_le_of (n d c : Int) (hd : 0 < d) (h : n ≤ c * d) : n / d ≤ c :=
  IsFloor.le_of hd (sz_isFloor_div n d hd) h

/-! ## Constants -/

theorem sz_wad_one_val : (math.fp.Wad.ONE : Std.I128).val = 10^18 := by
  unfold math.fp.Wad.ONE; exact WAD_val

theorem sz_wad_zero_val : (math.fp.Wad.ZERO : Std.I128).val = 0 := by
  unfold math.fp.Wad.ZERO; rfl

theorem sz_ray_zero_val : (math.fp.Ray.ZERO : Std.I128).val = 0 := by
  unfold math.fp.Ray.ZERO; rfl

theorem sz_min_dec_val : (constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS).val = 3 := by
  unfold constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS; rfl

/-- `RAY / WAD = 10^9` (the generated constant division). -/
theorem sz_ray_div_wad :
    constants.production.shared.RAY / constants.production.shared.WAD
      ⦃ r => r.val = 10^9 ⦄ := by
  have h1 := RAY_val
  have h2 := WAD_val
  step as ⟨r, hr⟩
  rw [hr, h1, h2]; norm_num

/-! ## Comparisons and `min` on `Ray` -/

@[simp] theorem sz_ray_le (a b : math.fp.Ray) :
    core.cmp.PartialOrd.le.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (a.val ≤ b.val)) := by
  simp only [core.cmp.PartialOrd.le.trait_default, core.cmp.PartialOrd.le.default,
    core.cmp.PartialOrd.le_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp,
    math.fp.Ray.Insts.CoreCmpOrd.cmp, core.cmp.impls.OrdI128.cmp, bind_ok, Option.some.injEq]
  congr 1
  rcases lt_trichotomy a.val b.val with h | h | h
  · simp [compare_lt_iff_lt.mpr h, h.le]
  · simp [compare_eq_iff_eq.mpr h, h.le]
  · simp [compare_gt_iff_gt.mpr h, not_le.mpr h]

@[simp] theorem sz_ray_min (a b : math.fp.Ray) :
    core.cmp.Ord.min.trait_default math.fp.Ray.Insts.CoreCmpOrd a b
      = ok (if b.val < a.val then b else a) := by
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default, core.cmp.Ord.min_body,
    math.fp.Ray.Insts.CoreCmpOrd, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    core.cmp.PartialOrd.lt.default, core.cmp.PartialOrd.lt_body,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    core.cmp.impls.OrdI128.cmp, bind_ok, Option.some.injEq]
  by_cases h : b.val < a.val
  · simp [compare_lt_iff_lt.mpr h]; intro h'; omega
  · simp [compare_lt_iff_lt, h]

@[simp] theorem sz_ray_eq (a b : math.fp.Ray) :
    math.fp.Ray.Insts.CoreCmpPartialEqRay.eq a b = ok (decide (a.val = b.val)) := by
  simp only [math.fp.Ray.Insts.CoreCmpPartialEqRay.eq]
  congr 1
  by_cases h : a = b
  · subst h; simp
  · have : a.val ≠ b.val := fun e => h (IScalar.eq_of_val_eq e)
    simp [h, this]

/-- `i128::min` returns the smaller value. -/
theorem sz_i128_min_val (a b : Std.I128) :
    (core.cmp.impls.OrdI128.min a b).val = min a.val b.val := by
  simp only [core.cmp.impls.OrdI128.min]
  split <;> rename_i h <;> simp [IScalar.lt_equiv] at h <;> omega

/-! ## `Ray` and `Wad` arithmetic in value form -/

/-- `Ray::mul` is `huD(a·b, RAY)` for non-negative operands whose result fits. -/
theorem sz_ray_mul_val (env : Env) (a b : math.fp.Ray) (ha : 0 ≤ a.val) (hb : 0 ≤ b.val)
    (hfit : huD (a.val * b.val) (10^27) < 2^127) :
    math.fp.Ray.mul a env b ⦃ r => r.val = huD (a.val * b.val) (10^27) ⦄ := by
  unfold math.fp.Ray.mul
  step with mul_div_half_up_spec env a b constants.production.shared.RAY ha hb
    (by rw [RAY_val]; norm_num)
    ⟨_, by rw [RAY_val]; exact sz_isHalfUp_huD _ _ (by norm_num), hfit⟩ as ⟨q, hq⟩
  rw [RAY_val] at hq; exact isHalfUp_eq (by norm_num) hq

/-- `Wad::mul` is `huD(a·b, WAD)` for non-negative operands whose result fits. -/
theorem sz_wad_mul_val (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 ≤ b.val)
    (hfit : huD (a.val * b.val) (10^18) < 2^127) :
    math.fp.Wad.mul a env b ⦃ r => r.val = huD (a.val * b.val) (10^18) ⦄ := by
  unfold math.fp.Wad.mul
  step with mul_div_half_up_spec env a b constants.production.shared.WAD ha hb
    (by rw [WAD_val]; norm_num)
    ⟨_, by rw [WAD_val]; exact sz_isHalfUp_huD _ _ (by norm_num), hfit⟩ as ⟨q, hq⟩
  rw [WAD_val] at hq; exact isHalfUp_eq (by norm_num) hq

/-- `Wad::div` is `huD(a·WAD, b)` for `a ≥ 0`, `b > 0` whose result fits. -/
theorem sz_wad_div_val (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 < b.val)
    (hfit : huD (a.val * 10^18) b.val < 2^127) :
    math.fp.Wad.div a env b ⦃ r => r.val = huD (a.val * 10^18) b.val ⦄ := by
  unfold math.fp.Wad.div
  step with mul_div_half_up_spec env a constants.production.shared.WAD b ha
    (by rw [WAD_val]; norm_num) hb
    ⟨_, by rw [WAD_val]; exact sz_isHalfUp_huD _ _ hb, hfit⟩ as ⟨q, hq⟩
  rw [WAD_val] at hq; exact isHalfUp_eq hb hq

/-- `Wad::div_floor` is `⌊a·WAD/b⌋` for `b > 0` whose result fits. -/
theorem sz_wad_div_floor_val (env : Env) (a b : math.fp.Wad) (hb : 0 < b.val)
    (hfit : -2^127 ≤ a.val * 10^18 / b.val ∧ a.val * 10^18 / b.val < 2^127) :
    math.fp.Wad.div_floor a env b ⦃ r => r.val = a.val * 10^18 / b.val ⦄ := by
  unfold math.fp.Wad.div_floor
  step with mul_div_floor_spec env a constants.production.shared.WAD b (by omega)
    ⟨_, by rw [WAD_val]; exact sz_isFloor_div _ _ hb, hfit.1, hfit.2⟩ as ⟨q, hq⟩
  rw [WAD_val] at hq; exact isFloor_eq hb hq

/-- `Ray::div_floor` is `⌊a·RAY/b⌋` for `b > 0` whose result fits. -/
theorem sz_ray_div_floor_val (env : Env) (a b : math.fp.Ray) (hb : 0 < b.val)
    (hfit : -2^127 ≤ a.val * 10^27 / b.val ∧ a.val * 10^27 / b.val < 2^127) :
    math.fp.Ray.div_floor a env b ⦃ r => r.val = a.val * 10^27 / b.val ⦄ := by
  step with ray_div_floor_spec env a b (by omega)
    ⟨_, sz_isFloor_div _ _ hb, hfit.1, hfit.2⟩ as ⟨q, hq⟩
  exact isFloor_eq hb hq

/-- `Ray::to_asset_floor` of a non-negative ray is `⌊a / 10^(27−dec)⌋`. -/
theorem sz_to_asset_floor_val (env : Env) (a : math.fp.Ray) (dec : Std.U32)
    (hdec : dec.val ≤ 27) (ha : 0 ≤ a.val) :
    math.fp.Ray.to_asset_floor a env dec ⦃ r => r.val = a.val / 10^(27 - dec.val) ⦄ := by
  step with ray_to_asset_floor_spec env a dec hdec as ⟨r, hr, _⟩
  exact isFloor_eq (by positivity) (hr ha)

/-- `Ray::to_asset_ceil` is `⌈a / 10^(27−dec)⌉`. -/
theorem sz_to_asset_ceil_val (env : Env) (a : math.fp.Ray) (dec : Std.U32)
    (hdec : dec.val ≤ 27) :
    math.fp.Ray.to_asset_ceil a env dec ⦃ r => r.val = clD a.val (10^(27 - dec.val)) ⦄ := by
  step with ray_to_asset_ceil_spec env a dec hdec as ⟨r, hr⟩
  exact isCeil_eq (by positivity) hr

/-- `Ray::to_wad` of a non-negative ray is `huD(a, 10^9)`. -/
theorem sz_to_wad_val (env : Env) (a : math.fp.Ray) (ha : 0 ≤ a.val) :
    math.fp.Ray.to_wad a env ⦃ r => r.val = huD a.val (10^9) ⦄ := by
  step with to_wad_spec env a ha as ⟨r, hr⟩
  exact isHalfUp_eq (by norm_num) hr

/-- `i128::saturating_mul` of a non-negative product: `min(a·b, i128::MAX)`. -/
theorem sz_sat_mul_val (a b : Std.I128) (h : 0 ≤ a.val * b.val) :
    core.num.I128.saturating_mul a b ⦃ r => r.val = min (a.val * b.val) (2^127 - 1) ⦄ := by
  unfold core.num.I128.saturating_mul IScalar.tryMk
  have e : max (-2^127) (min (2^127 - 1) (a.val * b.val)) = min (a.val * b.val) (2^127 - 1) := by
    rw [min_comm]; exact max_eq_right (by
      rcases le_total (a.val * b.val) (2^127 - 1) with h' | h'
      · rw [min_eq_left h']; omega
      · rw [min_eq_right h']; norm_num)
  rw [e]
  generalize hz : min (a.val * b.val) (2^127 - 1) = z
  have hz0 : 0 ≤ z ∧ z ≤ 2^127 - 1 := by
    rw [← hz]; exact ⟨le_min h (by norm_num), min_le_right _ _⟩
  have ht := IScalar.tryMkOpt_eq .I128 z
  cases ho : IScalar.tryMkOpt .I128 z with
  | none =>
    rw [ho] at ht; exfalso; apply ht
    simp only [IScalar.inBounds, IScalarTy.numBits]; omega
  | some y => rw [ho] at ht; simp [Result.ofOption, ht.1]

/-- `Bps::apply_to_ray` of a non-negative ray and a rate in `[0, BPS]` is
`huD(v·rate, BPS)`; never aborts. -/
theorem sz_apply_to_ray_val (env : Env) (f : math.fp.Bps) (v : math.fp.Ray)
    (hf0 : 0 ≤ f.val) (hf : f.val ≤ 10^4) (hv : 0 ≤ v.val) :
    math.fp.Bps.apply_to_ray f env v ⦃ r => r.val = huD (v.val * f.val) (10^4) ∧
      0 ≤ r.val ∧ r.val ≤ v.val ⦄ := by
  unfold math.fp.Bps.apply_to_ray math.fp.Ray.raw
  have hvb := v.hBounds
  simp only [IScalarTy.numBits] at hvb
  have hq := sz_isHalfUp_huD (v.val * f.val) (10^4) (by norm_num)
  have hle := IsHalfUp.le_of_ratio_le_one (by norm_num) hv hf0 hf hq
  simp only [bind_ok]
  step with mul_div_half_up_spec env v f constants.production.shared.BPS hv hf0
    (by rw [BPS_val]; norm_num) ⟨_, by rw [BPS_val]; exact hq, by omega⟩ as ⟨q, hq'⟩
  rw [BPS_val] at hq'
  have e := isHalfUp_eq (by norm_num) hq'
  refine ⟨e, ?_, ?_⟩ <;> rw [e] <;> omega

/-- `Ray::checked_sub` for `0 ≤ b ≤ a`. -/
theorem sz_ray_checked_sub_val (env : Env) (a b : math.fp.Ray) (hb : 0 ≤ b.val) (hab : b.val ≤ a.val) :
    math.fp.Ray.checked_sub a env b ⦃ r => r.val = a.val - b.val ⦄ := by
  unfold math.fp.Ray.checked_sub
  step with checked_sub_nonneg_spec env a b hb hab as ⟨c, hc⟩
  exact hc

/-- `Wad::checked_sub` for `0 ≤ b ≤ a`. -/
theorem sz_wad_checked_sub_val (env : Env) (a b : math.fp.Wad) (hb : 0 ≤ b.val) (hab : b.val ≤ a.val) :
    math.fp.Wad.checked_sub a env b ⦃ r => r.val = a.val - b.val ⦄ := by
  unfold math.fp.Wad.checked_sub
  step with checked_sub_nonneg_spec env a b hb hab as ⟨c, hc⟩
  exact hc

/-- `Wad::checked_add` when the sum fits. -/
theorem sz_wad_checked_add_val (env : Env) (a b : math.fp.Wad)
    (h : -2^127 ≤ a.val + b.val ∧ a.val + b.val < 2^127) :
    math.fp.Wad.checked_add a env b ⦃ r => r.val = a.val + b.val ⦄ := by
  unfold math.fp.Wad.checked_add
  step with checked_add_raw_spec env a b h as ⟨c, hc⟩
  exact hc

/-! ## Half-up rescaling (copied from `RatesAeneas/Proofs/HalfUpWrappers.lean`) -/

/-- Rounding to nearest of a value smaller than half the divisor is 0. -/
theorem sz_isHalfAway_zero (a D : Int) (hD : 0 < D) (h : 2 * |a| < D) : IsHalfAway a D 0 := by
  unfold IsHalfAway IsHalfUp
  split
  · rename_i ha
    rw [abs_of_nonneg ha] at h
    constructor <;> nlinarith
  · rename_i ha
    rw [abs_of_neg (by omega)] at h
    constructor <;> nlinarith

/-- **`rescale_half_up` downscaling** is `a / 10^(from - to)` rounded to
nearest with halves away from zero, and the call does not abort (copy of
`RatesAeneas.Proofs.rescale_half_up_down_spec`). -/
theorem sz_rescale_half_up_down_spec (env : Env) (a : Std.I128) (fromD toD : Std.U32)
    (h : toD.val < fromD.val) :
    math.fp_core.rescale_half_up env a fromD toD ⦃ q =>
      IsHalfAway a.val (10 ^ (fromD.val - toD.val)) q.val ⦄ := by
  unfold math.fp_core.rescale_half_up math.fp_core.rescale
  have hne : ¬ (fromD = toD) := by intro he; rw [he] at h; omega
  have hgt : ¬ (toD > fromD) := by simp only [gt_iff_lt, UScalar.lt_equiv]; omega
  have hle : ¬ (fromD.val ≤ toD.val) := by omega
  simp only [hne, hgt, ↓reduceIte, core.num.U32.abs_diff, hle]
  have hab := a.hBounds
  simp only [IScalarTy.numBits] at hab
  norm_num at hab
  have hk := UScalar.tryMkOpt_eq .U32 (fromD.val - toD.val)
  simp only [UScalar.tryMk]
  cases hko : UScalar.tryMkOpt .U32 (fromD.val - toD.val) with
  | none =>
    exfalso; rw [hko] at hk; apply hk
    have := fromD.hBounds
    simp only [UScalar.inBounds, UScalarTy.numBits] at this ⊢; omega
  | some k =>
    rw [hko] at hk; obtain ⟨hkv, _⟩ := hk
    simp only [Result.ofOption, bind_ok, core.num.I128.checked_pow]
    have h10 : (10#i128).val = 10 := by simp
    rw [h10]
    have hxv : (10 : Int) ^ k.val = 10 ^ (fromD.val - toD.val) := by rw [hkv]
    generalize hx : (10 : Int) ^ k.val = x at hxv
    have hxpos : 0 < x := by rw [hxv]; positivity
    have hp := IScalar.tryMkOpt_eq .I128 x
    cases hpo : IScalar.tryMkOpt .I128 x with
    | none =>
      rw [hpo] at hp
      simp only [math.fp_core.rescale_half_up.closure.Insts.CoreOpsFunctionFnPairI128OptionI128I128.call]
      simp only [IScalar.inBounds, IScalarTy.numBits] at hp
      norm_num at hp
      have hbig0 : 2^127 ≤ x := hp (by omega)
      have hbig : 2^128 < x := by
        rw [hxv] at hbig0 ⊢
        by_cases hn : fromD.val - toD.val ≤ 38
        · have : (10:Int) ^ (fromD.val - toD.val) ≤ 10 ^ 38 :=
            pow_le_pow_right₀ (by norm_num) hn
          norm_num at this hbig0; omega
        · have : (10:Int) ^ 39 ≤ 10 ^ (fromD.val - toD.val) :=
            pow_le_pow_right₀ (by norm_num) (by omega)
          norm_num at this ⊢; omega
      rw [← hxv]
      show (ok 0#i128 : Result Std.I128) ⦃ q => IsHalfAway a.val x q.val ⦄
      simp only [WP.spec_ok]
      have : (IScalar.val (ty := .I128) 0#i128) = 0 := by simp
      rw [this]
      apply sz_isHalfAway_zero _ _ hxpos
      have : |a.val| ≤ 2^127 := abs_le.mpr ⟨by omega, by omega⟩
      have h2 : (2:Int)^128 = 2 * 2^127 := by norm_num
      omega
    | some f =>
      rw [hpo] at hp; obtain ⟨hfv, _⟩ := hp
      simp only [math.fp_core.rescale_half_up.closure.Insts.CoreOpsFunctionFnPairI128OptionI128I128.call]
      have hf : 0 < f.val := by rw [hfv]; exact hxpos
      apply WP.spec_mono (div_by_int_half_up_spec env a f hf)
      intro q hq
      rw [← hxv, ← hfv]; exact hq

/-- `Ray::to_asset` of a non-negative ray is `huD(a, 10^(27−dec))`; never
aborts (copy of `RatesAeneas.Proofs.ray_to_asset_spec`, in value form). -/
theorem sz_to_asset_val (env : Env) (r : math.fp.Ray) (dec : Std.U32) (hdec : dec.val < 27)
    (hr : 0 ≤ r.val) :
    math.fp.Ray.to_asset r env dec ⦃ q => q.val = huD r.val (10 ^ (27 - dec.val)) ⦄ := by
  unfold math.fp.Ray.to_asset
  rw [RAY_DECIMALS_eq]
  have h27 : (27#u32 : Std.U32).val = 27 := by simp
  have := sz_rescale_half_up_down_spec env r 27#u32 dec (by rw [h27]; exact hdec)
  rw [h27] at this
  apply WP.spec_mono this
  intro q hq
  unfold IsHalfAway at hq; rw [if_pos hr] at hq
  exact isHalfUp_eq (by positivity) hq

/-- `Ray::from_asset` multiplies by `10^(27 − dec)` exactly whenever the
product fits (copy of `RatesAeneas.Proofs.from_asset_spec`). -/
theorem sz_from_asset_val (env : Env) (amount : Std.I128) (dec : Std.U32) (hdec : dec.val < 27)
    (hA : -2^127 ≤ amount.val * 10^(27 - dec.val) ∧ amount.val * 10^(27 - dec.val) < 2^127) :
    math.fp.Ray.from_asset env amount dec ⦃ r =>
      IScalar.val (ty := .I128) r = amount.val * 10^(27 - dec.val) ⦄ := by
  have hRD : constants.production.shared.RAY_DECIMALS = 27#u32 := RAY_DECIMALS_eq
  unfold math.fp.Ray.from_asset math.fp_core.rescale_half_up math.fp_core.rescale
  rw [hRD]
  have hne : ¬ (dec = 27#u32) := by
    intro h; rw [h] at hdec; simp at hdec
  have hgt : (27#u32 > dec) := by
    simp only [gt_iff_lt, UScalar.lt_equiv]; simpa using hdec
  simp only [hne, hgt, ↓reduceIte, core.num.U32.abs_diff]
  have h27 : (27#u32).val = 27 := by simp
  have hle : dec.val ≤ 27 := by omega
  simp only [h27]
  simp only [hle, ↓reduceIte]
  have h10 : (10#i128).val = 10 := by simp
  have hpow : (10 : Int) ^ (27 - dec.val) ≤ 10 ^ 27 :=
    pow_le_pow_right₀ (by norm_num) (by omega)
  have hk := UScalar.tryMkOpt_eq .U32 (27 - dec.val)
  simp only [UScalar.tryMk]
  cases hko : UScalar.tryMkOpt .U32 (27 - dec.val) with
  | none =>
    exfalso; rw [hko] at hk; apply hk; simp [UScalarTy.numBits]; omega
  | some k =>
    rw [hko] at hk; obtain ⟨hkv, _⟩ := hk
    simp only [Result.ofOption, bind_ok, core.num.I128.checked_pow, h10]
    have hxv : (10 : Int) ^ k.val = 10 ^ (27 - dec.val) := by rw [hkv]
    generalize hx : (10 : Int) ^ k.val = x
    rw [hx] at hxv
    have hp := IScalar.tryMkOpt_eq .I128 x
    cases hpo : IScalar.tryMkOpt .I128 x with
    | none =>
      exfalso; rw [hpo] at hp; apply hp
      simp only [IScalar.inBounds, IScalarTy.numBits]; rw [hxv]
      have hlt : (10 : Int) ^ 27 < 2 ^ 127 := by norm_num
      have hpos := pow_pos (show (0:Int) < 10 by norm_num) (27 - dec.val)
      norm_num
      constructor <;> linarith
    | some f =>
      rw [hpo] at hp; obtain ⟨hfv, _⟩ := hp
      rw [hxv] at hfv
      simp only [bind_ok, core.option.Option.and_then,
        math.fp_core.rescale.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128,
        math.fp_core.rescale.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
      rw [show I128.checked_mul amount f = core.num.checked_mul_IScalar amount f from rfl]
      have hm := core.num.checked_mul_IScalar_bv_spec amount f
      cases hmo : core.num.checked_mul_IScalar amount f with
      | none =>
        exfalso; rw [hmo] at hm; apply hm; rw [hfv]
        simp only [IScalar.min, IScalar.max, IScalarTy.numBits]; omega
      | some r =>
        rw [hmo] at hm; obtain ⟨_, _, hr, _⟩ := hm
        simp only [core.option.Option.unwrap_or_else, bind_ok, WP.spec_ok]
        rw [hr, hfv]

/-! ## `resolve_withdrawal` (value form) -/

/-- **`resolve_withdrawal` in value form.** With `A = huD(pos·index, RAY)`
(`Ray::mul`), `H = huD(A, 10^k)` the half-up held amount (`unscale_supply`)
and `F = ⌊⌊pos·index/RAY⌋/10^k⌋` the floor held amount
(`unscale_supply_floor`), `k = 27 − dec`: the call returns (does not abort)
`(burn, paid)` with `paid = F` when `H ≤ amount` and `paid = amount`
otherwise, and in both cases `0 ≤ paid ≤ amount`, `paid·10^k ≤ ⌊pos·index/RAY⌋`.

Hypotheses: `dec < 27` (listed assets have ≤ 18 decimals), `0 < index`
(supply index ≥ its floor), `0 ≤ pos` (INV-ACCT-01), `0 ≤ amount`, and the
half-up `A` fits `i128` (the exact abort condition of the first call, see
`RatesAeneas.Proofs.resolve_withdrawal_returns_iff`). Restated from
`RatesAeneas/Proofs/Withdrawal.lean` and re-proved on this extraction. -/
theorem sz_resolve_withdrawal_val (env : Env) (amount pos index : Std.I128) (dec : Std.U32)
    (hdec : dec.val < 27) (hidx : 0 < index.val) (hpos : 0 ≤ pos.val) (hamt : 0 ≤ amount.val)
    (hfit : huD (pos.val * index.val) (10^27) < 2^127) :
    rates.scaling.resolve_withdrawal env amount pos index dec ⦃ p =>
      p.2.val = (if huD (huD (pos.val * index.val) (10^27)) (10^(27 - dec.val)) ≤ amount.val
        then pos.val * index.val / 10^27 / 10^(27 - dec.val) else amount.val) ∧
      0 ≤ p.2.val ∧ p.2.val ≤ amount.val ∧
      p.2.val * 10^(27 - dec.val) ≤ pos.val * index.val / 10^27 ⦄ := by
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - dec.val)
  have hN : 0 ≤ pos.val * index.val := mul_nonneg hpos (by omega)
  unfold rates.scaling.resolve_withdrawal rates.scaling.unscale_supply
    rates.scaling.scaled_to_original
  step with sz_ray_mul_val env pos index hpos (by omega) hfit as ⟨r0, hr0⟩
  have hr0nn : 0 ≤ r0.val := by rw [hr0]; exact sz_huD_nonneg _ _ (by norm_num) hN
  step with sz_to_asset_val env r0 dec hdec hr0nn as ⟨H0, hH0⟩
  -- the floor intermediate fits
  have hfl : pos.val * index.val / 10^27 ≤ huD (pos.val * index.val) (10^27) := by
    have h1 := sz_div_bounds (pos.val * index.val) (10^27) (by norm_num)
    have h2 := sz_huD_bounds (pos.val * index.val) (10^27) (by norm_num)
    by_contra h
    have : (huD (pos.val * index.val) (10^27) + 1) * 10^27 ≤
        (pos.val * index.val / 10^27) * 10^27 := mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    nlinarith
  have hfl0 : 0 ≤ pos.val * index.val / 10^27 := sz_div_nonneg _ _ (by norm_num) hN
  step with unscale_supply_floor_spec env pos index dec hpos (by omega) hdec
    (by
      have h2 := sz_huD_bounds (pos.val * index.val) (10^27) (by norm_num)
      nlinarith) as ⟨F0, m, hm, hF0⟩
  have hme : m = pos.val * index.val / 10^27 := isFloor_eq (by norm_num) hm
  have hF0e : F0.val = pos.val * index.val / 10^27 / 10^(27 - dec.val) := by
    rw [isFloor_eq hk hF0, hme]
  have hF0b := sz_div_bounds (pos.val * index.val / 10^27) (10^(27 - dec.val)) hk
  rw [← hF0e] at hF0b
  have hF0nn : 0 ≤ F0.val := by rw [hF0e]; exact sz_div_nonneg _ _ hk hfl0
  have hFH : F0.val ≤ H0.val := by
    rw [hH0, hr0]
    have h3 := sz_huD_bounds (huD (pos.val * index.val) (10^27)) (10^(27 - dec.val)) hk
    by_contra h
    have : (huD (huD (pos.val * index.val) (10 ^ 27)) (10 ^ (27 - dec.val)) + 1) *
        10^(27 - dec.val) ≤ F0.val * 10^(27 - dec.val) :=
      mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith
  by_cases hge : H0.val ≤ amount.val
  · have hge' : (amount >= H0) := by simp [IScalar.le_equiv]; omega
    simp only [hge', ↓reduceIte, WP.spec_ok]
    rw [← hr0, ← hH0, if_pos hge, ← hF0e]
    exact ⟨rfl, hF0nn, by omega, hF0b.1⟩
  · have hlt' : ¬ (amount >= H0) := by simp [IScalar.le_equiv]; omega
    simp only [hlt', ↓reduceIte]
    -- `amount < H` gives `amount·10^k ≤ A`
    have hAD : amount.val * 10^(27 - dec.val) ≤ r0.val := by
      have h3 := sz_huD_bounds r0.val (10^(27 - dec.val)) hk
      rw [← hH0] at h3
      have : (amount.val + 1) * 10^(27 - dec.val) ≤ H0.val * 10^(27 - dec.val) :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    -- and `amount·10^k ≤ ⌊N/RAY⌋`
    have hAF : amount.val * 10^(27 - dec.val) ≤ pos.val * index.val / 10^27 := by
      have h3 := sz_huD_bounds r0.val (10^(27 - dec.val)) hk
      rw [← hH0] at h3
      have h4 : (amount.val + 1) * 10^(27 - dec.val) ≤ H0.val * 10^(27 - dec.val) :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      have h5 : 2 * (amount.val * 10^(27 - dec.val)) + 10^(27 - dec.val) ≤ 2 * r0.val := by
        nlinarith
      have h6 := sz_huD_bounds (pos.val * index.val) (10^27) (by norm_num)
      rw [← hr0] at h6
      have h7 := sz_div_bounds (pos.val * index.val) (10^27) (by norm_num)
      -- `2·A·RAY ≤ 2N + RAY` and `N < (⌊N/RAY⌋ + 1)·RAY`
      by_contra h
      have h8 : (pos.val * index.val / 10^27 + 1) * 10^27 ≤
          amount.val * 10^(27 - dec.val) * 10^27 :=
        mul_le_mul_of_nonneg_right (by omega) (by norm_num)
      have h9 : (2 * (amount.val * 10^(27 - dec.val)) + 10^(27 - dec.val)) * 10^27 ≤
          2 * r0.val * 10^27 := mul_le_mul_of_nonneg_right h5 (by norm_num)
      nlinarith
    have hrb := r0.hBounds
    simp only [IScalarTy.numBits] at hrb
    have hA0 : 0 ≤ amount.val * 10^(27 - dec.val) := mul_nonneg hamt hk.le
    unfold rates.scaling.calculate_scaled_supply_ceil
    step with sz_from_asset_val env amount dec hdec ⟨by omega, by omega⟩ as ⟨a1, ha1⟩
    -- the ceiling burn fits: `⌈a1·RAY/index⌉ ≤ pos`
    obtain ⟨c, hc⟩ := isCeil_exists (a1.val * 10^27) index.val hidx
    have hcc := (isCeil_pos hidx).mp hc
    have hcpos : c ≤ pos.val := by
      have h7 := sz_div_bounds (pos.val * index.val) (10^27) (by norm_num)
      by_contra h
      have : (pos.val + 1) * index.val ≤ c * index.val :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      have : a1.val * 10^27 ≤ pos.val * index.val := by
        rw [ha1]
        have := mul_le_mul_of_nonneg_right hAF (show (0:Int) ≤ 10^27 by norm_num)
        nlinarith
      nlinarith
    have hc0 : 0 ≤ c := by
      by_contra h
      have : c * index.val < 0 := mul_neg_of_neg_of_pos (by omega) hidx
      have : 0 ≤ a1.val * 10^27 := by rw [ha1]; positivity
      linarith [hcc.1]
    have hposb := pos.hBounds
    simp only [IScalarTy.numBits] at hposb
    step with ray_div_ceil_spec env a1 index (by omega) ⟨c, hc, by omega, by omega⟩ as ⟨s, hs⟩
    rw [← hr0, ← hH0, if_neg hge]
    exact ⟨rfl, hamt, le_refl _, hAF⟩

end ControllerAeneas.Proofs
