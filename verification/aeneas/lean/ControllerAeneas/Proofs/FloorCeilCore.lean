-- Snapshot of RatesAeneas/Proofs/FloorCeilCore.lean taken 2026-11-12 (namespace changed,
-- `mul_div_floor_saturating` section omitted); see WRITING-PROOFS.md, last section.
/-
`fp_core::mul_div_ceil` returns exactly `⌈x*y/d⌉`, and
`fp_core::mul_div_floor_saturating` returns exactly `⌊x*y/d⌋` or saturates,
for every `i128` input with `d ≠ 0`. Both the native `i128` path and the
widened `I256` path are covered. No other input bounds.

A plain (non-`module`) file for the same reason as `MulDiv.lean`: the signed
checked operations are only reachable from a non-module file.
-/
import ControllerAeneas.Proofs.MulDiv
import FixedPoint.FloorCeilSpec

open Aeneas Aeneas.Std Result
open FixedPoint.Spec
open controller_aeneas

namespace ControllerAeneas.Proofs

/-! ## Ceiling, `I256` path -/

/-- `div_ceil_i256` computes the ceiling of `n / d` for every dividend with
`|n| ≤ 2^254` (every product of two `i128` values satisfies this, see
`i128_prod_bounds`) and nonzero divisor, given a correct `nonneg` hint (when
true, the exact quotient is not negative). -/
theorem div_ceil_i256_spec (env : Env) (n d : Int) (hd : d ≠ 0)
    (hn : -2^254 ≤ n ∧ n ≤ 2^254)
    (nonneg : Bool) (hnn : nonneg = true → n = 0 ∨ ((n < 0) = (d < 0))) :
    math.fp_core.div_ceil_i256 env n d nonneg ⦃ c => IsCeil n d c ⦄ := by
  have hle : (Int.tdiv n d).natAbs ≤ n.natAbs := by
    rw [Int.natAbs_tdiv]; exact Nat.div_le_self _ _
  have hcl := truncCeil_isCeil n d hd (decide (Int.emod n d ≠ 0)) (emod_flag n d)
  have hzero : Int.emod 0 d = 0 := Int.zero_emod d
  have hMIN : aeneas_soroban_shim.I256.MIN = -2^255 := rfl
  have hMAX : aeneas_soroban_shim.I256.MAX = 2^255 - 1 := rfl
  unfold truncCeil at hcl
  unfold math.fp_core.div_ceil_i256
  step as ⟨z, hz⟩
  step as ⟨q, hq⟩
  · simp only [aeneas_soroban_shim.I256.InRange, hMIN, hMAX]; omega
  step as ⟨r, hr⟩
  step as ⟨b, hb⟩
  · simp [aeneas_soroban_shim.I256.Insts.CoreCmpPartialEqI256.eq]
  subst hz hq hr
  -- the add-one step never overflows: |tdiv n d| ≤ |n| ≤ 2^254
  have hadd : ∀ (one : aeneas_soroban_shim.I256), (one : Int) = 1 →
      (aeneas_soroban_shim.I256.add (Int.tdiv n d : aeneas_soroban_shim.I256) one)
        ⦃ c => (c : Int) = Int.tdiv n d + 1 ⦄ := by
    intro one hone
    step as ⟨c, hc⟩
    · simp only [aeneas_soroban_shim.I256.InRange, hMIN, hMAX, hone]; omega
    rw [hc, hone]
  by_cases hex : Int.emod n d = 0
  · have hbf : b = false := by
      cases b
      · rfl
      · exact absurd (hb.mp rfl) (by simp [hex])
    subst hbf
    have hc' : IsCeil n d (Int.tdiv n d) := by simpa [hex] using hcl
    cases nonneg <;> simp only [Bool.false_eq_true, ↓reduceIte, bind_ok, WP.spec_ok] <;>
      exact hc'
  · have hbt : b = true := hb.mpr (by simpa using hex)
    subst hbt
    cases nonneg with
    | true =>
      simp only [↓reduceIte, bind_ok]
      rcases hnn rfl with h0 | hs
      · exact absurd (h0 ▸ hzero) hex
      · rw [if_pos (by simp [hex, hs])] at hcl
        step as ⟨one, hone⟩
        apply WP.spec_mono (hadd one (by simp [hone]))
        intro c hc; rw [hc]; exact hcl
    | false =>
      simp only [Bool.false_eq_true, ↓reduceIte, i256_lt_eq, bind_ok]
      by_cases hs : (n < 0) = (d < 0)
      · have he : (decide (n < 0) = decide ((d : Int) < 0)) = True := by
          simp [hs]
        have hdec : (decide (decide (n < 0) = decide ((d : Int) < 0))) = true := by
          simp [hs]
        simp only [hdec, ↓reduceIte]
        rw [if_pos (by simp [hex, hs])] at hcl
        step as ⟨one, hone⟩
        apply WP.spec_mono (hadd one (by simp [hone]))
        intro c hc; rw [hc]; exact hcl
      · have hdec : (decide (decide (n < 0) = decide ((d : Int) < 0))) = false := by
          by_cases hn0 : n < 0 <;> by_cases hd0 : d < 0 <;> simp_all
        simp only [hdec, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
        rwa [if_neg (by simp [hs])] at hcl

/-! ## Ceiling, native `i128` path -/

/-- `div_ceil_i128` never aborts, and whenever it returns `Some q`, `q` is the
ceiling of `p / d`. (It returns `None` on `i128::MIN / -1` and when the
ceiling is `i128::MAX + 1`; the caller then takes the `I256` path.) -/
theorem div_ceil_i128_spec (p d : Std.I128) (hd : d.val ≠ 0) :
    math.fp_core.div_ceil_i128 p d ⦃ o => ∀ q, o = some q → IsCeil p.val d.val q.val ⦄ := by
  have hcl := truncCeil_isCeil p.val d.val hd (decide (Int.tmod p.val d.val ≠ 0))
    (by rw [decide_eq_true_iff, Int.dvd_iff_tmod_eq_zero])
  unfold truncCeil at hcl
  unfold math.fp_core.div_ceil_i128
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
      rw [ht]; simpa [hex] using hcl
    · have hr0 : (r != 0#i128) = true := by
        simp only [bne_iff_ne, ne_eq]; intro h; apply hex; rw [← hr, h]; rfl
      simp only [hr0, ↓reduceIte]
      by_cases hs : (p.val < 0) = (d.val < 0)
      · have hne : ((p < 0#i128) = (d < 0#i128)) := by
          simp [IScalar.lt_equiv, hs]
        simp only [hne, ↓reduceIte, WP.spec_ok]
        have hadd := I128.checked_add_bv_spec t 1#i128
        intro q hq
        rw [show I128.checked_add t 1#i128 = core.num.checked_add_IScalar t 1#i128 from rfl] at hq
        rw [hq] at hadd
        obtain ⟨_, _, hqv, _⟩ := hadd
        rw [hqv, ht]
        rw [if_pos (by simp [hex, hs])] at hcl
        simpa using hcl
      · have hne : ¬ ((p < 0#i128) = (d < 0#i128)) := by
          intro h; apply hs; simpa [IScalar.lt_equiv] using h
        simp only [hne, ↓reduceIte, WP.spec_ok, Option.some.injEq]
        rintro q rfl
        rw [ht]; rwa [if_neg (by simp [hs])] at hcl

/-! ## `mul_div_ceil` -/

/-- **`mul_div_ceil` is exact.** For every `i128` `x`, `y` and nonzero `d`
whose exact ceiling `⌈x*y/d⌉` fits `i128`, `mul_div_ceil` returns (does not
abort) and its result is that ceiling. No other bound on the inputs; both the
native path (`div_ceil_i128`) and the `I256` fallback (`div_ceil_i256`) are
covered.

Hypotheses: `d ≠ 0` (production calls `require_nonzero_divisor` first and
aborts otherwise; every caller in `rates` divides by an index or by `RAY`,
both positive); the result fits `i128` (otherwise production aborts with
`MathOverflow`). -/
theorem mul_div_ceil_spec (env : Env) (x y d : Std.I128) (hd : d.val ≠ 0)
    (hfit : ∃ q : Int, IsCeil (x.val * y.val) d.val q ∧ -2^127 ≤ q ∧ q < 2^127) :
    math.fp_core.mul_div_ceil env x y d ⦃ q => IsCeil (x.val * y.val) d.val q.val ⦄ := by
  obtain ⟨qc, hqc, hlo, hhi⟩ := hfit
  have hbounds := i128_prod_bounds x y
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
  have wide : (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let nonneg ← math.fp_core.quotient_is_nonnegative x y d
      let i ← aeneas_soroban_shim.I256.mul x256 y256
      let i1 ← math.fp_core.div_ceil_i256 env i d256 nonneg
      math.fp_core.to_i128 env i1) ⦃ q => IsCeil (x.val * y.val) d.val q.val ⦄ := by
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
    step with div_ceil_i256_spec env i d.val hd (by rw [hi]; exact hpb)
        nn (by rw [hi]; exact hnn) as ⟨f, hf⟩
    rw [hi] at hf
    have hfq : f = qc := IsCeil.unique hd hf hqc
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
  unfold math.fp_core.mul_div_ceil math.fp_core.require_nonzero_divisor
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
      math.fp_core.mul_div_ceil.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
    step with div_ceil_i128_spec m d hd as ⟨o1, ho1⟩
    cases o1 with
    | none => exact wide
    | some q =>
      simp only [WP.spec_ok]
      rw [← hm]; exact ho1 q rfl

-- (The `mul_div_floor_saturating` section of the original is omitted: the
-- controller extraction does not include that function.)

end ControllerAeneas.Proofs
