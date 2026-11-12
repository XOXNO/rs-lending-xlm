/-
The remaining pool operation legs (`net_settle`, `seize`, `recapitalize`,
`strategy`, `revenue`) and the load step (`Cache::load`, `synced_market`).
Conventions as in `Legs.lean`.
-/
import PoolAeneas.Proofs.Legs

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec PoolAeneas.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 1000000

namespace PoolAeneas.Proofs

/-! ## `net_settle` -/

/-- **`ops::net_settle::apply` (same-asset net settlement).** Whenever it
returns `(res, snapshot)` on a market synced to `c` with `PoolInv c`, with
`ss`, `ds` the incoming supply and debt positions, `F = ⌊ss·si/(RAY·10^k)⌋`
(floored claim), `D = ⌈ds·bi/(RAY·10^k)⌉` (ceiled debt), `g =
res.settled_amount`, `ns`, `nd` the returned positions:

* `amount, ss, ds ≥ 0` (validated); `0 ≤ g ≤ amount`, and when positive
  `g ≤ F`, `g ≤ D`;
* `0 ≤ ns ≤ ss`, `0 ≤ nd ≤ ds`; `g > 0` burns on both sides (`ns < ss`,
  `nd < ds`, `NetSettleRoundsToZeroShares`, INV-ACCT-05); `g = 0` leaves both
  positions unchanged — exactly `net_settle_summary`;
* a side closes fully only when the settlement exhausts it; partial burns
  favour the protocol (supply burn worth at least `g`, debt burn clears at
  most `g`, ADR-0003);
* the committed cache is `c` with `supplied -= ss − ns` and `borrowed -= ds
  − nd` only (INV-ACCT-10; no cash moves, INV-ACCT-02), `PoolInv` holds, and
  not `supplied = 0 ∧ borrowed ≠ 0` (INV-ACCT-09); the returned indexes are
  the synced market's.

Hypotheses: `PoolInv c`, `decimals ≤ 18`, and the RAY values `⌊ss·si/RAY⌋`,
`⌈ds·bi/RAY⌉` fit `i128` (otherwise the unscale calls abort). -/
theorem net_settle_apply_spec (env : Env) (entry : types.pool.PoolNetSettleEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.hub_asset = ok c)
    (hinv : PoolInv c) (hdec : c.params.asset_decimals.val ≤ 18)
    (hfitS : ∃ q : Int, IsFloor (entry.supply_position.scaled_amount.val * c.supply_index.val)
      (10^27) q ∧ q < 2^127)
    (hfitD : ∃ q : Int, IsCeil (entry.debt_position.scaled_amount.val * c.borrow_index.val)
      (10^27) q ∧ q < 2^127)
    (r : types.pool.PoolNetSettleResult × types.pool.MarketStateSnapshot)
    (h : ops.net_settle.apply env entry = ok r) :
    let ss := entry.supply_position.scaled_amount.val
    let ds := entry.debt_position.scaled_amount.val
    let g := r.1.settled_amount.val
    let ns := r.1.supply_position.scaled_amount.val
    let nd := r.1.debt_position.scaled_amount.val
    0 ≤ entry.amount.val ∧ 0 ≤ ss ∧ 0 ≤ ds ∧
    0 ≤ g ∧ g ≤ entry.amount.val ∧
    0 ≤ ns ∧ ns ≤ ss ∧ 0 ≤ nd ∧ nd ≤ ds ∧
    (0 < g → ns < ss ∧ nd < ds) ∧ (g = 0 → ns = ss ∧ nd = ds) ∧
    (∀ F D : Int, IsFloor (ss * c.supply_index.val) (10^27 * 10^(27 - c.params.asset_decimals.val)) F →
      IsCeil (ds * c.borrow_index.val) (10^27 * 10^(27 - c.params.asset_decimals.val)) D →
      (0 < g → g ≤ F ∧ g ≤ D) ∧ (0 < g → g = F → ns = 0) ∧ (0 < g → g = D → nd = 0) ∧
      (0 < g → g < F → g * 10^(27 - c.params.asset_decimals.val) * 10^27 ≤
        (ss - ns) * c.supply_index.val) ∧
      (0 < g → g < D → (ds - nd) * c.borrow_index.val ≤
        g * 10^(27 - c.params.asset_decimals.val) * 10^27)) ∧
    r.1.market_index = indexOf c ∧
    ∃ c' : market_cache.Cache, r.2 = snapOf c' ∧ PoolInv c' ∧
      ¬ (c'.supplied.val = 0 ∧ c'.borrowed.val ≠ 0) ∧
      c'.supplied.val = c.supplied.val - (ss - ns) ∧
      c'.borrowed.val = c.borrowed.val - (ds - nd) ∧
      c' = { c with supplied := c'.supplied, borrowed := c'.borrowed } := by
  intro ss ds g ns nd
  unfold ops.net_settle.apply at h
  obtain ⟨_, h1, h⟩ := bind_eq_ok h
  obtain ⟨_, h2, h⟩ := bind_eq_ok h
  obtain ⟨_, h3, h⟩ := bind_eq_ok h
  have hamt := nonneg_of_ok _ _ _ h1
  have hss := nonneg_of_ok _ _ _ h2
  have hds := nonneg_of_ok _ _ _ h3
  obtain ⟨c0, hs0, h⟩ := bind_eq_ok h
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  rw [ray_from_same, bind_ok, ray_from_same, bind_ok] at h
  obtain ⟨⟨bs, bd, gr⟩, hrn, h⟩ := bind_eq_ok h
  obtain ⟨_, hif, h⟩ := bind_eq_ok h
  obtain ⟨c1, hc1, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  obtain ⟨u, hsd, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok] at h
  obtain ⟨rs, hrs, h⟩ := bind_eq_ok h
  simp only [math.fp.Ray.raw, bind_ok] at h
  obtain ⟨rd, hrd, h⟩ := bind_eq_ok h
  rw [market_index_eq, bind_ok, ok.injEq] at h
  subst h
  simp only [ss, ds, g, ns, nd]
  have hrsv := (ray_checked_sub_eq_ok _ _ _ _ hrs).2.2.2
  have hrdv := (ray_checked_sub_eq_ok _ _ _ _ hrd).2.2.2
  have hdec27 : c.params.asset_decimals.val < 27 := by omega
  obtain ⟨qS, hqS, hqSlt⟩ := hfitS
  obtain ⟨qD, hqD, hqDlt⟩ := hfitD
  have hqS0 : 0 ≤ qS := IsFloor.nonneg (by norm_num) (by have := hinv.si_pos; positivity) hqS
  have hqD0 : 0 ≤ qD := by
    have hqc := hqD; unfold IsCeil at hqc; rw [if_pos (by norm_num)] at hqc
    have : 0 ≤ entry.debt_position.scaled_amount.val * c.borrow_index.val := by
      have := hinv.bi_pos; positivity
    nlinarith
  unfold market_cache.scale.Cache.resolve_net_settle at hrn
  have hspec := spec_ok_eq (resolve_net_settle_spec c.env entry.amount
    entry.supply_position.scaled_amount entry.debt_position.scaled_amount c.supply_index
    c.borrow_index c.params.asset_decimals hdec27 hinv.si_pos hinv.bi_pos hss hds
    ⟨qS, hqS, by omega, hqSlt⟩ ⟨qD, hqD, by omega, hqDlt⟩) hrn
  simp only at hspec
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  obtain ⟨F0, hF0⟩ := exists_isFloor (entry.supply_position.scaled_amount.val * c.supply_index.val)
    (10^27 * 10^(27 - c.params.asset_decimals.val)) (by positivity)
  obtain ⟨D0, hD0⟩ := exists_isCeil (entry.debt_position.scaled_amount.val * c.borrow_index.val)
    (10^27 * 10^(27 - c.params.asset_decimals.val)) (by positivity)
  obtain ⟨⟨hbs0, hbsle⟩, ⟨hbd0, hbdle⟩, hg0, hgle, hgz, hgF, hgD, hpS, hpD⟩ := hspec F0 D0 hF0 hD0
  -- positive settlement burns both sides
  have hbpos : gr.val ≠ 0 → 0 < bs.val ∧ 0 < bd.val := by
    intro hn
    have hn' : ¬ (gr = 0#i128) := fun he => hn (by simp [he])
    simp only [hn', ↓reduceIte] at hif
    obtain ⟨_, hr1, hif⟩ := bind_eq_ok hif
    obtain ⟨_, ha1, hif⟩ := bind_eq_ok hif
    obtain ⟨_, hr2, hif⟩ := bind_eq_ok hif
    simp only [math.fp.Ray.raw, ok.injEq] at hr1 hr2; subst hr1; subst hr2
    exact ⟨by simpa [IScalar.lt_equiv] using massert_ok ha1,
      by simpa [IScalar.lt_equiv] using massert_ok hif⟩
  obtain ⟨hsh1, _, _, hsup1, hc1e⟩ := burn_supply_preserves c c1 bs hinv.shares hc1
  obtain ⟨hsh2, _, _, hbor2, hc2e⟩ := burn_debt_preserves c1 c2 bd hsh1 hc2
  have h09 := (require_supply_for_debt_spec env c2).1 ⟨u, hsd⟩
  have hsup2 : c2.supplied = c1.supplied := by rw [hc2e]
  have hidx : indexOf c2 = indexOf c := by unfold indexOf; rw [hc2e, hc1e]
  have hc2full : c2 = { c with supplied := c2.supplied, borrowed := c2.borrowed } := by
    rw [hc2e, hc1e]
  refine ⟨hamt, hss, hds, hg0, ?_, by rw [hrsv]; omega, by rw [hrsv]; omega, by rw [hrdv]; omega,
    by rw [hrdv]; omega, ?_, ?_, ?_, hidx, c2, rfl, ?_, h09, ?_, ?_, hc2full⟩
  · rcases lt_or_eq_of_le hg0 with hp | he
    · exact (hgle hp).1
    · rw [← he]; exact hamt
  · intro hp; have := hbpos (by omega); rw [hrsv, hrdv]; omega
  · intro hz; have := hgz hz; rw [hrsv, hrdv]; omega
  · intro F D hF hD
    have e1 : F = F0 := IsFloor.unique (by positivity) hF hF0
    have e2 : D = D0 := IsCeil.unique (by positivity) hD hD0
    subst e1; subst e2
    refine ⟨fun hp => ⟨(hgle hp).2.1, (hgle hp).2.2⟩, fun hp he => ?_, fun hp he => ?_,
      fun hp hl => ?_, fun hp hl => ?_⟩
    · rw [hrsv, hgF hp he]; omega
    · rw [hrdv, hgD hp he]; omega
    · rw [hrsv]; simpa using hpS hp hl
    · rw [hrdv]; simpa using hpD hp hl
  · refine hinv.of_eq hsh2 (by rw [hc2full]; exact hinv.cash) (by rw [hc2full]) (by rw [hc2full])
  · rw [hsup2, hsup1, hrsv]; ring
  · rw [hbor2, show c1.borrowed = c.borrowed by rw [hc1e], hrdv]; ring

/-! ## `seize` -/

/-- **`ops::seize::apply` (one `seize_positions` entry).** Whenever it
returns `snapshot` on a market synced to `c` with `PoolInv c`, the seized
scaled position `p` was `≥ 0` and the committed cache `c'` (`snapshot =
snapOf c'`) satisfies `PoolInv c'` with cash unchanged (no tokens move):

* deposit side: `c'` is `c` with `revenue += p` only; `supplied` unchanged
  (INV-ACCT-01 reclassification; INV-ACCT-10: the account's `p` supply
  shares leave the account books and become revenue shares);
* borrow side: `c'` is `c` with `borrowed -= p` (the whole debt position,
  INV-ACCT-10) and a supply index written down for the bad debt
  `⌈p·bi/RAY⌉` (INV-IDX-03): the new index is `≥ SUPPLY_INDEX_FLOOR_RAW`
  and never above the old one; `revenue`, `supplied`, the borrow index and
  cash unchanged.

So `seize_positions_summary` (empty: no return value) abstracts a leg that
preserves the stored-state invariant `get_sync_data_summary` assumes.
Hypotheses: `PoolInv c`, and for the borrow side the half-up supply value
`hu(supplied·si, RAY)` fits `i128` (otherwise `apply_bad_debt_to_supply_index`
aborts). -/
theorem seize_apply_spec (env : Env) (entry : types.pool.PoolSeizeEntry)
    (c : market_cache.Cache) (hsync : ops.synced_market env entry.hub_asset = ok c)
    (hinv : PoolInv c)
    (hV : ∃ V : Int, IsHalfUp (c.supplied.val * c.supply_index.val) (10^27) V ∧ V < 2^127)
    (r : types.pool.MarketStateSnapshot) (h : ops.seize.apply env entry = ok r) :
    0 ≤ entry.position.scaled_amount.val ∧
    ∃ c' : market_cache.Cache, r = snapOf c' ∧ PoolInv c' ∧ c'.cash = c.cash ∧
      (entry.side = .Deposit →
        c'.revenue.val = c.revenue.val + entry.position.scaled_amount.val ∧
        c' = { c with revenue := c'.revenue }) ∧
      (entry.side = .Borrow →
        c'.borrowed.val = c.borrowed.val - entry.position.scaled_amount.val ∧
        c'.supply_index.val ≤ c.supply_index.val ∧
        c' = { c with borrowed := c'.borrowed, supply_index := c'.supply_index }) := by
  unfold ops.seize.apply at h
  obtain ⟨_, h1, h⟩ := bind_eq_ok h
  have hp := nonneg_of_ok _ _ _ h1
  obtain ⟨c0, hs0, h⟩ := bind_eq_ok h
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  rw [ray_from_same, bind_ok] at h
  obtain ⟨c1, hc1, h⟩ := bind_eq_ok h
  rw [commit_eq, ok.injEq] at h
  subst h
  refine ⟨hp, c1, rfl, ?_⟩
  cases hside : entry.side with
  | Deposit =>
    simp only [hside] at hc1
    obtain ⟨hsh, hrev, hce⟩ := absorb_supply_as_revenue_preserves c c1 _ hinv.shares hp hc1
    refine ⟨hinv.of_eq hsh (by rw [hce]; exact hinv.cash) (by rw [hce]) (by rw [hce]),
      by rw [hce], fun _ => ⟨hrev, hce⟩, fun h => by cases h⟩
  | Borrow =>
    simp only [hside] at hc1
    obtain ⟨bad, hbad, hk1⟩ := bind_eq_ok hc1
    obtain ⟨c2, hc2, hk2⟩ := bind_eq_ok hk1
    unfold market_cache.scale.Cache.unscale_borrow_ceil_ray math.fp.Ray.mul_ceil at hbad
    obtain ⟨bq, hbq, hbad⟩ := bind_eq_ok hbad
    simp only [ok.injEq] at hbad; subst hbad
    have hbc := mul_div_ceil_eq_ok c.env _ _ _ _ (by rw [ray_val]; norm_num) hbq
    rw [ray_val] at hbc
    have hb0 : 0 ≤ bq.val := by
      have hbc' := hbc; unfold IsCeil at hbc'; rw [if_pos (by norm_num)] at hbc'
      have : 0 ≤ entry.position.scaled_amount.val * c.borrow_index.val := by
        have := hinv.bi_pos; positivity
      nlinarith
    obtain ⟨V, hVh, hVlt⟩ := hV
    have hbd := spec_ok_eq (apply_bad_debt_spec c bq V
      (by have := hinv.shares; unfold SharesInv at this; omega) hinv.si_pos hb0 hVh hVlt) hc2
    obtain ⟨hc2e, hV0, hVne⟩ := hbd
    have hsi2 : 10^24 ≤ c2.supply_index.val ∧ c2.supply_index.val ≤ c.supply_index.val := by
      by_cases hz : V = 0
      · rw [hV0 hz]; exact ⟨hinv.si_lo, le_refl _⟩
      · obtain ⟨_, _, _, _, _, hlo, hhi, _⟩ := hVne hz
        have := hinv.si_lo
        exact ⟨hlo, by rw [max_eq_left (by omega)] at hhi; exact hhi⟩
    have hsh2 : SharesInv c2 := by rw [hc2e]; exact hinv.shares
    obtain ⟨hsh1, _, _, hbor1, hc1e⟩ := burn_debt_preserves c2 c1 _ hsh2 hk2
    have hsi1 : c1.supply_index = c2.supply_index := by rw [hc1e]
    refine ⟨⟨hsh1, by rw [hc1e, hc2e]; exact hinv.cash, by rw [hsi1]; exact hsi2.1,
      by rw [hsi1]; have := hinv.si_hi; omega, by rw [hc1e, hc2e]; exact hinv.bi_lo,
      by rw [hc1e, hc2e]; exact hinv.bi_hi⟩, by rw [hc1e, hc2e], fun h => absurd h (by simp),
      fun _ => ⟨by rw [hbor1, show c2.borrowed = c.borrowed by rw [hc2e]], by rw [hsi1]; exact hsi2.2,
        by rw [hc1e, hc2e]⟩⟩

/-! ## `recapitalize` -/

/-- `backing_shortfall` returned `sf`: `sf ≥ 0` (it is a `max(_, 0)`). -/
theorem backing_shortfall_nonneg (c : market_cache.Cache) (sf : Std.I128)
    (h : guards.backing_shortfall c = ok sf) : 0 ≤ sf.val := by
  unfold guards.backing_shortfall at h
  simp only [market_cache.Cache.impl.env, market_cache.Cache.impl.supplied,
    market_cache.Cache.impl.supply_index, market_cache.Cache.impl.borrowed,
    market_cache.Cache.impl.params, market_cache.Cache.impl.cash, bind_ok] at h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨b, _, h⟩ := bind_eq_ok h
  cases b
  · simp only [Bool.false_eq_true, ↓reduceIte] at h
    obtain ⟨_, _, h⟩ := bind_eq_ok h
    obtain ⟨_, _, h⟩ := bind_eq_ok h
    simp only [lift, bind_ok, ok.injEq] at h
    rw [← h, max_i128_val]; simp
  · simp only [↓reduceIte, ok.injEq] at h
    rw [← h]; simp

/-- **`ops::recapitalize::accounting`.** Whenever it returns `r` on a market
synced to `c` with `PoolInv c`: `amount ≥ 0`; the applied amount is
`min(amount, sf)` for the market's backing shortfall `sf ≥ 0`
(`backing_shortfall c = ok sf`; its value and the bound "never fills more
than the uncovered claim" are `backing_shortfall_spec`, INV-ACCT-04), so
`0 ≤ applied ≤ amount` — exactly `recapitalize_summary`; `refund = amount −
applied` (INV-ACCT-03: the unused receipt is refunded); the committed cache
is `c` with `cash += applied` only: no shares are minted or burned and the
indexes do not move (INV-ACCT-04: recapitalization cannot restore a
written-down index); `PoolInv` holds. Hypothesis: `PoolInv c`. -/
theorem recapitalize_accounting_spec (env : Env) (k : types.pool.HubAssetKey) (amount : Std.I128)
    (c : market_cache.Cache) (hsync : ops.synced_market env k = ok c) (hinv : PoolInv c)
    (r : ops.recapitalize.RecapitalizationOutcome)
    (h : ops.recapitalize.accounting env k amount = ok r) :
    0 ≤ amount.val ∧
    (∃ sf : Std.I128, guards.backing_shortfall c = ok sf ∧ 0 ≤ sf.val ∧
      r.mutation.actual_amount.val = min amount.val sf.val) ∧
    0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ amount.val ∧
    r.refund.val = amount.val - r.mutation.actual_amount.val ∧
    r.cache.cash.val = c.cash.val + r.mutation.actual_amount.val ∧
    r.cache = { c with cash := r.cache.cash } ∧ PoolInv r.cache := by
  unfold ops.recapitalize.accounting at h
  obtain ⟨_, h1, h⟩ := bind_eq_ok h
  have hamt := nonneg_of_ok _ _ _ h1
  obtain ⟨c0, hs0, h⟩ := bind_eq_ok h
  have hs0 := renewed_market_ok env k c0 hs0
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨sf, hsf, h⟩ := bind_eq_ok h
  obtain ⟨ap, hap, h⟩ := bind_eq_ok h
  simp only [lift, ok.injEq] at hap; subst hap
  obtain ⟨o, ho, h⟩ := bind_eq_ok h
  obtain ⟨rf, hrf, h⟩ := bind_eq_ok h
  obtain ⟨c1, hc1, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok, ok.injEq] at h
  subst h
  simp only
  have hsf0 := backing_shortfall_nonneg c sf hsf
  have hrfv := sub_unwrap_eq_ok _ _ (by intro v; simp [
    ops.recapitalize.accounting.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]) _ _ _ _ ho hrf
  obtain ⟨hcash, hce⟩ := (credit_cash_spec c _).2 c1 hc1
  rw [min_i128_val] at hrfv hcash ⊢
  refine ⟨hamt, ⟨sf, hsf, hsf0, rfl⟩, by omega, by omega, hrfv, hcash, hce, ?_⟩
  exact hinv.of_eq (by rw [hce]; exact hinv.shares) (by rw [hcash]; have := hinv.cash; omega)
    (by rw [hce]) (by rw [hce])

/-! ## `strategy` -/

/-- **`strategy::compute_fee`.** Whenever it returns `fee` (for `amount ≥ 0`
and a stored `flashloan_fee ≤ 10000` bps): `0 ≤ fee ≤ amount`
(`StrategyFeeExceeds`); without `charge_fee` the fee is 0; with it, it is
the flash-loan fee: `hu(amount·bps/10000)`, or 1 when `bps > 0` and that is
0. -/
theorem compute_fee_ok (env : Env) (c : market_cache.Cache) (amount : Std.I128) (charge : Bool)
    (hamt : 0 ≤ amount.val) (hbps : c.params.flashloan_fee.val ≤ 10000) (fee : Std.I128)
    (h : ops.strategy.compute_fee env c amount charge = ok fee) :
    0 ≤ fee.val ∧ fee.val ≤ amount.val ∧ (charge = false → fee.val = 0) ∧
    (charge = true → ∃ hh : Int, IsHalfUp (amount.val * c.params.flashloan_fee.val) 10000 hh ∧
      fee.val = (if (0 : Int) < c.params.flashloan_fee.val ∧ hh = 0 then 1 else hh)) := by
  unfold ops.strategy.compute_fee at h
  cases charge with
  | false =>
    simp only [Bool.false_eq_true, ↓reduceIte, ok.injEq] at h
    subst h
    have h0 : (0#i128 : Std.I128).val = 0 := by simp
    refine ⟨by omega, by omega, fun _ => h0, fun h => absurd h (by simp)⟩
  | true =>
    simp only [↓reduceIte, market_cache.Cache.impl.params, bind_ok] at h
    obtain ⟨k, hk, h⟩ := bind_eq_ok h
    have hkv : k.val = c.params.flashloan_fee.val := by
      simp only [I128.Insts.CoreConvertFromU32.from, IScalar.tryMk] at hk
      generalize hx : (c.params.flashloan_fee.val : Int) = x at hk ⊢
      have hm := IScalar.tryMkOpt_eq IScalarTy.I128 x
      cases ho : IScalar.tryMkOpt IScalarTy.I128 x with
      | none => rw [ho] at hk; simp [Result.ofOption] at hk
      | some z =>
        rw [ho] at hk hm
        simp only [Result.ofOption, ok.injEq] at hk
        rw [← hk, hm.1]
    simp only [math.fp.Bps.from, core.convert.IntoFrom.into, core.convert.FromSame.from,
      bind_ok] at h
    obtain ⟨f, hf, h⟩ := bind_eq_ok h
    obtain ⟨_, ha, h⟩ := bind_eq_ok h
    simp only [ok.injEq] at h; subst h
    have hfle : f.val ≤ amount.val := by simpa [IScalar.le_equiv] using massert_ok ha
    obtain ⟨hh, hhu, hh0, _, hfv⟩ := spec_ok_eq (flash_loan_fee_on_spec env k amount
      (by rw [hkv]; omega) (by rw [hkv]; omega) hamt) hf
    rw [hkv] at hhu hfv
    refine ⟨by rw [hfv]; split <;> omega, hfle, fun h => absurd h (by simp), fun _ => ⟨hh, hhu, hfv⟩⟩

/-- **`ops::strategy::accounting` (strategy open, before the transfer and
events).** Whenever it returns `r` on a market synced to `c` with
`PoolInv c` and `ParamsOk c`:

* `amount ≥ 0`, position `≥ 0`, and in fact `amount > 0` (the debt mint
  requires it); the liquidation-buffer guard passed for the **gross**
  `amount` (INV-ACCT-07: checked before the fee is withheld);
* `r.fee = fee` with `0 ≤ fee ≤ amount`, `fee = 0` without `charge_fee`;
* it minted `minted = ⌈amount·10^k·RAY/borrow_index⌉ > 0` debt shares; the
  returned position is `position + minted` (so `≥ position`),
  `actual_amount = amount`, `amount_received = amount − fee` — exactly
  `create_strategy_summary`; indexes and decimals are the synced market's;
* the committed cache is `c` with `borrowed += minted`, `revenue` and
  `supplied` both `+= s` (`s ≥ 0` protocol-fee shares, `0` for a zero fee;
  INV-ACCT-01 booking, INV-ACCT-10) and `cash -= amount − fee` (the fee
  stays in cash, INV-ACCT-02); `PoolInv` holds.

Hypotheses: `PoolInv c`, `ParamsOk c` (`decimals ≤ 18`, `flashloan_fee ≤
500`), `amount·10^k < 2^127`. -/
theorem strategy_accounting_spec (env : Env) (action : types.pool.PoolAction) (charge : Bool)
    (c : market_cache.Cache) (hsync : ops.synced_market env action.hub_asset = ok c)
    (hinv : PoolInv c) (hpar : ParamsOk c)
    (hA : action.amount.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (r : ops.strategy.StrategyOutcome) (h : ops.strategy.accounting env action charge = ok r) :
    0 < action.amount.val ∧ 0 ≤ action.position.scaled_amount.val ∧
    (∃ u, guards.require_liquidation_buffer env c action.amount = ok u) ∧
    0 ≤ r.fee.val ∧ r.fee.val ≤ action.amount.val ∧ (charge = false → r.fee.val = 0) ∧
    ∃ minted : Int,
      IsCeil (action.amount.val * 10^(27 - c.params.asset_decimals.val) * 10^27)
        c.borrow_index.val minted ∧ 0 < minted ∧
      r.mutation.position.scaled_amount.val = action.position.scaled_amount.val + minted ∧
      r.mutation.actual_amount = action.amount ∧
      r.mutation.amount_received.val = action.amount.val - r.fee.val ∧
      r.mutation.market_index = indexOf c ∧ r.mutation.asset_decimals = c.params.asset_decimals ∧
      PoolInv r.cache ∧
      r.cache.borrowed.val = c.borrowed.val + minted ∧
      (∃ s : Int, 0 ≤ s ∧ (r.fee.val = 0 → s = 0) ∧
        r.cache.revenue.val = c.revenue.val + s ∧ r.cache.supplied.val = c.supplied.val + s) ∧
      r.cache.cash.val = c.cash.val - (action.amount.val - r.fee.val) ∧
      r.cache = { c with borrowed := r.cache.borrowed, revenue := r.cache.revenue,
                         supplied := r.cache.supplied, cash := r.cache.cash } := by
  obtain ⟨hdec, hbps⟩ := hpar
  unfold ops.strategy.accounting at h
  obtain ⟨_, h1, h⟩ := bind_eq_ok h
  obtain ⟨_, h2, h⟩ := bind_eq_ok h
  have hamt := nonneg_of_ok _ _ _ h1
  have hpos := nonneg_of_ok _ _ _ h2
  obtain ⟨c0, hs0, h⟩ := bind_eq_ok h
  have hs0 := renewed_market_ok env _ c0 hs0
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨fee, hfee, h⟩ := bind_eq_ok h
  rw [ray_from_same, bind_ok] at h
  obtain ⟨⟨c1, p1⟩, hmd, h⟩ := bind_eq_ok h
  simp only [market_cache.Cache.impl.params, bind_ok] at h
  obtain ⟨pf, hpf, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  obtain ⟨o, ho, h⟩ := bind_eq_ok h
  obtain ⟨snd, hsnd, h⟩ := bind_eq_ok h
  obtain ⟨c3, hc3, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok, strategy_mutation_eq, bind_ok, ok.injEq] at h
  subst h
  simp only
  obtain ⟨hf0, hfle, hfz, _⟩ := compute_fee_ok env c _ charge hamt (by omega) fee hfee
  obtain ⟨hpos', _, hbuf, minted, hmc, hm0, hmA, hp1, hsh1, hbor1, hc1e, _⟩ :=
    mint_debt_ok env c _ _ hinv hdec hA c1 p1 hmd
  have hpar1 : c1.params = c.params := by rw [hc1e]
  rw [hpar1] at hpf
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - c.params.asset_decimals.val)
  have hFk : fee.val * 10^(27 - c.params.asset_decimals.val) ≤
      action.amount.val * 10^(27 - c.params.asset_decimals.val) :=
    mul_le_mul_of_nonneg_right hfle (by positivity)
  have hpfv := spec_ok_eq (from_asset_spec env fee c.params.asset_decimals (by omega)
    ⟨by nlinarith, by omega⟩) hpf
  have hsi1 : 0 < c1.supply_index.val := by rw [hc1e]; exact hinv.si_pos
  obtain ⟨hsh2, hz, hnz⟩ := spec_ok_eq (add_protocol_revenue_spec c1 pf hsh1 hsi1
    (by rw [hpfv]; positivity)) hc2
  have hsndv := sub_unwrap_eq_ok _ _ (by intro v; simp [
    ops.strategy.accounting.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]) _ _ _ _ ho hsnd
  obtain ⟨hcash3, hcash30, hc3e⟩ := (debit_cash_spec c2 _).2 c3 hc3
  -- the fee shares
  have hs : ∃ s : Int, 0 ≤ s ∧ (fee.val = 0 → s = 0) ∧
      c2.revenue.val = c1.revenue.val + s ∧ c2.supplied.val = c1.supplied.val + s ∧
      c2 = { c1 with revenue := c2.revenue, supplied := c2.supplied } := by
    by_cases hp0 : pf.val = 0
    · have := hz hp0; subst this
      exact ⟨0, le_refl _, fun _ => rfl, by simp, by simp, rfl⟩
    · obtain ⟨s, hs0, _, hr, hsu, hce⟩ := hnz hp0
      refine ⟨s, hs0, fun hf => absurd (by rw [hpfv, hf]; simp) hp0, hr, hsu, hce⟩
  obtain ⟨s, hs0, hsz, hr2, hsu2, hc2e⟩ := hs
  have hpar3 : c3.params = c.params := by rw [hc3e, hc2e, hc1e]
  have hidx : indexOf c3 = indexOf c := by unfold indexOf; rw [hc3e, hc2e, hc1e]
  have hc3full : c3 = { c with borrowed := c3.borrowed, revenue := c3.revenue, supplied := c3.supplied, cash := c3.cash } := by
    rw [hc3e, hc2e, hc1e]
  have hrev3 : c3.revenue = c2.revenue := by rw [hc3e]
  have hsup3 : c3.supplied = c2.supplied := by rw [hc3e]
  have hbor3 : c3.borrowed = c1.borrowed := by rw [hc3e, hc2e]
  refine ⟨hpos', hpos, hbuf, hf0, hfle, hfz, minted, hmc, hm0, hp1, trivial, hsndv, hidx,
    by simp [hpar3], ?_, by rw [hbor3, hbor1], ⟨s, hs0, hsz, ?_, ?_⟩, ?_, hc3full⟩
  · refine ⟨⟨?_, ?_, ?_⟩, hcash30, ?_, ?_, ?_, ?_⟩
    · rw [hrev3]; exact hsh2.1
    · rw [hrev3, hsup3]; exact hsh2.2.1
    · rw [hbor3]; exact hsh1.2.2
    all_goals rw [hc3full]
    · exact hinv.si_lo
    · exact hinv.si_hi
    · exact hinv.bi_lo
    · exact hinv.bi_hi
  · rw [hrev3, hr2, show c1.revenue = c.revenue by rw [hc1e]]
  · rw [hsup3, hsu2, show c1.supplied = c.supplied by rw [hc1e]]
  · rw [hcash3, hsndv, show c2.cash = c.cash by rw [hc2e, hc1e]]

/-! ## `revenue` (claim) -/

/-- **`ops::revenue::accounting` (claim revenue, before the payout
transfer).** Whenever it returns `r` on a market synced to `c` with
`PoolInv c`, with `T = ⌊revenue·supply_index/(RAY·10^k)⌋` the floored
revenue claim and `amt = r.mutation.actual_amount`:
`burn_claimable_revenue` produced `(amt, c1)` with `ClaimResult c T amt c1`
(`0 ≤ amt ≤ cash`, `amt ≤ T`, full payout burns all revenue, a cash-limited
payout burns a ceiling share, a positive payout burns at least one share:
INV-ACCT-06 — so `claim_revenue_summary`'s `amount ≥ 0` holds); the
utilization guard passed on `c1` (INV-ACCT-08); the committed cache is
`c1` with `cash -= amt` (INV-ACCT-02), `PoolInv` holds and not `supplied =
0 ∧ borrowed ≠ 0` (INV-ACCT-09). Hypotheses: `PoolInv c`, `decimals ≤ 27`,
`revenue·supply_index < 2^127·RAY`. -/
theorem revenue_accounting_spec (env : Env) (k : types.pool.HubAssetKey)
    (c : market_cache.Cache) (hsync : ops.synced_market env k = ok c) (hinv : PoolInv c)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hfit : c.revenue.val * c.supply_index.val < 2^127 * 10^27)
    (r : ops.revenue.RevenueOutcome) (h : ops.revenue.accounting env k = ok r) :
    ∃ (T : Int) (c1 : market_cache.Cache),
      IsFloor (c.revenue.val * c.supply_index.val) (10^27 * 10^(27 - c.params.asset_decimals.val)) T ∧
      ClaimResult c T r.mutation.actual_amount.val c1 ∧
      0 ≤ r.mutation.actual_amount.val ∧ r.mutation.actual_amount.val ≤ c.cash.val ∧
      (∃ u, guards.require_utilization_below_max env c1 = ok u) ∧
      r.cache.cash.val = c1.cash.val - r.mutation.actual_amount.val ∧
      r.cache = { c1 with cash := r.cache.cash } ∧ PoolInv r.cache ∧
      ¬ (r.cache.supplied.val = 0 ∧ r.cache.borrowed.val ≠ 0) := by
  unfold ops.revenue.accounting at h
  obtain ⟨c0, hs0, h⟩ := bind_eq_ok h
  have hs0 := renewed_market_ok env k c0 hs0
  rw [hsync] at hs0; simp only [ok.injEq] at hs0; subst hs0
  obtain ⟨⟨nt, c1⟩, hb, h⟩ := bind_eq_ok h
  obtain ⟨u1, hu, h⟩ := bind_eq_ok h
  obtain ⟨u2, hsd, h⟩ := bind_eq_ok h
  obtain ⟨c2, hc2, h⟩ := bind_eq_ok h
  rw [commit_eq, bind_ok, ok.injEq] at h
  subst h
  simp only
  obtain ⟨T, hT, hcr⟩ := spec_ok_eq (burn_claimable_revenue_spec c hinv.si_pos hinv.shares.1
    hinv.cash hinv.shares.2.1 hdec hfit) hb
  simp only at hcr
  have hsh1 := spec_ok_eq (burn_claimable_revenue_preserves c hinv.shares hinv.si_pos hinv.cash
    hdec hfit) hb
  simp only at hsh1
  have h09 := (require_supply_for_debt_spec env c1).1 ⟨u2, hsd⟩
  obtain ⟨hcash2, hc20, hc2e⟩ := (debit_cash_spec c1 _).2 c2 hc2
  obtain ⟨hle, _, h0, _, _, hz, hp⟩ := hcr
  -- c1 differs from c only in revenue and supplied
  have hc1 : c1 = { c with revenue := c1.revenue, supplied := c1.supplied } := by
    rcases lt_or_eq_of_le h0 with hpos | he
    · obtain ⟨_, _, _, _, _, hce, _⟩ := hp hpos; exact hce
    · rw [hz he.symm]
  refine ⟨T, c1, hT, ⟨hle, by omega, h0, by assumption, by assumption, hz, hp⟩, h0, hle,
    ⟨u1, hu⟩, hcash2, hc2e, ?_, by rw [hc2e]; exact h09⟩
  refine ⟨by rw [hc2e]; exact hsh1, hc20, ?_, ?_, ?_, ?_⟩ <;> rw [hc2e, hc1]
  · exact hinv.si_lo
  · exact hinv.si_hi
  · exact hinv.bi_lo
  · exact hinv.bi_hi

/-! ## Load and accrual (`Cache::load`, `ops::synced_market`) -/

/-- **`Cache::load` copies the stored state.** If storage holds `st` for the
key and `Cache::load` returns `c0`, the share totals, indexes, cash and
last-accrual timestamp of `c0` are exactly those of `st`. So `PoolInv c0` is
the stored-state invariant on `st`, and each leg's committed `snapOf c'`
(which `commit` writes back) carries `PoolInv c'` into the next load. -/
theorem load_ok (env : Env) (k : types.pool.HubAssetKey) (st : types.pool.PoolStateRaw)
    (c0 : market_cache.Cache) (hst : storedState k = ok st)
    (h : market_cache.Cache.load env k = ok c0) :
    c0.supplied = st.supplied ∧ c0.borrowed = st.borrowed ∧ c0.revenue = st.revenue ∧
    c0.borrow_index = st.borrow_index ∧ c0.supply_index = st.supply_index ∧
    c0.cash = st.cash ∧ c0.last_timestamp = st.last_timestamp := by
  unfold market_cache.Cache.load at h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨st', hst', h⟩ := bind_eq_ok h
  simp only [storage.read_state, hst, ok.injEq] at hst'; subst hst'
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨ps, hps, h⟩ := bind_eq_ok h
  unfold types.pool.PoolState.Insts.CoreConvertFromShared0PoolStateRaw.from at hps
  simp only [ray_from_same, bind_ok, ok.injEq] at hps
  subst hps
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  obtain ⟨_, _, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h
  subst h
  exact ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

/-- **`ops::synced_market` preserves the stored-state invariant (one
accrual chunk).** If the market loads to `c0` with `PoolInv c0`, in the
accrual domain (`AccrualDomain c0`) and with at most one year elapsed, then
`synced_market` returns (does not abort) a cache `c` with `PoolInv c`, the
same cash and params, and indexes no lower than before (INV-IDX-01/02).
Together with the leg theorems (each leg's committed cache satisfies
`PoolInv`), the stored-state invariant is inductive over every leg. -/
theorem synced_market_spec (env : Env) (k : types.pool.HubAssetKey) (c0 : market_cache.Cache)
    (hload : market_cache.Cache.load env k = ok c0) (hinv : PoolInv c0) (hD : AccrualDomain c0)
    (he : c0.current_timestamp.val - c0.last_timestamp.val ≤ 31556926000) :
    ops.synced_market env k ⦃ c => PoolInv c ∧ c.cash = c0.cash ∧ c.params = c0.params ∧
      c0.borrow_index.val ≤ c.borrow_index.val ∧ c0.supply_index.val ≤ c.supply_index.val ⦄ := by
  unfold ops.synced_market
  rw [hload, bind_ok]
  apply WP.spec_mono (global_sync_one_chunk_spec env c0 hD he)
  rintro c (⟨_, rfl⟩ | ⟨_, _, x, hx, rfl⟩)
  · exact ⟨hinv, rfl, rfl, le_refl _, le_refl _⟩
  · obtain ⟨hsh, hb1, hb2, hs1, hs2, sh, _, _, _, hxe⟩ := hx
    have hcash : x.cash = c0.cash := by rw [hxe]
    have hpar : x.params = c0.params := by rw [hxe]
    refine ⟨⟨hsh, by simp [hcash]; exact hinv.cash, by simp; have := hinv.si_lo; omega,
      by simpa using hs2, by simp; have := hinv.bi_lo; omega, by simpa using hb2⟩,
      by simp [hcash], by simp [hpar], by simpa using hb1, by simpa using hs1⟩

end PoolAeneas.Proofs
