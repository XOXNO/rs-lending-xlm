/-
The repayment plan of a liquidation (`liquidation/math.rs`):
`release_unbacked_repayment`, `normalize_repayment_plan`, and the composed
pipeline `calculate_repayment_amounts → (whole-unit raise) →
process_excess_payment → sum_repaid_usd` (`repayment_plan_spec`).
-/
import ControllerAeneas.Proofs.RepayPlanUnit
import ControllerAeneas.Proofs.Estimate

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

/-! ## Facts about the legs `calculate_repayment_amounts` records -/

/-- A leg's recorded value is backed by its kept amount:
`legUsd ≤ hu(legPay·10^(18−dec)·p / W)`. For a leg below its cap both sides
are the half-up value of the offered tokens; for a leg at its cap `D` the
floor value of the debt is at most the value of the ceiling-rounded `D`. -/
theorem legUsd_le_tokUsd (ctx : context.Context) (acc : types.Account)
    (l : types.HubAssetKey × Std.I128) (h : RepayLegOK ctx acc l) :
    legUsd ctx acc l ≤ tokUsd (legPay ctx acc l) (decOf ctx l.1.asset) (priceOf ctx l.1.asset) := by
  obtain ⟨-, -, ⟨hs, hi, hp, -, -⟩, hdec, ha, -⟩ := h
  unfold legUsd legPay
  split
  · rename_i hD
    rw [min_eq_right hD]
    -- fv(s,i,p) ≤ hu(D·S·p/W)
    set n := debtScaled acc l.1 * borrowIndexOf ctx l.1 with hn
    set S : Int := 10^(18 - decOf ctx l.1.asset) with hS
    have hn0 : 0 ≤ n := mul_nonneg hs hi
    have hS0 : 0 < S := by positivity
    have hd : (10:Int)^27 * 10^(27 - decOf ctx l.1.asset) = 10^36 * S := by
      rw [hS, ← pow_add, ← pow_add]; congr 1; omega
    have hDc : n ≤ debtTokens ctx acc l.1 * (10^36 * S) := by
      unfold debtTokens clD
      rw [← hn, hd]
      have hpos : (0:Int) < 10^36 * S := by positivity
      have := Int.lt_ediv_add_one_mul_self (n + 10^36 * S - 1) hpos
      linarith
    have hX : n / 10^27 / 10^9 ≤ debtTokens ctx acc l.1 * S := by
      rw [Int.ediv_ediv_of_nonneg (by norm_num)]
      apply Int.ediv_le_of_le_mul (by norm_num)
      nlinarith
    unfold fv tokUsd
    apply rp_huD_ge_of (by norm_num)
    have h1 : n / 10^27 / 10^9 * priceOf ctx l.1.asset / 10^18 * 10^18 ≤
        n / 10^27 / 10^9 * priceOf ctx l.1.asset := Int.ediv_mul_le _ (by norm_num)
    have h2 : n / 10^27 / 10^9 * priceOf ctx l.1.asset ≤
        debtTokens ctx acc l.1 * S * priceOf ctx l.1.asset := mul_le_mul_of_nonneg_right hX hp
    rw [← hn]; linarith
  · rename_i hD
    rw [min_eq_left (by omega)]

/-- A leg whose kept amount is not positive records value zero (its debt
rounds to zero tokens, so the debt itself is zero). -/
theorem legUsd_zero_of_pay_nonpos (ctx : context.Context) (acc : types.Account)
    (l : types.HubAssetKey × Std.I128) (h : RepayLegOK ctx acc l) (hpay : legPay ctx acc l ≤ 0) :
    legUsd ctx acc l = 0 := by
  obtain ⟨-, -, ⟨hs, hi, hp, -, -⟩, hdec, ha, -⟩ := h
  have hD0 := debtTokens_nonneg ctx acc l.1 hs hi
  unfold legPay at hpay
  have hDz : debtTokens ctx acc l.1 = 0 := by
    rcases le_total l.2.val (debtTokens ctx acc l.1) with h1 | h1
    · rw [min_eq_left h1] at hpay; omega
    · rw [min_eq_right h1] at hpay; omega
  set n := debtScaled acc l.1 * borrowIndexOf ctx l.1 with hn
  have hn0 : 0 ≤ n := mul_nonneg hs hi
  have hdpos : (0:Int) < 10^27 * 10^(27 - decOf ctx l.1.asset) := by positivity
  have hnz : n = 0 := by
    unfold debtTokens clD at hDz
    rw [← hn] at hDz
    have := Int.lt_ediv_add_one_mul_self (n + 10^27 * 10^(27 - decOf ctx l.1.asset) - 1) hdpos
    rw [hDz] at this
    linarith
  unfold legUsd
  rw [if_pos (by omega)]
  unfold fv; rw [← hn, hnz]; simp

/-- Extra domain of a leg the excess loop may trim (positive kept amount and
value): a positive price, the token quotient `usd·W/p` and the kept amount's
WAD upscale and USD value fit `i128`. Production prices are positive and
values are far below `2^127`. -/
def RepayLegTrimOK (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128) :
    Prop :=
  0 < legPay ctx acc l → 0 < legUsd ctx acc l →
    0 < priceOf ctx l.1.asset ∧
    legUsd ctx acc l * 10^18 ≤ (2^127 - 1) * priceOf ctx l.1.asset ∧
    legPay ctx acc l * 10^(18 - decOf ctx l.1.asset) < 2^127 ∧
    tokUsd (legPay ctx acc l) (decOf ctx l.1.asset) (priceOf ctx l.1.asset) < 2^127

/-- An entry recorded by `calculate_repayment_amounts` satisfies the excess
loop's leg conditions, in either mode. -/
theorem entry_excess_ok (ctx : context.Context) (acc : types.Account)
    (l : types.HubAssetKey × Std.I128) (e : types.RepayEntry) (keep : Bool)
    (hl : RepayLegOK ctx acc l) (ht : RepayLegTrimOK ctx acc l) (he : RepayEntryOf ctx acc l e) :
    ExcessLegOK keep e := by
  obtain ⟨-, ha, hu, hp, hd, -⟩ := he
  have hdec := hl.2.2.2.1
  refine ⟨by rw [hu]; exact legUsd_nonneg ctx acc l hl, fun h1 h2 => ?_⟩
  rw [ha] at h1; rw [hu] at h2
  obtain ⟨t1, t2, t3, t4⟩ := ht h1 h2
  refine ⟨by rw [hp]; exact t1, by rw [hd]; exact hdec, by rw [hu, hp]; exact t2,
    by rw [ha, hd]; exact t3, by rw [ha, hd, hp]; exact t4, fun _ => ?_⟩
  rw [hu, ha, hd, hp]; exact legUsd_le_tokUsd ctx acc l hl

/-- Tokens of the planning step: kept amounts plus refunds equal the offered
amounts (`repay_kept_plus_refund`, summed). -/
theorem calc_tokens_conserved (ctx : context.Context) (acc : types.Account) :
    ∀ (P : List (types.HubAssetKey × Std.I128)) (es : List types.RepayEntry)
      (rs : List types.PaymentTuple),
      List.Forall₂ (RepayEntryOf ctx acc) P es →
      List.Forall₂ (RefundOf ctx acc) (overpaid ctx acc P) rs →
      amtSum es + refSum rs = lsum (fun l => l.2.val) P := by
  intro P
  induction P with
  | nil => intro es rs he hr; cases he; cases hr; simp [amtSum, refSum]
  | cons l P ih =>
    intro es rs he hr
    cases he with
    | cons hel hes =>
      rename_i e es'
      have hpay := hel.2.1
      by_cases hov : debtTokens ctx acc l.1 < l.2.val
      · have : overpaid ctx acc (l :: P) = l :: overpaid ctx acc P := by
          simp [overpaid, hov]
        rw [this] at hr
        cases hr with
        | cons hrl hrs =>
          rename_i r rs'
          have := ih es' rs' hes hrs
          have hr2 := hrl.2
          unfold legPay at hpay
          rw [min_eq_right hov.le] at hpay
          simp only [amtSum, refSum, lsum, List.map_cons, List.sum_cons] at this ⊢
          linarith
      · have : overpaid ctx acc (l :: P) = overpaid ctx acc P := by
          simp [overpaid, hov]
        rw [this] at hr
        have := ih es' rs hes hr
        unfold legPay at hpay
        rw [min_eq_left (by omega)] at hpay
        simp only [amtSum, lsum, List.map_cons, List.sum_cons] at this ⊢
        linarith

/-- Membership through `Forall₂`. -/
theorem rp_forall2_mem_right {α β : Type} {R : α → β → Prop} :
    ∀ {l₁ : List α} {l₂ : List β}, List.Forall₂ R l₁ l₂ → ∀ y ∈ l₂, ∃ x ∈ l₁, R x y := by
  intro l₁ l₂ h
  induction h with
  | nil => intro y hy; simp at hy
  | cons hxy _ ih =>
    intro y hy
    rcases List.mem_cons.mp hy with rfl | hm
    · exact ⟨_, by simp, hxy⟩
    · obtain ⟨x, hx, hr⟩ := ih y hm; exact ⟨x, by simp [hx], hr⟩

/-! ## `release_unbacked_repayment` -/

/-- **`release_unbacked_repayment` releases at most the unbacked part, with
exact token refunds.** For a plan whose legs satisfy the excess loop's
conditions without `keep_within_quote` (fewer than `2^32` legs, values
summing below `2^127`):

* `E ≤ 0`: the plan is returned unchanged;
* `E > 0`: the legs and refunds are `process_excess_payment`'s output
  (`ExcessPost` with `keep = false`: kept + refunded = planned per leg,
  removed legs fully refunded), `repay_usd` is recomputed as the exact sum of
  the kept values, `full_close` and `repays_all_debt` are cleared, the bonus
  and `seize_all` are unchanged, and the released value is at most `E`:
  `Σ usd(before) − E ≤ repay_usd ≤ Σ usd(before)`. -/
theorem release_unbacked_repayment_spec (env : Env)
    (plan : positions.liquidation.math.NormalizedRepaymentPlan) (E : math.fp.Wad)
    (hlen : plan.repaid.length < 2^32) (hok : ∀ e ∈ plan.repaid, ExcessLegOK false e)
    (htot : usdSum plan.repaid < 2^127) :
    positions.liquidation.math.release_unbacked_repayment env plan E ⦃ p' =>
      (E.val ≤ 0 → p' = plan) ∧
      (0 < E.val →
        ExcessPost plan.repaid plan.refunds E.val false (p'.repaid, p'.refunds) ∧
        p'.repay_usd.val = usdSum p'.repaid ∧
        usdSum plan.repaid - E.val ≤ p'.repay_usd.val ∧
        p'.repay_usd.val ≤ usdSum plan.repaid ∧
        p'.full_close = false ∧ p'.repays_all_debt = false ∧
        p'.bonus = plan.bonus ∧ p'.seize_all = plan.seize_all) ⦄ := by
  unfold positions.liquidation.math.release_unbacked_repayment
  simp only [wad_le, rp_zero_val]
  by_cases hE : E.val ≤ 0
  · simp only [hE, decide_true, bind_ok, ↓reduceIte]
    rw [WP.spec_ok]
    exact ⟨fun _ => rfl, fun h => absurd h (by omega)⟩
  simp only [hE, decide_false, Bool.false_eq_true, bind_ok, ↓reduceIte]
  have hE0 : 0 ≤ E.val := by omega
  have hx := process_excess_payment_spec env plan.repaid plan.refunds E false hlen hE0 hok
  apply WP.spec_bind hx
  rintro ⟨v, v1⟩ hpost
  obtain ⟨k, hsh, hle, hnk, -⟩ := hpost
  have hnn : ∀ x ∈ v, 0 ≤ x.usd_wad.val := by
    obtain ⟨S, hS, hf⟩ := excess_sublist hsh
    intro x hx
    obtain ⟨s, hs, hsx⟩ := rp_forall2_mem_right hf x hx
    rcases hsx with h | ⟨a, u, h, -, -, hu, -⟩
    · rw [h]; exact (hok s (hS.subset hs)).1
    · rw [h]; exact hu
  have hlt : usdSum v < 2^127 := by simp only at hle; linarith
  step with sum_repaid_usd_nonneg env v hnn hlt as ⟨w, hw, hw0⟩
  refine ⟨fun h => h.elim, fun _ => ⟨⟨k, hsh, hle, hnk, (fun h => by cases h)⟩, hw, ?_, ?_⟩⟩
  · rw [hw]; exact hnk rfl
  · rw [hw]; exact hle

/-! ## `normalize_repayment_plan` -/

/-- A `LegLe` pair has the same one-unit value. -/
theorem LegLe.legUnit {e x : types.RepayEntry} (h : LegLe e x) : legUnit x = legUnit e := by
  rcases h with rfl | ⟨a, u, rfl, -⟩ <;> rfl

/-- A `LegLe` pair has the same feed snapshot. -/
theorem LegLe.feed {e x : types.RepayEntry} (h : LegLe e x) : x.feed = e.feed := by
  rcases h with rfl | ⟨a, u, rfl, -⟩ <;> rfl

/-- A `LegLe` pair has the same market-index snapshot. -/
theorem LegLe.market_index {e x : types.RepayEntry} (h : LegLe e x) :
    x.market_index = e.market_index := by
  rcases h with rfl | ⟨a, u, rfl, -⟩ <;> rfl

/-- A function preserved by `LegLe` maps both sides of a `LegLe` pairing to the same list. -/
theorem rp_forall2_map_eq {S out : List types.RepayEntry} (h : List.Forall₂ LegLe S out)
    (f : types.RepayEntry → Int) (hf : ∀ e x, LegLe e x → f x = f e) :
    out.map f = S.map f := by
  induction h with
  | nil => rfl
  | cons hxy _ ih => simp [ih, hf _ _ hxy]

/-- Sum of a non-negative function over a sublist is at most the sum over the list. -/
theorem rp_lsum_sublist_le {α : Type} {f : α → Int} {S L : List α} (h : S.Sublist L)
    (hf : ∀ x ∈ L, 0 ≤ f x) : lsum f S ≤ lsum f L := by
  unfold lsum
  apply (h.map f).sum_le_sum
  intro a ha; obtain ⟨x, hx, rfl⟩ := List.mem_map.mp ha; exact hf x hx

/-- The part of `normalize_repayment_plan` after the ideal repayment `I` is
known: excess trimming, the USD total, `seize_all`, and the plan record. -/
theorem normalize_tail_spec (env : Env) (acc : types.Account)
    (snap : positions.liquidation.curve.LiquidationSnapshot) (es : List types.RepayEntry)
    (rs : List types.PaymentTuple) (tot I : math.fp.Wad) (bonus : math.fp.Bps)
    (c2 : context.Context)
    (hlen : es.length < 2^32) (htot : tot.val = usdSum es) (hI0 : 0 ≤ I.val)
    (hok : ∀ e ∈ es, ExcessLegOK (decide (snap.total_collateral.val < snap.total_debt.val)) e)
    (hfeed : ∀ e ∈ es, e.feed.asset_decimals.val ≤ 18 ∧ 0 ≤ e.feed.price_wad.val)
    (hzero : ∀ e ∈ es, e.amount.val ≤ 0 → e.usd_wad.val = 0)
    (hfit : usdSum es + lsum legUnit es < 2^127)
    (hrep : ∀ l c, ∃ r, positions.liquidation.math.repays_every_debt_leg env acc l c = ok r) :
    (do
      let (refunds3, final_repayment_tokens) ←
        if decide (snap.total_debt.val ≤ I.val) = true then ok (rs, es)
        else
          if decide (I.val < tot.val) = true then do
            let excess_usd ← math.fp.Wad.checked_sub tot env I
            let (repaid_tokens1, refunds4) ←
              positions.liquidation.math.process_excess_payment env es rs excess_usd
                (decide (snap.total_collateral.val < snap.total_debt.val))
            ok (refunds4, repaid_tokens1)
          else ok (rs, es)
      let repay_usd ← positions.liquidation.math.sum_repaid_usd env final_repayment_tokens
      let seize_all ←
        if decide (snap.total_collateral.val < snap.total_debt.val) = true then
          if decide ((math.fp.Wad.ZERO : Std.I128).val < repay_usd.val) = true then do
            let w ← positions.liquidation.math.one_unit_per_leg_usd env final_repayment_tokens
            let w1 ← math.fp.Wad.checked_add repay_usd env w
            ok (decide (I.val ≤ w1.val))
          else ok false
        else ok false
      if decide (snap.total_debt.val ≤ I.val) = true then do
        let (repays_all_debt, cache3) ←
          positions.liquidation.math.repays_every_debt_leg env acc final_repayment_tokens c2
        ok
          (({ repaid := final_repayment_tokens, refunds := refunds3, repay_usd := repay_usd,
              bonus := bonus, full_close := decide (snap.total_debt.val ≤ I.val),
              repays_all_debt := repays_all_debt, seize_all := seize_all } :
              positions.liquidation.math.NormalizedRepaymentPlan), cache3)
      else
        ok
          (({ repaid := final_repayment_tokens, refunds := refunds3, repay_usd := repay_usd,
              bonus := bonus, full_close := decide (snap.total_debt.val ≤ I.val),
              repays_all_debt := false, seize_all := seize_all } :
              positions.liquidation.math.NormalizedRepaymentPlan), c2)) ⦃ res =>
      ∃ new, res.1.refunds = rs ++ new ∧
        (∃ S, S.Sublist es ∧ List.Forall₂ LegLe S res.1.repaid) ∧
        amtSum res.1.repaid + refSum new = amtSum es ∧
        res.1.repay_usd.val = usdSum res.1.repaid ∧ res.1.repay_usd.val ≤ usdSum es ∧
        res.1.bonus = bonus ∧ res.1.full_close = decide (snap.total_debt.val ≤ I.val) ∧
        (res.1.full_close = true → res.1.repaid = es ∧ new = []) ∧
        (res.1.seize_all = true → snap.total_collateral.val < snap.total_debt.val) ∧
        (res.1.full_close = false → snap.total_collateral.val < snap.total_debt.val →
          res.1.repay_usd.val ≤ I.val) ∧
        (res.1.full_close = false → snap.total_debt.val ≤ snap.total_collateral.val →
          min (usdSum es) I.val ≤ res.1.repay_usd.val) ⦄ := by
  have hes_nn : ∀ e ∈ es, 0 ≤ e.usd_wad.val := fun e he => (hok e he).1
  have hU0 : 0 ≤ lsum legUnit es := by
    unfold lsum; apply List.sum_nonneg
    intro x hx; obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
    exact unitUsd_nonneg _ _ (hfeed y hy).2
  have hS0 : 0 ≤ usdSum es := (usdSum_take_bounds es hes_nn es.length).1.trans (by simp)
  -- Step 1: the trimming `if`.
  have hX : (if decide (snap.total_debt.val ≤ I.val) = true then ok (rs, es)
        else
          if decide (I.val < tot.val) = true then do
            let excess_usd ← math.fp.Wad.checked_sub tot env I
            let (repaid_tokens1, refunds4) ←
              positions.liquidation.math.process_excess_payment env es rs excess_usd
                (decide (snap.total_collateral.val < snap.total_debt.val))
            ok (refunds4, repaid_tokens1)
          else ok (rs, es)) ⦃ out =>
        ∃ new, out.1 = rs ++ new ∧ (∃ S, S.Sublist es ∧ List.Forall₂ LegLe S out.2) ∧
          amtSum out.2 + refSum new = amtSum es ∧ usdSum out.2 ≤ usdSum es ∧
          (snap.total_debt.val ≤ I.val → out.2 = es ∧ new = []) ∧
          (¬ snap.total_debt.val ≤ I.val → snap.total_collateral.val < snap.total_debt.val →
            usdSum out.2 ≤ I.val) ∧
          (¬ snap.total_debt.val ≤ I.val → snap.total_debt.val ≤ snap.total_collateral.val →
            min (usdSum es) I.val ≤ usdSum out.2) ⦄ := by
    have hid : ∃ S, S.Sublist es ∧ List.Forall₂ LegLe S es :=
      ⟨es, List.Sublist.refl _, List.forall₂_same.mpr (fun x _ => Or.inl rfl)⟩
    by_cases hfc : snap.total_debt.val ≤ I.val
    · rw [if_pos (by simpa using hfc), WP.spec_ok]
      exact ⟨[], by simp, hid, by simp [refSum], le_refl _, fun _ => ⟨rfl, rfl⟩,
        fun h => absurd hfc h, fun h => absurd hfc h⟩
    rw [if_neg (by simpa using hfc)]
    by_cases hgt : I.val < tot.val
    swap
    · rw [if_neg (by simpa using hgt), WP.spec_ok]
      refine ⟨[], by simp, hid, by simp [refSum], le_refl _, fun h => absurd h hfc,
        fun _ _ => by rw [← htot]; omega, fun _ _ => min_le_left _ _⟩
    rw [if_pos (by simpa using hgt)]
    unfold math.fp.Wad.checked_sub
    step with checked_sub_nonneg_spec env tot I hI0 hgt.le as ⟨ex, hex⟩
    have hx := process_excess_payment_spec env es rs ex
      (decide (snap.total_collateral.val < snap.total_debt.val)) hlen (by omega) hok
    apply WP.spec_bind hx
    rintro ⟨v, v1⟩ ⟨k, hsh, hle, hnk, hk⟩
    apply (WP.spec_ok (v1, v)).mpr
    obtain ⟨new, hnew, hcons⟩ := excess_tokens_conserved hsh
    obtain ⟨S, hS, hf⟩ := excess_sublist hsh
    refine ⟨new, hnew, ⟨S, hS, hf⟩, hcons, hle, fun h => absurd h hfc, fun _ hins => ?_,
      fun _ hsol => ?_⟩
    · have hk' := hk (by simp [hins])
      rcases hk' with h | ⟨-, hall⟩
      · simp only at h; rw [hex, htot] at h; linarith
      · -- every leg left has value zero
        have : usdSum v = 0 := by
          have hz : ∀ x ∈ v, x.usd_wad.val = 0 := by
            intro x hx
            obtain ⟨s, hs, hsx⟩ := rp_forall2_mem_right hf x hx
            rcases hall x hx with ha | hu
            · rcases hsx with h | ⟨a, u, h, ha', -⟩
              · rw [h] at ha ⊢; exact hzero s (hS.subset hs) ha
              · rw [h] at ha; simp at ha; omega
            · exact hu
          unfold usdSum
          rw [List.sum_eq_zero]
          intro y hy; obtain ⟨x, hx, rfl⟩ := List.mem_map.mp hy; exact hz x hx
        simp only; rw [this]; exact hI0
    · have hnk' := hnk (by simp; omega)
      simp only at hnk' ⊢; rw [hex, htot] at hnk'
      have : usdSum es - (usdSum es - I.val) = I.val := by ring
      rw [this] at hnk'
      exact le_trans (min_le_right _ _) hnk'
  apply WP.spec_bind hX
  rintro ⟨r3, fin⟩ ⟨new, hnew, ⟨S, hS, hf⟩, hcons, hle, hfull, hins_le, hsol_ge⟩
  simp only at hnew hf hcons hle hfull hins_le hsol_ge
  -- Step 2: the USD total.
  have hfin_nn : ∀ x ∈ fin, 0 ≤ x.usd_wad.val := by
    intro x hx
    obtain ⟨s, hs, hsx⟩ := rp_forall2_mem_right hf x hx
    rcases hsx with h | ⟨a, u, h, -, -, hu, -⟩
    · rw [h]; exact hes_nn s (hS.subset hs)
    · rw [h]; exact hu
  step with sum_repaid_usd_nonneg env fin hfin_nn (by linarith) as ⟨w, hw, hw0⟩
  -- Step 3: `seize_all`.
  have hunit : lsum legUnit fin ≤ lsum legUnit es := by
    have : lsum legUnit fin = lsum legUnit S := by
      unfold lsum; rw [rp_forall2_map_eq hf legUnit (fun e x h => h.legUnit)]
    rw [this]
    exact rp_lsum_sublist_le hS (fun x hx => unitUsd_nonneg _ _ (hfeed x hx).2)
  have hfin_feed : ∀ x ∈ fin, x.feed.asset_decimals.val ≤ 18 ∧ 0 ≤ x.feed.price_wad.val := by
    intro x hx
    obtain ⟨s, hs, hsx⟩ := rp_forall2_mem_right hf x hx
    rw [hsx.feed]; exact hfeed s (hS.subset hs)
  have hseize : (if decide (snap.total_collateral.val < snap.total_debt.val) = true then
        if decide ((math.fp.Wad.ZERO : Std.I128).val < w.val) = true then do
          let w2 ← positions.liquidation.math.one_unit_per_leg_usd env fin
          let w1 ← math.fp.Wad.checked_add w env w2
          ok (decide (I.val ≤ w1.val))
        else ok false
      else ok false) ⦃ sa => sa = true → snap.total_collateral.val < snap.total_debt.val ⦄ := by
    by_cases hins : snap.total_collateral.val < snap.total_debt.val
    · simp only [hins, decide_true, ↓reduceIte]
      split
      · have hUf0 : 0 ≤ lsum legUnit fin := by
          unfold lsum; apply List.sum_nonneg
          intro x hx; obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
          exact unitUsd_nonneg _ _ (hfin_feed y hy).2
        step with one_unit_per_leg_usd_spec env fin hfin_feed (by linarith) as ⟨u, hu⟩
        unfold math.fp.Wad.checked_add
        step with checked_add_raw_spec env w u (by rw [hu, hw]; constructor <;> linarith) as ⟨t, ht⟩
        simp [hins]
      · simp
    · simp [hins]
  apply WP.spec_bind hseize
  intro sa hsa
  -- Step 4: the record.
  by_cases hfc : snap.total_debt.val ≤ I.val
  · rw [if_pos (by simpa using hfc)]
    obtain ⟨⟨rad, c3⟩, hr⟩ := hrep fin c2
    rw [hr]
    simp only [bind_ok]
    apply (WP.spec_ok _).mpr
    refine ⟨new, hnew, ⟨S, hS, hf⟩, hcons, hw, by show w.val ≤ _; rw [hw]; exact hle, rfl, rfl,
      fun _ => hfull hfc, hsa, fun h => absurd h (by simp [hfc]), fun h => absurd h (by simp [hfc])⟩
  · rw [if_neg (by simpa using hfc)]
    apply (WP.spec_ok _).mpr
    refine ⟨new, hnew, ⟨S, hS, hf⟩, hcons, hw, by show w.val ≤ _; rw [hw]; exact hle, rfl, rfl,
      fun h => absurd h (by simp [hfc]), hsa, fun _ hins => ?_, fun _ hsol => ?_⟩
    · show w.val ≤ I.val; rw [hw]; exact hins_le hfc hins
    · show _ ≤ w.val; rw [hw]; exact hsol_ge hfc hsol


/-- One-unit value of a merged payment leg's feed. -/
noncomputable def legUnitP (ctx : context.Context) (l : types.HubAssetKey × Std.I128) : Int :=
  unitUsd (decOf ctx l.1.asset) (priceOf ctx l.1.asset)

/-- Forall₂ transports sums. -/
theorem rp_forall2_lsum_eq {α β : Type} {R : α → β → Prop} {f : α → Int} {g : β → Int}
    (hfg : ∀ a b, R a b → g b = f a) :
    ∀ {l₁ : List α} {l₂ : List β}, List.Forall₂ R l₁ l₂ → lsum g l₂ = lsum f l₁ := by
  intro l₁ l₂ h
  induction h with
  | nil => rfl
  | cons hab _ ih => simp [lsum_cons, ih, hfg _ _ hab]

/-- `usdSum` as an `lsum`. -/
theorem rp_usdSum_eq_lsum (l : List types.RepayEntry) : usdSum l = lsum (fun e => e.usd_wad.val) l := rfl

/-- What `normalize_repayment_plan` guarantees (see `normalize_repayment_plan_spec`). -/
def NormSpec (ctx : context.Context) (acc : types.Account) (P : List (types.HubAssetKey × Std.I128))
    (env : Env) (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bb : positions.liquidation.curve.BonusBounds)
    (curve : positions.liquidation.curve.LiquidationCurve)
    (plan : positions.liquidation.math.NormalizedRepaymentPlan) : Prop :=
  ∃ es rs new,
    List.Forall₂ (RepayEntryOf ctx acc) P es ∧
    List.Forall₂ (RefundOf ctx acc) (overpaid ctx acc P) rs ∧
    plan.refunds = rs ++ new ∧
    (∃ S, S.Sublist es ∧ List.Forall₂ LegLe S plan.repaid) ∧
    amtSum plan.repaid + refSum new = amtSum es ∧
    plan.repay_usd.val = usdSum plan.repaid ∧
    plan.repay_usd.val ≤ lsum (legUsd ctx acc) P ∧
    (plan.full_close = true → plan.repaid = es ∧ new = []) ∧
    (plan.seize_all = true → snap.total_collateral.val < snap.total_debt.val) ∧
    ∃ q : math.fp.Wad,
      positions.liquidation.curve.estimate_liquidation_amount env snap bb curve = ok (q, plan.bonus) ∧
      0 ≤ q.val ∧ q.val ≤ snap.total_debt.val ∧ 0 ≤ plan.bonus.val ∧ plan.bonus.val ≤ bb.max.val ∧
      (snap.total_collateral.val < snap.total_debt.val →
        (plan.full_close = true ↔ snap.total_debt.val ≤ q.val) ∧
        (plan.full_close = false → plan.repay_usd.val ≤ q.val)) ∧
      (snap.total_debt.val ≤ snap.total_collateral.val → plan.full_close = false →
        ∃ I : Int, I < snap.total_debt.val ∧ min (lsum (legUsd ctx acc) P) I ≤ plan.repay_usd.val)

/-- **`normalize_repayment_plan`: the normalized plan.** Let `P` be the
merged payment list. Under the planning domain (`RepayLegOK` and
`RepayLegTrimOK` per leg, fewer than `2^32` legs, the leg values plus the
one-unit values sum below `2^127`), the estimate domain `EstDomain`, and
assuming the whole-unit raise and the opaque `repays_every_debt_leg` return
(`hwu`, `hrep`; `whole_unit_repayment_spec` discharges `hwu` on its
domain), the call returns (never aborts) a plan satisfying `NormSpec`:

* the planning step recorded entries `es` (`RepayEntryOf`: kept
  `min(offered, D)`) and refunds `rs` (`RefundOf`); the plan's refunds are
  `rs` followed by the excess refunds `new`;
* the plan's legs are a pointwise-smaller sublist of `es` (`LegLe`), and
  kept + excess-refunded tokens equal the planned tokens exactly;
* `repay_usd` is exactly the sum of the plan's recorded leg values, and at
  most the planned total `Σ legUsd`;
* a full-close plan is not trimmed;
* `seize_all` is only set on an insolvent account;
* the bonus is the estimate's, `q` (the curve quote) is in `[0, D]`; on an
  insolvent account the plan closes in full iff `q ≥ D`, and otherwise
  `repay_usd ≤ q` (never above the collateral-backed quote); on a solvent
  partial plan the ideal `I < D` is reached unless the offer is below it:
  `repay_usd ≥ min(Σ legUsd, I)`. -/
theorem normalize_repayment_plan_spec (env : Env) (acc : types.Account)
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
    (hrep : ∀ l c, ∃ r, positions.liquidation.math.repays_every_debt_leg env acc l c = ok r) :
    positions.liquidation.math.normalize_repayment_plan env acc raw snap bb curve ctx ⦃ res =>
      NormSpec ctx acc P env snap bb curve res.1 ⦄ := by
  have hU0 : 0 ≤ lsum (legUnitP ctx) P := by
    unfold lsum; apply List.sum_nonneg
    intro x hx; obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
    exact unitUsd_nonneg _ _ (hok y hy).2.2.1.2.2.1
  obtain ⟨⟨⟨tot, es⟩, rs0, c1⟩, hcalc, hc1, htot, ⟨rs, hrs, hrf⟩, hes⟩ :=
    spec_imp_eq_ok (calculate_repayment_amounts_spec env raw acc [] ctx P hagg hok (by linarith))
  obtain ⟨⟨q, bonus⟩, hest, hq0, hqD, hb0, hbm, -, -⟩ :=
    spec_imp_eq_ok (estimate_liquidation_amount_contract env snap bb curve hd)
  simp only at hc1 htot hrs hes hq0 hqD hb0 hbm
  simp only [List.nil_append] at hrs
  subst c1 rs0
  -- facts about the planned entries
  have hlen' : es.length < 2^32 := by rw [← hes.length_eq]; exact hlen
  have hsum : usdSum es = lsum (legUsd ctx acc) P :=
    rp_forall2_lsum_eq (fun l e (h : RepayEntryOf ctx acc l e) => h.2.2.1) hes
  have hunits : lsum legUnit es = lsum (legUnitP ctx) P :=
    rp_forall2_lsum_eq (fun l e (h : RepayEntryOf ctx acc l e) => by
      unfold legUnit legUnitP; rw [h.2.2.2.2.1, h.2.2.2.1]) hes
  have htot' : tot.val = usdSum es := by rw [hsum]; exact htot
  have hok' : ∀ (b : Bool), ∀ e ∈ es, ExcessLegOK b e := by
    intro b e he
    obtain ⟨l, hl, hle⟩ := rp_forall2_mem_right hes e he
    exact entry_excess_ok ctx acc l e b (hok l hl) (htrim l hl) hle
  have hfeed : ∀ e ∈ es, e.feed.asset_decimals.val ≤ 18 ∧ 0 ≤ e.feed.price_wad.val := by
    intro e he
    obtain ⟨l, hl, hle⟩ := rp_forall2_mem_right hes e he
    rw [hle.2.2.2.2.1, hle.2.2.2.1]
    exact ⟨(hok l hl).2.2.2.1, (hok l hl).2.2.1.2.2.1⟩
  have hzero : ∀ e ∈ es, e.amount.val ≤ 0 → e.usd_wad.val = 0 := by
    intro e he ha
    obtain ⟨l, hl, hle⟩ := rp_forall2_mem_right hes e he
    rw [hle.2.2.1]; rw [hle.2.1] at ha
    exact legUsd_zero_of_pay_nonpos ctx acc l (hok l hl) ha
  have hfit' : usdSum es + lsum legUnit es < 2^127 := by rw [hsum, hunits]; exact hfit
  have hbase : ∀ plan : positions.liquidation.math.NormalizedRepaymentPlan, ∀ I : math.fp.Wad,
      (∃ new, plan.refunds = rs ++ new ∧
        (∃ S, S.Sublist es ∧ List.Forall₂ LegLe S plan.repaid) ∧
        amtSum plan.repaid + refSum new = amtSum es ∧
        plan.repay_usd.val = usdSum plan.repaid ∧ plan.repay_usd.val ≤ usdSum es ∧
        plan.bonus = bonus ∧ plan.full_close = decide (snap.total_debt.val ≤ I.val) ∧
        (plan.full_close = true → plan.repaid = es ∧ new = []) ∧
        (plan.seize_all = true → snap.total_collateral.val < snap.total_debt.val) ∧
        (plan.full_close = false → snap.total_collateral.val < snap.total_debt.val →
          plan.repay_usd.val ≤ I.val) ∧
        (plan.full_close = false → snap.total_debt.val ≤ snap.total_collateral.val →
          min (usdSum es) I.val ≤ plan.repay_usd.val)) →
      (snap.total_collateral.val < snap.total_debt.val → I = q) →
      NormSpec ctx acc P env snap bb curve plan := by
    intro plan I ⟨new, hnew, hsub, hcons, hrepay, hle, hbon, hfc, hfull, hsa, hins, hsol⟩ hIq
    refine ⟨es, rs, new, hes, hrf, hnew, hsub, hcons, hrepay, by rw [← hsum]; exact hle, hfull,
      hsa, q, by rw [hbon]; exact hest, hq0, hqD, by rw [hbon]; exact hb0,
      by rw [hbon]; exact hbm, fun hC => ?_, fun hC hf => ?_⟩
    · have := hIq hC; subst this
      refine ⟨by rw [hfc]; simp, fun hf => hins hf hC⟩
    · refine ⟨I.val, ?_, by rw [← hsum]; exact hsol hf hC⟩
      rw [hfc] at hf; simpa using hf
  unfold positions.liquidation.math.normalize_repayment_plan
  simp only [aeneas_soroban_shim.Vec.new, bind_ok, hcalc, hest, wad_lt, wad_ge, wad_gt]
  simp only [Std.uncurry_apply_pair]
  by_cases hins : snap.total_collateral.val < snap.total_debt.val
  · rw [if_pos (by simpa using hins)]
    try simp only [bind_ok, Std.uncurry_apply_pair]
    apply WP.spec_mono (normalize_tail_spec env acc snap es rs tot q bonus ctx hlen' htot' hq0
      (hok' _) hfeed hzero hfit' hrep)
    intro res h
    exact hbase res.1 q h (fun _ => rfl)
  · rw [if_neg (by simpa using hins)]
    try simp only [bind_ok, Std.uncurry_apply_pair]
    by_cases hqD' : snap.total_debt.val ≤ q.val
    · rw [if_pos (by simpa using hqD')]
      try simp only [bind_ok, Std.uncurry_apply_pair]
      apply WP.spec_mono (normalize_tail_spec env acc snap es rs tot q bonus ctx hlen' htot' hq0
        (hok' _) hfeed hzero hfit' hrep)
      intro res h
      exact hbase res.1 q h (fun h => absurd h hins)
    · rw [if_neg (by simpa using hqD')]
      obtain ⟨⟨r, c3⟩, hr, hr0⟩ := hwu q bonus hq0 (by omega) (by omega) hb0 hbm
      rw [hr]
      try simp only [bind_ok, Std.uncurry_apply_pair]
      apply WP.spec_mono (normalize_tail_spec env acc snap es rs tot r bonus c3 hlen' htot' hr0
        (hok' _) hfeed hzero hfit' hrep)
      intro res h
      exact hbase res.1 r h (fun h => absurd h hins)

/-! ## The composed repayment plan -/

theorem legPay_nonneg (ctx : context.Context) (acc : types.Account)
    (l : types.HubAssetKey × Std.I128) (h : RepayLegOK ctx acc l) : 0 ≤ legPay ctx acc l := by
  obtain ⟨-, -, ⟨hs, hi, -, -, -⟩, -, ha, -⟩ := h
  have := debtTokens_nonneg ctx acc l.1 hs hi
  unfold legPay; exact le_min ha.le this

/-- **The repayment plan of a liquidation (pipeline theorem).** The
normalized plan built by `normalize_repayment_plan` (planning by
`calculate_repayment_amounts`, the curve quote and whole-unit raise, the
excess trimming of `process_excess_payment`, the total by `sum_repaid_usd`)
satisfies, under the hypotheses of `normalize_repayment_plan_spec`:

1. **per leg**: every repaid leg `x` belongs to a merged payment leg
   `l = (k, offered)` with the same hub asset, and
   `0 ≤ x.amount ≤ D(k)` (the ceiling-rounded debt) and `x.amount ≤ offered`,
   `0 ≤ x.usd_wad ≤ legUsd(l)`, and the feed (price, decimals) and borrow
   index snapshot of `k`;
2. **exact refunds**: `Σ repaid amounts + Σ refund amounts = Σ offered`;
3. **USD total**: `repay_usd = Σ usd_wad` over the repaid legs, with
   `0 ≤ repay_usd ≤ Σ legUsd` (the planned total);
4. **no new legs**: at most one repaid leg per merged payment leg;
5. everything in `NormSpec` (quote, bonus, full-close and insolvency facts). -/
theorem repayment_plan_spec (env : Env) (acc : types.Account)
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
    (hrep : ∀ l c, ∃ r, positions.liquidation.math.repays_every_debt_leg env acc l c = ok r) :
    positions.liquidation.math.normalize_repayment_plan env acc raw snap bb curve ctx ⦃ res =>
      (∀ x ∈ res.1.repaid, ∃ l ∈ P, x.hub_asset = l.1 ∧
        0 ≤ x.amount.val ∧ x.amount.val ≤ debtTokens ctx acc l.1 ∧ x.amount.val ≤ l.2.val ∧
        0 ≤ x.usd_wad.val ∧ x.usd_wad.val ≤ legUsd ctx acc l ∧
        x.feed.price_wad.val = priceOf ctx l.1.asset ∧
        x.feed.asset_decimals.val = decOf ctx l.1.asset ∧
        x.market_index.borrow_index.val = borrowIndexOf ctx l.1) ∧
      amtSum res.1.repaid + refSum res.1.refunds = lsum (fun l => l.2.val) P ∧
      res.1.repay_usd.val = usdSum res.1.repaid ∧
      0 ≤ res.1.repay_usd.val ∧ res.1.repay_usd.val ≤ lsum (legUsd ctx acc) P ∧
      res.1.repaid.length ≤ P.length ∧
      NormSpec ctx acc P env snap bb curve res.1 ⦄ := by
  apply WP.spec_mono (normalize_repayment_plan_spec env acc raw snap bb curve ctx P hagg hok htrim
    hlen hfit hd hwu hrep)
  intro res hn
  have hn' := hn
  obtain ⟨es, rs, new, hes, hrf, hrefs, ⟨S, hS, hf⟩, hcons, hrepay, hle, -, -, -⟩ := hn
  refine ⟨?_, ?_, hrepay, ?_, hle, ?_, hn'⟩
  · intro x hx
    obtain ⟨s, hs, hsx⟩ := rp_forall2_mem_right hf x hx
    obtain ⟨l, hl, hle⟩ := rp_forall2_mem_right hes s (hS.subset hs)
    have hlok := hok l hl
    obtain ⟨h1, h2, h3, h4, h5, h6⟩ := hle
    have hdeb := repay_entry_le_debt ctx acc l s ⟨h1, h2, h3, h4, h5, h6⟩
    refine ⟨l, hl, by rw [hsx.hub_asset, h1], ?_, le_trans hsx.amount_le hdeb.1,
      le_trans hsx.amount_le hdeb.2, ?_, by rw [← h3]; exact hsx.usd_le,
      by rw [hsx.feed, h4], by rw [hsx.feed, h5], by rw [hsx.market_index, h6]⟩
    · rcases hsx with h | ⟨a, u, h, ha, -⟩
      · rw [h, h2]; exact legPay_nonneg ctx acc l hlok
      · rw [h]; exact ha.le
    · rcases hsx with h | ⟨a, u, h, -, -, hu, -⟩
      · rw [h, h3]; exact legUsd_nonneg ctx acc l hlok
      · rw [h]; exact hu
  · have hcalc := calc_tokens_conserved ctx acc P es rs hes hrf
    rw [hrefs]
    simp only [refSum, List.map_append, List.sum_append] at hcons hcalc ⊢
    linarith
  · have hnn : ∀ x ∈ res.1.repaid, 0 ≤ x.usd_wad.val := by
      intro x hx
      obtain ⟨s, hs, hsx⟩ := rp_forall2_mem_right hf x hx
      obtain ⟨l, hl, hle⟩ := rp_forall2_mem_right hes s (hS.subset hs)
      rcases hsx with h | ⟨a, u, h, -, -, hu, -⟩
      · rw [h, hle.2.2.1]; exact legUsd_nonneg ctx acc l (hok l hl)
      · rw [h]; exact hu
    rw [hrepay]
    have := (usdSum_take_bounds res.1.repaid hnn res.1.repaid.length).1
    simpa using this
  · rw [← hf.length_eq, hes.length_eq]; exact hS.length_le

/-- Account-level domain of the whole-unit raise (the parts of
`WholeUnitDom` that do not depend on the quote and the bonus), with the
total debt at most `10^33` WAD (10^15 USD) so that `quote·(1 + bonus)` fits
for every bonus up to `BPS·(BPS − 1)`. -/
structure WholeUnitAcctDom (ctx : context.Context) (acc : types.Account)
    (snap : positions.liquidation.curve.LiquidationSnapshot) : Prop where
  Dmax : snap.total_debt.val ≤ 10^33
  slen : (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).length < 2^32
  supply : ∀ e ∈ (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)),
    ∃ pr ix, mapLookup ctx.token_prices e.1.asset = some pr ∧
      mapLookup ctx.market_indexes e.1 = some ix ∧
      pr.asset_decimals.val ≤ 18 ∧ 0 ≤ pr.price_wad.val ∧ pr.price_wad.val ≤ 10^36 ∧
      0 ≤ e.2.scaled_amount.val ∧ 0 ≤ ix.supply_index.val ∧
      e.2.scaled_amount.val * ix.supply_index.val ≤ 2^126 * 10^27
  borrow : ∀ k ∈ (acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst,
    UnitKeyOK ctx k
  ufit : snap.total_debt.val +
    lsum (keyUnit ctx) ((acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst)
      < 2^127

/-- **The whole-unit hypothesis of `normalize_repayment_plan_spec` holds on
the account-level domain.** With bonus bounds `max ≤ BPS·(BPS − 1)` (as in
`EstDomain`), `whole_unit_repayment` returns, for every quote in `[0, D)`
and bonus in `[0, max]`, a value `r ≥ quote ≥ 0` (`whole_unit_repayment_spec`). -/
theorem whole_unit_returns (env : Env) (acc : types.Account)
    (snap : positions.liquidation.curve.LiquidationSnapshot)
    (bb : positions.liquidation.curve.BonusBounds) (ctx : context.Context)
    (hd : WholeUnitAcctDom ctx acc snap) (hmax : bb.max.val ≤ 10^4 * (10^4 - 1)) :
    ∀ (q : math.fp.Wad) (b : math.fp.Bps), 0 ≤ q.val → q.val < snap.total_debt.val →
      snap.total_debt.val ≤ snap.total_collateral.val → 0 ≤ b.val → b.val ≤ bb.max.val →
      ∃ r : math.fp.Wad × context.Context,
        positions.liquidation.math.whole_unit_repayment env acc snap q b ctx = ok r ∧ 0 ≤ r.1.val := by
  intro q b hq0 hqD _ hb0 hbm
  obtain ⟨hDmax, hslen, hsup, hbor, hufit⟩ := hd
  have hdom : WholeUnitDom ctx acc snap q b :=
    ⟨hq0, hqD, by linarith, hb0, by linarith, hslen, hsup, hbor, by nlinarith, hufit⟩
  obtain ⟨r, hr, hpost⟩ := spec_imp_eq_ok (whole_unit_repayment_spec env acc snap q b ctx hdom)
  exact ⟨r, hr, le_trans hq0 hpost.2.1⟩

end ControllerAeneas.Proofs
