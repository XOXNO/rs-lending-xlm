import RatesAeneas.Proofs.Curve

/-
`rates::curve::calculate_borrow_rate` (per-millisecond rate) and
`rates::curve::calculate_deposit_rate`.
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open rates_aeneas

set_option maxRecDepth 20000

namespace RatesAeneas.Proofs

local notation "rv " r:max => IScalar.val (ty := IScalarTy.I128) r

/-- **`calculate_borrow_rate` is the annual rate divided by
`MILLISECONDS_PER_YEAR`, rounded half up, and never aborts.** Under the same
hypotheses as `calculate_annual_borrow_rate_spec` (utilization `≥ 0`, a rate
model passing `InterestRateModel::verify`), the per-millisecond rate `r`
satisfies `r = half_up(a / 31_556_926_000)` where `a` is the annual rate at
the clamped utilization (`AnnualRate`), which lies in
`[base_borrow_rate, max_borrow_rate]`. -/
theorem calculate_borrow_rate_spec (env : Env) (u : math.fp.Ray) (p : types.MarketParams)
    (hu : 0 ≤ rv u) (hp : ValidRateModel p) :
    rates.curve.calculate_borrow_rate env u p ⦃ r =>
      ∃ a, AnnualRate p (min (rv u) (10^27)) a ∧
        rv p.base_borrow_rate ≤ a ∧ a ≤ rv p.max_borrow_rate ∧
        IsHalfUp a 31556926000 (rv r) ⦄ := by
  unfold rates.curve.calculate_borrow_rate math.fp.Ray.div_by_int
  step with calculate_annual_borrow_rate_spec env u p hu hp as ⟨a, ha⟩
  have hab := AnnualRate.bounds hp (by omega) (by omega) ha
  have hmpy : constants.production.shared.MILLISECONDS_PER_YEAR = 31556926000#u64 := by
    unfold constants.production.shared.MILLISECONDS_PER_YEAR; rfl
  rw [hmpy]
  step with UScalar.hcast_inBounds_spec .I128 (31556926000#u64)
    (by scalar_tac) as ⟨n, hn⟩
  have hn' : n.val = 31556926000 := by rw [hn]; simp
  step with div_by_int_half_up_spec env a n (by omega) as ⟨q, hq⟩
  have hb0 := hp.base_nonneg
  refine ⟨rv a, ha, hab.1, hab.2, ?_⟩
  unfold IsHalfAway at hq
  rw [if_pos (by omega), hn'] at hq
  exact hq

/-- `(lo..hi).contains(&x)` on `i128` is `lo ≤ x < hi` (via the trusted `Range::contains` model). -/
theorem range_contains_eq (lo hi x : Std.I128) :
    core.ops.range.Range.contains core.cmp.PartialOrdI128 core.cmp.PartialOrdI128
      core.cmp.PartialOrdI128 { start := lo, «end» := hi } x
      = ok (decide (lo.val ≤ x.val ∧ x.val < hi.val)) := by
  simp only [core.ops.range.Range.contains, core.cmp.PartialOrdI128, liftFun2,
    core.cmp.impls.PartialOrdI128.le, core.cmp.impls.PartialOrdI128.lt]
  by_cases h : lo.val ≤ x.val <;> simp [h]

/-- **`calculate_deposit_rate` never exceeds the borrow rate it is derived
from, and never aborts.** For a utilization in `[0, RAY]` (`utilization_le_ray`:
holds when borrowed ≤ supplied; the curve clamps the same way) and a
non-negative borrow rate (`calculate_borrow_rate`/`calculate_annual_borrow_rate`
return rates `≥ base_borrow_rate ≥ 0`), the deposit rate `d` satisfies
`0 ≤ d ≤ borrow_rate`. It is 0 when utilization is 0 or the reserve factor is
outside `[0, BPS)`; otherwise it is
`half_up(half_up(u * borrow_rate / RAY) * (BPS - reserve_factor) / BPS)`. -/
theorem calculate_deposit_rate_spec (env : Env) (u br : math.fp.Ray) (rf : math.fp.Bps)
    (hu : 0 ≤ rv u) (huR : rv u ≤ 10^27) (hbr : 0 ≤ rv br) :
    rates.curve.calculate_deposit_rate env u br rf ⦃ d =>
      0 ≤ rv d ∧ rv d ≤ rv br ∧
      ((rv u = 0 ∨ ¬ (0 ≤ rv rf ∧ rv rf < 10000)) → rv d = 0) ∧
      ((rv u ≠ 0 ∧ 0 ≤ rv rf ∧ rv rf < 10000) →
        ∃ x, IsHalfUp (rv u * rv br) (10^27) x ∧
          IsHalfUp (x * (10000 - rv rf)) 10000 (rv d)) ⦄ := by
  unfold rates.curve.calculate_deposit_rate
  simp only [ray_eq_eq, bind_ok, ray_zero_val_hu, math.fp.Bps.raw]
  by_cases h0 : rv u = 0
  · simp only [h0, decide_true, ↓reduceIte, WP.spec_ok, ray_zero_val_hu]
    refine ⟨le_refl _, hbr, fun _ => by simp_all, fun h => by simp_all⟩
  · simp only [h0, decide_false, Bool.false_eq_true, ↓reduceIte, range_contains_eq, bind_ok]
    have hbps : (constants.production.shared.BPS : Std.I128).val = 10000 := bps_val
    rw [hbps]
    have h00 : (0#i128 : Std.I128).val = 0 := by simp
    rw [h00]
    by_cases hr : 0 ≤ rv rf ∧ rv rf < 10000
    · simp only [hr, and_self, decide_true, ↓reduceIte]
      step with ray_mul_frac_spec env u br hu huR hbr as ⟨x, hx, hx0, hxb⟩
      step as ⟨i1, hi1⟩
      try rw [hbps] at hi1
      simp only [math.fp.Bps.from, core.convert.IntoFrom, core.convert.IntoFrom.into,
        core.convert.FromSame, core.convert.FromSame.from, bind_ok]
      step with bps_apply_to_ray_spec env i1 x (by omega) hx0 as ⟨d, hd, hd0, hdx⟩
      refine ⟨hd0, by omega, fun h => by simp_all, fun _ => ⟨rv x, hx, ?_⟩⟩
      rw [← hi1]; exact hd
    · have hr' : ¬ (0 ≤ rv rf ∧ rv rf < 10000) := hr
      simp only [hr', decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok, ray_zero_val_hu]
      exact ⟨le_refl _, hbr, fun _ => by simp_all, fun h => by simp_all⟩

end RatesAeneas.Proofs
