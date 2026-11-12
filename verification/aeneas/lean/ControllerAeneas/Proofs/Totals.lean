/-
`contracts/controller/src/risk/totals.rs`: the account risk totals are exact
sums of per-leg valuations read from the `Context`, and they are conservative
(INV-RISK-02): collateral used by the risk gates is floored (never overstates
the exact value), debt is ceiled (never understates it).

Notation: `R = 10^27`, `B = 10^9`, `W = 10^18`, `BPS = 10^4`. For a leg with
scaled amount `s`, index `i` and price `p` (all read from the context):
`fv s i p = ⌊⌊⌊s·i/R⌋/B⌋·p/W⌋` (floor chain, `position_value_floor`),
`huv` the half-up chain (`position_value`), `cv` the ceiling chain
(`position_value_ceil`).
-/
import ControllerAeneas.Proofs.Model
import ControllerAeneas.Proofs.Value
import ControllerAeneas.Proofs.Saturating

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 1000000

namespace ControllerAeneas.Proofs

/-! ## Closed forms of the three valuation chains -/

/-- Value domain of a leg (the hypotheses of `position_value_*_spec`). -/
def VD (s i p : Int) : Prop :=
  0 ≤ s ∧ 0 ≤ i ∧ 0 ≤ p ∧ s * i + 10^27 ≤ 2^127 * 10^27 ∧
  (s * i + 2 * 10^27 * 10^9) * p + 10^18 * 10^27 * 10^9 ≤ 2^127 * (10^18 * 10^27 * 10^9)

/-- Floor chain `⌊⌊⌊s·i/R⌋/B⌋·p/W⌋`. -/
def fv (s i p : Int) : Int := s * i / 10^27 / 10^9 * p / 10^18
/-- Rounding to nearest, halves up, of `n/d` (`d > 0`). -/
def huD (n d : Int) : Int := (2 * n + d) / (2 * d)
/-- Half-up chain. -/
def huv (s i p : Int) : Int := huD (huD (huD (s * i) (10^27)) (10^9) * p) (10^18)
/-- Ceiling of `n/d` (`d > 0`). -/
def clD (n d : Int) : Int := (n + d - 1) / d
/-- Ceiling chain. -/
def cv (s i p : Int) : Int := clD (clD (clD (s * i) (10^27)) (10^9) * p) (10^18)

theorem isFloor_eq {n d q : Int} (hd : 0 < d) (h : IsFloor n d q) : q = n / d := by
  rw [isFloor_pos hd] at h
  have h1 : q ≤ n / d := (Int.le_ediv_iff_mul_le hd).mpr h.1
  have h2 : n / d < q + 1 := (Int.ediv_lt_iff_lt_mul hd).mpr (by linarith [h.2])
  omega

theorem isHalfUp_eq {n d q : Int} (hd : 0 < d) (h : IsHalfUp n d q) : q = huD n d := by
  rw [IsHalfUp.iff_isFloor hd] at h
  exact isFloor_eq (by omega) h

theorem isCeil_eq {n d q : Int} (hd : 0 < d) (h : IsCeil n d q) : q = clD n d := by
  rw [isCeil_pos hd] at h
  apply isFloor_eq hd
  rw [isFloor_pos hd]; constructor <;> linarith [h.1, h.2]

theorem cv_nonneg (s i p : Int) (hs : 0 ≤ s) (hi : 0 ≤ i) (hp : 0 ≤ p) : 0 ≤ cv s i p := by
  unfold cv clD
  have h1 : 0 ≤ (s * i + 10^27 - 1) / 10^27 :=
    Int.ediv_nonneg (by nlinarith [mul_nonneg hs hi]) (by norm_num)
  have h2 : 0 ≤ ((s * i + 10^27 - 1) / 10^27 + 10^9 - 1) / 10^9 :=
    Int.ediv_nonneg (by omega) (by norm_num)
  exact Int.ediv_nonneg (by nlinarith [mul_nonneg h2 hp]) (by norm_num)

/-- `position_value_floor` returns `fv`. -/
theorem pvf_val (env : Env) (s i p : Std.I128) (h : VD s.val i.val p.val) :
    rates.value.position_value_floor env s i p ⦃ v => v.val = fv s.val i.val p.val ⦄ := by
  obtain ⟨hs, hi, hp, hA, hB⟩ := h
  apply WP.spec_mono (position_value_floor_spec env s i p hs hi hp hA hB)
  rintro v ⟨f1, f2, h1, h2, h3⟩
  unfold fv
  rw [isFloor_eq (by norm_num) h3, isFloor_eq (by norm_num) h2, isFloor_eq (by norm_num) h1]

/-- `position_value` returns `huv`. -/
theorem pv_val (env : Env) (s i p : Std.I128) (h : VD s.val i.val p.val) :
    rates.value.position_value env s i p ⦃ v => v.val = huv s.val i.val p.val ⦄ := by
  obtain ⟨hs, hi, hp, hA, hB⟩ := h
  apply WP.spec_mono (position_value_spec env s i p hs hi hp hA hB)
  rintro v ⟨f1, f2, h1, h2, h3⟩
  unfold huv
  rw [isHalfUp_eq (by norm_num) h3, isHalfUp_eq (by norm_num) h2, isHalfUp_eq (by norm_num) h1]

/-- `position_value_ceil` returns `cv`. -/
theorem pvc_val (env : Env) (s i p : Std.I128) (h : VD s.val i.val p.val) :
    rates.value.position_value_ceil env s i p ⦃ v => v.val = cv s.val i.val p.val ⦄ := by
  obtain ⟨hs, hi, hp, hA, hB⟩ := h
  apply WP.spec_mono (position_value_ceil_spec env s i p hs hi hp hA hB)
  rintro v ⟨f1, f2, h1, h2, h3⟩
  unfold cv
  rw [isCeil_eq (by norm_num) h3, isCeil_eq (by norm_num) h2, isCeil_eq (by norm_num) h1]

/-- Order and exactness of the closed forms on the value domain:
`0 ≤ fv ≤ huv ≤ cv < 2^127` and `fv·RBW ≤ s·i·p ≤ cv·RBW`. -/
theorem chains_bounds (s i p : Std.I128) (h : VD s.val i.val p.val) :
    0 ≤ fv s.val i.val p.val ∧ fv s.val i.val p.val ≤ huv s.val i.val p.val ∧
    huv s.val i.val p.val ≤ cv s.val i.val p.val ∧ cv s.val i.val p.val < 2^127 ∧
    fv s.val i.val p.val * (10^27 * 10^9 * 10^18) ≤ s.val * i.val * p.val ∧
    s.val * i.val * p.val ≤ cv s.val i.val p.val * (10^27 * 10^9 * 10^18) := by
  obtain ⟨hs, hi, hp, hA, hB⟩ := h
  obtain ⟨vf, v, vc, e1, e2, e3, l1, l2⟩ := position_value_bracket () s i p hs hi hp hA hB
  obtain ⟨vf', vc', e1', e3', x1, x2⟩ := position_value_floor_ceil_exact () s i p hs hi hp hA hB
  have hf := pvf_val () s i p ⟨hs, hi, hp, hA, hB⟩
  have hh := pv_val () s i p ⟨hs, hi, hp, hA, hB⟩
  have hc := pvc_val () s i p ⟨hs, hi, hp, hA, hB⟩
  rw [e1, WP.spec_ok] at hf
  rw [e2, WP.spec_ok] at hh
  rw [e3, WP.spec_ok] at hc
  rw [e1] at e1'; rw [e3] at e3'
  obtain rfl : vf = vf' := by simpa using e1'
  obtain rfl : vc = vc' := by simpa using e3'
  have hvc := vc.hBounds
  simp only [IScalarTy.numBits] at hvc
  have hsi : 0 ≤ s.val * i.val := mul_nonneg hs hi
  have f0 : 0 ≤ fv s.val i.val p.val := by
    unfold fv
    apply Int.ediv_nonneg _ (by norm_num)
    apply mul_nonneg _ hp
    exact Int.ediv_nonneg (Int.ediv_nonneg hsi (by norm_num)) (by norm_num)
  refine ⟨f0, ?_, ?_, ?_, ?_, ?_⟩
  · rw [← hf, ← hh]; exact l1
  · rw [← hh, ← hc]; exact l2
  · rw [← hc]; omega
  · rw [← hf]; exact x1
  · rw [← hc]; exact x2

/-! ## Reading legs from the context -/

/-- Price (WAD) the context serves for an asset, 0 if none. -/
noncomputable def priceOf (ctx : context.Context) (a : aeneas_soroban_shim.Address) : Int :=
  match mapLookup ctx.token_prices a with
  | some r => r.price_wad.val
  | none => 0

/-- Supply index the context serves for a hub asset, 0 if none. -/
noncomputable def supplyIndexOf (ctx : context.Context) (k : types.HubAssetKey) : Int :=
  match mapLookup ctx.market_indexes k with
  | some r => r.supply_index.val
  | none => 0

/-- Borrow index the context serves for a hub asset, 0 if none. -/
noncomputable def borrowIndexOf (ctx : context.Context) (k : types.HubAssetKey) : Int :=
  match mapLookup ctx.market_indexes k with
  | some r => r.borrow_index.val
  | none => 0

/-- Floor value of a supply leg. -/
noncomputable def collFloor (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  fv e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- Half-up value of a supply leg. -/
noncomputable def collHU (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  huv e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- Ceiling value of a supply leg (only used to bound sums). -/
noncomputable def collCeil (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  cv e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- LTV-weighted floor value: `⌊fv · min(LTV, LT) / BPS⌋`. -/
noncomputable def ltvWeighted (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  collFloor ctx e * min e.2.loan_to_value.val e.2.liquidation_threshold.val / 10^4
/-- Threshold-weighted floor value: `⌊fv · LT / BPS⌋`. -/
noncomputable def ltWeighted (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  collFloor ctx e * e.2.liquidation_threshold.val / 10^4
/-- Exact (unrounded, scaled by `R·B·W`) value of a supply leg: `s·i·p`. -/
noncomputable def collExact (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Int :=
  e.2.scaled_amount.val * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset

/-- Conditions on a supply leg: the context serves it, its value is in the
value domain, and its stored LTV and threshold are at most 100%. -/
def SupplyLegOK (ctx : context.Context) (e : types.HubAssetKey × types.AccountPosition) : Prop :=
  Serves ctx e.1 ∧ VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) ∧
  0 ≤ e.2.liquidation_threshold.val ∧ e.2.liquidation_threshold.val ≤ 10^4 ∧
  0 ≤ e.2.loan_to_value.val ∧ e.2.loan_to_value.val ≤ 10^4

/-- Sum of a per-leg quantity over a list. -/
def lsum {α : Type} (f : α → Int) (l : List α) : Int := (l.map f).sum

@[simp] theorem lsum_nil {α : Type} (f : α → Int) : lsum f [] = 0 := rfl
@[simp] theorem lsum_cons {α : Type} (f : α → Int) (a : α) (l : List α) :
    lsum f (a :: l) = f a + lsum f l := by simp [lsum]

theorem cached_price_served (ctx : context.Context) (a : aeneas_soroban_shim.Address)
    (h : ∃ p, mapLookup ctx.token_prices a = some p) :
    ∃ f, context.Context.cached_price ctx a = ok (f, ctx) ∧ f.price.val = priceOf ctx a := by
  obtain ⟨p, hp⟩ := h
  exact ⟨_, cached_price_ok ctx a p hp, by simp [priceOf, hp]⟩

theorem cached_index_served (ctx : context.Context) (k : types.HubAssetKey)
    (h : ∃ i, mapLookup ctx.market_indexes k = some i) :
    ∃ m, context.Context.cached_market_index ctx k = ok (m, ctx) ∧
      m.supply_index.val = supplyIndexOf ctx k ∧ m.borrow_index.val = borrowIndexOf ctx k := by
  obtain ⟨i, hi⟩ := h
  exact ⟨_, cached_market_index_ok ctx k i hi, by simp [supplyIndexOf, hi],
    by simp [borrowIndexOf, hi]⟩

@[simp] theorem bps_min (a b : math.fp.Bps) :
    core.cmp.Ord.min.trait_default math.fp.Bps.Insts.CoreCmpOrd a b
      = ok (if b.val < a.val then b else a) := by
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default, core.cmp.Ord.min_body,
    math.fp.Bps.Insts.CoreCmpOrd, math.fp.Bps.Insts.CoreCmpPartialOrdBps,
    core.cmp.PartialOrd.lt.default, core.cmp.PartialOrd.lt_body,
    math.fp.Bps.Insts.CoreCmpPartialOrdBps.partial_cmp, math.fp.Bps.Insts.CoreCmpOrd.cmp,
    core.cmp.impls.OrdI128.cmp, bind_ok, Option.some.injEq]
  by_cases h : b.val < a.val
  · simp [compare_lt_iff_lt.mpr h]; intro h'; omega
  · simp [compare_lt_iff_lt, h]

/-- `apply_to_wad_floor` as a division. -/
theorem apply_to_wad_floor_val (env : Env) (rate : math.fp.Bps) (v : math.fp.Wad)
    (hr : 0 ≤ rate.val) (hrB : rate.val ≤ 10^4) (hv : 0 ≤ v.val) :
    math.fp.Bps.apply_to_wad_floor rate env v ⦃ r => r.val = v.val * rate.val / 10^4 ⦄ := by
  apply WP.spec_mono (apply_to_wad_floor_spec env rate v hr hrB hv)
  intro r hr'; exact isFloor_eq (by norm_num) hr'

/-! ## The collateral loop of `calculate_account_risk_totals_body` -/

theorem totals_loop_nil (env : Env) (ctx : context.Context) (t l w : math.fp.Wad) :
    risk.totals.calculate_account_risk_totals_body_loop [] env ctx t l w = ok (ctx, t, l, w) := by
  unfold risk.totals.calculate_account_risk_totals_body_loop; rw [loop]
  simp [risk.totals.calculate_account_risk_totals_body_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The collateral loop accumulates exactly the three per-leg sums and leaves
the context unchanged, provided each accumulator plus the remaining ceiling
values stays below `2^127`. -/
theorem totals_loop_spec (env : Env) (ctx : context.Context) :
    ∀ (L : List (types.HubAssetKey × types.AccountPosition)) (t l w : math.fp.Wad),
      (∀ e ∈ L, SupplyLegOK ctx e) →
      0 ≤ t.val → 0 ≤ l.val → 0 ≤ w.val →
      t.val + lsum (collCeil ctx) L < 2^127 →
      l.val + lsum (collCeil ctx) L < 2^127 →
      w.val + lsum (collCeil ctx) L < 2^127 →
      risk.totals.calculate_account_risk_totals_body_loop L env ctx t l w ⦃ r =>
        r.1 = ctx ∧ r.2.1.val = t.val + lsum (collHU ctx) L ∧
        r.2.2.1.val = l.val + lsum (ltvWeighted ctx) L ∧
        r.2.2.2.val = w.val + lsum (ltWeighted ctx) L ⦄ := by
  intro L
  induction L with
  | nil =>
    intro t l w _ _ _ _ _ _ _
    rw [totals_loop_nil]; simp
  | cons e L ih =>
    intro t l w hok ht hl hw bt bl bw
    obtain ⟨k, pos⟩ := e
    obtain ⟨hserve, hvd, hlt0, hltB, hltv0, hltvB⟩ := hok (k, pos) (by simp)
    have hrest : ∀ x ∈ L, SupplyLegOK ctx x := fun x hx => hok x (by simp [hx])
    obtain ⟨feed, hfeed, hfp⟩ := cached_price_served ctx k.asset hserve.1
    obtain ⟨mi, hmi, hms, -⟩ := cached_index_served ctx k hserve.2
    have hvd' : VD pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      rw [hms, hfp]; exact hvd
    obtain ⟨f0, fh, hc, cM, -, -⟩ := chains_bounds pos.scaled_amount mi.supply_index feed.price hvd'
    have hcc : collCeil ctx (k, pos) = cv pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      simp [collCeil, hms, hfp]
    have hcf : collFloor ctx (k, pos) = fv pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      simp [collFloor, hms, hfp]
    have hch : collHU ctx (k, pos) = huv pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      simp [collHU, hms, hfp]
    have hsum0 : 0 ≤ lsum (collCeil ctx) L := by
      unfold lsum
      apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨-, ⟨hs', hi', hp', -, -⟩, -⟩ := hrest y hy
      exact cv_nonneg _ _ _ hs' hi' hp'
    unfold risk.totals.calculate_account_risk_totals_body_loop
    rw [loop]
    dsimp only
    rw [risk.totals.calculate_account_risk_totals_body_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    simp [hfeed, hmi, math.fp.Wad.checked_add]
    step with pv_val env pos.scaled_amount mi.supply_index feed.price hvd' as ⟨v, hv⟩
    step with pvf_val env pos.scaled_amount mi.supply_index feed.price hvd' as ⟨g, hg⟩
    step with checked_add_raw_spec env t v (by
      rw [hv]; rw [lsum_cons, hcc] at bt; constructor <;> omega) as ⟨t1, ht1⟩
    have hmin0 : 0 ≤ (if pos.liquidation_threshold.val < pos.loan_to_value.val
        then pos.liquidation_threshold else pos.loan_to_value).val := by split <;> omega
    have hminB : (if pos.liquidation_threshold.val < pos.loan_to_value.val
        then pos.liquidation_threshold else pos.loan_to_value).val ≤ 10^4 := by split <;> omega
    have hminv : (if pos.liquidation_threshold.val < pos.loan_to_value.val
        then pos.liquidation_threshold else pos.loan_to_value).val =
        min pos.loan_to_value.val pos.liquidation_threshold.val := by
      split <;> omega
    have hg0 : 0 ≤ g.val := by rw [hg]; exact f0
    step with apply_to_wad_floor_val env _ g hmin0 hminB hg0 as ⟨w0, hw0⟩
    have hw0le : w0.val ≤ g.val := by
      rw [hw0]
      apply Int.ediv_le_of_le_mul (by norm_num)
      nlinarith
    have hw00 : 0 ≤ w0.val := by rw [hw0]; apply Int.ediv_nonneg (mul_nonneg hg0 hmin0) (by norm_num)
    step with checked_add_raw_spec env l w0 (by
      rw [lsum_cons, hcc] at bl; constructor <;> omega) as ⟨l1, hl1⟩
    step with apply_to_wad_floor_val env _ g hlt0 hltB hg0 as ⟨w2, hw2⟩
    have hw2le : w2.val ≤ g.val := by
      rw [hw2]
      apply Int.ediv_le_of_le_mul (by norm_num)
      nlinarith
    have hw20 : 0 ≤ w2.val := by rw [hw2]; apply Int.ediv_nonneg (mul_nonneg hg0 hlt0) (by norm_num)
    step with checked_add_raw_spec env w w2 (by
      rw [lsum_cons, hcc] at bw; constructor <;> omega) as ⟨w1, hw1⟩
    rw [lsum_cons, hcc] at bt bl bw
    have := ih t1 l1 w1 hrest (by rw [ht1, hv]; omega) (by rw [hl1]; omega) (by rw [hw1]; omega)
      (by rw [ht1, hv]; omega) (by rw [hl1]; omega) (by rw [hw1]; omega)
    unfold risk.totals.calculate_account_risk_totals_body_loop at this
    dsimp only at this
    apply WP.spec_mono this
    rintro ⟨c, a1, a2, a3⟩ ⟨hc1, hc2, hc3, hc4⟩
    refine ⟨hc1, ?_, ?_, ?_⟩
    · rw [hc2, ht1, hv, hch]; ring
    · rw [hc3, hl1, hw0, hminv, ltvWeighted, hcf, hg]; ring
    · rw [hc4, hw1, hw2, ltWeighted, hcf, hg]; ring

theorem totals_loop_nil' (env : Env) (ctx : context.Context) (a : math.fp.Wad) :
    risk.totals.calculate_ltv_collateral_wad_loop [] env ctx a = ok (a, ctx) := by
  unfold risk.totals.calculate_ltv_collateral_wad_loop; rw [loop]
  simp [risk.totals.calculate_ltv_collateral_wad_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The loop of `calculate_ltv_collateral_wad` accumulates exactly
`Σ ⌊fv·min(LTV, LT)/BPS⌋` and leaves the context unchanged. -/
theorem ltv_loop_spec (env : Env) (ctx : context.Context) :
    ∀ (L : List (types.HubAssetKey × types.AccountPosition)) (a : math.fp.Wad),
      (∀ e ∈ L, SupplyLegOK ctx e) → 0 ≤ a.val → a.val + lsum (collCeil ctx) L < 2^127 →
      risk.totals.calculate_ltv_collateral_wad_loop L env ctx a ⦃ r =>
        r.2 = ctx ∧ r.1.val = a.val + lsum (ltvWeighted ctx) L ⦄ := by
  intro L
  induction L with
  | nil => intro a _ _ _; rw [totals_loop_nil']; simp
  | cons e L ih =>
    intro a hok ha ba
    obtain ⟨k, pos⟩ := e
    obtain ⟨hserve, hvd, hlt0, hltB, hltv0, hltvB⟩ := hok (k, pos) (by simp)
    have hrest : ∀ x ∈ L, SupplyLegOK ctx x := fun x hx => hok x (by simp [hx])
    obtain ⟨feed, hfeed, hfp⟩ := cached_price_served ctx k.asset hserve.1
    obtain ⟨mi, hmi, hms, -⟩ := cached_index_served ctx k hserve.2
    have hvd' : VD pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      rw [hms, hfp]; exact hvd
    obtain ⟨f0, -, -, -, -, -⟩ := chains_bounds pos.scaled_amount mi.supply_index feed.price hvd'
    obtain ⟨-, fh, hc, -, -, -⟩ := chains_bounds pos.scaled_amount mi.supply_index feed.price hvd'
    have hcc : collCeil ctx (k, pos) = cv pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      simp [collCeil, hms, hfp]
    have hcf : collFloor ctx (k, pos) = fv pos.scaled_amount.val mi.supply_index.val feed.price.val := by
      simp [collFloor, hms, hfp]
    have hsum0 : 0 ≤ lsum (collCeil ctx) L := by
      unfold lsum
      apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨-, ⟨hs', hi', hp', -, -⟩, -⟩ := hrest y hy
      exact cv_nonneg _ _ _ hs' hi' hp'
    unfold risk.totals.calculate_ltv_collateral_wad_loop
    rw [loop]
    dsimp only
    rw [risk.totals.calculate_ltv_collateral_wad_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    simp [hfeed, hmi, math.fp.Wad.checked_add]
    step with pvf_val env pos.scaled_amount mi.supply_index feed.price hvd' as ⟨g, hg⟩
    have hmin0 : 0 ≤ (if pos.liquidation_threshold.val < pos.loan_to_value.val
        then pos.liquidation_threshold else pos.loan_to_value).val := by split <;> omega
    have hminB : (if pos.liquidation_threshold.val < pos.loan_to_value.val
        then pos.liquidation_threshold else pos.loan_to_value).val ≤ 10^4 := by split <;> omega
    have hminv : (if pos.liquidation_threshold.val < pos.loan_to_value.val
        then pos.liquidation_threshold else pos.loan_to_value).val =
        min pos.loan_to_value.val pos.liquidation_threshold.val := by
      split <;> omega
    have hg0 : 0 ≤ g.val := by rw [hg]; exact f0
    step with apply_to_wad_floor_val env _ g hmin0 hminB hg0 as ⟨w0, hw0⟩
    have hw0le : w0.val ≤ g.val := by
      rw [hw0]
      apply Int.ediv_le_of_le_mul (by norm_num)
      nlinarith
    have hw00 : 0 ≤ w0.val := by rw [hw0]; apply Int.ediv_nonneg (mul_nonneg hg0 hmin0) (by norm_num)
    rw [lsum_cons, hcc] at ba
    step with checked_add_raw_spec env a w0 (by constructor <;> omega) as ⟨a1, ha1⟩
    have := ih a1 hrest (by rw [ha1]; omega) (by rw [ha1]; omega)
    unfold risk.totals.calculate_ltv_collateral_wad_loop at this
    dsimp only at this
    apply WP.spec_mono this
    rintro ⟨r1, c⟩ ⟨hc1, hc2⟩
    refine ⟨hc1, ?_⟩
    rw [hc2, ha1, hw0, hminv, ltvWeighted, hcf, hg]; ring

/-! ## The debt loop of `sum_debt_usd_loaded` -/

/-- The valuation a `DebtValue` names: half-up or ceiling chain. -/
def debtVal (v : risk.totals.DebtValue) (s i p : Int) : Int :=
  match v with
  | .HalfUp => huv s i p
  | .Ceil => cv s i p

/-- Value of a debt leg under valuation `v`, read from the context. -/
noncomputable def debtLeg (v : risk.totals.DebtValue) (ctx : context.Context)
    (e : types.HubAssetKey × types.DebtPosition) : Int :=
  debtVal v e.2.scaled_amount.val (borrowIndexOf ctx e.1) (priceOf ctx e.1.asset)

/-- Exact (scaled by `R·B·W`) value of a debt leg: `s·i·p`. -/
noncomputable def debtExact (ctx : context.Context) (e : types.HubAssetKey × types.DebtPosition) : Int :=
  e.2.scaled_amount.val * borrowIndexOf ctx e.1 * priceOf ctx e.1.asset

/-- A debt leg is served by the context and its value is in the value domain. -/
def DebtLegOK (ctx : context.Context) (e : types.HubAssetKey × types.DebtPosition) : Prop :=
  Serves ctx e.1 ∧ VD e.2.scaled_amount.val (borrowIndexOf ctx e.1) (priceOf ctx e.1.asset)

theorem debtVal_spec (env : Env) (v : risk.totals.DebtValue) (s i p : Std.I128)
    (h : VD s.val i.val p.val) :
    risk.totals.DebtValue.apply v env s i p ⦃ r => r.val = debtVal v s.val i.val p.val ⦄ := by
  cases v
  · simp only [risk.totals.DebtValue.apply, debtVal]; exact pv_val env s i p h
  · simp only [risk.totals.DebtValue.apply, debtVal]; exact pvc_val env s i p h

theorem debt_loop_nil (env : Env) (ctx : context.Context) (v : risk.totals.DebtValue) (a : math.fp.Wad) :
    risk.totals.sum_debt_usd_loaded_loop [] env ctx v a = ok (a, ctx) := by
  unfold risk.totals.sum_debt_usd_loaded_loop; rw [loop]
  simp [risk.totals.sum_debt_usd_loaded_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The debt loop accumulates exactly `Σ value(leg)` and leaves the context unchanged. -/
theorem debt_loop_spec (env : Env) (ctx : context.Context) (v : risk.totals.DebtValue) :
    ∀ (L : List (types.HubAssetKey × types.DebtPosition)) (a : math.fp.Wad),
      (∀ e ∈ L, DebtLegOK ctx e) → 0 ≤ a.val → a.val + lsum (debtLeg .Ceil ctx) L < 2^127 →
      risk.totals.sum_debt_usd_loaded_loop L env ctx v a ⦃ r =>
        r.2 = ctx ∧ r.1.val = a.val + lsum (debtLeg v ctx) L ⦄ := by
  intro L
  induction L with
  | nil => intro a _ _ _; rw [debt_loop_nil]; simp
  | cons e L ih =>
    intro a hok ha ba
    obtain ⟨k, pos⟩ := e
    obtain ⟨hserve, hvd⟩ := hok (k, pos) (by simp)
    have hrest : ∀ x ∈ L, DebtLegOK ctx x := fun x hx => hok x (by simp [hx])
    obtain ⟨feed, hfeed, hfp⟩ := cached_price_served ctx k.asset hserve.1
    obtain ⟨mi, hmi, -, hmb⟩ := cached_index_served ctx k hserve.2
    have hvd' : VD pos.scaled_amount.val mi.borrow_index.val feed.price.val := by
      rw [hmb, hfp]; exact hvd
    obtain ⟨f0, fh, hc, -, -, -⟩ := chains_bounds pos.scaled_amount mi.borrow_index feed.price hvd'
    have hcc : debtLeg .Ceil ctx (k, pos) = cv pos.scaled_amount.val mi.borrow_index.val feed.price.val := by
      simp [debtLeg, debtVal, hmb, hfp]
    have hcv : debtLeg v ctx (k, pos) = debtVal v pos.scaled_amount.val mi.borrow_index.val feed.price.val := by
      simp [debtLeg, hmb, hfp]
    have hsum0 : 0 ≤ lsum (debtLeg .Ceil ctx) L := by
      unfold lsum
      apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨-, ⟨hs', hi', hp', -, -⟩⟩ := hrest y hy
      exact cv_nonneg _ _ _ hs' hi' hp'
    have hvb : 0 ≤ debtVal v pos.scaled_amount.val mi.borrow_index.val feed.price.val ∧
        debtVal v pos.scaled_amount.val mi.borrow_index.val feed.price.val ≤
          cv pos.scaled_amount.val mi.borrow_index.val feed.price.val := by
      cases v <;> simp only [debtVal] <;> constructor <;> omega
    unfold risk.totals.sum_debt_usd_loaded_loop
    rw [loop]
    dsimp only
    rw [risk.totals.sum_debt_usd_loaded_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    simp [hfeed, hmi, math.fp.Wad.checked_add]
    step with debtVal_spec env v pos.scaled_amount mi.borrow_index feed.price hvd' as ⟨d, hd⟩
    rw [lsum_cons, hcc] at ba
    step with checked_add_raw_spec env a d (by rw [hd]; constructor <;> omega) as ⟨a1, ha1⟩
    have := ih a1 hrest (by rw [ha1, hd]; omega) (by rw [ha1, hd]; omega)
    unfold risk.totals.sum_debt_usd_loaded_loop at this
    dsimp only at this
    apply WP.spec_mono this
    rintro ⟨r1, c⟩ ⟨hc1, hc2⟩
    refine ⟨hc1, ?_⟩
    rw [hc2, ha1, hd, hcv]; ring

/-! ## Raw-map statements

The production functions take the raw position maps; these are the per-leg
quantities in terms of a raw entry `(key, raw position)`. -/

/-- Floor value of a raw supply entry. -/
noncomputable def rFloor (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  fv e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- Half-up value of a raw supply entry. -/
noncomputable def rHU (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  huv e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- Ceiling value of a raw supply entry. -/
noncomputable def rCeil (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  cv e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- Exact value `s·i·p` (scaled by `R·B·W`) of a raw supply entry. -/
noncomputable def rExact (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  e.2.scaled_amount.val * supplyIndexOf ctx e.1 * priceOf ctx e.1.asset
/-- Stored liquidation threshold (BPS) of a raw entry. -/
def rLT (e : types.HubAssetKey × types.AccountPositionRaw) : Int := e.2.liquidation_threshold.val
/-- Stored `min(LTV, liquidation threshold)` (BPS) of a raw entry. -/
def rEffLTV (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  min (e.2.loan_to_value.val : Int) e.2.liquidation_threshold.val
/-- `⌊rFloor · min(LTV, LT) / BPS⌋`. -/
noncomputable def rLtv (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  rFloor ctx e * rEffLTV e / 10^4
/-- `⌊rFloor · LT / BPS⌋`. -/
noncomputable def rLt (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Int :=
  rFloor ctx e * rLT e / 10^4

/-- Conditions on a raw supply entry: served by the context, value domain,
stored LTV and threshold at most 100% (`validate_asset_config`). -/
def RawSupplyOK (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw) : Prop :=
  Serves ctx e.1 ∧ VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset) ∧
  (e.2.liquidation_threshold.val : Int) ≤ 10^4 ∧ (e.2.loan_to_value.val : Int) ≤ 10^4

/-- Value of a raw debt entry under valuation `v`. -/
noncomputable def rDebt (v : risk.totals.DebtValue) (ctx : context.Context)
    (e : types.HubAssetKey × types.DebtPositionRaw) : Int :=
  debtVal v e.2.scaled_amount.val (borrowIndexOf ctx e.1) (priceOf ctx e.1.asset)
/-- Exact value `s·i·p` (scaled by `R·B·W`) of a raw debt entry. -/
noncomputable def rDebtExact (ctx : context.Context) (e : types.HubAssetKey × types.DebtPositionRaw) : Int :=
  e.2.scaled_amount.val * borrowIndexOf ctx e.1 * priceOf ctx e.1.asset
/-- Conditions on a raw debt entry: served by the context and in the value domain. -/
def RawDebtOK (ctx : context.Context) (e : types.HubAssetKey × types.DebtPositionRaw) : Prop :=
  Serves ctx e.1 ∧ VD e.2.scaled_amount.val (borrowIndexOf ctx e.1) (priceOf ctx e.1.asset)

theorem forall2_lsum {f : types.HubAssetKey × types.AccountPosition → Int}
    {g : types.HubAssetKey × types.AccountPositionRaw → Int}
    (h : ∀ e t, TypedOf e t → f t = g e) :
    ∀ {m L}, List.Forall₂ TypedOf m L → lsum f L = lsum g m := by
  intro m L hf
  induction hf with
  | nil => rfl
  | cons hr _ ih => simp [lsum_cons, ih, h _ _ hr]

theorem forall2_ok {ctx : context.Context} :
    ∀ {m L}, List.Forall₂ TypedOf m L → (∀ e ∈ m, RawSupplyOK ctx e) →
      ∀ t ∈ L, SupplyLegOK ctx t := by
  intro m L hf
  induction hf with
  | nil => intro _ t ht; simp at ht
  | @cons e t m L hr _ ih =>
    intro hok x hx
    rcases List.mem_cons.mp hx with rfl | hx
    · obtain ⟨h1, h2, h3, h4, h5, h6⟩ := hr
      obtain ⟨o1, o2, o3, o4⟩ := hok e (by simp)
      refine ⟨by rw [h1]; exact o1, by rw [h1, h2]; exact o2, ?_, ?_, ?_, ?_⟩
      · rw [h3]; positivity
      · rw [h3]; exact o3
      · rw [h5]; positivity
      · rw [h5]; exact o4
    · exact ih (fun y hy => hok y (by simp [hy])) x hx

theorem typed_sums {ctx : context.Context} {m L} (hf : List.Forall₂ TypedOf m L) :
    lsum (collHU ctx) L = lsum (rHU ctx) m ∧ lsum (collCeil ctx) L = lsum (rCeil ctx) m ∧
    lsum (ltvWeighted ctx) L = lsum (rLtv ctx) m ∧ lsum (ltWeighted ctx) L = lsum (rLt ctx) m := by
  refine ⟨forall2_lsum ?_ hf, forall2_lsum ?_ hf, forall2_lsum ?_ hf, forall2_lsum ?_ hf⟩ <;>
  · rintro ⟨k, r⟩ ⟨k', p⟩ ⟨h1, h2, h3, h4, h5, h6⟩
    simp only at h1 h2 h3 h4 h5 h6
    subst h1
    simp [collHU, rHU, collCeil, rCeil, ltvWeighted, rLtv, ltWeighted, rLt, collFloor, rFloor,
      rLT, rEffLTV, h2, h3, h5]

theorem debt_sums (v : risk.totals.DebtValue) (ctx : context.Context)
    (m : List (types.HubAssetKey × types.DebtPositionRaw)) :
    lsum (debtLeg v ctx) (m.map (fun e => (e.1, ({ scaled_amount := e.2.scaled_amount } : types.DebtPosition))))
      = lsum (rDebt v ctx) m := by
  simp only [lsum, List.map_map]; rfl

theorem raw_debt_ok {ctx : context.Context} {m : List (types.HubAssetKey × types.DebtPositionRaw)}
    (h : ∀ e ∈ m, RawDebtOK ctx e) :
    ∀ t ∈ m.map (fun e => (e.1, ({ scaled_amount := e.2.scaled_amount } : types.DebtPosition))),
      DebtLegOK ctx t := by
  intro t ht
  obtain ⟨e, he, rfl⟩ := List.mem_map.mp ht
  exact h e he

/-- `Wad::div_floor_saturating` of non-negative by positive: `min(i128::MAX, ⌊a·W/b⌋)`. -/
theorem div_floor_saturating_val (env : Env) (a b : math.fp.Wad) (ha : 0 ≤ a.val) (hb : 0 < b.val) :
    math.fp.Wad.div_floor_saturating a env b ⦃ r => r.val = min (2^127 - 1) (a.val * 10^18 / b.val) ⦄ := by
  unfold math.fp.Wad.div_floor_saturating
  have hW : (constants.production.shared.WAD : Std.I128).val = 10^18 := WAD_val
  step with mul_div_floor_saturating_spec env a constants.production.shared.WAD b (by omega) as ⟨r, hr⟩
  have hq : IsFloor (a.val * (constants.production.shared.WAD : Std.I128).val) b.val
      (a.val * 10^18 / b.val) := by
    rw [hW, isFloor_pos hb]
    constructor
    · exact Int.ediv_mul_le _ (by omega)
    · have := Int.lt_ediv_add_one_mul_self (a.val * 10^18) hb
      linarith
  have h0 : 0 ≤ a.val * 10^18 / b.val := Int.ediv_nonneg (by positivity) (by omega)
  rw [hr _ hq]; omega

theorem mem_keys_serves {α : Type} {ctx : context.Context} {m : List (types.HubAssetKey × α)}
    {P : types.HubAssetKey × α → Prop} (h : ∀ e ∈ m, P e) (hP : ∀ e, P e → Serves ctx e.1) :
    ∀ k ∈ m.map Prod.fst, Serves ctx k := by
  intro k hk
  obtain ⟨e, he, rfl⟩ := List.mem_map.mp hk
  exact hP e (h e he)

theorem rCeil_sum_nonneg (ctx : context.Context) (m : List (types.HubAssetKey × types.AccountPositionRaw))
    (h : ∀ e ∈ m, RawSupplyOK ctx e) : 0 ≤ lsum (rCeil ctx) m := by
  unfold lsum; apply List.sum_nonneg
  intro x hx
  obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
  obtain ⟨-, ⟨hs', hi', hp', -, -⟩, -⟩ := h y hy
  exact cv_nonneg _ _ _ hs' hi' hp'

theorem rDebt_sum_nonneg (ctx : context.Context) (m : List (types.HubAssetKey × types.DebtPositionRaw))
    (h : ∀ e ∈ m, RawDebtOK ctx e) : 0 ≤ lsum (rDebt .Ceil ctx) m := by
  unfold lsum; apply List.sum_nonneg
  intro x hx
  obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
  obtain ⟨-, ⟨hs', hi', hp', -, -⟩⟩ := h y hy
  exact cv_nonneg _ _ _ hs' hi' hp'

/-- **`calculate_account_risk_totals` is the exact sum of the per-leg
valuations.** For a context that serves every position (as it does after
`load_markets`, which the function calls first and which then changes
nothing), the call returns (does not abort), leaves the context unchanged, and:

* `total_collateral = Σ huv(s, supply_index, price)` (half-up),
* `ltv_collateral = Σ ⌊fv · min(LTV, LT) / BPS⌋` (floor value, floor weighting),
* `weighted_collateral = Σ ⌊fv · LT / BPS⌋`,
* `total_debt = Σ cv(s, borrow_index, price)` (ceiling),
* `health_factor = i128::MAX` without debt, otherwise
  `min(i128::MAX, ⌊weighted · W / total_debt⌋)`.

Hypotheses: every supply and debt entry is served by the context and its value
is in the value domain `VD`; stored LTV and threshold are ≤ 100%; the ceiling
values of all supply legs and of all debt legs each sum below `2^127` (then no
`checked_add` can overflow). -/
theorem calculate_account_risk_totals_spec (env : Env) (ctx : context.Context)
    (supply : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (borrow : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw)
    (hs : ∀ e ∈ supply, RawSupplyOK ctx e) (hb : ∀ e ∈ borrow, RawDebtOK ctx e)
    (hsfit : lsum (rCeil ctx) supply < 2^127) (hbfit : lsum (rDebt .Ceil ctx) borrow < 2^127) :
    risk.totals.calculate_account_risk_totals env ctx supply borrow ⦃ r =>
      r.2 = ctx ∧
      r.1.total_collateral.val = lsum (rHU ctx) supply ∧
      r.1.ltv_collateral.val = lsum (rLtv ctx) supply ∧
      r.1.weighted_collateral.val = lsum (rLt ctx) supply ∧
      r.1.total_debt.val = lsum (rDebt .Ceil ctx) borrow ∧
      r.1.health_factor.val = (if lsum (rDebt .Ceil ctx) borrow = 0 then 2^127 - 1
        else min (2^127 - 1) (lsum (rLt ctx) supply * 10^18 / lsum (rDebt .Ceil ctx) borrow)) ⦄ := by
  unfold risk.totals.calculate_account_risk_totals risk.totals.calculate_account_risk_totals_body
  simp only [aeneas_soroban_shim.Map.keys, risk.totals.portfolio_hub_keys,
    aeneas_soroban_shim.Vec.append, bind_ok]
  rw [load_markets_cached ctx _ (by
    intro k hk
    rcases List.mem_append.mp hk with hk | hk
    · exact mem_keys_serves hs (fun e h => h.1) k hk
    · exact mem_keys_serves hb (fun e h => h.1) k hk)]
  simp only [bind_ok]
  step with iter_typed_positions_spec supply as ⟨L, hL⟩
  obtain ⟨e1, e2, e3, e4⟩ := typed_sums (ctx := ctx) hL
  have h0 : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  have hc0 := rCeil_sum_nonneg ctx supply hs
  apply WP.spec_bind (totals_loop_spec env ctx L math.fp.Wad.ZERO math.fp.Wad.ZERO math.fp.Wad.ZERO
    (forall2_ok hL hs) (by omega) (by omega) (by omega) (by rw [h0, e2]; omega)
    (by rw [h0, e2]; omega) (by rw [h0, e2]; omega))
  rintro ⟨c, tc, lc, wc⟩ ⟨hc, htc, hlc, hwc⟩
  simp only at hc htc hlc hwc
  subst c
  try simp only
  unfold risk.totals.sum_debt_usd_loaded
  step with iter_debt_positions_spec borrow as ⟨D, hD⟩
  subst hD
  have hd0 := rDebt_sum_nonneg ctx borrow hb
  apply WP.spec_bind (debt_loop_spec env ctx .Ceil _ math.fp.Wad.ZERO (raw_debt_ok hb) (by omega)
    (by rw [h0, debt_sums]; omega))
  rintro ⟨td, c'⟩ ⟨hc', htd⟩
  simp only at hc' htd
  subst c'
  try simp only
  rw [h0, debt_sums, zero_add] at htd
  rw [h0, e1, zero_add] at htc
  rw [h0, e3, zero_add] at hlc
  rw [h0, e4, zero_add] at hwc
  by_cases hz : lsum (rDebt .Ceil ctx) borrow = 0
  · have : td = math.fp.Wad.ZERO := by
      apply IScalar.eq_of_val_eq; rw [htd, h0, hz]
    subst this
    simp [math.fp.Wad.Insts.CoreCmpPartialEqWad.eq, hz, htc, hlc, hwc, htd, I128.rMax]
  · have : ¬ td = math.fp.Wad.ZERO := by
      intro h; apply hz; rw [← htd, h, h0]
    have hw0 : 0 ≤ wc.val := by
      rw [hwc]; unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      obtain ⟨-, ⟨hs', hi', hp', -, -⟩, -⟩ := hs y hy
      unfold rLt rFloor rLT
      apply Int.ediv_nonneg _ (by norm_num)
      apply mul_nonneg _ (by positivity)
      unfold fv
      apply Int.ediv_nonneg _ (by norm_num)
      apply mul_nonneg _ hp'
      exact Int.ediv_nonneg (Int.ediv_nonneg (mul_nonneg hs' hi') (by norm_num)) (by norm_num)
    simp [math.fp.Wad.Insts.CoreCmpPartialEqWad.eq, this]
    step with div_floor_saturating_val env wc td hw0 (by omega) as ⟨hf, hhf⟩
    refine ⟨htc, hlc, hwc, htd, ?_⟩
    rw [if_neg hz, hhf, hwc, htd]; norm_num

/-! ## Conservativeness (INV-RISK-02) -/

theorem lsum_le {α : Type} {f g : α → Int} {l : List α} (h : ∀ e ∈ l, f e ≤ g e) :
    lsum f l ≤ lsum g l := by
  induction l with
  | nil => simp
  | cons a l ih =>
    simp only [lsum_cons]
    have := h a (by simp)
    have := ih (fun e he => h e (by simp [he]))
    omega

theorem lsum_mul_right {α : Type} (f : α → Int) (c : Int) (l : List α) :
    lsum f l * c = lsum (fun e => f e * c) l := by
  induction l with
  | nil => simp
  | cons a l ih => simp only [lsum_cons]; rw [← ih]; ring

theorem served_vals (ctx : context.Context) (k : types.HubAssetKey) (h : Serves ctx k) :
    ∃ i p : Std.I128, supplyIndexOf ctx k = i.val ∧ priceOf ctx k.asset = p.val ∧
      ∃ b : Std.I128, borrowIndexOf ctx k = b.val := by
  obtain ⟨⟨pr, hp⟩, ⟨ix, hi⟩⟩ := h
  exact ⟨ix.supply_index, pr.price_wad, by simp [supplyIndexOf, hi], by simp [priceOf, hp],
    ix.borrow_index, by simp [borrowIndexOf, hi]⟩

/-- Per-leg facts for a supply entry: `0 ≤ fv ≤ huv ≤ cv`,
`fv·RBW ≤ s·i·p`, and the weighted floors are below their exact weights. -/
theorem supply_leg_bounds (ctx : context.Context) (e : types.HubAssetKey × types.AccountPositionRaw)
    (h : RawSupplyOK ctx e) :
    0 ≤ rFloor ctx e ∧ rFloor ctx e ≤ rHU ctx e ∧ rHU ctx e ≤ rCeil ctx e ∧
    rLt ctx e * (10^4 * (10^27 * 10^9 * 10^18)) ≤ rLT e * rExact ctx e ∧
    rLtv ctx e * (10^4 * (10^27 * 10^9 * 10^18)) ≤ rEffLTV e * rExact ctx e ∧
    rLtv ctx e ≤ rLt ctx e ∧ rLt ctx e ≤ rFloor ctx e := by
  obtain ⟨hserve, hvd, hlt, hltv⟩ := h
  obtain ⟨i, p, hi, hp, -⟩ := served_vals ctx e.1 hserve
  rw [hi, hp] at hvd
  obtain ⟨f0, fh, hc, -, fx, -⟩ := chains_bounds e.2.scaled_amount i p hvd
  have hF : rFloor ctx e = fv e.2.scaled_amount.val i.val p.val := by simp [rFloor, hi, hp]
  have hH : rHU ctx e = huv e.2.scaled_amount.val i.val p.val := by simp [rHU, hi, hp]
  have hC : rCeil ctx e = cv e.2.scaled_amount.val i.val p.val := by simp [rCeil, hi, hp]
  have hX : rExact ctx e = e.2.scaled_amount.val * i.val * p.val := by simp [rExact, hi, hp]
  have hlt0 : (0 : Int) ≤ rLT e := by unfold rLT; positivity
  have hltB : rLT e ≤ 10^4 := hlt
  have he0 : (0 : Int) ≤ rEffLTV e := by unfold rEffLTV; positivity
  have heLT : rEffLTV e ≤ rLT e := by unfold rEffLTV rLT; exact min_le_right _ _
  set F := rFloor ctx e with hFdef
  have hF0 : 0 ≤ F := by rw [hF]; exact f0
  have hFx : F * (10^27 * 10^9 * 10^18) ≤ rExact ctx e := by rw [hF, hX]; exact fx
  have k1 : rLt ctx e * 10^4 ≤ F * rLT e := Int.ediv_mul_le _ (by norm_num)
  have k2 : rLtv ctx e * 10^4 ≤ F * rEffLTV e := Int.ediv_mul_le _ (by norm_num)
  refine ⟨hF0, by rw [hF, hH]; exact fh, by rw [hH, hC]; exact hc, ?_, ?_, ?_, ?_⟩
  · have : rLt ctx e * (10^4 * (10^27 * 10^9 * 10^18)) ≤ F * rLT e * (10^27 * 10^9 * 10^18) := by
      nlinarith
    nlinarith
  · have : rLtv ctx e * (10^4 * (10^27 * 10^9 * 10^18)) ≤ F * rEffLTV e * (10^27 * 10^9 * 10^18) := by
      nlinarith
    nlinarith
  · unfold rLtv rLt
    exact Int.ediv_le_ediv (by norm_num) (mul_le_mul_of_nonneg_left heLT hF0)
  · unfold rLt
    apply Int.ediv_le_of_le_mul (by norm_num)
    nlinarith

/-- Per-leg fact for a debt entry: `s·i·p ≤ cv·RBW` and `0 ≤ huv ≤ cv`. -/
theorem debt_leg_bounds (ctx : context.Context) (e : types.HubAssetKey × types.DebtPositionRaw)
    (h : RawDebtOK ctx e) :
    rDebtExact ctx e ≤ rDebt .Ceil ctx e * (10^27 * 10^9 * 10^18) ∧
    0 ≤ rDebt .HalfUp ctx e ∧ rDebt .HalfUp ctx e ≤ rDebt .Ceil ctx e := by
  obtain ⟨hserve, hvd⟩ := h
  obtain ⟨-, p, -, hp, b, hb⟩ := served_vals ctx e.1 hserve
  rw [hb, hp] at hvd
  obtain ⟨f0, fh, hc, -, -, cx⟩ := chains_bounds e.2.scaled_amount b p hvd
  simp only [rDebtExact, rDebt, debtVal, hb, hp]
  exact ⟨cx, by omega, hc⟩

/-- **INV-RISK-02: the risk totals are conservative.** Under the hypotheses of
`calculate_account_risk_totals_spec`, the returned totals satisfy, with
`s·i·p` the exact value of a leg scaled by `R·B·W = 10^54`:

* `weighted_collateral · BPS · 10^54 ≤ Σ LT · s·i·p` and
  `ltv_collateral · BPS · 10^54 ≤ Σ min(LTV, LT) · s·i·p`: the collateral the
  risk gates use never exceeds its exact threshold- and LTV-weighted value;
* `ltv_collateral ≤ weighted_collateral ≤ total_collateral`;
* `Σ fv ≤ total_collateral ≤ Σ cv` (the half-up total lies between the floor
  and ceiling sums);
* `Σ s·i·p ≤ total_debt · 10^54`: debt never understates its exact value. -/
theorem calculate_account_risk_totals_conservative (env : Env) (ctx : context.Context)
    (supply : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (borrow : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw)
    (hs : ∀ e ∈ supply, RawSupplyOK ctx e) (hb : ∀ e ∈ borrow, RawDebtOK ctx e)
    (hsfit : lsum (rCeil ctx) supply < 2^127) (hbfit : lsum (rDebt .Ceil ctx) borrow < 2^127) :
    risk.totals.calculate_account_risk_totals env ctx supply borrow ⦃ r =>
      r.1.weighted_collateral.val * (10^4 * (10^27 * 10^9 * 10^18)) ≤
        lsum (fun e => rLT e * rExact ctx e) supply ∧
      r.1.ltv_collateral.val * (10^4 * (10^27 * 10^9 * 10^18)) ≤
        lsum (fun e => rEffLTV e * rExact ctx e) supply ∧
      r.1.ltv_collateral.val ≤ r.1.weighted_collateral.val ∧
      r.1.weighted_collateral.val ≤ r.1.total_collateral.val ∧
      lsum (rFloor ctx) supply ≤ r.1.total_collateral.val ∧
      r.1.total_collateral.val ≤ lsum (rCeil ctx) supply ∧
      lsum (rDebtExact ctx) borrow ≤ r.1.total_debt.val * (10^27 * 10^9 * 10^18) ⦄ := by
  apply WP.spec_mono (calculate_account_risk_totals_spec env ctx supply borrow hs hb hsfit hbfit)
  rintro r ⟨-, htc, hlc, hwc, htd, -⟩
  have B := fun e he => supply_leg_bounds ctx e (hs e he)
  have D := fun e he => debt_leg_bounds ctx e (hb e he)
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
  · rw [hwc, lsum_mul_right]; exact lsum_le (fun e he => (B e he).2.2.2.1)
  · rw [hlc, lsum_mul_right]; exact lsum_le (fun e he => (B e he).2.2.2.2.1)
  · rw [hlc, hwc]; exact lsum_le (fun e he => (B e he).2.2.2.2.2.1)
  · rw [hwc, htc]
    exact lsum_le (fun e he => le_trans (B e he).2.2.2.2.2.2 (B e he).2.1)
  · rw [htc]; exact lsum_le (fun e he => (B e he).2.1)
  · rw [htc]; exact lsum_le (fun e he => (B e he).2.2.1)
  · rw [htd, lsum_mul_right]; exact lsum_le (fun e he => (D e he).1)

/-- **`calculate_ltv_collateral_wad` is the exact LTV-weighted floor sum.**
For a context serving every supply entry (then `load_markets` changes
nothing), the call returns `Σ ⌊fv · min(LTV, LT) / BPS⌋`, the same value as
`calculate_account_risk_totals`' `ltv_collateral`, and leaves the context
unchanged. Same hypotheses as the supply half of
`calculate_account_risk_totals_spec`. -/
theorem calculate_ltv_collateral_wad_spec (env : Env) (ctx : context.Context)
    (supply : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (hs : ∀ e ∈ supply, RawSupplyOK ctx e) (hsfit : lsum (rCeil ctx) supply < 2^127) :
    risk.totals.calculate_ltv_collateral_wad env ctx supply ⦃ r =>
      r.2 = ctx ∧ r.1.val = lsum (rLtv ctx) supply ⦄ := by
  unfold risk.totals.calculate_ltv_collateral_wad
  simp only [aeneas_soroban_shim.Map.keys, bind_ok]
  rw [load_markets_cached ctx _ (mem_keys_serves hs (fun e h => h.1))]
  simp only [bind_ok]
  step with iter_typed_positions_spec supply as ⟨L, hL⟩
  obtain ⟨-, e2, e3, -⟩ := typed_sums (ctx := ctx) hL
  have h0 : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  have hc0 := rCeil_sum_nonneg ctx supply hs
  apply WP.spec_mono (ltv_loop_spec env ctx L math.fp.Wad.ZERO (forall2_ok hL hs) (by omega)
    (by rw [h0, e2]; omega))
  rintro ⟨a, c⟩ ⟨hc, ha⟩
  exact ⟨hc, by rw [ha, h0, e3, zero_add]⟩

/-- **`sum_debt_usd` is the exact half-up debt sum.** For a context serving
every debt entry, the call returns `Σ huv(s, borrow_index, price)` and leaves
the context unchanged; this is at most the ceiling total that
`calculate_account_risk_totals` uses for the risk gates. -/
theorem sum_debt_usd_spec (env : Env) (ctx : context.Context)
    (borrow : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw)
    (hb : ∀ e ∈ borrow, RawDebtOK ctx e) (hbfit : lsum (rDebt .Ceil ctx) borrow < 2^127) :
    risk.totals.sum_debt_usd env ctx borrow ⦃ r =>
      r.2 = ctx ∧ r.1.val = lsum (rDebt .HalfUp ctx) borrow ∧
      r.1.val ≤ lsum (rDebt .Ceil ctx) borrow ⦄ := by
  unfold risk.totals.sum_debt_usd
  simp only [aeneas_soroban_shim.Map.keys, bind_ok]
  rw [load_markets_cached ctx _ (mem_keys_serves hb (fun e h => h.1))]
  simp only [bind_ok]
  unfold risk.totals.sum_debt_usd_loaded
  step with iter_debt_positions_spec borrow as ⟨D, hD⟩
  subst hD
  have h0 : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  have hd0 := rDebt_sum_nonneg ctx borrow hb
  apply WP.spec_mono (debt_loop_spec env ctx .HalfUp _ math.fp.Wad.ZERO (raw_debt_ok hb) (by omega)
    (by rw [h0, debt_sums]; omega))
  rintro ⟨a, c⟩ ⟨hc, ha⟩
  rw [h0, debt_sums, zero_add] at ha
  exact ⟨hc, ha, by rw [ha]; exact lsum_le (fun e he => (debt_leg_bounds ctx e (hb e he)).2.2)⟩

end ControllerAeneas.Proofs
