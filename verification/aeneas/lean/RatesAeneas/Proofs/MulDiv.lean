/-
`fp_core::mul_div_floor` returns exactly `⌊x*y/d⌋` for every `i128` input
with `d ≠ 0`, and returns (does not abort) whenever that floor fits `i128`.

No input bounds: unlike the Kani harnesses, these hold over the whole `i128`
domain. Both the native `i128` path and the widened `I256` path are covered.
-/
import RatesAeneas.Funs
import FixedPoint.Spec
import RatesAeneas.Proofs.Host

/- A plain (non-`module`) file on purpose: the signed checked operations of the
Aeneas library (`I128.checked_div` and friends) are not exposed under Lean's
module system, so only a non-module file can unfold them to the definitions
their library specs talk about. -/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec
open rates_aeneas

namespace RatesAeneas.Proofs

abbrev Env := aeneas_soroban_shim.Env

/-! ## The `I256` path -/

/-- `inexact` flag from the Euclidean remainder: true exactly when `d ∤ n`. -/
theorem emod_flag (n d : Int) : decide (Int.emod n d ≠ 0) = true ↔ ¬ d ∣ n := by
  rw [decide_eq_true_iff, Int.dvd_iff_emod_eq_zero]
  exact Iff.rfl

/-- `<` on host `I256` values, through the generated `PartialOrd` instance. -/
theorem i256_lt_eq (a b : aeneas_soroban_shim.I256) :
    core.cmp.PartialOrd.lt.trait_default aeneas_soroban_shim.I256.Insts.CoreCmpPartialOrdI256 a b
      = ok (decide ((a : Int) < b)) := by
  simp [core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, aeneas_soroban_shim.I256.Insts.CoreCmpPartialOrdI256,
    aeneas_soroban_shim.I256.Insts.CoreCmpPartialOrdI256.partial_cmp, compare_lt_iff_lt]

/-- `div_floor_i256` computes the floor of `n / d` for every dividend strictly
inside the host range and nonzero divisor, given a correct `nonneg` hint (the
exact quotient is not negative). -/
theorem div_floor_i256_spec (env : Env) (n d : Int) (hd : d ≠ 0)
    (hn : aeneas_soroban_shim.I256.MIN < n ∧ n ≤ aeneas_soroban_shim.I256.MAX)
    (nonneg : Bool) (hnn : nonneg = true → n = 0 ∨ ((n < 0) = (d < 0))) :
    math.fp_core.div_floor_i256 env n d nonneg ⦃ c => IsFloor n d c ⦄ := by
  have hle : (Int.tdiv n d).natAbs ≤ n.natAbs := by
    rw [Int.natAbs_tdiv]; exact Nat.div_le_self _ _
  have hfl := truncFloor_isFloor n d hd (decide (Int.emod n d ≠ 0)) (emod_flag n d)
  have hzero : Int.emod 0 d = 0 := Int.zero_emod d
  have hMIN : aeneas_soroban_shim.I256.MIN = -2^255 := rfl
  have hMAX : aeneas_soroban_shim.I256.MAX = 2^255 - 1 := rfl
  rw [hMIN, hMAX] at hn
  unfold truncFloor at hfl
  unfold math.fp_core.div_floor_i256
  cases nonneg with
  | true =>
    simp only [↓reduceIte]
    step as ⟨q, hq⟩
    · simp only [aeneas_soroban_shim.I256.InRange, hMIN, hMAX]; omega
    rw [hq]
    rcases hnn rfl with h0 | hs
    · subst h0; simpa [hzero] using hfl
    · rwa [if_neg (by simp [hs])] at hfl
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte]
    step as ⟨z, hz⟩
    step as ⟨q, hq⟩
    · simp only [aeneas_soroban_shim.I256.InRange, hMIN, hMAX]; omega
    step as ⟨r, hr⟩
    step as ⟨b, hb⟩
    · simp [aeneas_soroban_shim.I256.Insts.CoreCmpPartialEqI256.eq]
    subst hz hq hr
    by_cases hex : Int.emod n d = 0
    · have hbf : b = false := by
        cases b
        · rfl
        · exact absurd (hb.mp rfl) (by simp [hex])
      subst hbf
      simp only [Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
      simpa [hex] using hfl
    · have hbt : b = true := hb.mpr (by simpa using hex)
      subst hbt
      simp only [↓reduceIte, i256_lt_eq, bind_ok]
      by_cases hs : (n < 0) = (d < 0)
      · have hne : (decide (n < 0) != decide ((d : Int) < 0)) = false := by
          simp [hs]
        simp only [hne, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
        rwa [if_neg (by simp [hs])] at hfl
      · have hne : (decide (n < 0) != decide ((d : Int) < 0)) = true := by
          by_cases hn0 : n < 0 <;> by_cases hd0 : d < 0 <;> simp_all
        simp only [hne, ↓reduceIte]
        step as ⟨one, hone⟩
        step as ⟨c, hc⟩
        · simp only [aeneas_soroban_shim.I256.InRange, hMIN, hMAX, hone]; simp; omega
        rw [hc, hone]
        rw [if_pos (by simp [hex, hs])] at hfl
        simpa using hfl

/-! ## The native `i128` path -/

/-- `div_floor_i128` never aborts, and whenever it returns `Some q`, `q` is the
floor of `p / d`. (It returns `None` on `i128::MIN / -1` and when the floor is
`i128::MIN - 1`; the caller then takes the `I256` path.) -/
theorem div_floor_i128_spec (p d : Std.I128) (hd : d.val ≠ 0) :
    math.fp_core.div_floor_i128 p d ⦃ o => ∀ q, o = some q → IsFloor p.val d.val q.val ⦄ := by
  have hfl := truncFloor_isFloor p.val d.val hd (decide (Int.tmod p.val d.val ≠ 0))
    (by rw [decide_eq_true_iff, Int.dvd_iff_tmod_eq_zero])
  unfold truncFloor at hfl
  unfold math.fp_core.div_floor_i128
  have hdiv := I128.checked_div_bv_spec p d
  rw [show I128.checked_div p d = core.num.checked_div_IScalar p d from rfl]
  cases hc : core.num.checked_div_IScalar p d with
  | none =>
    simp [lift, core.option.Option.Insts.CoreOpsTry_traitTry.branch,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual]
  | some t =>
    rw [hc] at hdiv
    obtain ⟨_, _, ht, _⟩ := hdiv
    simp only [lift, bind_ok, core.option.Option.Insts.CoreOpsTry_traitTry.branch]
    step as ⟨r, hr⟩
    by_cases hex : Int.tmod p.val d.val = 0
    · have hr0 : (r != 0#i128) = false := by
        simp only [bne_eq_false_iff_eq]; apply IScalar.eq_of_val_eq; simp [hr, hex]
      simp only [hr0, Bool.false_eq_true, ↓reduceIte, WP.spec_ok, Option.some.injEq]
      rintro q rfl
      rw [ht]; simpa [hex] using hfl
    · have hr0 : (r != 0#i128) = true := by
        simp only [bne_iff_ne, ne_eq]; intro h; apply hex; rw [← hr, h]; rfl
      simp only [hr0, ↓reduceIte]
      by_cases hs : (p.val < 0) = (d.val < 0)
      · have hne : (decide (p < 0#i128) != decide (d < 0#i128)) = false := by
          simp [IScalar.lt_equiv, hs]
        simp only [hne, Bool.false_eq_true, ↓reduceIte, WP.spec_ok, Option.some.injEq]
        rintro q rfl
        rw [ht]; rwa [if_neg (by simp [hs])] at hfl
      · have hne : (decide (p < 0#i128) != decide (d < 0#i128)) = true := by
          by_cases hp0 : p.val < 0 <;> by_cases hd0 : d.val < 0 <;>
            simp_all [IScalar.lt_equiv]
        simp only [hne, ↓reduceIte, WP.spec_ok]
        have hsub := I128.checked_sub_bv_spec t 1#i128
        intro q hq
        rw [show I128.checked_sub t 1#i128 = core.num.checked_sub_IScalar t 1#i128 from rfl] at hq
        rw [hq] at hsub
        obtain ⟨_, _, hqv, _⟩ := hsub
        rw [hqv, ht]
        rw [if_pos (by simp [hex, hs])] at hfl
        simpa using hfl

/-! ## `mul_div_floor` -/

/-- The product of two `i128` values lies strictly inside the host `I256` range. -/
theorem i128_prod_bounds (x y : Std.I128) :
    -2^255 < x.val * y.val ∧ x.val * y.val ≤ 2^255 - 1 := by
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

/-- `quotient_is_nonnegative` is a correct hint: when it says true, the exact
quotient `x*y/d` is not negative. -/
theorem quotient_is_nonnegative_spec (x y d : Std.I128) (hd : d.val ≠ 0) :
    math.fp_core.quotient_is_nonnegative x y d ⦃ b => b = true →
      (x.val * y.val = 0 ∨ ((x.val * y.val < 0) = (d.val < 0))) ⦄ := by
  unfold math.fp_core.quotient_is_nonnegative math.fp_core.quotient_is_negative
  by_cases hx : x = 0#i128
  · simp [hx]
  by_cases hy : y = 0#i128
  · simp [hy]
  have hx0 : x.val ≠ 0 := fun h => hx (IScalar.eq_of_val_eq (by simp [h]))
  have hy0 : y.val ≠ 0 := fun h => hy (IScalar.eq_of_val_eq (by simp [h]))
  simp only [hx, hy, ↓reduceIte, bind_ok, WP.spec_ok, IScalar.lt_equiv]
  intro h
  right
  rw [eq_iff_iff, mul_neg_iff]
  by_cases hxn : x.val < 0 <;> by_cases hyn : y.val < 0 <;> by_cases hdn : d.val < 0 <;>
    simp_all <;> omega

/-- **`mul_div_floor` is exact.** For every `i128` `x`, `y` and nonzero `d`
whose exact floor `⌊x*y/d⌋` fits `i128`, `mul_div_floor` returns (does not
abort) and its result is that floor. No other bound on the inputs. -/
theorem mul_div_floor_spec (env : Env) (x y d : Std.I128) (hd : d.val ≠ 0)
    (hfit : ∃ q : Int, IsFloor (x.val * y.val) d.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp_core.mul_div_floor env x y d ⦃ q => IsFloor (x.val * y.val) d.val q.val ⦄ := by
  obtain ⟨qf, hqf, hlo, hhi⟩ := hfit
  have hbounds := i128_prod_bounds x y
  have hd' : ¬ (d = 0#i128) := fun h => hd (by simp [h])
  -- The widened path, shared by both fallbacks.
  have wide : (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let nonneg ← math.fp_core.quotient_is_nonnegative x y d
      let i ← aeneas_soroban_shim.I256.mul x256 y256
      let i1 ← math.fp_core.div_floor_i256 env i d256 nonneg
      math.fp_core.to_i128 env i1) ⦃ q => IsFloor (x.val * y.val) d.val q.val ⦄ := by
    unfold math.fp_core.to_i256_operands
    step as ⟨a, ha⟩
    step as ⟨b, hb⟩
    step as ⟨c, hc⟩
    step with quotient_is_nonnegative_spec x y d hd as ⟨nn, hnn⟩
    step as ⟨i, hi⟩
    · simp only [aeneas_soroban_shim.I256.InRange, ha, hb,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega
    rw [ha, hb] at hi
    rw [hc] at *
    step with div_floor_i256_spec env i d.val hd (by
        rw [hi, show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
          show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]; omega)
        nn (by rw [hi]; exact hnn) as ⟨f, hf⟩
    rw [hi] at hf
    have hfq : f = qf := IsFloor.unique hd hf hqf
    unfold math.fp_core.to_i128
    step as ⟨o, ho⟩
    have htm := IScalar.tryMkOpt_eq .I128 f
    rw [← ho] at htm
    cases o with
    | none =>
      exfalso; apply htm
      simp only [IScalar.inBounds, IScalarTy.numBits]; rw [hfq]; omega
    | some v =>
      obtain ⟨hv, _⟩ := htm
      simp only [core.option.Option.unwrap_or_else, WP.spec_ok]
      rw [hv]; exact hf
  unfold math.fp_core.mul_div_floor math.fp_core.require_nonzero_divisor
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
      math.fp_core.mul_div_floor.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
    step with div_floor_i128_spec m d hd as ⟨o1, ho1⟩
    cases o1 with
    | none => exact wide
    | some q =>
      simp only [WP.spec_ok]
      rw [← hm]; exact ho1 q rfl

end RatesAeneas.Proofs
