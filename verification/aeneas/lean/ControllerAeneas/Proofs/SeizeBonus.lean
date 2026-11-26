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
open FixedPoint.Spec FixedPoint.HalfUpSpec
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

/-- Convert a host `I256` known to fit `i128`, preserving its mathematical value. -/
theorem bonus_to_i128_spec (env : Env) (v : aeneas_soroban_shim.I256)
    (hlo : -2^127 ≤ (v : Int)) (hhi : (v : Int) < 2^127) :
    math.fp_core.to_i128 env v ⦃ r => r.val = (v : Int) ⦄ := by
  unfold math.fp_core.to_i128
  step as ⟨o, ho⟩
  have htm := IScalar.tryMkOpt_eq .I128 v
  rw [← ho] at htm
  cases o with
  | none =>
    exfalso
    apply htm
    simpa [IScalar.inBounds, IScalarTy.numBits] using And.intro hlo hhi
  | some r =>
    obtain ⟨hr, -⟩ := htm
    simp only [core.option.Option.unwrap_or_else, WP.spec_ok]
    exact hr

/-- Exact Euclidean quotient and remainder for the widened non-negative
multiply-add helper. The numerator itself may exceed `i128`; only its quotient
must fit, which is the property needed by high liquidation bonuses. -/
theorem mul_add_div_rem_nonnegative_spec (env : Env) (x y carry d : Std.I128)
    (hx : 0 ≤ x.val) (hy : 0 ≤ y.val) (hc : 0 ≤ carry.val) (hd : 0 < d.val)
    (hqfit : (x.val * y.val + carry.val) / d.val < 2^127) :
    math.fp_core.mul_add_div_rem_nonnegative env x y carry d ⦃ qr =>
      qr.1.val = (x.val * y.val + carry.val) / d.val ∧
      qr.2.val = (x.val * y.val + carry.val) % d.val ⦄ := by
  let n := x.val * y.val + carry.val
  have hprod0 : 0 ≤ x.val * y.val := mul_nonneg hx hy
  have hn : 0 ≤ n := by dsimp [n]; positivity
  have hprod0 : 0 ≤ x.val * y.val := mul_nonneg hx hy
  have hq0 : 0 ≤ n / d.val := Int.ediv_nonneg hn (by omega)
  have hr0 : 0 ≤ n % d.val := Int.emod_nonneg _ (by omega)
  have hrd : n % d.val < d.val := Int.emod_lt_of_pos _ hd
  have hprod_le : x.val * y.val ≤ 2^254 := by
    have h1 : 0 ≤ (2^127 - x.val) * y.val := mul_nonneg (by
      have := x.hBounds; simp only [IScalarTy.numBits] at this; omega) hy
    have h2 : 0 ≤ (2^127 - y.val) * (2^127 : Int) := mul_nonneg (by
      have := y.hBounds; simp only [IScalarTy.numBits] at this; omega) (by positivity)
    nlinarith
  have hn256 : aeneas_soroban_shim.I256.InRange n := by
    have hxb := x.hBounds
    have hyb := y.hBounds
    have hcb := carry.hBounds
    simp only [IScalarTy.numBits] at hxb hyb hcb
    simp only [aeneas_soroban_shim.I256.InRange,
      show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
      show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]
    dsimp [n]
    norm_num at hprod_le ⊢
    omega
  have wide : (do
      let i ← aeneas_soroban_shim.I256.from_i128 env x
      let i1 ← aeneas_soroban_shim.I256.from_i128 env y
      let i2 ← aeneas_soroban_shim.I256.mul i i1
      let i3 ← aeneas_soroban_shim.I256.from_i128 env carry
      let numerator ← aeneas_soroban_shim.I256.add i2 i3
      let divisor ← aeneas_soroban_shim.I256.from_i128 env d
      let i4 ← aeneas_soroban_shim.I256.div numerator divisor
      let i5 ← math.fp_core.to_i128 env i4
      let i6 ← aeneas_soroban_shim.I256.rem_euclid numerator divisor
      let i7 ← math.fp_core.to_i128 env i6
      ok (i5, i7)) ⦃ qr => qr.1.val = n / d.val ∧ qr.2.val = n % d.val ⦄ := by
    step as ⟨ix, hix⟩
    step as ⟨iy, hiy⟩
    step as ⟨ip, hip⟩
    · simp only [aeneas_soroban_shim.I256.InRange, hix, hiy,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]
      norm_num at hprod_le ⊢
      omega
    step as ⟨ic, hic⟩
    step as ⟨inum, hinum⟩
    step as ⟨id, hid⟩
    have hnum : (inum : Int) = n := by rw [hinum, hip, hic, hix, hiy]
    have htdiv : Int.tdiv n d.val = n / d.val := Int.tdiv_eq_ediv_of_nonneg hn
    have htdiv0 : 0 ≤ Int.tdiv n d.val := by rw [htdiv]; exact hq0
    have htdivfit : Int.tdiv n d.val < 2^127 := by rw [htdiv]; exact hqfit
    have hqrange : aeneas_soroban_shim.I256.InRange
        (Int.tdiv (inum : Int) (id : Int)) := by
      simp only [aeneas_soroban_shim.I256.InRange,
        show aeneas_soroban_shim.I256.MIN = -2^255 from rfl,
        show aeneas_soroban_shim.I256.MAX = 2^255 - 1 from rfl]
      rw [hnum, hid]
      norm_num at htdivfit ⊢
      omega
    step with aeneas_soroban_shim.I256.div_spec inum id (by rw [hid]; exact ne_of_gt hd) hqrange
      as ⟨iq, hiq⟩
    have hiq' : (iq : Int) = Int.tdiv n d.val := by rw [hiq, hnum, hid]
    have hiq0 : 0 ≤ (iq : Int) := by rw [hiq']; exact htdiv0
    have hiqfit : (iq : Int) < 2^127 := by rw [hiq']; exact htdivfit
    step with bonus_to_i128_spec env iq (le_trans (by norm_num) hiq0) hiqfit
      as ⟨q, hq⟩
    step as ⟨ir, hir⟩
    rw [hnum, hid] at hir
    have hdhi : d.val < 2^127 := by
      have := d.hBounds
      simp only [IScalarTy.numBits] at this
      norm_num at this ⊢
      omega
    step with bonus_to_i128_spec env ir (by rw [hir]; exact le_trans (by norm_num) hr0)
      (by rw [hir]; exact lt_trans hrd hdhi)
      as ⟨r, hr⟩
    exact ⟨by rw [hq, hiq', htdiv], by
      rw [hr, hir]
      change Int.emod n d.val = Int.emod n d.val
      rfl⟩
  unfold math.fp_core.mul_add_div_rem_nonnegative math.fp_core.require_nonzero_divisor
  have hd0 : ¬ (d = 0#i128) := by intro h; have := congrArg IScalar.val h; simp at this; omega
  have hxn : ¬ (x < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hyn : ¬ (y < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hcn : ¬ (carry < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have hdn : ¬ (d < 0#i128) := by simp [IScalar.lt_equiv]; omega
  simp only [massert, hd0, hxn, hyn, hcn, hdn, not_false_eq_true, ↓reduceIte, bind_ok, lift]
  have hm := I128.checked_mul_bv_spec x y
  rw [show I128.checked_mul x y = core.num.checked_mul_IScalar x y from rfl]
  cases hmo : core.num.checked_mul_IScalar x y with
  | none =>
    simp only [core.option.Option.and_then, bind_ok]
    exact wide
  | some p =>
    rw [hmo] at hm
    obtain ⟨-, -, hp, -⟩ := hm
    simp only [core.option.Option.and_then,
      math.fp_core.mul_add_div_rem_nonnegative.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once,
      bind_ok]
    have ha := I128.checked_add_bv_spec p carry
    rw [show I128.checked_add p carry = core.num.checked_add_IScalar p carry from rfl]
    cases hao : core.num.checked_add_IScalar p carry with
    | none =>
      simp only [hao, bind_ok]
      exact wide
    | some numerator =>
      rw [hao] at ha
      obtain ⟨-, -, hnum, -⟩ := ha
      have hnum' : numerator.val = n := by rw [hnum, hp]
      simp only [hao, bind_ok]
      step as ⟨q, hq⟩
      step as ⟨r, hr⟩
      rw [hnum', Int.tdiv_eq_ediv_of_nonneg hn] at hq
      rw [hnum', Int.tmod_eq_emod_of_nonneg hn] at hr
      exact ⟨hq, hr⟩

theorem bonus_loop_nil (env : Env) (ctx : context.Context) (c : math.fp.Wad)
    (a : math.fp.Bps) (r : math.fp.Wad) :
    positions.liquidation.math.get_account_bonus_params_loop [] env ctx c a r = ok (ctx, a, r) := by
  unfold positions.liquidation.math.get_account_bonus_params_loop; rw [loop]
  simp [positions.liquidation.math.get_account_bonus_params_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The typed loop maintains the exact quotient/remainder decomposition
`acc·C + rem + Σ(value·bonus)`. Its fit condition bounds only the final
quotient, so an individual weighted product may exceed `i128`. -/
theorem bonus_loop_spec (env : Env) (ctx : context.Context) (c : math.fp.Wad)
    (hcpos : 0 < c.val) :
    ∀ (L : List (types.HubAssetKey × types.AccountPosition))
      (a : math.fp.Bps) (r : math.fp.Wad),
      (∀ e ∈ L, Serves ctx e.1 ∧ VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset)
        ∧ 0 ≤ e.2.liquidation_bonus.val) →
      0 ≤ a.val → 0 ≤ r.val → r.val < c.val →
      a.val * c.val + r.val + lsum (bonW ctx) L < 2^127 * c.val →
      positions.liquidation.math.get_account_bonus_params_loop L env ctx c a r ⦃ out =>
        out.1 = ctx ∧ 0 ≤ out.2.1.val ∧
        0 ≤ out.2.2.val ∧ out.2.2.val < c.val ∧
        out.2.1.val * c.val + out.2.2.val =
          a.val * c.val + r.val + lsum (bonW ctx) L ⦄ := by
  intro L
  induction L with
  | nil =>
    intro a r _ ha hr hrc _
    rw [bonus_loop_nil]
    simp [ha, hr, hrc]
  | cons e L ih =>
    intro a r hok ha hr hrc hfit
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
    have hprod : v.val * pos.liquidation_bonus.val = bonW ctx (k, pos) := by
      rw [hv, ← hch]; rfl
    have hprod' : pos.liquidation_bonus.val * v.val = bonW ctx (k, pos) := by
      rw [mul_comm, hprod]
    have hn : 0 ≤ pos.liquidation_bonus.val * v.val + r.val := by
      rw [hprod']
      omega
    have hnlt : pos.liquidation_bonus.val * v.val + r.val < 2^127 * c.val := by
      rw [hprod']
      have hac0 : 0 ≤ a.val * c.val := mul_nonneg ha hcpos.le
      nlinarith
    have hqfit : (pos.liquidation_bonus.val * v.val + r.val) / c.val < 2^127 := by
      have hfloor := Int.ediv_mul_le (pos.liquidation_bonus.val * v.val + r.val)
        (b := c.val) (by omega)
      by_contra h
      have hK : 2^127 ≤ (pos.liquidation_bonus.val * v.val + r.val) / c.val :=
        not_lt.mp h
      have hmul := mul_le_mul_of_nonneg_right hK (by omega)
      nlinarith
    unfold math.fp.Bps.add_weighted_term
    step with mul_add_div_rem_nonnegative_spec env pos.liquidation_bonus v r c
      hlb (by rw [hv, ← hch]; exact hv0) hr hcpos hqfit as ⟨qr, hq, hr1⟩
    obtain ⟨q, r1⟩ := qr
    have hq0 : 0 ≤ q.val := by
      rw [hq]
      exact Int.ediv_nonneg hn (by omega)
    have hr10 : 0 ≤ r1.val := by
      rw [hr1]
      exact Int.emod_nonneg _ (by omega)
    have hr1c : r1.val < c.val := by
      rw [hr1]
      exact Int.emod_lt_of_pos _ hcpos
    have hdecomp : q.val * c.val + r1.val =
        pos.liquidation_bonus.val * v.val + r.val := by
      rw [hq, hr1]
      have h := Int.mul_ediv_add_emod
        (pos.liquidation_bonus.val * v.val + r.val) c.val
      nlinarith
    have haddfit : a.val + q.val < 2^127 := by
      have hm : (a.val + q.val) * c.val < 2^127 * c.val := by
        nlinarith
      exact (Int.mul_lt_mul_right hcpos).mp hm
    unfold math.fp.Bps.checked_add
    step with checked_add_raw_spec env a q (by constructor <;> nlinarith) as ⟨a1, ha1⟩
    have hrecfit : a1.val * c.val + r1.val + lsum (bonW ctx) L < 2^127 * c.val := by
      rw [ha1]
      nlinarith
    have hrec := ih a1 r1 hrest (by rw [ha1]; omega) hr10 hr1c hrecfit
    unfold positions.liquidation.math.get_account_bonus_params_loop at hrec
    apply WP.spec_mono hrec
    rintro ⟨ctx', a', r'⟩ ⟨hctx, ha'0, hr'0, hr'c, hexact⟩
    refine ⟨hctx, ha'0, hr'0, hr'c, ?_⟩
    rw [hexact, ha1]
    nlinarith

theorem bonW_typed (ctx : context.Context) :
    ∀ {m : List (types.HubAssetKey × types.AccountPositionRaw)}
      {l : List (types.HubAssetKey × types.AccountPosition)},
      List.Forall₂ TypedOf m l → lsum (bonW ctx) l = lsum (rBonW ctx) m :=
  forall2_lsum (by
    rintro ⟨k, r⟩ ⟨k', p⟩ ⟨h1, h2, -, h4, -, -⟩
    simp only at h1 h2 h4
    subst h1
    simp [bonW, rBonW, collHU, rHU, h2, h4])

/-- Final half-up rounding of an exact floor quotient and remainder. -/
theorem round_weighted_remainder_spec (env : Env) (a : math.fp.Bps)
    (r d : math.fp.Wad) (ha : 0 ≤ a.val) (hr : 0 ≤ r.val)
    (hrd : r.val < d.val) (hd : 0 < d.val) (hfit : a.val + 1 < 2^127) :
    math.fp.Bps.round_weighted_remainder a env r d ⦃ out =>
      IsHalfUp (a.val * d.val + r.val) d.val out.val ⦄ := by
  unfold math.fp.Bps.round_weighted_remainder
  have h1 : ¬ (r < 0#i128) := by simp [IScalar.lt_equiv]; omega
  have h2 : ¬ (d <= 0#i128) := by simp [IScalar.le_equiv]; omega
  have h3 : ¬ (r >= d) := by simp [IScalar.le_equiv]; omega
  simp only [massert, h1, h2, h3, not_false_eq_true, ↓reduceIte, bind_ok]
  step as ⟨half, hhalf⟩
  step as ⟨odd, hodd⟩
  step as ⟨ceiling, hceiling⟩
  have hsum := Int.mul_tdiv_add_tmod d.val 2
  have hodd0 : 0 ≤ odd.val := by
    rw [hodd, Int.tmod_eq_emod_of_nonneg hd.le]
    exact Int.emod_nonneg _ (by norm_num)
  have hodd2 : odd.val < 2 := by
    rw [hodd, Int.tmod_eq_emod_of_nonneg hd.le]
    exact Int.emod_lt_of_pos _ (by norm_num)
  have hceil : ceiling.val = Int.tdiv d.val 2 + Int.tmod d.val 2 := by
    rw [hceiling, hhalf, hodd]
  have hceil_lo : d.val ≤ 2 * ceiling.val := by rw [hceil]; omega
  have hceil_hi : 2 * ceiling.val ≤ d.val + 1 := by rw [hceil]; omega
  by_cases hround : ceiling.val ≤ r.val
  · rw [ite_eq_left (by simpa using hround)]
    unfold math.fp.Bps.checked_add
    step with checked_add_raw_spec env a 1#i128 (by
      constructor
      · have : (0:Int) ≤ (1#i128 : Std.I128).val := by norm_num
        omega
      · norm_num at hfit ⊢
        exact hfit) as ⟨out, hout⟩
    rw [hout]
    unfold IsHalfUp
    constructor <;> nlinarith
  · rw [ite_eq_right (by simpa using hround)]
    apply (WP.spec_ok _).mpr
    unfold IsHalfUp
    constructor <;> nlinarith

/-- **`get_account_bonus_params` exact result.** With `max` the result of
`max_bonus_for_threshold(p)` (`max_bonus_for_threshold_spec`):
* no collateral (`C = 0`): `base = 0`;
* `C > 0`: `base = min(huD(Σ huv·bonus, C), max)`: the USD-weighted
  average of the stamped liquidation bonuses, rounded half up once (no
  per-leg rounding), capped at `max`.
The call returns (does not abort) and leaves the context unchanged.
Hypotheses: `C ≥ 0` (a sum of valuations); every supply entry served by the
context with its value in `VD`; and the final weighted quotient fits `i128`.
The weighted numerator itself may exceed `i128`. -/
theorem get_account_bonus_params_spec (env : Env) (ctx : context.Context)
    (supply : aeneas_soroban_shim.Map types.HubAssetKey types.AccountPositionRaw)
    (tc p : math.fp.Wad) (htc : 0 ≤ tc.val)
    (hs : ∀ e ∈ supply, Serves ctx e.1 ∧ VD e.2.scaled_amount.val (supplyIndexOf ctx e.1) (priceOf ctx e.1.asset))
    (hfit : 0 < tc.val → lsum (rBonW ctx) supply < (2^127 - 1) * tc.val) :
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
    have hloopfit : math.fp.Bps.ZERO.val * tc.val + math.fp.Wad.ZERO.val +
        lsum (bonW ctx) L < 2^127 * tc.val := by
      have hf := hfit hpos
      rw [← he] at hf
      unfold math.fp.Bps.ZERO math.fp.Wad.ZERO
      simp only [IScalar.ofInt_val_eq, zero_mul, zero_add]
      nlinarith
    step with bonus_loop_spec env ctx tc hpos L math.fp.Bps.ZERO math.fp.Wad.ZERO hLok
      (by unfold math.fp.Bps.ZERO; norm_num)
      (by unfold math.fp.Wad.ZERO; norm_num)
      (by unfold math.fp.Wad.ZERO; norm_num; exact hpos)
      hloopfit as ⟨c, w, rem, hc, hw0, hr0, hrc, hexact⟩
    have hexact' : w.val * tc.val + rem.val = lsum (bonW ctx) L := by
      simpa [math.fp.Bps.ZERO, math.fp.Wad.ZERO] using hexact
    have hwfit : w.val + 1 < 2^127 := by
      have hf := hfit hpos
      rw [← he] at hf
      have hwC : w.val * tc.val + rem.val < (2^127 - 1) * tc.val := by
        rw [hexact']
        exact hf
      have hm : (w.val + 1) * tc.val < 2^127 * tc.val := by nlinarith
      exact (Int.mul_lt_mul_right hpos).mp hm
    step with round_weighted_remainder_spec env w rem tc hw0 hr0 hrc hpos hwfit
      as ⟨rounded, hrounded⟩
    have hroundedval : rounded.val = huD (lsum (rBonW ctx) supply) tc.val := by
      apply IsHalfUp.unique hpos
      · have hrnd := hrounded
        rw [hexact', he] at hrnd
        exact hrnd
      · exact sz_isHalfUp_huD _ _ hpos
    simp only [bps_min, bind_ok, WP.spec_ok]
    refine ⟨hc, trivial, fun h => by omega, fun _ => ?_⟩
    have hminval : (if mx.val < rounded.val then mx else rounded).val =
        min rounded.val mx.val := by
      by_cases h : mx.val < rounded.val
      · rw [if_pos h, min_eq_right (by omega)]
      · rw [if_neg h, min_eq_left (by omega)]
    rw [hminval, hroundedval]

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
    (hfit : 0 < tc.val →
      lsum (rBonW ctx) acc.supply_positions < (2^127 - 1) * tc.val) :
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
