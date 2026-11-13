import RatesAeneas.Proofs.CurveBase
import RatesAeneas.Proofs.TypedWrappers

/-
`rates::curve` (common/src/rates/curve.rs): the three-segment borrow-rate
curve, the per-millisecond borrow rate and the deposit rate.

Hypotheses on the rate model are checks of
`InterestRateModel::verify` (common/src/types/pool.rs), a subset of them (see
`ValidRateModel`), which every market
creation and parameter update runs (contracts/pool/src/ops/market.rs,
contracts/governance/src/validate/asset.rs, contracts/governance/src/op.rs).
-/

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open rates_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 1000000

namespace RatesAeneas.Proofs

local notation "rv " r:max => IScalar.val (ty := IScalarTy.I128) r

/-- Checks `InterestRateModel::verify` performs, on the fields the rate
code reads (`MarketParams` carries them unchanged from the verified model):
non-negative base, `base ≤ slope1`, non-negative `slope2` and `slope3`,
`slope3 ≤ max`, `max > base`, `max ≤ MAX_BORROW_RATE_RAY = 2·RAY`,
`base + slope1 + slope2 + slope3 ≤ max`, `0 < mid < optimal < RAY`,
`optimal ≤ max_utilization ≤ RAY`, and `0 ≤ reserve_factor < BPS`
(`reserve_factor` is a `u32`). `verify` also requires the steepness per unit
of utilization not to fall at either kink (`slope1 / mid ≤ slope2 / (optimal - mid)
≤ slope3 / (RAY - optimal)`) and `slope1, slope2 ≤ max`; no theorem here needs
those, so they are left out and every theorem covers more models than `verify`
admits. -/
structure ValidRateModel (p : types.MarketParams) : Prop where
  base_nonneg : 0 ≤ rv p.base_borrow_rate
  base_le_s1 : rv p.base_borrow_rate ≤ rv p.slope1
  s2_nonneg : 0 ≤ rv p.slope2
  s3_nonneg : 0 ≤ rv p.slope3
  s3_le_max : rv p.slope3 ≤ rv p.max_borrow_rate
  base_lt_max : rv p.base_borrow_rate < rv p.max_borrow_rate
  max_le : rv p.max_borrow_rate ≤ 2 * 10^27
  sum_le_max : rv p.base_borrow_rate + rv p.slope1 + rv p.slope2 + rv p.slope3 ≤
    rv p.max_borrow_rate
  mid_pos : 0 < rv p.mid_utilization
  mid_lt_opt : rv p.mid_utilization < rv p.optimal_utilization
  opt_lt_ray : rv p.optimal_utilization < 10^27
  opt_le_maxu : rv p.optimal_utilization ≤ rv p.max_utilization
  maxu_le_ray : rv p.max_utilization ≤ 10^27
  rf_nonneg : 0 ≤ rv p.reserve_factor
  rf_lt_bps : rv p.reserve_factor < 10000

/-- The contribution of one curve segment after #225: `half_up(slope * e / range)`,
one rounding (`slope_contribution` = `Ray::mul_ray_ratio`), stated without
division. Before #225 it was `half_up(half_up(e*slope/RAY) * RAY / range)`. -/
def SegContrib (e s g c : Int) : Prop :=
  IsHalfUp (s * e) g c

/-- The annual borrow rate the code computes at clamped utilization
`m = min(u, RAY)`: the segment formula, capped at `max_borrow_rate`. -/
def AnnualRate (p : types.MarketParams) (m r : Int) : Prop :=
  (m < rv p.mid_utilization →
    ∃ c, SegContrib m (rv p.slope1) (rv p.mid_utilization) c ∧
      r = min (rv p.base_borrow_rate + c) (rv p.max_borrow_rate)) ∧
  (rv p.mid_utilization ≤ m → m < rv p.optimal_utilization →
    ∃ c, SegContrib (m - rv p.mid_utilization) (rv p.slope2)
        (rv p.optimal_utilization - rv p.mid_utilization) c ∧
      r = min (rv p.base_borrow_rate + rv p.slope1 + c) (rv p.max_borrow_rate)) ∧
  (rv p.optimal_utilization ≤ m →
    ∃ c, SegContrib (m - rv p.optimal_utilization) (rv p.slope3)
        (10^27 - rv p.optimal_utilization) c ∧
      r = min (rv p.base_borrow_rate + rv p.slope1 + rv p.slope2 + c) (rv p.max_borrow_rate))

/-- A segment contribution is monotone in the excess utilization. -/
theorem SegContrib.mono {e₁ e₂ s g c₁ c₂ : Int} (hs : 0 ≤ s) (hg : 0 < g) (he : e₁ ≤ e₂)
    (h₁ : SegContrib e₁ s g c₁) (h₂ : SegContrib e₂ s g c₂) : c₁ ≤ c₂ :=
  IsHalfUp.mono hg (Int.mul_le_mul_of_nonneg_left he hs) h₁ h₂

/-- **A segment contribution lies in `[0, slope]`.** For `0 ≤ e ≤ range`,
`half_up(slope * e / range) ≤ slope`: the contribution at the top of a segment
never exceeds the slope, which is what makes the curve monotone across
segment boundaries (`calculate_annual_borrow_rate_monotone`). -/
theorem SegContrib.bound {e s g c : Int} (he : 0 ≤ e) (heg : e ≤ g) (hg : 0 < g) (hs : 0 ≤ s)
    (h : SegContrib e s g c) : 0 ≤ c ∧ c ≤ s :=
  IsHalfUp.le_of_ratio_le_one hg hs he heg h

/-- At zero excess the contribution is 0. -/
theorem SegContrib.zero {s g c : Int} (hg : 0 < g) (h : SegContrib 0 s g c) : c = 0 :=
  IsHalfUp.unique hg h (by unfold IsHalfUp; simp; omega)

/-- At the full segment width the contribution is exactly the slope. -/
theorem SegContrib.full {s g c : Int} (hg : 0 < g) (h : SegContrib g s g c) : c = s :=
  IsHalfUp.unique hg h (IsHalfUp.mul_self s g hg)

/-- **`slope_contribution` is one half-up rounding, at most the slope** (#225).
For `0 ≤ excess ≤ range`, `range > 0` and `slope ≥ 0`, the call never aborts
and returns `c = half_up(slope * excess / range)` (`SegContrib`) with
`0 ≤ c ≤ slope`. (Outside this domain it is `Ray::mul_ray_ratio`, see
`ray_mul_ray_ratio_returns_iff`.) -/
theorem slope_contribution_spec (env : Env) (e s g : math.fp.Ray)
    (he : 0 ≤ rv e) (heg : rv e ≤ rv g) (hg : 0 < rv g) (hs : 0 ≤ rv s) :
    rates.curve.slope_contribution env e s g ⦃ c =>
      SegContrib (rv e) (rv s) (rv g) (rv c) ∧ 0 ≤ rv c ∧ rv c ≤ rv s ⦄ := by
  unfold rates.curve.slope_contribution
  exact ray_mul_ray_ratio_frac_spec env s e g hs he heg hg

/-- **`calculate_annual_borrow_rate` computes the capped three-segment
curve, and never aborts.** For every utilization `u ≥ 0` (production:
`utilization` returns a non-negative ratio) and every rate model that passes
`InterestRateModel::verify` (`ValidRateModel`), the call returns `r` with
`AnnualRate p (min u RAY) r`: the segment formula of the clamped utilization,
each segment contributing `half_up(slope * excess / range)` with a single
rounding (`slope_contribution`, #225), capped at `max_borrow_rate`. -/
theorem calculate_annual_borrow_rate_spec (env : Env) (u : math.fp.Ray) (p : types.MarketParams)
    (hu : 0 ≤ rv u) (hp : ValidRateModel p) :
    rates.curve.calculate_annual_borrow_rate env u p ⦃ r =>
      AnnualRate p (min (rv u) (10^27)) (rv r) ⦄ := by
  obtain ⟨hb0, hbs1, hs2n, hs3n, hs3m, hbm, hm2, hsum, hmid, hmo, hor, _, _, _, _⟩ := hp
  unfold rates.curve.calculate_annual_borrow_rate
  rw [ray_gt_eq u math.fp.Ray.ONE, ray_one_val]
  simp only [bind_ok]
  generalize hm : min (rv u) (10^27) = m
  have tail : ∀ X : math.fp.Ray, 0 ≤ rv X → rv X ≤ 10^27 → rv X = m →
      (do
        let b1 ← core.cmp.PartialOrd.lt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay
          X p.mid_utilization
        let annual_rate ←
          if b1 then do
            let contribution ← rates.curve.slope_contribution env X p.slope1 p.mid_utilization
            math.fp.Ray.checked_add p.base_borrow_rate env contribution
          else do
            let b2 ← core.cmp.PartialOrd.lt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay
              X p.optimal_utilization
            if b2 then do
              let excess ← math.fp.Ray.checked_sub X env p.mid_utilization
              let range ← math.fp.Ray.checked_sub p.optimal_utilization env p.mid_utilization
              let contribution ← rates.curve.slope_contribution env excess p.slope2 range
              let r1 ← math.fp.Ray.checked_add p.base_borrow_rate env p.slope1
              math.fp.Ray.checked_add r1 env contribution
            else do
              let r ← math.fp.Ray.checked_add p.base_borrow_rate env p.slope1
              let base_rate ← math.fp.Ray.checked_add r env p.slope2
              let excess ← math.fp.Ray.checked_sub X env p.optimal_utilization
              let range ← math.fp.Ray.checked_sub math.fp.Ray.ONE env p.optimal_utilization
              let contribution ← rates.curve.slope_contribution env excess p.slope3 range
              math.fp.Ray.checked_add base_rate env contribution
        let b2 ← core.cmp.PartialOrd.gt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay
          annual_rate p.max_borrow_rate
        if b2 then ok p.max_borrow_rate else ok annual_rate) ⦃ r => AnnualRate p m (rv r) ⦄ := by
    intro X hX0 hX1 hXm
    have cap : ∀ a : math.fp.Ray, (do
        let b2 ← core.cmp.PartialOrd.gt.trait_default math.fp.Ray.Insts.CoreCmpPartialOrdRay
          a p.max_borrow_rate
        if b2 then ok p.max_borrow_rate else ok a) ⦃ r => rv r = min (rv a) (rv p.max_borrow_rate) ⦄ := by
      intro a
      simp only [ray_gt_eq, bind_ok]
      by_cases hc : rv p.max_borrow_rate < rv a
      · simp only [hc, decide_true, ↓reduceIte, WP.spec_ok]; omega
      · simp only [hc, decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]; omega
    simp only [ray_lt_eq, bind_ok]
    by_cases h1 : rv X < rv p.mid_utilization
    · simp only [h1, decide_true, ↓reduceIte]
      step with slope_contribution_spec env X p.slope1 p.mid_utilization hX0 (by omega) hmid
        (by omega) as ⟨c, hc, hc0, hcs⟩
      step with ray_checked_add_spec env p.base_borrow_rate c (by omega) as ⟨a, ha⟩
      apply WP.spec_mono (cap a)
      intro z hz
      refine ⟨fun _ => ⟨rv c, by rw [← hXm]; exact hc, by rw [hz, ha]⟩,
        fun h => absurd h (by omega), fun h => absurd h (by omega)⟩
    · simp only [h1, decide_false, Bool.false_eq_true, ↓reduceIte, ray_lt_eq, bind_ok]
      by_cases h2 : rv X < rv p.optimal_utilization
      · simp only [h2, decide_true, ↓reduceIte]
        step with ray_checked_sub_spec env X p.mid_utilization (by omega) (by omega) as ⟨e, he⟩
        step with ray_checked_sub_spec env p.optimal_utilization p.mid_utilization (by omega)
          (by omega) as ⟨g, hg⟩
        step with slope_contribution_spec env e p.slope2 g (by omega) (by omega) (by omega)
          (by omega) as ⟨c, hc, hc0, hcs⟩
        step with ray_checked_add_spec env p.base_borrow_rate p.slope1 (by omega) as ⟨a1, ha1⟩
        step with ray_checked_add_spec env a1 c (by omega) as ⟨a, ha⟩
        apply WP.spec_mono (cap a)
        intro z hz
        refine ⟨fun h => absurd h (by omega), fun _ _ => ⟨rv c, ?_, ?_⟩,
          fun h => absurd h (by omega)⟩
        · rw [← hXm, ← he, ← hg]; exact hc
        · rw [hz, ha, ha1]
      · simp only [h2, decide_false, Bool.false_eq_true, ↓reduceIte]
        step with ray_checked_add_spec env p.base_borrow_rate p.slope1 (by omega) as ⟨a1, ha1⟩
        step with ray_checked_add_spec env a1 p.slope2 (by omega) as ⟨a2, ha2⟩
        step with ray_checked_sub_spec env X p.optimal_utilization (by omega) (by omega) as ⟨e, he⟩
        step with ray_checked_sub_spec env math.fp.Ray.ONE p.optimal_utilization (by omega)
          (by rw [ray_one_val]; omega) as ⟨g, hg⟩
        rw [ray_one_val] at hg
        step with slope_contribution_spec env e p.slope3 g (by omega) (by omega) (by omega)
          (by omega) as ⟨c, hc, hc0, hcs⟩
        step with ray_checked_add_spec env a2 c (by omega) as ⟨a, ha⟩
        apply WP.spec_mono (cap a)
        intro z hz
        refine ⟨fun h => absurd h (by omega), fun _ h => absurd h (by omega),
          fun _ => ⟨rv c, ?_, ?_⟩⟩
        · rw [← hXm, ← he, ← hg]; exact hc
        · rw [hz, ha, ha2, ha1]
  by_cases hU : 10^27 < rv u
  · simp only [hU, decide_true, ↓reduceIte, bind_ok]
    exact tail math.fp.Ray.ONE (by rw [ray_one_val]; norm_num) (by rw [ray_one_val])
      (by rw [ray_one_val, ← hm]; omega)
  · simp only [hU, decide_false, Bool.false_eq_true, ↓reduceIte, bind_ok]
    exact tail u hu (by omega) (by rw [← hm]; omega)

/-- Pure part of the bound: every `AnnualRate` value lies in `[base, max]`
(the uncapped value is at most `base + slope1 + slope2 + slope3 ≤ max`). -/
theorem AnnualRate.bounds {p : types.MarketParams} {m r : Int} (hp : ValidRateModel p)
    (hm0 : 0 ≤ m) (hm1 : m ≤ 10^27) (h : AnnualRate p m r) :
    rv p.base_borrow_rate ≤ r ∧ r ≤ rv p.max_borrow_rate := by
  obtain ⟨hb0, hbs1, hs2n, hs3n, hs3m, hbm, hm2, hsum, hmid, hmo, hor, _, _, _, _⟩ := hp
  obtain ⟨h1, h2, h3⟩ := h
  by_cases c1 : m < rv p.mid_utilization
  · obtain ⟨c, hc, hr⟩ := h1 c1
    have := SegContrib.bound hm0 (by omega) hmid (by omega) hc
    rw [hr]; constructor <;> simp only [le_min_iff, min_le_iff] <;> omega
  by_cases c2 : m < rv p.optimal_utilization
  · obtain ⟨c, hc, hr⟩ := h2 (by omega) c2
    have := SegContrib.bound (by omega) (by omega) (by omega) (by omega) hc
    rw [hr]; constructor <;> simp only [le_min_iff, min_le_iff] <;> omega
  · obtain ⟨c, hc, hr⟩ := h3 (by omega)
    have := SegContrib.bound (by omega) (by omega) (by omega) (by omega) hc
    rw [hr]; constructor <;> simp only [le_min_iff, min_le_iff] <;> omega

/-- **The annual borrow rate is bounded: `base_borrow_rate ≤ r ≤ max_borrow_rate`.**
For every utilization `u ≥ 0` and every rate model passing
`InterestRateModel::verify`, the call returns (does not abort) a rate in
`[base_borrow_rate, max_borrow_rate]`, so in particular at most
`MAX_BORROW_RATE_RAY = 2·RAY`. -/
theorem calculate_annual_borrow_rate_bounds (env : Env) (u : math.fp.Ray) (p : types.MarketParams)
    (hu : 0 ≤ rv u) (hp : ValidRateModel p) :
    rates.curve.calculate_annual_borrow_rate env u p ⦃ r =>
      rv p.base_borrow_rate ≤ rv r ∧ rv r ≤ rv p.max_borrow_rate ⦄ := by
  apply WP.spec_mono (calculate_annual_borrow_rate_spec env u p hu hp)
  intro r hr
  exact AnnualRate.bounds hp (by omega) (by omega) hr

/-- Two clamped utilizations `m ≤ m'` lie in the same curve segment. -/
def SameSegment (p : types.MarketParams) (m m' : Int) : Prop :=
  (m' < rv p.mid_utilization) ∨
  (rv p.mid_utilization ≤ m ∧ m' < rv p.optimal_utilization) ∨
  (rv p.optimal_utilization ≤ m)

/-- Pure part of monotonicity: within one segment, `AnnualRate` is monotone. -/
theorem AnnualRate.mono_segment {p : types.MarketParams} {m m' r r' : Int}
    (hp : ValidRateModel p) (hmm : m ≤ m') (hseg : SameSegment p m m')
    (h : AnnualRate p m r) (h' : AnnualRate p m' r') : r ≤ r' := by
  obtain ⟨hb0, hbs1, hs2n, hs3n, hs3m, hbm, hm2, hsum, hmid, hmo, hor, _, _, _, _⟩ := hp
  obtain ⟨h1, h2, h3⟩ := h
  obtain ⟨h1', h2', h3'⟩ := h'
  rcases hseg with c | ⟨c1, c2⟩ | c
  · obtain ⟨x, hx, hr⟩ := h1 (by omega)
    obtain ⟨x', hx', hr'⟩ := h1' c
    have := SegContrib.mono (by omega) hmid hmm hx hx'
    rw [hr, hr']; exact min_le_min_right _ (by omega)
  · obtain ⟨x, hx, hr⟩ := h2 c1 (by omega)
    obtain ⟨x', hx', hr'⟩ := h2' (by omega) c2
    have := SegContrib.mono (by omega) (by omega) (by omega : m - _ ≤ m' - _) hx hx'
    rw [hr, hr']; exact min_le_min_right _ (by omega)
  · obtain ⟨x, hx, hr⟩ := h3 c
    obtain ⟨x', hx', hr'⟩ := h3' (by omega)
    have := SegContrib.mono (by omega) (by omega) (by omega : m - _ ≤ m' - _) hx hx'
    rw [hr, hr']; exact min_le_min_right _ (by omega)

/-- **The annual borrow rate is monotone in utilization within each curve
segment.** For utilizations `0 ≤ u ≤ u'` whose clamped values
`min(u, RAY) ≤ min(u', RAY)` lie in the same segment (`[0, mid)`,
`[mid, optimal)` or `[optimal, RAY]`), the rate at `u` is at most the rate at
`u'`. Since #225 this is a special case of `calculate_annual_borrow_rate_monotone`,
which needs no same-segment hypothesis; it is kept for existing references. -/
theorem calculate_annual_borrow_rate_mono (env : Env) (u u' : math.fp.Ray)
    (p : types.MarketParams) (hu : 0 ≤ rv u) (huu : rv u ≤ rv u') (hp : ValidRateModel p)
    (hseg : SameSegment p (min (rv u) (10^27)) (min (rv u') (10^27))) :
    rates.curve.calculate_annual_borrow_rate env u p ⦃ r =>
      rates.curve.calculate_annual_borrow_rate env u' p ⦃ r' => rv r ≤ rv r' ⦄ ⦄ := by
  apply WP.spec_mono (calculate_annual_borrow_rate_spec env u p hu hp)
  intro r hr
  apply WP.spec_mono (calculate_annual_borrow_rate_spec env u' p (by omega) hp)
  intro r' hr'
  exact AnnualRate.mono_segment hp (by omega) hseg hr hr'

/-- Pure part of global monotonicity: for clamped utilizations
`0 ≤ m ≤ m' ≤ RAY`, `AnnualRate` is monotone across segment boundaries. Each
segment's uncapped value starts at `base` plus the slopes below it and, since
a contribution is at most its slope (`SegContrib.bound`), ends at most where
the next segment starts. -/
theorem AnnualRate.mono {p : types.MarketParams} {m m' r r' : Int}
    (hp : ValidRateModel p) (hm0 : 0 ≤ m) (hmm : m ≤ m') (hm1 : m' ≤ 10^27)
    (h : AnnualRate p m r) (h' : AnnualRate p m' r') : r ≤ r' := by
  obtain ⟨hb0, hbs1, hs2n, hs3n, hs3m, hbm, hm2, hsum, hmid, hmo, hor, _, _, _, _⟩ := hp
  obtain ⟨h1, h2, h3⟩ := h
  obtain ⟨h1', h2', h3'⟩ := h'
  by_cases c1 : m < rv p.mid_utilization
  · obtain ⟨x, hx, hr⟩ := h1 c1
    have bx := SegContrib.bound hm0 (by omega) hmid (by omega) hx
    by_cases d1 : m' < rv p.mid_utilization
    · obtain ⟨x', hx', hr'⟩ := h1' d1
      have := SegContrib.mono (by omega) hmid hmm hx hx'
      rw [hr, hr']; exact min_le_min_right _ (by omega)
    by_cases d2 : m' < rv p.optimal_utilization
    · obtain ⟨x', hx', hr'⟩ := h2' (by omega) d2
      have := SegContrib.bound (by omega) (by omega) (by omega) (by omega) hx'
      rw [hr, hr']; exact min_le_min_right _ (by omega)
    · obtain ⟨x', hx', hr'⟩ := h3' (by omega)
      have := SegContrib.bound (by omega) (by omega) (by omega) (by omega) hx'
      rw [hr, hr']; exact min_le_min_right _ (by omega)
  by_cases c2 : m < rv p.optimal_utilization
  · obtain ⟨x, hx, hr⟩ := h2 (by omega) c2
    have bx := SegContrib.bound (by omega) (by omega) (by omega) (by omega) hx
    by_cases d2 : m' < rv p.optimal_utilization
    · obtain ⟨x', hx', hr'⟩ := h2' (by omega) d2
      have := SegContrib.mono (by omega) (by omega) (by omega : m - _ ≤ m' - _) hx hx'
      rw [hr, hr']; exact min_le_min_right _ (by omega)
    · obtain ⟨x', hx', hr'⟩ := h3' (by omega)
      have := SegContrib.bound (by omega) (by omega) (by omega) (by omega) hx'
      rw [hr, hr']; exact min_le_min_right _ (by omega)
  · obtain ⟨x, hx, hr⟩ := h3 (by omega)
    obtain ⟨x', hx', hr'⟩ := h3' (by omega)
    have := SegContrib.mono (by omega) (by omega) (by omega : m - _ ≤ m' - _) hx hx'
    rw [hr, hr']; exact min_le_min_right _ (by omega)

/-- **The annual borrow rate never decreases as utilization rises, across
segment boundaries** (fixed by #225). For every rate model passing
`InterestRateModel::verify` (`ValidRateModel`) and all utilizations
`0 ≤ u ≤ u'` (any non-negative values, including above `RAY`, which the code
clamps), both calls return (do not abort) and `rate(u) ≤ rate(u')`.

Before #225 this failed by one unit (`10^-27`) at a segment boundary
(base 0, all slopes `4·10^25 + 2`, mid `3·10^26`, optimal `8·10^26`,
max `2·RAY` gave `rate(mid − 1) = rate(mid) + 1`); with that model the theorem
now gives `rate(mid − 1) ≤ rate(mid)`. -/
theorem calculate_annual_borrow_rate_monotone (env : Env) (u u' : math.fp.Ray)
    (p : types.MarketParams) (hu : 0 ≤ rv u) (huu : rv u ≤ rv u') (hp : ValidRateModel p) :
    rates.curve.calculate_annual_borrow_rate env u p ⦃ r =>
      rates.curve.calculate_annual_borrow_rate env u' p ⦃ r' => rv r ≤ rv r' ⦄ ⦄ := by
  apply WP.spec_mono (calculate_annual_borrow_rate_spec env u p hu hp)
  intro r hr
  apply WP.spec_mono (calculate_annual_borrow_rate_spec env u' p (by omega) hp)
  intro r' hr'
  exact AnnualRate.mono hp (by omega) (min_le_min_right _ huu) (min_le_right _ _) hr hr'

/-- Pure part of the kink values: at `m = mid` the rate is `base + slope1`, at
`m = optimal` it is `base + slope1 + slope2`, and at `m = RAY` it is
`base + slope1 + slope2 + slope3`; each equals its value capped at `max`,
since `verify` requires `base + slope1 + slope2 + slope3 ≤ max`. -/
theorem AnnualRate.at_kinks {p : types.MarketParams} {m r : Int} (hp : ValidRateModel p)
    (h : AnnualRate p m r) :
    (m = rv p.mid_utilization →
      r = min (rv p.base_borrow_rate + rv p.slope1) (rv p.max_borrow_rate) ∧
      r = rv p.base_borrow_rate + rv p.slope1) ∧
    (m = rv p.optimal_utilization →
      r = min (rv p.base_borrow_rate + rv p.slope1 + rv p.slope2) (rv p.max_borrow_rate) ∧
      r = rv p.base_borrow_rate + rv p.slope1 + rv p.slope2) ∧
    (m = 10^27 →
      r = min (rv p.base_borrow_rate + rv p.slope1 + rv p.slope2 + rv p.slope3)
        (rv p.max_borrow_rate) ∧
      r = rv p.base_borrow_rate + rv p.slope1 + rv p.slope2 + rv p.slope3) := by
  obtain ⟨hb0, hbs1, hs2n, hs3n, hs3m, hbm, hm2, hsum, hmid, hmo, hor, _, _, _, _⟩ := hp
  obtain ⟨h1, h2, h3⟩ := h
  refine ⟨fun hm => ?_, fun hm => ?_, fun hm => ?_⟩
  · obtain ⟨x, hx, hr⟩ := h2 (by omega) (by omega)
    rw [show m - rv p.mid_utilization = 0 by omega] at hx
    have hx0 := SegContrib.zero (by omega) hx
    rw [hr, hx0]; constructor <;> simp only [add_zero] <;> omega
  · obtain ⟨x, hx, hr⟩ := h3 (by omega)
    rw [show m - rv p.optimal_utilization = 0 by omega] at hx
    have hx0 := SegContrib.zero (by omega) hx
    rw [hr, hx0]; constructor <;> simp only [add_zero] <;> omega
  · obtain ⟨x, hx, hr⟩ := h3 (by omega)
    rw [hm] at hx
    have hx0 := SegContrib.full (by omega) hx
    rw [hr, hx0]; constructor <;> omega

/-- **The curve hits its kinks exactly** (#225). For every rate model passing
`InterestRateModel::verify`: `rate(mid) = min(base + slope1, max) = base + slope1`,
`rate(optimal) = min(base + slope1 + slope2, max) = base + slope1 + slope2`,
and for every utilization `u ≥ RAY` (clamped to `RAY`)
`rate(u) = min(base + slope1 + slope2 + slope3, max) = base + slope1 + slope2 + slope3`.
The call never aborts. (Before #225 the double rounding could miss these by
up to `RAY/(2·range)` units.) -/
theorem calculate_annual_borrow_rate_at_kinks (env : Env) (u : math.fp.Ray)
    (p : types.MarketParams) (hu : 0 ≤ rv u) (hp : ValidRateModel p) :
    rates.curve.calculate_annual_borrow_rate env u p ⦃ r =>
      (rv u = rv p.mid_utilization →
        rv r = min (rv p.base_borrow_rate + rv p.slope1) (rv p.max_borrow_rate) ∧
        rv r = rv p.base_borrow_rate + rv p.slope1) ∧
      (rv u = rv p.optimal_utilization →
        rv r = min (rv p.base_borrow_rate + rv p.slope1 + rv p.slope2) (rv p.max_borrow_rate) ∧
        rv r = rv p.base_borrow_rate + rv p.slope1 + rv p.slope2) ∧
      (10^27 ≤ rv u →
        rv r = min (rv p.base_borrow_rate + rv p.slope1 + rv p.slope2 + rv p.slope3)
          (rv p.max_borrow_rate) ∧
        rv r = rv p.base_borrow_rate + rv p.slope1 + rv p.slope2 + rv p.slope3) ⦄ := by
  have hmid := hp.mid_lt_opt
  have hor := hp.opt_lt_ray
  apply WP.spec_mono (calculate_annual_borrow_rate_spec env u p hu hp)
  intro r hr
  obtain ⟨k1, k2, k3⟩ := AnnualRate.at_kinks hp hr
  refine ⟨fun h => k1 (by omega), fun h => k2 (by omega), fun h => k3 (by omega)⟩

end RatesAeneas.Proofs
