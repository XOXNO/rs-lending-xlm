-- Copied from RatesAeneas/Proofs/Index.lean (2026-11-12 snapshot) for the pool
-- extraction (global_sync / accrue_step proofs), with `RatesAeneas`/`rates_aeneas`/
-- `aeneas_soroban_shim` renamed to `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim`,
-- `constants.production.` to `constants.`, and the theorem namespace
-- `RatesAeneas.Proofs` to `PoolAeneas.RatesProofs` (several names clash with
-- pool-local lemmas in `PoolAeneas.Proofs`) and `types.MarketParams` to
-- `types.pool.MarketParams`. Proof text otherwise unchanged.
import PoolAeneas.Proofs.RatesCurveBase

/-
`rates::index` (common/src/rates/index.rs): borrow and supply index updates,
the supplier-rewards / protocol-fee split, and the supply-index reward
shortfall.
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas
open PoolAeneas.Proofs

set_option maxRecDepth 20000

namespace PoolAeneas.RatesProofs

local notation "rv " r:max => IScalar.val (ty := IScalarTy.I128) r

/-- Bind inversion for total correctness: if `m >>= k` returns with `Q`, then
`m` returns with a value from which `k` returns with `Q`. -/
theorem spec_bind_inv {α β : Type} {m : Result α} {k : α → Result β} {Q : β → Prop}
    (h : (Std.bind m k) ⦃ v => Q v ⦄) : m ⦃ v => k v ⦃ w => Q w ⦄ ⦄ := by
  cases m using Result.cases with
  | ret v => simpa using h
  | vis e k' => simp at h
  | div => simp at h

/-- `MAX_BORROW_INDEX_RAY` is `10^36`. -/
theorem max_borrow_index_val :
    rv constants.pool.MAX_BORROW_INDEX_RAY = 10^36 := by
  unfold constants.pool.MAX_BORROW_INDEX_RAY; simp

/-- `MAX_SUPPLY_INDEX_RAY` equals `MAX_BORROW_INDEX_RAY = 10^36`. -/
theorem max_supply_index_val :
    rv constants.pool.MAX_SUPPLY_INDEX_RAY = 10^36 := by
  unfold constants.pool.MAX_SUPPLY_INDEX_RAY; exact max_borrow_index_val

/-! ## `update_borrow_index` -/

/-- **`update_borrow_index` never decreases and never exceeds the cap
(INV-IDX-01).** For an old index in `[0, MAX_BORROW_INDEX_RAY]` (every stored
borrow index starts at `RAY` and is only ever written by this function, so it
stays in range by induction) and a growth factor in `[0, 100·RAY]`
(`compound_interest` bounds its exponent by `MAX_COMPOUND_DELTA_MS` times
`MAX_BORROW_RATE_RAY`, i.e. at 2, so the factor is below `e^2·RAY`), the call
never aborts and returns `n ≤ MAX_BORROW_INDEX_RAY`; when the factor is at
least `RAY` (interest never shrinks debt), also `old ≤ n`. Below the cap,
`n = half_up(old * factor / RAY)`. -/
theorem update_borrow_index_spec (env : Env) (old f : math.fp.Ray)
    (ho : 0 ≤ rv old) (hoM : rv old ≤ 10^36) (hf : 0 ≤ rv f) (hfM : rv f ≤ 100 * 10^27) :
    rates.index.update_borrow_index env old f ⦃ n =>
      rv n ≤ 10^36 ∧ (10^27 ≤ rv f → rv old ≤ rv n) ∧
      (∀ q, IsHalfUp (rv old * rv f) (10^27) q → rv n = min q (10^36)) ⦄ := by
  unfold rates.index.update_borrow_index
  obtain ⟨q, hq⟩ := IsHalfUp.exists (rv old * rv f) (10^27) (by norm_num)
  have hqle : q ≤ 100 * 10^36 := by
    have hm : rv old * rv f ≤ 10^36 * (100 * 10^27) := by
      have := Int.mul_le_mul_of_nonneg_right hoM hf
      have := Int.mul_le_mul_of_nonneg_left hfM (by norm_num : (0:Int) ≤ 10^36)
      omega
    have := IsHalfUp.mono (by norm_num) hm hq
      (IsHalfUp.mul_self (100 * 10^36) (10^27) (by norm_num))
    norm_num at this ⊢; linarith [this]
  step with ray_mul_spec env old f ho hf ⟨q, hq, by omega⟩ as ⟨n, hn⟩
  have hnq : rv n = q := IsHalfUp.unique (by norm_num) hn hq
  have hmono : 10^27 ≤ rv f → rv old ≤ q := by
    intro h
    exact IsHalfUp.mono (by norm_num) (Int.mul_le_mul_of_nonneg_left h ho)
      (IsHalfUp.mul_self (rv old) (10^27) (by norm_num)) hq
  have uniq : ∀ q', IsHalfUp (rv old * rv f) (10^27) q' → q' = q :=
    fun q' h => IsHalfUp.unique (by norm_num) h hq
  simp only [math.fp.Ray.raw, bind_ok, max_borrow_index_val]
  by_cases hc : n > constants.pool.MAX_BORROW_INDEX_RAY
  · have hc' : 10^36 < rv n := by simpa [IScalar.lt_equiv, max_borrow_index_val] using hc
    simp only [hc, ↓reduceIte, math.fp.Ray.from, core.convert.IntoFrom,
      core.convert.IntoFrom.into, core.convert.FromSame, core.convert.FromSame.from, bind_ok,
      WP.spec_ok, max_borrow_index_val]
    refine ⟨le_refl _, fun h => by have := hmono h; omega, fun q' h => ?_⟩
    rw [uniq q' h]; omega
  · have hc' : ¬ 10^36 < rv n := by simpa [IScalar.lt_equiv, max_borrow_index_val] using hc
    simp only [hc, ↓reduceIte, WP.spec_ok]
    refine ⟨by omega, fun h => by have := hmono h; omega, fun q' h => ?_⟩
    rw [uniq q' h]; omega

/-! ## `update_supply_index` -/

/-- `quotient_is_negative` is a pure test and always returns. -/
theorem quotient_is_negative_returns (x y d : Std.I128) :
    math.fp_core.quotient_is_negative x y d ⦃ _ => True ⦄ := by
  unfold math.fp_core.quotient_is_negative
  split_ifs <;> simp

/-- `mul_div_floor_saturating` returns (does not abort) for every nonzero
divisor: the native path never aborts (`div_floor_i128_spec`), and the widened
path saturates instead of trapping. (Its value is not needed here: the supply
index clamps it.) -/
theorem mul_div_floor_saturating_returns (env : Env) (x y d : Std.I128) (hd : d.val ≠ 0) :
    math.fp_core.mul_div_floor_saturating env x y d ⦃ _ => True ⦄ := by
  have hbounds := i128_prod_bounds x y
  have hd' : ¬ (d = 0#i128) := fun h => hd (by simp [h])
  have wide : (do
      let (x256, y256, d256) ← math.fp_core.to_i256_operands env x y d
      let i ← aeneas_pool_shim.I256.mul x256 y256
      let b ← math.fp_core.quotient_is_nonnegative x y d
      let i1 ← math.fp_core.div_floor_i256 env i d256 b
      let o2 ← aeneas_pool_shim.I256.to_i128 i1
      let b1 ← math.fp_core.quotient_is_negative x y d
      let i2 ← if b1 then ok core.num.I128.MIN else ok core.num.I128.MAX
      ok (core.option.Option.unwrap_or o2 i2)) ⦃ _ => True ⦄ := by
    unfold math.fp_core.to_i256_operands
    step as ⟨a, ha⟩
    step as ⟨b, hb⟩
    step as ⟨c, hc⟩
    step as ⟨i, hi⟩
    · simp only [aeneas_pool_shim.I256.InRange, ha, hb,
        show aeneas_pool_shim.I256.MIN = -2^255 from rfl,
        show aeneas_pool_shim.I256.MAX = 2^255 - 1 from rfl]; omega
    rw [ha, hb] at hi
    rw [hc] at *
    step with quotient_is_nonnegative_spec x y d hd as ⟨nn, hnn⟩
    step with div_floor_i256_spec env i d.val hd (by
        rw [hi, show aeneas_pool_shim.I256.MIN = -2^255 from rfl,
          show aeneas_pool_shim.I256.MAX = 2^255 - 1 from rfl]; omega)
        nn (by rw [hi]; exact hnn) as ⟨f, hf⟩
    step as ⟨o, ho⟩
    step with quotient_is_negative_returns x y d as ⟨b1, _⟩
    split <;> simp
  unfold math.fp_core.mul_div_floor_saturating math.fp_core.require_nonzero_divisor
  simp only [massert, hd', not_false_eq_true, ↓reduceIte, bind_ok, lift]
  cases hc : I128.checked_mul x y with
  | none =>
    simp only [core.option.Option.and_then, bind_ok]
    exact wide
  | some m =>
    simp only [core.option.Option.and_then,
      math.fp_core.mul_div_floor_saturating.closure.Insts.CoreOpsFunctionFnOnceTupleI128OptionI128.call_once]
    step with div_floor_i128_spec m d hd as ⟨o1, _⟩
    cases o1 with
    | none => exact wide
    | some q => simp

/-- `Ray::div_floor_saturating` (the #224 typed wrapper over
`mul_div_floor_saturating`) never aborts for a nonzero divisor. Its exact
clamped-floor value is `ray_div_floor_saturating_spec` in
`FloorCeilScaling.lean`; only non-abort is needed here. -/
theorem ray_div_floor_saturating_returns (env : Env) (a b : math.fp.Ray) (hb : rv b ≠ 0) :
    math.fp.Ray.div_floor_saturating a env b ⦃ _ => True ⦄ := by
  unfold math.fp.Ray.div_floor_saturating
  step with mul_div_floor_saturating_returns env a constants.shared.RAY b hb
    as ⟨q, _⟩

/-- The derived `Ord::min` on `Ray` is the minimum of the raw values. (Same
statement as `ray_min_spec` in `FloorCeilResolve.lean`, which this file does not
import.) -/
theorem ray_ord_min_spec (x y : math.fp.Ray) :
    core.cmp.Ord.min.trait_default math.fp.Ray.Insts.CoreCmpOrd x y ⦃ r =>
      rv r = min (rv x) (rv y) ⦄ := by
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default, core.cmp.Ord.min_body,
    math.fp.Ray.Insts.CoreCmpOrd, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp,
    math.fp.Ray.Insts.CoreCmpOrd.cmp, bind_ok]
  by_cases h : rv y < rv x
  · simp [h, compare_lt_iff_lt]; omega
  · simp [h, compare_lt_iff_lt]; omega

/-- The derived `Ord::max` on `Ray` is the maximum of the raw values. -/
theorem ray_ord_max_spec (x y : math.fp.Ray) :
    core.cmp.Ord.max.trait_default math.fp.Ray.Insts.CoreCmpOrd x y ⦃ r =>
      rv r = max (rv x) (rv y) ⦄ := by
  simp only [core.cmp.Ord.max.trait_default, core.cmp.Ord.max.default, core.cmp.Ord.max_body,
    math.fp.Ray.Insts.CoreCmpOrd, math.fp.Ray.Insts.CoreCmpPartialOrdRay,
    core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, math.fp.Ray.Insts.CoreCmpPartialOrdRay.partial_cmp,
    math.fp.Ray.Insts.CoreCmpOrd.cmp, bind_ok]
  by_cases h : rv y < rv x
  · simp [h, compare_lt_iff_lt]; omega
  · simp [h, compare_lt_iff_lt]; omega

/-- **`update_supply_index` never decreases below the capped old index and
never exceeds the cap (INV-IDX-02).** For `supplied ≥ 0`, `old ≥ 0`,
`rewards ≥ 0` (pool totals and the non-negative supplier rewards of
`calculate_supplier_rewards`), whenever the supplied value
`half_up(supplied * old / RAY)` plus `rewards` fits `i128`, the call never
aborts and returns either `old` unchanged (no supply, no rewards, or zero
supplied value) or a value in `[min(old, MAX_SUPPLY_INDEX_RAY),
MAX_SUPPLY_INDEX_RAY]`. -/
theorem update_supply_index_spec (env : Env) (s old rw : math.fp.Ray)
    (hs : 0 ≤ rv s) (ho : 0 ≤ rv old) (hr : 0 ≤ rv rw)
    (hfit : ∀ t, IsHalfUp (rv s * rv old) (10^27) t → t + rv rw < 2^127) :
    rates.index.update_supply_index env s old rw ⦃ n =>
      rv n = rv old ∨ (min (rv old) (10^36) ≤ rv n ∧ rv n ≤ 10^36) ⦄ := by
  unfold rates.index.update_supply_index
  simp only [ray_eq_eq, bind_ok, ray_zero_val_hu]
  by_cases h1 : rv s = 0
  · simp [h1]
  simp only [h1, decide_false, Bool.false_eq_true, ↓reduceIte]
  by_cases h2 : rv rw = 0
  · simp [h2]
  simp only [h2, decide_false, Bool.false_eq_true, ↓reduceIte]
  obtain ⟨t, ht⟩ := IsHalfUp.exists (rv s * rv old) (10^27) (by norm_num)
  have ht0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hs ho) ht
  have htf := hfit t ht
  step with ray_mul_spec env s old hs ho ⟨t, ht, by omega⟩ as ⟨v, hv⟩
  have hvt : rv v = t := IsHalfUp.unique (by norm_num) hv ht
  by_cases h3 : rv v = 0
  · simp [h3]
  simp only [h3, decide_false, Bool.false_eq_true, ↓reduceIte]
  step with ray_checked_add_spec env v rw (by omega) as ⟨nv, hnv⟩
  step with ray_div_floor_saturating_returns env nv s h1 as ⟨g, _⟩
  simp only [math.fp.Ray.from, core.convert.IntoFrom, core.convert.IntoFrom.into,
    core.convert.FromSame, core.convert.FromSame.from, bind_ok]
  step with ray_ord_min_spec g constants.pool.MAX_SUPPLY_INDEX_RAY as ⟨m1, hm1⟩
  step with ray_ord_min_spec old constants.pool.MAX_SUPPLY_INDEX_RAY as ⟨m2, hm2⟩
  apply WP.spec_mono (ray_ord_max_spec m1 m2)
  intro n hn
  rw [max_supply_index_val] at hm1 hm2
  right
  rw [hn, hm1, hm2]
  constructor
  · exact le_max_right _ _
  · exact max_le (min_le_right _ _) (min_le_right _ _)

/-- **INV-IDX-02 in its production form.** With an old index in
`[0, MAX_SUPPLY_INDEX_RAY]` (initialised at `RAY`, only written by this
function), the new supply index lies in `[old, MAX_SUPPLY_INDEX_RAY]`: it
never decreases and never exceeds the cap. Same hypotheses as
`update_supply_index_spec`. -/
theorem update_supply_index_monotone (env : Env) (s old rw : math.fp.Ray)
    (hs : 0 ≤ rv s) (ho : 0 ≤ rv old) (hoM : rv old ≤ 10^36) (hr : 0 ≤ rv rw)
    (hfit : ∀ t, IsHalfUp (rv s * rv old) (10^27) t → t + rv rw < 2^127) :
    rates.index.update_supply_index env s old rw ⦃ n =>
      rv old ≤ rv n ∧ rv n ≤ 10^36 ⦄ := by
  apply WP.spec_mono (update_supply_index_spec env s old rw hs ho hr hfit)
  rintro n (h | ⟨h1, h2⟩)
  · omega
  · rw [min_eq_left hoM] at h1; exact ⟨h1, h2⟩

/-! ## `calculate_supplier_rewards` -/

/-- **The accrued interest splits exactly into supplier rewards and protocol
fee.** For `borrowed ≥ 0`, `0 ≤ old_index ≤ new_index` (indexes never
decrease, `update_borrow_index_spec`), a reserve factor in `[0, BPS]`
(`InterestRateModel::verify` enforces `< BPS`), and a new total debt
`half_up(borrowed * new_index / RAY)` that fits `i128`, the call never aborts
and returns `(rewards, fee)` with `rewards + fee = accrued`, both `≥ 0`, where
`accrued = half_up(borrowed*new/RAY) - half_up(borrowed*old/RAY)` and
`fee = half_up(accrued * reserve_factor / BPS)`. -/
theorem calculate_supplier_rewards_spec (env : Env) (p : types.pool.MarketParams)
    (b ni oi : math.fp.Ray) (hb : 0 ≤ rv b) (ho : 0 ≤ rv oi) (hon : rv oi ≤ rv ni)
    (hrf : 0 ≤ rv p.reserve_factor ∧ rv p.reserve_factor ≤ 10000)
    (hfit : ∀ t, IsHalfUp (rv b * rv ni) (10^27) t → t < 2^127) :
    rates.index.calculate_supplier_rewards env p b ni oi ⦃ sr fee =>
      0 ≤ rv sr ∧ 0 ≤ rv fee ∧
      ∃ od nd, IsHalfUp (rv b * rv oi) (10^27) od ∧ IsHalfUp (rv b * rv ni) (10^27) nd ∧
        rv sr + rv fee = nd - od ∧ IsHalfUp ((nd - od) * rv p.reserve_factor) 10000 (rv fee) ⦄ := by
  unfold rates.index.calculate_supplier_rewards
  obtain ⟨nd, hnd⟩ := IsHalfUp.exists (rv b * rv ni) (10^27) (by norm_num)
  obtain ⟨od, hod⟩ := IsHalfUp.exists (rv b * rv oi) (10^27) (by norm_num)
  have hle : od ≤ nd := IsHalfUp.mono (by norm_num) (Int.mul_le_mul_of_nonneg_left hon hb) hod hnd
  have hod0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hb ho) hod
  have hndf := hfit nd hnd
  step with ray_mul_spec env b oi hb ho ⟨od, hod, by omega⟩ as ⟨o, hoo⟩
  have hov : rv o = od := IsHalfUp.unique (by norm_num) hoo hod
  step with ray_mul_spec env b ni hb (by omega) ⟨nd, hnd, hndf⟩ as ⟨n, hn⟩
  have hnv : rv n = nd := IsHalfUp.unique (by norm_num) hn hnd
  step with ray_checked_sub_spec env n o (by omega) (by omega) as ⟨a, ha⟩
  step with bps_apply_to_ray_spec env p.reserve_factor a hrf (by omega) as ⟨f, hf, hf0, hfa⟩
  step with ray_checked_sub_spec env a f hf0 hfa as ⟨r, hr⟩
  refine ⟨by omega, hf0, od, nd, hod, hnd, by omega, ?_⟩
  rw [← hov, ← hnv, ← ha]; exact hf

/-! ## `supply_index_reward_shortfall` -/

/-- `Ray::checked_sub` returning means its preconditions held. -/
theorem ray_checked_sub_returns (env : Env) (a b : math.fp.Ray)
    (h : math.fp.Ray.checked_sub a env b ⦃ _ => True ⦄) : 0 ≤ rv b ∧ rv b ≤ rv a := by
  by_contra hc
  exact ray_checked_sub_aborts env a b (by omega) h

/-- **The reward shortfall is in `[0, rewards]`.** Whenever
`supply_index_reward_shortfall` returns (it aborts by design when the index
move distributes less than nothing or more than `rewards_increase`), its result
`r` satisfies `0 ≤ r ≤ rewards_increase`, and
`r = rewards_increase - (half_up(supplied*new/RAY) - half_up(supplied*old/RAY))`
as computed. No hypothesis: this is a partial-correctness statement, the abort
cases are the intended panics. -/
theorem supply_index_reward_shortfall_spec (env : Env) (s oi ni rw : math.fp.Ray)
    (hret : rates.index.supply_index_reward_shortfall env s oi ni rw ⦃ _ => True ⦄) :
    rates.index.supply_index_reward_shortfall env s oi ni rw ⦃ r =>
      0 ≤ rv r ∧ rv r ≤ rv rw ⦄ := by
  unfold rates.index.supply_index_reward_shortfall at hret ⊢
  obtain ⟨r1, h1, hk1⟩ := spec_imp_eq_ok (spec_bind_inv hret)
  rw [h1]
  simp only [bind_ok]
  obtain ⟨r2, h2, hk2⟩ := spec_imp_eq_ok (spec_bind_inv hk1)
  rw [h2]
  simp only [bind_ok]
  obtain ⟨d, h3, hk3⟩ := spec_imp_eq_ok (spec_bind_inv hk2)
  have hd := ray_checked_sub_returns env r1 r2 (by rw [h3]; simp)
  have hdv := ray_checked_sub_spec env r1 r2 hd.1 hd.2
  rw [h3] at hdv ⊢
  simp only [WP.spec_ok, bind_ok] at hdv ⊢
  have he := ray_checked_sub_returns env rw d hk3
  apply WP.spec_mono (ray_checked_sub_spec env rw d he.1 he.2)
  intro r hr
  omega

end PoolAeneas.RatesProofs
