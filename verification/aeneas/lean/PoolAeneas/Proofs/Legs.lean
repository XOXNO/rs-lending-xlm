/-
The pool operation legs of `contracts/pool/src/ops/` (`supply`, `borrow`,
`withdraw`, `repay`, `net_settle`, `seize`, `recapitalize`, `strategy`,
`revenue`), and what they discharge of the Certora pool summaries
(`certora/shared/summaries/pool.rs`) and of the pool invariants
(INV-ACCT-01/02/03/05/06/09/10, docs/reference/invariants.md).

Shape of every leg theorem. A leg loads the market and accrues it
(`ops::synced_market` = `Cache::load` + `interest::global_sync`), mutates
the cache, and commits it. Storage and ledger time are uninterpreted
(`FunsExternal.lean`), so the theorems name the synced cache `c` by the
hypothesis `ops.synced_market env key = ok c`, assume the stored-state
invariant `PoolInv c` on it (proved inductive below: every leg's committed
cache satisfies `PoolInv` again, and `synced_market_spec` carries it
through accrual), and then describe **every successful return** of the leg
(`leg = ok r → …`): Certora summaries constrain the values a call returns,
and an aborting call reverts the whole transaction. The leg's own guards
(`require_*` asserts, checked arithmetic) are then facts we obtain, not
hypotheses.

Fit hypotheses (`…·10^k < 2^127`, "the floor fits") are the conditions under
which the conversion helpers are proved exact; they say the RAY-scaled
amounts are representable in `i128` (otherwise those helpers abort).
-/
import PoolAeneas.Proofs.Withdrawal
import PoolAeneas.Proofs.GlobalSync
import PoolAeneas.Proofs.Cash
import PoolAeneas.Proofs.Guards
import PoolAeneas.Proofs.Flash

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 1000000

namespace PoolAeneas.Proofs

/-! ## The stored-state invariant -/

/-- The invariant every stored market state satisfies (what
`get_sync_data_summary` and `nondet_market_index_raw` assume of the pool):
INV-ACCT-01 (`0 ≤ revenue ≤ supplied`, `0 ≤ borrowed`), INV-ACCT-02
(`cash ≥ 0`), the supply index in `[SUPPLY_INDEX_FLOOR_RAW, MAX_SUPPLY_INDEX_RAY]
= [10^24, 10^36]` and the borrow index in `[RAY, MAX_BORROW_INDEX_RAY] =
[10^27, 10^36]`. -/
structure PoolInv (c : market_cache.Cache) : Prop where
  shares : SharesInv c
  cash : 0 ≤ c.cash.val
  si_lo : 10^24 ≤ c.supply_index.val
  si_hi : c.supply_index.val ≤ 10^36
  bi_lo : 10^27 ≤ c.borrow_index.val
  bi_hi : c.borrow_index.val ≤ 10^36

/-- The market-parameter facts the legs use, established by
`MarketParamsRaw::verify` at market creation and rate-model replacement
(not extracted): `asset_decimals ≤ MAX_ASSET_DECIMALS = 18` and
`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`. No leg changes the params. -/
def ParamsOk (c : market_cache.Cache) : Prop :=
  c.params.asset_decimals.val ≤ 18 ∧ c.params.flashloan_fee.val ≤ 500

/-- The market index DTO a leg returns for cache `c`. -/
def indexOf (c : market_cache.Cache) : types.pool.MarketIndexRaw :=
  { borrow_index := c.borrow_index, supply_index := c.supply_index }

/-- The snapshot `Cache::commit` / `Cache::snapshot` return for `c` (the
state written to storage, plus key and timestamp). -/
def snapOf (c : market_cache.Cache) : types.pool.MarketStateSnapshot :=
  { hub_asset := c.hub_asset, timestamp := c.current_timestamp,
    supply_index := c.supply_index, borrow_index := c.borrow_index, cash := c.cash,
    supplied := c.supplied, borrowed := c.borrowed, revenue := c.revenue }

/-! ## Plumbing: load, commit, DTOs -/

/-- `require_nonneg_amount` returned: the amount was non-negative. -/
theorem nonneg_of_ok (env : Env) (a : Std.I128) (u : Unit)
    (h : validation.require_nonneg_amount env a = ok u) : 0 ≤ a.val :=
  (require_nonneg_amount_iff env a).1 ⟨u, h⟩

/-- `require_positive_amount` returned: the amount was positive. -/
theorem positive_of_ok (env : Env) (a : Std.I128) (u : Unit)
    (h : validation.require_positive_amount env a = ok u) : 0 < a.val := by
  unfold validation.require_positive_amount at h
  by_contra hc
  have : ¬ (a > 0#i128) := by simp [IScalar.lt_equiv]; omega
  simp [this, massert] at h

/-- `massert b` returned: `b` held. -/
theorem massert_ok {b : Prop} [Decidable b] {u : Unit} (h : massert b = ok u) : b := by
  by_contra hc; simp [massert, hc] at h

/-- `Cache::commit` never aborts (the storage write returns `()`), and
returns the snapshot of the committed cache. -/
theorem commit_eq (c : market_cache.Cache) : market_cache.Cache.commit c = ok (snapOf c) := by
  simp [market_cache.Cache.commit, market_cache.report.Cache.snapshot, math.fp.Ray.raw,
    storage.write_state, types.pool.HubAssetKey.Insts.CoreCloneClone.clone,
    aeneas_pool_shim.Address.Insts.CoreCloneClone.clone, core.clone.impls.CloneU32.clone,
    lift, snapOf]

/-- `Cache::market_index` is `indexOf`. -/
theorem market_index_eq (c : market_cache.Cache) :
    market_cache.report.Cache.market_index c = ok (indexOf c) := by
  simp [market_cache.report.Cache.market_index, math.fp.Ray.raw, indexOf]

/-- `Cache::position_mutation`. -/
theorem position_mutation_eq (c : market_cache.Cache) (s : math.fp.Ray) (a : Std.I128) :
    market_cache.report.Cache.position_mutation c s a =
      ok { position := { scaled_amount := s }, market_index := indexOf c, actual_amount := a,
           asset_decimals := c.params.asset_decimals } := by
  simp [market_cache.report.Cache.position_mutation, math.fp.Ray.raw, market_index_eq]

/-- `Cache::strategy_mutation`. -/
theorem strategy_mutation_eq (c : market_cache.Cache) (s : math.fp.Ray) (a b : Std.I128) :
    market_cache.report.Cache.strategy_mutation c s a b =
      ok { position := { scaled_amount := s }, market_index := indexOf c, actual_amount := a,
           amount_received := b, asset_decimals := c.params.asset_decimals } := by
  simp [market_cache.report.Cache.strategy_mutation, math.fp.Ray.raw, market_index_eq]

/-- `ops::renewed_market` returned `c'`: `ttl::renew_instance` (instance
TTL, not modelled) returned and the market synced to `c'`. -/
theorem renewed_market_ok (env : Env) (k : types.pool.HubAssetKey) (c' : market_cache.Cache)
    (h : ops.renewed_market env k = ok c') : ops.synced_market env k = ok c' := by
  unfold ops.renewed_market at h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  exact h

/-- `ops::load_leg` returned `(c', p)`: the action's amount and scaled
position were non-negative, the market synced to `c'`, and `p` is the
action's scaled position. -/
theorem load_leg_ok (env : Env) (a : types.pool.PoolAction) (c' : market_cache.Cache)
    (p : math.fp.Ray) (h : ops.load_leg env a = ok (c', p)) :
    0 ≤ a.amount.val ∧ 0 ≤ a.position.scaled_amount.val ∧
      ops.synced_market env a.hub_asset = ok c' ∧ p = a.position.scaled_amount := by
  unfold ops.load_leg at h
  obtain ⟨u1, h1, h⟩ := bind_eq_ok h
  obtain ⟨u2, h2, h⟩ := bind_eq_ok h
  obtain ⟨c0, h3, h⟩ := bind_eq_ok h
  rw [ray_from_same] at h
  simp only [bind_ok, ok.injEq, Prod.mk.injEq] at h
  obtain ⟨rfl, rfl⟩ := h
  exact ⟨nonneg_of_ok _ _ _ h1, nonneg_of_ok _ _ _ h2, h3, rfl⟩

/-- `PoolInv` gives a positive supply index. -/
theorem PoolInv.si_pos {c : market_cache.Cache} (h : PoolInv c) : 0 < c.supply_index.val := by
  have := h.si_lo; omega

/-- `PoolInv` gives a positive borrow index. -/
theorem PoolInv.bi_pos {c : market_cache.Cache} (h : PoolInv c) : 0 < c.borrow_index.val := by
  have := h.bi_lo; omega

/-- `PoolInv` is a predicate on the share totals, cash and indexes only. -/
theorem PoolInv.of_eq {c c' : market_cache.Cache} (h : PoolInv c)
    (hs : SharesInv c') (hcash : 0 ≤ c'.cash.val)
    (hsi : c'.supply_index = c.supply_index) (hbi : c'.borrow_index = c.borrow_index) :
    PoolInv c' :=
  ⟨hs, hcash, hsi ▸ h.si_lo, hsi ▸ h.si_hi, hbi ▸ h.bi_lo, hbi ▸ h.bi_hi⟩

/-! ## `supply` -/

/-- **`ops::supply::apply` (the supply leg).** Whenever it returns
`(mutation, snapshot)` on a market that synced to `c` with `PoolInv c`:

* `amount ≥ 0` and the incoming position is `≥ 0` (validated);
* it minted `minted = ⌊amount·10^k·RAY/supply_index⌋ ≥ 0` shares (floor,
  ADR-0003: `minted·supply_index ≤ amount·10^k·RAY`), positive when
  `amount > 0` (`SupplyRoundsToZeroShares`, INV-ACCT-05) and zero when
  `amount = 0`;
* the returned position is `position + minted`, `actual_amount = amount`,
  the returned indexes and decimals are the synced market's;
* the committed cache `c'` (`snapshot = snapOf c'`) is `c` with
  `supplied += minted` and `cash += amount` only: `revenue`, `borrowed`
  and the indexes are unchanged (INV-ACCT-10: the account's shares and the
  pool's non-revenue supply move by the same `minted`; INV-ACCT-02/03: cash
  is credited exactly the amount the controller reports as received), and
  `PoolInv c'` holds;
* the committed cache passes the entry-ceiling guard (INV-ACCT-11; its exact
  condition is `require_within_entry_ceiling_spec`).

Hypotheses: `PoolInv c`, `decimals ≤ 18`; `amount·10^k` and its share
floor fit `i128` (otherwise the conversion aborts). -/
theorem supply_apply_spec (env : Env) (entry : types.pool.PoolSupplyEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (hfit : ∃ q : Int, IsFloor (entry.action.amount.val * 10^(27 - c.params.asset_decimals.val)
      * 10^27) c.supply_index.val q ∧ q < 2^127)
    (r : types.pool.PoolPositionMutation × types.pool.MarketStateSnapshot)
    (h : ops.supply.apply env entry = ok r) :
    0 ≤ entry.action.amount.val ∧ 0 ≤ entry.action.position.scaled_amount.val ∧
    ∃ (minted : Int) (c' : market_cache.Cache),
      IsFloor (entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
        c.supply_index.val minted ∧
      0 ≤ minted ∧ (0 < entry.action.amount.val → 0 < minted) ∧
      (entry.action.amount.val = 0 → minted = 0) ∧
      r.1.position.scaled_amount.val = entry.action.position.scaled_amount.val + minted ∧
      r.1.actual_amount = entry.action.amount ∧
      r.1.market_index = indexOf c ∧ r.1.asset_decimals = c.params.asset_decimals ∧
      r.2 = snapOf c' ∧ PoolInv c' ∧
      c'.supplied.val = c.supplied.val + minted ∧
      c'.cash.val = c.cash.val + entry.action.amount.val ∧
      c' = { c with supplied := c'.supplied, cash := c'.cash } ∧
      (∃ u, guards.require_within_entry_ceiling env c' = ok u) := by
  unfold ops.supply.apply at h
  obtain ⟨⟨c0, p0⟩, hl, h⟩ := bind_eq_ok h
  obtain ⟨hamt, hpos, hs0, rfl⟩ := load_leg_ok env _ _ _ hl
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨m, hm, h⟩ := bind_eq_ok h
  obtain ⟨_, hif, h⟩ := bind_eq_ok h
  obtain ⟨p1, hp1, h⟩ := bind_eq_ok h
  obtain ⟨c1, hc1, h⟩ := bind_eq_ok h
  obtain ⟨uc, hceil, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok, position_mutation_eq, bind_ok, ok.injEq] at h
  subst h
  -- the minted shares
  have hdec27 : c.params.asset_decimals.val < 27 := by omega
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  obtain ⟨q, hq, hqlt⟩ := hfit
  have hnum : 0 ≤ entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27 := by
    positivity
  have hq0 : 0 ≤ q := IsFloor.nonneg hinv.si_pos hnum hq
  unfold market_cache.scale.Cache.calculate_scaled_supply at hm
  have hmv := spec_ok_eq (calculate_scaled_supply_spec c.env entry.action.amount
    c.params.asset_decimals c.supply_index hdec27 hinv.si_pos ⟨by omega, hA⟩
    ⟨q, hq, by omega, hqlt⟩) hm
  have hmq : m.val = q := IsFloor.unique (by have := hinv.si_pos; omega) hmv hq
  have hm0 : 0 ≤ m.val := by omega
  have hmpos : 0 < entry.action.amount.val → 0 < m.val := by
    intro hpos'
    split at hif
    · rename_i he; simp [he] at hpos'
    · obtain ⟨_, hr, hif⟩ := bind_eq_ok hif
      simp only [math.fp.Ray.raw, ok.injEq] at hr; subst hr
      have := massert_ok hif
      simpa [IScalar.lt_equiv] using this
  have hmz : entry.action.amount.val = 0 → m.val = 0 := by
    intro h0
    rw [h0] at hmv
    simp only [zero_mul] at hmv
    exact IsFloor.unique (by have := hinv.si_pos; omega) hmv
      (by unfold IsFloor; rw [if_pos hinv.si_pos]; constructor <;> nlinarith [hinv.si_pos])
  have hp1v := ray_checked_add_eq_ok _ _ _ _ hp1
  obtain ⟨hsh1, hsup1, hc1e⟩ := mint_supply_preserves c c1 m hinv.shares hm0 hc1
  obtain ⟨hcash2, hc2e⟩ := (credit_cash_spec c1 _).2 c2 hc2
  have hcash2' : c2.cash.val = c.cash.val + entry.action.amount.val := by
    rw [hcash2, hc1e]
  have hc2full : c2 = { c with supplied := c2.supplied, cash := c2.cash } := by
    rw [hc2e, hc1e]
  have hpar : c2.params = c.params := by rw [hc2e, hc1e]
  have hidx : indexOf c2 = indexOf c := by unfold indexOf; rw [hc2e, hc1e]
  have hceil2 : guards.require_within_entry_ceiling env c2 = ok uc := by
    rw [require_within_entry_ceiling_congr env c1 c2 (by rw [hc2e]) (by rw [hc2e]) (by rw [hc2e])
      (by rw [hc2e])]
    exact hceil
  refine ⟨hamt, hpos, m.val, c2, hmv, hm0, hmpos, hmz, hp1v, rfl, hidx, by simp [hpar], rfl,
    ?_, ?_, hcash2', hc2full, ⟨uc, hceil2⟩⟩
  · have hs2 : SharesInv c2 := by rw [hc2e]; exact hsh1
    refine hinv.of_eq hs2 (by rw [hcash2']; have := hinv.cash; omega) ?_ ?_ <;>
      rw [hc2full]
  · rw [hc2e]; simpa using hsup1

/-! ## `borrow` (and the debt mint shared with `strategy`) -/

/-- A debt-share ceiling at a borrow index `≥ RAY` is at most the RAY amount:
`⌈A·RAY/bi⌉ ≤ A` for `A ≥ 0`, so it fits whenever `A` does. -/
theorem ceil_le_of_index_ge_ray {A bi q : Int} (hA : 0 ≤ A) (hbi : 10^27 ≤ bi)
    (hq : IsCeil (A * 10^27) bi q) : 0 ≤ q ∧ q ≤ A := by
  unfold IsCeil at hq; rw [if_pos (by omega)] at hq
  constructor
  · nlinarith
  · by_contra hc
    have : (A + 1) * bi ≤ q * bi := mul_le_mul_of_nonneg_right (by omega) (by omega)
    nlinarith

/-- **`ops::borrow::mint_debt`** (shared by `borrow` and `strategy`).
Whenever it returns `(c1, p1)`: `amount > 0` (`require_positive_amount`),
`amount ≤ cash` (`require_reserves`), the liquidation-buffer guard passed on
`c` for the full `amount` (INV-ACCT-07; its exact condition is
`require_liquidation_buffer_spec`), it minted `minted = ⌈amount·10^k·RAY/
borrow_index⌉ > 0` debt shares (ceiling, ADR-0003: the recorded debt is
never below the borrowed value; `BorrowRoundsToZeroShares`, INV-ACCT-05),
`p1 = position + minted`, `c1` is `c` with `borrowed += minted` only, and
the utilization guard passed on `c1` (INV-ACCT-08,
`require_utilization_below_max_spec`: at every ceiling, 100% included, so the
debt value after the mint is at most the supply value), as did the
entry-ceiling guard (INV-ACCT-11, `require_within_entry_ceiling_spec`).
Hypotheses: `PoolInv c`,
`decimals ≤ 18`, `amount·10^k < 2^127`. -/
theorem mint_debt_ok (env : Env) (c : market_cache.Cache) (pos amount : Std.I128)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (c1 : market_cache.Cache) (p1 : math.fp.Ray)
    (h : ops.borrow.mint_debt env c pos amount = ok (c1, p1)) :
    0 < amount.val ∧ amount.val ≤ c.cash.val ∧
    (∃ u, guards.require_liquidation_buffer env c amount = ok u) ∧
    ∃ minted : Int,
      IsCeil (amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
        c.borrow_index.val minted ∧
      0 < minted ∧ minted ≤ amount.val * 10^(27 - c.params.asset_decimals.val) ∧
      p1.val = pos.val + minted ∧
      SharesInv c1 ∧ c1.borrowed.val = c.borrowed.val + minted ∧
      c1 = { c with borrowed := c1.borrowed } ∧
      (∃ u, guards.require_utilization_below_max env c1 = ok u) ∧
      (∃ u, guards.require_within_entry_ceiling env c1 = ok u) := by
  unfold ops.borrow.mint_debt at h
  obtain ⟨u1, h1, h⟩ := bind_eq_ok h
  obtain ⟨u2, h2, h⟩ := bind_eq_ok h
  obtain ⟨u3, h3, h⟩ := bind_eq_ok h
  obtain ⟨m, hm, h⟩ := bind_eq_ok h
  obtain ⟨i, hi, h⟩ := bind_eq_ok h
  obtain ⟨u4, h4, h⟩ := bind_eq_ok h
  obtain ⟨q1, hq1, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  obtain ⟨u6, h6, h⟩ := bind_eq_ok h
  obtain ⟨u5, h5, h⟩ := bind_eq_ok h
  simp only [ok.injEq, Prod.mk.injEq] at h
  obtain ⟨rfl, rfl⟩ := h
  have hpos := positive_of_ok _ _ _ h1
  have hres := (require_reserves_iff c amount).1 ⟨u2, h2⟩
  simp only [math.fp.Ray.raw, ok.injEq] at hi; subst hi
  have hmpos : 0 < m.val := by simpa [IScalar.lt_equiv] using massert_ok h4
  have hdec27 : c.params.asset_decimals.val < 27 := by omega
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  have hA0 : 0 ≤ amount.val * 10^(27 - c.params.asset_decimals.val) := by positivity
  obtain ⟨q, hq⟩ := isCeil_exists (amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
    c.borrow_index.val hinv.bi_pos
  have hqb := ceil_le_of_index_ge_ray hA0 hinv.bi_lo hq
  unfold market_cache.scale.Cache.calculate_scaled_borrow at hm
  have hmv := spec_ok_eq (calculate_scaled_borrow_spec c.env amount c.params.asset_decimals
    c.borrow_index hdec27 (by have := hinv.bi_pos; omega) ⟨by omega, hA⟩
    ⟨q, hq, by omega, by omega⟩) hm
  have hmq : m.val = q := IsCeil.unique (by have := hinv.bi_pos; omega) hmv hq
  obtain ⟨hsh, hbor, hce⟩ := mint_debt_preserves c c2 m hinv.shares (by omega) hc2
  exact ⟨hpos, hres, ⟨u3, h3⟩, m.val, hmv, hmpos, by omega, ray_checked_add_eq_ok _ _ _ _ hq1,
    hsh, hbor, hce, ⟨u5, h5⟩, ⟨u6, h6⟩⟩

/-- **`ops::borrow::accounting` (the borrow leg, before the token transfer).**
Whenever it returns `r` on a market synced to `c` with `PoolInv c`:
`amount > 0`, the incoming position `≥ 0`, `amount ≤ cash`; it minted
`minted = ⌈amount·10^k·RAY/borrow_index⌉ > 0` debt shares; the returned
position is `position + minted` (so strictly larger, as `borrow_summary`
assumes), `actual_amount = amount`, the returned indexes and decimals are
the synced market's; the committed cache `r.cache` (`r.snapshot = snapOf
r.cache`) is `c` with `borrowed += minted` and `cash -= amount` only
(INV-ACCT-10: account debt and pool debt move by the same `minted`;
INV-ACCT-02: cash debited exactly the amount `apply` transfers out), and
`PoolInv r.cache` holds. The liquidation-buffer and utilization guards
passed (INV-ACCT-07/08), and the committed cache passes the entry-ceiling
guard (INV-ACCT-11). Hypotheses: `PoolInv c`, `decimals ≤ 18`,
`amount·10^k < 2^127`. -/
theorem borrow_accounting_spec (env : Env) (entry : types.pool.PoolBorrowEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hA : entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (r : ops.borrow.BorrowOutcome) (h : ops.borrow.accounting env entry = ok r) :
    0 < entry.action.amount.val ∧ 0 ≤ entry.action.position.scaled_amount.val ∧
    entry.action.amount.val ≤ c.cash.val ∧
    (∃ u, guards.require_liquidation_buffer env c entry.action.amount = ok u) ∧
    ∃ minted : Int,
      IsCeil (entry.action.amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
        c.borrow_index.val minted ∧ 0 < minted ∧
      r.mutation.position.scaled_amount.val = entry.action.position.scaled_amount.val + minted ∧
      r.mutation.actual_amount = entry.action.amount ∧
      r.mutation.market_index = indexOf c ∧ r.mutation.asset_decimals = c.params.asset_decimals ∧
      r.snapshot = snapOf r.cache ∧ PoolInv r.cache ∧
      r.cache.borrowed.val = c.borrowed.val + minted ∧
      r.cache.cash.val = c.cash.val - entry.action.amount.val ∧
      r.cache = { c with borrowed := r.cache.borrowed, cash := r.cache.cash } ∧
      (∃ u, guards.require_utilization_below_max env { c with borrowed := r.cache.borrowed }
        = ok u) ∧
      (∃ u, guards.require_within_entry_ceiling env r.cache = ok u) := by
  unfold ops.borrow.accounting at h
  obtain ⟨⟨c0, p0⟩, hl, h⟩ := bind_eq_ok h
  obtain ⟨hamt, hpos, hs0, rfl⟩ := load_leg_ok env _ _ _ hl
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨⟨c1, p1⟩, hmd, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok, position_mutation_eq, bind_ok, ok.injEq] at h
  subst h
  obtain ⟨hpos', hres, hbuf, minted, hmc, hm0, _, hp1, hsh1, hbor1, hc1e, hutil, ⟨uc, hceil⟩⟩ :=
    mint_debt_ok env c _ _ hinv hdec hA c1 p1 hmd
  obtain ⟨hcash2, _, hc2e⟩ := (debit_cash_spec c1 _).2 c2 hc2
  have hpar : c2.params = c.params := by rw [hc2e, hc1e]
  have hidx : indexOf c2 = indexOf c := by unfold indexOf; rw [hc2e, hc1e]
  have hbor2 : c2.borrowed = c1.borrowed := by rw [hc2e]
  have hc2full : c2 = { c with borrowed := c2.borrowed, cash := c2.cash } := by
    rw [hbor2, hc2e, hc1e]
  have hceil2 : guards.require_within_entry_ceiling env c2 = ok uc := by
    rw [require_within_entry_ceiling_congr env c1 c2 (by rw [hc2e]) (by rw [hc2e]) (by rw [hc2e])
      (by rw [hc2e])]
    exact hceil
  refine ⟨hpos', hpos, hres, hbuf, minted, hmc, hm0, hp1, rfl, hidx, by simp [hpar], rfl,
    ?_, by rw [hbor2]; exact hbor1, by rw [hcash2, hc1e], hc2full,
    by rw [hbor2, ← hc1e]; exact hutil, ⟨uc, hceil2⟩⟩
  have hs2 : SharesInv c2 := by rw [hc2e]; exact hsh1
  refine hinv.of_eq hs2 (by rw [hcash2, hc1e]; simp; omega) ?_ ?_ <;> rw [hc2full]

/-! ## `repay` -/

/-- `a.checked_sub(b).unwrap_or_else(f)` with an aborting `f` returned `n`:
`n = a - b` exactly. -/
theorem sub_unwrap_eq_ok {C : Type} (inst : core.ops.function.FnOnce C Unit Std.I128)
    (cl : C) (hf : ∀ v, inst.call_once cl () ≠ ok v) (a b n : Std.I128) (o : Option Std.I128)
    (ho : lift (I128.checked_sub a b) = ok o)
    (h : core.option.Option.unwrap_or_else inst o cl = ok n) : n.val = a.val - b.val := by
  simp only [lift, ok.injEq] at ho; subst ho
  have hs := I128.checked_sub_bv_spec a b
  rw [show I128.checked_sub a b = core.num.checked_sub_IScalar a b from rfl] at h
  cases hc : core.num.checked_sub_IScalar a b with
  | none => rw [hc] at h; exact absurd h (hf n)
  | some z =>
    rw [hc] at hs h
    simp only [core.option.Option.unwrap_or_else, ok.injEq] at h
    rw [← h]; exact hs.2.2.1

/-- **`ops::repay::accounting` (the repay leg, before the refund transfer).**
Whenever it returns `r` on a market synced to `c` with `PoolInv c`, with
`D = ⌈position·borrow_index/(RAY·10^k)⌉` the position's ceiling debt and
`net = r.mutation.actual_amount`, `new = r.mutation.position`:

* `amount ≥ 0`, `position ≥ 0` (validated); `0 ≤ net ≤ amount`,
  `overpayment = amount − net` (INV-ACCT-03: the unused part of the receipt
  is what `apply` refunds), `0 ≤ new ≤ position`;
* full repayment (`D ≤ amount`): `new = 0`, `net = D`;
* partial (`amount < D`): `net = amount`, the burn is
  `⌊amount·10^k·RAY/borrow_index⌋` (floor, ADR-0003: never clears more debt
  than paid), and it is positive when `amount > 0`
  (`RepayRoundsToZeroShares`, INV-ACCT-05);
* `repay_summary`'s position clause: `amount > 0 ∧ position > 0 → new <
  position`, otherwise `new = position`;
* the committed cache is `c` with `borrowed -= (position − new)` and
  `cash += net` only (INV-ACCT-10, INV-ACCT-02), `PoolInv` holds; indexes
  and decimals returned are the synced market's.

Hypotheses: `PoolInv c`, `decimals ≤ 18`, and `⌈position·borrow_index/RAY⌉`
fits `i128` (the position's RAY debt value; otherwise `unscale_borrow_ceil`
aborts). -/
theorem repay_accounting_spec (env : Env) (action : types.pool.PoolAction)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsCeil (action.position.scaled_amount.val * c.borrow_index.val) (10^27) q ∧
      q < 2^127)
    (r : ops.repay.RepayOutcome) (h : ops.repay.accounting env action = ok r) :
    0 ≤ action.amount.val ∧ 0 ≤ action.position.scaled_amount.val ∧
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ action.amount.val ∧
    r.overpayment.val = action.amount.val - r.mutation.actual_amount.val ∧
    0 ≤ r.mutation.position.scaled_amount.val ∧
    r.mutation.position.scaled_amount.val ≤ action.position.scaled_amount.val ∧
    (∀ D : Int, IsCeil (action.position.scaled_amount.val * c.borrow_index.val)
        (10^27 * 10^(27 - c.params.asset_decimals.val)) D →
      (D ≤ action.amount.val → r.mutation.position.scaled_amount.val = 0 ∧
        r.mutation.actual_amount.val = D) ∧
      (action.amount.val < D → r.mutation.actual_amount = action.amount ∧
        IsFloor (action.amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
          c.borrow_index.val
          (action.position.scaled_amount.val - r.mutation.position.scaled_amount.val))) ∧
    (0 < action.amount.val → 0 < action.position.scaled_amount.val →
      r.mutation.position.scaled_amount.val < action.position.scaled_amount.val) ∧
    (¬ (0 < action.amount.val ∧ 0 < action.position.scaled_amount.val) →
      r.mutation.position.scaled_amount = action.position.scaled_amount) ∧
    r.mutation.market_index = indexOf c ∧ r.mutation.asset_decimals = c.params.asset_decimals ∧
    r.snapshot = snapOf r.cache ∧ PoolInv r.cache ∧
    r.cache.borrowed.val = c.borrowed.val -
      (action.position.scaled_amount.val - r.mutation.position.scaled_amount.val) ∧
    r.cache.cash.val = c.cash.val + r.mutation.actual_amount.val ∧
    r.cache = { c with borrowed := r.cache.borrowed, cash := r.cache.cash } := by
  unfold ops.repay.accounting at h
  obtain ⟨⟨c0, p0⟩, hl, h⟩ := bind_eq_ok h
  obtain ⟨hamt, hpos, hs0, rfl⟩ := load_leg_ok env _ _ _ hl
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨⟨bu, ov⟩, hrr, h⟩ := bind_eq_ok h
  obtain ⟨o, ho, h⟩ := bind_eq_ok h
  obtain ⟨net, hnet, h⟩ := bind_eq_ok h
  obtain ⟨_, hif, h⟩ := bind_eq_ok h
  obtain ⟨p1, hp1, h⟩ := bind_eq_ok h
  obtain ⟨c1, hc1, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok, position_mutation_eq, bind_ok, ok.injEq] at h
  subst h
  have hnetv := sub_unwrap_eq_ok _ _ (by intro v; simp [
    ops.repay.accounting.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]) _ _ _ _ ho hnet
  have hdec27 : c.params.asset_decimals.val < 27 := by omega
  obtain ⟨q, hq, hqlt⟩ := hfit
  have hpi : 0 ≤ action.position.scaled_amount.val * c.borrow_index.val := by
    have := hinv.bi_pos; positivity
  have hq0 : 0 ≤ q := by
    unfold IsCeil at hq; rw [if_pos (by norm_num)] at hq; nlinarith
  unfold market_cache.scale.Cache.resolve_repay at hrr
  have hspec := spec_ok_eq (resolve_repay_spec c.env action.amount action.position.scaled_amount
    c.borrow_index c.params.asset_decimals hdec27 hinv.bi_pos hpos hamt ⟨q, hq, by omega, hqlt⟩) hrr
  simp only at hspec
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  obtain ⟨D, hD⟩ := isCeil_exists (action.position.scaled_amount.val * c.borrow_index.val)
    (10^27 * 10^(27 - c.params.asset_decimals.val)) (by positivity)
  have hD0 : 0 ≤ D := by
    have hDc := hD; unfold IsCeil at hDc; rw [if_pos (by positivity)] at hDc; nlinarith
  obtain ⟨hfull, hpart⟩ := hspec D hD
  have hp1v := (ray_checked_sub_eq_ok _ _ _ _ hp1).2.2.2
  obtain ⟨hsh1, _, _, hbor1, hc1e⟩ := burn_debt_preserves c c1 bu hinv.shares hc1
  obtain ⟨hcash2, hc2e⟩ := (credit_cash_spec c1 _).2 c2 hc2
  have hnet0 : 0 ≤ net.val := ((credit_cash_spec c1 _).1.mp ⟨c2, hc2⟩).1
  -- burn positive when the net repay is positive
  have hbpos : net.val ≠ 0 → 0 < bu.val := by
    intro hn
    have hn' : ¬ (net = 0#i128) := fun he => hn (by simp [he])
    simp only [hn', ↓reduceIte] at hif
    obtain ⟨_, hr, hif⟩ := bind_eq_ok hif
    simp only [math.fp.Ray.raw, ok.injEq] at hr; subst hr
    simpa [IScalar.lt_equiv] using massert_ok hif
  have hpar : c2.params = c.params := by rw [hc2e, hc1e]
  have hidx : indexOf c2 = indexOf c := by unfold indexOf; rw [hc2e, hc1e]
  have hbor2 : c2.borrowed = c1.borrowed := by rw [hc2e]
  have hc2full : c2 = { c with borrowed := c2.borrowed, cash := c2.cash } := by
    rw [hbor2, hc2e, hc1e]
  have hs2 : SharesInv c2 := by rw [hc2e]; exact hsh1
  -- the two branches
  have key : (D ≤ action.amount.val → bu = action.position.scaled_amount ∧ net.val = D) ∧
      (action.amount.val < D → net.val = action.amount.val ∧
        IsFloor (action.amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
          c.borrow_index.val bu.val ∧ 0 ≤ bu.val ∧ bu.val < action.position.scaled_amount.val) := by
    constructor
    · intro hle; obtain ⟨hb, hov, _⟩ := hfull hle; exact ⟨hb, by omega⟩
    · intro hlt; obtain ⟨hov, hfl, hb0, hblt⟩ := hpart hlt; exact ⟨by omega, hfl, hb0, hblt⟩
  have hbu : 0 ≤ bu.val ∧ bu.val ≤ action.position.scaled_amount.val := by
    by_cases hle : D ≤ action.amount.val
    · rw [(key.1 hle).1]; omega
    · have := key.2 (by omega); omega
  have hnetle : net.val ≤ action.amount.val := by
    by_cases hle : D ≤ action.amount.val
    · have := (key.1 hle).2; omega
    · have := (key.2 (by omega)).1; omega
  refine ⟨hamt, hpos, hnet0, hnetle, ?_, by rw [hp1v]; omega, by rw [hp1v]; omega, ?_, ?_, ?_,
    hidx, by simp [hpar], rfl, ?_, ?_, by rw [hcash2, hc1e], hc2full⟩
  · -- overpayment
    have hov : ov.val = action.amount.val - net.val := by omega
    exact hov
  · intro D' hD'
    have := IsCeil.unique (by positivity) hD' hD; subst this
    refine ⟨fun hle => ⟨by rw [hp1v, (key.1 hle).1]; omega, (key.1 hle).2⟩, fun hlt => ?_⟩
    obtain ⟨hn, hfl, _, _⟩ := key.2 hlt
    refine ⟨by apply IScalar.eq_of_val_eq; exact hn, ?_⟩
    rw [hp1v]; simpa using hfl
  · intro ha hp
    by_cases hle : D ≤ action.amount.val
    · rw [hp1v, (key.1 hle).1]; omega
    · have := key.2 (by omega)
      have := hbpos (by omega); rw [hp1v]; omega
  · intro hn
    apply IScalar.eq_of_val_eq
    rw [hp1v]
    by_cases hp0 : action.position.scaled_amount.val = 0
    · omega
    · have ha0 : action.amount.val = 0 := by omega
      have hDpos : 0 < D := by
        have hDc := hD; unfold IsCeil at hDc; rw [if_pos (by positivity)] at hDc
        have : 0 < action.position.scaled_amount.val * c.borrow_index.val := by
          have := hinv.bi_pos; positivity
        nlinarith
      obtain ⟨_, hfl, _, _⟩ := key.2 (by omega)
      rw [ha0] at hfl; simp only [zero_mul] at hfl
      have : bu.val = 0 := IsFloor.unique (by have := hinv.bi_pos; omega) hfl
        (by unfold IsFloor; rw [if_pos hinv.bi_pos]; constructor <;> nlinarith [hinv.bi_pos])
      omega
  · exact hinv.of_eq hs2 (by rw [hcash2, hc1e]; simp; have := hinv.cash; omega)
      (by rw [hc2full]) (by rw [hc2full])
  · rw [hbor2, hbor1, hp1v]; ring

/-! ## `withdraw` -/

/-- **`withdraw::withhold_liquidation_fee`.** Whenever it returns
`(net, c')`: outside a fee-bearing liquidation (`¬ is_liquidation` or
`protocol_fee = 0`) `net = gross` and the cache is unchanged; otherwise
`protocol_fee ≤ gross` (`WithdrawLessThanFee`), `net = gross −
protocol_fee`, and the fee is booked as `s ≥ 0` revenue shares added to both
`revenue` and `supplied` (`add_protocol_revenue`), nothing else changes.
Hypotheses: `SharesInv c`, positive supply index, `fee ≥ 0`,
`decimals < 27`, `gross·10^k < 2^127` (so the fee, at most the gross, fits
`Ray::from_asset`). -/
theorem withhold_liquidation_fee_ok (env : Env) (c : market_cache.Cache) (g fee : Std.I128)
    (isl : Bool) (hsh : SharesInv c) (hsi : 0 < c.supply_index.val) (hfee : 0 ≤ fee.val)
    (hdec : c.params.asset_decimals.val < 27)
    (hG : g.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (net : Std.I128) (c' : market_cache.Cache)
    (h : ops.withdraw.withhold_liquidation_fee env c g isl fee = ok (net, c')) :
    (¬ (isl = true ∧ fee.val ≠ 0) → net = g ∧ c' = c) ∧
    (isl = true ∧ fee.val ≠ 0 → fee.val ≤ g.val ∧ net.val = g.val - fee.val) ∧
    SharesInv c' ∧ ∃ s : Int, 0 ≤ s ∧ (¬ (isl = true ∧ fee.val ≠ 0) → s = 0) ∧
      s * c.supply_index.val ≤ fee.val * 10^(27 - c.params.asset_decimals.val) * 10^27 ∧
      c'.revenue.val = c.revenue.val + s ∧ c'.supplied.val = c.supplied.val + s ∧
      c' = { c with revenue := c'.revenue, supplied := c'.supplied } := by
  have hzb : (0 : Int) * c.supply_index.val ≤
      fee.val * 10^(27 - c.params.asset_decimals.val) * 10^27 := by
    rw [zero_mul]; exact mul_nonneg (mul_nonneg hfee (by positivity)) (by norm_num)
  unfold ops.withdraw.withhold_liquidation_fee at h
  obtain ⟨_, ha, h⟩ := bind_eq_ok h
  have ha := massert_ok ha
  cases isl with
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte, ok.injEq, Prod.mk.injEq] at h
    obtain ⟨rfl, rfl⟩ := h
    exact ⟨fun _ => ⟨rfl, rfl⟩, fun h => by simp at h, hsh, 0, le_refl _, fun _ => rfl, hzb,
      by simp, by simp, rfl⟩
  | true =>
    by_cases hf0 : fee = 0#i128
    · simp only [hf0, ↓reduceIte, ok.injEq, Prod.mk.injEq] at h
      obtain ⟨rfl, rfl⟩ := h
      have hv : fee.val = 0 := by simp [hf0]
      exact ⟨fun _ => ⟨rfl, rfl⟩, fun h => absurd hv h.2, hsh, 0, le_refl _, fun _ => rfl, hzb,
        by simp, by simp, rfl⟩
    · have hv : fee.val ≠ 0 := fun he => hf0 (by apply IScalar.eq_of_val_eq; simpa using he)
      simp only [hf0, ↓reduceIte] at h
      have hle : fee.val ≤ g.val := by
        simp only [hf0, Bool.not_true, Bool.false_or, decide_eq_true_eq, false_or] at ha
        simpa [IScalar.le_equiv] using ha
      obtain ⟨mp, hmp, h⟩ := bind_eq_ok h
      simp only [market_cache.Cache.impl.params, ok.injEq] at hmp; subst hmp
      obtain ⟨fr, hfr, h⟩ := bind_eq_ok h
      have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
      have hF : fee.val * 10^(27 - c.params.asset_decimals.val) ≤
          g.val * 10^(27 - c.params.asset_decimals.val) :=
        mul_le_mul_of_nonneg_right hle (by positivity)
      have hfrv := spec_ok_eq (from_asset_spec env fee c.params.asset_decimals hdec
        ⟨by nlinarith, by omega⟩) hfr
      obtain ⟨c1, hc1, h⟩ := bind_eq_ok h
      have hrev := spec_ok_eq (add_protocol_revenue_spec c fr hsh hsi (by rw [hfrv]; positivity)) hc1
      obtain ⟨o, ho, h⟩ := bind_eq_ok h
      obtain ⟨n, hn, h⟩ := bind_eq_ok h
      simp only [ok.injEq, Prod.mk.injEq] at h
      obtain ⟨rfl, rfl⟩ := h
      have hnv := sub_unwrap_eq_ok _ _ (by intro v; simp [
        ops.withdraw.withhold_liquidation_fee.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once])
        _ _ _ _ ho hn
      have hfrne : fr.val ≠ 0 := by
        rw [hfrv]; exact mul_ne_zero hv (by positivity)
      obtain ⟨hsh1, _, hne⟩ := hrev
      obtain ⟨s, hs0, hsq, hr1, hs1, hce⟩ := hne hfrne
      -- the fee shares are worth at most the fee: `s ≤ ⌊fee·RAY/si⌋`
      have hq := isFloor_ediv (fr.val * 10^27) c.supply_index.val hsi
      have hsle : s ≤ fr.val * 10^27 / c.supply_index.val := by
        rw [hsq _ hq]; exact le_trans (min_le_left _ _) (min_le_right _ _)
      have hfl : fr.val * 10^27 / c.supply_index.val * c.supply_index.val ≤ fr.val * 10^27 :=
        Int.ediv_mul_le _ (by omega)
      have hsb : s * c.supply_index.val ≤
          fee.val * 10^(27 - c.params.asset_decimals.val) * 10^27 := by
        rw [← hfrv]
        exact le_trans (Int.mul_le_mul_of_nonneg_right hsle (by omega)) hfl
      exact ⟨fun h => absurd ⟨rfl, hv⟩ h, fun _ => ⟨hle, hnv⟩, hsh1, s, hs0,
        fun h => absurd ⟨rfl, hv⟩ h, hsb, hr1, hs1, hce⟩

/-- **`withdraw::gate_and_debit`.** Whenever it returns `c'`: `0 ≤ net ≤
cash` (`require_reserves`, `debit_cash`), the exit utilization guard passed
unless skipped (INV-ACCT-08; `require_exit_utilization_below_max_spec`: it is
the full guard below a 100% ceiling and no check at 100%),
`require_supply_for_debt` passed, i.e. not `supplied = 0 ∧ borrowed ≠ 0`
(INV-ACCT-09), and `c'` is `c` with `cash -= net` only. -/
theorem gate_and_debit_ok (env : Env) (c : market_cache.Cache) (net : Std.I128) (skip : Bool)
    (c' : market_cache.Cache) (h : ops.withdraw.gate_and_debit env c net skip = ok c') :
    0 ≤ net.val ∧ net.val ≤ c.cash.val ∧
    (skip = false → ∃ u, guards.require_exit_utilization_below_max env c = ok u) ∧
    ¬ (c.supplied.val = 0 ∧ c.borrowed.val ≠ 0) ∧
    c'.cash.val = c.cash.val - net.val ∧ c' = { c with cash := c'.cash } := by
  unfold ops.withdraw.gate_and_debit at h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨_, hu, h⟩ := bind_eq_ok h
  obtain ⟨u2, hs, h⟩ := bind_eq_ok h
  have hd := (debit_cash_spec c net).1.mp ⟨c', h⟩
  obtain ⟨hcash, _, hce⟩ := (debit_cash_spec c net).2 c' h
  refine ⟨hd.1, hd.2, ?_, (require_supply_for_debt_spec env c).1 ⟨u2, hs⟩, hcash, hce⟩
  intro hsk; subst hsk
  exact ⟨_, by simpa using hu⟩

/-- **`ops::withdraw::accounting` (the withdraw leg, before the token
transfer).** Whenever it returns `r` on a market synced to `c` with
`PoolInv c`, with `N = position·supply_index`, `H = hu(hu(N, RAY), 10^k)`
(the half-up position value the full-close switch compares with), `F =
⌊N/(RAY·10^k)⌋` (its floor value), `G = r.mutation.actual_amount` (gross),
`new = r.mutation.position`, `net = r.net_transfer`:

* `amount, position, protocol_fee ≥ 0` (validated); `0 ≤ G ≤ F` (never
  pays more than the position's floor value), `G ≤ amount`,
  `0 ≤ new ≤ position`;
* full close (`H ≤ amount`): `new = 0`, `G = F`;
* partial (`amount < H`): `G = amount`, the burn `position − new` is
  `⌈amount·10^k·RAY/supply_index⌉` (ceiling, ADR-0003), worth at least `G`;
* a positive gross burns shares: `0 < G → new < position`
  (`WithdrawRoundsToZeroShares`, INV-ACCT-05);
* `withdraw_summary`'s position clause where it holds: `amount > 0 ∧
  position > 0 → new < position`; `position = 0 → new = 0`; `amount = 0 ∧
  H > 0 → new = position`. **It does not hold for `amount = 0` and a dust
  position with `H = 0`: the code closes the position (`new = 0`, `G = F =
  0`)**; see docs/pool.md, Findings;
* liquidation fee: outside a fee-bearing liquidation `net = G`; in one,
  `protocol_fee ≤ G` and `net = G − protocol_fee`; always `0 ≤ net ≤ G`;
* the committed cache is `c` with `supplied := supplied − (position − new)
  + s`, `revenue := revenue + s` (`s ≥ 0` fee shares, `0` without a fee) and
  `cash -= net` only: INV-ACCT-10 (the pool's non-revenue supply drops by
  exactly the account's burn), INV-ACCT-02 (cash debited by exactly the net
  transfer), `PoolInv` holds, and INV-ACCT-09 (not `supplied = 0 ∧ borrowed
  ≠ 0`) holds after the leg.

Hypotheses: `PoolInv c`, `decimals ≤ 18`, `hu(position·supply_index, RAY)`
fits `i128` (exactly the abort condition of `resolve_withdrawal`, see
RatesAeneas `resolve_withdrawal_returns_iff`). -/
theorem withdraw_accounting_spec (env : Env) (isl : Bool) (entry : types.pool.PoolWithdrawEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.action.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfit : ∃ q : Int, IsHalfUp (entry.action.position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (r : ops.withdraw.WithdrawOutcome) (h : ops.withdraw.accounting env isl entry = ok r) :
    let amt := entry.action.amount.val
    let pos := entry.action.position.scaled_amount.val
    let G := r.mutation.actual_amount.val
    let new := r.mutation.position.scaled_amount.val
    0 ≤ amt ∧ 0 ≤ pos ∧ 0 ≤ entry.protocol_fee.val ∧
    0 ≤ G ∧ G ≤ amt ∧ 0 ≤ new ∧ new ≤ pos ∧
    (∀ rr H F : Int, IsHalfUp (pos * c.supply_index.val) (10^27) rr →
      IsHalfUp rr (10^(27 - c.params.asset_decimals.val)) H →
      IsFloor (pos * c.supply_index.val) (10^27 * 10^(27 - c.params.asset_decimals.val)) F →
      G ≤ F ∧ (H ≤ amt → new = 0 ∧ G = F) ∧
      (amt < H → G = amt ∧
        IsCeil (amt * 10^(27 - c.params.asset_decimals.val) * 10^27) c.supply_index.val
          (pos - new)) ∧
      (amt = 0 → 0 < H → new = pos)) ∧
    (0 < G → new < pos) ∧
    (0 < amt → 0 < pos → new < pos) ∧ (pos = 0 → new = 0) ∧
    (¬ (isl = true ∧ entry.protocol_fee.val ≠ 0) → r.net_transfer.val = G) ∧
    (isl = true ∧ entry.protocol_fee.val ≠ 0 → entry.protocol_fee.val ≤ G ∧
      r.net_transfer.val = G - entry.protocol_fee.val) ∧
    0 ≤ r.net_transfer.val ∧ r.net_transfer.val ≤ G ∧
    r.mutation.market_index = indexOf c ∧ r.mutation.asset_decimals = c.params.asset_decimals ∧
    r.snapshot = snapOf r.cache ∧ PoolInv r.cache ∧
    ¬ (r.cache.supplied.val = 0 ∧ r.cache.borrowed.val ≠ 0) ∧
    (∃ s : Int, 0 ≤ s ∧ (¬ (isl = true ∧ entry.protocol_fee.val ≠ 0) → s = 0) ∧
      s ≤ pos - new ∧
      r.cache.supplied.val = c.supplied.val - (pos - new) + s ∧
      r.cache.revenue.val = c.revenue.val + s) ∧
    r.cache.cash.val = c.cash.val - r.net_transfer.val ∧
    r.cache = { c with supplied := r.cache.supplied, revenue := r.cache.revenue,
                       cash := r.cache.cash } := by
  intro amt pos G new
  unfold ops.withdraw.accounting at h
  obtain ⟨_, hfee, h⟩ := bind_eq_ok h
  have hfee := nonneg_of_ok _ _ _ hfee
  obtain ⟨⟨c0, p0⟩, hl, h⟩ := bind_eq_ok h
  obtain ⟨hamt, hpos, hs0, rfl⟩ := load_leg_ok env _ _ _ hl
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨⟨bu, gr⟩, hrc, h⟩ := bind_eq_ok h
  obtain ⟨⟨rem, c1⟩, hbp, h⟩ := bind_eq_ok h
  obtain ⟨⟨nt, c2⟩, hwh, h⟩ := bind_eq_ok h
  simp only [math.fp.Ray.raw, bind_ok] at h
  obtain ⟨ec, hec, h⟩ := bind_eq_ok h
  -- resolve_close_or_partial
  unfold ops.withdraw.resolve_close_or_partial at hrc
  obtain ⟨⟨bu0, gr0⟩, hrw, hrc⟩ := bind_eq_ok hrc
  have hgb : gr = gr0 ∧ bu = bu0 ∧ (gr0.val ≠ 0 → 0 < bu0.val) := by
    by_cases hg : gr0 = 0#i128
    · simp [hg] at hrc
      try simp only [ok.injEq, Prod.mk.injEq] at hrc
      obtain ⟨h1, h2⟩ := hrc
      exact ⟨by rw [← h2, hg], h1.symm, fun hn => absurd (by simp [hg]) hn⟩
    · by_cases hb : 0 < bu0.val
      · have hb' : bu0 > 0#i128 := by simpa [IScalar.lt_equiv] using hb
        simp [hg, hb, math.fp.Ray.raw] at hrc
        try simp only [ok.injEq, Prod.mk.injEq] at hrc
        obtain ⟨h1, h2⟩ := hrc
        exact ⟨h2.symm, h1.symm, fun _ => hb⟩
      · have hb' : ¬ bu0 > 0#i128 := by simpa [IScalar.lt_equiv] using hb
        simp [hg, hb, math.fp.Ray.raw, market_cache.Cache.impl.env] at hrc
  obtain ⟨rfl, rfl, hbpos⟩ := hgb
  have hdec27 : c.params.asset_decimals.val < 27 := by omega
  unfold market_cache.scale.Cache.resolve_withdrawal at hrw
  have hspec := spec_ok_eq (resolve_withdrawal_spec c.env entry.action.amount
    entry.action.position.scaled_amount c.supply_index c.params.asset_decimals hdec27
    hinv.si_pos hpos hamt hfit) hrw
  simp only at hspec
  -- the canonical r, H, F
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  have hN : 0 ≤ pos * c.supply_index.val := by have := hinv.si_pos; positivity
  obtain ⟨r0, hr0⟩ := isHalfUp_exists (pos * c.supply_index.val) (10^27) (by norm_num)
  have hr00 : 0 ≤ r0 := IsHalfUp.nonneg (by norm_num) hN hr0
  obtain ⟨H0, hH0⟩ := isHalfUp_exists r0 (10^(27 - c.params.asset_decimals.val)) hk
  obtain ⟨F0, hF0⟩ := exists_isFloor (pos * c.supply_index.val)
    (10^27 * 10^(27 - c.params.asset_decimals.val)) (by positivity)
  obtain ⟨⟨hb0, hbpos'⟩, ⟨hg0, hgF, hga⟩, hfull, hpart⟩ := hspec r0 H0 F0 hr0 hH0 hF0
  -- burn_position
  unfold ops.withdraw.burn_position at hbp
  obtain ⟨c1', hc1, hbp⟩ := bind_eq_ok hbp
  obtain ⟨rem', hrem, hbp⟩ := bind_eq_ok hbp
  simp only [ok.injEq, Prod.mk.injEq] at hbp
  have e1 := hbp.1.symm; have e2 := hbp.2.symm; subst e1; subst e2
  have hremv := (ray_checked_sub_eq_ok _ _ _ _ hrem).2.2.2
  obtain ⟨hsh1, _, _, hsup1, hc1e⟩ := burn_supply_preserves c c1 bu hinv.shares hc1
  -- the fee fits: fee ≤ gross ≤ F and F·10^k ≤ hu(N, RAY) < 2^127
  obtain ⟨q, hq, hqlt⟩ := hfit
  have hrq : r0 = q := IsHalfUp.unique (by norm_num) hr0 hq
  have hFk : F0 * 10^(27 - c.params.asset_decimals.val) ≤ q := by
    have hFc := hF0; unfold IsFloor at hFc; rw [if_pos (by positivity)] at hFc
    have hqc := hq; unfold IsHalfUp at hqc
    by_contra hc
    have : (q + 1) * 10^27 ≤ F0 * 10^(27 - c.params.asset_decimals.val) * 10^27 :=
      mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    nlinarith
  have hsi1 : 0 < c1.supply_index.val := by rw [hc1e]; exact hinv.si_pos
  have hpar1 : c1.params = c.params := by rw [hc1e]
  -- withhold
  have hwh' := withhold_liquidation_fee_ok env c1 gr entry.protocol_fee isl hsh1 hsi1 hfee
    (by rw [hpar1]; exact hdec27) (by
      rw [hpar1]
      have : gr.val * 10^(27 - c.params.asset_decimals.val) ≤
          F0 * 10^(27 - c.params.asset_decimals.val) :=
        mul_le_mul_of_nonneg_right hgF (by positivity)
      omega) nt c2 hwh
  obtain ⟨hwn, hwf, hsh2, s, hs0, hsz, hsb, hrev2, hsup2, hc2e⟩ := hwh'
  have hfin : ∃ skip c3, ops.withdraw.gate_and_debit env c2 nt skip = ok c3 ∧
      r = { cache := c3, snapshot := snapOf c3, net_transfer := nt,
            mutation := ({ position := { scaled_amount := rem }, market_index := indexOf c3, actual_amount := gr, asset_decimals := c3.params.asset_decimals } : types.pool.PoolPositionMutation) } := by
    cases isl
    · simp only [Bool.false_eq_true, ↓reduceIte] at h
      obtain ⟨c3, hg, h⟩ := bind_eq_ok h
      rw [commit_eq, bind_ok, position_mutation_eq, bind_ok, ok.injEq] at h
      exact ⟨ec, c3, hg, h.symm⟩
    · simp only [↓reduceIte] at h
      obtain ⟨c3, hg, h⟩ := bind_eq_ok h
      rw [commit_eq, bind_ok, position_mutation_eq, bind_ok, ok.injEq] at h
      exact ⟨true, c3, hg, h.symm⟩
  obtain ⟨skip, c3, hgd, rfl⟩ := hfin
  obtain ⟨hnt0, hntc, _, h09, hcash3, hc3e⟩ := gate_and_debit_ok env c2 nt skip c3 hgd
  simp only [amt, pos, G, new] at *
  have hsup3 : c3.supplied = c2.supplied := by rw [hc3e]
  have hrev3 : c3.revenue = c2.revenue := by rw [hc3e]
  have hbor3 : c3.borrowed = c.borrowed := by rw [hc3e, hc2e, hc1e]
  have hsi3 : c3.supply_index = c.supply_index := by rw [hc3e, hc2e, hc1e]
  have hbi3 : c3.borrow_index = c.borrow_index := by rw [hc3e, hc2e, hc1e]
  have hpar3 : c3.params = c.params := by rw [hc3e, hc2e, hc1e]
  have hcash2 : c2.cash = c.cash := by rw [hc2e, hc1e]
  have hidx : indexOf c3 = indexOf c := by unfold indexOf; rw [hsi3, hbi3]
  have hc3full : c3 = { c with supplied := c3.supplied, revenue := c3.revenue, cash := c3.cash } := by
    rw [hc3e, hc2e, hc1e]
  have hfe : ¬ (isl = true ∧ entry.protocol_fee.val ≠ 0) → nt = gr := fun hn => (hwn hn).1
  have hntG : nt.val ≤ gr.val := by
    by_cases hz : isl = true ∧ entry.protocol_fee.val ≠ 0
    · have := (hwf hz).2; omega
    · rw [hfe hz]
  -- the fee shares are worth at most the fee, the fee is at most the gross
  -- amount, and the burned shares are worth at least the gross amount
  have hsbu : s ≤ bu.val := by
    by_cases hz : isl = true ∧ entry.protocol_fee.val ≠ 0
    · have hfg := (hwf hz).1
      rw [show c1.supply_index = c.supply_index by rw [hc1e], hpar1] at hsb
      have hfee_le : entry.protocol_fee.val * 10^(27 - c.params.asset_decimals.val) * 10^27 ≤
          gr.val * 10^(27 - c.params.asset_decimals.val) * 10^27 :=
        mul_le_mul_of_nonneg_right (mul_le_mul_of_nonneg_right hfg (le_of_lt hk)) (by norm_num)
      have hgb : gr.val * 10^(27 - c.params.asset_decimals.val) * 10^27 ≤
          bu.val * c.supply_index.val := by
        by_cases hle : H0 ≤ entry.action.amount.val
        · obtain ⟨hbe, hge⟩ := hfull hle
          have hFc := hF0; unfold IsFloor at hFc; rw [if_pos (by positivity)] at hFc
          have e : gr.val * 10^(27 - c.params.asset_decimals.val) * 10^27 =
              F0 * (10^27 * 10^(27 - c.params.asset_decimals.val)) := by rw [hge]; ring
          rw [e, hbe]; exact hFc.1
        · obtain ⟨hge, _, hc, _⟩ := hpart (by omega)
          rw [hge]; exact hc
      exact le_of_mul_le_mul_right (le_trans hsb (le_trans hfee_le hgb)) hinv.si_pos
    · rw [hsz hz]; exact hb0
  have hceil0 : ∀ q' : Int, IsCeil (0 * 10^(27 - c.params.asset_decimals.val) * 10^27)
      c.supply_index.val q' → q' = 0 := by
    intro q' hq'
    simp only [zero_mul] at hq'
    exact IsCeil.unique (by have := hinv.si_pos; omega) hq'
      (by unfold IsCeil; rw [if_pos hinv.si_pos]; constructor <;> nlinarith [hinv.si_pos])
  refine ⟨hamt, hpos, hfee, hg0, hga, by rw [hremv]; omega, by rw [hremv]; omega, ?_, ?_, ?_,
    ?_, ?_, ?_, hnt0, hntG, hidx, by simp [hpar3], by simp, ?_, ?_,
    ⟨s, hs0, hsz, by rw [hremv]; omega, ?_, ?_⟩, ?_, hc3full⟩
  · intro rr H F hrr hH hF
    have e1 : rr = r0 := IsHalfUp.unique (by norm_num) hrr hr0
    subst e1
    have e2 : H = H0 := IsHalfUp.unique hk hH hH0
    have e3 : F = F0 := IsFloor.unique (by positivity) hF hF0
    subst e2; subst e3
    refine ⟨hgF, fun hle => ⟨by rw [hremv, (hfull hle).1]; omega, (hfull hle).2⟩,
      fun hlt => ?_, fun ha0 hH => ?_⟩
    · obtain ⟨hg, hc, _, _⟩ := hpart hlt
      exact ⟨by rw [hg], by rw [hremv]; simpa using hc⟩
    · obtain ⟨_, hc, _, _⟩ := hpart (by omega)
      rw [ha0] at hc
      have := hceil0 _ hc
      rw [hremv]; omega
  · intro hG; have := hbpos (by omega); rw [hremv]; omega
  · intro ha hp
    by_cases hle : H0 ≤ entry.action.amount.val
    · rw [hremv, (hfull hle).1]; omega
    · obtain ⟨hg, _, _, _⟩ := hpart (by omega)
      have := hbpos (by rw [hg]; omega); rw [hremv]; omega
  · intro hp0; rw [hremv]; omega
  · intro hn; rw [hfe hn]
  · exact hwf
  · refine ⟨⟨?_, ?_, ?_⟩, by rw [hcash3]; omega, by rw [hsi3]; exact hinv.si_lo,
      by rw [hsi3]; exact hinv.si_hi, by rw [hbi3]; exact hinv.bi_lo, by rw [hbi3]; exact hinv.bi_hi⟩
    · rw [hrev3]; exact hsh2.1
    · rw [hrev3, hsup3]; exact hsh2.2.1
    · rw [hbor3]; exact hinv.shares.2.2
  · rw [hsup3, hbor3]; rw [show c.borrowed = c2.borrowed by rw [hc2e, hc1e]]; exact h09
  · rw [hsup3, hsup2, hsup1, hremv]; ring
  · rw [hrev3, hrev2, show c1.revenue = c.revenue by rw [hc1e]]
  · rw [hcash3, hcash2]

end PoolAeneas.Proofs
