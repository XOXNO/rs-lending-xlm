/-
`rates::compound::compound_interest` (common/src/rates/compound.rs): the
truncated Taylor series `1 + x + x²/2! + … + x⁸/8!` of `e^x` over the array
`[2, 6, 24, 120, 720, 5040, 40320]`, with `x = rate · delta_ms` (exact,
`Ray::mul_int`), each power `pow ← half_up(pow · x / RAY)` (`Ray::mul`) and each
term `half_up(pow / divisor)` (`Ray::div_by_int`).

Proved: for a non-negative rate whose exponent `x = rate · delta_ms` is at most
`3·RAY`, the call never aborts and returns a factor `f` with
`RAY + x ≤ f ≤ 21·RAY`. The production exponent is at most `2·RAY + 1.6·10^10`
(`compound_x_le` in `Accrual.lean`: per-ms rate `≤ half_up(2·RAY / ms-per-year)`,
`delta_ms ≤ MAX_COMPOUND_DELTA_MS`). The upper bound uses the divisors:
with `x ≤ 3·RAY` the k-th power is at most `3^k·RAY` (half-up rounding of an
integer product below `3·B·RAY + RAY/2` cannot exceed `3·B`), and
`4 + Σ_{k=2..8} 3^k/k! < 21`.
-/
import PoolAeneas.Proofs.Wrappers
import PoolAeneas.Proofs.RatesHalfUpWrappers

/- Plain (non-`module`) file, like `Wrappers.lean`. -/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- The loop over an exhausted divisor iterator returns the running sum. -/
theorem ci_loop_nil (env : Env) (x sum pow : math.fp.Ray) :
    rates.compound.compound_interest_loop ([] : List Std.I128) env x sum pow = ok sum := by
  unfold rates.compound.compound_interest_loop
  rw [loop]
  simp [rates.compound.compound_interest_loop.body,
    core.array.iter.IntoIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The exhausted loop satisfies any postcondition its running sum satisfies. -/
theorem ci_loop_nil_spec (env : Env) (x sum pow : math.fp.Ray) {Q : math.fp.Ray → Prop}
    (h : Q sum) :
    rates.compound.compound_interest_loop ([] : List Std.I128) env x sum pow ⦃ Q ⦄ := by
  rw [ci_loop_nil]; simpa using h

/-- One loop iteration with divisor `d`: `pow ← pow·x` (half-up), add
`pow / d` (half-up) to the sum, continue with the remaining divisors. -/
theorem ci_loop_cons (env : Env) (x sum pow d : Std.I128) (l : List Std.I128) :
    rates.compound.compound_interest_loop (d :: l) env x sum pow =
      (do
        let pow1 ← math.fp.Ray.mul pow env x
        let r ← math.fp.Ray.div_by_int pow1 env d
        let sum1 ← math.fp.Ray.checked_add sum env r
        rates.compound.compound_interest_loop l env x sum1 pow1) := by
  conv => lhs; unfold rates.compound.compound_interest_loop; rw [loop]
  simp [rates.compound.compound_interest_loop.body,
    core.array.iter.IntoIter.Insts.CoreIterTraitsIteratorIterator.next,
    rates.compound.compound_interest_loop]

/-- Half-up rounding of `p·x/RAY` with `0 ≤ p ≤ B`, `0 ≤ x ≤ 3·RAY` is in
`[0, 3·B]` (no rounding excess: `2q·RAY ≤ 6B·RAY + RAY` forces `q ≤ 3B`). -/
theorem hu_pow_bound {p x q B : Int} (hp : 0 ≤ p) (hpB : p ≤ B) (hx : 0 ≤ x)
    (hx3 : x ≤ 3 * 10^27) (h : IsHalfUp (p * x) (10^27) q) : 0 ≤ q ∧ q ≤ 3 * B := by
  refine ⟨IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hp hx) h, ?_⟩
  have h1 := h.mul_le
  have h2 : p * x ≤ B * (3 * 10^27) := by nlinarith
  by_contra hc
  have : (3 * B + 1) * 10^27 ≤ q * 10^27 := by nlinarith
  nlinarith

/-- **One step of the series, with bounds.** If `0 ≤ pow ≤ B`,
`0 ≤ x ≤ 3·RAY`, `0 ≤ sum` and the sum stays representable, the iteration with
divisor `d > 0` does not abort; the continuation receives the new power in
`[0, 3B]` and a new sum that grew by a term `t ≥ 0` with `2·t·d ≤ 6B + d`. -/
theorem ci_step (env : Env) (x sum pow d : Std.I128) (l : List Std.I128) (dv B : Int)
    {Q : math.fp.Ray → Prop}
    (hd : d.val = dv) (hdv : 0 < dv) (hx0 : 0 ≤ x.val) (hx : x.val ≤ 3 * 10^27)
    (hp0 : 0 ≤ pow.val) (hp : pow.val ≤ B)
    (hs0 : 0 ≤ sum.val) (hS : sum.val + 3 * B < 2^127)
    (k : ∀ pow1 sum1 : Std.I128, 0 ≤ pow1.val → pow1.val ≤ 3 * B →
      0 ≤ sum1.val - sum.val → 2 * ((sum1.val - sum.val) * dv) ≤ 6 * B + dv →
      rates.compound.compound_interest_loop l env x sum1 pow1 ⦃ Q ⦄) :
    rates.compound.compound_interest_loop (d :: l) env x sum pow ⦃ Q ⦄ := by
  rw [ci_loop_cons]
  obtain ⟨q0, hq0⟩ := IsHalfUp.exists (pow.val * x.val) (10^27) (by norm_num)
  have hq0b := hu_pow_bound hp0 hp hx0 hx hq0
  step with RatesProofs.ray_mul_spec env pow x hp0 hx0 ⟨q0, hq0, by omega⟩ as ⟨p1, hp1⟩
  have hp1b := hu_pow_bound hp0 hp hx0 hx hp1
  unfold math.fp.Ray.div_by_int
  step with div_by_int_half_up_spec env p1 d (by omega) as ⟨r, hr⟩
  unfold IsHalfAway at hr
  simp only [hp1b.1, ↓reduceIte] at hr
  rw [hd] at hr
  have hr0 := IsHalfUp.nonneg hdv hp1b.1 hr
  have hr1 := hr.mul_le
  have hrp : r.val ≤ p1.val := by
    by_contra hc
    have : (p1.val + 1) * dv ≤ r.val * dv := by nlinarith
    nlinarith
  step with ray_checked_add_spec env sum r (by omega) (by omega) as ⟨s1, hs1⟩
  exact k p1 s1 hp1b.1 hp1b.2 (by omega) (by rw [hs1]; ring_nf; ring_nf at hr1; omega)

/-- **`compound_interest` never aborts and returns a factor in
`[RAY + rate·delta_ms, 21·RAY]`.** For a rate `≥ 0` whose exponent
`x = rate·delta_ms` is at most `3·RAY` (production: `≤ 2·RAY + 1.6·10^10`, see
`compound_x_le`), the truncated Taylor series of `e^x` (all eight terms
non-negative) is at least `1 + x` in RAY and, since the k-th power is at most
`3^k·RAY` and `4 + Σ_{k=2..8} 3^k/k! < 21`, at most `21·RAY`. For
`delta_ms = 0` the result is exactly `RAY`. -/
theorem compound_interest_spec (env : Env) (rate : math.fp.Ray) (delta : Std.U64)
    (hr : 0 ≤ rate.val) (hx : rate.val * delta.val ≤ 3 * 10^27) :
    rates.compound.compound_interest env rate delta ⦃ f =>
      10^27 + rate.val * delta.val ≤ f.val ∧ f.val ≤ 21 * 10^27 ⦄ := by
  unfold rates.compound.compound_interest
  by_cases hd : delta = 0#u64
  · subst hd
    simp only [↓reduceIte, WP.spec_ok, ray_one_val]
    simp
  · simp only [hd, ↓reduceIte]
    have hdpos : (0 : Int) ≤ delta.val := by omega
    have hx0 : 0 ≤ rate.val * delta.val := Int.mul_nonneg hr hdpos
    step with i128_from_u64_spec delta as ⟨i, hi⟩
    step with ray_mul_int_spec env rate i (by rw [hi]; omega) as ⟨x, hxv⟩
    rw [hi] at hxv
    step with ray_checked_add_spec env math.fp.Ray.ONE x
      (by rw [ray_one_val]; omega) (by rw [ray_one_val]; omega) as ⟨s, hs⟩
    rw [ray_one_val] at hs
    simp only [Array.Insts.CoreIterTraitsCollectIntoIteratorTIntoIter.into_iter, Array.make,
      bind_ok]
    apply ci_step env x s x _ _ 2 (3 * 10^27) (by simp) (by norm_num) (by omega) (by omega)
      (by omega) (by omega) (by omega) (by omega)
    intro p1 s1 hp1 hp1' hs1 hs1'
    apply ci_step env x s1 p1 _ _ 6 (9 * 10^27) (by simp) (by norm_num) (by omega) (by omega)
      (by omega) (by omega) (by omega) (by omega)
    intro p2 s2 hp2 hp2' hs2 hs2'
    apply ci_step env x s2 p2 _ _ 24 (27 * 10^27) (by simp) (by norm_num) (by omega) (by omega)
      (by omega) (by omega) (by omega) (by omega)
    intro p3 s3 hp3 hp3' hs3 hs3'
    apply ci_step env x s3 p3 _ _ 120 (81 * 10^27) (by simp) (by norm_num) (by omega) (by omega)
      (by omega) (by omega) (by omega) (by omega)
    intro p4 s4 hp4 hp4' hs4 hs4'
    apply ci_step env x s4 p4 _ _ 720 (243 * 10^27) (by simp) (by norm_num) (by omega) (by omega)
      (by omega) (by omega) (by omega) (by omega)
    intro p5 s5 hp5 hp5' hs5 hs5'
    apply ci_step env x s5 p5 _ _ 5040 (729 * 10^27) (by simp) (by norm_num) (by omega)
      (by omega) (by omega) (by omega) (by omega) (by omega)
    intro p6 s6 hp6 hp6' hs6 hs6'
    apply ci_step env x s6 p6 _ _ 40320 (2187 * 10^27) (by simp) (by norm_num) (by omega)
      (by omega) (by omega) (by omega) (by omega) (by omega)
    intro p7 s7 hp7 hp7' hs7 hs7'
    apply ci_loop_nil_spec
    constructor <;> omega

end PoolAeneas.Proofs
