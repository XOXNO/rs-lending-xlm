/-
Interest accrual: `rates::simulate::accrue_step` (common/src/rates/simulate.rs),
`interest::accrue_chunk` and `interest::global_sync`
(contracts/pool/src/interest.rs). INV-IDX-01, INV-IDX-02, INV-IDX-04,
INV-IDX-05 (revenue value bound), INV-ACCT-01.

`accrue_step` composes the per-step functions, each specified in a copied
RatesAeneas file (`Rates*.lean`, namespace `PoolAeneas.RatesProofs`) or here:
`market_value` (`Ray::try_mul`), `value_room`, `utilization`,
`calculate_borrow_rate`, `compound_interest` (`Compound.lean`),
`update_borrow_index`, `cap_borrow_index`, `calculate_supplier_rewards`,
`update_supply_index` (strengthened here: the new index distributes at most
the rewards), `supply_index_reward_shortfall` (total correctness here),
`protocol_fee_shares`.

The step holds interest at `MAX_MARKET_VALUE_RAY` (INV-IDX-06), so
`accrue_step_spec` needs no hypothesis that a market value fits `i128`: it
never aborts for any non-negative share totals (audit finding S-04).
-/
import PoolAeneas.Proofs.Guards
import PoolAeneas.Proofs.Interest
import PoolAeneas.Proofs.Compound
import PoolAeneas.Proofs.RatesCurveRates
import PoolAeneas.Proofs.RatesIndex

/- Plain (non-`module`) file, like `Wrappers.lean`. -/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-! ## Helper facts -/

/-- Half-up rounding of `p·f/RAY` with `0 ≤ p` and `0 ≤ f ≤ K·RAY` is at most
`K·p` (the rounding excess is below one unit). -/
theorem hu_scale_le {p f q : Int} (K : Int) (hp : 0 ≤ p) (hf : f ≤ K * 10^27)
    (h : IsHalfUp (p * f) (10^27) q) : q ≤ K * p := by
  have h1 := h.mul_le
  have h2 : p * f ≤ p * (K * 10^27) := Int.mul_le_mul_of_nonneg_left hf hp
  by_contra hc
  have : (K * p + 1) * 10^27 ≤ q * 10^27 := by nlinarith
  nlinarith

/-- Half-up rounding of `n/RAY` is at most `B` when `n ≤ B·RAY`. -/
theorem hu_le_of_le {n q B : Int} (hn : n ≤ B * 10^27) (h : IsHalfUp n (10^27) q) : q ≤ B := by
  have h1 := h.mul_le
  by_contra hc
  have : (B + 1) * 10^27 ≤ q * 10^27 := by nlinarith
  nlinarith

/-- Half-up rounding of `n/RAY` for `0 ≤ n < 2^125·RAY` is below `2^125 + 1`. -/
theorem hu_lt_of_lt {n q : Int} (hn : n < 2^125 * 10^27) (h : IsHalfUp n (10^27) q) :
    q ≤ 2^125 := hu_le_of_le (by omega) h

/-- Half-up rounding shifts with whole units: if `n' ≤ n + r·RAY` then
`half_up(n'/RAY) ≤ half_up(n/RAY) + r`. This is what turns a bound on a
product (`shares·index`) into the same bound on its rounded value. -/
theorem hu_add_le {n n' q q' r : Int} (h : n' ≤ n + r * 10^27)
    (hq : IsHalfUp n (10^27) q) (hq' : IsHalfUp n' (10^27) q') : q' ≤ q + r := by
  unfold IsHalfUp at hq hq'
  by_contra hc
  have h1 : (q + r + 1) * 10^27 ≤ q' * 10^27 := by nlinarith
  nlinarith

/-- `MAX_COMPOUND_DELTA_MS = MILLISECONDS_PER_YEAR = 31_556_926_000`. -/
theorem max_compound_delta_val :
    rates.compound.MAX_COMPOUND_DELTA_MS = 31556926000#u64 := by
  unfold rates.compound.MAX_COMPOUND_DELTA_MS constants.shared.MILLISECONDS_PER_YEAR; rfl

/-- **The production exponent of `compound_interest` is at most `3·RAY`.**
The per-millisecond rate is `r = half_up(a / 31_556_926_000)` with
`0 ≤ a ≤ max_borrow_rate ≤ 2·RAY` (`calculate_borrow_rate_spec`,
`MAX_BORROW_RATE_RAY`), and `delta_ms ≤ MAX_COMPOUND_DELTA_MS = 31_556_926_000`,
so `r·delta_ms ≤ a + 31_556_926_000/2 ≤ 2·RAY + 1.6·10^10`. -/
theorem compound_x_le {a r d : Int} (ha : 0 ≤ a) (haM : a ≤ 2 * 10^27) (hd0 : 0 ≤ d)
    (hd : d ≤ 31556926000) (h : IsHalfUp a 31556926000 r) :
    0 ≤ r ∧ r * d ≤ 3 * 10^27 := by
  have hr0 := IsHalfUp.nonneg (by norm_num) ha h
  have h1 := h.mul_le
  have h2 : r * d ≤ r * 31556926000 := Int.mul_le_mul_of_nonneg_left hd hr0
  exact ⟨hr0, by omega⟩

/-! ## `protocol_fee_shares` -/

/-- **`protocol_fee_shares` never aborts and is capped by the supply
headroom.** For `fee ≥ 0`, `supply_index > 0`, `supplied ≥ 0`, the result
`sh` satisfies `0 ≤ sh`, `supplied + sh ≤ i128::MAX` and
`sh·supply_index ≤ fee·RAY` (floor rounding: the shares are worth at most the
fee). -/
theorem protocol_fee_shares_spec (env : Env) (fee si s : math.fp.Ray)
    (hfee : 0 ≤ fee.val) (hsi : 0 < si.val) (hs : 0 ≤ s.val) :
    rates.index.protocol_fee_shares env fee si s ⦃ sh =>
      0 ≤ sh.val ∧ s.val + sh.val ≤ 2^127 - 1 ∧ sh.val * si.val ≤ fee.val * 10^27 ⦄ := by
  unfold rates.index.protocol_fee_shares
  step with ray_div_floor_saturating_spec env fee si (by omega) as ⟨raw, hraw⟩
  simp only [math.fp.Ray.raw, bind_ok]
  have hq := isFloor_ediv (fee.val * 10^27) si.val hsi
  have hq0 : 0 ≤ fee.val * 10^27 / si.val := IsFloor.nonneg hsi (by positivity) hq
  have hrv := hraw _ hq
  have hsb := i128_bounds s
  simp only [lift, ray_from_same, bind_ok]
  have hmax : (core.num.I128.MAX).val = 2^127 - 1 := by
    simp [core.num.I128.MAX, I128.rMax]
  have hhead := sat_sub_val core.num.I128.MAX s (by rw [hmax]; omega) (by rw [hmax]; omega)
  rw [hmax] at hhead
  simp only [WP.spec_ok]
  rw [min_i128_val, hhead, hrv]
  have hfl : fee.val * 10^27 / si.val * si.val ≤ fee.val * 10^27 :=
    Int.ediv_mul_le _ (by omega)
  refine ⟨by simp only [le_min_iff, le_max_iff]; omega, by omega, ?_⟩
  have hle : min (max (-2^127) (min (2^127 - 1) (fee.val * 10^27 / si.val)))
      (2^127 - 1 - s.val) ≤ fee.val * 10^27 / si.val := by
    simp only [min_le_iff, max_le_iff]; omega
  have hle0 : 0 ≤ min (max (-2^127) (min (2^127 - 1) (fee.val * 10^27 / si.val)))
      (2^127 - 1 - s.val) := by simp only [le_min_iff, le_max_iff]; omega
  calc _ ≤ fee.val * 10^27 / si.val * si.val := Int.mul_le_mul_of_nonneg_right hle (by omega)
    _ ≤ fee.val * 10^27 := hfl

/-! ## `update_supply_index`: the new index distributes at most the rewards -/

/-- **`update_supply_index`, strengthened (INV-IDX-02).** For `supplied ≥ 0`,
`0 < old ≤ MAX_SUPPLY_INDEX_RAY`, `rewards ≥ 0` and `V + rewards` fitting
`i128` (`V = half_up(supplied·old/RAY)`), the call never aborts and returns
`n` with `old ≤ n ≤ MAX_SUPPLY_INDEX_RAY`; moreover either `n = old` or
`supplied·n ≤ (V + rewards)·RAY`: the index move never distributes more
value than `rewards`. (This is what makes `supply_index_reward_shortfall`
non-aborting in `accrue_step`.) -/
theorem update_supply_index_strong (env : Env) (s old rw : math.fp.Ray)
    (hs : 0 ≤ s.val) (ho : 0 < old.val) (hoM : old.val ≤ 10^36) (hr : 0 ≤ rw.val)
    (hfit : ∀ t, IsHalfUp (s.val * old.val) (10^27) t → t + rw.val < 2^127) :
    rates.index.update_supply_index env s old rw ⦃ n =>
      old.val ≤ n.val ∧ n.val ≤ 10^36 ∧
      (n.val = old.val ∨ ∀ t, IsHalfUp (s.val * old.val) (10^27) t →
        s.val * n.val ≤ (t + rw.val) * 10^27) ⦄ := by
  unfold rates.index.update_supply_index
  simp only [ray_eq_eq, bind_ok, zero_val]
  by_cases h1 : s.val = 0
  · simp [h1]; omega
  simp only [h1, decide_false, Bool.false_eq_true, ↓reduceIte]
  by_cases h2 : rw.val = 0
  · simp [h2]; omega
  simp only [h2, decide_false, Bool.false_eq_true, ↓reduceIte]
  obtain ⟨t, ht⟩ := IsHalfUp.exists (s.val * old.val) (10^27) (by norm_num)
  have ht0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hs (by omega)) ht
  have htf := hfit t ht
  step with RatesProofs.ray_mul_spec env s old hs (by omega) ⟨t, ht, by omega⟩ as ⟨v, hv⟩
  have hvt : v.val = t := IsHalfUp.unique (by norm_num) hv ht
  by_cases h3 : v.val = 0
  · simp [h3]; omega
  simp only [h3, decide_false, Bool.false_eq_true, ↓reduceIte]
  step with ray_checked_add_spec env v rw (by omega) (by omega) as ⟨nv, hnv⟩
  step with ray_div_floor_saturating_spec env nv s h1 as ⟨g, hg⟩
  have hspos : 0 < s.val := by omega
  have hq := isFloor_ediv (nv.val * 10^27) s.val hspos
  have hgv := hg _ hq
  simp only [ray_from_same, bind_ok]
  step with RatesProofs.ray_ord_min_spec g constants.pool.MAX_SUPPLY_INDEX_RAY as ⟨m1, hm1⟩
  step with RatesProofs.ray_ord_min_spec old constants.pool.MAX_SUPPLY_INDEX_RAY as ⟨m2, hm2⟩
  apply WP.spec_mono (RatesProofs.ray_ord_max_spec m1 m2)
  intro n hn
  rw [RatesProofs.max_supply_index_val] at hm1 hm2
  rw [hm2, min_eq_left hoM] at hn
  rw [hm1] at hn
  have hq0 : 0 ≤ nv.val * 10^27 / s.val := IsFloor.nonneg hspos (Int.mul_nonneg (by omega) (by norm_num)) hq
  refine ⟨by rw [hn]; exact le_max_right _ _, by rw [hn]; exact max_le (min_le_right _ _) hoM, ?_⟩
  by_cases hc : min g.val (10^36) ≤ old.val
  · left; rw [hn, max_eq_right hc]
  · right
    intro t' ht'
    have : t' = t := IsHalfUp.unique (by norm_num) ht' ht
    subst this
    have hnle : n.val ≤ nv.val * 10^27 / s.val := by
      rw [hn, max_eq_left (by omega), hgv]
      simp only [min_le_iff, max_le_iff]; omega
    have hfl : nv.val * 10^27 / s.val * s.val ≤ nv.val * 10^27 :=
      Int.ediv_mul_le _ (by omega)
    calc s.val * n.val ≤ s.val * (nv.val * 10^27 / s.val) := Int.mul_le_mul_of_nonneg_left hnle hs
      _ = nv.val * 10^27 / s.val * s.val := by ring
      _ ≤ nv.val * 10^27 := hfl
      _ = (t' + rw.val) * 10^27 := by rw [hnv, hvt]

/-! ## `supply_index_reward_shortfall` (total correctness) -/

/-- **The reward shortfall does not abort when the index move distributes
between 0 and `rewards`.** With `t = half_up(supplied·old/RAY)` and
`t' = half_up(supplied·new/RAY)`, `t ≤ t' ≤ t + rewards < 2^127`, it returns
`rewards − (t' − t) ∈ [0, rewards]`. -/
theorem supply_index_reward_shortfall_total (env : Env) (s oi ni rw : math.fp.Ray) (t t' : Int)
    (hs : 0 ≤ s.val) (ho : 0 ≤ oi.val) (hon : oi.val ≤ ni.val)
    (ht : IsHalfUp (s.val * oi.val) (10^27) t) (ht' : IsHalfUp (s.val * ni.val) (10^27) t')
    (hle : t ≤ t') (hle' : t' ≤ t + rw.val) (hfit : t + rw.val < 2^127) :
    rates.index.supply_index_reward_shortfall env s oi ni rw ⦃ r =>
      r.val = rw.val - (t' - t) ∧ 0 ≤ r.val ∧ r.val ≤ rw.val ⦄ := by
  unfold rates.index.supply_index_reward_shortfall
  have ht0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hs ho) ht
  step with RatesProofs.ray_mul_spec env s ni hs (by omega) ⟨t', ht', by omega⟩ as ⟨a, ha⟩
  have hav : a.val = t' := IsHalfUp.unique (by norm_num) ha ht'
  step with RatesProofs.ray_mul_spec env s oi hs ho ⟨t, ht, by omega⟩ as ⟨b, hb⟩
  have hbv : b.val = t := IsHalfUp.unique (by norm_num) hb ht
  step with ray_checked_sub_spec env a b (by omega) (by omega) as ⟨d, hd⟩
  step with ray_checked_sub_spec env rw d (by omega) (by omega) as ⟨r, hr⟩
  omega

/-! ## The value ceiling inside `accrue_step` (INV-IDX-06) -/

/-- **`market_value` never aborts on non-negative operands.** It returns
`Some(half_up(shares·index/RAY))` when that value fits `i128` and `None`
exactly when it does not (`HalfUpResult`). The sign checks are the only abort
paths, and INV-ACCT-01 and the index bounds exclude them. -/
theorem market_value_spec (env : Env) (x i : math.fp.Ray) (hx : 0 ≤ x.val) (hi : 0 ≤ i.val) :
    rates.simulate.market_value env x i ⦃ o => HalfUpResult (x.val * i.val) (10^27) o ⦄ := by
  unfold rates.simulate.market_value
  have hx' : ¬ x.val < 0 := by omega
  have hi' : ¬ i.val < 0 := by omega
  simp only [RatesProofs.ray_lt_eq, RatesProofs.ray_zero_val_hu, bind_ok, hx', hi', decide_false,
    Bool.false_eq_true, massert, not_false_eq_true, ↓reduceIte]
  apply WP.spec_mono (ray_try_mul_spec env x i)
  rintro o ⟨_, ho⟩
  exact ho ⟨hx, hi⟩

/-- **`value_room` is the distance of the larger market total from
`MAX_MARKET_VALUE_RAY`, floored at zero.** Never aborts for non-negative
values: `room = max(0, MAX_MARKET_VALUE_RAY − max(debt_value, supply_value))`. -/
theorem value_room_spec (env : Env) (d s : math.fp.Ray) (hd : 0 ≤ d.val) (hs : 0 ≤ s.val) :
    rates.simulate.value_room env d s ⦃ r =>
      r.val = max 0 (MaxMarketValue - max d.val s.val) ⦄ := by
  unfold rates.simulate.value_room
  step with max_market_value_spec as ⟨m, hm⟩
  simp only [ray_from_same, bind_ok]
  step with RatesProofs.ray_ord_max_spec d s as ⟨top, htop⟩
  simp only [ray_ge_eq, bind_ok]
  by_cases hge : m.val ≤ top.val
  · simp only [hge, decide_true, ↓reduceIte, WP.spec_ok, zero_val]
    have hle : MaxMarketValue - max d.val s.val ≤ 0 := by rw [← hm, ← htop]; omega
    rw [max_eq_left hle]
  · simp only [hge, decide_false, Bool.false_eq_true, ↓reduceIte]
    have htop0 : 0 ≤ top.val := by rw [htop]; exact le_trans hd (le_max_left _ _)
    step with ray_checked_sub_spec env m top htop0 (by omega) as ⟨r, hr⟩
    have hle : 0 ≤ MaxMarketValue - max d.val s.val := by rw [← hm, ← htop]; omega
    rw [max_eq_right hle, hr, hm, htop]

/-- **`cap_borrow_index` never aborts and caps the debt-value growth at
`room`.** For `borrowed ≥ 0`, `0 ≤ old ≤ grown` and `room ≥ 0` the result `n`
satisfies `old ≤ n ≤ grown` and `borrowed·(n − old) ≤ room·RAY`: valued at
`n`, the debt exceeds its value at `old` by at most `room`. The allowed index
growth `⌊room·RAY/borrowed⌋` rounds down (and saturates at `i128::MAX`, which
only lowers it). -/
theorem cap_borrow_index_spec (env : Env) (b old g room : math.fp.Ray)
    (hb : 0 ≤ b.val) (ho : 0 ≤ old.val) (hog : old.val ≤ g.val) (hroom : 0 ≤ room.val) :
    rates.simulate.cap_borrow_index env b old g room ⦃ n =>
      old.val ≤ n.val ∧ n.val ≤ g.val ∧
      b.val * (n.val - old.val) ≤ room.val * 10^27 ⦄ := by
  unfold rates.simulate.cap_borrow_index
  have hR : 0 ≤ room.val * 10^27 := Int.mul_nonneg hroom (by norm_num)
  simp only [ray_eq_eq, zero_val, bind_ok, ray_le_eq]
  by_cases hb0 : b.val = 0
  · simp only [hb0, decide_true, ↓reduceIte, WP.spec_ok]
    refine ⟨hog, le_refl _, ?_⟩
    rw [zero_mul]; exact hR
  simp only [hb0, decide_false, Bool.false_eq_true, ↓reduceIte]
  by_cases hle : g.val ≤ old.val
  · simp only [hle, decide_true, ↓reduceIte, WP.spec_ok]
    refine ⟨hog, le_refl _, ?_⟩
    have : g.val - old.val = 0 := by omega
    rw [this, mul_zero]; exact hR
  simp only [hle, decide_false, Bool.false_eq_true, ↓reduceIte]
  have hbpos : 0 < b.val := by omega
  step with ray_div_floor_saturating_spec env room b hb0 as ⟨mg, hmg⟩
  have hq := isFloor_ediv (room.val * 10^27) b.val hbpos
  have hq0 : 0 ≤ room.val * 10^27 / b.val := IsFloor.nonneg hbpos hR hq
  have hmgv := hmg _ hq
  have hfl : room.val * 10^27 / b.val * b.val ≤ room.val * 10^27 :=
    Int.ediv_mul_le _ (by omega)
  have hmg0 : 0 ≤ mg.val := by rw [hmgv]; simp only [le_max_iff, le_min_iff]; omega
  have hmgq : mg.val ≤ room.val * 10^27 / b.val := by
    rw [hmgv]; simp only [max_le_iff, min_le_iff]; omega
  have hmgb : b.val * mg.val ≤ room.val * 10^27 := by
    calc b.val * mg.val ≤ b.val * (room.val * 10^27 / b.val) :=
          Int.mul_le_mul_of_nonneg_left hmgq hb
      _ = room.val * 10^27 / b.val * b.val := by ring
      _ ≤ room.val * 10^27 := hfl
  step with ray_checked_sub_spec env g old ho (by omega) as ⟨gr, hgr⟩
  by_cases hfit : gr.val ≤ mg.val
  · simp only [hfit, decide_true, ↓reduceIte, WP.spec_ok]
    refine ⟨hog, le_refl _, ?_⟩
    calc b.val * (g.val - old.val) ≤ b.val * mg.val :=
          Int.mul_le_mul_of_nonneg_left (by omega) hb
      _ ≤ room.val * 10^27 := hmgb
  · simp only [hfit, decide_false, Bool.false_eq_true, ↓reduceIte]
    have hgb := i128_bounds g
    step with ray_checked_add_spec env old mg (by omega) (by omega) as ⟨n, hn⟩
    refine ⟨by omega, by omega, ?_⟩
    have : n.val - old.val = mg.val := by omega
    rw [this]; exact hmgb

/-! ## `accrue_step` -/

/-- **One accrual step never aborts, for any non-negative share totals, and
holds interest at the market value ceiling (INV-IDX-06, audit finding S-04).**
It keeps the indexes monotone and capped (INV-IDX-01, INV-IDX-02) and its
revenue shares are worth at most the undistributed interest (INV-IDX-05).

Hypotheses (the stored-state invariants of one chunk; there is no hypothesis
that a market value fits `i128`):
* `ValidRateModel params` (`InterestRateModel::verify`, with
  `max_borrow_rate ≤ MAX_BORROW_RATE_RAY = 2·RAY`);
* share totals `borrowed, supplied ≥ 0` (INV-ACCT-01);
* `0 ≤ borrow_index ≤ MAX_BORROW_INDEX_RAY`, `0 < supply_index ≤
  MAX_SUPPLY_INDEX_RAY` (stored indexes start at `RAY`; INV-IDX-01/02 and the
  bad-debt floor keep them there);
* `delta_ms ≤ MAX_COMPOUND_DELTA_MS` (what `global_sync` passes).

Conclusions, writing `st` for the result: `borrow_index ≤ st.borrow_index ≤
10^36`, `supply_index ≤ st.supply_index ≤ 10^36`, `0 ≤ st.revenue_shares` and
`supplied + st.revenue_shares ≤ i128::MAX` (so the following `accrue_revenue`
cannot overflow); and with `od, nd` the old/new debt values
(`half_up(borrowed·index/RAY)`) and `t ≤ t'` the old/new supply values:

* the revenue shares valued at the new supply index plus the value distributed
  to suppliers never exceed the accrued interest:
  `revenue_shares·new_supply_index + (t' − t)·RAY ≤ (nd − od)·RAY`;
* the accrued interest is at most the room the larger total has below
  `MAX_MARKET_VALUE_RAY`: `nd − od ≤ max(0, MAX_MARKET_VALUE_RAY − max(od, t))`.
  When a total is at or above the ceiling, or does not fit `i128`, the room is
  zero and the step accrues nothing. -/
theorem accrue_step_spec (env : Env) (p : types.pool.MarketParams) (b s bi si : math.fp.Ray)
    (delta : Std.U64)
    (hp : RatesProofs.ValidRateModel p) (hb : 0 ≤ b.val) (hs : 0 ≤ s.val)
    (hbi : 0 ≤ bi.val) (hbiM : bi.val ≤ 10^36) (hsi : 0 < si.val) (hsiM : si.val ≤ 10^36)
    (hd : delta.val ≤ 31556926000) :
    rates.simulate.accrue_step env p b s bi si delta ⦃ st =>
      bi.val ≤ st.borrow_index.val ∧ st.borrow_index.val ≤ 10^36 ∧
      si.val ≤ st.supply_index.val ∧ st.supply_index.val ≤ 10^36 ∧
      0 ≤ st.revenue_shares.val ∧ s.val + st.revenue_shares.val ≤ 2^127 - 1 ∧
      ∃ od nd t t' : Int,
        IsHalfUp (b.val * bi.val) (10^27) od ∧
        IsHalfUp (b.val * st.borrow_index.val) (10^27) nd ∧
        IsHalfUp (s.val * si.val) (10^27) t ∧
        IsHalfUp (s.val * st.supply_index.val) (10^27) t' ∧ t ≤ t' ∧
        st.revenue_shares.val * st.supply_index.val + (t' - t) * 10^27 ≤ (nd - od) * 10^27 ∧
        nd - od ≤ max 0 (MaxMarketValue - max od t) ⦄ := by
  unfold rates.simulate.accrue_step
  have hsb := i128_bounds s
  -- old debt and supply values (as integers; they may not fit `i128`)
  obtain ⟨od, hod⟩ := IsHalfUp.exists (b.val * bi.val) (10^27) (by norm_num)
  have hod0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hb hbi) hod
  obtain ⟨t, ht⟩ := IsHalfUp.exists (s.val * si.val) (10^27) (by norm_num)
  have ht0 := IsHalfUp.nonneg (by norm_num) (Int.mul_nonneg hs (by omega)) ht
  -- the held result: both indexes unchanged, no revenue
  have held : ∀ flag : Bool,
      (ok ({ borrow_index := bi, supply_index := si, revenue_shares := math.fp.Ray.ZERO,
             value_capped := flag } : rates.simulate.AccrualStep)) ⦃ st =>
        bi.val ≤ st.borrow_index.val ∧ st.borrow_index.val ≤ 10^36 ∧
        si.val ≤ st.supply_index.val ∧ st.supply_index.val ≤ 10^36 ∧
        0 ≤ st.revenue_shares.val ∧ s.val + st.revenue_shares.val ≤ 2^127 - 1 ∧
        ∃ od nd t t' : Int,
          IsHalfUp (b.val * bi.val) (10^27) od ∧
          IsHalfUp (b.val * st.borrow_index.val) (10^27) nd ∧
          IsHalfUp (s.val * si.val) (10^27) t ∧
          IsHalfUp (s.val * st.supply_index.val) (10^27) t' ∧ t ≤ t' ∧
          st.revenue_shares.val * st.supply_index.val + (t' - t) * 10^27 ≤ (nd - od) * 10^27 ∧
          nd - od ≤ max 0 (MaxMarketValue - max od t) ⦄ := by
    intro flag
    simp only [WP.spec_ok, zero_val]
    refine ⟨le_refl _, hbiM, le_refl _, hsiM, le_refl _, by omega, od, od, t, t, hod, hod, ht,
      ht, le_refl _, by simp, ?_⟩
    rw [sub_self]; exact le_max_left _ _
  step with market_value_spec env b bi hb hbi as ⟨o, ho⟩
  step with market_value_spec env s si hs (by omega) as ⟨o1, ho1⟩
  cases o with
  | none =>
    simp only [ray_ne_eq, bind_ok]
    exact held _
  | some bo =>
    cases o1 with
    | none =>
      simp only [ray_ne_eq, bind_ok]
      exact held _
    | some so =>
      simp only [HalfUpResult] at ho ho1
      have hbov : bo.val = od := IsHalfUp.unique (by norm_num) ho hod
      have hsov : so.val = t := IsHalfUp.unique (by norm_num) ho1 ht
      have hbob := i128_bounds bo
      have hsob := i128_bounds so
      simp only
      -- room below the ceiling
      step with value_room_spec env bo so (by omega) (by omega) as ⟨room, hroom⟩
      rw [hbov, hsov] at hroom
      have hroom0 : 0 ≤ room.val := by rw [hroom]; exact le_max_left _ _
      have hMV : MaxMarketValue < 2^127 := by unfold MaxMarketValue; norm_num
      -- any value within `room` of an old total still fits `i128`
      have hfitd : od + room.val < 2^127 := by
        rw [hroom]
        rcases le_total (MaxMarketValue - max od t) 0 with h | h
        · rw [max_eq_left h]; omega
        · rw [max_eq_right h]; have := le_max_left od t; omega
      have hfits : t + room.val < 2^127 := by
        rw [hroom]
        rcases le_total (MaxMarketValue - max od t) 0 with h | h
        · rw [max_eq_left h]; omega
        · rw [max_eq_right h]; have := le_max_right od t; omega
      -- utilization and rate
      step with RatesProofs.utilization_spec env bo so (by omega) (by omega) as ⟨u, hu0, _⟩
      step with RatesProofs.calculate_borrow_rate_spec env u p hu0 hp as ⟨r, a, _, ha1, ha2, hr⟩
      have hb0 := hp.base_nonneg
      have hmax := hp.max_le
      have hx := compound_x_le (d := (delta.val : Int)) (by omega) (by omega) (by omega)
        (by omega) hr
      -- growth factor and borrow index
      step with compound_interest_spec env r delta hx.1 hx.2 as ⟨f, hf1, hf2⟩
      have hdel0 : (0 : Int) ≤ delta.val := by omega
      have hrd0 : 0 ≤ r.val * delta.val := Int.mul_nonneg hx.1 hdel0
      step with RatesProofs.update_borrow_index_spec env bi f hbi hbiM (by omega) (by omega)
        as ⟨gbi, hn1, hn2, _⟩
      have hbig : bi.val ≤ gbi.val := hn2 (by omega)
      -- the ceiling cap on the index
      step with cap_borrow_index_spec env b bi gbi room hb hbi hbig hroom0
        as ⟨nbi, hc1, hc2, hc3⟩
      obtain ⟨nd, hnd⟩ := IsHalfUp.exists (b.val * nbi.val) (10^27) (by norm_num)
      have hndle : nd ≤ od + room.val :=
        hu_add_le (by nlinarith) hod hnd
      -- reward split
      have hrf : 0 ≤ p.reserve_factor.val ∧ p.reserve_factor.val ≤ 10000 :=
        ⟨hp.rf_nonneg, by have := hp.rf_lt_bps; omega⟩
      step with RatesProofs.calculate_supplier_rewards_spec env p b nbi bi hb hbi hc1 hrf
        (fun x h => by have := IsHalfUp.unique (by norm_num) h hnd; omega)
        as ⟨sr, fee, hsr0, hfee0, od', nd', hod', hnd', hsum, _⟩
      have hodeq : od' = od := IsHalfUp.unique (by norm_num) hod' hod
      have hndeq : nd' = nd := IsHalfUp.unique (by norm_num) hnd' hnd
      subst hodeq hndeq
      -- supply index
      step with update_supply_index_strong env s si sr hs hsi hsiM hsr0
        (fun t2 h => by have := IsHalfUp.unique (by norm_num) h ht; omega)
        as ⟨nsi, hs1, hs2, hs3⟩
      obtain ⟨t', ht'⟩ := IsHalfUp.exists (s.val * nsi.val) (10^27) (by norm_num)
      have htt' : t ≤ t' :=
        IsHalfUp.mono (by norm_num) (Int.mul_le_mul_of_nonneg_left hs1 hs) ht ht'
      have ht'le : t' ≤ t + sr.val := by
        rcases hs3 with h | h
        · rw [h] at ht'; have := IsHalfUp.unique (by norm_num) ht' ht; omega
        · exact hu_le_of_le (h t ht) ht'
      step with supply_index_reward_shortfall_total env s si nsi sr t t' hs (by omega) hs1 ht ht'
        htt' ht'le (by omega) as ⟨sf, hsf, hsf0, hsfr⟩
      step with ray_checked_add_spec env fee sf (by omega) (by omega) as ⟨pr, hpr⟩
      simp only [ray_eq_eq, bind_ok, zero_val]
      have hint : nd' - od' ≤ max 0 (MaxMarketValue - max od' t) := by rw [← hroom]; omega
      by_cases hz : pr.val = 0
      · simp only [hz, decide_true, ↓reduceIte, ray_ne_eq, bind_ok, WP.spec_ok, zero_val]
        refine ⟨by omega, by omega, hs1, hs2, le_refl _, by omega,
          od', nd', t, t', hod', hnd', ht, ht', htt', ?_, hint⟩
        nlinarith
      · simp only [hz, decide_false, Bool.false_eq_true, ↓reduceIte]
        step with protocol_fee_shares_spec env pr nsi s (by omega) (by omega) hs
          as ⟨sh, hsh0, hshs, hshv⟩
        simp only [ray_ne_eq, bind_ok, WP.spec_ok]
        refine ⟨by omega, by omega, hs1, hs2, hsh0, hshs, od', nd', t, t', hod', hnd', ht, ht',
          htt', ?_, hint⟩
        rw [hpr, hsf] at hshv
        nlinarith

/-- **Accrual never takes a market total across `MAX_MARKET_VALUE_RAY`
(INV-IDX-06).** Arithmetic consequence of `accrue_step_spec`: if the old debt
value `od` and the old supply value `t` are at most the ceiling, then so are
the new debt value `nd` and the supply value `t''` after the step's revenue
shares `rev` are added to total supply at the new index. So a state within the
ceiling stays within it under accrual, for any number of chunks. -/
theorem accrue_step_within_ceiling {s rev nsi od nd t t' t'' : Int}
    (hodM : od ≤ MaxMarketValue) (htM : t ≤ MaxMarketValue)
    (ht' : IsHalfUp (s * nsi) (10^27) t')
    (ht'' : IsHalfUp ((s + rev) * nsi) (10^27) t'')
    (hrev : rev * nsi + (t' - t) * 10^27 ≤ (nd - od) * 10^27)
    (hint : nd - od ≤ max 0 (MaxMarketValue - max od t)) :
    nd ≤ MaxMarketValue ∧ t'' ≤ MaxMarketValue := by
  have hmax : max od t ≤ MaxMarketValue := max_le hodM htM
  rw [max_eq_right (by omega)] at hint
  have h1 := le_max_left od t
  have h2 := le_max_right od t
  refine ⟨by omega, ?_⟩
  have hle : t'' ≤ t' + (nd - od - (t' - t)) :=
    hu_add_le (by nlinarith) ht' ht''
  omega

end PoolAeneas.Proofs
