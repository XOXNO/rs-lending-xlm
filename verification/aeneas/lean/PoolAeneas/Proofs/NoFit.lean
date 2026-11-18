import PoolAeneas.Proofs.Summaries

/-
The withdraw and repay legs without their fit hypotheses.

`withdraw_accounting_spec`, `withdraw_summary_partial` and
`repay_summary_holds` assume that the value of the position fits `i128`
(the shadow of INV-ACCT-10, "a position is at most its market total"). That
fit is exactly the condition under which the leg returns, so a successful
leg implies it. The theorems below derive it from the leg's `ok` result and
restate the conclusions without it: they are not conditional on INV-ACCT-10.
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas
open PoolAeneas.RatesProofs

set_option maxRecDepth 20000
set_option maxHeartbeats 1000000

namespace PoolAeneas.Proofs

/-- **`resolve_withdrawal` aborts when the half-up position value does not
fit `i128`.** Pool-extraction port of the RatesAeneas theorem of the same
name: the first call (`Ray::mul` in `unscale_supply`) fails. -/
theorem resolve_withdrawal_aborts (env : Env) (amount pos index : Std.I128) (dec : Std.U32)
    (hover : ¬ ∃ q : Int, IsHalfUp (pos.val * index.val) (10^27) q ∧ q < 2^127) :
    ¬ (rates.scaling.resolve_withdrawal env amount pos index dec ⦃ _ => True ⦄) := by
  intro h
  unfold rates.scaling.resolve_withdrawal rates.scaling.unscale_supply
    rates.scaling.scaled_to_original math.fp.Ray.mul at h
  have hret : math.fp_core.mul_div_half_up env pos index constants.shared.RAY
      ⦃ _ => True ⦄ :=
    WP.spec_mono (spec_bind_inv (spec_bind_inv (spec_bind_inv h))) (fun _ _ => trivial)
  obtain ⟨_, _, _, hfit⟩ := (mul_div_half_up_returns_iff env pos index _).1 hret
  rw [ray_val_hu] at hfit
  exact hover hfit

/-- **A successful withdraw leg implies the fit hypothesis of
`withdraw_accounting_spec`.** If `ops::withdraw::accounting` returns, the
half-up value `hu(position·supply_index, RAY)` fits `i128`. -/
theorem withdraw_fit_of_ok (env : Env) (isl : Bool) (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    ∃ q : Int, IsHalfUp (entry.action.position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127 := by
  unfold ops.withdraw.accounting at h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨⟨c0, p0⟩, hl, h⟩ := bind_eq_ok h
  obtain ⟨_, _, hs0, rfl⟩ := load_leg_ok env _ _ _ hl
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨⟨bu, gr⟩, hrc, _⟩ := bind_eq_ok h
  unfold ops.withdraw.resolve_close_or_partial at hrc
  obtain ⟨⟨bu0, gr0⟩, hrw, _⟩ := bind_eq_ok hrc
  unfold market_cache.scale.Cache.resolve_withdrawal at hrw
  by_contra hover
  apply resolve_withdrawal_aborts _ _ _ _ _ hover
  rw [hrw]; simp

/-- **`withdraw_summary_partial` without the fit hypothesis.** Every
successful withdraw leg meets the Certora `withdraw_summary` clauses that
hold (all but the dust clause, see `withdraw_zero_amount_closes_dust`), given
only `PoolInv` and `decimals ≤ 18`. -/
theorem withdraw_summary_partial_nofit (env : Env) (isl : Bool)
    (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    0 ≤ r.mutation.position.scaled_amount.val ∧
    (0 < entry.action.amount.val → 0 < entry.action.position.scaled_amount.val →
      r.mutation.position.scaled_amount.val < entry.action.position.scaled_amount.val) ∧
    (entry.action.position.scaled_amount.val ≤ 0 →
      r.mutation.position.scaled_amount.val = entry.action.position.scaled_amount.val) ∧
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ entry.action.amount.val ∧
    IndexInRange r.mutation.market_index ∧ r.mutation.asset_decimals.val ≤ 18 :=
  withdraw_summary_partial env isl entry c hsync hinv hdec
    (withdraw_fit_of_ok env isl entry c hsync r h) r h

/-- **A successful withdraw leg preserves `PoolInv` without the fit
hypothesis.** Same conclusion as the `PoolInv` clause of
`withdraw_accounting_spec`, so INV-ACCT-01 after a withdraw is not
conditional on INV-ACCT-10. -/
theorem withdraw_poolInv_nofit (env : Env) (isl : Bool) (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    PoolInv r.cache := by
  have := withdraw_accounting_spec env isl entry c hsync hinv hdec
    (withdraw_fit_of_ok env isl entry c hsync r h) r h
  exact this.2.2.2.2.2.2.2.2.2.2.2.2.2.2.2.2.2.2.1

/-- **A successful repay leg implies the fit hypothesis of
`repay_summary_holds`.** If `ops::repay::accounting` returns, the ceiled debt
value `⌈position·borrow_index/RAY⌉` fits `i128` (`mul_div_ceil` returned it). -/
theorem repay_fit_of_ok (env : Env) (action : types.pool.PoolAction)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (r : ops.repay.RepayOutcome) (h : ops.repay.accounting env action = ok r) :
    ∃ q : Int, IsCeil (action.position.scaled_amount.val * c.borrow_index.val) (10^27) q ∧
      q < 2^127 := by
  unfold ops.repay.accounting at h
  obtain ⟨⟨c0, p0⟩, hl, h⟩ := bind_eq_ok h
  obtain ⟨_, _, hs0, rfl⟩ := load_leg_ok env _ _ _ hl
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨⟨bu, ov⟩, hrr, _⟩ := bind_eq_ok h
  unfold market_cache.scale.Cache.resolve_repay rates.scaling.resolve_repay
    rates.scaling.unscale_borrow_ceil math.fp.Ray.mul_ceil at hrr
  obtain ⟨d, hd, _⟩ := bind_eq_ok hrr
  obtain ⟨q0, hq0, _⟩ := bind_eq_ok hd
  obtain ⟨q, hq, _⟩ := bind_eq_ok hq0
  have := mul_div_ceil_eq_ok c.env _ _ _ q (by rw [ray_val_hu]; norm_num) hq
  rw [ray_val_hu] at this
  exact ⟨q.val, this, by have := q.hBounds; simp only [IScalarTy.numBits] at this; omega⟩

/-- **The Certora `repay_summary` amount clauses without the fit
hypothesis.** Every successful repay leg books an amount in
`[0, action.amount]`. -/
theorem repay_summary_holds_nofit (env : Env) (action : types.pool.PoolAction)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (r : ops.repay.RepayOutcome) (h : ops.repay.accounting env action = ok r) :
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ action.amount.val := by
  obtain ⟨_, _, h1, h2, _⟩ := repay_summary_holds env action c hsync hinv hdec
    (repay_fit_of_ok env action c hsync r h) r h
  exact ⟨h1, h2⟩

end PoolAeneas.Proofs
