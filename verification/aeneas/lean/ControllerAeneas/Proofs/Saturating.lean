/-
`fp_core::mul_div_floor_saturating` returns exactly `⌊x*y/d⌋`, clamped to the
`i128` range, for every `i128` input with `d ≠ 0` (native and `I256` paths).
Needed for the health factor of `calculate_account_risk_totals`
(`Wad::div_floor_saturating`), which the controller extraction now reaches.

Snapshot of the `mul_div_floor_saturating` section of
`RatesAeneas/Proofs/FloorCeilCore.lean` (from `IsFloor.neg_iff` to the end),
taken 2026-11-12, namespace changed; see WRITING-PROOFS.md, last section.
A plain (non-`module`) file for the same reason as `MulDiv.lean`.
-/
import ControllerAeneas.Proofs.MulDiv
import ControllerAeneas.Proofs.FloorCeilSpec

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec
open controller_aeneas

namespace ControllerAeneas.Proofs

/-! ## `mul_div_floor_saturating` -/

/-- The sign of a floor: `⌊n/d⌋ < 0` exactly when the exact quotient is
negative, that is `n ≠ 0` and `n`, `d` have opposite signs. -/
theorem IsFloor.neg_iff {n d f : Int} (hd : d ≠ 0) (hf : IsFloor n d f) :
    f < 0 ↔ (n ≠ 0 ∧ ((n < 0) ≠ (d < 0))) := by
  unfold IsFloor at hf
  rcases lt_or_gt_of_ne hd with hneg | hpos
  · rw [if_neg (by omega)] at hf
    constructor
    · intro hf0
      have : 0 ≤ f * d + d := by nlinarith
      refine ⟨by omega, ?_⟩
      simp only [ne_eq, eq_iff_iff, hneg, iff_true]; omega
    · rintro ⟨hn0, hs⟩
      have hn : 0 < n := by simp [hneg] at hs; omega
      by_contra hf0
      have : f * d ≤ 0 := by nlinarith
      omega
  · rw [if_pos hpos] at hf
    constructor
    · intro hf0
      have : f * d + d ≤ 0 := by nlinarith
      refine ⟨by omega, ?_⟩
      simp only [ne_eq, eq_iff_iff, show ¬ d < 0 by omega, iff_false]; omega
    · rintro ⟨hn0, hs⟩
      have hn : n < 0 := by simp [show ¬ d < 0 by omega] at hs; omega
      by_contra hf0
      have : 0 ≤ f * d := by nlinarith
      omega

/-- `quotient_is_negative` is exact: it returns true precisely when the
exact rational `x*y/d` is strictly negative. -/
theorem quotient_is_negative_spec (x y d : Std.I128) :
    math.fp_core.quotient_is_negative x y d ⦃ b => b = true ↔
      (x.val * y.val ≠ 0 ∧ ((x.val * y.val < 0) ≠ (d.val < 0))) ⦄ := by
  unfold math.fp_core.quotient_is_negative
  by_cases hx : x = 0#i128
  · simp [hx]
  by_cases hy : y = 0#i128
  · simp [hy]
  have hx0 : x.val ≠ 0 := fun h => hx (IScalar.eq_of_val_eq (by simp [h]))
  have hy0 : y.val ≠ 0 := fun h => hy (IScalar.eq_of_val_eq (by simp [h]))
  simp only [hx, hy, ↓reduceIte, WP.spec_ok, IScalar.lt_equiv]
  have hxy : x.val * y.val ≠ 0 := mul_ne_zero hx0 hy0
  have hp : x.val * y.val < 0 ↔ ¬((x.val < 0) ↔ (y.val < 0)) := by
    rcases lt_or_gt_of_ne hx0 with hx1 | hx1 <;> rcases lt_or_gt_of_ne hy0 with hy1 | hy1
    · have : 0 < x.val * y.val := mul_pos_of_neg_of_neg hx1 hy1
      simp [hx1, hy1]; omega
    · have : x.val * y.val < 0 := mul_neg_of_neg_of_pos hx1 hy1
      simp [hx1, this]; omega
    · have : x.val * y.val < 0 := mul_neg_of_pos_of_neg hx1 hy1
      simp [hy1, this]; omega
    · have : 0 < x.val * y.val := mul_pos hx1 hy1
      have hxn : ¬ x.val < 0 := by omega
      have hyn : ¬ y.val < 0 := by omega
      simp [hxn, hyn]; omega
  rw [hp]
  by_cases hxn : x.val < 0 <;> by_cases hyn : y.val < 0 <;> by_cases hdn : d.val < 0 <;>
    simp [hxn, hyn, hdn, hxy]

/-- `i128::MIN = -2^127`. -/
theorem i128_min_val : (core.num.I128.MIN).val = -2^127 := by
  simp [core.num.I128.MIN, I128.rMin]

/-- `i128::MAX = 2^127 - 1`. -/
theorem i128_max_val : (core.num.I128.MAX).val = 2^127 - 1 := by
  simp [core.num.I128.MAX, I128.rMax]

/-- **`mul_div_floor_saturating` is the floor clamped to `i128`.** For every
`i128` `x`, `y` and nonzero `d` it returns (never aborts), and its result is
`⌊x*y/d⌋` when that fits `i128`, `i128::MAX` when the floor is above the range
and `i128::MIN` when it is below: `r = max(i128::MIN, min(i128::MAX, ⌊x*y/d⌋))`.
Since the floor is above the range only for a positive quotient and below it
only for a negative one, this is the "saturate by the sign of the exact
quotient" of the Rust doc comment.

Hypothesis: `d ≠ 0` (production panics with `DivisionByZero` otherwise; the
callers `calculate_scaled_cap` and `protocol_fee_shares` divide by an index,
always ≥ `SUPPLY_INDEX_FLOOR_RAW` > 0). -/
theorem mul_div_floor_saturating_spec (env : Env) (x y d : Std.I128) (hd : d.val ≠ 0) :
    math.fp_core.mul_div_floor_saturating env x y d ⦃ r => ∀ q : Int,
      IsFloor (x.val * y.val) d.val q → r.val = max (-2^127) (min (2^127 - 1) q) ⦄ := by
  have hbounds := i128_prod_bounds x y
  have hd' : ¬ (d = 0#i128) := fun h => hd (by simp [h])
  have wide : (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let i ← aeneas_soroban_shim.I256.mul x256 y256
      let b ← math.fp_core.quotient_is_nonnegative x y d
      let i1 ← math.fp_core.div_floor_i256 env i d256 b
      let o2 ← aeneas_soroban_shim.I256.to_i128 i1
      let b1 ← math.fp_core.quotient_is_negative x y d
      let i2 ← if b1 then ok core.num.I128.MIN else ok core.num.I128.MAX
      ok (core.option.Option.unwrap_or o2 i2)) ⦃ r => ∀ q : Int,
        IsFloor (x.val * y.val) d.val q → r.val = max (-2^127) (min (2^127 - 1) q) ⦄ := by
    unfold math.fp_core.to_i256_operands
    step as ⟨a, ha⟩
    step as ⟨b, hb⟩
    step as ⟨c, hc⟩
    step as ⟨i, hi⟩
    · simp only [aeneas_soroban_shim.I256.InRange, ha, hb,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega
    rw [ha, hb] at hi
    rw [hc] at *
    step with quotient_is_nonnegative_spec x y d hd as ⟨nn, hnn⟩
    step with div_floor_i256_spec env i d.val hd (by
        rw [hi, show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
          show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega)
        nn (by rw [hi]; exact hnn) as ⟨f, hf⟩
    rw [hi] at hf
    step as ⟨o, ho⟩
    step with quotient_is_negative_spec x y d as ⟨neg, hneg⟩
    have hsign := IsFloor.neg_iff hd hf
    have htm := IScalar.tryMkOpt_eq .I128 f
    rw [← ho] at htm
    cases o with
    | some v =>
      obtain ⟨hv, hin⟩ := htm
      simp only [IScalar.inBounds, IScalarTy.numBits] at hin
      cases neg <;> simp only [Bool.false_eq_true, ↓reduceIte, bind_ok, WP.spec_ok,
        core.option.Option.unwrap_or] <;>
      · intro q hq
        have := IsFloor.unique hd hq hf
        subst this
        rw [hv]; omega
    | none =>
      simp only [IScalar.inBounds, IScalarTy.numBits] at htm
      cases neg with
      | true =>
        simp only [↓reduceIte, bind_ok, WP.spec_ok, core.option.Option.unwrap_or]
        intro q hq
        have := IsFloor.unique hd hq hf
        subst this
        have : q < 0 := hsign.mpr (hneg.mp rfl)
        rw [i128_min_val]; omega
      | false =>
        simp only [Bool.false_eq_true, ↓reduceIte, bind_ok, WP.spec_ok,
          core.option.Option.unwrap_or]
        intro q hq
        have := IsFloor.unique hd hq hf
        subst this
        have : ¬ q < 0 := fun h => by simpa using hneg.mpr (hsign.mp h)
        rw [i128_max_val]; omega
  unfold math.fp_core.mul_div_floor_saturating math.fp_core.require_nonzero_divisor
  simp only [massert, hd', not_false_eq_true, ↓reduceIte, bind_ok, lift]
  have hmul := I128.checked_mul_bv_spec x y
  rw [show I128.checked_mul x y = core.num.checked_mul_IScalar x y from rfl]
  cases hc : core.num.checked_mul_IScalar x y with
  | none =>
    simp only [core.option.Option.and_then, bind_ok]
    exact wide
  | some m =>
    rw [hc] at hmul
    obtain ⟨_, _, hm, _⟩ := hmul
    simp only [core.option.Option.and_then,
      math.fp_core.mul_div_floor_saturating.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
    step with div_floor_i128_spec m d hd as ⟨o1, ho1⟩
    cases o1 with
    | none => exact wide
    | some q =>
      simp only [WP.spec_ok]
      intro q' hq'
      have h1 := ho1 q rfl
      rw [hm] at h1
      have := IsFloor.unique hd hq' h1
      subst this
      have := q.hBounds
      simp only [IScalarTy.numBits] at this
      omega

/-- Corollary: when the floor fits `i128`, `mul_div_floor_saturating` returns
exactly the floor (same hypotheses as `mul_div_floor_spec`). -/
theorem mul_div_floor_saturating_exact (env : Env) (x y d : Std.I128) (hd : d.val ≠ 0)
    (q : Int) (hq : IsFloor (x.val * y.val) d.val q) (hlo : -2^127 ≤ q) (hhi : q < 2^127) :
    math.fp_core.mul_div_floor_saturating env x y d ⦃ r => r.val = q ⦄ := by
  apply WP.spec_mono (mul_div_floor_saturating_spec env x y d hd)
  intro r hr
  rw [hr q hq]; omega

end ControllerAeneas.Proofs
