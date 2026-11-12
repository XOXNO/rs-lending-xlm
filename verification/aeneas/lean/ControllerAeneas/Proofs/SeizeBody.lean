/-
Total correctness of one iteration of the generated loop of
`calculate_seized_collateral`: it returns the exact closed forms of
`SeizeLeg.lean`.

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.SeizeLeg

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 8000000

namespace ControllerAeneas.Proofs

/-- The integer inputs of the loop iteration on supply leg `e`, read from the
position, the context and the loop's fixed parameters. -/
noncomputable def legOf (ctx : context.Context) (tc tsu opb : math.fp.Wad) (rad sa : Bool)
    (e : types.HubAssetKey × types.AccountPosition) : SzLeg where
  s := e.2.scaled_amount.val
  i := supplyIndexOf ctx e.1
  p := priceOf ctx e.1.asset
  dec := decOf ctx e.1.asset
  f := e.2.liquidation_fees.val
  C := tc.val
  B := tsu.val
  opb := opb.val
  rad := rad
  sa := sa

/-- The entry pushed for leg `e`: every field is the closed form of
`SeizeLeg.lean`, and the feed / index snapshot is the context's. -/
def EntryOf (ctx : context.Context) (k : types.HubAssetKey) (g : SzLeg) (e : types.SeizeEntry) : Prop :=
  e.hub_asset = k ∧ e.amount.val = g.Amt ∧ e.protocol_fee.val = g.PFee ∧
  e.scaled_amount.val = g.Seized ∧ e.bonus_scaled.val = g.Bsc ∧
  (e.liquidation_fees.val : Int) = g.f ∧ e.feed.price_wad.val = g.p ∧
  e.feed.asset_decimals.val = g.dec ∧ e.market_index.supply_index.val = g.i

theorem sz_ite_ok {α : Type} (c : Prop) [Decidable c] (a b : α) :
    (if c then (ok a : Result α) else ok b) = ok (if c then a else b) := by
  split <;> rfl

theorem sz_lift_min (a b : Std.I128) :
    lift (core.cmp.impls.OrdI128.min a b) = ok (core.cmp.impls.OrdI128.min a b) := rfl

theorem sz_next_cons {T : Type} (x : T) (l : List T) :
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next (x :: l)
      = ok (some x, l) := rfl

/-- **One loop iteration of `calculate_seized_collateral`.** On a supply leg
`(k, pos)` served by the context and in the production domain `SzLegOK`, the
body returns (does not abort) `continue` with the rest of the legs, the
context unchanged, `unseized_usd` increased by exactly `Uns`, and the seizure
list extended by one entry `EntryOf` when `Emit` holds (unchanged otherwise).
Extra hypothesis: the running `unseized_usd` plus this leg's contribution
fits `i128` (it is at most the sum of the legs' budgets). -/
theorem sz_body_spec (env : Env) (tc opb tsu : math.fp.Wad) (rad sa : Bool)
    (k : types.HubAssetKey) (pos : types.AccountPosition)
    (rest : List (types.HubAssetKey × types.AccountPosition)) (ctx : context.Context)
    (seized : List types.SeizeEntry) (uns : math.fp.Wad)
    (hserve : Serves ctx k) (hok : SzLegOK (legOf ctx tc tsu opb rad sa (k, pos)))
    (huns0 : 0 ≤ uns.val)
    (huns : uns.val + (legOf ctx tc tsu opb rad sa (k, pos)).Uns < 2^127) :
    positions.liquidation.math.calculate_seized_collateral_loop.body env tc rad sa opb tsu
      ((k, pos) :: rest) ctx seized uns ⦃ r => ∃ seized' uns',
        r = ControlFlow.cont (rest, ctx, seized', uns') ∧
        uns'.val = uns.val + (legOf ctx tc tsu opb rad sa (k, pos)).Uns ∧
        ((legOf ctx tc tsu opb rad sa (k, pos)).Emit →
          ∃ e, seized' = seized ++ [e] ∧ EntryOf ctx k (legOf ctx tc tsu opb rad sa (k, pos)) e) ∧
        (¬ (legOf ctx tc tsu opb rad sa (k, pos)).Emit → seized' = seized) ⦄ := by
  obtain ⟨⟨pr, hpr⟩, ⟨ir, hir⟩⟩ := hserve
  set g := legOf ctx tc tsu opb rad sa (k, pos) with hg
  have gs : g.s = pos.scaled_amount.val := rfl
  have gi : g.i = ir.supply_index.val := by simp [hg, legOf, supplyIndexOf, hir]
  have gp : g.p = pr.price_wad.val := by simp [hg, legOf, priceOf, hpr]
  have gd : g.dec = pr.asset_decimals.val := by simp [hg, legOf, decOf, hpr]
  have gf : g.f = pos.liquidation_fees.val := rfl
  have gC : g.C = tc.val := rfl
  have gB : g.B = tsu.val := rfl
  have go : g.opb = opb.val := rfl
  have grad : g.rad = rad := rfl
  have gsa : g.sa = sa := rfl
  unfold positions.liquidation.math.calculate_seized_collateral_loop.body
  simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
  simp [cached_price_ok ctx k.asset pr hpr, cached_market_index_ok ctx k ir hir]
  have hD := hok.Dd_pos
  have hdec : (pr.asset_decimals.val : Nat) ≤ 18 := by rw [← gd]; exact hok.dec18
  have hDd : g.Dd = 10 ^ (27 - pr.asset_decimals.val) := by simp only [SzLeg.Dd, gd]
  have vd : VD pos.scaled_amount.val ir.supply_index.val pr.price_wad.val := by
    rw [← gs, ← gi, ← gp]; exact hok.vd
  have hA : huD (pos.scaled_amount.val * ir.supply_index.val) (10^27) = g.A := by
    simp only [SzLeg.A, gs, gi]
  step with sz_ray_mul_val env pos.scaled_amount ir.supply_index hok.s0 (by rw [← gi]; exact hok.i0.le)
    (by rw [hA]; have := hok.AB; omega) as ⟨A, hAv⟩
  rw [hA] at hAv
  step with pv_val env pos.scaled_amount ir.supply_index pr.price_wad vd as ⟨v, hv⟩
  have hv' : v.val = g.V := by rw [hv]; simp only [SzLeg.V, gs, gi, gp]
  step with sz_wad_div_val env v tc (by rw [hv']; exact hok.V0) (by rw [← gC]; exact hok.C0)
    (by rw [hv', ← gC]; have := hok.ShW; unfold SzLeg.Sh at this; omega) as ⟨sh, hsh⟩
  have hsh' : sh.val = g.Sh := by rw [hsh, hv', ← gC]; rfl
  step with sz_wad_mul_val env tsu sh (by rw [← gB]; exact hok.B0) (by rw [hsh']; exact hok.Sh0)
    (by rw [hsh', ← gB]; have := hok.LB; have := hok.B0; unfold SzLeg.L at *;
        have := tsu.hBounds; simp only [IScalarTy.numBits] at this; omega) as ⟨L, hL⟩
  have hL' : L.val = g.L := by rw [hL, hsh', ← gB]; rfl
  step with sz_wad_div_val env L pr.price_wad (by rw [hL']; exact hok.L0) (by rw [← gp]; exact hok.p0)
    (by rw [hL']; have := hok.QB; unfold SzLeg.Q at this; rw [gp] at this; omega) as ⟨q, hq⟩
  have hq' : q.val = g.Q := by rw [hq, hL', ← gp]; rfl
  simp only [math.fp.Wad.raw, bind_ok]
  step with sz_ray_div_wad as ⟨i1, hi1⟩
  step with sz_sat_mul_val q i1 (by rw [hi1, hq']; have := hok.Q0; positivity) as ⟨S0, hS0⟩
  have hS0' : S0.val = g.S0 := by rw [hS0, hi1, hq']; rfl
  have hWU : g.WU ↔ (sa = false ∧ pr.asset_decimals.val < 3 ∧ S0.val < A.val) := by
    unfold SzLeg.WU; rw [gsa, gd, hS0', hAv]
  have hS00 := hok.S00
  have hAB := hok.AB
  have hA0 := hok.A0
  refine WP.spec_bind (Pₘ := fun t => t.1 = ctx ∧ t.2.2.1 = rest ∧ t.2.1.val = uns.val + g.Uns ∧
    t.2.2.2.1.val = g.A1 ∧ t.2.2.2.2.val = g.S1) ?_ ?_
  · by_cases hsa : sa = true
    · have hw : ¬ g.WU := fun c => by rw [hWU, hsa] at c; exact absurd c.1 (by simp)
      simp only [hsa, ↓reduceIte, WP.spec_ok]
      exact ⟨trivial, trivial, by rw [hok.nWU_Uns hw]; ring, by rw [hok.nWU_A1 hw, hq'],
        by rw [hok.nWU_S1 hw, hS0']⟩
    · have hsa' : sa = false := by simpa using hsa
      simp only [hsa, Bool.false_eq_true, ↓reduceIte, sz_min_dec_val]
      by_cases hd3 : pr.asset_decimals.val < 3
      · simp only [hd3, ↓reduceIte]
        by_cases hlt : S0.val < A.val
        · have hw : g.WU := hWU.mpr ⟨hsa', hd3, hlt⟩
          simp only [hlt, ↓reduceIte]
          by_cases hr : rad = true
          · simp only [hr, ↓reduceIte]
            step with sz_to_asset_ceil_val env S0 pr.asset_decimals (by omega) as ⟨w, hwv⟩
            have hcl := sz_clD_bounds S0.val g.Dd hD
            have hcl0 := sz_clD_nonneg S0.val g.Dd hD (by omega)
            have hDle := hok.Dd_le
            rw [← hDd] at hwv
            step with sz_from_asset_val env w pr.asset_decimals (by omega)
              ⟨by rw [← hDd, hwv]; nlinarith, by rw [← hDd, hwv]; nlinarith⟩ as ⟨r, hr'⟩
            rw [← hDd, hwv] at hr'
            step with sz_to_wad_val env (if A.val < r.val then A else r) (by split <;> omega) as ⟨a2, ha2⟩
            have hS1 : g.S1 = min (clD S0.val g.Dd * g.Dd) A.val := by
              unfold SzLeg.S1; rw [if_pos hw, grad, if_pos hr, hS0', hAv]
            have hmin : (if A.val < r.val then A else r).val = g.S1 := by
              rw [hS1, ← hr']; split <;> rename_i c <;> simp [min_def] <;> omega
            refine ⟨?_, ?_, hmin⟩
            · unfold SzLeg.Uns; rw [if_neg (by rw [grad, hr]; simp)]; ring
            · rw [hok.WU_A1 hw, ← hmin, ha2]
          · have hr' : rad = false := by simpa using hr
            simp only [hr', Bool.false_eq_true, ↓reduceIte]
            step with sz_to_asset_floor_val env S0 pr.asset_decimals (by omega) (by omega) as ⟨w, hwv⟩
            rw [← hDd] at hwv
            have hfl := sz_div_bounds S0.val g.Dd hD
            have hfl0 := sz_div_nonneg S0.val g.Dd hD (by omega)
            step with sz_from_asset_val env w pr.asset_decimals (by omega)
              ⟨by rw [← hDd, hwv]; nlinarith, (by
                rw [← hDd, hwv]; have := S0.hBounds
                simp only [IScalarTy.numBits] at this; nlinarith)⟩ as ⟨r, hrv⟩
            rw [← hDd, hwv] at hrv
            have hS1 : g.S1 = S0.val / g.Dd * g.Dd := by
              unfold SzLeg.S1; rw [if_pos hw, grad, if_neg (by rw [hr']; simp), hS0']
            step with sz_to_wad_val env r (by rw [hrv]; exact mul_nonneg hfl0 hD.le) as ⟨a2, ha2⟩
            have hA1 : a2.val = g.A1 := by rw [hok.WU_A1 hw, ha2, hrv, hS1]
            have ha20 : 0 ≤ a2.val := by rw [hA1]; exact hok.A1_0
            have hfit := hok.wusd_fit r.val (by rw [hrv]; exact mul_nonneg hfl0 hD.le)
              (by rw [hrv]; omega)
            rw [← ha2, gp] at hfit
            step with sz_wad_mul_val env a2 pr.price_wad ha20 (by rw [← gp]; exact hok.p0.le)
              (by omega) as ⟨wu, hwu⟩
            have hwu' : wu.val = g.WUsd := by rw [hwu, ← gp, hA1]; rfl
            have hwu0 : 0 ≤ wu.val := by
              rw [hwu]; exact sz_huD_nonneg _ _ (by norm_num) (mul_nonneg ha20 (by rw [← gp]; exact hok.p0.le))
            refine WP.spec_bind (Pₘ := fun w => w.val = uns.val + g.Uns) ?_ ?_
            · by_cases hc : wu.val < L.val
              · simp only [hc, ↓reduceIte]
                have hU : g.Uns = L.val - wu.val := by
                  unfold SzLeg.Uns; rw [if_pos ⟨hw, by rw [grad, hr']⟩, if_pos (by rw [← hwu', ← hL']; exact hc),
                    hwu', hL']
                step with sz_wad_checked_sub_val env L wu hwu0 (by omega) as ⟨w1, hw1⟩
                step with sz_wad_checked_add_val env uns w1 ⟨by omega, by rw [hw1, ← hU]; exact huns⟩
                  as ⟨w2, hw2⟩
                rw [hw2, hw1, hU]
              · simp only [hc, ↓reduceIte, WP.spec_ok]
                unfold SzLeg.Uns; rw [if_pos ⟨hw, by rw [grad, hr']⟩, if_neg (by rw [← hwu', ← hL']; exact hc)]
                ring
            · intro w hw'
              simp only [WP.spec_ok]
              exact ⟨trivial, trivial, hw', hA1, by rw [hrv, hS1]⟩
        · have hw : ¬ g.WU := fun c => hlt (hWU.mp c).2.2
          simp only [hlt, ↓reduceIte, WP.spec_ok]
          exact ⟨trivial, trivial, by rw [hok.nWU_Uns hw]; ring, by rw [hok.nWU_A1 hw, hq'],
            by rw [hok.nWU_S1 hw, hS0']⟩
      · have hw : ¬ g.WU := fun c => hd3 (hWU.mp c).2.1
        simp only [hd3, ↓reduceIte, WP.spec_ok]
        exact ⟨trivial, trivial, by rw [hok.nWU_Uns hw]; ring, by rw [hok.nWU_A1 hw, hq'],
          by rw [hok.nWU_S1 hw, hS0']⟩
  rintro ⟨c3, u1, it2, a1, s1⟩ ⟨hc3, hit2, hu1, ha1, hs1⟩
  simp only at hc3 hit2 hu1 ha1 hs1
  subst hc3 hit2
  simp [sz_ray_zero_val]
  have hS1v : s1.val = g.S1 := hs1
  by_cases hz1 : s1.val ≤ 0
  · simp only [hz1, ↓reduceIte, WP.spec_ok]
    refine ⟨seized, u1, rfl, hu1, fun he => absurd he.1 (by omega), fun _ => rfl⟩
  simp only [hz1, ↓reduceIte, sz_ite_ok, bind_ok]
  generalize hcapd : (if sa = true then A else if A.val < s1.val then A else s1) = cap
  have hcap : cap.val = g.Capped := by
    rw [← hcapd]; unfold SzLeg.Capped; rw [gsa, ← hS1v, ← hAv]
    split_ifs <;> simp_all [min_def] <;> omega
  have hC0 := hok.Capped0
  have hCA := hok.CappedA
  by_cases hz2 : cap.val ≤ 0
  · simp only [hz2, ↓reduceIte, WP.spec_ok]
    refine ⟨seized, u1, rfl, hu1, fun he => absurd he.2.1 (by omega), fun _ => rfl⟩
  simp only [hz2, ↓reduceIte]
  have hA10 := hok.A1_0
  have hA1B := hok.A1B
  have hopb := hok.opbW
  have hbase0 : 0 ≤ a1.val * 10^18 / opb.val := by rw [ha1, ← go]; exact sz_div_nonneg _ _ (by omega) (by positivity)
  have hbaseB : a1.val * 10^18 / opb.val ≤ a1.val := by
    rw [ha1, ← go]; exact sz_div_le_of _ _ _ (by omega) (by nlinarith)
  step with sz_wad_div_floor_val env a1 opb (by rw [← go]; omega) ⟨by omega, by omega⟩ as ⟨i3, hi3⟩
  step with sz_sat_mul_val i3 i1 (by rw [hi3, hi1]; positivity) as ⟨i4, hi4⟩
  have hbase : i4.val = g.Base := by rw [hi4, hi3, hi1, ha1, ← go]; rfl
  have hbase0' := hok.Base0
  refine WP.spec_bind (Pₘ := fun b => b.val = g.Bonus) ?_ ?_
  · by_cases hc : i4.val < cap.val
    · simp only [hc, ↓reduceIte]
      step with sz_ray_checked_sub_val env cap i4 (by omega) (by omega) as ⟨b, hb⟩
      rw [hb]; unfold SzLeg.Bonus; rw [← hbase, ← hcap, if_pos hc]
    · simp only [hc, ↓reduceIte, WP.spec_ok]
      unfold SzLeg.Bonus; rw [← hbase, ← hcap, if_neg hc, sz_ray_zero_val]
  intro bon hbon
  have hB0 := hok.Bonus0
  have hBC := hok.BonusC
  step with sz_apply_to_ray_val env pos.liquidation_fees bon (by rw [← gf]; exact hok.f0)
    (by rw [← gf]; exact hok.fB) (by omega) as ⟨fr, hfr, hfr0, hfrB⟩
  have hfr' : fr.val = g.FeeR := by rw [hfr, hbon, ← gf]; rfl
  have hsB := hok.sB
  refine WP.spec_bind (Pₘ := fun x => x.val = g.Seized) ?_ ?_
  · by_cases hc : cap.val = A.val
    · simp only [hc, ↓reduceIte, WP.spec_ok]
      unfold SzLeg.Seized; rw [← hcap, ← hAv, if_pos hc, gs]
    · simp only [hc, ↓reduceIte]
      have hlt : g.Capped < g.A := by rw [← hcap, ← hAv]; omega
      have := hok.part_le g.Capped hlt
      have h0 : 0 ≤ g.Capped * 10^27 / g.i := sz_div_nonneg _ _ hok.i0 (by positivity)
      rw [gi, ← hcap] at this h0
      step with sz_ray_div_floor_val env cap ir.supply_index (by rw [← gi]; exact hok.i0)
        ⟨by omega, by rw [← gs] at *; omega⟩ as ⟨x, hx⟩
      rw [hx]; unfold SzLeg.Seized; rw [← hcap, ← hAv, if_neg hc, gi]
  intro ss hss
  have hdf := hok.div_fit g.Bonus (by omega)
  have hdf0 : 0 ≤ g.Bonus * 10^27 / g.i := sz_div_nonneg _ _ hok.i0 (by positivity)
  rw [gi, ← hbon] at hdf hdf0
  step with sz_ray_div_floor_val env bon ir.supply_index (by rw [← gi]; exact hok.i0)
    ⟨by omega, by rw [← gs] at *; omega⟩ as ⟨rb, hrb⟩
  by_cases hz3 : ss.val ≤ 0
  · simp only [hz3, ↓reduceIte, WP.spec_ok]
    refine ⟨seized, u1, rfl, hu1, fun he => absurd he.2.2.1 (by omega), fun _ => rfl⟩
  simp only [hz3, ↓reduceIte]
  have hdlt : pr.asset_decimals.val < 27 := by omega
  refine WP.spec_bind (Pₘ := fun x => x.val = g.Amt) ?_ ?_
  · by_cases hc : cap.val = A.val
    · simp only [hc, ↓reduceIte]
      step with sz_to_asset_val env cap pr.asset_decimals hdlt (by omega) as ⟨x, hx⟩
      rw [hx]; unfold SzLeg.Amt; rw [← hcap, ← hAv, if_pos hc, hDd]
    · simp only [hc, ↓reduceIte]
      step with sz_to_asset_floor_val env cap pr.asset_decimals (by omega) (by omega) as ⟨x, hx⟩
      rw [hx]; unfold SzLeg.Amt; rw [← hcap, ← hAv, if_neg hc, hDd]
  intro amt hamt
  by_cases hz4 : amt.val ≤ 0
  · simp only [hz4, ↓reduceIte, WP.spec_ok]
    refine ⟨seized, u1, rfl, hu1, fun he => absurd he.2.2.2 (by omega), fun _ => rfl⟩
  simp only [hz4, ↓reduceIte]
  have hemit : g.Emit := ⟨by omega, by omega, by omega, by omega⟩
  step with sz_resolve_withdrawal_val env amt pos.scaled_amount ir.supply_index pr.asset_decimals
    hdlt (by rw [← gi]; exact hok.i0) hok.s0 (by omega)
    (by rw [hA]; have := hok.AB; omega) as ⟨bu, pg, hpg, hpg0, hpga, hpgD⟩
  have hpg' : pg.val = g.PG := by
    rw [hpg]; unfold SzLeg.PG; rw [hA, ← hDd, hamt, gs, gi]
  step with sz_to_asset_floor_val env fr pr.asset_decimals (by omega) hfr0 as ⟨fa, hfa⟩
  have hfa' : fa.val = g.FeeA := by rw [hfa, hfr', ← hDd]; rfl
  have hsiR : pos.scaled_amount.val * ir.supply_index.val / 10^27 ≤ 2^126 := by
    have := hok.siB; rw [gs, gi] at this
    exact sz_div_le_of _ _ _ (by norm_num) (by linarith)
  step with sz_from_asset_val env pg pr.asset_decimals hdlt
    ⟨by have := pow_pos (show (0:Int) < 10 by norm_num) (27 - pr.asset_decimals.val); nlinarith,
     by omega⟩ as ⟨pd, hpd⟩
  rw [← hDd] at hpd
  refine WP.spec_bind (Pₘ := fun x => x.val = g.Real) ?_ ?_
  · by_cases hc : i4.val < pd.val
    · simp only [hc, ↓reduceIte]
      step with sz_ray_checked_sub_val env pd i4 (by omega) (by omega) as ⟨r1, hr1⟩
      step with sz_to_asset_floor_val env r1 pr.asset_decimals (by omega) (by omega) as ⟨x, hx⟩
      rw [hx, hr1, hpd, ← hDd]; unfold SzLeg.Real; rw [← hbase, ← hpg', if_pos (by rw [← hpd]; exact hc)]
    · simp only [hc, ↓reduceIte, WP.spec_ok]
      unfold SzLeg.Real; rw [← hbase, ← hpg', if_neg (by rw [← hpd]; exact hc)]; rfl
  intro re hre
  simp only [sz_lift_min, bind_ok, math.fp.Ray.raw, math.fp.Bps.raw]
  have hf0 := hok.f0; have hfB := hok.fB
  rw [gf] at hf0 hfB
  step with IScalar.hcast_inBounds_spec .U32 pos.liquidation_fees
    ⟨hf0, by simp only [UScalar.max, UScalarTy.numBits]; omega⟩ as ⟨fu, hfu⟩
  simp only [types.PriceFeedRaw.Insts.CoreConvertFromShared0PriceFeed.from,
    types.MarketIndexRaw.Insts.CoreConvertFromShared0MarketIndex.from, math.fp.Wad.raw,
    math.fp.Ray.raw, bind_ok, aeneas_soroban_shim.Vec.push_back, WP.spec_ok]
  refine ⟨_, u1, rfl, hu1, fun _ => ⟨_, rfl, ?_⟩, fun h => absurd hemit h⟩
  refine ⟨rfl, hamt, ?_, hss, ?_, by rw [hfu, gf], by rw [gp], by rw [gd], by rw [gi]⟩
  · rw [sz_i128_min_val, hre]
    unfold SzLeg.PFee SzLeg.Bumped
    congr 1
    rw [← hfr', ← hfa']
    have e : (fa = 0#i128) ↔ fa.val = 0 := by
      constructor
      · intro h; rw [h]; rfl
      · intro h; exact IScalar.eq_of_val_eq (by rw [h]; rfl)
    by_cases c1 : 0 < fr.val
    · by_cases c2 : fa.val = 0
      · have : fa = 0#i128 := e.mpr c2
        simp [c1, c2, this]
      · have : ¬ fa = 0#i128 := fun h => c2 (e.mp h)
        simp [c1, c2, this]
    · simp [c1]
  · have hr' : rb.val = g.Bonus * 10^27 / g.i := by rw [hrb, hbon, gi]
    unfold SzLeg.Bsc; rw [← hss, ← hr']
    by_cases c : ss.val < rb.val
    · rw [if_pos c, min_eq_right c.le]
    · rw [if_neg c, min_eq_left (by omega)]

end ControllerAeneas.Proofs
