/-
Small facts shared by the pool proofs: `i128` bounds, inverting a `bind`
that returned `ok`, the checked `Ray` addition and subtraction of
`common/src/math/fp.rs`, and the existence of floors and ceilings.
-/
import PoolAeneas.Proofs.FloorCeilScaling
import PoolAeneas.Proofs.HalfUp

/- Plain (non-`module`) file: unfolds `I128.checked_add`, sealed under the
module system (WRITING-PROOFS.md). -/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec
open pool_aeneas

namespace PoolAeneas.Proofs

/-- Every `i128` lies in `[-2^127, 2^127)`. -/
theorem i128_bounds (a : Std.I128) : -2^127 ≤ a.val ∧ a.val < 2^127 := by
  have := a.hBounds; simp only [IScalarTy.numBits] at this; omega

/-- A `do` block that returned `ok` ran its first step to `ok` and its continuation to the same result. -/
theorem bind_eq_ok {α β : Type} {m : Result α} {k : α → Result β} {y : β}
    (h : (do let x ← m; k x) = ok y) : ∃ x, m = ok x ∧ k x = ok y := by
  cases m <;> simp_all

/-- `fp::checked_add_raw` returned `r`: `r = a + b` exactly (it aborts on overflow). -/
theorem checked_add_raw_eq_ok (env : Env) (a b r : Std.I128)
    (h : math.fp.checked_add_raw env a b = ok r) : r.val = a.val + b.val := by
  unfold math.fp.checked_add_raw at h
  have hs := I128.checked_add_bv_spec a b
  rw [show I128.checked_add a b = core.num.checked_add_IScalar a b from rfl] at h
  cases hc : core.num.checked_add_IScalar a b with
  | none => simp [hc, lift, core.option.Option.unwrap_or_else,
      math.fp.checked_add_raw.closure.Insts.CoreOpsFunctionFnOnceTupleI128,
      math.fp.checked_add_raw.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once] at h
  | some z =>
    rw [hc] at hs
    simp [hc, lift, core.option.Option.unwrap_or_else] at h
    rw [← h]; exact hs.2.2.1

/-- `fp::checked_add_raw` returns `a + b` whenever the sum fits `i128`. -/
theorem checked_add_raw_spec (env : Env) (a b : Std.I128)
    (hlo : -2^127 ≤ a.val + b.val) (hhi : a.val + b.val < 2^127) :
    math.fp.checked_add_raw env a b ⦃ r => r.val = a.val + b.val ⦄ := by
  unfold math.fp.checked_add_raw
  have hs := I128.checked_add_bv_spec a b
  rw [show I128.checked_add a b = core.num.checked_add_IScalar a b from rfl]
  cases hc : core.num.checked_add_IScalar a b with
  | none =>
    rw [hc] at hs; exfalso; apply hs
    simp only [I128.min_eq, I128.max_eq]; omega
  | some z =>
    rw [hc] at hs
    simp [lift, core.option.Option.unwrap_or_else]
    exact hs.2.2.1

/-- `fp::checked_sub_nonneg` returned `r`: both operands were non-negative, `b ≤ a`, and `r = a - b`. -/
theorem checked_sub_nonneg_eq_ok (env : Env) (a b r : Std.I128)
    (h : math.fp.checked_sub_nonneg env a b = ok r) :
    0 ≤ a.val ∧ 0 ≤ b.val ∧ b.val ≤ a.val ∧ r.val = a.val - b.val := by
  unfold math.fp.checked_sub_nonneg at h
  by_cases ha : a < 0#i128
  · simp [massert, ha] at h
  by_cases hb : b < 0#i128
  · simp [massert, ha, hb] at h
  by_cases hab : b > a
  · simp [massert, ha, hb, hab] at h
  simp only [massert, ha, hb, hab, not_false_eq_true, ↓reduceIte, bind_ok] at h
  simp only [IScalar.lt_equiv, gt_iff_lt] at ha hb hab
  simp at ha hb
  refine ⟨by omega, by omega, by omega, ?_⟩
  have hsp : (a - b) ⦃ z => (↑z : Int) = ↑a - ↑b ∧ z.bv = a.bv - b.bv ⦄ :=
    I128.sub_bv_spec (by rw [I128.min_eq]; omega)
      (by rw [I128.max_eq]; have := i128_bounds a; omega)
  obtain ⟨v, hv, hP⟩ := spec_imp_eq_ok hsp
  rw [hv] at h; simp only [ok.injEq] at h; rw [← h]; exact hP.1


/-- `Ray::checked_add` returned `r`: `r = a + b` exactly. -/
theorem ray_checked_add_eq_ok (env : Env) (a b r : Std.I128)
    (h : math.fp.Ray.checked_add a env b = ok r) : r.val = a.val + b.val := by
  unfold math.fp.Ray.checked_add at h
  obtain ⟨x, hx, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; rw [← h]; exact checked_add_raw_eq_ok env a b x hx

/-- `Ray::checked_add` returns `a + b` whenever it fits `i128`. -/
theorem ray_checked_add_spec (env : Env) (a b : Std.I128)
    (hlo : -2^127 ≤ a.val + b.val) (hhi : a.val + b.val < 2^127) :
    math.fp.Ray.checked_add a env b ⦃ r => r.val = a.val + b.val ⦄ := by
  unfold math.fp.Ray.checked_add
  step with checked_add_raw_spec env a b hlo hhi as ⟨x, hx⟩
  exact hx

/-- `Ray::checked_sub` returned `r`: both operands were non-negative, `b ≤ a`,
and `r = a - b`. -/
theorem ray_checked_sub_eq_ok (env : Env) (a b r : Std.I128)
    (h : math.fp.Ray.checked_sub a env b = ok r) :
    0 ≤ a.val ∧ 0 ≤ b.val ∧ b.val ≤ a.val ∧ r.val = a.val - b.val := by
  unfold math.fp.Ray.checked_sub at h
  obtain ⟨x, hx, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; rw [← h]; exact checked_sub_nonneg_eq_ok env a b x hx

/-- `Ray::checked_sub` returns `a - b` when `0 ≤ b ≤ a`. -/
theorem ray_checked_sub_spec (env : Env) (a b : Std.I128)
    (hb : 0 ≤ b.val) (hab : b.val ≤ a.val) :
    math.fp.Ray.checked_sub a env b ⦃ r => r.val = a.val - b.val ⦄ := by
  unfold math.fp.Ray.checked_sub math.fp.checked_sub_nonneg
  have ha : ¬ (a < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hb' : ¬ (b < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hab' : ¬ (b > a) := by simp [IScalar.lt_equiv]; omega
  simp only [massert, ha, hb', hab', not_false_eq_true, ↓reduceIte, bind_ok]
  have hsp : (a - b) ⦃ z => (↑z : Int) = ↑a - ↑b ∧ z.bv = a.bv - b.bv ⦄ :=
    I128.sub_bv_spec (by rw [I128.min_eq]; omega)
      (by rw [I128.max_eq]; have := i128_bounds a; omega)
  step with hsp as ⟨z, hz, _⟩
  exact hz

/-- Every positive divisor has a floor (`Int.ediv`). Used only to discharge
"the result fits" hypotheses; specifications never divide. -/
theorem isFloor_ediv (n d : Int) (hd : 0 < d) : IsFloor n d (n / d) := by
  unfold IsFloor; rw [if_pos hd]
  have h1 := Int.ediv_mul_le n (Int.ne_of_gt hd)
  have h2 := Int.emod_lt_of_pos n hd
  have h3 := Int.emod_add_mul_ediv n d
  constructor
  · exact h1
  · nlinarith [mul_comm d (n / d)]

/-- Every positive divisor has a ceiling. -/
theorem isCeil_exists (n d : Int) (hd : 0 < d) : ∃ q, IsCeil n d q := by
  by_cases h : d ∣ n
  · obtain ⟨k, hk⟩ := h
    exact ⟨k, by unfold IsCeil; rw [if_pos hd]; constructor <;> nlinarith⟩
  · refine ⟨n / d + 1, ?_⟩
    have hf := isFloor_ediv n d hd
    unfold IsFloor at hf; unfold IsCeil; rw [if_pos hd] at hf ⊢
    have hne : (n / d) * d ≠ n := by
      intro he; exact h ⟨n / d, by rw [mul_comm]; exact he.symm⟩
    constructor <;> nlinarith [lt_of_le_of_ne hf.1 hne]

/-- A floor of a non-negative `n` below `2^127 * d` fits `i128`. -/
theorem floor_fits (n d : Int) (hd : 0 < d) (hn : 0 ≤ n) (hlt : n < 2^127 * d) :
    ∃ q : Int, IsFloor n d q ∧ -2^127 ≤ q ∧ q < 2^127 := by
  refine ⟨n / d, isFloor_ediv n d hd, ?_, ?_⟩
  · have := Int.ediv_nonneg hn (le_of_lt hd); omega
  · have hf := isFloor_ediv n d hd
    unfold IsFloor at hf; rw [if_pos hd] at hf
    by_contra hc
    have hc := not_lt.mp hc
    nlinarith

/-- The floor of a non-negative quotient is non-negative. -/
theorem IsFloor.nonneg {n d q : Int} (hd : 0 < d) (hn : 0 ≤ n) (h : IsFloor n d q) : 0 ≤ q := by
  unfold IsFloor at h; rw [if_pos hd] at h
  by_contra hc
  have hc := not_le.mp hc
  nlinarith

/-! ## `Ray` comparisons (derived `PartialEq`/`PartialOrd`/`Ord`) -/

/-- Two `Ray`s are equal iff their raw values are. -/
theorem ray_val_inj {a b : math.fp.Ray} : a = b ↔ (a.val = b.val) :=
  ⟨fun h => by rw [h], fun h => IScalar.eq_of_val_eq h⟩

/-- `<` on `Ray` (derived `PartialOrd`) is `<` on the raw values. -/
theorem ray_lt_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.lt.default math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp a b
      = ok (decide (a.val < b.val)) := by
  simp [core.cmp.PartialOrd.lt.default, core.cmp.PartialOrd.lt_body,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_lt_iff_lt]

/-- `<=` on `Ray` is `≤` on the raw values. -/
theorem ray_le_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.le.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (a.val ≤ b.val)) := by
  simp [core.cmp.PartialOrd.le.trait_default, core.cmp.PartialOrd.le.default,
    core.cmp.PartialOrd.le_body,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_lt_iff_lt]
  by_cases h1 : (a : Int) < b <;> by_cases h2 : (a : Int) = b <;> simp [h1, h2] <;> omega

/-- `>=` on `Ray` is `≥` on the raw values. -/
theorem ray_ge_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.ge.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (b.val ≤ a.val)) := by
  simp [core.cmp.PartialOrd.ge.trait_default, core.cmp.PartialOrd.ge.default,
    core.cmp.PartialOrd.ge_body,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_gt_iff_gt]
  by_cases h1 : (b : Int) < a <;> by_cases h2 : (a : Int) = b <;> simp [h1, h2] <;> omega

/-- `>` on `Ray` is `>` on the raw values. -/
theorem ray_gt_eq (a b : math.fp.Ray) :
    core.cmp.PartialOrd.gt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay a b
      = ok (decide (b.val < a.val)) := by
  simp [core.cmp.PartialOrd.gt.trait_default, core.cmp.PartialOrd.gt.default,
    core.cmp.PartialOrd.gt_body,
    math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp, math.fp.Ray.Insts.CoreCmpOrd.cmp,
    compare_gt_iff_gt]

/-- `==` on `Ray` is equality of the raw values. -/
theorem ray_eq_eq (a b : math.fp.Ray) :
    math.fp.Ray.Insts.CoreCmpPartialEqRay.eq a b = ok (decide (a.val = b.val)) := by
  simp [math.fp.Ray.Insts.CoreCmpPartialEqRay.eq, ray_val_inj]

/-- `!=` on `Ray` is disequality of the raw values. -/
theorem ray_ne_eq (a b : math.fp.Ray) :
    core.cmp.PartialEq.ne.trait_default math.fp.Ray.Insts.CoreCmpPartialEqRay a b
      = ok (decide (a.val ≠ b.val)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    math.fp.Ray.Insts.CoreCmpPartialEqRay.eq, ray_val_inj]

/-- `Ord::min` on `Ray` is the minimum of the raw values (second argument on ties, as in Rust). -/
theorem ray_min_eq (a b : math.fp.Ray) :
    core.cmp.Ord.min.trait_default math.fp.Ray.Insts.CoreCmpOrd a b
      = ok (if b.val < a.val then b else a) := by
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default, core.cmp.Ord.min_body,
    math.fp.Ray.Insts.CoreCmpOrd, ray_lt_eq, bind_ok]
  by_cases h : (b : Int) < a <;> simp [h]

/-- `Ord::max` on `Ray` is the maximum of the raw values (second argument on ties, as in Rust). -/
theorem ray_max_eq (a b : math.fp.Ray) :
    core.cmp.Ord.max.trait_default math.fp.Ray.Insts.CoreCmpOrd a b
      = ok (if b.val < a.val then a else b) := by
  simp only [core.cmp.Ord.max.trait_default, core.cmp.Ord.max.default, core.cmp.Ord.max_body,
    math.fp.Ray.Insts.CoreCmpOrd, ray_lt_eq, bind_ok]
  by_cases h : (b : Int) < a <;> simp [h]

/-- `Ray::ZERO` is 0. -/
theorem zero_val : (math.fp.Ray.ZERO).val = 0 := by unfold math.fp.Ray.ZERO; rfl
/-! ## Partial correctness of `mul_div_ceil` -/

/-- A total-correctness triple tells what any `ok` result satisfies. -/
theorem spec_ok_eq {α : Type} {m : Result α} {P : α → Prop} {v : α}
    (h : m ⦃ x => P x ⦄) (he : m = ok v) : P v := by
  obtain ⟨w, hw, hp⟩ := spec_imp_eq_ok h
  rw [hw] at he; simp only [ok.injEq] at he; rw [← he]; exact hp

/-- Whatever `mul_div_ceil` returns is the exact ceiling (partial correctness,
no fit hypothesis): a ceiling that does not fit `i128` makes it abort. -/
theorem mul_div_ceil_eq_ok (env : Env) (x y d q : Std.I128) (hd : d.val ≠ 0)
    (h : math.fp_core.mul_div_ceil env x y d = ok q) : IsCeil (x.val * y.val) d.val q.val := by
  have hpb : -2^254 ≤ x.val * y.val ∧ x.val * y.val ≤ 2^254 := by
    have hx := x.hBounds
    have hy := y.hBounds
    simp only [IScalarTy.numBits] at hx hy
    have hax : x.val.natAbs ≤ 2^127 := by omega
    have hay : y.val.natAbs ≤ 2^127 := by omega
    have hp : (x.val * y.val).natAbs ≤ 2^254 := by
      rw [Int.natAbs_mul]
      calc x.val.natAbs * y.val.natAbs ≤ 2^127 * 2^127 := Nat.mul_le_mul hax hay
        _ = 2^254 := by norm_num
    omega
  have hd' : ¬ (d = 0#i128) := fun h => hd (by simp [h])
  have wide : ∀ v, (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let nonneg ← math.fp_core.quotient_is_nonnegative x y d
      let i ← aeneas_pool_shim.I256.mul x256 y256
      let i1 ← math.fp_core.div_ceil_i256 env i d256 nonneg
      math.fp_core.to_i128 env i1) = ok v → IsCeil (x.val * y.val) d.val v.val := by
    intro v hv
    unfold math.fp_core.to_i256_operands at hv
    simp only [aeneas_pool_shim.I256.from_i128, bind_ok] at hv
    obtain ⟨nn, hnn_eq, hv⟩ := bind_eq_ok hv
    have hnn := spec_ok_eq (quotient_is_nonnegative_spec x y d hd) hnn_eq
    obtain ⟨i, hi_eq, hv⟩ := bind_eq_ok hv
    have hi : (i : Int) = x.val * y.val := by
      have := spec_ok_eq (aeneas_pool_shim.I256.mul_spec (x.val : Int) (y.val : Int) (by
        simp only [aeneas_pool_shim.I256.InRange,
          show aeneas_pool_shim.I256.MIN = -2^255 from rfl,
          show aeneas_pool_shim.I256.MAX = 2^255 - 1 from rfl]; omega)) hi_eq
      exact this
    obtain ⟨f, hf_eq, hv⟩ := bind_eq_ok hv
    have hf := spec_ok_eq (div_ceil_i256_spec env i d.val hd (by rw [hi]; exact hpb)
        nn (by rw [hi]; exact hnn)) hf_eq
    rw [hi] at hf
    unfold math.fp_core.to_i128 at hv
    simp only [aeneas_pool_shim.I256.to_i128, bind_ok] at hv
    have htm := IScalar.tryMkOpt_eq .I128 f
    cases ho : IScalar.tryMkOpt .I128 f with
    | none =>
      rw [ho] at hv
      simp [core.option.Option.unwrap_or_else,
        math.fp_core.to_i128.closure.Insts.CoreOpsFunctionFnOnceTupleI128,
        math.fp_core.to_i128.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once] at hv
    | some w =>
      rw [ho] at hv htm
      simp only [core.option.Option.unwrap_or_else, ok.injEq] at hv
      rw [← hv, htm.1]; exact hf
  unfold math.fp_core.mul_div_ceil math.fp_core.require_nonzero_divisor at h
  simp only [massert, hd', not_false_eq_true, ↓reduceIte, bind_ok, lift] at h
  have hmul := I128.checked_mul_bv_spec x y
  rw [show I128.checked_mul x y = core.num.checked_mul_IScalar x y from rfl] at h
  cases hc : core.num.checked_mul_IScalar x y with
  | none =>
    rw [hc] at h
    simp only [core.option.Option.and_then, bind_ok] at h
    exact wide q h
  | some m =>
    rw [hc] at hmul h
    obtain ⟨_, _, hm, _⟩ := hmul
    simp only [core.option.Option.and_then,
      math.fp_core.mul_div_ceil.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once] at h
    obtain ⟨o1, ho1_eq, h⟩ := bind_eq_ok h
    have ho1 := spec_ok_eq (div_ceil_i128_spec m d hd) ho1_eq
    cases o1 with
    | none => exact wide q h
    | some r =>
      simp only [ok.injEq] at h
      rw [← hm, ← h]; exact ho1 r rfl
/-- `i128::saturating_sub` is plain subtraction when the difference fits. -/
theorem sat_sub_val (x y : Std.I128) (h1 : -2^127 ≤ x.val - y.val) (h2 : x.val - y.val < 2^127) :
    (core.num.I128.saturating_sub x y).val = x.val - y.val := by
  simp only [core.num.I128.saturating_sub, IScalar.saturating_sub, IScalar.min, IScalar.max,
    IScalarTy.numBits]
  rw [IScalar.val]
  simp
  rw [min_eq_right (by omega), max_eq_right (by omega)]
  exact Int.bmod_eq_of_le (by omega) (by omega)

/-- `i128::min` is the integer minimum. -/
theorem min_i128_val (x y : Std.I128) :
    (core.cmp.impls.OrdI128.min x y).val = min x.val y.val := by
  simp only [core.cmp.impls.OrdI128.min]
  split <;> rename_i h <;> simp only [IScalar.lt_equiv] at h <;> omega

/-- `i128::max` is the integer maximum. -/
theorem max_i128_val (x y : Std.I128) :
    (core.cmp.impls.OrdI128.max x y).val = max x.val y.val := by
  simp only [core.cmp.impls.OrdI128.max]
  split <;> rename_i h <;> simp only [IScalar.lt_equiv] at h <;> omega

/-- `Ray::ONE` is `RAY = 10^27`. -/
theorem ray_one_val : (math.fp.Ray.ONE).val = 10^27 := by
  unfold math.fp.Ray.ONE; exact ray_val

end PoolAeneas.Proofs
