/-
**The health factor after the executed liquidation plan.**

Model of the post-liquidation account (credit-mode seizure, repayment
through `resolve_repay`):
* every supply leg loses exactly the shares its seizure entry records
  (`Seized` of `calculate_seized_collateral`, 0 for a leg without entry):
  `apply_liquidation_share_credit` subtracts `entry.scaled_amount`;
* every debt leg follows `DebtStep`: untouched, partially repaid with a
  floor burn `⌊a·10^(27−dec)·R/ib⌋` and credited at most the half-up USD
  value of `a` tokens, or fully repaid.

Both accounts are valued with the production `calculate_account_risk_totals`
(`Totals.lean`). With the HF-preserving bonus cap `(BPS + b)·p ≤ hf·BPS`
(what `estimate_liquidation_amount_contract` guarantees on a solvent
account), post-HF ≥ pre-HF up to an explicit rounding allowance.

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.HealthAfterCombine
import ControllerAeneas.Proofs.SeizeBonus
import ControllerAeneas.Proofs.EstimateTarget

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

theorem ha_forall2_lsum {α β : Type} {R : α → β → Prop} (f : α → Int) (g : β → Int)
    (h : ∀ a b, R a b → g b = f a) :
    ∀ {m : List α} {l : List β}, List.Forall₂ R m l → lsum g l = lsum f m := by
  intro m l hf
  induction hf with
  | nil => rfl
  | @cons a b m l hab _ ih => rw [lsum_cons, lsum_cons, ih, h a b hab]

theorem ha_forall2_lsum_le {α β : Type} {R : α → β → Prop} (f : α → Int) (g : β → Int)
    (h : ∀ a b, R a b → f a ≤ g b) :
    ∀ {m : List α} {l : List β}, List.Forall₂ R m l → lsum f m ≤ lsum g l := by
  intro m l hf
  induction hf with
  | nil => exact le_refl _
  | @cons a b m l hab _ ih => rw [lsum_cons, lsum_cons]; have := h a b hab; omega

theorem ha_lsum_sub {α : Type} (f g : α → Int) (l : List α) :
    lsum (fun e => f e - g e) l = lsum f l - lsum g l := by
  induction l with
  | nil => simp
  | cons a l ih => simp only [lsum_cons]; rw [ih]; ring

theorem ha_lsum_add {α : Type} (f g : α → Int) (l : List α) :
    lsum (fun e => f e + g e) l = lsum f l + lsum g l := by
  induction l with
  | nil => simp
  | cons a l ih => simp only [lsum_cons]; rw [ih]; ring

theorem ha_lsum_le_add {α : Type} (f g : α → Int) (c : Int) :
    ∀ l : List α, (∀ e ∈ l, f e ≤ g e + c) → lsum f l ≤ lsum g l + l.length * c := by
  intro l
  induction l with
  | nil => intro _; simp
  | cons x l ih =>
    intro h
    simp only [lsum_cons, List.length_cons]
    have h1 := h x (by simp)
    have h2 := ih (fun e he => h e (by simp [he]))
    push_cast; linarith

theorem ha_forall2_sum_le {α β : Type} {R : α → β → Prop} (f g : α → Int) (h : β → Int)
    (hR : ∀ a b, R a b → f a ≤ g a - h b) :
    ∀ {m : List α} {l : List β}, List.Forall₂ R m l → lsum f m ≤ lsum g m - lsum h l := by
  intro m l hf
  induction hf with
  | nil => simp
  | @cons a b m l hab _ ih => simp only [lsum_cons]; have := hR a b hab; linarith

theorem ha_forall2_sum_le_mem {α β : Type} {R : α → β → Prop} (f g : α → Int) (h : β → Int) :
    ∀ {m : List α} {l : List β}, List.Forall₂ R m l →
      (∀ a ∈ m, ∀ b, R a b → f a ≤ g a - h b) → lsum f m ≤ lsum g m - lsum h l := by
  intro m l hf
  induction hf with
  | nil => intro _; simp
  | @cons a b m l hab _ ih =>
    intro hR
    simp only [lsum_cons]
    have := hR a (by simp) b hab
    have := ih (fun x hx y hy => hR x (by simp [hx]) y hy)
    linarith

/-- Shares seized from leg `e` by the plan (`Seized` when the leg emits an
entry, else 0). -/
noncomputable def haSigma (G : types.HubAssetKey × types.AccountPositionRaw → SzLeg)
    (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  if (G e).Emit then (G e).Seized else 0

/-- Post-liquidation supply entry: same key and risk parameters, scaled
balance reduced by exactly the seized shares (credit-mode application). -/
def SupplyAfter (G : types.HubAssetKey × types.AccountPositionRaw → SzLeg)
    (e e' : types.HubAssetKey × types.AccountPositionRaw) : Prop :=
  e'.1 = e.1 ∧ e'.2.scaled_amount.val = e.2.scaled_amount.val - haSigma G e ∧
  e'.2.liquidation_threshold = e.2.liquidation_threshold

/-- Post-liquidation debt entry: same key, `DebtStep` with the leg's repaid
tokens `aOf k` and credited USD `uOf k`. -/
noncomputable def DebtAfter (ctx : context.Context) (aOf uOf : types.HubAssetKey → Int)
    (e e' : types.HubAssetKey × types.DebtPositionRaw) : Prop :=
  e'.1 = e.1 ∧ DebtStep e.2.scaled_amount.val (borrowIndexOf ctx e.1) (priceOf ctx e.1.asset)
    (decOf ctx e.1.asset) (aOf e.1) (uOf e.1) e'.2.scaled_amount.val

/-- Threshold of a raw supply entry, as an integer. -/
def haLT (e : types.HubAssetKey × types.AccountPositionRaw) : Int := e.2.liquidation_threshold.val

/-- **Post-liquidation health factor, HF-preserving bonus (headline).**

Setting: a pre-liquidation account (supply map `sup`, debt map `debt`) whose
totals are computed by `calculate_account_risk_totals` (`C`, `Wc`, `D`, all
hypotheses of `calculate_account_risk_totals_spec`), a plan with
`0 ≤ bonus b ≤ 10^8` BPS, `repay_usd = rep ≥ 0`, not seize-all, every supply
leg in the seizure domain `SzLegOK` (`planLeg` with `C` and the budget
`B = huD(rep·(W + b·10^14), W)`), and the post-liquidation account
(`sup'`, `debt'`) obtained by subtracting the seized shares (`SupplyAfter`)
and applying the repayments (`DebtAfter`, credited USD summing to `rep`).
Debt legs have `0 < ib ≤ 10^36`, `dec ≤ 18`. The post account satisfies the
totals hypotheses too.

If the bonus satisfies the **HF-preserving cap**
`(BPS + b)·p ≤ hf·BPS` with the production `p = huD(Wc·W, C)` and
`hf = ⌊Wc·W/D⌋` (`estimate_liquidation_amount_contract`: on a solvent
account `C ≥ D` with `0 < p` and `hf < W` either this holds or the estimate
closes the whole debt at bonus 0), then both totals calls return and, with
`Wc'`, `D'` the post totals,

`Wc'·D ≥ Wc·D' − Wc·Ed/(2·10^54) − D·ErrC`,

i.e. `Wc'/D' ≥ Wc/D − (Wc·Ed/(2·10^54) + D·ErrC)/(D·D')`: the health factor
does not fall by more than the rounding allowance, where
`Ed = Σ_debt (3·10^54 + 6·10^36·p_j)` (1.5 WAD units plus 3 token-wei of
value per debt leg) and `ErrC = haErrC` (half a unit per seizure rounding,
`n` units per supply leg, the per-leg `Slack` of `seized_value_le_budget`,
and a relative `10^-18`-scale share of the budget). In exact arithmetic
(`hf_preserved_of_rate`) the allowance is 0; `ha_counterexample` shows
the rounded code can lose a few units. -/
theorem health_after_hf_preserving (env : Env) (ctx : context.Context)
    (sup sup' : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (debt debt' : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw)
    (rep : positions.liquidation.math.NormalizedRepaymentPlan)
    (aOf uOf : types.HubAssetKey → Int)
    -- pre and post totals domain
    (hs : ∀ e ∈ sup, RawSupplyOK ctx e) (hb : ∀ e ∈ debt, RawDebtOK ctx e)
    (hsfit : lsum (rCeil ctx) sup < 2^127) (hbfit : lsum (rDebt .Ceil ctx) debt < 2^127)
    (hs' : ∀ e ∈ sup', RawSupplyOK ctx e) (hb' : ∀ e ∈ debt', RawDebtOK ctx e)
    (hsfit' : lsum (rCeil ctx) sup' < 2^127) (hbfit' : lsum (rDebt .Ceil ctx) debt' < 2^127)
    -- the plan
    (hr0 : 0 ≤ rep.repay_usd.val) (hb0 : 0 ≤ rep.bonus.val) (hbB : rep.bonus.val ≤ 10^8)
    (hsa : rep.seize_all = false)
    (hlegs : ∀ e ∈ sup, SzLegOK (planLeg ctx (lsum (rHU ctx) sup) rep e))
    (hsup : List.Forall₂ (SupplyAfter (planLeg ctx (lsum (rHU ctx) sup) rep)) sup sup')
    (hdebt : List.Forall₂ (DebtAfter ctx aOf uOf) debt debt')
    (hrep : rep.repay_usd.val = lsum (fun e => uOf e.1) debt)
    (hdl : ∀ e ∈ debt, 0 < borrowIndexOf ctx e.1 ∧ borrowIndexOf ctx e.1 ≤ 10^36 ∧
      decOf ctx e.1.asset ≤ 18)
    -- the pre-liquidation snapshot and the HF-preserving cap
    (hD : 0 < lsum (rDebt .Ceil ctx) debt) (hC : 0 < lsum (rHU ctx) sup)
    (hcap : (10^4 + rep.bonus.val) * huD (lsum (rLt ctx) sup * 10^18) (lsum (rHU ctx) sup) ≤
      lsum (rLt ctx) sup * 10^18 / lsum (rDebt .Ceil ctx) debt * 10^4) :
    ∃ pre post : risk.totals.AccountRiskTotals,
      risk.totals.calculate_account_risk_totals env ctx sup debt = ok (pre, ctx) ∧
      risk.totals.calculate_account_risk_totals env ctx sup' debt' = ok (post, ctx) ∧
      (pre.weighted_collateral.val : ℚ) * post.total_debt.val -
          pre.weighted_collateral.val *
            (lsum (fun e => 3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset) debt : ℚ) / (2 * 10^54) -
          pre.total_debt.val * haErrC rep.repay_usd.val (szOpb rep.bonus.val)
            (szBudget rep.repay_usd.val rep.bonus.val) pre.total_collateral.val sup.length
            (lsum haLT sup)
            (lsum (fun e => haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup)
            (lsum (fun e => haLT e * (planLeg ctx (lsum (rHU ctx) sup) rep e).Slack) sup)
            (lsum (fun e => haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset)) sup)
        ≤ (post.weighted_collateral.val : ℚ) * pre.total_debt.val := by
  -- the two totals calls
  obtain ⟨pre, hpre, hpc, hC1, hL1, hW1, hD1, hH1⟩ :=
    spec_imp_eq_ok (calculate_account_risk_totals_spec env ctx sup debt hs hb hsfit hbfit)
  obtain ⟨pre2, hpre2, -, -, -, hWC, -, -, -⟩ :=
    spec_imp_eq_ok (calculate_account_risk_totals_conservative env ctx sup debt hs hb hsfit hbfit)
  obtain ⟨post, hpost, hqc, -, -, hW2, hD2, -⟩ :=
    spec_imp_eq_ok (calculate_account_risk_totals_spec env ctx sup' debt' hs' hb' hsfit' hbfit')
  obtain ⟨pre, c⟩ := pre
  obtain ⟨post, c'⟩ := post
  simp only at hpc hqc hC1 hL1 hW1 hD1 hW2 hD2
  subst c c'
  rw [hpre] at hpre2
  have hpp : pre2 = (pre, ctx) := by simpa using hpre2.symm
  subst hpp
  simp only at hWC
  refine ⟨pre, post, hpre, hpost, ?_⟩
  -- notation
  set G := planLeg ctx (lsum (rHU ctx) sup) rep with hG
  set C := lsum (rHU ctx) sup with hCdef
  set Wc := lsum (rLt ctx) sup with hWcdef
  set D := lsum (rDebt .Ceil ctx) debt with hDdef
  set B := szBudget rep.repay_usd.val rep.bonus.val with hBdef
  set opb := szOpb rep.bonus.val with hopbdef
  set prop := huD (Wc * 10^18) C with hpropdef
  set hf := Wc * 10^18 / D with hhfdef
  have gC : ∀ e, (G e).C = C := fun e => rfl
  have gB : ∀ e, (G e).B = B := fun e => rfl
  have gs : ∀ e, (G e).s = e.2.scaled_amount.val := fun e => rfl
  have gi : ∀ e, (G e).i = supplyIndexOf ctx e.1 := fun e => rfl
  have gp : ∀ e, (G e).p = priceOf ctx e.1.asset := fun e => rfl
  have gsa : ∀ e, (G e).sa = false := fun e => hsa
  have gV : ∀ e, (G e).V = rHU ctx e := fun e => rfl
  -- (1) floor-weighted loss
  set X := lsum (fun e => haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset)) sup
  set Ec := lsum (fun e => haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup
  have hWc' : lsum (rLt ctx) sup' =
      lsum (fun e => fv (e.2.scaled_amount.val - haSigma G e) (supplyIndexOf ctx e.1)
        (priceOf ctx e.1.asset) * haLT e / 10^4) sup := by
    apply ha_forall2_lsum _ _ _ hsup
    rintro e e' ⟨h1, h2, h3⟩
    simp only [rLt, rFloor, rLT, haLT, h1, h2, h3]
  have h1 : 10^4 * 10^54 * (Wc - lsum (rLt ctx) sup') ≤ X + Ec + sup.length * 10^4 * 10^54 := by
    rw [hWc']
    have hl : lsum (fun e => 10^4 * 10^54 * (rLt ctx e - fv (e.2.scaled_amount.val - haSigma G e)
          (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) * haLT e / 10^4)) sup ≤
        lsum (fun e => haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset) +
          haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup + sup.length * (10^4 * 10^54) := by
      apply ha_lsum_le_add
      intro e he
      obtain ⟨-, ⟨hs0, hi0, hp0, -, -⟩, hLTB, -⟩ := hs e he
      have hl := hlegs e he
      have hσ0 : 0 ≤ haSigma G e := by
        unfold haSigma; split_ifs with c
        · exact (hl.seized_le c).1.le
        · exact le_refl _
      have hσs : haSigma G e ≤ e.2.scaled_amount.val := by
        unfold haSigma; split_ifs with c
        · exact (hl.seized_le c).2
        · exact hs0
      have := ha_coll_leg e.2.scaled_amount.val (haSigma G e) (supplyIndexOf ctx e.1)
        (priceOf ctx e.1.asset) (haLT e) hσ0 hσs hi0 hp0 (by unfold haLT; positivity)
      simp only [rLt, rFloor, rLT]; unfold haLT at this ⊢
      linarith
    have e1 : lsum (fun e => 10^4 * 10^54 * (rLt ctx e - fv (e.2.scaled_amount.val - haSigma G e)
          (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) * haLT e / 10^4)) sup =
        10^4 * 10^54 * (Wc - lsum (fun e => fv (e.2.scaled_amount.val - haSigma G e)
          (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) * haLT e / 10^4) sup) := by
      rw [sz_lsum_mul_left, ha_lsum_sub]
    have e2 : lsum (fun e => haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset) +
          haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup = X + Ec := by
      exact ha_lsum_add _ _ _
    rw [e1, e2] at hl
    linarith
  -- (2) seized value ≤ leg budget + slack
  set Y := lsum (fun e => haLT e * (G e).L) sup
  set Es := lsum (fun e => haLT e * (G e).Slack) sup
  have h2 : 2 * X ≤ 2 * 10^54 * Y + Es := by
    have hl : lsum (fun e => 2 * (haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset)))
        sup ≤ lsum (fun e => 2 * 10^54 * (haLT e * (G e).L) + haLT e * (G e).Slack) sup := by
      apply lsum_le
      intro e he
      have hLT : 0 ≤ haLT e := by unfold haLT; positivity
      have hv := (hlegs e he).seizedVal_le (gsa e)
      have hsv : haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset = (G e).SeizedVal := by
        unfold haSigma SzLeg.SeizedVal; split_ifs <;> simp [gi, gp]
      rw [hsv]
      have := mul_le_mul_of_nonneg_left hv hLT
      linarith
    rw [sz_lsum_mul_left, sz_lsum_lin] at hl
    exact hl
  -- (3) leg budget ≈ pro-rata share of the budget
  set Z := lsum (fun e => haLT e * rHU ctx e) sup
  set SLT := lsum haLT sup
  have h3 : 4 * 10^18 * C * Y ≤ 4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT := by
    have hl : lsum (fun e => 4 * 10^18 * C * (haLT e * (G e).L)) sup ≤
        lsum (fun e => 4 * B * 10^18 * (haLT e * rHU ctx e) + 2 * C * (B + 10^18) * haLT e) sup := by
      apply lsum_le
      intro e he
      have hLT : 0 ≤ haLT e := by unfold haLT; positivity
      have hv := (hlegs e he).L_share
      rw [gC, gB, gV] at hv
      have := mul_le_mul_of_nonneg_left hv hLT
      nlinarith
    rw [sz_lsum_mul_left, ha_lsum_add, sz_lsum_mul_left, sz_lsum_mul_left] at hl
    exact hl
  -- (4) half-up value vs weighted floor value
  set Ev := lsum (fun e => haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset)) sup
  have h4 : 10^54 * Z ≤ 10^54 * 10^4 * (Wc + sup.length) + Ev := by
    have hl : lsum (fun e => 10^54 * (haLT e * rHU ctx e)) sup ≤
        lsum (fun e => 10^54 * 10^4 * rLt ctx e + haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset))
          sup + sup.length * (10^54 * 10^4) := by
      apply ha_lsum_le_add
      intro e he
      obtain ⟨-, ⟨hs0, hi0, hp0, -, -⟩, -, -⟩ := hs e he
      have := ha_huv_weighted e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
        (haLT e) (mul_nonneg hs0 hi0) hp0 (by unfold haLT; positivity)
      simp only [rHU, rLt, rFloor, rLT]; unfold haLT at this ⊢
      linarith
    rw [sz_lsum_mul_left, sz_lsum_lin] at hl
    linarith
  -- (5) half-up budget; (6) half-up proportion; (7) cap; (8) floored HF
  have h5 : 2 * 10^18 * B ≤ 2 * rep.repay_usd.val * opb + 10^18 := by
    have := (sz_huD_bounds (rep.repay_usd.val * opb) (10^18) (by norm_num)).1
    have hB' : B = huD (rep.repay_usd.val * opb) (10^18) := rfl
    rw [← hB'] at this; linarith
  have h6 : 2 * Wc * 10^18 ≤ C * (2 * prop + 1) := by
    have := (sz_huD_bounds (Wc * 10^18) C hC).2
    linarith
  have h7 : opb * prop ≤ hf * 10^18 := by
    have e : opb = (10^4 + rep.bonus.val) * 10^14 := by rw [hopbdef]; unfold szOpb; ring
    rw [e]; nlinarith
  have h8 : hf * D ≤ Wc * 10^18 := (sz_div_bounds (Wc * 10^18) D hD).1
  -- (9) debt shed
  set Ed := lsum (fun e => 3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset) debt
  have h9 : 2 * 10^54 * rep.repay_usd.val - Ed ≤ 2 * 10^54 * (D - lsum (rDebt .Ceil ctx) debt') := by
    have hl : lsum (fun e => 2 * 10^54 * uOf e.1 - (3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset)) debt ≤
        lsum (fun e => 2 * 10^54 * rDebt .Ceil ctx e) debt -
          lsum (fun e => 2 * 10^54 * rDebt .Ceil ctx e) debt' := by
      apply ha_forall2_sum_le_mem _ _ _ hdebt
      rintro e he e' ⟨hk, hst⟩
      obtain ⟨hib0, hibB, hdec⟩ := hdl e he
      obtain ⟨-, ⟨ht0, -, hp0, -, -⟩⟩ := hb e he
      have := ha_debt_leg _ _ _ _ _ _ _ ht0 hib0 hibB hp0 hdec hst
      simp only [rDebt, debtVal, hk]
      linarith [this.2.2]
    rw [sz_lsum_mul_left, sz_lsum_mul_left, ha_lsum_sub, sz_lsum_mul_left, ← hrep] at hl
    linarith
  -- combine
  have hWcC : Wc ≤ C := by rw [← hW1, ← hC1]; exact hWC
  have hWc0 : 0 ≤ Wc := by
    rw [hWcdef]; unfold lsum; apply List.sum_nonneg
    intro x hx
    obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
    obtain ⟨-, ⟨hs0, hi0, hp0, -, -⟩, -, -⟩ := hs y hy
    unfold rLt rFloor rLT fv
    apply sz_div_nonneg _ _ (by norm_num); apply mul_nonneg _ (by positivity)
    apply sz_div_nonneg _ _ (by norm_num); apply mul_nonneg _ hp0
    exact sz_div_nonneg _ _ (by norm_num) (sz_div_nonneg _ _ (by norm_num) (mul_nonneg hs0 hi0))
  have hopb0 : 0 ≤ opb := by rw [hopbdef]; unfold szOpb; nlinarith
  have hB0 : 0 ≤ B := sz_huD_nonneg _ _ (by norm_num) (mul_nonneg hr0 hopb0)
  have key := ha_combine (Wc : ℚ) (lsum (rLt ctx) sup' : ℚ) (D : ℚ) (lsum (rDebt .Ceil ctx) debt' : ℚ)
    (C : ℚ) (B : ℚ) (rep.repay_usd.val : ℚ) (opb : ℚ) (prop : ℚ) (hf : ℚ) (X : ℚ) (Y : ℚ) (Z : ℚ)
    (Ec : ℚ) (Es : ℚ) (Ev : ℚ) (Ed : ℚ) (sup.length : ℚ) (SLT : ℚ)
    (by exact_mod_cast hC) (by exact_mod_cast hD) (by exact_mod_cast hB0) (by exact_mod_cast hr0)
    (by exact_mod_cast hopb0) (by exact_mod_cast hWc0) (by exact_mod_cast hWcC)
    (by exact_mod_cast h1) (by exact_mod_cast h2) (by exact_mod_cast h3) (by exact_mod_cast h4)
    (by exact_mod_cast h5) (by exact_mod_cast h6) (by exact_mod_cast h7) (by exact_mod_cast h8)
    (by exact_mod_cast h9)
  rw [hW1, hD1, hC1, hW2, hD2]
  exact key

/-- **Post-liquidation health factor, curve-target branch (headline).**
Same setting and hypotheses as `health_after_hf_preserving`, except that
instead of the HF-preserving cap the estimate took the curve-target branch:
the snapshot satisfies `HitsTarget D Wc p T b r` for the quote `r`
(`estimate_hits_curve_target`, with the production `p = huD(Wc·W, C)`),
`0 ≤ T`, and the plan repays at least the quote, `r ≤ rep` (production may
accept less when the liquidator offers less; then the target is not
reached). Then, with `δ = huD(p·(W + b·10^14), W)` the code's seizure rate,

`W·Wc' ≥ T·D' − (T·Ed/(2·10^54) + (T − δ + W)/2 + rep/2 + W·ErrC)`,

i.e. the post-liquidation health factor `Wc'/D'` is at least the target
`T/W` up to the allowance divided by `W·D'`: the `HitsTarget` slack
(`≤ 5.5` WAD units), half a unit per repaid unit for the rounded rate `δ`
(`10^-18` relative), and the same collateral/debt rounding as the cap
theorem. -/
theorem health_after_target (env : Env) (ctx : context.Context)
    (sup sup' : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (debt debt' : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw)
    (rep : positions.liquidation.math.NormalizedRepaymentPlan)
    (aOf uOf : types.HubAssetKey → Int)
    -- pre and post totals domain
    (hs : ∀ e ∈ sup, RawSupplyOK ctx e) (hb : ∀ e ∈ debt, RawDebtOK ctx e)
    (hsfit : lsum (rCeil ctx) sup < 2^127) (hbfit : lsum (rDebt .Ceil ctx) debt < 2^127)
    (hs' : ∀ e ∈ sup', RawSupplyOK ctx e) (hb' : ∀ e ∈ debt', RawDebtOK ctx e)
    (hsfit' : lsum (rCeil ctx) sup' < 2^127) (hbfit' : lsum (rDebt .Ceil ctx) debt' < 2^127)
    -- the plan
    (hr0 : 0 ≤ rep.repay_usd.val) (hb0 : 0 ≤ rep.bonus.val) (hbB : rep.bonus.val ≤ 10^8)
    (hsa : rep.seize_all = false)
    (hlegs : ∀ e ∈ sup, SzLegOK (planLeg ctx (lsum (rHU ctx) sup) rep e))
    (hsup : List.Forall₂ (SupplyAfter (planLeg ctx (lsum (rHU ctx) sup) rep)) sup sup')
    (hdebt : List.Forall₂ (DebtAfter ctx aOf uOf) debt debt')
    (hrep : rep.repay_usd.val = lsum (fun e => uOf e.1) debt)
    (hdl : ∀ e ∈ debt, 0 < borrowIndexOf ctx e.1 ∧ borrowIndexOf ctx e.1 ≤ 10^36 ∧
      decOf ctx e.1.asset ≤ 18)
    -- the pre-liquidation snapshot and the HF-preserving cap
    (hD : 0 < lsum (rDebt .Ceil ctx) debt) (hC : 0 < lsum (rHU ctx) sup)
    (T r : Int) (hT0 : 0 ≤ T) (hrr : r ≤ rep.repay_usd.val)
    (htgt : HitsTarget (lsum (rDebt .Ceil ctx) debt) (lsum (rLt ctx) sup)
      (huD (lsum (rLt ctx) sup * 10^18) (lsum (rHU ctx) sup)) T rep.bonus.val r) :
    ∃ pre post : risk.totals.AccountRiskTotals,
      risk.totals.calculate_account_risk_totals env ctx sup debt = ok (pre, ctx) ∧
      risk.totals.calculate_account_risk_totals env ctx sup' debt' = ok (post, ctx) ∧
      (T : ℚ) * post.total_debt.val -
          ((T : ℚ) * (lsum (fun e => 3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset) debt : ℚ) / (2 * 10^54) +
            ((T : ℚ) - huD (huD (lsum (rLt ctx) sup * 10^18) (lsum (rHU ctx) sup) *
              szOpb rep.bonus.val) (10^18) + 10^18) / 2 + (rep.repay_usd.val : ℚ) / 2 +
            10^18 * haErrC rep.repay_usd.val (szOpb rep.bonus.val)
            (szBudget rep.repay_usd.val rep.bonus.val) pre.total_collateral.val sup.length
            (lsum haLT sup)
            (lsum (fun e => haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup)
            (lsum (fun e => haLT e * (planLeg ctx (lsum (rHU ctx) sup) rep e).Slack) sup)
            (lsum (fun e => haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset)) sup))
        ≤ 10^18 * (post.weighted_collateral.val : ℚ) := by
  -- the two totals calls
  obtain ⟨pre, hpre, hpc, hC1, hL1, hW1, hD1, hH1⟩ :=
    spec_imp_eq_ok (calculate_account_risk_totals_spec env ctx sup debt hs hb hsfit hbfit)
  obtain ⟨pre2, hpre2, -, -, -, hWC, -, -, -⟩ :=
    spec_imp_eq_ok (calculate_account_risk_totals_conservative env ctx sup debt hs hb hsfit hbfit)
  obtain ⟨post, hpost, hqc, -, -, hW2, hD2, -⟩ :=
    spec_imp_eq_ok (calculate_account_risk_totals_spec env ctx sup' debt' hs' hb' hsfit' hbfit')
  obtain ⟨pre, c⟩ := pre
  obtain ⟨post, c'⟩ := post
  simp only at hpc hqc hC1 hL1 hW1 hD1 hW2 hD2
  subst c c'
  rw [hpre] at hpre2
  have hpp : pre2 = (pre, ctx) := by simpa using hpre2.symm
  subst hpp
  simp only at hWC
  refine ⟨pre, post, hpre, hpost, ?_⟩
  -- notation
  set G := planLeg ctx (lsum (rHU ctx) sup) rep with hG
  set C := lsum (rHU ctx) sup with hCdef
  set Wc := lsum (rLt ctx) sup with hWcdef
  set D := lsum (rDebt .Ceil ctx) debt with hDdef
  set B := szBudget rep.repay_usd.val rep.bonus.val with hBdef
  set opb := szOpb rep.bonus.val with hopbdef
  set prop := huD (Wc * 10^18) C with hpropdef
  set hf := Wc * 10^18 / D with hhfdef
  have gC : ∀ e, (G e).C = C := fun e => rfl
  have gB : ∀ e, (G e).B = B := fun e => rfl
  have gs : ∀ e, (G e).s = e.2.scaled_amount.val := fun e => rfl
  have gi : ∀ e, (G e).i = supplyIndexOf ctx e.1 := fun e => rfl
  have gp : ∀ e, (G e).p = priceOf ctx e.1.asset := fun e => rfl
  have gsa : ∀ e, (G e).sa = false := fun e => hsa
  have gV : ∀ e, (G e).V = rHU ctx e := fun e => rfl
  -- (1) floor-weighted loss
  set X := lsum (fun e => haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset)) sup
  set Ec := lsum (fun e => haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup
  have hWc' : lsum (rLt ctx) sup' =
      lsum (fun e => fv (e.2.scaled_amount.val - haSigma G e) (supplyIndexOf ctx e.1)
        (priceOf ctx e.1.asset) * haLT e / 10^4) sup := by
    apply ha_forall2_lsum _ _ _ hsup
    rintro e e' ⟨h1, h2, h3⟩
    simp only [rLt, rFloor, rLT, haLT, h1, h2, h3]
  have h1 : 10^4 * 10^54 * (Wc - lsum (rLt ctx) sup') ≤ X + Ec + sup.length * 10^4 * 10^54 := by
    rw [hWc']
    have hl : lsum (fun e => 10^4 * 10^54 * (rLt ctx e - fv (e.2.scaled_amount.val - haSigma G e)
          (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) * haLT e / 10^4)) sup ≤
        lsum (fun e => haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset) +
          haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup + sup.length * (10^4 * 10^54) := by
      apply ha_lsum_le_add
      intro e he
      obtain ⟨-, ⟨hs0, hi0, hp0, -, -⟩, hLTB, -⟩ := hs e he
      have hl := hlegs e he
      have hσ0 : 0 ≤ haSigma G e := by
        unfold haSigma; split_ifs with c
        · exact (hl.seized_le c).1.le
        · exact le_refl _
      have hσs : haSigma G e ≤ e.2.scaled_amount.val := by
        unfold haSigma; split_ifs with c
        · exact (hl.seized_le c).2
        · exact hs0
      have := ha_coll_leg e.2.scaled_amount.val (haSigma G e) (supplyIndexOf ctx e.1)
        (priceOf ctx e.1.asset) (haLT e) hσ0 hσs hi0 hp0 (by unfold haLT; positivity)
      simp only [rLt, rFloor, rLT]; unfold haLT at this ⊢
      linarith
    have e1 : lsum (fun e => 10^4 * 10^54 * (rLt ctx e - fv (e.2.scaled_amount.val - haSigma G e)
          (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) * haLT e / 10^4)) sup =
        10^4 * 10^54 * (Wc - lsum (fun e => fv (e.2.scaled_amount.val - haSigma G e)
          (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) * haLT e / 10^4) sup) := by
      rw [sz_lsum_mul_left, ha_lsum_sub]
    have e2 : lsum (fun e => haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset) +
          haLT e * (10^54 + 2 * 10^36 * priceOf ctx e.1.asset)) sup = X + Ec := by
      exact ha_lsum_add _ _ _
    rw [e1, e2] at hl
    linarith
  -- (2) seized value ≤ leg budget + slack
  set Y := lsum (fun e => haLT e * (G e).L) sup
  set Es := lsum (fun e => haLT e * (G e).Slack) sup
  have h2 : 2 * X ≤ 2 * 10^54 * Y + Es := by
    have hl : lsum (fun e => 2 * (haLT e * (haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset)))
        sup ≤ lsum (fun e => 2 * 10^54 * (haLT e * (G e).L) + haLT e * (G e).Slack) sup := by
      apply lsum_le
      intro e he
      have hLT : 0 ≤ haLT e := by unfold haLT; positivity
      have hv := (hlegs e he).seizedVal_le (gsa e)
      have hsv : haSigma G e * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset = (G e).SeizedVal := by
        unfold haSigma SzLeg.SeizedVal; split_ifs <;> simp [gi, gp]
      rw [hsv]
      have := mul_le_mul_of_nonneg_left hv hLT
      linarith
    rw [sz_lsum_mul_left, sz_lsum_lin] at hl
    exact hl
  -- (3) leg budget ≈ pro-rata share of the budget
  set Z := lsum (fun e => haLT e * rHU ctx e) sup
  set SLT := lsum haLT sup
  have h3 : 4 * 10^18 * C * Y ≤ 4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT := by
    have hl : lsum (fun e => 4 * 10^18 * C * (haLT e * (G e).L)) sup ≤
        lsum (fun e => 4 * B * 10^18 * (haLT e * rHU ctx e) + 2 * C * (B + 10^18) * haLT e) sup := by
      apply lsum_le
      intro e he
      have hLT : 0 ≤ haLT e := by unfold haLT; positivity
      have hv := (hlegs e he).L_share
      rw [gC, gB, gV] at hv
      have := mul_le_mul_of_nonneg_left hv hLT
      nlinarith
    rw [sz_lsum_mul_left, ha_lsum_add, sz_lsum_mul_left, sz_lsum_mul_left] at hl
    exact hl
  -- (4) half-up value vs weighted floor value
  set Ev := lsum (fun e => haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset)) sup
  have h4 : 10^54 * Z ≤ 10^54 * 10^4 * (Wc + sup.length) + Ev := by
    have hl : lsum (fun e => 10^54 * (haLT e * rHU ctx e)) sup ≤
        lsum (fun e => 10^54 * 10^4 * rLt ctx e + haLT e * (2 * 10^54 + 4 * 10^36 * priceOf ctx e.1.asset))
          sup + sup.length * (10^54 * 10^4) := by
      apply ha_lsum_le_add
      intro e he
      obtain ⟨-, ⟨hs0, hi0, hp0, -, -⟩, -, -⟩ := hs e he
      have := ha_huv_weighted e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
        (haLT e) (mul_nonneg hs0 hi0) hp0 (by unfold haLT; positivity)
      simp only [rHU, rLt, rFloor, rLT]; unfold haLT at this ⊢
      linarith
    rw [sz_lsum_mul_left, sz_lsum_lin] at hl
    linarith
  -- (5) half-up budget; (6) half-up proportion; (7) cap; (8) floored HF
  have h5 : 2 * 10^18 * B ≤ 2 * rep.repay_usd.val * opb + 10^18 := by
    have := (sz_huD_bounds (rep.repay_usd.val * opb) (10^18) (by norm_num)).1
    have hB' : B = huD (rep.repay_usd.val * opb) (10^18) := rfl
    rw [← hB'] at this; linarith
  have h6 : 2 * Wc * 10^18 ≤ C * (2 * prop + 1) := by
    have := (sz_huD_bounds (Wc * 10^18) C hC).2
    linarith
  obtain ⟨δ, hδ, hδT, hT1, -⟩ := htgt
  have hδe : δ = huD (prop * opb) (10^18) := by
    rw [hopbdef]; unfold szOpb; exact isHalfUp_eq (by norm_num) hδ
  have h7 : 2 * (opb * prop) < 2 * δ * 10^18 + 10^18 := by
    have := (sz_huD_bounds (prop * opb) (10^18) (by norm_num)).2
    rw [← hδe] at this; linarith
  -- (9) debt shed
  set Ed := lsum (fun e => 3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset) debt
  have h9 : 2 * 10^54 * rep.repay_usd.val - Ed ≤ 2 * 10^54 * (D - lsum (rDebt .Ceil ctx) debt') := by
    have hl : lsum (fun e => 2 * 10^54 * uOf e.1 - (3 * 10^54 + 6 * 10^36 * priceOf ctx e.1.asset)) debt ≤
        lsum (fun e => 2 * 10^54 * rDebt .Ceil ctx e) debt -
          lsum (fun e => 2 * 10^54 * rDebt .Ceil ctx e) debt' := by
      apply ha_forall2_sum_le_mem _ _ _ hdebt
      rintro e he e' ⟨hk, hst⟩
      obtain ⟨hib0, hibB, hdec⟩ := hdl e he
      obtain ⟨-, ⟨ht0, -, hp0, -, -⟩⟩ := hb e he
      have := ha_debt_leg _ _ _ _ _ _ _ ht0 hib0 hibB hp0 hdec hst
      simp only [rDebt, debtVal, hk]
      linarith [this.2.2]
    rw [sz_lsum_mul_left, sz_lsum_mul_left, ha_lsum_sub, sz_lsum_mul_left, ← hrep] at hl
    linarith
  -- combine
  have hWcC : Wc ≤ C := by rw [← hW1, ← hC1]; exact hWC
  have hWc0 : 0 ≤ Wc := by
    rw [hWcdef]; unfold lsum; apply List.sum_nonneg
    intro x hx
    obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
    obtain ⟨-, ⟨hs0, hi0, hp0, -, -⟩, -, -⟩ := hs y hy
    unfold rLt rFloor rLT fv
    apply sz_div_nonneg _ _ (by norm_num); apply mul_nonneg _ (by positivity)
    apply sz_div_nonneg _ _ (by norm_num); apply mul_nonneg _ hp0
    exact sz_div_nonneg _ _ (by norm_num) (sz_div_nonneg _ _ (by norm_num) (mul_nonneg hs0 hi0))
  have hopb0 : 0 ≤ opb := by rw [hopbdef]; unfold szOpb; nlinarith
  have hB0 : 0 ≤ B := sz_huD_nonneg _ _ (by norm_num) (mul_nonneg hr0 hopb0)
  have key := ha_combine_target (Wc : ℚ) (lsum (rLt ctx) sup' : ℚ) (D : ℚ)
    (lsum (rDebt .Ceil ctx) debt' : ℚ) (C : ℚ) (B : ℚ) (rep.repay_usd.val : ℚ) (opb : ℚ) (prop : ℚ)
    (δ : ℚ) (T : ℚ) (r : ℚ) (X : ℚ) (Y : ℚ) (Z : ℚ)
    (Ec : ℚ) (Es : ℚ) (Ev : ℚ) (Ed : ℚ) (sup.length : ℚ) (SLT : ℚ)
    (by exact_mod_cast hC) (by exact_mod_cast hB0) (by exact_mod_cast hr0)
    (by exact_mod_cast hopb0) (by exact_mod_cast hWc0) (by exact_mod_cast hWcC) (by exact_mod_cast hT0)
    (by exact_mod_cast h1) (by exact_mod_cast h2) (by exact_mod_cast h3) (by exact_mod_cast h4)
    (by exact_mod_cast h5) (by exact_mod_cast h6) (by exact_mod_cast h7) (by exact_mod_cast hδT)
    (by exact_mod_cast hrr) (by exact_mod_cast hT1) (by exact_mod_cast h9)
  rw [hC1, hW2, hD2]
  rw [← hδe]
  convert key using 2

/-- The leg of `ha_counterexample`: USD 1100 of an 18-decimal asset at price
1, index `RAY`, budget for repaying 1001 raw units at a 10% bonus. -/
def haCexLeg : SzLeg :=
  ⟨1100 * 10^27, 10^27, 10^18, 18, 0, 1100 * 10^18, szBudget 1001 1000, szOpb 1000, false, false⟩

/-- **Counterexample: the rounded code can lower the health factor even at
the HF-preserving cap.** Account: one supply leg (USD 1100, threshold 50%,
18 decimals, price 1, index `RAY`), one debt leg (USD 1000, 18 decimals,
price 1, index `RAY`). Production values: `C = 1100 W`, `Wc = 550 W`,
`D = 1000 W`, `hf = 0.55 W`, `p = 0.5 W`, and the HF-preserving cap is
exactly `1000` BPS with no slack: `(BPS + 1000)·p = hf·BPS`. The liquidator
repays 1001 raw units (`usd = 1001`, accepted as offered below the ideal),
the budget is `huD(1001·1.1) = 1101`, the leg seizes exactly
`1101·10^9` shares (`SzLeg` closed forms of `calculate_seized_collateral`),
and the floor burn removes `1001·10^9` debt shares. Then
`Wc' = 550 W − 551` and `D' = 1000 W − 1001`, so `Wc'·D < Wc·D'` and the
production health factor drops from `0.55 W` to `0.55 W − 1`. The drop is
one `10^-18` unit (the half unit lost in `⌊(…)·LT/BPS⌋` of the seized
`1101` against the `1100.1` the cap allows), well inside the allowance of
`health_after_hf_preserving`, so the theorem's rounding term is necessary. -/
theorem ha_counterexample :
    let s : Int := 1100 * 10^27
    let t : Int := 1000 * 10^27
    let Wc := fv s (10^27) (10^18) * 5000 / 10^4
    let D := cv t (10^27) (10^18)
    let hf := Wc * 10^18 / D
    let prop := huD (Wc * 10^18) (huv s (10^27) (10^18))
    let Wc' := fv (s - haCexLeg.Seized) (10^27) (10^18) * 5000 / 10^4
    let D' := cv (t - 1001 * 10^9 * 10^27 / 10^27) (10^27) (10^18)
    Wc = 550 * 10^18 ∧ D = 1000 * 10^18 ∧ hf = 55 * 10^16 ∧ prop = 5 * 10^17 ∧
    (10^4 + 1000) * prop = hf * 10^4 ∧
    huD (1001 * 10^(18 - 18) * 10^18) (10^18) = 1001 ∧
    haCexLeg.Emit ∧ haCexLeg.Seized = 1101 * 10^9 ∧
    Wc' = 550 * 10^18 - 551 ∧ D' = 1000 * 10^18 - 1001 ∧
    Wc' * D < Wc * D' ∧ Wc' * 10^18 / D' = 55 * 10^16 - 1 := by
  have hseized : haCexLeg.Seized = 1101 * 10^9 := by
    simp only [haCexLeg, SzLeg.Seized, SzLeg.Capped, SzLeg.S1, SzLeg.WU, SzLeg.S0, SzLeg.Q,
      SzLeg.L, SzLeg.Sh, SzLeg.V, SzLeg.A, szBudget, szOpb, huv, huD]
    norm_num
  have hemit : haCexLeg.Emit := by
    simp only [haCexLeg, SzLeg.Emit, SzLeg.Seized, SzLeg.Capped, SzLeg.S1, SzLeg.WU, SzLeg.S0,
      SzLeg.Q, SzLeg.L, SzLeg.Sh, SzLeg.V, SzLeg.A, SzLeg.Amt, SzLeg.Dd, szBudget, szOpb, huv, huD]
    norm_num
  simp only [hseized]
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, hemit, trivial, ?_, ?_, ?_, ?_⟩ <;>
    simp only [fv, cv, huv, huD, clD] <;> norm_num

end ControllerAeneas.Proofs
