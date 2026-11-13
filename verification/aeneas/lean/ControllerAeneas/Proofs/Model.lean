/-
Lemmas about the Soroban `Map` model and the controller `Context` model
(`FunsExternal.lean`, "Map" and "Context" sections), used by the risk-totals
and liquidation proofs:

* `mapLookup`: the value `Map::get` returns, as a plain function;
* `cached_price_ok` / `cached_market_index_ok`: a cached entry is returned
  converted, and the context is unchanged;
* `load_markets_cached`: `load_markets` changes nothing when every key's price
  and index are already cached;
* `iter_typed_positions_spec` / `iter_debt_positions_spec`: the stand-in
  iterators yield the map's entries in order, converted field by field.
-/
import ControllerAeneas.Proofs.Basic

open Aeneas Aeneas.Std Result
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

open Classical in
/-- The value the `Map` model returns for `k`: the value of the first entry
with key `k`, if any. -/
noncomputable def mapLookup {K V : Type} (m : List (K × V)) (k : K) : Option V :=
  (m.find? (fun e => decide (e.1 = k))).map Prod.snd

/-- `Map::get` is `mapLookup`. -/
theorem map_get_eq {K V : Type} (m : aeneas_soroban_shim.Map K V) (k : K) :
    aeneas_soroban_shim.Map.get m k = ok (mapLookup m k) := rfl

open Classical in
/-- A key with a value is present in the entry list. -/
theorem mapLookup_some_any {K V : Type} (m : List (K × V)) (k : K) (v : V)
    (h : mapLookup m k = some v) : m.any (fun e => decide (e.1 = k)) = true := by
  unfold mapLookup at h
  cases hf : m.find? (fun e => decide (e.1 = k)) with
  | none => rw [hf] at h; simp at h
  | some e =>
    have := List.find?_some hf
    rw [List.any_eq_true]
    exact ⟨e, List.mem_of_find?_eq_some hf, this⟩

/-- `i128::from(u32)` keeps the value. -/
theorem i128_from_u32_val (x : Std.U32) :
    I128.Insts.CoreConvertFromU32.from x ⦃ y => y.val = x.val ⦄ := by
  have hx : x.val ≤ 4294967295 := by scalar_tac
  simp only [I128.Insts.CoreConvertFromU32.from, IScalar.tryMk]
  generalize hz : (x.val : Int) = z
  have h := IScalar.tryMkOpt_eq IScalarTy.I128 z
  cases hm : IScalar.tryMkOpt IScalarTy.I128 z with
  | some y => rw [hm] at h; simp [Result.ofOption, h.1]
  | none =>
    rw [hm] at h; exfalso; apply h
    simp only [IScalar.inBounds, IScalarTy.numBits]; omega

/-- `Context::cached_price` on a cached asset returns the cached feed
(`price = price_wad`, same decimals) and leaves the context unchanged. -/
theorem cached_price_ok (ctx : context.Context) (a : aeneas_soroban_shim.Address)
    (raw : types.PriceFeedRaw) (h : mapLookup ctx.token_prices a = some raw) :
    context.Context.cached_price ctx a =
      ok ({ price := raw.price_wad, asset_decimals := raw.asset_decimals,
            timestamp := raw.timestamp }, ctx) := by
  unfold context.Context.cached_price
  simp [aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone, map_get_eq, h,
    core.option.Option.unwrap_or_else, core.convert.IntoFrom.into,
    types.PriceFeed.Insts.CoreConvertFromShared0PriceFeedRaw,
    types.PriceFeed.Insts.CoreConvertFromShared0PriceFeedRaw.from]

/-- `Context::cached_market_index` on a cached key returns the cached index
and leaves the context unchanged. -/
theorem cached_market_index_ok (ctx : context.Context) (k : types.HubAssetKey)
    (raw : types.MarketIndexRaw) (h : mapLookup ctx.market_indexes k = some raw) :
    context.Context.cached_market_index ctx k =
      ok ({ borrow_index := raw.borrow_index, supply_index := raw.supply_index }, ctx) := by
  unfold context.Context.cached_market_index
  simp [types.HubAssetKey.Insts.CoreCloneClone.clone, lift,
    aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone, map_get_eq, h,
    core.convert.IntoFrom.into, core.clone.impls.CloneU32.clone,
    types.MarketIndex.Insts.CoreConvertFromShared0MarketIndexRaw,
    types.MarketIndex.Insts.CoreConvertFromShared0MarketIndexRaw.from]

/-- A context serves a key when the price of its asset and its index are cached. -/
def Serves (ctx : context.Context) (k : types.HubAssetKey) : Prop :=
  (∃ p, mapLookup ctx.token_prices k.asset = some p) ∧
  (∃ i, mapLookup ctx.market_indexes k = some i)

open Classical in
/-- `load_markets` leaves a context that already serves every key unchanged. -/
theorem load_markets_cached (ctx : context.Context) (keys : List types.HubAssetKey)
    (h : ∀ k ∈ keys, Serves ctx k) :
    context.Context.load_markets ctx keys = ok ctx := by
  unfold context.Context.load_markets
  induction keys with
  | nil => rfl
  | cons k ks ih =>
    obtain ⟨⟨p, hp⟩, ⟨i, hi⟩⟩ := h k (by simp)
    simp only [List.foldlM_cons]
    have hp' := mapLookup_some_any _ _ _ hp
    have hi' := mapLookup_some_any _ _ _ hi
    simp only [List.any_eq_true, decide_eq_true_eq] at hp' hi'
    have h1 : loadOne ctx k = ok ctx := by
      unfold loadOne
      obtain ⟨e1, he1, he1k⟩ := hp'
      obtain ⟨e2, he2, he2k⟩ := hi'
      have c1 : (ctx.token_prices : List _).any (fun e => decide (e.1 = k.asset)) = true :=
        List.any_eq_true.mpr ⟨e1, he1, by simp [he1k]⟩
      have c2 : (ctx.market_indexes : List _).any (fun e => decide (e.1 = k)) = true :=
        List.any_eq_true.mpr ⟨e2, he2, by simp [he2k]⟩
      simp only [c1, c2, ↓reduceIte, bind_ok]
    rw [h1, bind_tc_ok]
    exact ih (fun k' hk' => h k' (by simp [hk']))

/-- Entry-wise conversion `AccountPositionRaw → AccountPosition` (production
`From` impl): same key, same scaled amount, the four `u32` parameters as the
same numbers. -/
def TypedOf (e : types.HubAssetKey × types.AccountPositionRaw)
    (t : types.HubAssetKey × types.AccountPosition) : Prop :=
  t.1 = e.1 ∧ t.2.scaled_amount = e.2.scaled_amount ∧
  t.2.liquidation_threshold.val = e.2.liquidation_threshold.val ∧
  t.2.liquidation_bonus.val = e.2.liquidation_bonus.val ∧
  t.2.loan_to_value.val = e.2.loan_to_value.val ∧
  t.2.liquidation_fees.val = e.2.liquidation_fees.val

/-- `AccountPosition::from(&raw)` converts field by field. -/
theorem account_position_from_spec (r : types.AccountPositionRaw) :
    types.AccountPosition.Insts.CoreConvertFromShared0AccountPositionRaw.from r ⦃ p =>
      p.scaled_amount = r.scaled_amount ∧
      p.liquidation_threshold.val = r.liquidation_threshold.val ∧
      p.liquidation_bonus.val = r.liquidation_bonus.val ∧
      p.loan_to_value.val = r.loan_to_value.val ∧
      p.liquidation_fees.val = r.liquidation_fees.val ⦄ := by
  unfold types.AccountPosition.Insts.CoreConvertFromShared0AccountPositionRaw.from
  simp only [ray_from_same, bps_from_same, bind_ok]
  step with i128_from_u32_val as ⟨a, ha⟩
  step with i128_from_u32_val as ⟨b, hb⟩
  step with i128_from_u32_val as ⟨c, hc⟩
  step with i128_from_u32_val as ⟨d, hd⟩
  simp [ha, hb, hc, hd]

theorem iter_typed_positions_loop_cons (k : types.HubAssetKey) (r : types.AccountPositionRaw)
    (m : List (types.HubAssetKey × types.AccountPositionRaw))
    (out : List (types.HubAssetKey × types.AccountPosition)) :
    storage.iter_typed_positions_loop ((k, r) :: m) out = (do
      let p ← types.AccountPosition.Insts.CoreConvertFromShared0AccountPositionRaw.from r
      storage.iter_typed_positions_loop m (out ++ [(k, p)])) := by
  conv => lhs; unfold storage.iter_typed_positions_loop; rw [loop]
  simp [storage.iter_typed_positions_loop.body, aeneas_soroban_shim.Vec.push_back,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next,
    storage.iter_typed_positions_loop]

theorem iter_typed_positions_loop_spec :
    ∀ (m : List (types.HubAssetKey × types.AccountPositionRaw))
      (out : List (types.HubAssetKey × types.AccountPosition)),
      storage.iter_typed_positions_loop m out ⦃ r =>
        ∃ l, r = out ++ l ∧ List.Forall₂ TypedOf m l ⦄ := by
  intro m
  induction m with
  | nil =>
    intro out
    unfold storage.iter_typed_positions_loop; rw [loop]
    simp [storage.iter_typed_positions_loop.body,
      aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]
  | cons e m ih =>
    intro out
    obtain ⟨k, r⟩ := e
    rw [iter_typed_positions_loop_cons]
    step with account_position_from_spec r as ⟨p, hp1, hp2, hp3, hp4, hp5⟩
    step with ih (out ++ [(k, p)]) as ⟨res, l, hl, hf⟩
    refine ⟨(k, p) :: l, by simp [hl], ?_⟩
    exact List.Forall₂.cons (R := TypedOf) ⟨rfl, hp1, hp2, hp3, hp4, hp5⟩ hf

/-- **`iter_typed_positions` yields every entry, in order, converted.** -/
theorem iter_typed_positions_spec (m : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw) :
    storage.iter_typed_positions m ⦃ l => List.Forall₂ TypedOf m l ⦄ := by
  unfold storage.iter_typed_positions
  simp only [aeneas_soroban_shim.Map.iter, aeneas_soroban_shim.Vec.new, bind_ok,
    aeneas_soroban_shim.Vec.Insts.CoreIterTraitsCollectIntoIteratorTVecIter.into_iter]
  step with iter_typed_positions_loop_spec m [] as ⟨r, l, hl, hf⟩
  simpa [hl] using hf

theorem iter_debt_positions_loop_cons (k : types.HubAssetKey) (r : types.DebtPositionRaw)
    (m : List (types.HubAssetKey × types.DebtPositionRaw))
    (out : List (types.HubAssetKey × types.DebtPosition)) :
    storage.iter_debt_positions_loop ((k, r) :: m) out =
      storage.iter_debt_positions_loop m (out ++ [(k, { scaled_amount := r.scaled_amount })]) := by
  conv => lhs; unfold storage.iter_debt_positions_loop; rw [loop]
  simp [storage.iter_debt_positions_loop.body, aeneas_soroban_shim.Vec.push_back,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next,
    types.DebtPosition.Insts.CoreConvertFromShared0DebtPositionRaw.from,
    storage.iter_debt_positions_loop]

theorem iter_debt_positions_loop_spec :
    ∀ (m : List (types.HubAssetKey × types.DebtPositionRaw))
      (out : List (types.HubAssetKey × types.DebtPosition)),
      storage.iter_debt_positions_loop m out ⦃ r =>
        r = out ++ m.map (fun e => (e.1, ({ scaled_amount := e.2.scaled_amount } : types.DebtPosition))) ⦄ := by
  intro m
  induction m with
  | nil =>
    intro out
    unfold storage.iter_debt_positions_loop; rw [loop]
    simp [storage.iter_debt_positions_loop.body,
      aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]
  | cons e m ih =>
    intro out
    obtain ⟨k, dr⟩ := e
    rw [iter_debt_positions_loop_cons]
    step with ih (out ++ [(k, { scaled_amount := dr.scaled_amount })]) as ⟨res, hres⟩
    simp [hres]

/-- **`iter_debt_positions` yields every entry, in order, converted.** -/
theorem iter_debt_positions_spec (m : aeneas_soroban_shim.Map types.HubAssetKey types.DebtPositionRaw) :
    storage.iter_debt_positions m ⦃ l =>
      l = (m : List _).map (fun e => (e.1, ({ scaled_amount := e.2.scaled_amount } : types.DebtPosition))) ⦄ := by
  unfold storage.iter_debt_positions
  simp only [aeneas_soroban_shim.Map.iter, aeneas_soroban_shim.Vec.new, bind_ok,
    aeneas_soroban_shim.Vec.Insts.CoreIterTraitsCollectIntoIteratorTVecIter.into_iter]
  step with iter_debt_positions_loop_spec m [] as ⟨r, hr⟩
  simpa using hr

end ControllerAeneas.Proofs
