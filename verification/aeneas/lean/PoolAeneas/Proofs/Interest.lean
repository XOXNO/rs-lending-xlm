/-
Protocol revenue booking and bad-debt socialization of
`contracts/pool/src/interest.rs`.
-/
import PoolAeneas.Proofs.Shares
import PoolAeneas.Proofs.Wrappers

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- `SUPPLY_INDEX_FLOOR_RAW = RAY / 1000` evaluates to `10^24`. -/
theorem supply_index_floor_spec :
    constants.pool.SUPPLY_INDEX_FLOOR_RAW ⦃ v => v.val = 10^24 ⦄ := by
  unfold constants.pool.SUPPLY_INDEX_FLOOR_RAW
  have : constants.shared.RAY = 1000000000000000000000000000#i128 := by
    unfold constants.shared.RAY; rfl
  rw [this]
  step as ⟨v, hv⟩
  rw [hv]; rfl

/-- `Ray::from(v)` for an `i128` wraps `v` unchanged. -/
theorem ray_from_same (v : Std.I128) :
    math.fp.Ray.from (core.convert.IntoFrom (core.convert.FromSame Std.I128)) v = ok v := by
  simp [math.fp.Ray.from, core.convert.IntoFrom, core.convert.IntoFrom.into,
    core.convert.FromSame, core.convert.FromSame.from]

/-! ## `add_protocol_revenue` -/

/-- **`add_protocol_revenue` books a fee as revenue shares and never aborts.**
For a RAY fee `fee ≥ 0`: a zero fee leaves the cache unchanged; otherwise it
mints `s` revenue shares into both `revenue` and `supplied` (nothing else
changes), where `s = min(⌊fee * RAY / supply_index⌋, i128::MAX - supplied)`
(the floor saturating at `i128::MAX`): floor-rounded, so the protocol never
books more shares than the fee is worth, and capped so `supplied` cannot
overflow. INV-ACCT-01 is preserved.

Hypotheses: INV-ACCT-01 (`SharesInv`), `supply_index > 0` (floored at
`SUPPLY_INDEX_FLOOR_RAW`), `fee ≥ 0` (a flash fee or accrued interest). -/
theorem add_protocol_revenue_spec (c : market_cache.Cache) (fee : math.fp.Ray)
    (hinv : SharesInv c) (hidx : 0 < c.supply_index.val) (hfee : 0 ≤ fee.val) :
    interest.add_protocol_revenue c fee ⦃ c' =>
      SharesInv c' ∧
      (fee.val = 0 → c' = c) ∧
      (fee.val ≠ 0 → ∃ s : Int, 0 ≤ s ∧
        (∀ q, IsFloor (fee.val * 10^27) c.supply_index.val q →
          s = min (min (2^127 - 1) q) (2^127 - 1 - c.supplied.val)) ∧
        c'.revenue.val = c.revenue.val + s ∧ c'.supplied.val = c.supplied.val + s ∧
        c' = { c with revenue := c'.revenue, supplied := c'.supplied }) ⦄ := by
  unfold interest.add_protocol_revenue
  simp only [ray_eq_eq, zero_val, bind_ok]
  by_cases h0 : fee.val = 0
  · simp only [h0, decide_true, ↓reduceIte, WP.spec_ok]
    exact ⟨hinv, by simp, by simp⟩
  simp only [h0, decide_false, Bool.false_eq_true, ↓reduceIte, market_cache.Cache.impl.env,
    market_cache.Cache.impl.supply_index, market_cache.Cache.impl.supplied, bind_ok]
  unfold rates.index.protocol_fee_shares
  step with ray_div_floor_saturating_spec c.env fee c.supply_index (by omega) as ⟨raw, hraw⟩
  simp only [math.fp.Ray.raw, bind_ok]
  have hq := isFloor_ediv (fee.val * 10^27) c.supply_index.val hidx
  have hq0 : 0 ≤ fee.val * 10^27 / c.supply_index.val :=
    IsFloor.nonneg hidx (by positivity) hq
  have hrv := hraw _ hq
  have hsb := i128_bounds c.supplied
  have hS0 : 0 ≤ c.supplied.val := by unfold SharesInv at hinv; omega
  simp only [lift, ray_from_same, bind_ok]
  have hmax : (core.num.I128.MAX).val = 2^127 - 1 := by
    simp [core.num.I128.MAX, I128.rMax]
  have hhead := sat_sub_val core.num.I128.MAX c.supplied (by rw [hmax]; omega) (by rw [hmax]; omega)
  rw [hmax] at hhead
  generalize hs : core.cmp.impls.OrdI128.min raw (core.num.I128.saturating_sub core.num.I128.MAX c.supplied) = s
  have hsv : s.val = min raw.val (2^127 - 1 - c.supplied.val) := by
    rw [← hs, min_i128_val, hhead]
  have hs0 : 0 ≤ s.val := by
    rw [hsv, hrv]; simp only [le_min_iff, le_max_iff]; omega
  have hsfit : c.supplied.val + s.val ≤ 2^127 - 1 := by rw [hsv]; omega
  unfold SharesInv at hinv
  have hrb := i128_bounds c.revenue
  unfold market_cache.shares.Cache.accrue_revenue
  step with ray_checked_add_spec c.env c.revenue s (by omega) (by omega) as ⟨r, hr⟩
  step with ray_checked_add_spec c.env c.supplied s (by omega) (by omega) as ⟨r1, hr1⟩
  refine ⟨⟨by simp; omega, by simp; omega, hinv.2.2⟩, by intro h; exfalso; simp_all, fun _ =>
    ⟨s.val, hs0, fun q hq' => ?_, hr, hr1⟩⟩
  rw [hsv, hraw q hq']
  have hq'0 : 0 ≤ q := IsFloor.nonneg hidx (by positivity) hq'
  rw [max_eq_right (by simp only [le_min_iff]; omega)]

/-! ## `apply_bad_debt_to_supply_index` (INV-IDX-03) -/

/-- **Bad-debt socialization lowers the supply index pro rata, floored
(INV-IDX-03).** With `V` the supplied value `supplied * supply_index / RAY`
rounded half up (`Ray::mul`): if `V = 0` the cache is unchanged. Otherwise,
with `rem = V - min(bad_debt, V)`, `f = ⌊rem * RAY / V⌋` and
`n = ⌊supply_index * f / RAY⌋`, the new index is `max(n, RAY/1000)` and
nothing but the index changes. Consequences: the index never drops below
`SUPPLY_INDEX_FLOOR_RAW = RAY/1000`, never exceeds
`max(supply_index, RAY/1000)`, and the written-down index is at most the
exact pro-rata index (`n * V ≤ supply_index * rem`). Never aborts.

Hypotheses: `supplied ≥ 0` (INV-ACCT-01), `supply_index > 0`, `bad_debt ≥ 0`
(an unpaid debt value), and `V` fits `i128` (otherwise `Ray::mul` aborts). -/
theorem apply_bad_debt_spec (c : market_cache.Cache) (bad : math.fp.Ray) (V : Int)
    (hs : 0 ≤ c.supplied.val) (hidx : 0 < c.supply_index.val) (hbad : 0 ≤ bad.val)
    (hV : IsHalfUp (c.supplied.val * c.supply_index.val) (10^27) V) (hVfit : V < 2^127) :
    interest.apply_bad_debt_to_supply_index c bad ⦃ c' =>
      c' = { c with supply_index := c'.supply_index } ∧
      (V = 0 → c' = c) ∧
      (V ≠ 0 → ∃ f n : Int,
        IsFloor ((V - min bad.val V) * 10^27) V f ∧
        IsFloor (c.supply_index.val * f) (10^27) n ∧
        c'.supply_index.val = max n (10^24) ∧
        10^24 ≤ c'.supply_index.val ∧
        c'.supply_index.val ≤ max c.supply_index.val (10^24) ∧
        n * V ≤ c.supply_index.val * (V - min bad.val V)) ⦄ := by
  have hV0 : 0 ≤ V := IsHalfUp.nonneg (by norm_num) (by positivity) hV
  unfold interest.apply_bad_debt_to_supply_index math.fp.Ray.mul
  simp only [market_cache.Cache.impl.supplied, market_cache.Cache.impl.env,
    market_cache.Cache.impl.supply_index, bind_ok]
  step with mul_div_half_up_spec c.env c.supplied c.supply_index constants.shared.RAY hs
    (by omega) (by rw [ray_val]; norm_num) ⟨V, by rw [ray_val]; exact hV, hVfit⟩ as ⟨tv, htv⟩
  rw [ray_val] at htv
  have htvV : tv.val = V := IsHalfUp.unique (by norm_num) htv hV
  simp only [ray_eq_eq, zero_val, htvV]
  by_cases h0 : V = 0
  · have hb : decide (V = 0) = true := by simp [h0]
    simp only [bind_ok, hb, ↓reduceIte, WP.spec_ok]
    exact ⟨by simp, fun _ => trivial, fun h => absurd h0 h⟩
  have hVpos : 0 < V := lt_of_le_of_ne hV0 (Ne.symm h0)
  simp only [h0, decide_false, Bool.false_eq_true, ↓reduceIte, ray_min_eq, bind_ok]
  generalize hcap : (if tv.val < bad.val then tv else bad) = cap
  have hcapv : cap.val = min bad.val V := by
    rw [← hcap]; split <;> rename_i h <;> (try simp only [htvV] at h ⊢) <;> omega
  step with ray_checked_sub_spec c.env tv cap (by omega) (by omega) as ⟨rem, hrem⟩
  rw [htvV, hcapv] at hrem
  have hrem0 : 0 ≤ rem.val := by omega
  have hremle : rem.val ≤ V := by omega
  -- the reduction factor f ≤ RAY
  have hf := isFloor_ediv (rem.val * 10^27) V hVpos
  have hf0 : 0 ≤ rem.val * 10^27 / V := IsFloor.nonneg hVpos (by positivity) hf
  have hfle : rem.val * 10^27 / V ≤ 10^27 := by
    have h' := hf; unfold IsFloor at h'; rw [if_pos hVpos] at h'
    by_contra hc; have hc := not_le.mp hc
    have : (10^27 + 1) * V ≤ (rem.val * 10^27 / V) * V := by nlinarith
    nlinarith
  step with ray_div_floor_spec c.env rem tv (by omega)
    ⟨_, by rw [htvV]; exact hf, by omega, by omega⟩ as ⟨f, hfv⟩
  rw [htvV] at hfv
  have hff : f.val = rem.val * 10^27 / V := IsFloor.unique (by omega) hfv hf
  have hn := isFloor_ediv (c.supply_index.val * f.val) (10^27) (by norm_num)
  have hib := i128_bounds c.supply_index
  have hn0 : 0 ≤ c.supply_index.val * f.val / 10^27 :=
    IsFloor.nonneg (by norm_num) (by rw [hff]; positivity) hn
  have hnle : c.supply_index.val * f.val / 10^27 ≤ c.supply_index.val := by
    have h' := hn; unfold IsFloor at h'; rw [if_pos (by norm_num)] at h'
    have : c.supply_index.val * f.val ≤ c.supply_index.val * 10^27 := by
      rw [hff]; exact mul_le_mul_of_nonneg_left hfle (by omega)
    nlinarith
  step with ray_mul_floor_spec c.env c.supply_index f
    ⟨_, hn, by omega, by omega⟩ as ⟨n, hnv⟩
  have hnn : n.val = c.supply_index.val * f.val / 10^27 := IsFloor.unique (by norm_num) hnv hn
  step with supply_index_floor_spec as ⟨fl, hfl⟩
  simp only [ray_from_same, ray_max_eq, bind_ok]
  unfold market_cache.report.Cache.set_supply_index
  simp only [WP.spec_ok]
  have hmaxv : (if fl.val < n.val then n else fl).val = max n.val (10^24) := by
    split <;> rename_i h <;> (try simp only [hfl] at h ⊢) <;> omega
  refine ⟨by simp, by intro h; exfalso; simp_all, fun _ => ⟨f.val, n.val, ?_, hnv, hmaxv, ?_, ?_, ?_⟩⟩
  · rw [← hrem]; exact hfv
  · rw [hmaxv]; omega
  · rw [hmaxv]; omega
  · -- n * V ≤ idx * rem: from n * RAY ≤ idx * f and f * V ≤ rem * RAY
    have h1 := hnv; unfold IsFloor at h1; rw [if_pos (by norm_num)] at h1
    have h2 := hfv; unfold IsFloor at h2; rw [if_pos hVpos] at h2
    rw [← hrem]
    have a1 : n.val * V * 10^27 ≤ c.supply_index.val * f.val * V := by nlinarith
    have a2 : c.supply_index.val * (f.val * V) ≤ c.supply_index.val * (rem.val * 10^27) :=
      mul_le_mul_of_nonneg_left h2.1 (by omega)
    nlinarith

end PoolAeneas.Proofs
