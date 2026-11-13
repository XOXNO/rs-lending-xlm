-- Copied from RatesAeneas/Proofs/HalfUpWrappers.lean (2026-11-13 snapshot) for the pool
-- extraction (global_sync / accrue_step proofs), with `RatesAeneas`/`rates_aeneas`/
-- `aeneas_soroban_shim` renamed to `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim`,
-- `constants.production.` to `constants.`, and the theorem namespace
-- `RatesAeneas.Proofs` to `PoolAeneas.RatesProofs` (several names clash with
-- pool-local lemmas in `PoolAeneas.Proofs`) and `types.MarketParams` to
-- `types.pool.MarketParams`. Proof text otherwise unchanged.
import PoolAeneas.Proofs.HalfUpDivInt

/-
Half-up wrappers in common/src/math/fp.rs and the downscaling branch of
`fp_core::rescale_half_up`:

* `rescale_half_up` downscaling is `a / 10^(from - to)` rounded to nearest,
  ties away from zero, for every `i128` `a` and every decimal gap, including
  gaps whose power of ten overflows `i128` (that branch returns 0, which is the
  correct rounding).
* `Ray::to_asset`, `Ray::mul`, `Ray::div`, `Ray::try_div`, `Bps::apply_to`,
  `Bps::apply_to_ray`, `Bps::flash_loan_fee_on`.
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas
open PoolAeneas.Proofs

set_option maxRecDepth 20000

namespace PoolAeneas.RatesProofs

/-- `RAY` is `10^27` (`constants::RAY`). -/
theorem ray_val_hu : IScalar.val (ty := .I128) constants.shared.RAY = 10^27 := by
  unfold constants.shared.RAY; simp

/-- `BPS` is `10_000` (`constants::BPS`). -/
theorem bps_val : IScalar.val (ty := .I128) constants.shared.BPS = 10000 := by
  unfold constants.shared.BPS; simp

/-- A numerator below half the divisor in absolute value rounds to zero. -/
theorem isHalfAway_zero (a D : Int) (hD : 0 < D) (h : 2 * |a| < D) : IsHalfAway a D 0 := by
  unfold IsHalfAway IsHalfUp
  split_ifs with ha
  · rw [abs_of_nonneg ha] at h; constructor <;> linarith
  · rw [abs_of_neg (by omega)] at h; constructor <;> linarith

/-- **`rescale_half_up` downscaling rounds to nearest, ties away from zero.**
For every `i128` `a` and decimal counts `to < from`, the result is
`a / 10^(from - to)` rounded to nearest with halves away from zero, and the
call does not abort. When `10^(from - to)` overflows `i128` (gap ≥ 39) the
code returns 0, which is that rounding too (`|a| < 2^127 < 10^39 / 2`).
No hypothesis beyond `to < from`. -/
theorem rescale_half_up_down_spec (env : Env) (a : Std.I128) (fromD toD : Std.U32)
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
      apply isHalfAway_zero _ _ hxpos
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

/-- **`Ray::to_asset` rounds to nearest, ties away from zero.** For an asset
with fewer than 27 decimals (production: at most `MAX_ASSET_DECIMALS = 18`),
the token amount is `ray / 10^(27 - decimals)` rounded half away from zero,
for every `i128` ray value, and the call never aborts. -/
theorem ray_to_asset_spec (env : Env) (r : math.fp.Ray) (dec : Std.U32) (hdec : dec.val < 27) :
    math.fp.Ray.to_asset r env dec ⦃ q =>
      IsHalfAway (IScalar.val (ty := .I128) r) (10 ^ (27 - dec.val)) q.val ⦄ := by
  unfold math.fp.Ray.to_asset
  have hRD : constants.shared.RAY_DECIMALS = 27#u32 := by
    unfold constants.shared.RAY_DECIMALS; rfl
  rw [hRD]
  have h27 : (27#u32).val = 27 := by simp
  have := rescale_half_up_down_spec env r 27#u32 dec (by rw [h27]; exact hdec)
  rw [h27] at this
  exact this

/-- **`Ray::mul` is the half-up product.** For non-negative rays `a`, `b`
whose half-up product `a*b/RAY` fits `i128`, `Ray::mul` returns exactly it.
(Negative operands or an overflowing product abort, see
`mul_div_half_up_returns_iff`.) -/
theorem ray_mul_spec (env : Env) (a b : math.fp.Ray)
    (ha : 0 ≤ IScalar.val (ty := .I128) a) (hb : 0 ≤ IScalar.val (ty := .I128) b)
    (hfit : ∃ q : Int, IsHalfUp (IScalar.val (ty := .I128) a * IScalar.val (ty := .I128) b)
      (10^27) q ∧ q < 2^127) :
    math.fp.Ray.mul a env b ⦃ q =>
      IsHalfUp (IScalar.val (ty := .I128) a * IScalar.val (ty := .I128) b) (10^27)
        (IScalar.val (ty := .I128) q) ⦄ := by
  unfold math.fp.Ray.mul
  have h := mul_div_half_up_spec env a b constants.shared.RAY ha hb
    (by rw [ray_val_hu]; norm_num) (by rw [ray_val_hu]; exact hfit)
  rw [ray_val_hu] at h
  step with h as ⟨q, hq⟩
  exact hq

/-- **`Ray::div` is the half-up quotient.** For a non-negative ray `a` and a
positive ray `b` whose half-up quotient `a*RAY/b` fits `i128`, `Ray::div`
returns exactly it. -/
theorem ray_div_spec (env : Env) (a b : math.fp.Ray)
    (ha : 0 ≤ IScalar.val (ty := .I128) a) (hb : 0 < IScalar.val (ty := .I128) b)
    (hfit : ∃ q : Int, IsHalfUp (IScalar.val (ty := .I128) a * 10^27)
      (IScalar.val (ty := .I128) b) q ∧ q < 2^127) :
    math.fp.Ray.div a env b ⦃ q =>
      IsHalfUp (IScalar.val (ty := .I128) a * 10^27) (IScalar.val (ty := .I128) b)
        (IScalar.val (ty := .I128) q) ⦄ := by
  unfold math.fp.Ray.div
  have h := mul_div_half_up_spec env a constants.shared.RAY b ha
    (by rw [ray_val_hu]; norm_num) hb (by rw [ray_val_hu]; exact hfit)
  rw [ray_val_hu] at h
  step with h as ⟨q, hq⟩
  exact hq

-- `ray_try_div_spec` and `ray_try_mul_spec` are omitted. `utilization` no longer
-- calls `Ray::try_div` (it divides only below `supplied`, with `Ray::div`), so
-- the pool extraction does not contain it; RatesAeneas still proves it. The
-- pool's `Ray::try_mul` spec is `PoolAeneas.Proofs.ray_try_mul_spec`
-- (`Wrappers.lean`).

/-- Half-up of `x*y/d` with `0 ≤ y ≤ d` exists, is in `[0, x]`, and so fits. -/
theorem halfUp_ratio_fits (x y d : Int) (hd : 0 < d) (hx : 0 ≤ x) (hxb : x < 2^127)
    (hy : 0 ≤ y) (hyd : y ≤ d) : ∃ q, IsHalfUp (x * y) d q ∧ q < 2^127 := by
  obtain ⟨q, hq⟩ := IsHalfUp.exists (x * y) d hd
  exact ⟨q, hq, by have := (IsHalfUp.le_of_ratio_le_one hd hx hy hyd hq).2; omega⟩

/-- **`Bps::apply_to` is the half-up share, at most the amount.** For an amount
`≥ 0` and `0 ≤ bps ≤ BPS` (production: reserve factor `< BPS` and flash-loan
fee `≤ MAX_FLASHLOAN_FEE_BPS = 500`, both checked by
`InterestRateModel::verify`), `apply_to` never aborts and returns
`half_up(amount * bps / BPS)`, which lies in `[0, amount]`. -/
theorem bps_apply_to_spec (env : Env) (bps : math.fp.Bps) (amount : Std.I128)
    (hbps : 0 ≤ IScalar.val (ty := .I128) bps ∧ IScalar.val (ty := .I128) bps ≤ 10000)
    (hamt : 0 ≤ amount.val) :
    math.fp.Bps.apply_to bps env amount ⦃ q =>
      IsHalfUp (amount.val * IScalar.val (ty := .I128) bps) 10000 q.val ∧
      0 ≤ q.val ∧ q.val ≤ amount.val ⦄ := by
  unfold math.fp.Bps.apply_to
  have hab := amount.hBounds
  simp only [IScalarTy.numBits] at hab; norm_num at hab
  have h := mul_div_half_up_spec env amount bps constants.shared.BPS hamt hbps.1
    (by rw [bps_val]; norm_num)
    (by rw [bps_val]; exact halfUp_ratio_fits _ _ _ (by norm_num) hamt (by omega) hbps.1 hbps.2)
  rw [bps_val] at h
  apply WP.spec_mono h
  intro q hq
  exact ⟨hq, IsHalfUp.le_of_ratio_le_one (by norm_num) hamt hbps.1 hbps.2 hq⟩

/-- **`Bps::apply_to_ray` is the half-up share of a ray, at most the value.**
Same claim as `bps_apply_to_spec` for a ray value. Used with the reserve factor
(`calculate_supplier_rewards`) and the supplier share `BPS - reserve_factor`
(`calculate_deposit_rate`), both in `[0, BPS]`. -/
theorem bps_apply_to_ray_spec (env : Env) (bps : math.fp.Bps) (v : math.fp.Ray)
    (hbps : 0 ≤ IScalar.val (ty := .I128) bps ∧ IScalar.val (ty := .I128) bps ≤ 10000)
    (hv : 0 ≤ IScalar.val (ty := .I128) v) :
    math.fp.Bps.apply_to_ray bps env v ⦃ q =>
      IsHalfUp (IScalar.val (ty := .I128) v * IScalar.val (ty := .I128) bps) 10000
        (IScalar.val (ty := .I128) q) ∧
      0 ≤ IScalar.val (ty := .I128) q ∧ IScalar.val (ty := .I128) q ≤ IScalar.val (ty := .I128) v ⦄ := by
  unfold math.fp.Bps.apply_to_ray math.fp.Ray.raw
  have := bps_apply_to_spec env bps v hbps hv
  unfold math.fp.Bps.apply_to at this
  step with this as ⟨q, hq1, hq2, hq3⟩
  exact ⟨hq1, hq2, hq3⟩

/-- **`Bps::flash_loan_fee_on`: at least 1, at most the amount.** For an
amount `≥ 1` and `0 ≤ bps ≤ BPS` (production: `flashloan_fee ≤
MAX_FLASHLOAN_FEE_BPS = 500`, `InterestRateModel::verify`), the fee never
aborts, is at most the amount, is at least 1 whenever `bps > 0`, and equals
`half_up(amount * bps / BPS)` unless that is 0 with `bps > 0`, in which case
it is 1 (the minimum fee). -/
theorem flash_loan_fee_on_spec (env : Env) (bps : math.fp.Bps) (amount : Std.I128)
    (hbps : 0 ≤ IScalar.val (ty := .I128) bps ∧ IScalar.val (ty := .I128) bps ≤ 10000)
    (hamt : 1 ≤ amount.val) :
    math.fp.Bps.flash_loan_fee_on bps env amount ⦃ f =>
      f.val ≤ amount.val ∧ 0 ≤ f.val ∧
      (0 < IScalar.val (ty := .I128) bps → 1 ≤ f.val) ∧
      (∀ h, IsHalfUp (amount.val * IScalar.val (ty := .I128) bps) 10000 h →
        f.val = (if 0 < IScalar.val (ty := .I128) bps ∧ h = 0 then 1 else h)) ⦄ := by
  unfold math.fp.Bps.flash_loan_fee_on math.fp.Bps.raw
  step with bps_apply_to_spec env bps amount hbps (by omega) as ⟨fee, hf1, hf2, hf3⟩
  have uniq : ∀ h, IsHalfUp (amount.val * IScalar.val (ty := .I128) bps) 10000 h → h = fee.val :=
    fun h hh => IsHalfUp.unique (by norm_num) hh hf1
  by_cases hp : bps > 0#i128
  · have hp' : 0 < IScalar.val (ty := .I128) bps := by simpa [IScalar.lt_equiv] using hp
    simp only [hp, ↓reduceIte]
    by_cases hz : fee = 0#i128
    · have hz' : fee.val = 0 := by rw [hz]; simp
      simp only [hz, ↓reduceIte, WP.spec_ok]
      refine ⟨by simp; omega, by simp, fun _ => by simp, fun h hh => ?_⟩
      rw [uniq h hh, hz']; simp [hp']
    · have hz' : fee.val ≠ 0 := fun h => hz (IScalar.eq_of_val_eq (by simp [h]))
      simp only [hz, ↓reduceIte, WP.spec_ok]
      refine ⟨hf3, hf2, fun _ => by omega, fun h hh => ?_⟩
      rw [uniq h hh]; simp [hz']
  · have hp' : ¬ 0 < IScalar.val (ty := .I128) bps := by simpa [IScalar.lt_equiv] using hp
    simp only [hp, ↓reduceIte, WP.spec_ok]
    refine ⟨hf3, hf2, fun h => absurd h hp', fun h hh => ?_⟩
    rw [uniq h hh]; simp [hp']

end PoolAeneas.RatesProofs
