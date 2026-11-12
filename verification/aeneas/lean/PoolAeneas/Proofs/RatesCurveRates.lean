-- Copied from RatesAeneas/Proofs/CurveRates.lean (2026-11-12 snapshot) for the pool
-- extraction (global_sync / accrue_step proofs), with `RatesAeneas`/`rates_aeneas`/
-- `aeneas_soroban_shim` renamed to `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim`,
-- `constants.production.` to `constants.`, and the theorem namespace
-- `RatesAeneas.Proofs` to `PoolAeneas.RatesProofs` (several names clash with
-- pool-local lemmas in `PoolAeneas.Proofs`) and `types.MarketParams` to
-- `types.pool.MarketParams`. Proof text otherwise unchanged.
import PoolAeneas.Proofs.RatesCurve

/-
`rates::curve::calculate_borrow_rate` (per-millisecond rate) and
`rates::curve::calculate_deposit_rate`.
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas
open PoolAeneas.Proofs

set_option maxRecDepth 20000

namespace PoolAeneas.RatesProofs

local notation "rv " r:max => IScalar.val (ty := IScalarTy.I128) r

/-- **`calculate_borrow_rate` is the annual rate divided by
`MILLISECONDS_PER_YEAR`, rounded half up, and never aborts.** Under the same
hypotheses as `calculate_annual_borrow_rate_spec` (utilization `≥ 0`, a rate
model passing `InterestRateModel::verify`), the per-millisecond rate `r`
satisfies `r = half_up(a / 31_556_926_000)` where `a` is the annual rate at
the clamped utilization (`AnnualRate`), which lies in
`[base_borrow_rate, max_borrow_rate]`. -/
theorem calculate_borrow_rate_spec (env : Env) (u : math.fp.Ray) (p : types.pool.MarketParams)
    (hu : 0 ≤ rv u) (hp : ValidRateModel p) :
    rates.curve.calculate_borrow_rate env u p ⦃ r =>
      ∃ a, AnnualRate p (min (rv u) (10^27)) a ∧
        rv p.base_borrow_rate ≤ a ∧ a ≤ rv p.max_borrow_rate ∧
        IsHalfUp a 31556926000 (rv r) ⦄ := by
  unfold rates.curve.calculate_borrow_rate math.fp.Ray.div_by_int
  step with calculate_annual_borrow_rate_spec env u p hu hp as ⟨a, ha⟩
  have hab := AnnualRate.bounds hp (by omega) (by omega) ha
  have hmpy : constants.shared.MILLISECONDS_PER_YEAR = 31556926000#u64 := by
    unfold constants.shared.MILLISECONDS_PER_YEAR; rfl
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

-- (`range_contains_eq` and `calculate_deposit_rate_spec` are left out of this copy:
-- `calculate_deposit_rate` is not reachable from the pool extraction roots.)

end PoolAeneas.RatesProofs
