-- Copied from RatesAeneas/Proofs/FloorCeilResolve.lean (2026-11-12 snapshot) for the pool
-- extraction, with `RatesAeneas`/`rates_aeneas`/`aeneas_soroban_shim` renamed to
-- `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim` and `constants.production.` to
-- `constants.` (WRITING-PROOFS.md, last section). The trailing `i128_saturating_sub_val`
-- and `protocol_fee_shares_spec` are omitted (the pool has its own `protocol_fee_shares_spec`
-- in Accrual.lean). Proof text otherwise unchanged.
/-
Bounds on the settlement resolvers of `common/src/rates/scaling.rs`
(`resolve_repay`, `resolve_net_settle`) and on `rates/index.rs::
protocol_fee_shares`. Unbounded counterparts of the Kani harnesses in
`verification/kani/src/proofs/scaling.rs`.

Notation: `RAY = 10^27`, `k = 27 - decimals`, `P = RAY * 10^k` (one asset
unit expressed in `scaled * index` units).
-/
import PoolAeneas.Proofs.FloorCeilRoundTrip

open Aeneas Aeneas.Std Result
open FixedPoint.Spec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-! ## `resolve_repay` -/

/-- **`resolve_repay` never burns more debt shares than held and never clears
more debt than paid.** Let `D = ⌈pos*index/P⌉` be the position's ceiling debt
in asset units (what `unscale_borrow_ceil` returns). Then `resolve_repay`
returns (does not abort) a pair `(burn, refund)` with:

* full repayment (`D ≤ amount`): `burn = pos` (the whole position) and
  `refund = amount - D ≥ 0` (the excess is returned, nothing more);
* partial repayment (`amount < D`): `refund = 0` and
  `burn = ⌊amount*10^k*RAY/index⌋` (floor, ADR-0003), so
  `burn*index ≤ amount*10^k*RAY` (the burned debt is never worth more than the
  payment) and `0 ≤ burn < pos` (strictly fewer shares than held).

Hypotheses: `decimals < 27` (listed assets have ≤ `MAX_ASSET_DECIMALS = 18`);
`0 < index` (borrow index ≥ RAY, INV-IDX-01); `0 ≤ pos` (debt shares are
non-negative, INV-ACCT-01); `0 ≤ amount` (repay entry rejects non-positive
amounts); `⌈pos*index/RAY⌉` fits `i128` (otherwise `unscale_borrow_ceil`
aborts). All other fits are derived. -/
theorem resolve_repay_spec (env : Env) (amount pos index : Std.I128) (dec : Std.U32)
    (hdec : dec.val < 27) (hidx : 0 < index.val) (hpos : 0 ≤ pos.val) (hamt : 0 ≤ amount.val)
    (hfit : ∃ q : Int, IsCeil (pos.val * index.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    rates.scaling.resolve_repay env amount pos index dec ⦃ p => ∀ D : Int,
      IsCeil (pos.val * index.val) (10^27 * 10^(27 - dec.val)) D →
      (D ≤ amount.val → p.1 = pos ∧ p.2.val = amount.val - D ∧ 0 ≤ p.2.val) ∧
      (amount.val < D → p.2.val = 0 ∧
        IsFloor (amount.val * 10^(27 - dec.val) * 10^27) index.val p.1.val ∧
        0 ≤ p.1.val ∧ p.1.val < pos.val) ⦄ := by
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - dec.val)
  have hP : (0:Int) < 10^27 * 10^(27 - dec.val) := by positivity
  have hposb := pos.hBounds
  simp only [IScalarTy.numBits] at hposb
  obtain ⟨q0, hq0, hq0lo, hq0hi⟩ := hfit
  unfold rates.scaling.resolve_repay
  step with unscale_borrow_ceil_spec env pos index dec (by omega) ⟨q0, hq0, hq0lo, hq0hi⟩
    as ⟨D0, hD0⟩
  -- facts about the ceiling debt
  have hpi : 0 ≤ pos.val * index.val := by positivity
  have hD0c := hD0
  unfold IsCeil at hD0c; rw [if_pos hP] at hD0c
  have hD0nn : 0 ≤ D0.val := by
    by_contra h
    have : D0.val * (10^27 * 10^(27 - dec.val)) < 0 := by nlinarith
    omega
  by_cases hge : D0.val ≤ amount.val
  · have hge' : (amount >= D0) := by simp [IScalar.le_equiv]; omega
    simp only [hge', ↓reduceIte, lift, bind_ok]
    have hsub := I128.checked_sub_bv_spec amount D0
    rw [show I128.checked_sub amount D0 = core.num.checked_sub_IScalar amount D0 from rfl]
    cases hc : core.num.checked_sub_IScalar amount D0 with
    | none =>
      exfalso; rw [hc] at hsub; apply hsub
      have := amount.hBounds; have := D0.hBounds
      simp only [I128.min, I128.max, I128.numBits, IScalarTy.numBits] at *; omega
    | some r =>
      rw [hc] at hsub; obtain ⟨_, _, hr, _⟩ := hsub
      simp only [core.option.Option.unwrap_or_else, bind_ok, WP.spec_ok]
      intro D hD
      have hDe : D = D0.val := IsCeil.unique (by omega) hD hD0
      subst hDe
      refine ⟨fun _ => ⟨by simp, by simp [hr], by omega⟩, fun h => absurd h (by omega)⟩
  · have hlt' : ¬ (amount >= D0) := by simp [IScalar.le_equiv]; omega
    simp only [hlt', ↓reduceIte]
    -- amount ≤ D0 - 1, so amount*P < pos*index
    have hAP : amount.val * (10^27 * 10^(27 - dec.val)) < pos.val * index.val := by
      have : amount.val * (10^27 * 10^(27 - dec.val)) ≤
          (D0.val - 1) * (10^27 * 10^(27 - dec.val)) :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    -- A = amount*10^k fits: A*RAY ≤ amount*P < pos*index < (q0+1)*RAY
    have hq0c := hq0
    unfold IsCeil at hq0c; rw [if_pos (by norm_num)] at hq0c
    have hA0 : 0 ≤ amount.val * 10^(27 - dec.val) := by positivity
    have hAlt : amount.val * 10^(27 - dec.val) < 2^127 := by
      have h1 : amount.val * 10^(27 - dec.val) * 10^27 < q0 * 10^27 := by nlinarith
      have h2 : amount.val * 10^(27 - dec.val) < q0 := lt_of_mul_lt_mul_right h1 (by norm_num)
      omega
    obtain ⟨b, hb⟩ := exists_isFloor (amount.val * 10^(27 - dec.val) * 10^27) index.val (by omega)
    have hbc := hb
    unfold IsFloor at hbc; rw [if_pos hidx] at hbc
    have hb0 : 0 ≤ b := by
      by_contra h
      have : (b + 1) * index.val ≤ 0 := by nlinarith
      nlinarith
    have hbpos : b < pos.val := by
      by_contra h
      have : pos.val * index.val ≤ b * index.val := mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    step with calculate_scaled_borrow_floor_spec env amount dec index hdec (by omega)
      ⟨by omega, hAlt⟩ ⟨b, hb, by omega, by omega⟩ as ⟨s, hs⟩
    have hsb : s.val = b := IsFloor.unique (by omega) hs hb
    intro D hD
    have hDe : D = D0.val := IsCeil.unique (by omega) hD hD0
    subst hDe
    refine ⟨fun h => absurd h (by omega), fun _ => ⟨by simp, hs, by omega, by omega⟩⟩

/-! ## `resolve_net_settle` -/

/-- `Ord::min` on `Ray` (the derived `Ord`) is the minimum of the raw values. -/
theorem ray_min_spec (x y : Std.I128) :
    core.cmp.Ord.min.trait_default math.fp.Ray.Insts.CoreCmpOrd x y ⦃ r =>
      IScalar.val (ty := .I128) r = min x.val y.val ⦄ := by
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default, core.cmp.Ord.min_body,
    math.fp.Ray.Insts.CoreCmpOrd, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp,
    math.fp.Ray.Insts.CoreCmpOrd.cmp, bind_ok]
  by_cases h : y.val < x.val
  · simp [h, compare_lt_iff_lt]; omega
  · simp [h, compare_lt_iff_lt]; omega

/-- `Ray::ZERO` is 0. -/
theorem ray_zero_val : IScalar.val (ty := .I128) math.fp.Ray.ZERO = 0 := by
  unfold math.fp.Ray.ZERO; simp

/-- **`resolve_net_settle` bounds.** Offsetting same-asset supply
(`ss` shares at supply index `si`) against debt (`ds` shares at borrow index
`bi`) for a requested `amount`. Let `F = ⌊ss*si/P⌋` (floor-valued supply
claim, `unscale_supply_floor`) and `D = ⌈ds*bi/P⌉` (ceiling-valued debt,
`unscale_borrow_ceil`). The call returns (does not abort)
`(burned_supply, burned_debt, settled)` with:

* `0 ≤ burned_supply ≤ ss` and `0 ≤ burned_debt ≤ ds`: never burns more
  shares than held on either side;
* `settled ≥ 0`, and when positive, `settled ≤ amount`, `settled ≤ F`
  (never pays more than the floor-valued claim) and `settled ≤ D`;
* `settled = 0` burns nothing;
* a side is burned entirely exactly when the settlement exhausts it
  (`settled = F` gives `burned_supply = ss`, `settled = D` gives
  `burned_debt = ds`);
* partial burns are directed in the protocol's favour (ADR-0003): a partial
  supply burn is worth at least the settled amount
  (`settled*10^k*RAY ≤ burned_supply*si`, ceiling), and a partial debt burn
  clears at most the settled amount (`burned_debt*bi ≤ settled*10^k*RAY`,
  floor). The `min(_, position)` caps in the code are never binding.

Hypotheses: `decimals < 27` (listed assets ≤ `MAX_ASSET_DECIMALS = 18`);
`0 < si`, `0 < bi` (indexes ≥ `SUPPLY_INDEX_FLOOR_RAW`, ≥ RAY; INV-IDX-01/02);
`0 ≤ ss`, `0 ≤ ds` (INV-ACCT-01); the intermediates `⌊ss*si/RAY⌋` and
`⌈ds*bi/RAY⌉` fit `i128` (otherwise the unscale calls abort). Every other
fit is derived. -/
theorem resolve_net_settle_spec (env : Env) (amount ss ds si bi : Std.I128) (dec : Std.U32)
    (hdec : dec.val < 27) (hsi : 0 < si.val) (hbi : 0 < bi.val)
    (hss : 0 ≤ ss.val) (hds : 0 ≤ ds.val)
    (hfitS : ∃ q : Int, IsFloor (ss.val * si.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127)
    (hfitD : ∃ q : Int, IsCeil (ds.val * bi.val) (10^27) q ∧ -2^127 ≤ q ∧ q < 2^127) :
    rates.scaling.resolve_net_settle env amount ss ds si bi dec ⦃ p => ∀ F D : Int,
      IsFloor (ss.val * si.val) (10^27 * 10^(27 - dec.val)) F →
      IsCeil (ds.val * bi.val) (10^27 * 10^(27 - dec.val)) D →
      (0 ≤ p.1.val ∧ p.1.val ≤ ss.val) ∧
      (0 ≤ p.2.1.val ∧ p.2.1.val ≤ ds.val) ∧
      0 ≤ p.2.2.val ∧
      (0 < p.2.2.val → p.2.2.val ≤ amount.val ∧ p.2.2.val ≤ F ∧ p.2.2.val ≤ D) ∧
      (p.2.2.val = 0 → p.1.val = 0 ∧ p.2.1.val = 0) ∧
      (0 < p.2.2.val → p.2.2.val = F → p.1 = ss) ∧
      (0 < p.2.2.val → p.2.2.val = D → p.2.1 = ds) ∧
      (0 < p.2.2.val → p.2.2.val < F →
        p.2.2.val * 10^(27 - dec.val) * 10^27 ≤ p.1.val * si.val) ∧
      (0 < p.2.2.val → p.2.2.val < D →
        p.2.1.val * bi.val ≤ p.2.2.val * 10^(27 - dec.val) * 10^27) ⦄ := by
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - dec.val)
  have hP : (0:Int) < 10^27 * 10^(27 - dec.val) := by positivity
  have hssb := ss.hBounds
  have hdsb := ds.hBounds
  simp only [IScalarTy.numBits] at hssb hdsb
  obtain ⟨qS, hqS, hqSlo, hqShi⟩ := hfitS
  obtain ⟨qD, hqD, hqDlo, hqDhi⟩ := hfitD
  have hssi : 0 ≤ ss.val * si.val := by positivity
  have hdbi : 0 ≤ ds.val * bi.val := by positivity
  unfold rates.scaling.resolve_net_settle
  step with unscale_supply_floor_spec env ss si dec (by omega) hssi ⟨qS, hqS, hqSlo, hqShi⟩
    as ⟨F0, hF0⟩
  step with unscale_borrow_ceil_spec env ds bi dec (by omega) ⟨qD, hqD, hqDlo, hqDhi⟩
    as ⟨D0, hD0⟩
  have hF0c := hF0
  unfold IsFloor at hF0c; rw [if_pos hP] at hF0c
  have hD0c := hD0
  unfold IsCeil at hD0c; rw [if_pos hP] at hD0c
  have hqSc := hqS
  unfold IsFloor at hqSc; rw [if_pos (by norm_num)] at hqSc
  have hqDc := hqD
  unfold IsCeil at hqDc; rw [if_pos (by norm_num)] at hqDc
  simp only [lift, bind_ok]
  generalize hi : core.cmp.impls.OrdI128.min amount F0 = i
  have hiv : i.val = min amount.val F0.val := by rw [← hi]; simp
  generalize hst : core.cmp.impls.OrdI128.min i D0 = st
  have hstv : st.val = min i.val D0.val := by rw [← hst]; simp
  by_cases hle : st.val ≤ 0
  · have hle' : (st <= 0#i128) := by simp [IScalar.le_equiv]; omega
    simp only [hle', ↓reduceIte, WP.spec_ok]
    intro F D _ _
    simp only [ray_zero_val]
    have h0 : (0#i128 : Std.I128).val = 0 := by simp
    rw [h0]
    exact ⟨le_refl _, hss, le_refl _, hds, le_refl _, by simp, by simp, by simp, by simp,
      by simp, by simp⟩
  · have hle' : ¬ (st <= 0#i128) := by simp [IScalar.le_equiv]; omega
    simp only [hle', ↓reduceIte]
    have hstF : st.val ≤ F0.val := by omega
    have hstD : st.val ≤ D0.val := by omega
    have hstA : st.val ≤ amount.val := by omega
    -- the amount in RAY units, A = st * 10^k
    have hA0 : 0 ≤ st.val * 10^(27 - dec.val) := by
      have : 0 ≤ st.val := by omega
      positivity
    -- the debt side, computed first as a pure fact
    have debt_side : ∀ (bs : Std.I128), (do
        if st = D0 then ok (bs, ds, st)
        else
          let r ← rates.scaling.calculate_scaled_borrow_floor env st dec bi
          let burned_debt ← core.cmp.Ord.min.trait_default math.fp.Ray.Insts.CoreCmpOrd r ds
          ok (bs, burned_debt, st)) ⦃ p => p.1 = bs ∧ p.2.2 = st ∧
            0 ≤ p.2.1.val ∧ p.2.1.val ≤ ds.val ∧
            (st.val = D0.val → p.2.1 = ds) ∧
            (st.val < D0.val → p.2.1.val * bi.val ≤ st.val * 10^(27 - dec.val) * 10^27) ⦄ := by
      intro bs
      by_cases he : st = D0
      · simp only [he, ↓reduceIte, WP.spec_ok]
        exact ⟨trivial, trivial, hds, le_refl _, fun _ => trivial, fun h => absurd h (lt_irrefl _)⟩
      · simp only [he, ↓reduceIte]
        have hlt : st.val < D0.val := by
          rcases lt_or_eq_of_le hstD with h | h
          · exact h
          · exact absurd (IScalar.eq_of_val_eq h) he
        -- st*P < ds*bi
        have hsP : st.val * (10^27 * 10^(27 - dec.val)) < ds.val * bi.val := by
          have : st.val * (10^27 * 10^(27 - dec.val)) ≤
              (D0.val - 1) * (10^27 * 10^(27 - dec.val)) :=
            mul_le_mul_of_nonneg_right (by omega) (by omega)
          nlinarith
        have hAlt : st.val * 10^(27 - dec.val) < 2^127 := by
          have h1 : st.val * 10^(27 - dec.val) * 10^27 < qD * 10^27 := by nlinarith
          have h2 : st.val * 10^(27 - dec.val) < qD := lt_of_mul_lt_mul_right h1 (by norm_num)
          omega
        obtain ⟨b, hb⟩ := exists_isFloor (st.val * 10^(27 - dec.val) * 10^27) bi.val (by omega)
        have hbc := hb
        unfold IsFloor at hbc; rw [if_pos hbi] at hbc
        have hb0 : 0 ≤ b := by
          by_contra h
          have : (b + 1) * bi.val ≤ 0 := by nlinarith
          nlinarith
        have hbds : b < ds.val := by
          by_contra h
          have : ds.val * bi.val ≤ b * bi.val := mul_le_mul_of_nonneg_right (by omega) (by omega)
          nlinarith
        step with calculate_scaled_borrow_floor_spec env st dec bi hdec (by omega)
          ⟨by omega, hAlt⟩ ⟨b, hb, by omega, by omega⟩ as ⟨r, hr⟩
        have hrb : r.val = b := IsFloor.unique (by omega) hr hb
        step with ray_min_spec r ds as ⟨m, hm⟩
        have hmv : m.val = b := by rw [hm, hrb]; omega
        refine ⟨by omega, by omega, fun h => absurd h (by omega), fun _ => ?_⟩
        rw [hmv]; exact hbc.1
    have hrest := fun F D (hF : IsFloor (ss.val * si.val) (10^27 * 10^(27 - dec.val)) F)
        (hD : IsCeil (ds.val * bi.val) (10^27 * 10^(27 - dec.val)) D) =>
      And.intro (IsFloor.unique (by omega) hF hF0) (IsCeil.unique (by omega) hD hD0)
    by_cases heF : st = F0
    · simp only [heF, ↓reduceIte, bind_ok]
      subst heF
      apply WP.spec_mono (debt_side ss)
      rintro ⟨p1, p2, p3⟩ ⟨h1, h2, h3, h4, h5, h6⟩
      simp only at h1 h2 h3 h4 h5 h6 ⊢
      subst h1 h2
      intro F D hF hD
      obtain ⟨rfl, rfl⟩ := hrest F D hF hD
      exact ⟨hss, le_refl _, h3, h4, by omega, fun _ => ⟨hstA, le_refl _, hstD⟩,
        fun h => absurd h (by omega), fun _ _ => rfl, fun _ h => h5 h,
        fun _ h => absurd h (lt_irrefl _), fun _ h => h6 h⟩
    · simp only [heF, ↓reduceIte]
      have hlt : st.val < F0.val := by
        rcases lt_or_eq_of_le hstF with h | h
        · exact h
        · exact absurd (IScalar.eq_of_val_eq h) heF
      -- st*P ≤ ss*si - P
      have hsP : st.val * (10^27 * 10^(27 - dec.val)) + 10^27 * 10^(27 - dec.val)
          ≤ ss.val * si.val := by
        have : st.val * (10^27 * 10^(27 - dec.val)) ≤
            (F0.val - 1) * (10^27 * 10^(27 - dec.val)) :=
          mul_le_mul_of_nonneg_right (by omega) (by omega)
        nlinarith
      have hAlt : st.val * 10^(27 - dec.val) < 2^127 := by
        have h1 : st.val * 10^(27 - dec.val) * 10^27 < (qS + 1) * 10^27 := by nlinarith
        have h2 : st.val * 10^(27 - dec.val) < qS + 1 :=
          lt_of_mul_lt_mul_right h1 (by norm_num)
        omega
      obtain ⟨c, hc⟩ := exists_isCeil (st.val * 10^(27 - dec.val) * 10^27) si.val (by omega)
      have hcc := hc
      unfold IsCeil at hcc; rw [if_pos hsi] at hcc
      have heqA : st.val * 10^(27 - dec.val) * 10^27 = st.val * (10^27 * 10^(27 - dec.val)) := by
        ring
      have hA27 : 0 ≤ st.val * 10^(27 - dec.val) * 10^27 := by positivity
      have hc0 : 0 ≤ c := by
        by_contra h
        have : c * si.val < 0 := mul_neg_of_neg_of_pos (by omega) hsi
        linarith [hcc.1]
      have hcss : c ≤ ss.val := by
        by_contra h
        have : (ss.val + 1) * si.val ≤ c * si.val :=
          mul_le_mul_of_nonneg_right (by omega) (by omega)
        linarith [hcc.2]
      step with calculate_scaled_supply_ceil_spec env st dec si hdec (by omega)
        ⟨by omega, hAlt⟩ ⟨c, hc, by omega, by omega⟩ as ⟨r, hr⟩
      have hrc : r.val = c := IsCeil.unique (by omega) hr hc
      step with ray_min_spec r ss as ⟨m, hm⟩
      have hmv : m.val = c := by rw [hm, hrc]; omega
      apply WP.spec_mono (debt_side m)
      rintro ⟨p1, p2, p3⟩ ⟨h1, h2, h3, h4, h5, h6⟩
      simp only at h1 h2 h3 h4 h5 h6 ⊢
      subst h1 h2
      intro F D hF hD
      obtain ⟨rfl, rfl⟩ := hrest F D hF hD
      refine ⟨by omega, by omega, h3, h4, by omega, fun _ => ⟨hstA, hstF, hstD⟩,
        fun h => absurd h (by omega), fun _ h => absurd h (by omega), fun _ h => h5 h,
        fun _ _ => ?_, fun _ h => h6 h⟩
      rw [hmv]; exact hcc.1

end PoolAeneas.Proofs
