/-
Cross-leg bound of `calculate_seized_collateral`: the total exact value of
the seized shares is at most the seizure budget plus a stated number of
rounding units per leg (INV-LIQ-02).

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.SeizeLoop

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-- Exact value (scaled by `10^54`) of the shares a leg seizes, `0` if it
emits no entry. -/
def SzLeg.SeizedVal (g : SzLeg) : Int := if g.Emit then g.Seized * g.i * g.p else 0

/-- Rounding allowance of one leg, in the `10^54` scale:
`10^27·p·(10^9 + 1 + 2X)`, `X = Dd − 1` on the whole-unit round-up branch. -/
def SzLeg.Slack (g : SzLeg) : Int :=
  10^27 * g.p * (10^9 + 1 + 2 * (if g.WU ∧ g.rad = true then g.Dd - 1 else 0))

/-- Per leg: `2·SeizedVal ≤ 2·10^54·L + Slack` (outside seize-all). -/
theorem SzLegOK.seizedVal_le {g : SzLeg} (h : SzLegOK g) (hsa : g.sa = false) :
    2 * g.SeizedVal ≤ 2 * 10^54 * g.L + g.Slack := by
  unfold SzLeg.SeizedVal SzLeg.Slack
  have hb := h.seized_value_le_budget hsa
  have := h.L0; have := h.p0; have := h.Dd_pos
  split_ifs at hb ⊢ with he c
  · linarith
  · linarith
  · have : 0 ≤ 10^27 * g.p * (10^9 + 1 + 2 * (g.Dd - 1)) := by
      apply mul_nonneg (by positivity); omega
    linarith
  · have : 0 ≤ 10^27 * g.p * (10^9 + 1 + 2 * 0) := by positivity
    linarith

/-- Per leg: the leg budget is the pro-rata share of the total budget up to
rounding: `4·W·C·L ≤ 4·B·W·V + 2·C·(B + W)`. -/
theorem SzLegOK.L_share {g : SzLeg} (h : SzLegOK g) :
    4 * 10^18 * g.C * g.L ≤ 4 * g.B * 10^18 * g.V + 2 * g.C * (g.B + 10^18) := by
  have h1 := sz_huD_bounds (g.B * g.Sh) (10^18) (by norm_num)
  have h2 := sz_huD_bounds (g.V * 10^18) g.C h.C0
  have hL : g.L = huD (g.B * g.Sh) (10^18) := rfl
  have hS : g.Sh = huD (g.V * 10^18) g.C := rfl
  rw [← hL] at h1; rw [← hS] at h2
  have := h.C0; have := h.B0
  nlinarith

theorem sz_lsum_mul_left {α : Type} (f : α → Int) (c : Int) (l : List α) :
    lsum (fun e => c * f e) l = c * lsum f l := by
  induction l with
  | nil => simp
  | cons a l ih => simp only [lsum_cons]; rw [ih]; ring

theorem sz_lsum_lin {α : Type} (f h : α → Int) (c : Int) (l : List α) :
    lsum (fun e => c * f e + h e) l = c * lsum f l + lsum h l := by
  induction l with
  | nil => simp
  | cons a l ih => simp only [lsum_cons]; rw [ih]; ring

theorem sz_lsum_bound {α : Type} (f g : α → Int) (a c : Int) :
    ∀ l : List α, (∀ e ∈ l, f e ≤ a * g e + c) → lsum f l ≤ a * lsum g l + l.length * c := by
  intro l
  induction l with
  | nil => intro _; simp
  | cons x l ih =>
    intro h
    simp only [lsum_cons, List.length_cons]
    have h1 := h x (by simp)
    have h2 := ih (fun e he => h e (by simp [he]))
    push_cast; nlinarith

/-- **Total seized value ≤ budget + rounding per leg.** Let `gs` be the legs
of one call (common total collateral `C`, budget `B`, not seize-all), each in
the production domain `SzLegOK`, with `Σ V ≤ C` (production: `C = Σ V`, the
risk totals' half-up collateral over the same legs, `calculate_account_risk_totals_spec`).
Then, with `n` legs,
`2·W·Σ SeizedVal ≤ 10^54·(2·W·B + n·(B + W)) + W·Σ Slack`,
i.e. in USD (WAD) the seized shares are worth at most
`B + n·(B + W)/(2W) + Σ Slack/(2·10^54)`: the budget, plus half a WAD unit
per leg and a relative `n/(2·10^18)` of the budget (the half-up `share` and
leg budget), plus per leg half a token-wei and half a RAY unit at the leg's
price, plus one whole token on the whole-unit round-up branch. The budget
itself is `huD(repay·(1 + bonus))` (`seizure_budget_spec`), half up. -/
theorem seized_total_le_budget (gs : List SzLeg) (C B : Int)
    (hok : ∀ g ∈ gs, SzLegOK g ∧ g.C = C ∧ g.B = B ∧ g.sa = false)
    (hV : lsum SzLeg.V gs ≤ C) (hB0 : 0 ≤ B) :
    2 * 10^18 * lsum SzLeg.SeizedVal gs ≤
      10^54 * (2 * 10^18 * B + gs.length * (B + 10^18)) + 10^18 * lsum SzLeg.Slack gs := by
  -- per leg: 2·SeizedVal ≤ 2·10^54·L + Slack
  have h1 : 2 * lsum SzLeg.SeizedVal gs ≤ 2 * 10^54 * lsum SzLeg.L gs + lsum SzLeg.Slack gs := by
    have : ∀ g ∈ gs, 2 * SzLeg.SeizedVal g ≤ 2 * 10^54 * SzLeg.L g + SzLeg.Slack g := by
      intro g hg
      obtain ⟨h, -, -, hsa⟩ := hok g hg
      unfold SzLeg.SeizedVal SzLeg.Slack
      split_ifs with he
      · have := h.seized_value_le_budget hsa
        rw [if_pos (by assumption)] at this; linarith
      · have := h.seized_value_le_budget hsa
        rw [if_neg (by assumption)] at this; linarith
      · have := h.L0; have := h.p0; have := h.Dd_pos
        have : 0 ≤ 10^27 * g.p * (10^9 + 1 + 2 * (g.Dd - 1)) := by
          apply mul_nonneg (by positivity); omega
        linarith
      · have := h.L0; have := h.p0
        have : 0 ≤ 10^27 * g.p * (10^9 + 1 + 2 * 0) := by positivity
        linarith
    have e1 : lsum (fun g => 2 * SzLeg.SeizedVal g) gs = 2 * lsum SzLeg.SeizedVal gs := by
      exact sz_lsum_mul_left _ _ _
    have e2 : lsum (fun g => 2 * 10^54 * SzLeg.L g + SzLeg.Slack g) gs =
        2 * 10^54 * lsum SzLeg.L gs + lsum SzLeg.Slack gs := by
      exact sz_lsum_lin _ _ _ _
    rw [← e1, ← e2]; exact lsum_le this
  -- 2W·L ≤ 2B·Sh + W
  have h2 : lsum (fun g => 2 * 10^18 * SzLeg.L g) gs ≤ 2 * B * lsum SzLeg.Sh gs + gs.length * 10^18 := by
    apply sz_lsum_bound
    intro g hg
    obtain ⟨-, -, hB, -⟩ := hok g hg
    have := sz_huD_bounds (g.B * g.Sh) (10^18) (by norm_num)
    have hL : g.L = huD (g.B * g.Sh) (10^18) := rfl
    rw [← hL, hB] at this; linarith
  -- 2C·Sh ≤ 2W·V + C
  have h3 : lsum (fun g => 2 * C * SzLeg.Sh g) gs ≤ 2 * 10^18 * lsum SzLeg.V gs + gs.length * C := by
    apply sz_lsum_bound
    intro g hg
    obtain ⟨h, hC, -, -⟩ := hok g hg
    have := sz_huD_bounds (g.V * 10^18) g.C h.C0
    have hS : g.Sh = huD (g.V * 10^18) g.C := rfl
    rw [← hS, hC] at this; linarith
  have eL : lsum (fun g => 2 * 10^18 * SzLeg.L g) gs = 2 * 10^18 * lsum SzLeg.L gs :=
    sz_lsum_mul_left _ _ _
  have eS : lsum (fun g => 2 * C * SzLeg.Sh g) gs = 2 * C * lsum SzLeg.Sh gs :=
    sz_lsum_mul_left _ _ _
  rw [eL] at h2; rw [eS] at h3
  -- Σ Sh ≤ W + n/2 (when there is a leg, C > 0)
  rcases gs.eq_nil_or_concat with hnil | ⟨l, x, hx⟩
  · subst hnil; simp [lsum] at h1 ⊢; linarith
  have hC : 0 < C := by
    obtain ⟨h, hC, -, -⟩ := hok x (by rw [hx]; simp)
    rw [← hC]; exact h.C0
  have hSh : 2 * lsum SzLeg.Sh gs ≤ 2 * 10^18 + gs.length := by
    have : 2 * C * lsum SzLeg.Sh gs ≤ (2 * 10^18 + gs.length) * C := by nlinarith
    by_contra c
    have : (2 * 10^18 + gs.length) * C < 2 * lsum SzLeg.Sh gs * C :=
      mul_lt_mul_of_pos_right (by omega) hC
    nlinarith
  have h4 : 2 * 10^18 * lsum SzLeg.L gs ≤ B * (2 * 10^18 + gs.length) + gs.length * 10^18 := by
    have : 2 * B * lsum SzLeg.Sh gs ≤ B * (2 * 10^18 + gs.length) := by nlinarith
    linarith
  nlinarith

/-- **Every seizure entry is well-formed (INV-LIQ-02).** Under the
hypotheses of `calculate_seized_collateral_spec`, every returned entry comes
from a supply entry `e` of the account (same hub asset) and satisfies:
`0 < scaled_amount ≤` the held scaled shares; `0 ≤ bonus_scaled ≤ scaled_amount`;
`0 < amount ≤ huD(huD(s·i, R), 10^(27−dec))` (the half-up held token amount,
`unscale_supply`); `0 ≤ protocol_fee ≤ amount`. These are the seizure checks
of `LiquidationPlan::validate` except `liquidation_fees < BPS` (a stamped
rate, an input). -/
theorem calculate_seized_collateral_entries (env : Env) (acc : types.Account) (tc : math.fp.Wad)
    (rep : positions.liquidation.math.NormalizedRepaymentPlan) (ctx : context.Context)
    (htc : 0 < tc.val) (hr0 : 0 ≤ rep.repay_usd.val) (hb0 : 0 ≤ rep.bonus.val)
    (hb : rep.bonus.val ≤ 10^8)
    (hfit : szBudget rep.repay_usd.val rep.bonus.val < 2^127)
    (hlegs : ∀ e ∈ acc.supply_positions, Serves ctx e.1 ∧ SzLegOK (planLeg ctx tc.val rep e))
    (huns : lsum (fun e => (planLeg ctx tc.val rep e).Uns) acc.supply_positions < 2^127) :
    positions.liquidation.math.calculate_seized_collateral env acc tc rep ctx ⦃ r =>
      ∀ x ∈ r.1.1, ∃ e ∈ (acc.supply_positions : List _), x.hub_asset = e.1 ∧
        0 < x.scaled_amount.val ∧ x.scaled_amount.val ≤ e.2.scaled_amount.val ∧
        0 ≤ x.bonus_scaled.val ∧ x.bonus_scaled.val ≤ x.scaled_amount.val ∧
        0 < x.amount.val ∧
        x.amount.val ≤ huD (huD (e.2.scaled_amount.val * supplyIndexOf ctx e.1) (10^27))
          (10^(27 - decOf ctx e.1.asset)) ∧
        0 ≤ x.protocol_fee.val ∧ x.protocol_fee.val ≤ x.amount.val ⦄ := by
  apply WP.spec_mono (calculate_seized_collateral_spec env acc tc rep ctx htc hr0 hb0 hb hfit hlegs huns)
  rintro r ⟨-, hout, -⟩ x hx
  obtain ⟨e, he, hem, hk, ha, hf, hs, hbs, -⟩ := SzOut.mem hout x hx
  have h := (hlegs e he).2
  obtain ⟨s0, s1⟩ := h.seized_le hem
  obtain ⟨b0, b1⟩ := h.bsc_le hem
  obtain ⟨a0, f0, f1⟩ := h.fee_le hem
  have hA := h.amt_le
  refine ⟨e, he, hk, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · rw [hs]; exact s0
  · rw [hs]; exact s1
  · rw [hbs]; exact b0
  · rw [hbs, hs]; exact b1
  · rw [ha]; exact a0
  · rw [ha]; exact hA
  · rw [hf]; exact f0
  · rw [hf, ha]; exact f1

end ControllerAeneas.Proofs
