-- Copied from RatesAeneas/Proofs/TypedWrappers.lean (the #225 `mul_ray_ratio` section only) (after #225) for the pool extraction, with
-- `RatesAeneas`/`rates_aeneas`/`aeneas_soroban_shim` renamed to `PoolAeneas`/`pool_aeneas`/
-- `aeneas_pool_shim`, `constants.production.` to `constants.`, the theorem namespace
-- `RatesAeneas.Proofs` to `PoolAeneas.RatesProofs` and `types.MarketParams` to
-- `types.pool.MarketParams`. Proof text otherwise unchanged.
import PoolAeneas.Proofs.RatesHalfUpWrappers

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec PoolAeneas.HalfUpSpec
open pool_aeneas
open PoolAeneas.Proofs

set_option maxRecDepth 20000

namespace PoolAeneas.RatesProofs

/-! ## Half-up ratio wrapper (#225) -/

/-- `Ray::mul_ray_ratio` is literally one `mul_div_half_up(self, numerator, denominator)`. -/
theorem ray_mul_ray_ratio_eq (env : Env) (x n d : Std.I128) :
    math.fp.Ray.mul_ray_ratio x env n d = math.fp_core.mul_div_half_up env x n d := by
  unfold math.fp.Ray.mul_ray_ratio
  exact Std.bind_pure _

/-- **`Ray::mul_ray_ratio` is one half-up rounding** (added by #225).
For `x, n ≥ 0` and `d > 0` whose half-up quotient `q` of `x*n/d` fits `i128`,
`x.mul_ray_ratio(n, d)` returns (does not abort) exactly `q`: the ratio `n/d`
is applied unquantized, with no intermediate rounding. -/
theorem ray_mul_ray_ratio_spec (env : Env) (x n d : Std.I128)
    (hx : 0 ≤ x.val) (hn : 0 ≤ n.val) (hd : 0 < d.val)
    (hfit : ∃ q : Int, IsHalfUp (x.val * n.val) d.val q ∧ q < 2^127) :
    math.fp.Ray.mul_ray_ratio x env n d ⦃ r => IsHalfUp (x.val * n.val) d.val r.val ⦄ := by
  rw [ray_mul_ray_ratio_eq]
  exact mul_div_half_up_spec env x n d hx hn hd hfit

/-- **When `Ray::mul_ray_ratio` aborts.** It returns (does not abort) exactly
when `x ≥ 0`, `n ≥ 0`, `d > 0` and the half-up quotient of `x*n/d` fits
`i128`; a negative operand, a zero or negative denominator, or an overflowing
result aborts (`mul_div_half_up_returns_iff`). -/
theorem ray_mul_ray_ratio_returns_iff (env : Env) (x n d : Std.I128) :
    (math.fp.Ray.mul_ray_ratio x env n d ⦃ _ => True ⦄) ↔
      (0 ≤ x.val ∧ 0 ≤ n.val ∧ 0 < d.val ∧
        ∃ q : Int, IsHalfUp (x.val * n.val) d.val q ∧ q < 2^127) := by
  rw [ray_mul_ray_ratio_eq]
  exact mul_div_half_up_returns_iff env x n d

/-- **`Ray::mul_ray_ratio` by a ratio at most one never aborts.** For
`x ≥ 0` and `0 ≤ n ≤ d`, `d > 0`, it returns `q = half_up(x*n/d)` with
`0 ≤ q ≤ x` (so the result always fits). -/
theorem ray_mul_ray_ratio_frac_spec (env : Env) (x n d : Std.I128)
    (hx : 0 ≤ x.val) (hn : 0 ≤ n.val) (hnd : n.val ≤ d.val) (hd : 0 < d.val) :
    math.fp.Ray.mul_ray_ratio x env n d ⦃ r =>
      IsHalfUp (x.val * n.val) d.val r.val ∧ 0 ≤ r.val ∧ r.val ≤ x.val ⦄ := by
  have hxb := x.hBounds
  simp only [IScalarTy.numBits] at hxb; norm_num at hxb
  apply WP.spec_mono (ray_mul_ray_ratio_spec env x n d hx hn hd
    (halfUp_ratio_fits x.val n.val d.val hd hx (by omega) hn hnd))
  intro r hr
  exact ⟨hr, IsHalfUp.le_of_ratio_le_one hd hx hn hnd hr⟩

end PoolAeneas.RatesProofs
