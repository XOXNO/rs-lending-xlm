/-
The Certora pool summaries (`certora/shared/summaries/pool.rs`), stated in
their own terms and derived from the leg theorems (`Legs.lean`,
`Legs2.lean`). Each theorem says: every successful return of the production
leg satisfies the summary's postconditions on the values the summary
constrains (the new scaled position, the returned amounts, and the
`nondet_market_index` / `nondet_asset_decimals` draws). Token side effects,
storage and events are not covered (docs/pool.md).
-/
import PoolAeneas.Proofs.Legs2

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- What `nondet_market_index_raw` assumes of a returned index pair:
`SUPPLY_INDEX_FLOOR_RAW ≤ supply_index ≤ MAX_SUPPLY_INDEX_RAY` and
`RAY ≤ borrow_index ≤ MAX_BORROW_INDEX_RAY`. -/
def IndexInRange (m : types.pool.MarketIndexRaw) : Prop :=
  10^24 ≤ m.supply_index.val ∧ m.supply_index.val ≤ 10^36 ∧
  10^27 ≤ m.borrow_index.val ∧ m.borrow_index.val ≤ 10^36

/-- The index pair of a cache satisfying `PoolInv` is in range. -/
theorem indexOf_inRange {c : market_cache.Cache} (h : PoolInv c) : IndexInRange (indexOf c) :=
  ⟨h.si_lo, h.si_hi, h.bi_lo, h.bi_hi⟩

/-- **`supply_summary` holds for `ops::supply::apply`.** Every return has:
`amount > 0 → new position > old` and otherwise `new = old`;
`actual_amount = amount`; the index pair in range; `asset_decimals ≤ 18`.
Same hypotheses as `supply_apply_spec`. -/
theorem supply_summary_holds (env : Env) (entry : types.pool.PoolSupplyEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (hfit : ∃ q : Int, IsFloor (entry.action.amount.val * 10^(27 - c.params.asset_decimals.val)
      * 10^27) c.supply_index.val q ∧ q < 2^127)
    (r : types.pool.PoolPositionMutation × types.pool.MarketStateSnapshot)
    (h : ops.supply.apply env entry = ok r) :
    (0 < entry.action.amount.val →
      entry.action.position.scaled_amount.val < r.1.position.scaled_amount.val) ∧
    (¬ 0 < entry.action.amount.val →
      r.1.position.scaled_amount.val = entry.action.position.scaled_amount.val) ∧
    r.1.actual_amount = entry.action.amount ∧
    IndexInRange r.1.market_index ∧ r.1.asset_decimals.val ≤ 18 := by
  obtain ⟨hamt, _, m, c', _, _, hmp, hmz, hp, ha, hi, hd, _⟩ :=
    supply_apply_spec env entry c hsync hinv hdec hA hfit r h
  refine ⟨fun h => by have := hmp h; omega, fun h => by have := hmz (by omega); omega, ha,
    by rw [hi]; exact indexOf_inRange hinv, by rw [hd]; exact hdec⟩

/-- **`borrow_summary` holds for `ops::borrow::accounting`.** Every return
has `amount > 0` and `new position > old` (the `amount ≤ 0` branch of the
summary is never taken: the leg aborts), `actual_amount = amount`, the
index pair in range, `asset_decimals ≤ 18`. -/
theorem borrow_summary_holds (env : Env) (entry : types.pool.PoolBorrowEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (r : ops.borrow.BorrowOutcome) (h : ops.borrow.accounting env entry = ok r) :
    0 < entry.action.amount.val ∧
    entry.action.position.scaled_amount.val < r.mutation.position.scaled_amount.val ∧
    r.mutation.actual_amount = entry.action.amount ∧
    IndexInRange r.mutation.market_index ∧ r.mutation.asset_decimals.val ≤ 18 := by
  obtain ⟨hamt, _, _, _, m, _, hm0, hp, ha, hi, hd, _⟩ :=
    borrow_accounting_spec env entry c hsync hinv hdec hA r h
  exact ⟨hamt, by omega, ha, by rw [hi]; exact indexOf_inRange hinv, by rw [hd]; exact hdec⟩

/-- **`withdraw_summary`, the parts that hold for `ops::withdraw::accounting`.**
Every return has: `new ≥ 0`; `amount > 0 ∧ position > 0 → new < position`;
`position ≤ 0 → new = position`; `amount > 0`-or-position-empty no-op
cases as in the summary except `amount = 0` (next theorem);
`0 ≤ actual_amount ≤ amount`; the index pair in range;
`asset_decimals ≤ 18`. -/
theorem withdraw_summary_partial (env : Env) (isl : Bool) (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsHalfUp (entry.action.position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    0 ≤ r.mutation.position.scaled_amount.val ∧
    (0 < entry.action.amount.val → 0 < entry.action.position.scaled_amount.val →
      r.mutation.position.scaled_amount.val < entry.action.position.scaled_amount.val) ∧
    (entry.action.position.scaled_amount.val ≤ 0 →
      r.mutation.position.scaled_amount.val = entry.action.position.scaled_amount.val) ∧
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ entry.action.amount.val ∧
    IndexInRange r.mutation.market_index ∧ r.mutation.asset_decimals.val ≤ 18 := by
  obtain ⟨_, hpos, _, hg0, hga, hn0, _, _, _, hlt, hp0, _, _, _, _, hi, hd, _⟩ :=
    withdraw_accounting_spec env isl entry c hsync hinv hdec hfit r h
  exact ⟨hn0, hlt, fun hp => by have := hp0 (by omega); omega, hg0, hga,
    by rw [hi]; exact indexOf_inRange hinv, by rw [hd]; exact hdec⟩

/-- **Where `withdraw_summary` is stricter than the code.** For `amount =
0` and a position whose half-up value `H = hu(hu(pos·si, RAY), 10^k)` is 0
(a dust position), every return closes the position: `new = 0`, while the
summary says `new = position`. With `position > 0` this contradicts the
summary. (Concrete instance: `withdraw_dust_instance`.) -/
theorem withdraw_zero_amount_closes_dust (env : Env) (isl : Bool)
    (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsHalfUp (entry.action.position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r)
    (h0 : entry.action.amount.val = 0) (rr : Int)
    (hrr : IsHalfUp (entry.action.position.scaled_amount.val * c.supply_index.val) (10^27) rr)
    (hH : IsHalfUp rr (10^(27 - c.params.asset_decimals.val)) 0) :
    r.mutation.position.scaled_amount.val = 0 ∧ r.mutation.actual_amount.val = 0 := by
  obtain ⟨_, _, _, _, hga, _, _, hall, _⟩ :=
    withdraw_accounting_spec env isl entry c hsync hinv hdec hfit r h
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  obtain ⟨F, hF⟩ := exists_isFloor (entry.action.position.scaled_amount.val * c.supply_index.val)
    (10^27 * 10^(27 - c.params.asset_decimals.val)) (by positivity)
  obtain ⟨_, hfull, _, _⟩ := hall rr 0 F hrr hH hF
  obtain ⟨hn, hg⟩ := hfull (by omega)
  have hg0 : 0 ≤ r.mutation.actual_amount.val := by
    obtain ⟨_, _, _, hg0, _⟩ := withdraw_accounting_spec env isl entry c hsync hinv hdec hfit r h
    exact hg0
  exact ⟨hn, by omega⟩

/-- The dust instance is real: an 0-decimal asset at supply index `RAY` and
a position of one scaled unit has `hu(1·RAY, RAY) = 1` and
`hu(1, 10^27) = 0`, so a zero-amount withdraw of it closes it. -/
theorem withdraw_dust_instance :
    IsHalfUp (1 * 10^27) (10^27) 1 ∧ IsHalfUp 1 (10^(27 - 0)) 0 := by
  unfold IsHalfUp; norm_num

/-- **`repay_summary` holds for `ops::repay::accounting`.** Every return
has: `amount > 0 ∧ position > 0 → 0 ≤ new < position`, otherwise `new =
position`; `0 ≤ actual_amount ≤ amount`; the index pair in range;
`asset_decimals ≤ 18`. -/
theorem repay_summary_holds (env : Env) (action : types.pool.PoolAction)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsCeil (action.position.scaled_amount.val * c.borrow_index.val) (10^27) q ∧
      q < 2^127)
    (r : ops.repay.RepayOutcome) (h : ops.repay.accounting env action = ok r) :
    (0 < action.amount.val ∧ 0 < action.position.scaled_amount.val →
      0 ≤ r.mutation.position.scaled_amount.val ∧
      r.mutation.position.scaled_amount.val < action.position.scaled_amount.val) ∧
    (¬ (0 < action.amount.val ∧ 0 < action.position.scaled_amount.val) →
      r.mutation.position.scaled_amount = action.position.scaled_amount) ∧
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ action.amount.val ∧
    IndexInRange r.mutation.market_index ∧ r.mutation.asset_decimals.val ≤ 18 := by
  obtain ⟨_, _, hn0, hnle, _, hp0, _, _, hlt, hnop, hi, hd, _⟩ :=
    repay_accounting_spec env action c hsync hinv hdec hfit r h
  exact ⟨fun ⟨ha, hp⟩ => ⟨hp0, hlt ha hp⟩, hnop, hn0, hnle,
    by rw [hi]; exact indexOf_inRange hinv, by rw [hd]; exact hdec⟩

/-- **`net_settle_summary` holds for `ops::net_settle::apply`.** Every
return has `0 ≤ settled ≤ amount`, `0 ≤ new_supply ≤ supply`, `0 ≤ new_debt
≤ debt`, both strict when `settled > 0`, both unchanged when `settled = 0`,
and the index pair in range. -/
theorem net_settle_summary_holds (env : Env) (entry : types.pool.PoolNetSettleEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfitS : ∃ q : Int, IsFloor (entry.supply_position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (hfitD : ∃ q : Int, IsCeil (entry.debt_position.scaled_amount.val * c.borrow_index.val)
      (10^27) q ∧ q < 2^127)
    (r : types.pool.PoolNetSettleResult × types.pool.MarketStateSnapshot)
    (h : ops.net_settle.apply env entry = ok r) :
    0 ≤ r.1.settled_amount.val ∧ r.1.settled_amount.val ≤ entry.amount.val ∧
    0 ≤ r.1.supply_position.scaled_amount.val ∧
    r.1.supply_position.scaled_amount.val ≤ entry.supply_position.scaled_amount.val ∧
    0 ≤ r.1.debt_position.scaled_amount.val ∧
    r.1.debt_position.scaled_amount.val ≤ entry.debt_position.scaled_amount.val ∧
    (0 < r.1.settled_amount.val →
      r.1.supply_position.scaled_amount.val < entry.supply_position.scaled_amount.val ∧
      r.1.debt_position.scaled_amount.val < entry.debt_position.scaled_amount.val) ∧
    (¬ 0 < r.1.settled_amount.val →
      r.1.supply_position.scaled_amount.val = entry.supply_position.scaled_amount.val ∧
      r.1.debt_position.scaled_amount.val = entry.debt_position.scaled_amount.val) ∧
    IndexInRange r.1.market_index := by
  obtain ⟨_, _, _, hg0, hgle, hs0, hsle, hd0, hdle, hpos, hzero, _, hi, _⟩ :=
    net_settle_apply_spec env entry c hsync hinv hdec hfitS hfitD r h
  exact ⟨hg0, hgle, hs0, hsle, hd0, hdle, hpos, fun hn => hzero (by omega),
    by rw [hi]; exact indexOf_inRange hinv⟩

/-- **`recapitalize_summary` holds for `ops::recapitalize::accounting`:**
every return has `0 ≤ actual_amount ≤ amount`. -/
theorem recapitalize_summary_holds (env : Env) (k : types.pool.HubAssetKey) (amount : Std.I128)
    (c : market_cache.Cache) (hsync : ops.synced_market env k = ok c) (hinv : PoolInv c)
    (r : ops.recapitalize.RecapitalizationOutcome)
    (h : ops.recapitalize.accounting env k amount = ok r) :
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ amount.val := by
  obtain ⟨_, _, h0, hle, _⟩ := recapitalize_accounting_spec env k amount c hsync hinv r h
  exact ⟨h0, hle⟩

/-- **`create_strategy_summary` holds for `ops::strategy::accounting`.**
Every return has `new position ≥ old`, `amount ≥ 0`, a fee `f` with
`0 ≤ f ≤ amount` (`f = 0` without `charge_fee`), `actual_amount = amount`,
`amount_received = amount − f`, the index pair in range,
`asset_decimals ≤ 18`. -/
theorem create_strategy_summary_holds (env : Env) (action : types.pool.PoolAction) (charge : Bool)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hpar : ParamsOk c)
    (hA : action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (r : ops.strategy.StrategyOutcome) (h : ops.strategy.accounting env action charge = ok r) :
    action.position.scaled_amount.val ≤ r.mutation.position.scaled_amount.val ∧
    0 ≤ action.amount.val ∧
    ∃ f : Int, 0 ≤ f ∧ f ≤ action.amount.val ∧ (charge = false → f = 0) ∧
      r.mutation.actual_amount = action.amount ∧
      r.mutation.amount_received.val = action.amount.val - f ∧
      IndexInRange r.mutation.market_index ∧ r.mutation.asset_decimals.val ≤ 18 := by
  obtain ⟨hamt, _, _, hf0, hfle, hfz, m, _, hm0, hp, ha, hrec, hi, hd, _⟩ :=
    strategy_accounting_spec env action charge c hsync hinv hpar hA r h
  exact ⟨by omega, by omega, r.fee.val, hf0, hfle, hfz, ha, hrec,
    by rw [hi]; exact indexOf_inRange hinv, by rw [hd]; exact hpar.1⟩

/-- **`claim_revenue_summary` holds for `ops::revenue::accounting`:** every
return has `actual_amount ≥ 0` (and, beyond the summary, `≤ cash`). -/
theorem claim_revenue_summary_holds (env : Env) (k : types.pool.HubAssetKey)
    (c : market_cache.Cache) (hsync : ops.synced_market env k = ok c) (hinv : PoolInv c)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hfit : c.revenue.val * c.supply_index.val < 2^127 * 10^27)
    (r : ops.revenue.RevenueOutcome) (h : ops.revenue.accounting env k = ok r) :
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ c.cash.val := by
  obtain ⟨_, _, _, _, h0, hle, _⟩ := revenue_accounting_spec env k c hsync hinv hdec hfit r h
  exact ⟨h0, hle⟩

end PoolAeneas.Proofs
