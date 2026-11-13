/-
The market value ceiling as a stored-state invariant (INV-IDX-06,
INV-ACCT-11).

`ValueCeil c`: the half-up value of total supply at the supply index and of
total debt at the borrow index are both at most `MAX_MARKET_VALUE_RAY`.

Every extracted leg keeps it:

* accrual (`accrue_chunk`, `global_sync`, `synced_market`), for any elapsed
  time: interest is capped at the room below the ceiling;
* the entries (supply, borrow, strategy opening) establish it outright: the
  committed cache passes the entry-ceiling guard, which is eight times lower;
* the exits (withdraw, repay, net settlement, seize, revenue claim) and
  recapitalization never raise a market value.

* the flash-loan fee (`flash::finalize`) is booked and then checked against
  the entry ceiling, so it establishes the invariant outright as well.

So no stored state reached through these legs has a market total above the
ceiling.

The last section turns the ceiling into the fit conditions the leg theorems
assume: a market total within the ceiling fits `i128`, and so does the value
of any position that is at most its market's total (the pool-side shape of
INV-ACCT-10).
-/
import PoolAeneas.Proofs.Legs2

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-! ## The invariant -/

/-- `shares` valued at `index`, half up, are at most `MAX_MARKET_VALUE_RAY`. -/
def ValueWithin (x i : Int) : Prop :=
  ∀ v, IsHalfUp (x * i) (10^27) v → v ≤ MaxMarketValue

/-- **The market value ceiling on a cache (INV-IDX-06):** total supply value
and total debt value are both at most `MAX_MARKET_VALUE_RAY`. -/
def ValueCeil (c : market_cache.Cache) : Prop :=
  ValueWithin c.supplied.val c.supply_index.val ∧
  ValueWithin c.borrowed.val c.borrow_index.val

/-- A smaller product has a smaller half-up value. -/
theorem ValueWithin.mono {x i x' i' : Int} (h : ValueWithin x i) (hle : x' * i' ≤ x * i) :
    ValueWithin x' i' := by
  intro v' hv'
  obtain ⟨v, hv⟩ := IsHalfUp.exists (x * i) (10^27) (by norm_num)
  exact le_trans (IsHalfUp.mono (by norm_num) hle hv' hv) (h v hv)

/-- The entry ceiling is below the value ceiling: a total that fits the entry
ceiling is within the value ceiling. -/
theorem ValueWithin.of_entryFits {x i : Int} (h : EntryFits x i) : ValueWithin x i := by
  obtain ⟨_, v, hv, hle⟩ := h
  intro v' hv'
  have e : v' = v := IsHalfUp.unique (by norm_num) hv' hv
  have hc : MaxMarketEntry ≤ MaxMarketValue := by unfold MaxMarketEntry MaxMarketValue; norm_num
  omega

/-- A leg that raises neither market product keeps the ceiling. -/
theorem ValueCeil.of_le {c c' : market_cache.Cache} (h : ValueCeil c)
    (hs : c'.supplied.val * c'.supply_index.val ≤ c.supplied.val * c.supply_index.val)
    (hb : c'.borrowed.val * c'.borrow_index.val ≤ c.borrowed.val * c.borrow_index.val) :
    ValueCeil c' :=
  ⟨h.1.mono hs, h.2.mono hb⟩

/-- A cache that passes the entry-ceiling guard is within the value ceiling. -/
theorem ValueCeil.of_entry_guard (env : Env) (c : market_cache.Cache) (hinv : PoolInv c)
    (h : ∃ u, guards.require_within_entry_ceiling env c = ok u) : ValueCeil c := by
  have hsh := hinv.shares
  unfold SharesInv at hsh
  have hsi := hinv.si_lo
  have hbi := hinv.bi_lo
  obtain ⟨h1, h2⟩ := (require_within_entry_ceiling_spec env c (by omega) (by omega) hsh.2.2
    (by omega)).mp h
  exact ⟨ValueWithin.of_entryFits h1, ValueWithin.of_entryFits h2⟩

/-! ## What the ceiling gives: values that fit `i128` -/

/-- The floor of `n/RAY` is at most its half-up rounding. -/
theorem isFloor_le_halfUp {n q v : Int} (hq : IsFloor n (10^27) q)
    (hv : IsHalfUp n (10^27) v) : q ≤ v := by
  unfold IsFloor at hq; rw [if_pos (by norm_num)] at hq
  unfold IsHalfUp at hv
  by_contra hc
  have h1 : (v + 1) * 10^27 ≤ q * 10^27 := by nlinarith
  nlinarith

/-- The ceiling of `n/RAY` is at most one above its half-up rounding. -/
theorem isCeil_le_halfUp_succ {n q v : Int} (hq : IsCeil n (10^27) q)
    (hv : IsHalfUp n (10^27) v) : q ≤ v + 1 := by
  unfold IsCeil at hq; rw [if_pos (by norm_num)] at hq
  unfold IsHalfUp at hv
  by_contra hc
  have h1 : (v + 2) * 10^27 ≤ q * 10^27 := by nlinarith
  nlinarith

/-- **A position inside a market total within the ceiling has a half-up value
that fits `i128`.** For `p ≤ x` and `index ≥ 0`. -/
theorem ValueWithin.fits_half_up {x i p : Int} (h : ValueWithin x i) (hi : 0 ≤ i)
    (hp : p ≤ x) : ∃ q, IsHalfUp (p * i) (10^27) q ∧ q < 2^127 := by
  obtain ⟨q, hq⟩ := IsHalfUp.exists (p * i) (10^27) (by norm_num)
  have hle := (h.mono (Int.mul_le_mul_of_nonneg_right hp hi)) q hq
  exact ⟨q, hq, by unfold MaxMarketValue at hle; omega⟩

/-- The same for the floored value. -/
theorem ValueWithin.fits_floor {x i p : Int} (h : ValueWithin x i) (hi : 0 ≤ i)
    (hp : p ≤ x) : ∃ q, IsFloor (p * i) (10^27) q ∧ q < 2^127 := by
  obtain ⟨v, hv, hlt⟩ := h.fits_half_up hi hp
  obtain ⟨q, hq⟩ := exists_isFloor (p * i) (10^27) (by norm_num)
  exact ⟨q, hq, by have := isFloor_le_halfUp hq hv; omega⟩

/-- The same for the ceiled value: the ceiling leaves one whole token of room
below `i128::MAX`, more than the one raw unit rounding up can add. -/
theorem ValueWithin.fits_ceil {x i p : Int} (h : ValueWithin x i) (hi : 0 ≤ i)
    (hp : p ≤ x) : ∃ q, IsCeil (p * i) (10^27) q ∧ q < 2^127 := by
  obtain ⟨v, hv⟩ := IsHalfUp.exists (p * i) (10^27) (by norm_num)
  have hle := (h.mono (Int.mul_le_mul_of_nonneg_right hp hi)) v hv
  obtain ⟨q, hq⟩ := isCeil_exists (p * i) (10^27) (by norm_num)
  exact ⟨q, hq, by
    have := isCeil_le_halfUp_succ hq hv
    unfold MaxMarketValue at hle; omega⟩

/-- **Within the ceiling, the total supply value fits `i128`.** This is the
fit hypothesis of `seize_apply_spec`. -/
theorem ValueCeil.supply_value_fits {c : market_cache.Cache} (hV : ValueCeil c) :
    ∃ V : Int, IsHalfUp (c.supplied.val * c.supply_index.val) (10^27) V ∧ V < 2^127 := by
  obtain ⟨V, hVv⟩ := IsHalfUp.exists (c.supplied.val * c.supply_index.val) (10^27) (by norm_num)
  have hle := hV.1 V hVv
  exact ⟨V, hVv, by unfold MaxMarketValue at hle; omega⟩

/-- **Within the ceiling, the revenue value fits.** Revenue shares are part of
total supply (INV-ACCT-01), so their value is below `2^127` RAY. This is the
fit hypothesis of `revenue_accounting_spec`. -/
theorem ValueCeil.revenue_fits {c : market_cache.Cache} (hV : ValueCeil c)
    (hsh : SharesInv c) (hsi : 0 ≤ c.supply_index.val) :
    c.revenue.val * c.supply_index.val < 2^127 * 10^27 := by
  unfold SharesInv at hsh
  obtain ⟨V, hVv⟩ := IsHalfUp.exists (c.supplied.val * c.supply_index.val) (10^27) (by norm_num)
  have hle := hV.1 V hVv
  have hrs : c.revenue.val * c.supply_index.val ≤ c.supplied.val * c.supply_index.val :=
    Int.mul_le_mul_of_nonneg_right hsh.2.1 hsi
  unfold IsHalfUp at hVv
  unfold MaxMarketValue at hle
  nlinarith

/-! ## Accrual -/

/-- **One accrual chunk keeps the market value ceiling (INV-IDX-06).** For a
cache in `AccrualDomain` within the ceiling, `accrue_chunk` returns a cache
that satisfies `ChunkPost` and is again within the ceiling: the debt value at
the new borrow index, and the supply value at the new supply index with the
chunk's revenue shares added, are both at most `MAX_MARKET_VALUE_RAY`. -/
theorem accrue_chunk_keeps_ceiling (env : Env) (c : market_cache.Cache) (k : Std.U64)
    (hD : AccrualDomain c) (hk : k.val ≤ 31556926000) (hV : ValueCeil c) :
    interest.accrue_chunk env c k ⦃ y => ChunkPost c y.2 ∧ ValueCeil y.2 ⦄ := by
  obtain ⟨hp, hinv, hbi0, hbiM, hsi0, hsiM⟩ := hD
  have hinv' := hinv
  unfold SharesInv at hinv'
  unfold interest.accrue_chunk
  simp only [market_cache.Cache.impl.params, market_cache.Cache.impl.borrowed,
    market_cache.Cache.impl.supplied, market_cache.Cache.impl.borrow_index,
    market_cache.Cache.impl.supply_index, bind_ok]
  step with accrue_step_spec env c.params c.borrowed c.supplied c.borrow_index c.supply_index k
    hp hinv'.2.2 (by omega) hbi0 hbiM hsi0 hsiM (by omega)
    as ⟨st, h1, h2, h3, h4, h5, h6, od, nd, t, t', hod, hnd, ht, ht', htt', hrev, hint⟩
  simp only [market_cache.report.Cache.set_borrow_index, market_cache.report.Cache.set_supply_index,
    bind_ok]
  unfold market_cache.shares.Cache.accrue_revenue
  have hrb := i128_bounds c.revenue
  step with ray_checked_add_spec c.env c.revenue st.revenue_shares (by simp; omega)
    (by simp; omega) as ⟨r, hr⟩
  step with ray_checked_add_spec c.env c.supplied st.revenue_shares (by simp; omega)
    (by simp; omega) as ⟨r1, hr1⟩
  have hodM := hV.2 od hod
  have htM := hV.1 t ht
  obtain ⟨t'', ht''⟩ := IsHalfUp.exists
    ((c.supplied.val + st.revenue_shares.val) * st.supply_index.val) (10^27) (by norm_num)
  have hboth := accrue_step_within_ceiling hodM htM ht' ht'' hrev hint
  refine ⟨⟨⟨by simp; omega, by simp; omega, by simp; omega⟩, by simpa using h1, by simpa using h2,
    by simpa using h3, by simpa using h4, st.revenue_shares.val, h5, by simpa using hr,
    by simpa using hr1, rfl⟩, fun v hv => ?_, fun v hv => ?_⟩
  · have hv' : IsHalfUp ((c.supplied.val + st.revenue_shares.val) * st.supply_index.val)
        (10^27) v := by simpa [hr1] using hv
    rw [IsHalfUp.unique (by norm_num) hv' ht'']; exact hboth.2
  · have hv' : IsHalfUp (c.borrowed.val * st.borrow_index.val) (10^27) v := by simpa using hv
    rw [IsHalfUp.unique (by norm_num) hv' hnd]; exact hboth.1

/-- **`global_sync` keeps the market value ceiling, for any elapsed time
(INV-IDX-06).** For a cache in `AccrualDomain` within the ceiling,
`global_sync` returns a cache within the ceiling, however many chunks it
accrues. -/
theorem global_sync_keeps_ceiling (env : Env) (c : market_cache.Cache) (hD : AccrualDomain c)
    (hV : ValueCeil c) :
    interest.global_sync env c ⦃ c' => ValueCeil c' ⦄ := by
  let Inv : Nat → market_cache.Cache → Prop :=
    fun _ x => ChunkPost c x ∧ AccrualDomain x ∧ ValueCeil x
  have hstep : ∀ (r : Nat) (k : Std.U64) (x : market_cache.Cache), Inv r x → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env x k ⦃ y => Inv (r - k.val) y.2 ⦄ := by
    rintro r k x ⟨hcx, hDx, hVx⟩ _ hk
    apply WP.spec_mono (accrue_chunk_keeps_ceiling env x k hDx (by omega) hVx)
    rintro y ⟨hy, hVy⟩
    exact ⟨hcx.trans hy, hDx.of_chunk hy, hVy⟩
  apply WP.spec_mono (global_sync_spec_of_inv env Inv hstep c ⟨ChunkPost.refl hD, hD, hV⟩)
  rintro c' (⟨_, rfl⟩ | ⟨_, x, ⟨_, _, hVx⟩, rfl⟩)
  · exact hV
  · exact hVx

/-- **`ops::synced_market` keeps the market value ceiling (INV-IDX-06).** If
the market loads to `c0` with `PoolInv c0`, a validated rate model and both
totals within the ceiling, the synced cache every leg starts from is within
the ceiling too, whatever the elapsed time. -/
theorem synced_market_keeps_ceiling (env : Env) (k : types.pool.HubAssetKey)
    (c0 : market_cache.Cache) (hload : market_cache.Cache.load env k = ok c0)
    (hinv : PoolInv c0) (hp : RatesProofs.ValidRateModel c0.params) (hV : ValueCeil c0) :
    ops.synced_market env k ⦃ c => ValueCeil c ⦄ := by
  unfold ops.synced_market
  rw [hload, bind_ok]
  exact global_sync_keeps_ceiling env c0 (AccrualDomain.of_poolInv hinv hp) hV

/-! ## Entries: the committed cache is within the ceiling outright -/

/-- **Supply leaves the market within the value ceiling (INV-ACCT-11).** -/
theorem supply_keeps_ceiling (env : Env) (entry : types.pool.PoolSupplyEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (hfit : ∃ q : Int, IsFloor (entry.action.amount.val * 10^(27 - c.params.asset_decimals.val)
      * 10^27) c.supply_index.val q ∧ q < 2^127)
    (r : types.pool.PoolPositionMutation × types.pool.MarketStateSnapshot)
    (h : ops.supply.apply env entry = ok r) :
    ∃ c' : market_cache.Cache, r.2 = snapOf c' ∧ PoolInv c' ∧ ValueCeil c' := by
  obtain ⟨_, _, m, c', _, _, _, _, _, _, _, _, hsnap, hinv', _, _, _, hceil⟩ :=
    supply_apply_spec env entry c hsync hinv hdec hA hfit r h
  exact ⟨c', hsnap, hinv', ValueCeil.of_entry_guard env c' hinv' hceil⟩

/-- **Borrow leaves the market within the value ceiling (INV-ACCT-11).** -/
theorem borrow_keeps_ceiling (env : Env) (entry : types.pool.PoolBorrowEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (r : ops.borrow.BorrowOutcome) (h : ops.borrow.accounting env entry = ok r) :
    ValueCeil r.cache := by
  obtain ⟨_, _, _, _, m, _, _, _, _, _, _, _, hinv', _, _, _, _, hceil⟩ :=
    borrow_accounting_spec env entry c hsync hinv hdec hA r h
  exact ValueCeil.of_entry_guard env r.cache hinv' hceil

/-- **A strategy opening leaves the market within the value ceiling
(INV-ACCT-11).** -/
theorem strategy_keeps_ceiling (env : Env) (action : types.pool.PoolAction) (charge : Bool)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hpar : ParamsOk c)
    (hA : action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (r : ops.strategy.StrategyOutcome) (h : ops.strategy.accounting env action charge = ok r) :
    ValueCeil r.cache := by
  obtain ⟨_, _, _, _, _, _, m, _, _, _, _, _, _, _, hinv', _, _, _, _, hceil⟩ :=
    strategy_accounting_spec env action charge c hsync hinv hpar hA r h
  exact ValueCeil.of_entry_guard env r.cache hinv' hceil

/-! ## Exits and recapitalization: no market value rises -/

/-- **Repay keeps the market value ceiling.** Debt shares only fall; supply
and both indexes are unchanged. -/
theorem repay_keeps_ceiling (env : Env) (action : types.pool.PoolAction)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsCeil (action.position.scaled_amount.val * c.borrow_index.val) (10^27) q ∧
      q < 2^127)
    (hV : ValueCeil c)
    (r : ops.repay.RepayOutcome) (h : ops.repay.accounting env action = ok r) :
    ValueCeil r.cache := by
  obtain ⟨_, _, _, _, _, _, hnle, _, _, _, _, _, _, _, hbor, _, hce⟩ :=
    repay_accounting_spec env action c hsync hinv hdec hfit r h
  have hbi := hinv.bi_lo
  refine hV.of_le ?_ ?_
  · rw [show r.cache.supplied = c.supplied by rw [hce],
      show r.cache.supply_index = c.supply_index by rw [hce]]
  · rw [show r.cache.borrow_index = c.borrow_index by rw [hce]]
    exact Int.mul_le_mul_of_nonneg_right (by omega) (by omega)

/-- **Withdraw keeps the market value ceiling, with or without a liquidation
fee.** The fee shares minted are at most the supply shares burned
(`withdraw_accounting_spec`), so total supply shares do not rise; debt and
both indexes are unchanged. -/
theorem withdraw_keeps_ceiling (env : Env) (isl : Bool) (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsHalfUp (entry.action.position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (hV : ValueCeil c)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    ValueCeil r.cache := by
  obtain ⟨_, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _,
    ⟨s, _, _, hsle, hsup, _⟩, _, hce⟩ :=
    withdraw_accounting_spec env isl entry c hsync hinv hdec hfit r h
  have hsi := hinv.si_lo
  refine hV.of_le ?_ ?_
  · rw [show r.cache.supply_index = c.supply_index by rw [hce]]
    exact Int.mul_le_mul_of_nonneg_right (by omega) (by omega)
  · rw [show r.cache.borrowed = c.borrowed by rw [hce],
      show r.cache.borrow_index = c.borrow_index by rw [hce]]

/-- **Net settlement keeps the market value ceiling.** Supply and debt shares
only fall; both indexes are unchanged. -/
theorem net_settle_keeps_ceiling (env : Env) (entry : types.pool.PoolNetSettleEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfitS : ∃ q : Int, IsFloor (entry.supply_position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (hfitD : ∃ q : Int, IsCeil (entry.debt_position.scaled_amount.val * c.borrow_index.val)
      (10^27) q ∧ q < 2^127)
    (hV : ValueCeil c)
    (r : types.pool.PoolNetSettleResult × types.pool.MarketStateSnapshot)
    (h : ops.net_settle.apply env entry = ok r) :
    ∃ c' : market_cache.Cache, r.2 = snapOf c' ∧ PoolInv c' ∧ ValueCeil c' := by
  obtain ⟨_, _, _, _, _, _, hns, _, hnd, _, _, _, _, c', hsnap, hinv', _, hsup, hbor, hce⟩ :=
    net_settle_apply_spec env entry c hsync hinv hdec hfitS hfitD r h
  have hsi := hinv.si_lo
  have hbi := hinv.bi_lo
  refine ⟨c', hsnap, hinv', hV.of_le ?_ ?_⟩
  · rw [show c'.supply_index = c.supply_index by rw [hce]]
    exact Int.mul_le_mul_of_nonneg_right (by omega) (by omega)
  · rw [show c'.borrow_index = c.borrow_index by rw [hce]]
    exact Int.mul_le_mul_of_nonneg_right (by omega) (by omega)

/-- **Seizure keeps the market value ceiling.** The deposit side only
reclassifies shares as revenue. The borrow side burns debt shares and writes
the supply index down, never up. The fit hypothesis of `seize_apply_spec` is
discharged from the ceiling. -/
theorem seize_keeps_ceiling (env : Env) (entry : types.pool.PoolSeizeEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.hub_asset = ok c)
    (hinv : PoolInv c) (hV : ValueCeil c)
    (r : types.pool.MarketStateSnapshot) (h : ops.seize.apply env entry = ok r) :
    ∃ c' : market_cache.Cache, r = snapOf c' ∧ PoolInv c' ∧ ValueCeil c' := by
  obtain ⟨hp0, c', hsnap, hinv', _, hdep, hbor⟩ :=
    seize_apply_spec env entry c hsync hinv hV.supply_value_fits r h
  have hsh := hinv.shares
  unfold SharesInv at hsh
  have hbi := hinv.bi_lo
  refine ⟨c', hsnap, hinv', ?_⟩
  cases hside : entry.side with
  | Deposit =>
    obtain ⟨_, hce⟩ := hdep hside
    refine hV.of_le ?_ ?_
    · rw [show c'.supplied = c.supplied by rw [hce],
        show c'.supply_index = c.supply_index by rw [hce]]
    · rw [show c'.borrowed = c.borrowed by rw [hce],
        show c'.borrow_index = c.borrow_index by rw [hce]]
  | Borrow =>
    obtain ⟨hb, hsile, hce⟩ := hbor hside
    refine hV.of_le ?_ ?_
    · rw [show c'.supplied = c.supplied by rw [hce]]
      exact Int.mul_le_mul_of_nonneg_left hsile (by omega)
    · rw [show c'.borrow_index = c.borrow_index by rw [hce]]
      exact Int.mul_le_mul_of_nonneg_right (by omega) (by omega)

/-- **Recapitalization keeps the market value ceiling.** Only cash changes. -/
theorem recapitalize_keeps_ceiling (env : Env) (k : types.pool.HubAssetKey) (amount : Std.I128)
    (c : market_cache.Cache) (hsync : ops.synced_market env k = ok c) (hinv : PoolInv c)
    (hV : ValueCeil c)
    (r : ops.recapitalize.RecapitalizationOutcome)
    (h : ops.recapitalize.accounting env k amount = ok r) :
    ValueCeil r.cache := by
  obtain ⟨_, _, _, _, _, _, hce, _⟩ := recapitalize_accounting_spec env k amount c hsync hinv r h
  refine hV.of_le ?_ ?_
  · rw [show r.cache.supplied = c.supplied by rw [hce],
      show r.cache.supply_index = c.supply_index by rw [hce]]
  · rw [show r.cache.borrowed = c.borrowed by rw [hce],
      show r.cache.borrow_index = c.borrow_index by rw [hce]]

/-- **A revenue claim keeps the market value ceiling.** Revenue and supply
shares only fall; debt and both indexes are unchanged. The fit hypothesis of
`revenue_accounting_spec` is discharged from the ceiling. -/
theorem revenue_keeps_ceiling (env : Env) (k : types.pool.HubAssetKey)
    (c : market_cache.Cache) (hsync : ops.synced_market env k = ok c) (hinv : PoolInv c)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hV : ValueCeil c)
    (r : ops.revenue.RevenueOutcome) (h : ops.revenue.accounting env k = ok r) :
    ValueCeil r.cache := by
  obtain ⟨T, c1, _, hcr, h0, _, _, _, hce, _, _⟩ :=
    revenue_accounting_spec env k c hsync hinv hdec
      (hV.revenue_fits hinv.shares (by have := hinv.si_lo; omega)) r h
  unfold ClaimResult at hcr
  obtain ⟨_, _, _, _, _, hz, hp⟩ := hcr
  have hsi := hinv.si_lo
  -- the claim leaves `c1`: `c` with revenue and supply shares burned
  have hc1 : c1.supplied.val ≤ c.supplied.val ∧ c1.supply_index = c.supply_index ∧
      c1.borrowed = c.borrowed ∧ c1.borrow_index = c.borrow_index := by
    rcases lt_or_eq_of_le h0 with hpos | he
    · obtain ⟨burned, hb0, _, _, hsup, hc1e, _⟩ := hp hpos
      exact ⟨by omega, by rw [hc1e], by rw [hc1e], by rw [hc1e]⟩
    · rw [hz he.symm]; exact ⟨le_refl _, rfl, rfl, rfl⟩
  obtain ⟨hsle, hsie, hbe, hbie⟩ := hc1
  refine hV.of_le ?_ ?_
  · rw [show r.cache.supplied = c1.supplied by rw [hce],
      show r.cache.supply_index = c1.supply_index by rw [hce], hsie]
    exact Int.mul_le_mul_of_nonneg_right hsle (by omega)
  · rw [show r.cache.borrowed = c1.borrowed by rw [hce],
      show r.cache.borrow_index = c1.borrow_index by rw [hce], hbe, hbie]

/-! ## Flash loan: the fee booking -/

/-- **A flash loan leaves the market within the value ceiling (INV-ACCT-11).**
Whenever `flash::finalize` returns `c'` on a cache `c` with `PoolInv c`: the
fee was booked (`cash += fee`), the stored-state invariant holds, and both
market totals are within the ceiling, with no ceiling hypothesis: the leg
checks the entry ceiling after it mints the fee shares. `finalize` runs only
after the token balance checks confirm principal and fee came back.

Hypotheses are those of `book_fee_spec`: `fee ≥ 0` (`terms_spec`),
`decimals < 27`, and `fee·10^k` and `cash + fee` fit `i128`. -/
theorem flash_finalize_keeps_ceiling (env : Env) (c : market_cache.Cache) (fee : Std.I128)
    (hinv : PoolInv c) (hfee : 0 ≤ fee.val) (hdec : c.params.asset_decimals.val < 27)
    (hA : fee.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (hcash : c.cash.val + fee.val < 2^127)
    (c' : market_cache.Cache) (h : ops.flash.finalize env c fee = ok c') :
    PoolInv c' ∧ ValueCeil c' ∧ c'.cash.val = c.cash.val + fee.val := by
  unfold ops.flash.finalize at h
  obtain ⟨c1, hb, h⟩ := bind_eq_ok h
  obtain ⟨u, hg, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok] at h
  simp only [events.emit_market_state, bind_ok, ok.injEq] at h
  subst h
  obtain ⟨hsh, hcashv, s, hs0, hr, hsu, hce⟩ :=
    spec_ok_eq (book_fee_spec c fee hinv.shares hinv.si_pos hfee hdec hA hcash) hb
  have hinv1 : PoolInv c1 :=
    hinv.of_eq hsh (by rw [hcashv]; have := hinv.cash; omega) (by rw [hce]) (by rw [hce])
  exact ⟨hinv1, ValueCeil.of_entry_guard env c1 hinv1 ⟨u, hg⟩, hcashv⟩

/-! ## Exits under "position ≤ market total"

The exit leg theorems assume that the value of the caller's position fits
`i128`. The pool keeps no per-account book, so it cannot know that a position
is part of its total; the controller maintains it (INV-ACCT-10). Given that
one fact and the ceiling, the fit hypotheses hold. -/

/-- **Withdraw, with "position ≤ total supply" in place of the fit
hypothesis.** -/
theorem withdraw_keeps_ceiling_of_position_le (env : Env) (isl : Bool)
    (entry : types.pool.PoolWithdrawEntry) (c : market_cache.Cache)
    (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hpos : entry.action.position.scaled_amount.val ≤ c.supplied.val) (hV : ValueCeil c)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    ValueCeil r.cache :=
  withdraw_keeps_ceiling env isl entry c hsync hinv hdec
    (hV.1.fits_half_up (by have := hinv.si_lo; omega) hpos) hV r h

/-- **Repay, with "position ≤ total debt" in place of the fit hypothesis.** -/
theorem repay_keeps_ceiling_of_position_le (env : Env) (action : types.pool.PoolAction)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hpos : action.position.scaled_amount.val ≤ c.borrowed.val) (hV : ValueCeil c)
    (r : ops.repay.RepayOutcome) (h : ops.repay.accounting env action = ok r) :
    ValueCeil r.cache :=
  repay_keeps_ceiling env action c hsync hinv hdec
    (hV.2.fits_ceil (by have := hinv.bi_lo; omega) hpos) hV r h

/-- **Net settlement, with "positions ≤ market totals" in place of the fit
hypotheses.** -/
theorem net_settle_keeps_ceiling_of_position_le (env : Env)
    (entry : types.pool.PoolNetSettleEntry) (c : market_cache.Cache)
    (hsync : ops.synced_market env entry.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hposS : entry.supply_position.scaled_amount.val ≤ c.supplied.val)
    (hposD : entry.debt_position.scaled_amount.val ≤ c.borrowed.val) (hV : ValueCeil c)
    (r : types.pool.PoolNetSettleResult × types.pool.MarketStateSnapshot)
    (h : ops.net_settle.apply env entry = ok r) :
    ∃ c' : market_cache.Cache, r.2 = snapOf c' ∧ PoolInv c' ∧ ValueCeil c' :=
  net_settle_keeps_ceiling env entry c hsync hinv hdec
    (hV.1.fits_floor (by have := hinv.si_lo; omega) hposS)
    (hV.2.fits_ceil (by have := hinv.bi_lo; omega) hposD) hV r h

end PoolAeneas.Proofs
