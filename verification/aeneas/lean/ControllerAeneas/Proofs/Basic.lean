/-
Small facts about the generated fixed-point newtype code (`math/fp.rs`) used
by every controller proof: the comparison instances of `Wad`, `Ray` and `Bps`
(each a plain `i128`), `min`, and the checked add/sub helpers.

A plain (non-`module`) file: `checked_add_raw` uses the sealed
`I128.checked_add` (see WRITING-PROOFS.md, "Pitfalls").
-/
import ControllerAeneas.Proofs.MulDiv

open Aeneas Aeneas.Std Result
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## Comparisons on `Wad` (derived `PartialOrd`/`Ord` on the `i128` field) -/

@[simp] theorem wad_lt (a b : math.fp.Wad) :
    core.cmp.PartialOrd.lt.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad a b
      = ok (decide (a.val < b.val)) := by
  simp [core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, math.fp.Wad.Insts.CoreCmpPartialOrdWad.partial_cmp,
    math.fp.Wad.Insts.CoreCmpOrd.cmp, compare_lt_iff_lt]

@[simp] theorem wad_le (a b : math.fp.Wad) :
    core.cmp.PartialOrd.le.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad a b
      = ok (decide (a.val ≤ b.val)) := by
  simp only [core.cmp.PartialOrd.le.trait_default, core.cmp.PartialOrd.le.default,
    core.cmp.PartialOrd.le_body, math.fp.Wad.Insts.CoreCmpPartialOrdWad,
    math.fp.Wad.Insts.CoreCmpPartialOrdWad.partial_cmp,
    math.fp.Wad.Insts.CoreCmpOrd.cmp, core.cmp.impls.OrdI128.cmp, bind_ok, Option.some.injEq]
  congr 1
  rcases lt_trichotomy a.val b.val with h | h | h
  · simp [compare_lt_iff_lt.mpr h, h.le]
  · simp [compare_eq_iff_eq.mpr h, h.le]
  · simp [compare_gt_iff_gt.mpr h, not_le.mpr h]

@[simp] theorem wad_gt (a b : math.fp.Wad) :
    core.cmp.PartialOrd.gt.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad a b
      = ok (decide (b.val < a.val)) := by
  simp [core.cmp.PartialOrd.gt.trait_default, core.cmp.PartialOrd.gt.default,
    core.cmp.PartialOrd.gt_body, math.fp.Wad.Insts.CoreCmpPartialOrdWad.partial_cmp,
    math.fp.Wad.Insts.CoreCmpOrd.cmp, compare_gt_iff_gt]

@[simp] theorem wad_ge (a b : math.fp.Wad) :
    core.cmp.PartialOrd.ge.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad a b
      = ok (decide (b.val ≤ a.val)) := by
  simp only [core.cmp.PartialOrd.ge.trait_default, core.cmp.PartialOrd.ge.default,
    core.cmp.PartialOrd.ge_body, math.fp.Wad.Insts.CoreCmpPartialOrdWad,
    math.fp.Wad.Insts.CoreCmpPartialOrdWad.partial_cmp,
    math.fp.Wad.Insts.CoreCmpOrd.cmp, core.cmp.impls.OrdI128.cmp, bind_ok, Option.some.injEq]
  congr 1
  rcases lt_trichotomy a.val b.val with h | h | h
  · simp [compare_lt_iff_lt.mpr h, not_le.mpr h]
  · simp [compare_eq_iff_eq.mpr h, h.ge]
  · simp [compare_gt_iff_gt.mpr h, h.le]

@[simp] theorem wad_min (a b : math.fp.Wad) :
    core.cmp.Ord.min.trait_default math.fp.Wad.Insts.CoreCmpOrd a b
      = ok (if b.val < a.val then b else a) := by
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default, core.cmp.Ord.min_body,
    math.fp.Wad.Insts.CoreCmpOrd, math.fp.Wad.Insts.CoreCmpPartialOrdWad,
    core.cmp.PartialOrd.lt.default, core.cmp.PartialOrd.lt_body,
    math.fp.Wad.Insts.CoreCmpPartialOrdWad.partial_cmp, math.fp.Wad.Insts.CoreCmpOrd.cmp,
    core.cmp.impls.OrdI128.cmp, bind_ok, Option.some.injEq]
  by_cases h : b.val < a.val
  · simp [compare_lt_iff_lt.mpr h]; intro h'; omega
  · simp [compare_lt_iff_lt, h]

/-! ## Comparisons on `Ray` -/

@[simp] theorem ray_lt (a b : math.fp.Ray) :
    core.cmp.PartialOrd.lt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (a.val < b.val)) := by
  simp [core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp,
    math.fp.Ray.Insts.CoreCmpOrd.cmp, compare_lt_iff_lt]

@[simp] theorem ray_gt (a b : math.fp.Ray) :
    core.cmp.PartialOrd.gt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (b.val < a.val)) := by
  simp [core.cmp.PartialOrd.gt.trait_default, core.cmp.PartialOrd.gt.default,
    core.cmp.PartialOrd.gt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp,
    math.fp.Ray.Insts.CoreCmpOrd.cmp, compare_gt_iff_gt]

@[simp] theorem ray_ne (a b : math.fp.Ray) :
    core.cmp.PartialEq.ne.trait_default math.fp.Ray.Insts.CoreCmpPartialEqRay a b
      = ok (decide (a ≠ b)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    math.fp.Ray.Insts.CoreCmpPartialEqRay.eq]
  exact ⟨fun h => by rw [h], fun h => IScalar.eq_of_val_eq h⟩

/-! ## Conversions -/

@[simp] theorem wad_from_same (x : Std.I128) :
    math.fp.Wad.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) x = ok x := by
  simp [math.fp.Wad.from]

@[simp] theorem ray_from_same (x : Std.I128) :
    math.fp.Ray.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) x = ok x := by
  simp [math.fp.Ray.from]

@[simp] theorem bps_from_same (x : Std.I128) :
    math.fp.Bps.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) x = ok x := by
  simp [math.fp.Bps.from]

/-! ## Checked add / sub -/

/-- `checked_add_raw` adds exactly whenever the sum fits `i128`. -/
theorem checked_add_raw_spec (env : Env) (a b : Std.I128)
    (h : -2^127 ≤ a.val + b.val ∧ a.val + b.val < 2^127) :
    math.fp.checked_add_raw env a b ⦃ c => c.val = a.val + b.val ⦄ := by
  unfold math.fp.checked_add_raw
  have hs := core.num.checked_add_IScalar_bv_spec a b
  rw [show I128.checked_add a b = core.num.checked_add_IScalar a b from rfl]
  cases hc : core.num.checked_add_IScalar a b with
  | none =>
    rw [hc] at hs; exfalso; apply hs
    simp only [IScalar.min, IScalar.max, IScalarTy.numBits]; omega
  | some c =>
    rw [hc] at hs; obtain ⟨_, _, hv, _⟩ := hs
    simp [lift, core.option.Option.unwrap_or_else, hv]

/-- `checked_add_raw` aborts when the sum does not fit `i128`. -/
theorem checked_add_raw_fail (env : Env) (a b : Std.I128)
    (h : ¬ (-2^127 ≤ a.val + b.val ∧ a.val + b.val < 2^127)) :
    ∀ c, math.fp.checked_add_raw env a b ≠ ok c := by
  intro c
  unfold math.fp.checked_add_raw
  have hs := core.num.checked_add_IScalar_bv_spec a b
  rw [show I128.checked_add a b = core.num.checked_add_IScalar a b from rfl]
  cases hc : core.num.checked_add_IScalar a b with
  | none =>
    simp [lift, core.option.Option.unwrap_or_else,
      math.fp.checked_add_raw.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]
  | some c' =>
    rw [hc] at hs; obtain ⟨_, _, hv, _⟩ := hs
    exfalso; apply h
    have := c'.hBounds
    simp only [IScalarTy.numBits] at this
    omega

/-- `checked_sub_nonneg` subtracts exactly when `0 ≤ b ≤ a`. -/
theorem checked_sub_nonneg_spec (env : Env) (a b : Std.I128) (hb : 0 ≤ b.val)
    (hba : b.val ≤ a.val) :
    math.fp.checked_sub_nonneg env a b ⦃ c => c.val = a.val - b.val ⦄ := by
  unfold math.fp.checked_sub_nonneg
  have ha' : ¬ (a < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hb' : ¬ (b < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hab : ¬ (b > a) := by simp [IScalar.lt_equiv]; omega
  simp only [massert, ha', hb', hab, not_false_eq_true, ↓reduceIte, bind_ok]
  step as ⟨c, hc⟩
  simpa using hc

end ControllerAeneas.Proofs
