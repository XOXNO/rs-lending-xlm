/-
`liquidation::math::{one_unit_per_leg_usd, one_unit_per_debt_leg_usd,
whole_unit_repayment}`: the value of one native token unit per leg, and the
whole-unit raise of a solvent partial quote whose only collateral leg is
below `MIN_BORROWABLE_ASSET_DECIMALS = 3`.

Notation: `W = 10^18`; for a feed with `dec ≤ 18` decimals and WAD price `p`,
`unitUsd dec p = hu(10^(18−dec)·p / W)` is the WAD USD value of one native
unit (`Wad::from_token(1, dec).mul(price)`).
-/
import ControllerAeneas.Proofs.RepayPlanExcess
import ControllerAeneas.Proofs.EstimateBasic

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

/-- WAD USD value of one native unit: `hu(10^(18−dec)·p / W)`. -/
def unitUsd (dec : Nat) (p : Int) : Int := huD (10^(18 - dec) * p) (10^18)

/-- One unit at a non-negative price has a non-negative value. -/
theorem unitUsd_nonneg (dec : Nat) (p : Int) (hp : 0 ≤ p) : 0 ≤ unitUsd dec p :=
  rp_huD_nonneg (by norm_num) (by positivity)

/-- One unit is worth at most the price: `unitUsd dec p ≤ p` for `p ≥ 0`. -/
theorem unitUsd_le (dec : Nat) (p : Int) (hp : 0 ≤ p) : unitUsd dec p ≤ p := by
  unfold unitUsd
  apply rp_huD_le_of (by norm_num)
  have : (10:Int)^(18 - dec) ≤ 10^18 := pow_le_pow_right₀ (by norm_num) (by omega)
  nlinarith

/-- `Wad::from_token(1, dec).mul(price)` is `unitUsd dec p`. -/
theorem unit_value_spec (env : Env) (dec : Std.U32) (p : math.fp.Wad) (hdec : dec.val ≤ 18)
    (hp : 0 ≤ p.val) :
    (do
      let w ← math.fp.Wad.from_token env 1#i128 dec
      math.fp.Wad.mul w env p) ⦃ u => u.val = unitUsd dec.val p.val ⦄ := by
  have hpow : (10:Int)^(18 - dec.val) ≤ 10^18 := pow_le_pow_right₀ (by norm_num) (by omega)
  have hpos : (0:Int) < 10^(18 - dec.val) := by positivity
  step with from_token_spec env 1#i128 dec hdec (by simp; constructor <;> nlinarith) as ⟨w, hw⟩
  have hw' : w.val = 10^(18 - dec.val) := by simpa using hw
  have hfit : huD (w.val * p.val) (10^18) < 2^127 := by
    rw [hw']
    have := unitUsd_le dec.val p.val hp
    have hpb := p.hBounds
    simp only [IScalarTy.numBits] at hpb
    unfold unitUsd at this; omega
  step with rp_mul_spec env w p (by omega) hp hfit as ⟨u, hu⟩
  rw [hu, hw']; rfl

/-- Fit and value of `hu(w·p/W)` for `w = 10^(18−dec)` (the second step of the
one-unit value). -/
theorem rp_unit_fit (dec : Nat) (w p : Int) (hw : w = 10^(18 - dec)) (hdec : dec ≤ 18)
    (hp : 0 ≤ p) (hpb : p < 2^127) :
    0 ≤ w ∧ huD (w * p) (10^18) < 2^127 ∧ huD (w * p) (10^18) = unitUsd dec p := by
  have := unitUsd_le dec p hp
  subst hw
  refine ⟨by positivity, ?_, rfl⟩
  unfold unitUsd at this; omega

/-! ## `one_unit_per_leg_usd` -/

/-- One-unit value of a repayment leg (its stored feed). -/
def legUnit (e : types.RepayEntry) : Int := unitUsd e.feed.asset_decimals.val e.feed.price_wad.val

/-- One iteration of the `one_unit_per_leg_usd` loop. -/
theorem one_unit_leg_loop_cons (env : Env) (e : types.RepayEntry) (l : List types.RepayEntry)
    (acc : math.fp.Wad) :
    positions.liquidation.math.one_unit_per_leg_usd_loop (e :: l) env acc = (do
      let w ← math.fp.Wad.from_token env 1#i128 e.feed.asset_decimals
      let unit ← math.fp.Wad.mul w env e.feed.price_wad
      let t ← math.fp.Wad.checked_add acc env unit
      positions.liquidation.math.one_unit_per_leg_usd_loop l env t) := by
  conv => lhs; unfold positions.liquidation.math.one_unit_per_leg_usd_loop; rw [loop]
  simp [positions.liquidation.math.one_unit_per_leg_usd_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next,
    positions.liquidation.math.one_unit_per_leg_usd_loop]

/-- The `one_unit_per_leg_usd` loop stops on an exhausted iterator. -/
theorem one_unit_leg_loop_nil (env : Env) (acc : math.fp.Wad) :
    positions.liquidation.math.one_unit_per_leg_usd_loop [] env acc = ok acc := by
  unfold positions.liquidation.math.one_unit_per_leg_usd_loop
  rw [loop]
  simp [positions.liquidation.math.one_unit_per_leg_usd_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The `one_unit_per_leg_usd` loop accumulates the exact sum of one-unit values. -/
theorem one_unit_leg_loop_spec (env : Env) :
    ∀ (l : List types.RepayEntry) (acc : math.fp.Wad),
      (∀ e ∈ l, e.feed.asset_decimals.val ≤ 18 ∧ 0 ≤ e.feed.price_wad.val) →
      0 ≤ acc.val → acc.val + lsum legUnit l < 2^127 →
      positions.liquidation.math.one_unit_per_leg_usd_loop l env acc
        ⦃ r => r.val = acc.val + lsum legUnit l ⦄ := by
  intro l
  induction l with
  | nil => intro acc _ _ _; simp [one_unit_leg_loop_nil]
  | cons e l ih =>
    intro acc hok h0 hfit
    obtain ⟨hdec, hp⟩ := hok e (by simp)
    have hu0 : 0 ≤ legUnit e := unitUsd_nonneg _ _ hp
    have hrest0 : 0 ≤ lsum legUnit l := by
      unfold lsum; apply List.sum_nonneg
      intro x hx; obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      exact unitUsd_nonneg _ _ (hok y (by simp [hy])).2
    rw [lsum_cons] at hfit
    rw [one_unit_leg_loop_cons]
    step with from_token_spec env 1#i128 e.feed.asset_decimals hdec (by
      have : (10:Int)^(18 - e.feed.asset_decimals.val) ≤ 10^18 := pow_le_pow_right₀ (by norm_num) (by omega)
      simp; constructor <;> nlinarith [pow_pos (show (0:Int) < 10 by norm_num) (18 - e.feed.asset_decimals.val)]) as ⟨w, hw⟩
    have hpb := e.feed.price_wad.hBounds
    simp only [IScalarTy.numBits] at hpb
    obtain ⟨hw0, hfitu, hequ⟩ := rp_unit_fit _ w.val _ (by simpa using hw) hdec hp (by omega)
    step with rp_mul_spec env w e.feed.price_wad hw0 hp hfitu as ⟨u, hu⟩
    rw [hequ] at hu
    have hu' : u.val = legUnit e := hu
    unfold math.fp.Wad.checked_add
    step with checked_add_raw_spec env acc u (by
      rw [hu']; constructor <;> linarith) as ⟨t, ht⟩
    have := ih t (fun x hx => hok x (by simp [hx])) (by rw [ht, hu']; linarith)
      (by rw [ht, hu']; linarith)
    apply WP.spec_mono this
    intro r hr
    rw [hr, ht, hu', lsum_cons]; ring

/-- **`one_unit_per_leg_usd` is the exact sum of one-unit values.** For legs
with at most 18 decimals and non-negative prices whose one-unit values sum
below `2^127`, it returns (does not abort) `Σ hu(10^(18−dec)·p/W)`. -/
theorem one_unit_per_leg_usd_spec (env : Env) (l : List types.RepayEntry)
    (hok : ∀ e ∈ l, e.feed.asset_decimals.val ≤ 18 ∧ 0 ≤ e.feed.price_wad.val)
    (hfit : lsum legUnit l < 2^127) :
    positions.liquidation.math.one_unit_per_leg_usd env l ⦃ r => r.val = lsum legUnit l ⦄ := by
  unfold positions.liquidation.math.one_unit_per_leg_usd
  simp only [aeneas_soroban_shim.Vec.iter, bind_ok]
  have := one_unit_leg_loop_spec env l math.fp.Wad.ZERO hok (by rw [rp_zero_val])
    (by rw [rp_zero_val]; simpa using hfit)
  apply WP.spec_mono this
  intro r hr; rw [hr, rp_zero_val, zero_add]

/-! ## `one_unit_per_debt_leg_usd` -/

/-- `cached_price` on a cached asset, in specification form. -/
theorem rp_cached_price_spec (ctx : context.Context) (a : aeneas_soroban_shim.Address)
    (raw : types.PriceFeedRaw) (h : mapLookup ctx.token_prices a = some raw) :
    context.Context.cached_price ctx a ⦃ r =>
      r = (types.PriceFeed.mk raw.price_wad raw.asset_decimals raw.timestamp, ctx) ⦄ := by
  rw [cached_price_ok ctx a raw h]; simp

/-- `cached_market_index` on a cached key, in specification form. -/
theorem rp_cached_market_index_spec (ctx : context.Context) (k : types.HubAssetKey)
    (raw : types.MarketIndexRaw) (h : mapLookup ctx.market_indexes k = some raw) :
    context.Context.cached_market_index ctx k ⦃ r =>
      r = (types.MarketIndex.mk raw.borrow_index raw.supply_index, ctx) ⦄ := by
  rw [cached_market_index_ok ctx k raw h]; simp

/-- One-unit value of a debt key: the cached feed of its asset. -/
noncomputable def keyUnit (ctx : context.Context) (k : types.HubAssetKey) : Int :=
  match mapLookup ctx.token_prices k.asset with
  | some r => unitUsd r.asset_decimals.val r.price_wad.val
  | none => 0

/-- The context has a price for the key's asset with at most 18 decimals and a
non-negative price. -/
def UnitKeyOK (ctx : context.Context) (k : types.HubAssetKey) : Prop :=
  ∃ r, mapLookup ctx.token_prices k.asset = some r ∧ r.asset_decimals.val ≤ 18 ∧
    0 ≤ r.price_wad.val

/-- A served debt key has a non-negative one-unit value. -/
theorem keyUnit_nonneg (ctx : context.Context) (k : types.HubAssetKey) (h : UnitKeyOK ctx k) :
    0 ≤ keyUnit ctx k := by
  obtain ⟨r, hr, -, hp⟩ := h
  simp only [keyUnit, hr]; exact unitUsd_nonneg _ _ hp

/-- The `one_unit_per_debt_leg_usd` loop accumulates the exact sum and leaves the context unchanged. -/
theorem one_unit_debt_loop_spec (env : Env) (ctx : context.Context) :
    ∀ (l : List types.HubAssetKey) (acc : math.fp.Wad),
      (∀ k ∈ l, UnitKeyOK ctx k) → 0 ≤ acc.val → acc.val + lsum (keyUnit ctx) l < 2^127 →
      positions.liquidation.math.one_unit_per_debt_leg_usd_loop l env ctx acc
        ⦃ r => r.1.val = acc.val + lsum (keyUnit ctx) l ∧ r.2 = ctx ⦄ := by
  intro l
  induction l with
  | nil =>
    intro acc _ _ _
    unfold positions.liquidation.math.one_unit_per_debt_leg_usd_loop; rw [loop]
    simp [positions.liquidation.math.one_unit_per_debt_leg_usd_loop.body,
      aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]
  | cons k l ih =>
    intro acc hok h0 hfit
    obtain ⟨r, hr, hdec, hp⟩ := hok k (by simp)
    have hu0 : 0 ≤ keyUnit ctx k := keyUnit_nonneg ctx k ⟨r, hr, hdec, hp⟩
    have hrest0 : 0 ≤ lsum (keyUnit ctx) l := by
      unfold lsum; apply List.sum_nonneg
      intro x hx; obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      exact keyUnit_nonneg ctx y (hok y (by simp [hy]))
    have hku : keyUnit ctx k = unitUsd r.asset_decimals.val r.price_wad.val := by
      simp [keyUnit, hr]
    rw [lsum_cons] at hfit
    unfold positions.liquidation.math.one_unit_per_debt_leg_usd_loop
    rw [loop]
    dsimp only
    rw [positions.liquidation.math.one_unit_per_debt_leg_usd_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    step with rp_cached_price_spec ctx k.asset r hr as ⟨fc, hfc⟩
    subst hfc
    step with from_token_spec env 1#i128 r.asset_decimals hdec (by
      have : (10:Int)^(18 - r.asset_decimals.val) ≤ 10^18 := pow_le_pow_right₀ (by norm_num) (by omega)
      simp; constructor <;> nlinarith [pow_pos (show (0:Int) < 10 by norm_num) (18 - r.asset_decimals.val)]) as ⟨w, hw⟩
    have hpb := r.price_wad.hBounds
    simp only [IScalarTy.numBits] at hpb
    obtain ⟨hw0, hfitu, hequ⟩ := rp_unit_fit _ w.val _ (by simpa using hw) hdec hp (by omega)
    step with rp_mul_spec env w r.price_wad hw0 hp hfitu as ⟨u, hu⟩
    rw [hequ] at hu
    unfold math.fp.Wad.checked_add
    step with checked_add_raw_spec env acc u (by rw [hu, ← hku]; constructor <;> linarith) as ⟨t, ht⟩
    have := ih t (fun x hx => hok x (by simp [hx])) (by rw [ht, hu, ← hku]; linarith)
      (by rw [ht, hu, ← hku]; linarith)
    unfold positions.liquidation.math.one_unit_per_debt_leg_usd_loop at this
    dsimp only at this
    apply WP.spec_mono this
    intro res ⟨h1, h2⟩
    refine ⟨?_, h2⟩
    rw [h1, ht, hu, lsum_cons, hku]; ring

/-- **`one_unit_per_debt_leg_usd` is the exact sum over the debt keys.** If
the context has a price (≤ 18 decimals, non-negative) for every debt key's
asset and the one-unit values sum below `2^127`, it returns (does not abort)
`Σ_k unitUsd(dec_k, p_k)` over the account's borrow keys and leaves the
context unchanged. -/
theorem one_unit_per_debt_leg_usd_spec (env : Env) (acc : types.Account) (ctx : context.Context)
    (hok : ∀ k ∈ (acc.borrow_positions : List _).map Prod.fst, UnitKeyOK ctx k)
    (hfit : lsum (keyUnit ctx) ((acc.borrow_positions : List _).map Prod.fst) < 2^127) :
    positions.liquidation.math.one_unit_per_debt_leg_usd env acc ctx ⦃ r =>
      r.1.val = lsum (keyUnit ctx) ((acc.borrow_positions : List _).map Prod.fst) ∧ r.2 = ctx ⦄ := by
  unfold positions.liquidation.math.one_unit_per_debt_leg_usd
  simp only [aeneas_soroban_shim.Map.keys, bind_ok,
    aeneas_soroban_shim.Vec.Insts.CoreIterTraitsCollectIntoIteratorTVecIter.into_iter]
  have := one_unit_debt_loop_spec env ctx _ math.fp.Wad.ZERO hok (by rw [rp_zero_val])
    (by rw [rp_zero_val]; simpa using hfit)
  apply WP.spec_mono this
  intro r ⟨hr, hc⟩; exact ⟨by rw [hr, rp_zero_val, zero_add], hc⟩

/-! ## `whole_unit_repayment` -/

/-- `Ray::mul` is `hu(a·b / RAY)` for non-negative operands whose result fits. -/
theorem rp_ray_mul_spec (env : Env) (a b : math.fp.Ray) (ha : 0 ≤ a.val) (hb : 0 ≤ b.val)
    (hfit : huD (a.val * b.val) (10^27) < 2^127) :
    math.fp.Ray.mul a env b ⦃ r => r.val = huD (a.val * b.val) (10^27) ⦄ := by
  unfold math.fp.Ray.mul
  obtain ⟨q, hq⟩ := IsHalfUp.exists (a.val * b.val) (10^27) (by norm_num)
  have hqe : q = huD (a.val * b.val) (10^27) := isHalfUp_eq (by norm_num) hq
  step with mul_div_half_up_spec env a b constants.production.shared.RAY ha hb
    (by rw [RAY_val]; norm_num) ⟨q, by rw [RAY_val]; exact hq, by rw [hqe]; exact hfit⟩ as ⟨r, hr⟩
  rw [RAY_val] at hr; exact isHalfUp_eq (by norm_num) hr

/-- Domain of `whole_unit_repayment` (production): the quote is in `[0, D)`
(`normalize_repayment_plan` calls it only on a solvent account with
`quote < D`; `D ≤ 10^36` as in `EstDomain`), the bonus is in `[0, 10^8]`
BPS, the account has fewer than `2^32` supply legs, each served by the
context with at most 18 decimals, a price at most `10^36` and a scaled
balance whose RAY value fits, every debt key's asset has a cached price, and
`D` plus the debt legs' one-unit values fits `i128`. -/
structure WholeUnitDom (ctx : context.Context) (acc : types.Account)
    (snap : positions.liquidation.curve.LiquidationSnapshot) (q : math.fp.Wad) (b : math.fp.Bps) :
    Prop where
  q0 : 0 ≤ q.val
  qD : q.val < snap.total_debt.val
  Dmax : snap.total_debt.val ≤ 10^36
  b0 : 0 ≤ b.val
  bmax : b.val ≤ 10^8
  slen : (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).length < 2^32
  supply : ∀ e ∈ (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)),
    ∃ pr ix, mapLookup ctx.token_prices e.1.asset = some pr ∧
      mapLookup ctx.market_indexes e.1 = some ix ∧
      pr.asset_decimals.val ≤ 18 ∧ 0 ≤ pr.price_wad.val ∧ pr.price_wad.val ≤ 10^36 ∧
      0 ≤ e.2.scaled_amount.val ∧ 0 ≤ ix.supply_index.val ∧
      e.2.scaled_amount.val * ix.supply_index.val ≤ 2^126 * 10^27
  borrow : ∀ k ∈ (acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst,
    UnitKeyOK ctx k
  qfit : q.val * (10^18 + b.val * 10^14) ≤ 2^126 * 10^18
  ufit : snap.total_debt.val +
    lsum (keyUnit ctx) ((acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst)
      < 2^127

/-- What a raised whole-unit repayment `r ≠ quote` is. `u` is one unit of
the collateral, `m = u + max(⌊u/10^6⌋, 1)` one unit plus the margin,
`opb = W + bonus·10^14` (`1 + bonus`), `U` the one-unit values of the debt
legs. -/
def WholeUnitRaised (q D U u opb r : Int) : Prop :=
  huD (q * opb) (10^18) < u + max (u / 10^6) 1 ∧
  ((r = D ∧ (D + U) * opb ≤ u * 10^18) ∨
   (IsCeil ((u + max (u / 10^6) 1) * 10^18) opb r ∧ r < D ∧ u * 10^18 < (D + U) * opb))

/-- A raised whole-unit repayment is at least the quote (`⌈m·W/(1+b)⌉ > quote` because `hu(quote·(1+b)) < m`). -/
theorem raised_ge_quote {q D U u opb r : Int} (hopb : 0 < opb) (hq : 0 ≤ q)
    (hqD : q < D) (h : WholeUnitRaised q D U u opb r) : q ≤ r := by
  obtain ⟨hlt, h | ⟨hc, hrD, -⟩⟩ := h
  · rw [h.1]; exact hqD.le
  · obtain ⟨hq', hhq⟩ := IsHalfUp.exists (q * opb) (10^18) (by norm_num)
    have he := isHalfUp_eq (by norm_num) hhq
    have h2 := IsHalfUp.lt_mul hhq
    rw [he] at h2
    have hc' := (isCeil_pos hopb).mp hc
    -- q·opb < m·W ≤ r·opb
    have h3 : q * opb < (u + max (u / 10^6) 1) * 10^18 := by
      have : huD (q * opb) (10^18) + 1 ≤ u + max (u / 10^6) 1 := hlt
      nlinarith
    have h4 : q * opb < r * opb := by linarith [hc'.1]
    by_contra hc2
    have : r * opb ≤ q * opb := mul_le_mul_of_nonneg_right (by omega) hopb.le
    linarith

/-- **`whole_unit_repayment`: exact outcome.** On its domain
(`WholeUnitDom`) the call returns (never aborts), leaves the context
unchanged, and its value `r` satisfies `quote ≤ r ≤ D`. If `r ≠ quote`, the
account has exactly one supply leg `k`, with fewer than 3 decimals
(`MIN_BORROWABLE_ASSET_DECIMALS`), the quote seizes less than one unit plus
the margin (`hu(quote·(1+b)) < m`), and `WholeUnitRaised` holds:

* either `r = D` (full close) and one unit at the bonus, rounded down
  (`Wad::div_floor`), covers the whole debt plus one native unit per debt
  leg: `(D + U)·(1+b) ≤ u·W`;
* or `r = ⌈m·W/(1+b)⌉` (`Wad::div_ceil`, so `r·(1+b) ≥ m·W`: the raised
  repayment backs at least one unit plus the margin at the bonus), `r < D`,
  and the full-close test failed. -/
theorem whole_unit_repayment_spec (env : Env) (acc : types.Account)
    (snap : positions.liquidation.curve.LiquidationSnapshot) (q : math.fp.Wad) (b : math.fp.Bps)
    (ctx : context.Context) (hd : WholeUnitDom ctx acc snap q b) :
    positions.liquidation.math.whole_unit_repayment env acc snap q b ctx ⦃ r =>
      r.2 = ctx ∧ q.val ≤ r.1.val ∧ r.1.val ≤ snap.total_debt.val ∧
      (r.1.val ≠ q.val → ∃ k raw pr,
        (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)) = [(k, raw)] ∧
        mapLookup ctx.token_prices k.asset = some pr ∧ pr.asset_decimals.val < 3 ∧
        WholeUnitRaised q.val snap.total_debt.val
          (lsum (keyUnit ctx) ((acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst))
          (unitUsd pr.asset_decimals.val pr.price_wad.val) (10^18 + b.val * 10^14) r.1.val) ⦄ := by
  obtain ⟨hq0, hqD, hDmax, hb0, hbmax, hslen, hsup, hbor, hqfit, hufit⟩ := hd
  have hquote : (q, ctx).2 = ctx ∧ q.val ≤ (q, ctx).1.val ∧ (q, ctx).1.val ≤ snap.total_debt.val ∧
      ((q, ctx).1.val ≠ q.val → False) := ⟨rfl, le_refl _, hqD.le, fun h => h rfl⟩
  unfold positions.liquidation.math.whole_unit_repayment
  simp only [aeneas_soroban_shim.Map.len, UScalar.tryMk]
  have hk := UScalar.tryMkOpt_eq .U32
    (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).length
  cases hko : UScalar.tryMkOpt .U32
    (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).length with
  | none =>
    exfalso; rw [hko] at hk; apply hk
    simp [UScalar.inBounds, UScalarTy.numBits]; omega
  | some n =>
  rw [hko] at hk; obtain ⟨hn, -⟩ := hk
  simp only [Result.ofOption, bind_ok]
  by_cases hl1 : (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).length = 1
  swap
  · have hne : (n != 1#u32) = true := by
      simp only [bne_iff_ne, ne_eq]; intro h; apply hl1; rw [← hn, h]; rfl
    simp only [hne, ↓reduceIte]
    apply (WP.spec_ok _).mpr
    exact ⟨rfl, le_refl _, hqD.le, fun h => absurd rfl h⟩
  have hne : (n != 1#u32) = false := by
    simp only [bne_eq_false_iff_eq]; apply UScalar.eq_of_val_eq; rw [hn, hl1]; rfl
  simp only [hne, Bool.false_eq_true, ↓reduceIte]
  obtain ⟨⟨k, raw⟩, he⟩ := List.length_eq_one_iff.mp hl1
  obtain ⟨pr, ix, hpr, hix, hdec, hp0, hpmax, hs0, hi0, hsi⟩ := hsup (k, raw) (by rw [he]; simp)
  simp only at hpr hix hs0 hsi
  step with iter_typed_positions_spec acc.supply_positions as ⟨vi, hvi⟩
  rw [he] at hvi
  obtain ⟨⟨k', pos⟩, hvi'⟩ : ∃ t, vi = [t] ∧ TypedOf (k, raw) t := by
    cases hvi with
    | cons h1 h2 => cases h2; exact ⟨_, rfl, h1⟩
  obtain ⟨rfl, ht⟩ := hvi'
  obtain ⟨hkk, hsc, -⟩ := ht
  simp only at hkk hsc
  subst hkk
  simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok,
    Std.uncurry_apply_pair]
  rw [cached_price_ok ctx k'.asset pr hpr]
  simp only [bind_ok, Std.uncurry_apply_pair]
  by_cases hd3 : 3 ≤ pr.asset_decimals.val
  · have : pr.asset_decimals >= constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS := by
      unfold constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS
      simp [UScalar.le_equiv]; omega
    simp only [this, ↓reduceIte]
    apply (WP.spec_ok _).mpr
    exact ⟨rfl, le_refl _, hqD.le, fun h => absurd rfl h⟩
  have hd3' : ¬ (pr.asset_decimals >= constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS) := by
    unfold constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS
    simp [UScalar.le_equiv]; omega
  simp only [hd3', ↓reduceIte]
  rw [cached_market_index_ok ctx k' ix hix]
  simp only [bind_ok, Std.uncurry_apply_pair]
  rw [hsc]
  have hRfit : huD (raw.scaled_amount.val * ix.supply_index.val) (10^27) < 2^127 := by
    have := rp_huD_le_of (n := raw.scaled_amount.val * ix.supply_index.val) (d := 10^27)
      (c := 2^126) (by norm_num) (by linarith)
    linarith
  step with rp_ray_mul_spec env raw.scaled_amount ix.supply_index hs0 hi0 hRfit as ⟨rr, hrr⟩
  step with ray_to_asset_floor_spec env rr pr.asset_decimals (by omega) as ⟨hu, hhu⟩
  have hquote' : ∀ c, c = ctx → (q, c).2 = ctx ∧ q.val ≤ (q, c).1.val ∧
      (q, c).1.val ≤ snap.total_debt.val ∧ ((q, c).1.val ≠ q.val → False) := by
    intro c hc; subst hc; exact ⟨rfl, le_refl _, hqD.le, fun h => h rfl⟩
  by_cases hh1 : hu.val < 1
  · have : hu < 1#i128 := by simp [IScalar.lt_equiv]; omega
    simp only [this, ↓reduceIte]
    apply (WP.spec_ok _).mpr
    exact ⟨rfl, le_refl _, hqD.le, fun h => absurd rfl h⟩
  have : ¬ (hu < 1#i128) := by simp [IScalar.lt_equiv]; omega
  simp only [this, ↓reduceIte]
  step with bps_to_wad_spec env b hb0 hbmax as ⟨w, hw⟩
  unfold math.fp.Wad.checked_add
  step with checked_add_raw_spec env math.fp.Wad.ONE w
    (by rw [rp_one_val, hw]; constructor <;> nlinarith) as ⟨opb, hopb⟩
  rw [rp_one_val, hw] at hopb
  have hopbW : (10:Int)^18 ≤ opb.val := by rw [hopb]; nlinarith
  have hopb0 : 0 < opb.val := by linarith
  step with from_token_spec env 1#i128 pr.asset_decimals (by omega) (by
      have : (10:Int)^(18 - pr.asset_decimals.val) ≤ 10^18 := pow_le_pow_right₀ (by norm_num) (by omega)
      simp; constructor <;> nlinarith [pow_pos (show (0:Int) < 10 by norm_num) (18 - pr.asset_decimals.val)]) as ⟨w1, hw1⟩
  have hpb := pr.price_wad.hBounds
  simp only [IScalarTy.numBits] at hpb
  obtain ⟨hw10, hfitu, hequ⟩ := rp_unit_fit _ w1.val _ (by simpa using hw1) (by omega) hp0 (by omega)
  step with rp_mul_spec env w1 pr.price_wad hw10 hp0 hfitu as ⟨uu, huu⟩
  rw [hequ] at huu
  have hu0 : 0 ≤ uu.val := by rw [huu]; exact unitUsd_nonneg _ _ hp0
  have hule : uu.val ≤ 10^36 := by rw [huu]; exact le_trans (unitUsd_le _ _ hp0) hpmax
  simp only [math.fp.Wad.raw, bind_ok]
  step as ⟨i2, hi2⟩
  rw [Int.tdiv_eq_ediv_of_nonneg hu0] at hi2
  simp only [lift, bind_ok, wad_from_same]
  have hmx : (core.cmp.impls.OrdI128.max i2 1#i128).val = max (uu.val / 10^6) 1 := by
    rw [core.cmp.impls.OrdI128.max_val, hi2]; simp
  have hdiv0 : 0 ≤ uu.val / 10^6 := Int.ediv_nonneg hu0 (by norm_num)
  have hdivle : uu.val / 10^6 ≤ uu.val := Int.ediv_le_self _ hu0
  step with checked_add_raw_spec env uu (core.cmp.impls.OrdI128.max i2 1#i128)
    (by rw [hmx]; constructor <;> [skip; skip] <;>
        (have := le_max_left (uu.val / 10^6) 1; have := max_le (show uu.val / 10^6 ≤ uu.val + 1 by omega) (show (1:Int) ≤ uu.val + 1 by omega); omega)) as ⟨m, hm⟩
  rw [hmx] at hm
  have hm0 : 0 < m.val := by have := le_max_right (uu.val / 10^6) 1; omega
  have hmle : m.val ≤ 2 * 10^36 + 1 := by
    have := max_le (show uu.val / 10^6 ≤ uu.val + 1 by omega) (show (1:Int) ≤ uu.val + 1 by omega)
    omega
  have hqbfit : huD (q.val * opb.val) (10^18) < 2^127 := by
    have := rp_huD_le_of (n := q.val * opb.val) (d := 10^18) (c := 2^126) (by norm_num)
      (by rw [hopb]; linarith)
    linarith
  step with rp_mul_spec env q opb hq0 hopb0.le hqbfit as ⟨qb, hqb⟩
  try simp only [wad_ge, bind_ok]
  by_cases hcov : m.val ≤ qb.val
  · rw [if_pos (by simpa using hcov)]
    apply (WP.spec_ok _).mpr
    exact ⟨rfl, le_refl _, hqD.le, fun h => absurd rfl h⟩
  rw [if_neg (by simpa using hcov)]
  step with rp_div_floor_spec env uu opb hu0 hopb0 (by nlinarith) as ⟨uab, huab⟩
  have huab' := (isFloor_pos hopb0).mp huab
  have hU0 : 0 ≤ lsum (keyUnit ctx)
      ((acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst) := by
    unfold lsum; apply List.sum_nonneg
    intro x hx; obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
    exact keyUnit_nonneg ctx y (hbor y hy)
  have hD0 : 0 ≤ snap.total_debt.val := by linarith
  apply WP.spec_bind (one_unit_per_debt_leg_usd_spec env acc ctx hbor (by linarith))
  rintro ⟨w4, c4⟩ ⟨hw4, hc4⟩
  simp only at hw4 hc4
  subst c4
  try simp only [Std.uncurry_apply_pair]
  step with checked_add_raw_spec env snap.total_debt w4 (by rw [hw4]; constructor <;> linarith)
    as ⟨fcc, hfcc⟩
  rw [hw4] at hfcc
  try simp only [wad_ge, bind_ok]
  have hraisedpre : huD (q.val * opb.val) (10^18) < uu.val + max (uu.val / 10^6) 1 := by
    rw [← hqb, ← hm]; omega
  by_cases hfull : fcc.val ≤ uab.val
  · rw [if_pos (by simpa using hfull)]
    apply (WP.spec_ok _).mpr
    refine ⟨rfl, hqD.le, le_refl _, fun _ => ⟨k', raw, pr, he, hpr, by omega, ?_⟩⟩
    rw [← huu, ← hopb]
    refine ⟨hraisedpre, Or.inl ⟨rfl, ?_⟩⟩
    have : fcc.val * opb.val ≤ uab.val * opb.val := mul_le_mul_of_nonneg_right hfull hopb0.le
    rw [hfcc] at this; linarith [huab'.1]
  rw [if_neg (by simpa using hfull)]
  step with rp_div_ceil_spec env m opb hm0.le hopb0 (by nlinarith) as ⟨ur, hur⟩
  try simp only [wad_ge, bind_ok]
  by_cases hDur : snap.total_debt.val ≤ ur.val
  · rw [if_pos (by simpa using hDur)]
    apply (WP.spec_ok _).mpr
    exact ⟨rfl, le_refl _, hqD.le, fun h => absurd rfl h⟩
  rw [if_neg (by simpa using hDur)]
  apply (WP.spec_ok _).mpr
  have hraised : WholeUnitRaised q.val snap.total_debt.val
      (lsum (keyUnit ctx) ((acc.borrow_positions : List (types.HubAssetKey × types.DebtPositionRaw)).map Prod.fst))
      (unitUsd pr.asset_decimals.val pr.price_wad.val) (10^18 + b.val * 10^14) ur.val := by
    rw [← huu, ← hopb]
    refine ⟨hraisedpre, Or.inr ⟨by rw [← hm]; exact hur, by omega, ?_⟩⟩
    have h1 : uab.val + 1 ≤ fcc.val := by omega
    have h2 : (uab.val + 1) * opb.val ≤ fcc.val * opb.val := mul_le_mul_of_nonneg_right h1 hopb0.le
    rw [hfcc] at h2; linarith [huab'.2]
  refine ⟨rfl, raised_ge_quote (by positivity) hq0 hqD hraised, le_of_lt (not_le.mp hDur),
    fun _ => ⟨k', raw, pr, he, hpr, by omega, hraised⟩⟩

end ControllerAeneas.Proofs
