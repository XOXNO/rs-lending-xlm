/-
`liquidation::math::calculate_repayment_amounts`: every repayment leg is
capped at the ceiling-rounded debt it repays, the excess is recorded as a
refund of exactly `offered − debt`, so `kept + refunded = offered` per leg,
and the planned USD total is exactly the sum of the recorded leg values (a
leg paid at its cap is credited the floor value of the debt it clears).

Notation: for a debt leg with scaled debt `s`, borrow index `i`, price `p`
and token decimals `dec` (read from the context and the account),
`D = ⌈s·i / (R·10^(27−dec))⌉` is the debt in token units
(`unscale_borrow_ceil`).
-/
import ControllerAeneas.Proofs.Totals
import ControllerAeneas.Proofs.Scaling
import ControllerAeneas.Proofs.HalfUp

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 2000000

namespace ControllerAeneas.Proofs

/-! ## Token amount to WAD USD -/

/-- `Wad::from_token(a, dec)` for `dec ≤ 18` is the exact upscale
`a·10^(18−dec)` whenever that fits `i128`. -/
theorem from_token_spec (env : Env) (a : Std.I128) (dec : Std.U32) (hdec : dec.val ≤ 18)
    (hfit : -2^127 ≤ a.val * 10^(18 - dec.val) ∧ a.val * 10^(18 - dec.val) < 2^127) :
    math.fp.Wad.from_token env a dec ⦃ w => w.val = a.val * 10^(18 - dec.val) ⦄ := by
  unfold math.fp.Wad.from_token math.fp_core.rescale_half_up
  rw [WAD_DECIMALS_eq]
  by_cases h18 : dec.val = 18
  · have he : dec = 18#u32 := by apply UScalar.eq_of_val_eq; simp [h18]
    rw [he, rescale_eq _ env a 18#u32 18#u32 env rfl]
    simp
  · have h := rescale_up_spec
      math.fp_core.rescale_half_up.closure.Insts.CoreOpsFunctionFnPairI128OptionI128I128
      env a dec 18#u32 env (by simp; omega) (by simp; omega) (by simpa using hfit)
    step with h as ⟨r, hr⟩
    simpa using hr

/-- USD value (WAD) of `a` tokens with `dec` decimals at price `p`:
`hu(a·10^(18−dec)·p / W)`. -/
def tokUsd (a : Int) (dec : Nat) (p : Int) : Int := huD (a * 10^(18 - dec) * p) (10^18)

/-- `PriceFeed::usd_value_wad` is `tokUsd` when the intermediate values fit. -/
theorem usd_value_wad_spec (env : Env) (f : types.PriceFeed) (a : Std.I128)
    (ha : 0 ≤ a.val) (hp : 0 ≤ f.price.val) (hdec : f.asset_decimals.val ≤ 18)
    (h1 : a.val * 10^(18 - f.asset_decimals.val) < 2^127)
    (h2 : tokUsd a.val f.asset_decimals.val f.price.val < 2^127) :
    types.PriceFeed.usd_value_wad f env a ⦃ u => u.val = tokUsd a.val f.asset_decimals.val f.price.val ⦄ := by
  unfold types.PriceFeed.usd_value_wad
  have hpow : (0 : Int) < 10^(18 - f.asset_decimals.val) := by positivity
  step with from_token_spec env a f.asset_decimals hdec (by constructor <;> nlinarith) as ⟨w, hw⟩
  unfold math.fp.Wad.mul
  have hW : (constants.production.shared.WAD : Std.I128).val = 10^18 := WAD_val
  have hw0 : 0 ≤ w.val := by rw [hw]; positivity
  obtain ⟨q, hq⟩ := IsHalfUp.exists (w.val * f.price.val) (10^18) (by norm_num)
  have hqe : q = tokUsd a.val f.asset_decimals.val f.price.val := by
    rw [isHalfUp_eq (by norm_num) hq, hw]; rfl
  step with mul_div_half_up_spec env w f.price constants.production.shared.WAD hw0 hp
    (by rw [hW]; norm_num) ⟨q, by rw [hW]; exact hq, by rw [hqe]; exact h2⟩ as ⟨u, hu⟩
  rw [hW] at hu
  rw [← hqe]; exact IsHalfUp.unique (by norm_num) hu hq

/-! ## Per-leg quantities -/

/-- Scaled debt of the account's position for a hub asset (0 if none). -/
noncomputable def debtScaled (acc : types.Account) (k : types.HubAssetKey) : Int :=
  match mapLookup acc.borrow_positions k with
  | some d => d.scaled_amount.val
  | none => 0

/-- Token decimals of the price feed the context serves for an asset. -/
noncomputable def decOf (ctx : context.Context) (a : aeneas_soroban_shim.Address) : Nat :=
  match mapLookup ctx.token_prices a with
  | some r => r.asset_decimals.val
  | none => 0

/-- Debt in token units, `⌈s·i / (R·10^(27−dec))⌉` (`unscale_borrow_ceil`). -/
noncomputable def debtTokens (ctx : context.Context) (acc : types.Account) (k : types.HubAssetKey) : Int :=
  clD (debtScaled acc k * borrowIndexOf ctx k) (10^27 * 10^(27 - decOf ctx k.asset))

/-- Kept repayment of a leg: `min(offered, D)`. -/
noncomputable def legPay (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128) : Int :=
  min l.2.val (debtTokens ctx acc l.1)

/-- Recorded USD value of a leg: the floor value of the whole debt when the
leg pays the cap `D`, otherwise the half-up value of the offered tokens. -/
noncomputable def legUsd (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128) : Int :=
  if debtTokens ctx acc l.1 ≤ l.2.val then
    fv (debtScaled acc l.1) (borrowIndexOf ctx l.1) (priceOf ctx l.1.asset)
  else tokUsd l.2.val (decOf ctx l.1.asset) (priceOf ctx l.1.asset)

/-- Conditions on a merged payment leg `(k, a)`: the context serves `k`, the
account has a debt position for `k`, the debt value is in the value domain,
the asset has at most 18 decimals (`MAX_ASSET_DECIMALS`), `a > 0`
(`aggregate_positive_payments` rejects anything else), and, for a leg paid
below its cap, the USD valuation of `a` tokens fits `i128`. -/
def RepayLegOK (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128) : Prop :=
  Serves ctx l.1 ∧ (∃ d, mapLookup acc.borrow_positions l.1 = some d) ∧
  VD (debtScaled acc l.1) (borrowIndexOf ctx l.1) (priceOf ctx l.1.asset) ∧
  decOf ctx l.1.asset ≤ 18 ∧ 0 < l.2.val ∧
  (l.2.val < debtTokens ctx acc l.1 →
    l.2.val * 10^(18 - decOf ctx l.1.asset) < 2^127 ∧
    tokUsd l.2.val (decOf ctx l.1.asset) (priceOf ctx l.1.asset) < 2^127)

/-- The repayment entry recorded for leg `(k, a)`. -/
def RepayEntryOf (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128)
    (e : types.RepayEntry) : Prop :=
  e.hub_asset = l.1 ∧ e.amount.val = legPay ctx acc l ∧ e.usd_wad.val = legUsd ctx acc l ∧
  e.feed.price_wad.val = priceOf ctx l.1.asset ∧ e.feed.asset_decimals.val = decOf ctx l.1.asset ∧
  e.market_index.borrow_index.val = borrowIndexOf ctx l.1

/-- The refund recorded for a leg offered above its cap: `a − D` of the asset. -/
def RefundOf (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128)
    (r : types.PaymentTuple) : Prop :=
  r.asset = l.1.asset ∧ r.amount.val = l.2.val - debtTokens ctx acc l.1

/-- Legs offered above their cap. -/
noncomputable def overpaid (ctx : context.Context) (acc : types.Account)
    (P : List (types.HubAssetKey × Std.I128)) : List (types.HubAssetKey × Std.I128) :=
  P.filter (fun l => decide (debtTokens ctx acc l.1 < l.2.val))

theorem legUsd_nonneg (ctx : context.Context) (acc : types.Account) (l : types.HubAssetKey × Std.I128)
    (h : RepayLegOK ctx acc l) : 0 ≤ legUsd ctx acc l := by
  obtain ⟨-, -, ⟨hs, hi, hp, -, -⟩, -, ha, -⟩ := h
  unfold legUsd
  split
  · unfold fv
    apply Int.ediv_nonneg _ (by norm_num)
    apply mul_nonneg _ hp
    exact Int.ediv_nonneg (Int.ediv_nonneg (mul_nonneg hs hi) (by norm_num)) (by norm_num)
  · unfold tokUsd huD
    apply Int.ediv_nonneg _ (by norm_num)
    have : 0 ≤ l.2.val * 10 ^ (18 - decOf ctx l.1.asset) * priceOf ctx l.1.asset := by
      apply mul_nonneg (mul_nonneg ha.le (by positivity)) hp
    omega

theorem debtTokens_nonneg (ctx : context.Context) (acc : types.Account) (k : types.HubAssetKey)
    (hs : 0 ≤ debtScaled acc k) (hi : 0 ≤ borrowIndexOf ctx k) : 0 ≤ debtTokens ctx acc k := by
  unfold debtTokens clD
  apply Int.ediv_nonneg _ (by positivity)
  have : 0 ≤ debtScaled acc k * borrowIndexOf ctx k := mul_nonneg hs hi
  have : (1 : Int) ≤ 10^27 * 10^(27 - decOf ctx k.asset) := by
    have : (1 : Int) ≤ 10^(27 - decOf ctx k.asset) := one_le_pow₀ (by norm_num)
    nlinarith
  omega

/-! ## The loop -/

theorem repay_loop_nil (env : Env) (acc : types.Account) (refunds : List types.PaymentTuple)
    (ctx : context.Context) (t : math.fp.Wad) (rep : List types.RepayEntry) :
    positions.liquidation.math.calculate_repayment_amounts_loop [] env acc refunds ctx t rep =
      ok (refunds, ctx, t, rep) := by
  unfold positions.liquidation.math.calculate_repayment_amounts_loop; rw [loop]
  simp [positions.liquidation.math.calculate_repayment_amounts_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The repayment loop: refunds exactly the overpaid legs' excess, records one
entry per leg (capped amount, recorded USD value, feed and index snapshot),
accumulates exactly `Σ legUsd`, and leaves the context unchanged. -/
theorem repay_loop_spec (env : Env) (acc : types.Account) (ctx : context.Context) :
    ∀ (P : List (types.HubAssetKey × Std.I128)) (refunds : List types.PaymentTuple)
      (t : math.fp.Wad) (rep : List types.RepayEntry),
      (∀ l ∈ P, RepayLegOK ctx acc l) → 0 ≤ t.val → t.val + lsum (legUsd ctx acc) P < 2^127 →
      positions.liquidation.math.calculate_repayment_amounts_loop P env acc refunds ctx t rep ⦃ r =>
        r.2.1 = ctx ∧ r.2.2.1.val = t.val + lsum (legUsd ctx acc) P ∧
        (∃ rs, r.1 = refunds ++ rs ∧ List.Forall₂ (RefundOf ctx acc) (overpaid ctx acc P) rs) ∧
        (∃ es, r.2.2.2 = rep ++ es ∧ List.Forall₂ (RepayEntryOf ctx acc) P es) ⦄ := by
  intro P
  induction P with
  | nil =>
    intro refunds t rep _ _ _
    rw [repay_loop_nil]
    simp [overpaid]
  | cons l P ih =>
    intro refunds t rep hok ht hfit
    obtain ⟨k, a⟩ := l
    have hl := hok (k, a) (by simp)
    have hrest : ∀ x ∈ P, RepayLegOK ctx acc x := fun x hx => hok x (by simp [hx])
    obtain ⟨hserve, ⟨dpr, hdpr⟩, hvd, hdec, ha, hpart⟩ := hl
    obtain ⟨⟨pr, hpr⟩, ⟨ix, hix⟩⟩ := hserve
    simp only at hvd hdec ha hpart
    have hS : debtScaled acc k = dpr.scaled_amount.val := by simp [debtScaled, hdpr]
    have hB : borrowIndexOf ctx k = ix.borrow_index.val := by simp [borrowIndexOf, hix]
    have hP : priceOf ctx k.asset = pr.price_wad.val := by simp [priceOf, hpr]
    have hD : decOf ctx k.asset = pr.asset_decimals.val := by simp [decOf, hpr]
    rw [hS, hB, hP] at hvd
    rw [hD] at hdec
    have hvdF := hvd
    obtain ⟨hs0, hi0, hp0, hA, -⟩ := hvd
    have hu0 := legUsd_nonneg ctx acc (k, a) (hok (k, a) (by simp))
    have hsum0 : 0 ≤ lsum (legUsd ctx acc) P := by
      unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      exact legUsd_nonneg ctx acc y (hrest y hy)
    rw [lsum_cons] at hfit
    -- the ceiling of `s·i/R` fits (for `unscale_borrow_ceil`)
    obtain ⟨c1, hc1⟩ := isCeil_exists (dpr.scaled_amount.val * ix.borrow_index.val) (10^27) (by norm_num)
    have hc1' := (isCeil_pos (by norm_num)).mp hc1
    have hsi0 : 0 ≤ dpr.scaled_amount.val * ix.borrow_index.val := mul_nonneg hs0 hi0
    have hc1lo : 0 ≤ c1 := by nlinarith
    have hc1hi : c1 < 2^127 := by nlinarith
    unfold positions.liquidation.math.calculate_repayment_amounts_loop
    rw [loop]
    dsimp only
    rw [positions.liquidation.math.calculate_repayment_amounts_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    simp [cached_price_ok ctx k.asset pr hpr, cached_market_index_ok ctx k ix hix,
      types.HubAssetKey.Insts.CoreCloneClone.clone, aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone,
      lift, core.clone.impls.CloneU32.clone, map_get_eq, hdpr, core.option.Option.unwrap_or_else,
      core.convert.IntoFrom.into, types.DebtPosition.Insts.CoreConvertFromShared0DebtPositionRaw.from]
    step with unscale_borrow_ceil_spec env dpr.scaled_amount ix.borrow_index pr.asset_decimals
      (by omega) ⟨c1, hc1, by omega, hc1hi⟩ as ⟨D, hDc⟩
    have hDeq : D.val = debtTokens ctx acc k := by
      rw [isCeil_eq (by positivity) hDc]; unfold debtTokens; rw [hS, hB, hD]
    have hD0 : 0 ≤ D.val := by rw [hDeq]; apply debtTokens_nonneg <;> omega
    have hfvB := chains_bounds dpr.scaled_amount ix.borrow_index pr.price_wad hvdF
    have hfeedP : (types.PriceFeedRaw.Insts.CoreConvertFromShared0PriceFeed.from
        { price := pr.price_wad, asset_decimals := pr.asset_decimals, timestamp := pr.timestamp })
        = ok pr := by
      simp [types.PriceFeedRaw.Insts.CoreConvertFromShared0PriceFeed.from, math.fp.Wad.raw]
    have hmiP : (types.MarketIndexRaw.Insts.CoreConvertFromShared0MarketIndex.from
        { borrow_index := ix.borrow_index, supply_index := ix.supply_index }) = ok ix := by
      simp [types.MarketIndexRaw.Insts.CoreConvertFromShared0MarketIndex.from, math.fp.Ray.raw]
    have hrest_fit : ∀ (t1 : math.fp.Wad), t1.val = t.val + legUsd ctx acc (k, a) →
        0 ≤ t1.val ∧ t1.val + lsum (legUsd ctx acc) P < 2^127 := by
      intro t1 h; constructor <;> omega
    by_cases hlt : D.val < a.val
    · -- overpaid: refund `a − D`, keep `D`, credit the floor value of the debt
      simp only [hlt, ↓reduceIte]
      step as ⟨ex, hex⟩
      simp only [aeneas_soroban_shim.Vec.push_back, bind_ok]
      simp only [↓reduceIte, hfeedP, hmiP, math.fp.Wad.checked_add, math.fp.Wad.raw, bind_ok]
      have hU : legUsd ctx acc (k, a) = fv dpr.scaled_amount.val ix.borrow_index.val pr.price_wad.val := by
        simp only [legUsd]; rw [if_pos (by rw [← hDeq]; omega), hS, hB, hP]
      step with pvf_val env dpr.scaled_amount ix.borrow_index pr.price_wad hvdF as ⟨v, hv⟩
      step with checked_add_raw_spec env t v (by rw [hv, ← hU]; constructor <;> omega) as ⟨t1, ht1⟩
      obtain ⟨h1, h2⟩ := hrest_fit t1 (by rw [ht1, hv, hU])
      have := ih (refunds ++ [{ asset := k.asset, amount := ex }]) t1
        (rep ++ [{ hub_asset := k, amount := D, usd_wad := v, feed := pr, market_index := ix }])
        hrest h1 h2
      unfold positions.liquidation.math.calculate_repayment_amounts_loop at this
      dsimp only at this
      apply WP.spec_mono this
      rintro ⟨rf, c, tt, rp⟩ ⟨hc, htt, ⟨rs, hrs, hrf⟩, ⟨es, hes, hef⟩⟩
      refine ⟨hc, by rw [htt, ht1, hv, hU]; ring,
        ⟨{ asset := k.asset, amount := ex } :: rs, by rw [hrs]; simp, ?_⟩,
        ⟨{ hub_asset := k, amount := D, usd_wad := v, feed := pr, market_index := ix } :: es,
          by rw [hes]; simp, ?_⟩⟩
      · have : overpaid ctx acc ((k, a) :: P) = (k, a) :: overpaid ctx acc P := by
          simp only [overpaid, List.filter_cons]
          rw [if_pos (by simp only [decide_eq_true_eq]; rw [← hDeq]; exact hlt)]
        rw [this]
        exact List.Forall₂.cons ⟨rfl, by rw [hex, hDeq]⟩ hrf
      · refine List.Forall₂.cons ⟨rfl, ?_, ?_, ?_, ?_, ?_⟩ hef
        · simp only [legPay]; rw [← hDeq]; omega
        · rw [hv, hU]
        · rw [hP]
        · rw [hD]
        · rw [hB]
    · -- paid at most the cap: keep all of `a`
      simp only [hlt, ↓reduceIte, bind_ok]
      have hnot : overpaid ctx acc ((k, a) :: P) = overpaid ctx acc P := by
        simp only [overpaid, List.filter_cons]
        rw [if_neg (by simp only [decide_eq_true_eq]; rw [← hDeq]; exact hlt)]
      by_cases heq : a = D
      · subst heq
        simp [hfeedP, hmiP, math.fp.Wad.checked_add, math.fp.Wad.raw]
        have hU : legUsd ctx acc (k, a) = fv dpr.scaled_amount.val ix.borrow_index.val pr.price_wad.val := by
          simp only [legUsd]; rw [if_pos (by rw [← hDeq]), hS, hB, hP]
        step with pvf_val env dpr.scaled_amount ix.borrow_index pr.price_wad hvdF as ⟨v, hv⟩
        step with checked_add_raw_spec env t v (by rw [hv, ← hU]; constructor <;> omega) as ⟨t1, ht1⟩
        obtain ⟨h1, h2⟩ := hrest_fit t1 (by rw [ht1, hv, hU])
        have := ih refunds t1
          (rep ++ [{ hub_asset := k, amount := a, usd_wad := v, feed := pr, market_index := ix }])
          hrest h1 h2
        unfold positions.liquidation.math.calculate_repayment_amounts_loop at this
        dsimp only at this
        simp only [aeneas_soroban_shim.Vec.push_back, bind_ok]
        apply WP.spec_mono this
        rintro ⟨rf, c, tt, rp⟩ ⟨hc, htt, ⟨rs, hrs, hrf⟩, ⟨es, hes, hef⟩⟩
        refine ⟨hc, by rw [htt, ht1, hv, hU]; ring, ⟨rs, hrs, by rw [hnot]; exact hrf⟩,
          ⟨{ hub_asset := k, amount := a, usd_wad := v, feed := pr, market_index := ix } :: es,
            by rw [hes]; simp, ?_⟩⟩
        refine List.Forall₂.cons ⟨rfl, ?_, ?_, ?_, ?_, ?_⟩ hef
        · simp only [legPay]; rw [← hDeq]; omega
        · rw [hv, hU]
        · rw [hP]
        · rw [hD]
        · rw [hB]
      · have hne : ¬ a.val = D.val := fun h => heq (IScalar.eq_of_val_eq h)
        have hlt2 : a.val < D.val := by omega
        simp [heq, hfeedP, hmiP, math.fp.Wad.checked_add, math.fp.Wad.raw]
        have hU : legUsd ctx acc (k, a) = tokUsd a.val pr.asset_decimals.val pr.price_wad.val := by
          simp only [legUsd]; rw [if_neg (by rw [← hDeq]; omega), hD, hP]
        obtain ⟨hf1, hf2⟩ := hpart (by rw [← hDeq]; exact hlt2)
        rw [hD] at hf1; rw [hD, hP] at hf2
        step with usd_value_wad_spec env
          { price := pr.price_wad, asset_decimals := pr.asset_decimals, timestamp := pr.timestamp }
          a ha.le hp0 hdec hf1 hf2 as ⟨v, hv⟩
        step with checked_add_raw_spec env t v (by rw [hv, ← hU]; constructor <;> omega) as ⟨t1, ht1⟩
        obtain ⟨h1, h2⟩ := hrest_fit t1 (by rw [ht1, hv, hU])
        have := ih refunds t1
          (rep ++ [{ hub_asset := k, amount := a, usd_wad := v, feed := pr, market_index := ix }])
          hrest h1 h2
        unfold positions.liquidation.math.calculate_repayment_amounts_loop at this
        dsimp only at this
        simp only [aeneas_soroban_shim.Vec.push_back, bind_ok]
        apply WP.spec_mono this
        rintro ⟨rf, c, tt, rp⟩ ⟨hc, htt, ⟨rs, hrs, hrf⟩, ⟨es, hes, hef⟩⟩
        refine ⟨hc, by rw [htt, ht1, hv, hU]; ring, ⟨rs, hrs, by rw [hnot]; exact hrf⟩,
          ⟨{ hub_asset := k, amount := a, usd_wad := v, feed := pr, market_index := ix } :: es,
            by rw [hes]; simp, ?_⟩⟩
        refine List.Forall₂.cons ⟨rfl, ?_, ?_, ?_, ?_, ?_⟩ hef
        · simp only [legPay]; rw [← hDeq]; omega
        · rw [hv, hU]
        · rw [hP]
        · rw [hD]
        · rw [hB]

/-- **`calculate_repayment_amounts`: caps, exact refunds and the exact USD
total.** Let `P` be the merged payment list (`aggregate_positive_payments`,
included verbatim; it aborts on an empty list, a non-positive amount or an
overflow). If every merged leg satisfies `RepayLegOK` (context serves it, the
account has that debt position, value domain, ≤ 18 decimals) and the leg
values sum below `2^127`, the call returns (does not abort), leaves the
context unchanged, and:

* records one entry per leg, in order, with `amount = min(offered, D)`
  (`RepayEntryOf`/`legPay`; `D` = ceiling-rounded debt in tokens), the
  leg's recorded USD value `legUsd`, and the feed/index snapshot;
* appends to `refunds` exactly one refund per leg offered above its cap, of
  `offered − D` of that asset (`RefundOf`), in order;
* returns `total = Σ legUsd`, the sum of the recorded `usd_wad` values.

See `repay_entry_le_debt` and `repay_kept_plus_refund` for the per-leg
consequences. -/
theorem calculate_repayment_amounts_spec (env : Env)
    (raw : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128)) (acc : types.Account)
    (refunds : aeneas_soroban_shim.Vec types.PaymentTuple) (ctx : context.Context)
    (P : List (types.HubAssetKey × Std.I128))
    (hagg : payment_legs.aggregate_positive_payments env raw = ok P)
    (hok : ∀ l ∈ P, RepayLegOK ctx acc l) (hfit : lsum (legUsd ctx acc) P < 2^127) :
    positions.liquidation.math.calculate_repayment_amounts env raw acc refunds ctx ⦃ r =>
      r.2.2 = ctx ∧ r.1.1.val = lsum (legUsd ctx acc) P ∧
      (∃ rs, r.2.1 = (refunds : List _) ++ rs ∧ List.Forall₂ (RefundOf ctx acc) (overpaid ctx acc P) rs) ∧
      List.Forall₂ (RepayEntryOf ctx acc) P r.1.2 ⦄ := by
  unfold positions.liquidation.math.calculate_repayment_amounts
  simp only [aeneas_soroban_shim.Vec.new, hagg, bind_ok,
    aeneas_soroban_shim.Vec.Insts.CoreIterTraitsCollectIntoIteratorTVecIter.into_iter]
  have h0 : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  apply WP.spec_bind (repay_loop_spec env acc ctx P refunds math.fp.Wad.ZERO [] hok (by omega)
    (by rw [h0]; omega))
  rintro ⟨rf, c, t, es⟩ ⟨hc, ht, hrf, ⟨es', hes, hef⟩⟩
  simp only at hc ht hrf hes
  simp only [List.nil_append] at hes
  subst hes
  simp [hc, ht, h0, hrf, hef]

/-- A recorded repayment never exceeds the ceiling-rounded debt of its leg
(liquidation never repays more than the debt). -/
theorem repay_entry_le_debt (ctx : context.Context) (acc : types.Account)
    (l : types.HubAssetKey × Std.I128) (e : types.RepayEntry) (h : RepayEntryOf ctx acc l e) :
    e.amount.val ≤ debtTokens ctx acc l.1 ∧ e.amount.val ≤ l.2.val := by
  rw [h.2.1]; unfold legPay; exact ⟨min_le_right _ _, min_le_left _ _⟩

/-- Tokens are conserved per leg: an overpaid leg keeps exactly `D` and
refunds `offered − D` (so kept + refunded = offered); a leg at or below its
cap keeps everything it offered and gets no refund. -/
theorem repay_kept_plus_refund (ctx : context.Context) (acc : types.Account)
    (l : types.HubAssetKey × Std.I128) (e : types.RepayEntry) (h : RepayEntryOf ctx acc l e) :
    (debtTokens ctx acc l.1 < l.2.val → ∀ r, RefundOf ctx acc l r → e.amount.val + r.amount.val = l.2.val) ∧
    (l.2.val ≤ debtTokens ctx acc l.1 → e.amount.val = l.2.val) := by
  rw [h.2.1]; unfold legPay
  refine ⟨fun hlt r hr => ?_, fun hle => ?_⟩
  · rw [hr.2]; rw [min_eq_right hlt.le]; ring
  · exact min_eq_left hle

end ControllerAeneas.Proofs
