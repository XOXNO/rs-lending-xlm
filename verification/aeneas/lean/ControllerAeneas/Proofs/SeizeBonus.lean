/-
`get_account_bonus_params` and `calculate_seizure_proportions`
(`contracts/controller/src/positions/liquidation/math.rs`): exact values,
the bounds `0 ≤ base ≤ max`, the weighted base bonus lies between the
smallest and the largest stamped bonus, and the result satisfies the bonus
fields of the estimate's domain `EstDomain` (`Estimate.lean`).

A plain (non-`module`) file: it uses the sealed `I128.checked_mul`.
-/
import ControllerAeneas.Proofs.SeizeLoop
import ControllerAeneas.Proofs.Curve

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 2000000

namespace ControllerAeneas.Proofs

/-- Half-up value times stamped liquidation bonus of a typed supply leg. -/
noncomputable def bonW (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  collHU ctx e * e.2.liquidation_bonus.val

/-- Half-up value times stamped liquidation bonus of a raw supply entry. -/
noncomputable def rBonW (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  rHU ctx e * e.2.liquidation_bonus.val

theorem bonus_loop_nil (env : Env) (ctx : context.Context) (a : Std.I128) :
    positions.liquidation.math.get_account_bonus_params_loop [] env ctx a = ok (ctx, a) := by
  unfold positions.liquidation.math.get_account_bonus_params_loop; rw [loop]
  simp [positions.liquidation.math.get_account_bonus_params_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The loop of `get_account_bonus_params` accumulates exactly
`Σ huv · liquidation_bonus`, leaves the context unchanged, and does not abort
while the accumulator plus the remaining terms fits `i128`. -/
theorem bonus_loop_spec (env : Env) (ctx : context.Context) :
    ∀ (L : List (types.HubAssetKey × types.AccountPosition)) (a : Std.I128),
      (∀ e ∈ L, Serves ctx e.1 ∧ VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
        ∧ 0 ≤ e.2.liquidation_bonus.val) →
      0 ≤ a.val → a.val + lsum (bonW ctx) L < 2^127 →
      positions.liquidation.math.get_account_bonus_params_loop L env ctx a ⦃ r =>
        r.1 = ctx ∧ r.2.val = a.val + lsum (bonW ctx) L ⦄ := by
  intro L
  induction L with
  | nil => intro a _ _ _; rw [bonus_loop_nil]; simp
  | cons e L ih =>
    intro a hok ha hfit
    obtain ⟨k, pos⟩ := e
    obtain ⟨hserve, hvd, hlb⟩ := hok (k, pos) (by simp)
    have hrest := fun x (hx : x ∈ L) => hok x (by simp [hx])
    obtain ⟨feed, hfeed, hfp⟩ := cached_price_served ctx k.asset hserve.1
    obtain ⟨mi, hmi, hms, -⟩ := cached_index_served ctx k hserve.2
    have hvd' : VD pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      rw [hms, hfp]; exact hvd
    have hch : collHU ctx (k, pos) = huv pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      simp [collHU, hms, hfp]
    obtain ⟨f0, fh, -, -, -, -⟩ := chains_bounds pos.scaled_amount mi.supply_index feed.price hvd'
    have hv0 : 0 ≤ collHU ctx (k, pos) := by rw [hch]; omega
    have hsum0 : 0 ≤ lsum (bonW ctx) L := by
      unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨⟨_, _⟩, ⟨hs', hi', hp', -, -⟩, hl'⟩ := hrest y hy
      unfold bonW collHU huv
      apply mul_nonneg _ hl'
      apply sz_huD_nonneg _ _ (by norm_num)
      apply mul_nonneg _ hp'
      exact sz_huD_nonneg _ _ (by norm_num) (sz_huD_nonneg _ _ (by norm_num) (mul_nonneg hs' hi'))
    have hb0 : 0 ≤ bonW ctx (k, pos) := mul_nonneg hv0 hlb
    rw [lsum_cons] at hfit
    unfold positions.liquidation.math.get_account_bonus_params_loop
    rw [loop]
    dsimp only
    rw [positions.liquidation.math.get_account_bonus_params_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    simp [hfeed, hmi]
    step with pv_val env pos.scaled_amount mi.supply_index feed.price hvd' as ⟨v, hv⟩
    simp only [math.fp.Wad.raw, math.fp.Bps.raw, bind_ok, lift]
    have hprod : v.val * pos.liquidation_bonus.val = bonW ctx (k, pos) := by
      rw [hv, ← hch]; rfl
    rw [show I128.checked_mul v pos.liquidation_bonus =
      core.num.checked_mul_IScalar v pos.liquidation_bonus from rfl]
    have hm := core.num.checked_mul_IScalar_bv_spec v pos.liquidation_bonus
    cases hmo : core.num.checked_mul_IScalar v pos.liquidation_bonus with
    | none =>
      exfalso; rw [hmo] at hm; apply hm
      simp only [IScalar.min, IScalar.max, IScalarTy.numBits]; rw [hprod]; omega
    | some m =>
      rw [hmo] at hm; obtain ⟨_, _, hmv, _⟩ := hm
      rw [hprod] at hmv
      simp only [core.option.Option.and_then,
        positions.liquidation.math.get_account_bonus_params.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128,
        positions.liquidation.math.get_account_bonus_params.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once,
        bind_ok]
      rw [show I128.checked_add a m = core.num.checked_add_IScalar a m from rfl]
      have hs := core.num.checked_add_IScalar_bv_spec a m
      cases hso : core.num.checked_add_IScalar a m with
      | none =>
        exfalso; rw [hso] at hs; apply hs
        simp only [IScalar.min, IScalar.max, IScalarTy.numBits]; omega
      | some a' =>
        rw [hso] at hs; obtain ⟨_, _, hav, _⟩ := hs
        simp only [core.option.Option.unwrap_or_else, bind_ok]
        have := ih a' hrest (by omega) (by omega)
        unfold positions.liquidation.math.get_account_bonus_params_loop at this
        apply WP.spec_mono this
        rintro ⟨c, r⟩ ⟨hc, hr⟩
        exact ⟨hc, by rw [hr, hav, hmv]; try simp only [lsum_cons]
                      ring⟩

theorem bonW_typed (ctx : context.Context) :
    ∀ {m : List (types.HubAssetKey × types.AccountPositionRaw)}
      {l : List (types.HubAssetKey × types.AccountPosition)},
      List.Forall₂ TypedOf m l → lsum (bonW ctx) l = lsum (rBonW ctx) m :=
  forall2_lsum (by
    rintro ⟨k, r⟩ ⟨k', p⟩ ⟨h1, h2, -, h4, -, -⟩
    simp only at h1 h2 h4
    subst h1
    simp [bonW, rBonW, collHU, rHU, h2, h4])

/-- **`get_account_bonus_params` exact result.** With `max` the result of
`max_bonus_for_threshold(p)` (`max_bonus_for_threshold_spec`):
* no collateral (`C = 0`): `base = 0`;
* `C > 0`: `base = min(huD(Σ huv·bonus, C), max)`: the USD-weighted
  average of the stamped liquidation bonuses, rounded half up once (no
  per-leg rounding), capped at `max`.
The call returns (does not abort) and leaves the context unchanged.
Hypotheses: `C ≥ 0` (a sum of valuations); every supply entry served by the
context with its value in `VD`; the weighted sum `Σ huv·bonus` fits `i128`
(production: values ≤ about `1.7·10^20` USD·BPS; otherwise the code aborts
with `MathOverflow`). -/
theorem get_account_bonus_params_spec (env : Env) (ctx : context.Context)
    (supply : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (tc p : math.fp.Wad) (htc : 0 ≤ tc.val)
    (hs : ∀ e ∈ supply, Serves ctx e.1 ∧ VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset))
    (hfit : lsum (rBonW ctx) supply < 2^127) :
    positions.liquidation.math.get_account_bonus_params env ctx supply tc p ⦃ r =>
      r.2 = ctx ∧
      positions.liquidation.curve.max_bonus_for_threshold env p = ok r.1.max ∧
      (tc.val = 0 → r.1.base.val = 0) ∧
      (0 < tc.val → r.1.base.val = min (huD (lsum (rBonW ctx) supply) tc.val) r.1.max.val) ⦄ := by
  unfold positions.liquidation.math.get_account_bonus_params
  obtain ⟨mx, hmx, -⟩ := spec_imp_eq_ok (max_bonus_for_threshold_spec env p)
  rw [hmx, bind_ok]
  simp only [math.fp.Wad.Insts.CoreCmpPartialEqWad.eq, bind_ok]
  by_cases h0 : tc = math.fp.Wad.ZERO
  · have h0v : tc.val = 0 := by rw [h0, sz_wad_zero_val]
    simp only [h0, decide_true, ↓reduceIte]
    simp [math.fp.Bps.from, core.convert.IntoFrom, core.convert.IntoFrom.into,
      core.convert.FromI128I32, core.convert.num.FromI128I32.from]
    exact ⟨fun _ => rfl, by intro h; rw [sz_wad_zero_val] at h; omega⟩
  · have hpos : 0 < tc.val := by
      rcases lt_or_eq_of_le htc with h | h
      · exact h
      · exfalso; apply h0; apply IScalar.eq_of_val_eq; rw [← h, sz_wad_zero_val]
    simp only [h0, decide_false, Bool.false_eq_true, ↓reduceIte]
    step with iter_typed_positions_spec supply as ⟨L, hL⟩
    have he := bonW_typed ctx hL
    have hLok : ∀ x ∈ L, Serves ctx x.1 ∧ VD x.2.scaled_amount.val (supplyIndexOf ctx x.1)
        (priceOf ctx x.1.asset) ∧ 0 ≤ x.2.liquidation_bonus.val := by
      intro x hx
      obtain ⟨b, hb, h1, h2, -, h4, -, -⟩ := sz_forall2_mem_right hL hx
      obtain ⟨o1, o2⟩ := hs b hb
      refine ⟨by rw [h1]; exact o1, by rw [h1, h2]; exact o2, by rw [h4]; positivity⟩
    step with bonus_loop_spec env ctx L 0#i128 hLok (by simp) (by simp; rw [he]; exact hfit)
      as ⟨c, w, hc, hw⟩
    simp only [IScalar.ofInt_val_eq, zero_add] at hw
    unfold math.fp.Bps.from_weighted_sum
    step with div_by_int_half_up_spec env w tc hpos as ⟨q, hq⟩
    have hw0 : 0 ≤ w.val := by
      rw [hw, he]; unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨⟨_, _⟩, ⟨hs', hi', hp', -, -⟩⟩ := hs y hy
      unfold rBonW rHU huv
      apply mul_nonneg _ (by positivity)
      apply sz_huD_nonneg _ _ (by norm_num)
      apply mul_nonneg _ hp'
      exact sz_huD_nonneg _ _ (by norm_num) (sz_huD_nonneg _ _ (by norm_num) (mul_nonneg hs' hi'))
    unfold IsHalfAway at hq; rw [if_pos hw0] at hq
    simp only [math.fp.Bps.raw, bind_ok, lift, bps_from_same]
    simp only [WP.spec_ok]
    exact ⟨hc, trivial, fun h => by omega,
      fun _ => by rw [sz_i128_min_val, isHalfUp_eq hpos hq, hw, he]⟩

/-- **The base bonus is a weighted average of the stamped bonuses.** If every
supply entry's stamped bonus lies in `[lo, hi]` and the total collateral is
the half-up sum of the leg values (`C = Σ huv`, as `calculate_account_risk_totals`
computes it), then `lo ≤ huD(Σ huv·bonus, C) ≤ hi` (for `C > 0`). With
`get_account_bonus_params_spec`, `base = min(that, max)`. -/
theorem weighted_bonus_between (ctx : context.Context)
    (supply : List (types.HubAssetKey × types.AccountPositionRaw)) (lo hi : Int)
    (hb : ∀ e ∈ supply, lo ≤ e.2.liquidation_bonus.val ∧ (e.2.liquidation_bonus.val : Int) ≤ hi)
    (hv : ∀ e ∈ supply, 0 ≤ rHU ctx e)
    (hC : 0 < lsum (rHU ctx) supply) :
    lo ≤ huD (lsum (rBonW ctx) supply) (lsum (rHU ctx) supply) ∧
    huD (lsum (rBonW ctx) supply) (lsum (rHU ctx) supply) ≤ hi := by
  have h1 : lsum (fun e => lo * rHU ctx e) supply ≤ lsum (rBonW ctx) supply := by
    apply lsum_le; intro e he
    unfold rBonW; have := hb e he; have := hv e he; nlinarith
  have h2 : lsum (rBonW ctx) supply ≤ lsum (fun e => hi * rHU ctx e) supply := by
    apply lsum_le; intro e he
    unfold rBonW; have := hb e he; have := hv e he; nlinarith
  rw [show lsum (fun e => lo * rHU ctx e) supply = lo * lsum (rHU ctx) supply from by
    rw [mul_comm, lsum_mul_right]; congr 1; funext e; ring] at h1
  rw [show lsum (fun e => hi * rHU ctx e) supply = hi * lsum (rHU ctx) supply from by
    rw [mul_comm, lsum_mul_right]; congr 1; funext e; ring] at h2
  have hq := sz_huD_bounds (lsum (rBonW ctx) supply) (lsum (rHU ctx) supply) hC
  constructor
  · by_contra c
    have : (huD (lsum (rBonW ctx) supply) (lsum (rHU ctx) supply) + 1) * lsum (rHU ctx) supply ≤
        lo * lsum (rHU ctx) supply := mul_le_mul_of_nonneg_right (by omega) hC.le
    nlinarith
  · exact sz_huD_le_of _ _ _ hC (by linarith)

/-- **`calculate_seizure_proportions` exact result.** It returns
`p = huD(Wc·W, C)` when `C > 0` and `p = 0` otherwise (`Wad::div`, half
up), the bonus bounds of `get_account_bonus_params` at that `p`, and leaves
the context unchanged. With `0 ≤ Wc ≤ C` (production: weighted collateral
`Σ ⌊fv·LT/BPS⌋ ≤ Σ huv`, `calculate_account_risk_totals_conservative`) the
proportion lies in `[0, W]`, so `max_bonus_for_threshold_bound` applies and
the bounds satisfy the bonus fields of `EstDomain`:
`0 ≤ base ≤ max ≤ BPS·(BPS−1)` and `(BPS + max)·p ≤ BPS·W`. -/
theorem calculate_seizure_proportions_spec (env : Env) (acc : types.Account)
    (tc wc : math.fp.Wad) (ctx : context.Context) (htc : 0 ≤ tc.val) (hwc : 0 ≤ wc.val)
    (hwcC : wc.val ≤ tc.val)
    (hs : ∀ e ∈ acc.supply_positions, Serves ctx e.1 ∧
      VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset))
    (hfit : lsum (rBonW ctx) acc.supply_positions < 2^127) :
    positions.liquidation.math.calculate_seizure_proportions env acc tc wc ctx ⦃ r =>
      r.2 = ctx ∧
      r.1.1.val = (if 0 < tc.val then huD (wc.val * 10^18) tc.val else 0) ∧
      0 ≤ r.1.1.val ∧ r.1.1.val ≤ 10^18 ∧
      positions.liquidation.curve.max_bonus_for_threshold env r.1.1 = ok r.1.2.max ∧
      (tc.val = 0 → r.1.2.base.val = 0) ∧
      (0 < tc.val → r.1.2.base.val =
        min (huD (lsum (rBonW ctx) acc.supply_positions) tc.val) r.1.2.max.val) ∧
      0 ≤ r.1.2.base.val ∧ r.1.2.base.val ≤ r.1.2.max.val ∧
      r.1.2.max.val ≤ 10^4 * (10^4 - 1) ∧
      (10^4 + r.1.2.max.val) * r.1.1.val ≤ 10^4 * 10^18 ⦄ := by
  unfold positions.liquidation.math.calculate_seizure_proportions
  simp only [wad_gt, sz_wad_zero_val, bind_ok]
  -- the proportion
  have hprop : ∃ p : math.fp.Wad,
      (if 0 < tc.val then math.fp.Wad.div wc env tc else ok math.fp.Wad.ZERO) = ok p ∧
      p.val = (if 0 < tc.val then huD (wc.val * 10^18) tc.val else 0) ∧
      0 ≤ p.val ∧ p.val ≤ 10^18 := by
    by_cases hc : 0 < tc.val
    · have hle : huD (wc.val * 10^18) tc.val ≤ 10^18 := sz_huD_le_of _ _ _ hc (by nlinarith)
      have hge : 0 ≤ huD (wc.val * 10^18) tc.val := sz_huD_nonneg _ _ hc (by positivity)
      obtain ⟨p, hp, hpv⟩ := spec_imp_eq_ok (sz_wad_div_val env wc tc hwc hc (by omega))
      refine ⟨p, by simp [hc, hp], by simp [hc, hpv], by omega, by omega⟩
    · exact ⟨math.fp.Wad.ZERO, by simp [hc], by simp [hc, sz_wad_zero_val], by
        rw [sz_wad_zero_val], by rw [sz_wad_zero_val]; norm_num⟩
  obtain ⟨p, hpe, hpv, hp0, hp1⟩ := hprop
  have hpe' : (if decide (0 < tc.val) = true then math.fp.Wad.div wc env tc else ok math.fp.Wad.ZERO) =
      ok p := by simpa using hpe
  rw [hpe', bind_ok]
  step with get_account_bonus_params_spec env ctx acc.supply_positions tc p htc hs hfit as
    ⟨bb, c, hc, hmax, hz, hb⟩
  -- bounds on max
  have hmb : 0 ≤ bb.max.val ∧ bb.max.val ≤ 10^4 * (10^4 - 1) ∧
      (10^4 + bb.max.val) * p.val ≤ 10^4 * 10^18 := by
    by_cases hp : 0 < p.val
    · have := max_bonus_for_threshold_bound env p hp hp1
      rw [hmax, WP.spec_ok] at this; exact this
    · have := max_bonus_for_threshold_spec env p
      rw [hmax, WP.spec_ok] at this
      have hm0 := this.1 (by omega)
      have hpz : p.val = 0 := by omega
      rw [hm0, hpz]; norm_num
  have hbase0 : 0 ≤ bb.base.val := by
    by_cases h : tc.val = 0
    · rw [hz h]
    · have hpos : 0 < tc.val := by omega
      rw [hb hpos]
      apply le_min _ hmb.1
      apply sz_huD_nonneg _ _ hpos
      unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨_, ⟨hs', hi', hp', -, -⟩⟩ := hs y hy
      unfold rBonW rHU huv
      apply mul_nonneg _ (by positivity)
      apply sz_huD_nonneg _ _ (by norm_num)
      apply mul_nonneg _ hp'
      exact sz_huD_nonneg _ _ (by norm_num) (sz_huD_nonneg _ _ (by norm_num) (mul_nonneg hs' hi'))
  have hbasemax : bb.base.val ≤ bb.max.val := by
    by_cases h : tc.val = 0
    · rw [hz h]; exact hmb.1
    · rw [hb (by omega)]; exact min_le_right _ _
  exact ⟨hc, hpv, hp0, hp1, hmax, hz, hb, hbase0, hbasemax, hmb.2.1, hmb.2.2⟩

end ControllerAeneas.Proofs
