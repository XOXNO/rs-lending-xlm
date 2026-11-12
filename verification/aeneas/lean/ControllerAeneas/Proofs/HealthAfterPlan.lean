/-
**The health factor after the plan `normalize_repayment_plan` produced.**

`health_after_hf_preserving` (HealthAfter.lean) is stated for any plan whose
per-debt-leg credited USD `uOf` sums to `repay_usd`. This file discharges
that hypothesis, and `0 ≤ repay_usd`, from the production plan: with
`res = normalize_repayment_plan …` and `repayment_plan_spec` (RepayPlan.lean),
crediting each debt leg of the account the `usd_wad` of the repaid entries
with its key (`planU`) sums to exactly `res.repay_usd`, provided the debt
map's keys are distinct (a Soroban `Map`). That every repaid entry names one
of the account's debt legs is part of `RepayLegOK` and is not assumed again.

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.HealthAfter
import ControllerAeneas.Proofs.RepayPlan

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

section Regroup

variable {K V X : Type} [DecidableEq K]

/-- A sum over a map of "`c` at key `k`, else 0" is 0 when `k` is not a key. -/
theorem lsum_ite_key_absent (D : List (K × V)) (k : K) (hk : k ∉ D.map Prod.fst) (c : Int) :
    lsum (fun e => if k = e.1 then c else 0) D = 0 := by
  induction D with
  | nil => simp [lsum]
  | cons e D ih =>
    simp only [List.map_cons, List.mem_cons, not_or] at hk
    have h1 : ¬ k = e.1 := hk.1
    have := ih hk.2
    simp only [lsum, List.map_cons, List.sum_cons, h1, ↓reduceIte] at this ⊢
    simpa using this

/-- A sum over a map with distinct keys of "`c` at key `k`, else 0" is `c`
when `k` is a key. -/
theorem lsum_ite_key (D : List (K × V)) (hD : (D.map Prod.fst).Nodup) (k : K)
    (hk : k ∈ D.map Prod.fst) (c : Int) :
    lsum (fun e => if k = e.1 then c else 0) D = c := by
  induction D with
  | nil => simp at hk
  | cons e D ih =>
    simp only [List.map_cons, List.nodup_cons] at hD
    simp only [List.map_cons, List.mem_cons] at hk
    by_cases h : k = e.1
    · have h0 := lsum_ite_key_absent D k (by rw [h]; exact hD.1) c
      simp only [lsum, List.map_cons, List.sum_cons, h, ↓reduceIte] at h0 ⊢
      rw [h0]; simp
    · have hk' : k ∈ D.map Prod.fst := by
        rcases hk with hk | hk
        · exact absurd hk h
        · exact hk
      have := ih hD.2 hk'
      simp only [lsum, List.map_cons, List.sum_cons, h, ↓reduceIte] at this ⊢
      simpa using this

omit [DecidableEq K] in
/-- `lsum` distributes over a pointwise sum. -/
theorem lsum_add' (f g : (K × V) → Int) (D : List (K × V)) :
    lsum (fun e => f e + g e) D = lsum f D + lsum g D := by
  induction D with
  | nil => simp [lsum]
  | cons e D ih =>
    simp only [lsum, List.map_cons, List.sum_cons] at ih ⊢
    rw [ih]; ring

/-- **Regrouping a sum by key.** If every item's key `f x` is a key of a map
`D` with distinct keys, summing `u` over the items equals summing, over the
map, the `u`-total of the items with that key. -/
theorem sum_regroup (D : List (K × V)) (hD : (D.map Prod.fst).Nodup)
    (f : X → K) (u : X → Int) (R : List X) (hR : ∀ x ∈ R, f x ∈ D.map Prod.fst) :
    (R.map u).sum = lsum (fun e => ((R.filter (fun x => f x = e.1)).map u).sum) D := by
  induction R with
  | nil =>
    simp only [List.map_nil, List.sum_nil, List.filter_nil]
    clear hD hR
    induction D with
    | nil => simp [lsum]
    | cons e D ih => simp only [lsum, List.map_cons, List.sum_cons] at ih ⊢; omega
  | cons x R ih =>
    have hx := hR x (by simp)
    have ih' := ih (fun y hy => hR y (by simp [hy]))
    have hsplit : (fun e : K × V => (((x :: R).filter (fun y => f y = e.1)).map u).sum) =
        (fun e => (if f x = e.1 then u x else 0) +
          ((R.filter (fun y => f y = e.1)).map u).sum) := by
      funext e
      by_cases h : f x = e.1
      · simp [h]
      · simp [h]
    rw [hsplit, lsum_add', lsum_ite_key D hD (f x) hx, ← ih']
    simp

end Regroup

section Plan

attribute [local instance] Classical.propDecidable

/-- A key that `mapLookup` finds is one of the map's keys. -/
theorem mapLookup_mem_keys {K V : Type} {m : List (K × V)} {k : K} {v : V}
    (h : mapLookup m k = some v) : k ∈ m.map Prod.fst := by
  unfold mapLookup at h
  obtain ⟨e, he, -⟩ := Option.map_eq_some_iff.mp h
  have hk : e.1 = k := by simpa using List.find?_some he
  exact List.mem_map.mpr ⟨e, List.mem_of_find?_eq_some he, hk⟩

/-- USD credited to debt key `k` by a plan: the `usd_wad` of its repaid
entries for `k`. -/
noncomputable def planU (p : positions.liquidation.math.NormalizedRepaymentPlan)
    (k : types.HubAssetKey) : Int :=
  usdSum (p.repaid.filter (fun x => x.hub_asset = k))

/-- Tokens repaid on debt key `k` by a plan: the `amount` of its repaid
entries for `k`. -/
noncomputable def planA (p : positions.liquidation.math.NormalizedRepaymentPlan)
    (k : types.HubAssetKey) : Int :=
  amtSum (p.repaid.filter (fun x => x.hub_asset = k))

/-- **The per-leg credits of the production plan sum to `repay_usd`.** For
the plan returned by `normalize_repayment_plan` for account `acc` (domain of
`repayment_plan_spec`), if the keys of the account's debt map are distinct
(they are in a Soroban `Map`), then `0 ≤ repay_usd` and `repay_usd` equals
the sum over the account's debt legs of `planU`. These are hypotheses `hr0`
and `hrep` of `health_after_hf_preserving` / `health_after_target`. Every
repaid entry names a debt leg of the account because `RepayLegOK` requires
the position to exist (production aborts with `DebtPositionNotFound`). -/
theorem plan_credits_sum (env : Env) (acc : types.Account)
    (raw : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128))
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bb : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve) (ctx : context.Context)
    (P : List (types.HubAssetKey × Std.I128))
    (hagg : payment_legs.aggregate_positive_payments env raw = ok P)
    (hok : ∀ l ∈ P, RepayLegOK ctx acc l) (htrim : ∀ l ∈ P, RepayLegTrimOK ctx acc l)
    (hlen : P.length < 2^32)
    (hfit : lsum (legUsd ctx acc) P + lsum (legUnitP ctx) P < 2^127)
    (hd : EstDomain snap bb curve)
    (hwu : ∀ (q : math.fp.Wad) (b : math.fp.Bps), 0 ≤ q.val → q.val < snap.total_debt.val →
      snap.total_debt.val ≤ snap.total_collateral.val → 0 ≤ b.val → b.val ≤ bb.max.val →
      ∃ r : math.fp.Wad × context.Context,
        positions.liquidation.math.whole_unit_repayment env acc snap q b ctx = ok r ∧ 0 ≤ r.1.val)
    (hrp : ∀ l c, ∃ r, positions.liquidation.math.repays_every_debt_leg env acc l c = ok r)
    (res : positions.liquidation.math.NormalizedRepaymentPlan × context.Context)
    (hnorm : positions.liquidation.math.normalize_repayment_plan env acc raw snap bb curve ctx
      = ok res)
    (hkeys : (acc.borrow_positions.map Prod.fst).Nodup) :
    0 ≤ res.1.repay_usd.val ∧
      res.1.repay_usd.val = lsum (fun e => planU res.1 e.1) acc.borrow_positions := by
  obtain ⟨res', hres', hlegs, -, husd, h0, -⟩ := spec_imp_eq_ok
    (repayment_plan_spec env acc raw snap bb curve ctx P hagg hok htrim hlen hfit hd hwu hrp)
  rw [hnorm] at hres'
  have : res = res' := by simpa using hres'
  subst this
  have hin : ∀ x ∈ res.1.repaid, x.hub_asset ∈ acc.borrow_positions.map Prod.fst := by
    intro x hx
    obtain ⟨l, hl, hxl, -⟩ := hlegs x hx
    obtain ⟨-, ⟨d, hd⟩, -⟩ := hok l hl
    rw [hxl]; exact mapLookup_mem_keys hd
  refine ⟨h0, ?_⟩
  rw [husd]
  unfold usdSum planU usdSum
  exact sum_regroup acc.borrow_positions hkeys (fun x : types.RepayEntry => x.hub_asset)
    (fun x : types.RepayEntry => x.usd_wad.val) _ hin

/-- **Post-liquidation health factor of the production plan (HF-preserving
cap).** `health_after_hf_preserving` for account `acc` and the plan `res`
that `normalize_repayment_plan` returns for it. The pre-liquidation maps are
the account's own (`acc.supply_positions`, `acc.borrow_positions`); each debt
leg is repaid `planA` tokens and credited `planU` USD (the repaid entries
with its key); and the plan hypotheses `0 ≤ repay_usd` and
`repay_usd = Σ credits` come from `plan_credits_sum` instead of being
assumed. The remaining hypotheses are the distinct debt keys, the totals
domain of both accounts, the seizure domain of each supply leg, how the plan
is applied (`SupplyAfter`, `DebtAfter`, modelled), and the HF-preserving
cap. -/
theorem health_after_executed_plan (env : Env) (acc : types.Account)
    (raw : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128))
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bb : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve) (ctx : context.Context)
    (P : List (types.HubAssetKey × Std.I128))
    (hagg : payment_legs.aggregate_positive_payments env raw = ok P)
    (hok : ∀ l ∈ P, RepayLegOK ctx acc l) (htrim : ∀ l ∈ P, RepayLegTrimOK ctx acc l)
    (hlen : P.length < 2^32)
    (hfit : lsum (legUsd ctx acc) P + lsum (legUnitP ctx) P < 2^127)
    (hd : EstDomain snap bb curve)
    (hwu : ∀ (q : math.fp.Wad) (b : math.fp.Bps), 0 ≤ q.val → q.val < snap.total_debt.val →
      snap.total_debt.val ≤ snap.total_collateral.val → 0 ≤ b.val → b.val ≤ bb.max.val →
      ∃ r : math.fp.Wad × context.Context,
        positions.liquidation.math.whole_unit_repayment env acc snap q b ctx = ok r ∧ 0 ≤ r.1.val)
    (hrp : ∀ l c, ∃ r, positions.liquidation.math.repays_every_debt_leg env acc l c = ok r)
    (res : positions.liquidation.math.NormalizedRepaymentPlan × context.Context)
    (hnorm : positions.liquidation.math.normalize_repayment_plan env acc raw snap bb curve ctx
      = ok res)
    (sup' : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (debt' : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw)
    (hkeys : (acc.borrow_positions.map Prod.fst).Nodup)
    -- pre and post totals domain
    (hs : ∀ e ∈ acc.supply_positions, RawSupplyOK ctx e)
    (hb : ∀ e ∈ acc.borrow_positions, RawDebtOK ctx e)
    (hsfit : lsum (rCeil ctx) acc.supply_positions < 2^127)
    (hbfit : lsum (rDebt .Ceil ctx) acc.borrow_positions < 2^127)
    (hs' : ∀ e ∈ sup', RawSupplyOK ctx e) (hb' : ∀ e ∈ debt', RawDebtOK ctx e)
    (hsfit' : lsum (rCeil ctx) sup' < 2^127) (hbfit' : lsum (rDebt .Ceil ctx) debt' < 2^127)
    -- the plan's bonus, partial seizure, seizure domain and application
    (hb0 : 0 ≤ res.1.bonus.val) (hbB : res.1.bonus.val ≤ 10^8)
    (hsa : res.1.seize_all = false)
    (hlegs : ∀ e ∈ acc.supply_positions,
      SzLegOK (planLeg ctx (lsum (rHU ctx) acc.supply_positions) res.1 e))
    (hsup : List.Forall₂ (SupplyAfter (planLeg ctx (lsum (rHU ctx) acc.supply_positions) res.1))
      acc.supply_positions sup')
    (hdebt : List.Forall₂ (DebtAfter ctx (planA res.1) (planU res.1)) acc.borrow_positions debt')
    (hdl : ∀ e ∈ acc.borrow_positions, 0 < borrowIndexOf ctx e.1 ∧
      borrowIndexOf ctx e.1 ≤ 10^36 ∧ decOf ctx e.1.asset ≤ 18)
    -- the pre-liquidation snapshot and the HF-preserving cap
    (hD : 0 < lsum (rDebt .Ceil ctx) acc.borrow_positions)
    (hC : 0 < lsum (rHU ctx) acc.supply_positions)
    (hcap : (10^4 + res.1.bonus.val) *
        huD (lsum (rLt ctx) acc.supply_positions * 10^18) (lsum (rHU ctx) acc.supply_positions) ≤
      lsum (rLt ctx) acc.supply_positions * 10^18 / lsum (rDebt .Ceil ctx) acc.borrow_positions *
        10^4) :
    ∃ pre post : risk.totals.AccountRiskTotals,
      risk.totals.calculate_account_risk_totals env ctx acc.supply_positions acc.borrow_positions
        = ok (pre, ctx) ∧
      risk.totals.calculate_account_risk_totals env ctx sup' debt' = ok (post, ctx) ∧
      (pre.weighted_collateral.val : ℚ) * post.total_debt.val -
          pre.weighted_collateral.val *
            (lsum (fun e => 3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset)
              acc.borrow_positions : ℚ) / (2 * 10^54) -
          pre.total_debt.val * haErrC res.1.repay_usd.val (szOpb res.1.bonus.val)
            (szBudget res.1.repay_usd.val res.1.bonus.val) pre.total_collateral.val
            acc.supply_positions.length
            (lsum haLT acc.supply_positions)
            (lsum (fun e => haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset))
              acc.supply_positions)
            (lsum (fun e => haLT e *
              (planLeg ctx (lsum (rHU ctx) acc.supply_positions) res.1 e).Slack)
              acc.supply_positions)
            (lsum (fun e => haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset))
              acc.supply_positions)
        ≤ (post.weighted_collateral.val : ℚ) * pre.total_debt.val := by
  obtain ⟨hr0, hrep⟩ := plan_credits_sum env acc raw snap bb curve ctx P hagg hok htrim hlen
    hfit hd hwu hrp res hnorm hkeys
  exact health_after_hf_preserving env ctx acc.supply_positions sup' acc.borrow_positions debt'
    res.1 (planA res.1) (planU res.1) hs hb hsfit hbfit hs' hb' hsfit' hbfit' hr0 hb0 hbB hsa
    hlegs hsup hdebt hrep hdl hD hC hcap

end Plan

end ControllerAeneas.Proofs
