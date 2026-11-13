/-
Solvency, utilization and liquidity guards of `contracts/pool/src/guards.rs`.

Each guard is an assertion: it returns `()` (passes) or aborts. The theorems
give the exact pass condition, stated without division, in terms of the
conservative values the code computes:

* `B = ⌈borrowed * borrow_index / RAY⌉`   ceiled debt value (RAY units)
* `S = ⌊supplied * supply_index / RAY⌋`   floored supply value (RAY units)

so debt is rounded up and supply down (ADR-0003).
-/
import PoolAeneas.Proofs.Shares
import PoolAeneas.Proofs.Wrappers

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-! ## `require_supply_for_debt` (INV-ACCT-09) -/

/-- **`require_supply_for_debt` passes exactly when it should (INV-ACCT-09).**
It returns iff it is not the case that total supply shares are zero while
debt shares are not. No hypotheses. -/
theorem require_supply_for_debt_spec (env : Env) (c : market_cache.Cache) :
    (∃ u, guards.require_supply_for_debt env c = ok u) ↔
      ¬ (c.supplied.val = 0 ∧ c.borrowed.val ≠ 0) := by
  unfold guards.require_supply_for_debt
  simp only [market_cache.Cache.impl.supplied, market_cache.Cache.impl.borrowed, bind_ok,
    ray_eq_eq, ray_ne_eq, zero_val]
  by_cases hs : c.supplied.val = 0 <;> by_cases hb : c.borrowed.val = 0 <;>
    simp [hs, hb, massert]

/-! ## `require_utilization_below_max` (INV-ACCT-08) -/

/-- The pass condition of `require_utilization_below_max`: no supply shares,
no debt value, or `B ≤ ⌊S * max_utilization / RAY⌋`, i.e.
`B * RAY ≤ max_utilization * S`. There is no exemption for a 100% ceiling:
at `max_utilization = RAY` the condition is `B ≤ S`, debt value at most
supply value. -/
def UtilPass (c : market_cache.Cache) (B S : Int) : Prop :=
  c.supplied.val = 0 ∨ B = 0 ∨ B * 10^27 ≤ c.params.max_utilization.val * S

/-- **`require_utilization_below_max` passes exactly on `UtilPass`
(INV-ACCT-08), at every ceiling including 100%, and divides nothing.** For the
ceiled debt value `B` and floored supply value `S` the code computes, the
guard returns iff `UtilPass c B S`; otherwise it aborts. The code compares `B`
with `⌊S * max_utilization / RAY⌋`: debt is rounded up, supply down and the
allowed side down, so rounding can never admit utilization above the ceiling,
and no quotient can overflow. Debt against a zero floored supply value fails.
This is the guard of the debt-creating legs (`mint_debt`): audit finding S-03.

Hypotheses: share totals and indexes are non-negative (INV-ACCT-01, indexes
start at RAY and stay above `SUPPLY_INDEX_FLOOR_RAW`); `0 ≤ max_utilization ≤
RAY` (`InterestRateModel::verify`); `B` and `S` fit `i128` (otherwise
`mul_ceil`/`mul_floor` abort with `MathOverflow`, which is also a rejection). -/
theorem require_utilization_below_max_spec (env : Env) (c : market_cache.Cache) (B S : Int)
    (hb : 0 ≤ c.borrowed.val) (hbi : 0 ≤ c.borrow_index.val)
    (hs : 0 ≤ c.supplied.val) (hsi : 0 ≤ c.supply_index.val)
    (hm : 0 ≤ c.params.max_utilization.val) (hmR : c.params.max_utilization.val ≤ 10^27)
    (hB : IsCeil (c.borrowed.val * c.borrow_index.val) (10^27) B) (hBfit : B < 2^127)
    (hS : IsFloor (c.supplied.val * c.supply_index.val) (10^27) S) (hSfit : S < 2^127) :
    (∃ u, guards.require_utilization_below_max env c = ok u) ↔ UtilPass c B S := by
  have hB0 : 0 ≤ B := by
    unfold IsCeil at hB; rw [if_pos (by norm_num)] at hB; nlinarith
  have hS0 : 0 ≤ S := IsFloor.nonneg (by norm_num) (by positivity) hS
  unfold guards.require_utilization_below_max UtilPass
  simp only [market_cache.Cache.impl.supplied, market_cache.Cache.impl.borrowed,
    market_cache.Cache.impl.params, market_cache.Cache.impl.borrow_index,
    market_cache.Cache.impl.supply_index, bind_ok, ray_eq_eq, zero_val]
  by_cases h0 : c.supplied.val = 0
  · simp [h0]
  simp only [h0, decide_false, Bool.false_eq_true, ↓reduceIte, false_or]
  obtain ⟨b, hbeq, hbv⟩ := spec_imp_eq_ok (ray_mul_ceil_spec env c.borrowed c.borrow_index
    ⟨B, hB, by omega, hBfit⟩)
  have hbB : b.val = B := IsCeil.unique (by norm_num) hbv hB
  rw [hbeq]; simp only [bind_ok, ray_eq_eq, zero_val, hbB]
  by_cases hB0' : B = 0
  · simp [hB0']
  simp only [hB0', decide_false, Bool.false_eq_true, ↓reduceIte, false_or]
  obtain ⟨s, hseq, hsv⟩ := spec_imp_eq_ok (ray_mul_floor_spec env c.supplied c.supply_index
    ⟨S, hS, by omega, hSfit⟩)
  have hsS : s.val = S := IsFloor.unique (by norm_num) hsv hS
  rw [hseq]; simp only [bind_ok]
  -- the allowed debt value: `A = ⌊S * max_utilization / RAY⌋ ≤ S`
  have hA := isFloor_ediv (S * c.params.max_utilization.val) (10^27) (by norm_num)
  set A := S * c.params.max_utilization.val / 10^27 with hAdef
  have hA' := hA; unfold IsFloor at hA'; rw [if_pos (by norm_num)] at hA'
  have hA0 : 0 ≤ A := IsFloor.nonneg (by norm_num) (Int.mul_nonneg hS0 hm) hA
  have hAS : A ≤ S := by
    by_contra hc; have hc := not_le.mp hc
    have h1 : S * c.params.max_utilization.val ≤ S * 10^27 :=
      Int.mul_le_mul_of_nonneg_left hmR hS0
    have h2 : (S + 1) * 10^27 ≤ A * 10^27 := by nlinarith
    nlinarith
  obtain ⟨a, haeq, hav⟩ := spec_imp_eq_ok (ray_mul_floor_spec env s c.params.max_utilization
    ⟨A, by rw [hsS]; exact hA, by omega, by omega⟩)
  have haA : a.val = A := IsFloor.unique (by norm_num) hav (by rw [hsS]; exact hA)
  rw [haeq]; simp only [bind_ok, ray_le_eq, hbB, haA]
  have key : B ≤ A ↔ B * 10^27 ≤ c.params.max_utilization.val * S := by
    constructor
    · intro h; nlinarith
    · intro h
      by_contra hc; have hc := not_le.mp hc
      have : (A + 1) * 10^27 ≤ B * 10^27 := by nlinarith
      nlinarith
  by_cases hle : B ≤ A
  · simp only [hle, decide_true, massert, ↓reduceIte, key.mp hle, iff_true]
    exact ⟨(), by simp⟩
  · have hnk : ¬ B * 10^27 ≤ c.params.max_utilization.val * S := fun h => hle (key.mpr h)
    constructor
    · rintro ⟨u, hu⟩; simp [hle, massert] at hu
    · intro h; exact absurd h hnk

/-- **`require_exit_utilization_below_max` is the same gate, skipped at a 100%
ceiling (INV-ACCT-08).** It returns iff `max_utilization ≥ RAY` or `UtilPass c
B S`. Ordinary withdrawals and revenue claims call it: an exit cannot draw
more than cash (`require_reserves`), so at a 100% ceiling the comparison could
only fail on the rounding dust between cash plus debt and the supplier
claims, and would stop the withdrawal of the last cash. Below 100% it is
exactly `require_utilization_below_max`. Hypotheses as there. -/
theorem require_exit_utilization_below_max_spec (env : Env) (c : market_cache.Cache)
    (B S : Int)
    (hb : 0 ≤ c.borrowed.val) (hbi : 0 ≤ c.borrow_index.val)
    (hs : 0 ≤ c.supplied.val) (hsi : 0 ≤ c.supply_index.val)
    (hm : 0 ≤ c.params.max_utilization.val) (hmR : c.params.max_utilization.val ≤ 10^27)
    (hB : IsCeil (c.borrowed.val * c.borrow_index.val) (10^27) B) (hBfit : B < 2^127)
    (hS : IsFloor (c.supplied.val * c.supply_index.val) (10^27) S) (hSfit : S < 2^127) :
    (∃ u, guards.require_exit_utilization_below_max env c = ok u) ↔
      (10^27 ≤ c.params.max_utilization.val ∨ UtilPass c B S) := by
  unfold guards.require_exit_utilization_below_max
  simp only [market_cache.Cache.impl.params, bind_ok, ray_ge_eq, ray_one_val]
  by_cases hmax : 10^27 ≤ c.params.max_utilization.val
  · simp only [hmax, decide_true, ↓reduceIte, true_or, iff_true]; exact ⟨(), by simp⟩
  · simp only [hmax, decide_false, Bool.false_eq_true, ↓reduceIte, false_or]
    exact require_utilization_below_max_spec env c B S hb hbi hs hsi hm hmR hB hBfit hS hSfit

/-- Whatever the exit gate returns below a 100% ceiling, the full gate
returned too (same cache). -/
theorem require_exit_utilization_ok (env : Env) (c : market_cache.Cache) (u : Unit)
    (h : guards.require_exit_utilization_below_max env c = ok u) :
    10^27 ≤ c.params.max_utilization.val ∨
      ∃ u', guards.require_utilization_below_max env c = ok u' := by
  unfold guards.require_exit_utilization_below_max at h
  simp only [market_cache.Cache.impl.params, bind_ok, ray_ge_eq, ray_one_val] at h
  by_cases hmax : 10^27 ≤ c.params.max_utilization.val
  · exact Or.inl hmax
  · simp only [hmax, decide_false, Bool.false_eq_true, ↓reduceIte] at h
    exact Or.inr ⟨u, h⟩

/-! ## The market value and entry ceilings (INV-IDX-06, INV-ACCT-11) -/

/-- `MAX_MARKET_VALUE_RAY = i128::MAX − RAY`: the ceiling accrual holds a
market's total supply value and total debt value at. -/
def MaxMarketValue : Int := 2^127 - 1 - 10^27

/-- `MAX_MARKET_ENTRY_RAY = MAX_MARKET_VALUE_RAY / 8`: the ceiling entries
enforce on a market total, as a share count and as a value. -/
def MaxMarketEntry : Int := 21267647932433653966460912964485513215

/-- The entry ceiling is the value ceiling divided by the headroom factor 8,
rounded down. -/
theorem maxMarketEntry_eq : MaxMarketEntry = MaxMarketValue / 8 := by
  unfold MaxMarketEntry MaxMarketValue; norm_num

/-- `MAX_MARKET_VALUE_RAY` evaluates to `i128::MAX − RAY`. -/
theorem max_market_value_spec :
    constants.pool.MAX_MARKET_VALUE_RAY ⦃ v => v.val = MaxMarketValue ⦄ := by
  unfold constants.pool.MAX_MARKET_VALUE_RAY MaxMarketValue
  have hR : constants.shared.RAY = 1000000000000000000000000000#i128 := by
    unfold constants.shared.RAY; rfl
  have hM : (core.num.I128.MAX).val = 2^127 - 1 := by simp [core.num.I128.MAX, I128.rMax]
  rw [hR]
  step as ⟨v, hv⟩
  rw [hv]; simp [I128.rMax]

/-- `MAX_MARKET_ENTRY_RAY` evaluates to `(i128::MAX − RAY) / 8`. -/
theorem max_market_entry_spec :
    constants.pool.MAX_MARKET_ENTRY_RAY ⦃ v => v.val = MaxMarketEntry ⦄ := by
  unfold constants.pool.MAX_MARKET_ENTRY_RAY constants.pool.MARKET_ENTRY_HEADROOM
  step with max_market_value_spec as ⟨i, hi⟩
  step as ⟨v, hv⟩
  rw [hv, hi]; unfold MaxMarketValue MaxMarketEntry; norm_num

/-- A market total fits the entry ceiling: the share count is at most
`MAX_MARKET_ENTRY_RAY`, and so is its half-up value at the index (which must
therefore be representable). -/
def EntryFits (x i : Int) : Prop :=
  x ≤ MaxMarketEntry ∧ ∃ v, IsHalfUp (x * i) (10^27) v ∧ v ≤ MaxMarketEntry

/-- **`fits_entry_ceiling` never aborts and decides `EntryFits`** for
non-negative shares and index: `true` iff the share count and its half-up
value are both at most `MAX_MARKET_ENTRY_RAY`. A value that does not fit
`i128` gives `false` (`Ray::try_mul` returns `None`), not an abort. -/
theorem fits_entry_ceiling_spec (env : Env) (x i : math.fp.Ray)
    (hx : 0 ≤ x.val) (hi : 0 ≤ i.val) :
    guards.fits_entry_ceiling env x i ⦃ r => r = true ↔ EntryFits x.val i.val ⦄ := by
  unfold guards.fits_entry_ceiling EntryFits
  step with max_market_entry_spec as ⟨e, he⟩
  simp only [ray_from_same, bind_ok, ray_gt_eq, he]
  obtain ⟨v, hv⟩ := IsHalfUp.exists (x.val * i.val) (10^27) (by norm_num)
  have hv0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hx hi) hv
  by_cases hgt : MaxMarketEntry < x.val
  · simp only [hgt, decide_true, ↓reduceIte, WP.spec_ok]
    constructor
    · intro h; exact absurd h (by simp)
    · rintro ⟨h, _⟩; omega
  simp only [hgt, decide_false, Bool.false_eq_true, ↓reduceIte]
  step with ray_try_mul_spec env x i as ⟨o, _, ho⟩
  have hres := ho hx hi
  cases o with
  | none =>
    simp only [WP.spec_ok]
    simp only [HalfUpResult] at hres
    constructor
    · intro h; exact absurd h (by simp)
    · rintro ⟨_, w, hw, hwle⟩
      exact absurd (by unfold MaxMarketEntry at hwle; omega) (hres w hw)
  | some w =>
    simp only [HalfUpResult] at hres
    have hwv : w.val = v := IsHalfUp.unique (by norm_num) hres hv
    simp only [ray_le_eq, WP.spec_ok, he, hwv]
    constructor
    · intro h
      exact ⟨by omega, v, hv, by simpa using h⟩
    · rintro ⟨_, v', hv', hle⟩
      have : v' = v := IsHalfUp.unique (by norm_num) hv' hv
      subst this; simpa using hle

/-- **`require_within_entry_ceiling` passes exactly when both market totals
fit the entry ceiling (INV-ACCT-11).** It returns iff `EntryFits` holds for
total supply at the supply index and for total debt at the borrow index;
otherwise it aborts (`MarketCeilingReached`). Supply, borrow, strategy and
flash-loan entries call it after they raise a total, so an entry leaves both
totals at most `MAX_MARKET_ENTRY_RAY`, a factor 8 below the ceiling accrual
holds them at. Hypotheses: non-negative share totals and indexes. -/
theorem require_within_entry_ceiling_spec (env : Env) (c : market_cache.Cache)
    (hs : 0 ≤ c.supplied.val) (hsi : 0 ≤ c.supply_index.val)
    (hb : 0 ≤ c.borrowed.val) (hbi : 0 ≤ c.borrow_index.val) :
    (∃ u, guards.require_within_entry_ceiling env c = ok u) ↔
      (EntryFits c.supplied.val c.supply_index.val ∧
        EntryFits c.borrowed.val c.borrow_index.val) := by
  unfold guards.require_within_entry_ceiling
  simp only [market_cache.Cache.impl.supplied, market_cache.Cache.impl.supply_index,
    market_cache.Cache.impl.borrowed, market_cache.Cache.impl.borrow_index, bind_ok]
  obtain ⟨r1, hr1, h1⟩ := spec_imp_eq_ok
    (fits_entry_ceiling_spec env c.supplied c.supply_index hs hsi)
  obtain ⟨r2, hr2, h2⟩ := spec_imp_eq_ok
    (fits_entry_ceiling_spec env c.borrowed c.borrow_index hb hbi)
  rw [hr1]; simp only [bind_ok, massert]
  by_cases hf1 : r1 = true
  · simp only [hf1, ↓reduceIte, bind_ok, hr2]
    by_cases hf2 : r2 = true
    · simp only [hf2, ↓reduceIte]
      exact ⟨fun _ => ⟨h1.mp hf1, h2.mp hf2⟩, fun _ => ⟨(), by simp⟩⟩
    · simp only [hf2, Bool.false_eq_true, ↓reduceIte]
      constructor
      · rintro ⟨u, hu⟩; exact absurd hu (by simp)
      · rintro ⟨_, h⟩; exact absurd (h2.mpr h) hf2
  · simp only [hf1, Bool.false_eq_true, ↓reduceIte]
    constructor
    · rintro ⟨u, hu⟩; simp at hu
    · rintro ⟨h, _⟩; exact absurd (h1.mpr h) hf1

/-- The entry-ceiling guard reads only the share totals and the indexes: two
caches that agree on them get the same answer (so a guard that passed before
a cash movement also holds for the committed cache). -/
theorem require_within_entry_ceiling_congr (env : Env) (c c' : market_cache.Cache)
    (h1 : c'.supplied = c.supplied) (h2 : c'.supply_index = c.supply_index)
    (h3 : c'.borrowed = c.borrowed) (h4 : c'.borrow_index = c.borrow_index) :
    guards.require_within_entry_ceiling env c' = guards.require_within_entry_ceiling env c := by
  unfold guards.require_within_entry_ceiling
  simp only [market_cache.Cache.impl.supplied, market_cache.Cache.impl.supply_index,
    market_cache.Cache.impl.borrowed, market_cache.Cache.impl.borrow_index, h1, h2, h3, h4]

/-! ## `require_liquidation_buffer` (INV-ACCT-07) -/

/-- `LIQUIDATION_BUFFER_BPS` is 200. -/
theorem liq_buffer_val : (constants.pool.LIQUIDATION_BUFFER_BPS).val = 200 := by
  unfold constants.pool.LIQUIDATION_BUFFER_BPS; simp

/-- **`require_liquidation_buffer` passes exactly when the draw leaves a 2%
buffer (INV-ACCT-07).** With `Sa = ⌊supplied * supply_index /
(RAY * 10^(27-dec))⌋` the floored supplied token value and `R = ⌈Sa * 200 /
10000⌉` the reserved cash (rounded up), the guard returns iff
`cash - draw ≥ R`, equivalently `(cash - draw) * 10000 ≥ Sa * 200`.

Hypotheses: `cash ≥ 0` (INV-ACCT-02) and `draw ≥ 0` (borrow amounts are
validated non-negative), so `cash - draw` does not saturate; non-negative
supply shares and index; `decimals ≤ 27`; the supply RAY value fits `i128`
(`mul_floor` aborts otherwise). -/
theorem require_liquidation_buffer_spec (env : Env) (c : market_cache.Cache) (draw : Std.I128)
    (hcash : 0 ≤ c.cash.val) (hdraw : 0 ≤ draw.val)
    (hs : 0 ≤ c.supplied.val) (hsi : 0 ≤ c.supply_index.val)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hfit : c.supplied.val * c.supply_index.val < 2^127 * 10^27) :
    ∃ Sa R : Int, IsFloor (c.supplied.val * c.supply_index.val)
        (10^27 * 10^(27 - c.params.asset_decimals.val)) Sa ∧
      IsCeil (Sa * 200) 10000 R ∧
      ((∃ u, guards.require_liquidation_buffer env c draw = ok u) ↔ R ≤ c.cash.val - draw.val) ∧
      (R ≤ c.cash.val - draw.val ↔ Sa * 200 ≤ (c.cash.val - draw.val) * 10000) := by
  have hnn : 0 ≤ c.supplied.val * c.supply_index.val := by positivity
  obtain ⟨sa, hsaeq, hsa⟩ := spec_imp_eq_ok (unscale_supply_floor_spec env c.supplied
    c.supply_index c.params.asset_decimals hdec hnn (floor_fits _ _ (by norm_num) hnn hfit))
  have hD : (0 : Int) < 10^27 * 10^(27 - c.params.asset_decimals.val) := by positivity
  have hsa0 : 0 ≤ sa.val := IsFloor.nonneg hD hnn hsa
  have hsale : sa.val ≤ c.supplied.val * c.supply_index.val := by
    unfold IsFloor at hsa; rw [if_pos hD] at hsa
    have : (1 : Int) ≤ 10^27 * 10^(27 - c.params.asset_decimals.val) := by omega
    nlinarith
  have hsab := i128_bounds sa
  obtain ⟨R, hR⟩ := isCeil_exists (sa.val * 200) 10000 (by norm_num)
  have hR' := hR; unfold IsCeil at hR'; rw [if_pos (by norm_num)] at hR'
  have hR0 : 0 ≤ R := by nlinarith
  have hRle : R ≤ sa.val := by nlinarith
  refine ⟨sa.val, R, hsa, hR, ?_, ?_⟩
  · unfold guards.require_liquidation_buffer market_cache.scale.Cache.unscale_supply_floor
    simp only [market_cache.Cache.impl.supplied, market_cache.Cache.impl.cash, bind_ok, hsaeq,
      bps_from_same]
    obtain ⟨r, hreq, hrv⟩ := spec_imp_eq_ok (bps_apply_to_ceil_spec env
      constants.pool.LIQUIDATION_BUFFER_BPS sa
      ⟨R, by rw [liq_buffer_val]; exact hR, by omega, by omega⟩)
    rw [liq_buffer_val] at hrv
    have hrR : r.val = R := IsCeil.unique (by norm_num) hrv hR
    have hcb := i128_bounds c.cash
    have hdb := i128_bounds draw
    rw [hreq]
    simp only [bind_ok, lift, IScalar.le_equiv, ge_iff_le]
    rw [sat_sub_val c.cash draw (by omega) (by omega), hrR]
    by_cases hle : R ≤ c.cash.val - draw.val <;> simp [hle, massert]
  · constructor
    · intro h; nlinarith
    · intro h; by_contra hc; have hc := not_le.mp hc; nlinarith

/-! ## `backing_shortfall` and `require_backed_market` (INV-ACCT-04) -/

/-- **`backing_shortfall` is the conservative shortfall (INV-ACCT-04).** With
`C = ⌊supplied * supply_index / RAY⌋` (floored claims) and `Dt = ⌈borrowed *
borrow_index / RAY⌉` (ceiled debt): if `C ≤ Dt` the shortfall is 0; otherwise
it is `max(0, U - cash)` with `U = ⌊(C - Dt) / 10^(27-dec)⌋`, the uncovered
claim floored to token units once. Never aborts under the hypotheses, and the
shortfall is never over-reported: `shortfall * 10^(27-dec) ≤ C - Dt -
cash * 10^(27-dec)` whenever it is positive.

Hypotheses: non-negative share totals and indexes (INV-ACCT-01), `cash ≥ 0`
(INV-ACCT-02), `decimals ≤ 27`, and the claim and debt RAY values fit
`i128` (otherwise `mul_floor`/`mul_ceil` abort). -/
theorem backing_shortfall_spec (c : market_cache.Cache) (C Dt : Int)
    (hb : 0 ≤ c.borrowed.val) (hbi : 0 ≤ c.borrow_index.val)
    (hs : 0 ≤ c.supplied.val) (hsi : 0 ≤ c.supply_index.val) (hcash : 0 ≤ c.cash.val)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hC : IsFloor (c.supplied.val * c.supply_index.val) (10^27) C) (hCfit : C < 2^127)
    (hD : IsCeil (c.borrowed.val * c.borrow_index.val) (10^27) Dt) (hDfit : Dt < 2^127) :
    guards.backing_shortfall c ⦃ r =>
      (C ≤ Dt → r.val = 0) ∧
      (Dt < C → ∃ U, IsFloor (C - Dt) (10^(27 - c.params.asset_decimals.val)) U ∧
        r.val = max 0 (U - c.cash.val)) ∧
      (0 < r.val → r.val * 10^(27 - c.params.asset_decimals.val) ≤
        C - Dt - c.cash.val * 10^(27 - c.params.asset_decimals.val)) ⦄ := by
  have hC0 : 0 ≤ C := IsFloor.nonneg (by norm_num) (by positivity) hC
  have hD0 : 0 ≤ Dt := by
    unfold IsCeil at hD; rw [if_pos (by norm_num)] at hD; nlinarith
  unfold guards.backing_shortfall market_cache.scale.Cache.unscale_borrow_ceil_ray
  simp only [market_cache.Cache.impl.env, market_cache.Cache.impl.supplied,
    market_cache.Cache.impl.supply_index, market_cache.Cache.impl.borrowed,
    market_cache.Cache.impl.params, market_cache.Cache.impl.cash, bind_ok]
  step with ray_mul_floor_spec c.env c.supplied c.supply_index ⟨C, hC, by omega, hCfit⟩
    as ⟨cl, hcl⟩
  have hclC : cl.val = C := IsFloor.unique (by norm_num) hcl hC
  step with ray_mul_ceil_spec c.env c.borrowed c.borrow_index ⟨Dt, hD, by omega, hDfit⟩
    as ⟨dt, hdt⟩
  have hdtD : dt.val = Dt := IsCeil.unique (by norm_num) hdt hD
  simp only [ray_le_eq, bind_ok, hclC, hdtD]
  by_cases hle : C ≤ Dt
  · simp only [hle, decide_true, ↓reduceIte, WP.spec_ok]
    have h0 : ((0#i128 : Std.I128).val) = 0 := by simp
    refine ⟨fun _ => h0, fun h => by omega, fun h => by omega⟩
  simp only [hle, decide_false, Bool.false_eq_true, ↓reduceIte]
  step with ray_checked_sub_spec c.env cl dt (by omega) (by omega) as ⟨d, hd⟩
  step with ray_to_asset_floor_spec c.env d c.params.asset_decimals hdec as ⟨u, hu, _⟩
  have hd0 : 0 ≤ d.val := by omega
  have hU := hu hd0
  rw [hd, hclC, hdtD] at hU
  have hk : (0 : Int) < 10^(27 - c.params.asset_decimals.val) := by positivity
  have hu0 : 0 ≤ u.val := IsFloor.nonneg hk (by omega) hU
  have hub := i128_bounds u
  have hcb := i128_bounds c.cash
  simp only [lift, bind_ok, WP.spec_ok]
  rw [max_i128_val, sat_sub_val u c.cash (by omega) (by omega)]
  have h0 : ((0#i128 : Std.I128).val) = 0 := by simp
  rw [h0]
  refine ⟨fun h => h.elim, fun _ => ⟨u.val, hU, by rw [max_comm]⟩, fun hpos => ?_⟩
  have hpos' : 0 < u.val - c.cash.val := by
    simp only [max_def] at hpos; split at hpos <;> omega
  rw [max_eq_left (le_of_lt hpos')]
  unfold IsFloor at hU; rw [if_pos hk] at hU
  nlinarith

/-- **`require_backed_market` passes exactly when there is no backing
shortfall (INV-ACCT-04).** Under the hypotheses of `backing_shortfall_spec`,
it returns iff `C ≤ Dt`, or the uncovered claim floored to token units is at
most `cash`: `C - Dt < (cash + 1) * 10^(27-dec)`. -/
theorem require_backed_market_spec (env : Env) (c : market_cache.Cache) (C Dt : Int)
    (hb : 0 ≤ c.borrowed.val) (hbi : 0 ≤ c.borrow_index.val)
    (hs : 0 ≤ c.supplied.val) (hsi : 0 ≤ c.supply_index.val) (hcash : 0 ≤ c.cash.val)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hC : IsFloor (c.supplied.val * c.supply_index.val) (10^27) C) (hCfit : C < 2^127)
    (hD : IsCeil (c.borrowed.val * c.borrow_index.val) (10^27) Dt) (hDfit : Dt < 2^127) :
    (∃ u, guards.require_backed_market env c = ok u) ↔
      (C ≤ Dt ∨ C - Dt < (c.cash.val + 1) * 10^(27 - c.params.asset_decimals.val)) := by
  obtain ⟨r, hreq, hr1, hr2, _⟩ := spec_imp_eq_ok
    (backing_shortfall_spec c C Dt hb hbi hs hsi hcash hdec hC hCfit hD hDfit)
  unfold guards.require_backed_market
  rw [hreq]
  simp only [bind_ok, massert]
  have hk : (0 : Int) < 10^(27 - c.params.asset_decimals.val) := by positivity
  have hr0 : (r = 0#i128) ↔ r.val = 0 := by
    constructor
    · intro h; rw [h]; simp
    · intro h; apply IScalar.eq_of_val_eq; simp [h]
  by_cases hle : C ≤ Dt
  · have := hr1 hle
    simp [hr0.mpr this, hle]
  · obtain ⟨U, hU, hrv⟩ := hr2 (by omega)
    unfold IsFloor at hU; rw [if_pos hk] at hU
    have key : r.val = 0 ↔ C - Dt < (c.cash.val + 1) * 10^(27 - c.params.asset_decimals.val) := by
      rw [hrv]
      constructor
      · intro h
        have : U ≤ c.cash.val := by
          rcases le_total (U - c.cash.val) 0 with h' | h'
          · omega
          · rw [max_eq_right h'] at h; omega
        nlinarith
      · intro h
        have : U ≤ c.cash.val := by
          by_contra hc; have hc := not_le.mp hc
          have : (c.cash.val + 1) * 10^(27 - c.params.asset_decimals.val) ≤
              U * 10^(27 - c.params.asset_decimals.val) := by nlinarith
          omega
        omega
    by_cases hz : r.val = 0
    · simp only [hr0.mpr hz, ↓reduceIte, hle, false_or]
      exact ⟨fun _ => key.mp hz, fun _ => ⟨(), by simp⟩⟩
    · have : ¬ (r = 0#i128) := fun h => hz (hr0.mp h)
      simp only [this, ↓reduceIte, hle, false_or]
      constructor
      · rintro ⟨u, hu⟩; exact absurd hu (by simp)
      · intro h; exact absurd (key.mpr h) hz

end PoolAeneas.Proofs
