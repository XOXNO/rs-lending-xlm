-- Copied from RatesAeneas/Proofs/Scaling.lean (2026-11-12 snapshot) for the pool
-- extraction, with `RatesAeneas`/`rates_aeneas`/`aeneas_soroban_shim` renamed to
-- `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim` and `constants.production.` to
-- `constants.` (WRITING-PROOFS.md, last section). Proof text otherwise unchanged.
import PoolAeneas.Proofs.MulDiv

/-
Rate formulas in `common/src/rates/scaling.rs`.

`calculate_scaled_supply` (the shares minted for a supply) rounds **down**:
it returns exactly `⌊A * RAY / index⌋` with `A = amount * 10^(27 - decimals)`,
so the minted shares are never worth more than the deposit (ADR-0003).
-/

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec
open pool_aeneas

-- Normalising the 2^127 bounds of `i128` literals recurses deeply.
set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- `Ray::from_asset` for an asset with fewer than 27 decimals multiplies by
`10^(27 - decimals)` exactly, whenever the product fits `i128`. -/
theorem from_asset_spec (env : Env) (amount : Std.I128) (dec : Std.U32) (hdec : dec.val < 27)
    (hA : -2^127 ≤ amount.val * 10^(27 - dec.val) ∧ amount.val * 10^(27 - dec.val) < 2^127) :
    math.fp.Ray.from_asset env amount dec ⦃ r =>
      IScalar.val (ty := .I128) r = amount.val * 10^(27 - dec.val) ⦄ := by
  have hRD : constants.shared.RAY_DECIMALS = 27#u32 := by
    unfold constants.shared.RAY_DECIMALS; rfl
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
  -- 27 - decimals as a u32
  have hk := UScalar.tryMkOpt_eq .U32 (27 - dec.val)
  simp only [UScalar.tryMk]
  cases hko : UScalar.tryMkOpt .U32 (27 - dec.val) with
  | none =>
    exfalso; rw [hko] at hk; apply hk; simp [UScalarTy.numBits]; omega
  | some k =>
    rw [hko] at hk; obtain ⟨hkv, _⟩ := hk
    simp only [Result.ofOption, bind_ok, core.num.I128.checked_pow, h10]
    -- 10^(27 - decimals) fits i128
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

theorem ray_val : IScalar.val (ty := .I128) constants.shared.RAY = 10^27 := by
  unfold constants.shared.RAY; simp

/-- **`calculate_scaled_supply` is the exact floor.** For every amount, asset
decimal count below 27 and positive index, the shares minted for a supply are
exactly `⌊A * RAY / index⌋` with `A = amount * 10^(27 - decimals)`, whenever
`A` and that floor fit `i128`. The call does not abort in that case. -/
theorem calculate_scaled_supply_spec (env : Env) (amount : Std.I128) (dec : Std.U32)
    (index : Std.I128) (hdec : dec.val < 27) (hidx : 0 < index.val)
    (hA : -2^127 ≤ amount.val * 10^(27 - dec.val) ∧ amount.val * 10^(27 - dec.val) < 2^127)
    (hfit : ∃ q : Int, IsFloor (amount.val * 10^(27 - dec.val) * 10^27) index.val q ∧
      -2^127 ≤ q ∧ q < 2^127) :
    rates.scaling.calculate_scaled_supply env amount dec index ⦃ s =>
      IsFloor (amount.val * 10^(27 - dec.val) * 10^27) index.val
        (IScalar.val (ty := .I128) s) ⦄ := by
  unfold rates.scaling.calculate_scaled_supply math.fp.Ray.div_floor
  step with from_asset_spec env amount dec hdec hA as ⟨r, hr⟩
  step with mul_div_floor_spec env r constants.shared.RAY index (by omega)
    (by rw [hr, ray_val]; exact hfit) as ⟨q, hq⟩
  rw [hr, ray_val] at hq
  exact hq

/-- **Supply mints round in the protocol's favour (ADR-0003).** The minted
shares, valued at the index, are never worth more than the deposit
(`shares * index ≤ A * RAY`), and one more share would be (`A * RAY <
(shares + 1) * index`): no value is created or lost beyond one share. -/
theorem calculate_scaled_supply_favours_protocol (env : Env) (amount : Std.I128)
    (dec : Std.U32) (index : Std.I128) (hdec : dec.val < 27) (hidx : 0 < index.val)
    (hA : -2^127 ≤ amount.val * 10^(27 - dec.val) ∧ amount.val * 10^(27 - dec.val) < 2^127)
    (hfit : ∃ q : Int, IsFloor (amount.val * 10^(27 - dec.val) * 10^27) index.val q ∧
      -2^127 ≤ q ∧ q < 2^127) :
    rates.scaling.calculate_scaled_supply env amount dec index ⦃ s =>
      IScalar.val (ty := .I128) s * index.val ≤ amount.val * 10^(27 - dec.val) * 10^27 ∧
      amount.val * 10^(27 - dec.val) * 10^27 < (IScalar.val (ty := .I128) s + 1) * index.val ⦄ := by
  have h := calculate_scaled_supply_spec env amount dec index hdec hidx hA hfit
  apply WP.spec_mono h
  intro s hs
  unfold IsFloor at hs
  rw [if_pos hidx] at hs
  exact ⟨hs.1, by linarith [hs.2]⟩

end PoolAeneas.Proofs
