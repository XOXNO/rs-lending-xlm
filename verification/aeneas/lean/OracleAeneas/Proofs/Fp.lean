/-
The typed fixed-point wrappers of `common/src/math/fp.rs` that the oracle
code goes through since #224 (typed fixed-point boundary):
`Wad::try_value_of_token` (`lp.rs::reserve_value_wad`),
`Wad::try_from_token_upscale` (`observation.rs::try_amount_to_wad` and
`try_normalize_positive_price`), and `Wad::try_mul`. All three are total over
their whole domain: they never abort and return `None` on every invalid input
or overflow.
-/
import OracleAeneas.Proofs.HalfUp

/- A plain (non-`module`) file: `I128.checked_mul` is only reachable from
non-module files (see WRITING-PROOFS.md). -/

open Aeneas Aeneas.Std Result
open OracleAeneas.Spec
open oracle_aeneas

set_option maxRecDepth 20000

namespace OracleAeneas.Proofs

/-- **`Wad::try_value_of_token` is the half-up token value or `None`.** For
every WAD price `p` (`self`), token amount `a` and decimal count `dec` it
returns (never aborts) either `Some w` with `w = a * p / 10^dec` rounded half
up (stated as `IsHalfUp`), only when `a ≥ 0`, `p ≥ 0` and `10^dec` fits
`i128`, or `None`, only when `10^dec` overflows `i128`, `a` or `p` is
negative, or the value does not fit `i128`. The amount is not normalised to
WAD first: there is a single rounding, of `a * p` by `10^dec`. -/
theorem Wad_try_value_of_token_spec (p : math.fp.Wad) (env : aeneas_oracle_shim.Env)
    (a : Std.I128) (dec : Std.U32) :
    math.fp.Wad.try_value_of_token p env a dec ⦃ o =>
      match o with
      | some w => 0 ≤ a.val ∧ 0 ≤ p.val ∧ (10 : Int)^dec.val < 2^127 ∧
          IsHalfUp (a.val * p.val) (10^dec.val) w.val
      | none => (2 : Int)^127 ≤ 10^dec.val ∨ a.val < 0 ∨ p.val < 0 ∨
          ∀ q : Int, IsHalfUp (a.val * p.val) (10^dec.val) q → 2^127 ≤ q ⦄ := by
  unfold math.fp.Wad.try_value_of_token
  simp only [core.num.I128.checked_pow]
  have h10 : (10#i128 : Std.I128).val = 10 := by simp
  rw [h10]
  generalize hx : (10 : Int) ^ dec.val = x
  have hxpos : 0 < x := by rw [← hx]; positivity
  have hp := IScalar.tryMkOpt_eq .I128 x
  cases hpo : IScalar.tryMkOpt .I128 x with
  | none =>
    rw [hpo] at hp
    simp only [IScalar.inBounds, IScalarTy.numBits] at hp
    simp only [bind_ok, core.option.Option.Insts.CoreOpsTry_traitTry.branch,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual,
      WP.spec_ok]
    left
    by_contra h; apply hp; constructor <;> omega
  | some f =>
    rw [hpo] at hp; obtain ⟨hf, hfb⟩ := hp
    simp only [IScalar.inBounds, IScalarTy.numBits] at hfb
    simp only [bind_ok, core.option.Option.Insts.CoreOpsTry_traitTry.branch]
    by_cases hneg : a.val < 0 ∨ p.val < 0
    · step with try_mul_div_half_up_rejects env a p f
        (by rcases hneg with h | h <;> simp [h]) as ⟨o, ho⟩
      subst ho
      simp only [core.option.Option.map_none, WP.spec_ok]
      rcases hneg with h | h
      · exact Or.inr (Or.inl h)
      · exact Or.inr (Or.inr (Or.inl h))
    · step with try_mul_div_half_up_spec env a p f (by omega) (by omega) (by omega)
        as ⟨o, ho⟩
      rw [hf] at ho
      cases o with
      | none =>
        simp only [core.option.Option.map_none, WP.spec_ok]
        exact Or.inr (Or.inr (Or.inr ho))
      | some q =>
        simp only [core.option.Option.map_some, P.Insts.CoreOpsFunctionFnOnceTupleI128Wad,
          P.Insts.CoreOpsFunctionFnOnceTupleI128Wad.call_once, math.fp.Wad.constructor,
          bind_tc_ok, WP.spec_ok]
        exact ⟨by omega, by omega, by omega, ho⟩

/-- `10i128.checked_pow(e)` for `e ≤ 18` is `Some(10^e)`. -/
theorem checked_pow_ten_le_18 (e : Std.U32) (h : e.val ≤ 18) :
    ∃ f : Std.I128, core.num.I128.checked_pow 10#i128 e = ok (some f) ∧
      f.val = 10 ^ e.val := by
  unfold core.num.I128.checked_pow
  have h10 : (10#i128 : Std.I128).val = 10 := by simp
  rw [h10]
  generalize hx : (10 : Int) ^ e.val = x
  have hxle : x ≤ 10^18 := by
    rw [← hx]; exact pow_le_pow_right₀ (by norm_num) h
  have hxpos : 0 < x := by rw [← hx]; positivity
  have hp := IScalar.tryMkOpt_eq .I128 x
  cases hpo : IScalar.tryMkOpt .I128 x with
  | none =>
    exfalso; rw [hpo] at hp; apply hp
    simp only [IScalar.inBounds, IScalarTy.numBits]; constructor <;> omega
  | some f =>
    rw [hpo] at hp
    exact ⟨f, rfl, hp.1⟩

/-- **`Wad::try_from_token_upscale` is the exact upscale or `None`.** For
every `i128` amount `a` and `u32` decimal count `dec` it returns (never
aborts) either `Some w` with `w = a * 10^(18 - dec)` exactly, only when
`a ≥ 0` and `dec ≤ 18`, or `None`, only when `a` is negative, `dec > 18`, or
the product does not fit `i128`. -/
theorem Wad_try_from_token_upscale_spec (a : Std.I128) (dec : Std.U32) :
    math.fp.Wad.try_from_token_upscale a dec ⦃ o =>
      match o with
      | some w => 0 ≤ a.val ∧ dec.val ≤ 18 ∧ w.val = a.val * 10^(18 - dec.val)
      | none => a.val < 0 ∨ 18 < dec.val ∨ 2^127 ≤ a.val * 10^(18 - dec.val) ⦄ := by
  unfold math.fp.Wad.try_from_token_upscale
  by_cases hneg : a < 0#i128
  · have : a.val < 0 := by simpa [IScalar.lt_equiv] using hneg
    simp only [hneg, ↓reduceIte, WP.spec_ok]
    exact Or.inl this
  have hnn : 0 ≤ a.val := by
    have : ¬ a.val < 0 := by simpa [IScalar.lt_equiv] using hneg
    omega
  simp only [hneg, ↓reduceIte]
  have hW : constants.production.shared.WAD_DECIMALS = 18#u32 := by
    unfold constants.production.shared.WAD_DECIMALS; rfl
  rw [hW]
  have hsub := U32.checked_sub_bv_spec 18#u32 dec
  have h18 : (18#u32 : Std.U32).val = 18 := by simp
  simp only [lift, bind_ok]
  cases hs : U32.checked_sub 18#u32 dec with
  | none =>
    rw [hs] at hsub
    simp only at hsub
    rw [h18] at hsub
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual,
      WP.spec_ok]
    exact Or.inr (Or.inl hsub)
  | some e =>
    rw [hs] at hsub
    obtain ⟨hle, he, _⟩ := hsub
    rw [h18] at hle he
    obtain ⟨f, hpf, hf⟩ := checked_pow_ten_le_18 e (by omega)
    rw [he] at hf
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok, hpf]
    rw [show I128.checked_mul a f = core.num.checked_mul_IScalar a f from rfl]
    have hm := core.num.checked_mul_IScalar_bv_spec a f
    have hprod : 0 ≤ a.val * f.val := by rw [hf]; positivity
    cases hmo : core.num.checked_mul_IScalar a f with
    | none =>
      rw [hmo] at hm
      simp only [IScalar.min, IScalar.max, IScalarTy.numBits] at hm
      simp only [core.option.Option.map_none, WP.spec_ok]
      right; right
      rw [hf] at hm hprod
      omega
    | some r =>
      rw [hmo] at hm; obtain ⟨_, _, hr, _⟩ := hm
      simp only [core.option.Option.map_some, P.Insts.CoreOpsFunctionFnOnceTupleI128Wad,
        P.Insts.CoreOpsFunctionFnOnceTupleI128Wad.call_once, math.fp.Wad.constructor,
        bind_tc_ok, WP.spec_ok]
      refine ⟨hnn, by omega, ?_⟩
      show r.val = _
      rw [hr, hf]

/-- **`Wad::try_mul` is the half-up WAD product or `None`.** For WAD values
`x`, `y` it returns (never aborts) `Some w` with `w = x * y / 10^18` rounded
half up, only when both are non-negative, or `None`, only when one is
negative or the product does not fit `i128`. -/
theorem Wad_try_mul_spec (x : math.fp.Wad) (env : aeneas_oracle_shim.Env) (y : math.fp.Wad) :
    math.fp.Wad.try_mul x env y ⦃ o =>
      match o with
      | some w => 0 ≤ x.val ∧ 0 ≤ y.val ∧ IsHalfUp (x.val * y.val) (10^18) w.val
      | none => x.val < 0 ∨ y.val < 0 ∨
          ∀ q : Int, IsHalfUp (x.val * y.val) (10^18) q → 2^127 ≤ q ⦄ := by
  unfold math.fp.Wad.try_mul
  have hW : (constants.production.shared.WAD : Std.I128).val = 10^18 := by
    unfold constants.production.shared.WAD; simp
  by_cases hneg : x.val < 0 ∨ y.val < 0
  · step with try_mul_div_half_up_rejects env x y constants.production.shared.WAD
      (by rcases hneg with h | h <;> simp [h]) as ⟨o, ho⟩
    subst ho
    simp only [core.option.Option.map_none, WP.spec_ok]
    rcases hneg with h | h
    · exact Or.inl h
    · exact Or.inr (Or.inl h)
  · step with try_mul_div_half_up_spec env x y constants.production.shared.WAD (by omega)
      (by omega) (by rw [hW]; norm_num) as ⟨o, ho⟩
    rw [hW] at ho
    cases o with
    | none =>
      simp only [core.option.Option.map_none, WP.spec_ok]
      exact Or.inr (Or.inr ho)
    | some q =>
      simp only [core.option.Option.map_some, P.Insts.CoreOpsFunctionFnOnceTupleI128Wad,
        P.Insts.CoreOpsFunctionFnOnceTupleI128Wad.call_once, math.fp.Wad.constructor,
        bind_tc_ok, WP.spec_ok]
      exact ⟨by omega, by omega, ho⟩

end OracleAeneas.Proofs
