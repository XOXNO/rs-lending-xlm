/-
`liquidation::math::scale_seizures_to_received`: when the repayment actually
received falls short of the planned repayment, every seizure is scaled down by
`received / planned`, rounding down (INV-LIQ-03: under-delivery reduces the
seizure proportionally, never increases it).
-/
import ControllerAeneas.Proofs.Value
import ControllerAeneas.Proofs.Basic
import ControllerAeneas.Proofs.Wrappers

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## `unscale_supply_floor` -/

/-- `rates::scaling::unscale_supply_floor` of non-negative shares at a
non-negative index, for an asset with fewer than 27 decimals, is
`⌊⌊shares·index / RAY⌋ / 10^(27 − decimals)⌋`; it does not abort when
`shares·index < 2^127·RAY`. -/
theorem unscale_supply_floor_spec (env : Env) (x idx : math.fp.Ray) (dec : Std.U32)
    (hx : 0 ≤ x.val) (hi : 0 ≤ idx.val) (hd : dec.val < 27)
    (hfit : x.val * idx.val < 2^127 * 10^27) :
    rates.scaling.unscale_supply_floor env x idx dec ⦃ r =>
      ∃ m, IsFloor (x.val * idx.val) (10^27) m ∧ IsFloor m (10^(27 - dec.val)) r.val ⦄ := by
  unfold rates.scaling.unscale_supply_floor math.fp.Ray.mul_floor
  obtain ⟨q, hq⟩ := isFloor_exists (x.val * idx.val) (10^27) (by norm_num)
  have hq0 : 0 ≤ q := IsFloor.nonneg_of (by norm_num) hq (mul_nonneg hx hi)
  have hq' := (isFloor_pos (by norm_num)).mp hq
  have hqM : q < 2^127 := by
    by_contra h
    have : 2^127 * 10^27 ≤ q * 10^27 := mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    linarith [hq'.1]
  step with mul_div_floor_spec env x idx constants.production.shared.RAY
    (by rw [RAY_val]; norm_num) ⟨q, by rw [RAY_val]; exact hq, by omega, hqM⟩ as ⟨m, hm⟩
  rw [RAY_val] at hm
  have hm0 : 0 ≤ m.val := IsFloor.nonneg_of (by norm_num) hm (mul_nonneg hx hi)
  try simp only [bind_ok]
  unfold math.fp.Ray.to_asset_floor
  rw [RAY_DECIMALS_eq]
  step with rescale_floor_down_spec env m 27#u32 dec (by simpa using hd) as ⟨r, hr, _⟩
  exact ⟨m.val, hm, by simpa using hr hm0⟩

/-! ## One seizure entry -/

/-- The per-entry body of the generated loop, as a standalone computation
(`scale_seizures_to_received_loop_cons` proves the loop runs exactly this on
each entry). -/
def scaleEntry (env : Env) (num den : math.fp.Wad) (entry : types.SeizeEntry) :
    Result types.SeizeEntry := do
  massert (entry.feed.asset_decimals >=
    constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS)
  let scaled_amount ← math.fp.Ray.mul_wad_ratio_floor entry.scaled_amount env num den
  let shares_amount ← rates.scaling.unscale_supply_floor env scaled_amount
    entry.market_index.supply_index entry.feed.asset_decimals
  let i ← math.fp.Wad.apply_ratio_to_amount_floor num env den entry.amount
  let amount := core.cmp.impls.OrdI128.min i shares_amount
  let i1 ← math.fp.Wad.apply_ratio_to_amount_floor num env den entry.protocol_fee
  let i2 := core.cmp.impls.OrdI128.min i1 amount
  let i3 ← math.fp.Ray.mul_wad_ratio_floor entry.bonus_scaled env num den
  ok { entry with amount, protocol_fee := i2, scaled_amount, bonus_scaled := i3 }

theorem scale_loop_nil (env : Env) (num den : math.fp.Wad) (acc : List types.SeizeEntry) :
    positions.liquidation.math.scale_seizures_to_received_loop [] env num den acc = ok acc := by
  unfold positions.liquidation.math.scale_seizures_to_received_loop
  rw [loop]
  simp [positions.liquidation.math.scale_seizures_to_received_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

theorem scale_loop_cons (env : Env) (num den : math.fp.Wad) (e : types.SeizeEntry)
    (l acc : List types.SeizeEntry) :
    positions.liquidation.math.scale_seizures_to_received_loop (e :: l) env num den acc =
      (do
        let e' ← scaleEntry env num den e
        positions.liquidation.math.scale_seizures_to_received_loop l env num den (acc ++ [e'])) := by
  conv => lhs; unfold positions.liquidation.math.scale_seizures_to_received_loop; rw [loop]
  simp [positions.liquidation.math.scale_seizures_to_received_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, scaleEntry,
    aeneas_soroban_shim.Vec.push_back, lift, math.fp.Ray.raw,
    positions.liquidation.math.scale_seizures_to_received_loop]

/-- What production guarantees about a planned seizure entry: its asset has
`MIN_BORROWABLE_ASSET_DECIMALS ≤ decimals ≤ MAX_ASSET_DECIMALS` (3 and 18;
the lower bound is the documented precondition, a smaller one aborts with
`InvalidPayments`), its shares and the supply index are non-negative, and
their product is below `2^127·RAY` (the seized asset value fits `i128`). -/
structure SeizeEntryOk (e : types.SeizeEntry) : Prop where
  dec_lo : 3 ≤ e.feed.asset_decimals.val
  dec_hi : e.feed.asset_decimals.val ≤ 18
  scaled_nonneg : 0 ≤ e.scaled_amount.val
  index_nonneg : 0 ≤ e.market_index.supply_index.val
  value_fits : e.scaled_amount.val * e.market_index.supply_index.val < 2^127 * 10^27

/-- The relation between an entry and its scaled replacement: identity,
feed, index and fee rate unchanged; shares, bonus shares and the requested
amount floored by `num/den`; the amount additionally capped at the floor
asset value of the new shares, and the protocol fee floored and capped at the
new amount. -/
def ScaledEntry (num den : Int) (e e' : types.SeizeEntry) : Prop :=
  e'.hub_asset = e.hub_asset ∧ e'.feed = e.feed ∧ e'.market_index = e.market_index ∧
  e'.liquidation_fees = e.liquidation_fees ∧
  IsFloor (e.scaled_amount.val * num) den e'.scaled_amount.val ∧
  IsFloor (e.bonus_scaled.val * num) den e'.bonus_scaled.val ∧
  (∃ a sh m, IsFloor (e.amount.val * num) den a ∧
    IsFloor (e'.scaled_amount.val * e.market_index.supply_index.val) (10^27) m ∧
    IsFloor m (10^(27 - e.feed.asset_decimals.val)) sh ∧
    e'.amount.val = min a sh) ∧
  (∃ f, IsFloor (e.protocol_fee.val * num) den f ∧ e'.protocol_fee.val = min f e'.amount.val)

theorem scaleEntry_spec (env : Env) (num den : math.fp.Wad) (e : types.SeizeEntry)
    (hn : 0 ≤ num.val) (hnd : num.val < den.val) (he : SeizeEntryOk e) :
    scaleEntry env num den e ⦃ e' => ScaledEntry num.val den.val e e' ⦄ := by
  have hd0 : den.val ≠ 0 := by omega
  have bnd := fun (x : Std.I128) => (by have := x.hBounds; simp only [IScalarTy.numBits] at this; omega :
    -2^127 ≤ x.val ∧ x.val < 2^127)
  unfold scaleEntry
  have hdec : (e.feed.asset_decimals >=
      constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS) := by
    have : (constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS : Std.U32).val = 3 := by
      unfold constants.production.shared.MIN_BORROWABLE_ASSET_DECIMALS; rfl
    simp only [ge_iff_le, UScalar.le_equiv, this]; exact he.dec_lo
  simp only [massert, hdec, ↓reduceIte, bind_ok, mul_wad_ratio_floor_eq,
    apply_ratio_to_amount_floor_eq]
  step with mul_div_floor_spec env e.scaled_amount num den hd0
    (floor_ratio_fits _ _ _ hn hnd (bnd _)) as ⟨sa, hsa⟩
  have hsa' := (isFloor_pos (by omega)).mp hsa
  have hsa0 : 0 ≤ sa.val := IsFloor.nonneg_of (by omega) hsa (mul_nonneg he.scaled_nonneg hn)
  have hsale : sa.val ≤ e.scaled_amount.val :=
    IsFloor.le_of (by omega) hsa (mul_le_mul_of_nonneg_left hnd.le he.scaled_nonneg)
  step with unscale_supply_floor_spec env sa e.market_index.supply_index e.feed.asset_decimals
    hsa0 he.index_nonneg (by have := he.dec_hi; omega)
    (lt_of_le_of_lt (mul_le_mul_of_nonneg_right hsale he.index_nonneg) he.value_fits)
    as ⟨sh, m, hm, hsh⟩
  step with mul_div_floor_spec env e.amount num den hd0
    (floor_ratio_fits _ _ _ hn hnd (bnd _)) as ⟨a, ha⟩
  step with mul_div_floor_spec env e.protocol_fee num den hd0
    (floor_ratio_fits _ _ _ hn hnd (bnd _)) as ⟨f, hf⟩
  step with mul_div_floor_spec env e.bonus_scaled num den hd0
    (floor_ratio_fits _ _ _ hn hnd (bnd _)) as ⟨bs, hbs⟩
  refine ⟨rfl, rfl, rfl, rfl, hsa, hbs, ⟨a.val, sh.val, m, ha, hm, hsh, ?_⟩, ⟨f.val, hf, ?_⟩⟩
  · simp [core.cmp.impls.OrdI128.min] <;> split <;> omega
  · simp only [core.cmp.impls.OrdI128.min_val]

/-! ## The whole vector -/

theorem scale_loop_spec (env : Env) (num den : math.fp.Wad) (hn : 0 ≤ num.val)
    (hnd : num.val < den.val) :
    ∀ (l acc : List types.SeizeEntry), (∀ e ∈ l, SeizeEntryOk e) →
      positions.liquidation.math.scale_seizures_to_received_loop l env num den acc ⦃ out =>
        ∃ l', out = acc ++ l' ∧ List.Forall₂ (ScaledEntry num.val den.val) l l' ⦄ := by
  intro l
  induction l with
  | nil =>
    intro acc _
    rw [scale_loop_nil]
    exact (WP.spec_ok _).mpr ⟨[], by simp, List.Forall₂.nil⟩
  | cons e l ih =>
    intro acc hok
    rw [scale_loop_cons]
    step with scaleEntry_spec env num den e hn hnd (hok e (by simp)) as ⟨e', he'⟩
    apply WP.spec_mono (ih (acc ++ [e']) (fun x hx => hok x (by simp [hx])))
    rintro out ⟨l', hout, hl'⟩
    exact ⟨e' :: l', by simp [hout], List.Forall₂.cons he' hl'⟩

/-- **`scale_seizures_to_received` exact behaviour (INV-LIQ-03).**
For received `0 ≤ r` and planned `p` USD (WAD):
* if `p ≤ 0` or `r ≥ p` the seizures are returned unchanged;
* otherwise (a short receipt, `0 ≤ r < p`) the call returns (does not abort)
  a vector of the same length whose `k`-th entry is the `k`-th input entry
  scaled by `r/p` in the sense of `ScaledEntry`: shares, bonus shares and
  amount floored by `r/p`, the amount also capped at the floor value of the
  new shares, the protocol fee floored and capped at the new amount, every
  other field unchanged.

Hypotheses: `0 ≤ received` (a USD value) and, on the short-receipt path,
each entry satisfies `SeizeEntryOk` (decimals in `[3, 18]`, non-negative
shares and index, value fits `i128`). -/
theorem scale_seizures_to_received_spec (env : Env) (seized : List types.SeizeEntry)
    (received planned : math.fp.Wad) (hr : 0 ≤ received.val)
    (hok : received.val < planned.val → ∀ e ∈ seized, SeizeEntryOk e) :
    positions.liquidation.math.scale_seizures_to_received env seized received planned ⦃ out =>
      ((planned.val ≤ 0 ∨ planned.val ≤ received.val) → out = seized) ∧
      ((0 < planned.val ∧ received.val < planned.val) →
        List.Forall₂ (ScaledEntry received.val planned.val) seized out) ⦄ := by
  unfold positions.liquidation.math.scale_seizures_to_received
  rw [wad_le]
  simp only [bind_ok]
  have hz : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  rw [hz]
  by_cases h0 : planned.val ≤ 0
  · rw [show decide (planned.val ≤ 0) = true from by simp [h0]]
    simp only [↓reduceIte, aeneas_soroban_shim.Vec.Insts.CoreCloneClone.clone]
    exact (WP.spec_ok _).mpr ⟨fun _ => rfl, fun h => absurd h.1 (by omega)⟩
  · rw [show decide (planned.val ≤ 0) = false from by simp [h0]]
    simp only [Bool.false_eq_true, ↓reduceIte, wad_ge, bind_ok]
    by_cases h1 : planned.val ≤ received.val
    · rw [show decide (planned.val ≤ received.val) = true from by simp [h1]]
      simp only [↓reduceIte, aeneas_soroban_shim.Vec.Insts.CoreCloneClone.clone]
      exact (WP.spec_ok _).mpr ⟨fun _ => rfl, fun h => absurd h.2 (by omega)⟩
    · rw [show decide (planned.val ≤ received.val) = false from by simp [h1]]
      simp only [Bool.false_eq_true, ↓reduceIte, math.fp.Wad.raw, bind_ok,
        aeneas_soroban_shim.Vec.new, aeneas_soroban_shim.Vec.iter]
      apply WP.spec_mono (scale_loop_spec env received planned hr (by omega) seized []
        (hok (by omega)))
      rintro out ⟨l', hout, hl'⟩
      refine ⟨fun h => by omega, fun _ => ?_⟩
      simpa [hout] using hl'

/-- **Short receipts never increase a seizure (INV-LIQ-03), per entry.**
For an entry `e` scaled to `e'` by `num/den` with `0 ≤ num < den`, and
non-negative input fields: the new shares, amount and fee are at most
`num/den` of the old ones (`x'·den ≤ x·num`), so in particular at most the
old ones; the protocol fee never exceeds the new amount; and if the bonus
shares were at most the seized shares before, they still are. -/
theorem ScaledEntry.proportional {num den : Int} {e e' : types.SeizeEntry}
    (h : ScaledEntry num den e e') (hn : 0 ≤ num) (hnd : num < den)
    (hs : 0 ≤ e.scaled_amount.val) (ha : 0 ≤ e.amount.val) :
    e'.scaled_amount.val * den ≤ e.scaled_amount.val * num ∧
    e'.amount.val * den ≤ e.amount.val * num ∧
    e'.protocol_fee.val ≤ e'.amount.val ∧
    e'.scaled_amount.val ≤ e.scaled_amount.val ∧
    e'.amount.val ≤ e.amount.val ∧
    (e.bonus_scaled.val ≤ e.scaled_amount.val → e'.bonus_scaled.val ≤ e'.scaled_amount.val) := by
  obtain ⟨-, -, -, -, hsc, hbs, ⟨a, sh, m, ha', -, -, hamt⟩, ⟨f, -, hfee⟩⟩ := h
  have hd : 0 < den := by omega
  rw [isFloor_pos hd] at hsc ha'
  have hamtle : e'.amount.val ≤ a := by rw [hamt]; exact min_le_left _ _
  have h1 : e'.amount.val * den ≤ e.amount.val * num :=
    le_trans (mul_le_mul_of_nonneg_right hamtle hd.le) ha'.1
  have hsd : e.scaled_amount.val * num ≤ e.scaled_amount.val * den :=
    mul_le_mul_of_nonneg_left hnd.le hs
  have had : e.amount.val * num ≤ e.amount.val * den :=
    mul_le_mul_of_nonneg_left hnd.le ha
  refine ⟨hsc.1, h1, by rw [hfee]; exact min_le_right _ _, ?_, ?_, ?_⟩
  · by_contra hc
    have : (e.scaled_amount.val + 1) * den ≤ e'.scaled_amount.val * den :=
      mul_le_mul_of_nonneg_right (by omega) hd.le
    linarith [hsc.1]
  · by_contra hc
    have : (e.amount.val + 1) * den ≤ e'.amount.val * den :=
      mul_le_mul_of_nonneg_right (by omega) hd.le
    linarith
  · intro hbs_le
    rw [isFloor_pos hd] at hbs
    have hmono : e.bonus_scaled.val * num ≤ e.scaled_amount.val * num :=
      mul_le_mul_of_nonneg_right hbs_le hn
    by_contra hc
    have : (e'.scaled_amount.val + 1) * den ≤ e'.bonus_scaled.val * den :=
      mul_le_mul_of_nonneg_right (by omega) hd.le
    linarith [hbs.1, hsc.2]

end ControllerAeneas.Proofs
