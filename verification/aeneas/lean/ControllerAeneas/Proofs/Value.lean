/-
`common/src/rates/value.rs`: the USD value of a scaled position,
`scaled * index` (RAY) rescaled to WAD and multiplied by the price (WAD), with
three rounding modes:

* `position_value`       half-up at every step (unweighted collateral, debt
                         sums used for display and `sum_debt_usd`);
* `position_value_floor` floor at every step (collateral used by the risk
                         gates: LTV and liquidation-threshold weighting);
* `position_value_ceil`  ceiling at every step (debt used by the risk gates).

INV-RISK-02 (conservative valuation) needs the floor version never to exceed
the half-up one, which never exceeds the ceiling one, and the floor/ceiling
versions to under/over-estimate the exact rational value. Both are proved
here for every non-negative input whose ceiling chain fits `i128` (no other
bound, unlike the Certora and Kani harnesses which bound the inputs).

Notation in the statements: `R = 10^27` (RAY), `B = 10^9` (the RAY→WAD
rescale factor `10^(27-18)`), `W = 10^18` (WAD).
-/
import ControllerAeneas.Proofs.FloorCeilRescale
import ControllerAeneas.Proofs.HalfUpDivInt
import ControllerAeneas.Proofs.Rounding

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## Constants -/

theorem RAY_val : IScalar.val (ty := .I128) constants.production.shared.RAY = 10^27 := by
  unfold constants.production.shared.RAY; simp

theorem WAD_val : IScalar.val (ty := .I128) constants.production.shared.WAD = 10^18 := by
  unfold constants.production.shared.WAD; simp

theorem BPS_val : IScalar.val (ty := .I128) constants.production.shared.BPS = 10^4 := by
  unfold constants.production.shared.BPS; simp

theorem RAY_DECIMALS_eq : constants.production.shared.RAY_DECIMALS = 27#u32 := by
  unfold constants.production.shared.RAY_DECIMALS; rfl

theorem WAD_DECIMALS_eq : constants.production.shared.WAD_DECIMALS = 18#u32 := by
  unfold constants.production.shared.WAD_DECIMALS; rfl

/-! ## RAY → WAD conversions -/

/-- `Ray::to_wad_floor` of a non-negative value is `⌊a / 10^9⌋`; never aborts. -/
theorem to_wad_floor_spec (env : Env) (a : math.fp.Ray) (ha : 0 ≤ a.val) :
    math.fp.Ray.to_wad_floor a env ⦃ r => IsFloor a.val (10^9) r.val ⦄ := by
  unfold math.fp.Ray.to_wad_floor
  rw [RAY_DECIMALS_eq, WAD_DECIMALS_eq]
  step with rescale_floor_down_spec env a 27#u32 18#u32 (by simp) as ⟨r, hr, _⟩
  simpa using hr ha

/-- `Ray::to_wad_ceil` is `⌈a / 10^9⌉` for every value; never aborts. -/
theorem to_wad_ceil_spec (env : Env) (a : math.fp.Ray) :
    math.fp.Ray.to_wad_ceil a env ⦃ r => IsCeil a.val (10^9) r.val ⦄ := by
  unfold math.fp.Ray.to_wad_ceil
  rw [RAY_DECIMALS_eq, WAD_DECIMALS_eq]
  step with rescale_ceil_down_spec env a 27#u32 18#u32 (by simp) as ⟨r, hr⟩
  simpa using hr

/-- `Ray::to_wad` of a non-negative value is `a / 10^9` rounded half up;
never aborts. -/
theorem to_wad_spec (env : Env) (a : math.fp.Ray) (ha : 0 ≤ a.val) :
    math.fp.Ray.to_wad a env ⦃ r => IsHalfUp a.val (10^9) r.val ⦄ := by
  unfold math.fp.Ray.to_wad math.fp_core.rescale_half_up
  rw [RAY_DECIMALS_eq, WAD_DECIMALS_eq,
    rescale_down_eq _ env a 27#u32 18#u32 env (by simp)]
  have h9 : ((27#u32 : Std.U32).val - (18#u32 : Std.U32).val) = 9 := by simp
  rw [h9]
  have hp := IScalar.tryMkOpt_eq .I128 ((10 : Int) ^ 9)
  cases hpo : IScalar.tryMkOpt .I128 ((10 : Int) ^ 9) with
  | none =>
    exfalso; rw [hpo] at hp; apply hp
    simp [IScalar.inBounds, IScalarTy.numBits]
  | some f =>
    rw [hpo] at hp; obtain ⟨hfv, _⟩ := hp
    simp only [math.fp_core.rescale_half_up.closure.Insts.CoreOpsFunctionFnPairI128OptionI128I128.call]
    step with div_by_int_half_up_spec env a f (by rw [hfv]; norm_num) as ⟨q, hq⟩
    rw [hfv] at hq
    unfold IsHalfAway at hq
    rwa [if_pos ha] at hq

/-! ## The rounding chains -/

/-- The ceiling chain is the largest; its two quantities that must fit `i128`
(the first and last step) do, under the domain hypotheses of the value
theorems. -/
theorem ceil_chain_bounds (R B W M s i p c1 c2 c3 : Int) (hR : 0 < R) (hB : 1 ≤ B) (hW : 0 < W)
    (hp : 0 ≤ p) (hc1 : IsCeil (s * i) R c1) (hc2 : IsCeil c1 B c2)
    (hc3 : IsCeil (c2 * p) W c3) (hA : s * i + R ≤ M * R)
    (hBnd : (s * i + 2 * R * B) * p + W * R * B ≤ M * (W * R * B)) :
    c1 < M ∧ c3 < M := by
  rw [isCeil_pos hR] at hc1
  rw [isCeil_pos (by omega)] at hc2
  rw [isCeil_pos hW] at hc3
  constructor
  · by_contra h
    have : M * R ≤ c1 * R := mul_le_mul_of_nonneg_right (by omega) (by omega)
    linarith [hc1.2]
  · have hRB : R ≤ R * B := by nlinarith
    have h2 : c2 * B * R < c1 * R + B * R := by nlinarith [hc2.2]
    have h3 : c2 * B * R < s * i + 2 * R * B := by nlinarith [hc1.2]
    have h4 : c2 * B * R * p ≤ (s * i + 2 * R * B) * p :=
      mul_le_mul_of_nonneg_right h3.le hp
    have hWRB : 0 < W * R * B := by positivity
    have h5 : c3 * (W * R * B) < (c2 * p + W) * (R * B) := by
      have := mul_lt_mul_of_pos_right hc3.2 (show 0 < R * B by positivity)
      linarith
    by_contra h
    have : M * (W * R * B) ≤ c3 * (W * R * B) := mul_le_mul_of_nonneg_right (by omega) hWRB.le
    nlinarith

/-- **`position_value_floor` is the exact floor chain.** For non-negative
`scaled`, `index` and `price` satisfying the domain bound, the call returns
(does not abort) `⌊⌊⌊scaled·index / R⌋ / B⌋ · price / W⌋`.

Hypotheses: all three inputs are non-negative (they are `Ray`/`Wad` values:
scaled balances, indexes ≥ their floors, oracle prices > 0); the domain bound
`scaled·index + R ≤ 2^127·R` and
`(scaled·index + 2RB)·price + WRB ≤ 2^127·WRB` (the exact value plus the
rounding slack fits `i128`; production positions are many orders of magnitude
below this). -/
theorem position_value_floor_spec (env : Env) (s i p : Std.I128)
    (hs : 0 ≤ s.val) (hi : 0 ≤ i.val) (hp : 0 ≤ p.val)
    (hA : s.val * i.val + 10^27 ≤ 2^127 * 10^27)
    (hB : (s.val * i.val + 2 * 10^27 * 10^9) * p.val + 10^18 * 10^27 * 10^9
      ≤ 2^127 * (10^18 * 10^27 * 10^9)) :
    rates.value.position_value_floor env s i p ⦃ v =>
      ∃ f1 f2, IsFloor (s.val * i.val) (10^27) f1 ∧ IsFloor f1 (10^9) f2 ∧
        IsFloor (f2 * p.val) (10^18) v.val ⦄ := by
  have hsi : 0 ≤ s.val * i.val := mul_nonneg hs hi
  obtain ⟨c1, hc1⟩ := isCeil_exists (s.val * i.val) (10^27) (by norm_num)
  obtain ⟨c2, hc2⟩ := isCeil_exists c1 (10^9) (by norm_num)
  obtain ⟨c3, hc3⟩ := isCeil_exists (c2 * p.val) (10^18) (by norm_num)
  obtain ⟨hc1M, hc3M⟩ := ceil_chain_bounds _ _ _ _ _ _ _ c1 c2 c3 (by norm_num) (by norm_num)
    (by norm_num) hp hc1 hc2 hc3 hA hB
  have hc1' := (isCeil_pos (by norm_num)).mp hc1
  have hc2' := (isCeil_pos (by norm_num)).mp hc2
  obtain ⟨q1, hq1⟩ := isFloor_exists (s.val * i.val) (10^27) (by norm_num)
  unfold rates.value.position_value_floor math.fp.Ray.mul_floor
  step with mul_div_floor_spec env s i constants.production.shared.RAY
    (by rw [RAY_val]; norm_num)
    ⟨q1, by rw [RAY_val]; exact hq1,
      by have := IsFloor.nonneg_of (by norm_num) hq1 hsi; omega,
      by have := IsFloor.le_of (by norm_num) hq1 hc1'.1; omega⟩ as ⟨a1, ha1⟩
  rw [RAY_val] at ha1
  have ha1n : 0 ≤ a1.val := IsFloor.nonneg_of (by norm_num) ha1 hsi
  have ha1c : a1.val ≤ c1 := IsFloor.le_of (by norm_num) ha1 hc1'.1
  try simp only [bind_ok]
  step with to_wad_floor_spec env a1 ha1n as ⟨a2, ha2⟩
  have ha2n : 0 ≤ a2.val := IsFloor.nonneg_of (by norm_num) ha2 ha1n
  have ha2c : a2.val ≤ c2 := IsFloor.le_of (by norm_num) ha2 (by linarith [hc2'.1])
  have hc3' := (isCeil_pos (by norm_num)).mp hc3
  have hpc : a2.val * p.val ≤ c3 * 10^18 :=
    le_trans (mul_le_mul_of_nonneg_right ha2c hp) hc3'.1
  obtain ⟨q3, hq3⟩ := isFloor_exists (a2.val * p.val) (10^18) (by norm_num)
  unfold math.fp.Wad.mul_floor
  step with mul_div_floor_spec env a2 p constants.production.shared.WAD
    (by rw [WAD_val]; norm_num)
    ⟨q3, by rw [WAD_val]; exact hq3,
      by have := IsFloor.nonneg_of (by norm_num) hq3 (mul_nonneg ha2n hp); omega,
      by have := IsFloor.le_of (by norm_num) hq3 hpc; omega⟩ as ⟨v, hv⟩
  rw [WAD_val] at hv
  exact ⟨a1.val, a2.val, ha1, ha2, hv⟩

/-- **`position_value` is the exact half-up chain.** Same hypotheses as
`position_value_floor_spec`; the call returns (does not abort)
`hu(hu(hu(scaled·index / R) / B) · price / W)`, `hu` = rounding to nearest
with exact halves up. -/
theorem position_value_spec (env : Env) (s i p : Std.I128)
    (hs : 0 ≤ s.val) (hi : 0 ≤ i.val) (hp : 0 ≤ p.val)
    (hA : s.val * i.val + 10^27 ≤ 2^127 * 10^27)
    (hB : (s.val * i.val + 2 * 10^27 * 10^9) * p.val + 10^18 * 10^27 * 10^9
      ≤ 2^127 * (10^18 * 10^27 * 10^9)) :
    rates.value.position_value env s i p ⦃ v =>
      ∃ h1 h2, IsHalfUp (s.val * i.val) (10^27) h1 ∧ IsHalfUp h1 (10^9) h2 ∧
        IsHalfUp (h2 * p.val) (10^18) v.val ⦄ := by
  have hsi : 0 ≤ s.val * i.val := mul_nonneg hs hi
  obtain ⟨c1, hc1⟩ := isCeil_exists (s.val * i.val) (10^27) (by norm_num)
  obtain ⟨c2, hc2⟩ := isCeil_exists c1 (10^9) (by norm_num)
  obtain ⟨c3, hc3⟩ := isCeil_exists (c2 * p.val) (10^18) (by norm_num)
  obtain ⟨hc1M, hc3M⟩ := ceil_chain_bounds _ _ _ _ _ _ _ c1 c2 c3 (by norm_num) (by norm_num)
    (by norm_num) hp hc1 hc2 hc3 hA hB
  have hc1' := (isCeil_pos (by norm_num)).mp hc1
  have hc2' := (isCeil_pos (by norm_num)).mp hc2
  have hc3' := (isCeil_pos (by norm_num)).mp hc3
  obtain ⟨q1, hq1⟩ := IsHalfUp.exists (s.val * i.val) (10^27) (by norm_num)
  unfold rates.value.position_value math.fp.Ray.mul
  step with mul_div_half_up_spec env s i constants.production.shared.RAY hs hi
    (by rw [RAY_val]; norm_num)
    ⟨q1, by rw [RAY_val]; exact hq1,
      by have := IsHalfUp.le_of (by norm_num) hq1 hc1'.1; omega⟩ as ⟨a1, ha1⟩
  rw [RAY_val] at ha1
  have ha1n : 0 ≤ a1.val := IsHalfUp.nonneg (by norm_num) hsi ha1
  have ha1c : a1.val ≤ c1 := IsHalfUp.le_of (by norm_num) ha1 hc1'.1
  try simp only [bind_ok]
  step with to_wad_spec env a1 ha1n as ⟨a2, ha2⟩
  have ha2n : 0 ≤ a2.val := IsHalfUp.nonneg (by norm_num) ha1n ha2
  have ha2c : a2.val ≤ c2 := IsHalfUp.le_of (by norm_num) ha2 (by linarith [hc2'.1])
  have hpc : a2.val * p.val ≤ c3 * 10^18 :=
    le_trans (mul_le_mul_of_nonneg_right ha2c hp) hc3'.1
  obtain ⟨q3, hq3⟩ := IsHalfUp.exists (a2.val * p.val) (10^18) (by norm_num)
  unfold math.fp.Wad.mul
  step with mul_div_half_up_spec env a2 p constants.production.shared.WAD ha2n hp
    (by rw [WAD_val]; norm_num)
    ⟨q3, by rw [WAD_val]; exact hq3,
      by have := IsHalfUp.le_of (by norm_num) hq3 hpc; omega⟩ as ⟨v, hv⟩
  rw [WAD_val] at hv
  exact ⟨a1.val, a2.val, ha1, ha2, hv⟩

/-- **`position_value_ceil` is the exact ceiling chain.** Same hypotheses as
`position_value_floor_spec`; the call returns (does not abort)
`⌈⌈⌈scaled·index / R⌉ / B⌉ · price / W⌉`. -/
theorem position_value_ceil_spec (env : Env) (s i p : Std.I128)
    (hs : 0 ≤ s.val) (hi : 0 ≤ i.val) (hp : 0 ≤ p.val)
    (hA : s.val * i.val + 10^27 ≤ 2^127 * 10^27)
    (hB : (s.val * i.val + 2 * 10^27 * 10^9) * p.val + 10^18 * 10^27 * 10^9
      ≤ 2^127 * (10^18 * 10^27 * 10^9)) :
    rates.value.position_value_ceil env s i p ⦃ v =>
      ∃ c1 c2, IsCeil (s.val * i.val) (10^27) c1 ∧ IsCeil c1 (10^9) c2 ∧
        IsCeil (c2 * p.val) (10^18) v.val ⦄ := by
  have hsi : 0 ≤ s.val * i.val := mul_nonneg hs hi
  obtain ⟨c1, hc1⟩ := isCeil_exists (s.val * i.val) (10^27) (by norm_num)
  obtain ⟨c2, hc2⟩ := isCeil_exists c1 (10^9) (by norm_num)
  obtain ⟨c3, hc3⟩ := isCeil_exists (c2 * p.val) (10^18) (by norm_num)
  obtain ⟨hc1M, hc3M⟩ := ceil_chain_bounds _ _ _ _ _ _ _ c1 c2 c3 (by norm_num) (by norm_num)
    (by norm_num) hp hc1 hc2 hc3 hA hB
  have hc1n : 0 ≤ c1 := IsCeil.ge_of (by norm_num) hc1 (by simpa using hsi)
  have hc2n : 0 ≤ c2 := IsCeil.ge_of (by norm_num) hc2 (by simpa using hc1n)
  have hc3n : 0 ≤ c3 := IsCeil.ge_of (by norm_num) hc3 (by simpa using mul_nonneg hc2n hp)
  unfold rates.value.position_value_ceil math.fp.Ray.mul_ceil
  step with mul_div_ceil_spec env s i constants.production.shared.RAY
    (by rw [RAY_val]; norm_num)
    ⟨c1, by rw [RAY_val]; exact hc1, by omega, hc1M⟩ as ⟨a1, ha1⟩
  rw [RAY_val] at ha1
  have ha1e : a1.val = c1 := IsCeil.unique (by norm_num) ha1 hc1
  try simp only [bind_ok]
  step with to_wad_ceil_spec env a1 as ⟨a2, ha2⟩
  rw [ha1e] at ha2
  have ha2e : a2.val = c2 := IsCeil.unique (by norm_num) ha2 hc2
  unfold math.fp.Wad.mul_ceil
  step with mul_div_ceil_spec env a2 p constants.production.shared.WAD
    (by rw [WAD_val]; norm_num)
    ⟨c3, by rw [WAD_val, ha2e]; exact hc3, by omega, hc3M⟩ as ⟨v, hv⟩
  rw [WAD_val] at hv
  exact ⟨c1, a2.val, hc1, by rw [ha2e]; exact hc2, hv⟩

/-! ## Headline: conservative valuation (INV-RISK-02) -/

/-- **Floor ≤ half-up ≤ ceiling (INV-RISK-02).** For every non-negative
scaled balance, index and price satisfying the domain bound of
`position_value_floor_spec`, all three valuations return (none aborts) and
`position_value_floor ≤ position_value ≤ position_value_ceil`.

So the collateral the risk gates use (floor) is never more than the reported
collateral (half-up), and the debt they use (ceiling) is never less than the
reported debt. -/
theorem position_value_bracket (env : Env) (s i p : Std.I128)
    (hs : 0 ≤ s.val) (hi : 0 ≤ i.val) (hp : 0 ≤ p.val)
    (hA : s.val * i.val + 10^27 ≤ 2^127 * 10^27)
    (hB : (s.val * i.val + 2 * 10^27 * 10^9) * p.val + 10^18 * 10^27 * 10^9
      ≤ 2^127 * (10^18 * 10^27 * 10^9)) :
    ∃ vf v vc,
      rates.value.position_value_floor env s i p = ok vf ∧
      rates.value.position_value env s i p = ok v ∧
      rates.value.position_value_ceil env s i p = ok vc ∧
      vf.val ≤ v.val ∧ v.val ≤ vc.val := by
  obtain ⟨vf, hvf, f1, f2, hf1, hf2, hf3⟩ :=
    spec_imp_eq_ok (position_value_floor_spec env s i p hs hi hp hA hB)
  obtain ⟨v, hv, h1, h2, hh1, hh2, hh3⟩ :=
    spec_imp_eq_ok (position_value_spec env s i p hs hi hp hA hB)
  obtain ⟨vc, hvc, c1, c2, hc1, hc2, hc3⟩ :=
    spec_imp_eq_ok (position_value_ceil_spec env s i p hs hi hp hA hB)
  refine ⟨vf, v, vc, hvf, hv, hvc, ?_, ?_⟩
  · rw [isFloor_pos (by norm_num)] at hf1 hf2 hf3
    have e1 : f1 ≤ h1 := IsHalfUp.ge_of (by norm_num) hh1 hf1.1
    have e2 : f2 ≤ h2 := IsHalfUp.ge_of (by norm_num) hh2 (by linarith [hf2.1])
    exact IsHalfUp.ge_of (by norm_num) hh3
      (le_trans hf3.1 (mul_le_mul_of_nonneg_right e2 hp))
  · rw [isCeil_pos (by norm_num)] at hc1 hc2 hc3
    have e1 : h1 ≤ c1 := IsHalfUp.le_of (by norm_num) hh1 hc1.1
    have e2 : h2 ≤ c2 := IsHalfUp.le_of (by norm_num) hh2 (by linarith [hc2.1])
    exact IsHalfUp.le_of (by norm_num) hh3
      (le_trans (mul_le_mul_of_nonneg_right e2 hp) hc3.1)

/-- **Floor and ceiling bracket the exact value (INV-RISK-02).** Under the
same hypotheses, with `D = R·B·W = 10^54` the total scale,
`position_value_floor · D ≤ scaled · index · price ≤ position_value_ceil · D`:
the floor valuation never overstates the exact rational value
`scaled·index·price / D` and the ceiling valuation never understates it, so
no rounding step can make collateral look larger or debt look smaller. -/
theorem position_value_floor_ceil_exact (env : Env) (s i p : Std.I128)
    (hs : 0 ≤ s.val) (hi : 0 ≤ i.val) (hp : 0 ≤ p.val)
    (hA : s.val * i.val + 10^27 ≤ 2^127 * 10^27)
    (hB : (s.val * i.val + 2 * 10^27 * 10^9) * p.val + 10^18 * 10^27 * 10^9
      ≤ 2^127 * (10^18 * 10^27 * 10^9)) :
    ∃ vf vc,
      rates.value.position_value_floor env s i p = ok vf ∧
      rates.value.position_value_ceil env s i p = ok vc ∧
      vf.val * (10^27 * 10^9 * 10^18) ≤ s.val * i.val * p.val ∧
      s.val * i.val * p.val ≤ vc.val * (10^27 * 10^9 * 10^18) := by
  obtain ⟨vf, hvf, f1, f2, hf1, hf2, hf3⟩ :=
    spec_imp_eq_ok (position_value_floor_spec env s i p hs hi hp hA hB)
  obtain ⟨vc, hvc, c1, c2, hc1, hc2, hc3⟩ :=
    spec_imp_eq_ok (position_value_ceil_spec env s i p hs hi hp hA hB)
  refine ⟨vf, vc, hvf, hvc, ?_, ?_⟩
  · rw [isFloor_pos (by norm_num)] at hf1 hf2 hf3
    have a : f2 * 10^9 * 10^27 ≤ f1 * 10^27 := by nlinarith [hf2.1]
    have b : f2 * 10^9 * 10^27 * p.val ≤ s.val * i.val * p.val :=
      mul_le_mul_of_nonneg_right (by linarith [hf1.1]) hp
    have c : vf.val * 10^18 * (10^9 * 10^27) ≤ f2 * p.val * (10^9 * 10^27) :=
      mul_le_mul_of_nonneg_right hf3.1 (by norm_num)
    nlinarith
  · rw [isCeil_pos (by norm_num)] at hc1 hc2 hc3
    have a : c1 * 10^27 ≤ c2 * 10^9 * 10^27 := by nlinarith [hc2.1]
    have b : s.val * i.val * p.val ≤ c2 * 10^9 * 10^27 * p.val :=
      mul_le_mul_of_nonneg_right (by linarith [hc1.1]) hp
    have c : c2 * p.val * (10^9 * 10^27) ≤ vc.val * 10^18 * (10^9 * 10^27) :=
      mul_le_mul_of_nonneg_right hc3.1 (by norm_num)
    nlinarith

end ControllerAeneas.Proofs

namespace ControllerAeneas.Proofs

/-- **`Bps::apply_to_wad_floor` is the exact floor of `value·rate/BPS`.**
For a rate `0 ≤ rate ≤ BPS` (LTV and liquidation thresholds are at most
100%, validated on every asset config) and a value `0 ≤ v` (WAD), the call returns (does not
abort) `⌊v·rate / BPS⌋`. The intermediate `Bps::to_wad` is exact
(`rate·10^14`, since `BPS` divides `WAD`), so there is a single rounding,
downward. This is the per-leg risk weighting of
`risk::totals::calculate_ltv_collateral_wad` and
`calculate_account_risk_totals` (LTV and liquidation-threshold collateral):
weighted collateral never rounds up (INV-RISK-02). -/
theorem apply_to_wad_floor_spec (env : Env) (rate : math.fp.Bps) (v : math.fp.Wad)
    (hr : 0 ≤ rate.val) (hrB : rate.val ≤ 10^4) (hv : 0 ≤ v.val) :
    math.fp.Bps.apply_to_wad_floor rate env v ⦃ r => IsFloor (v.val * rate.val) (10^4) r.val ⦄ := by
  have hvb := v.hBounds
  simp only [IScalarTy.numBits] at hvb
  have hfit : v.val * rate.val < 2^127 * 10^4 := by
    have : v.val * rate.val ≤ v.val * 10^4 := mul_le_mul_of_nonneg_left hrB hv
    norm_num at hvb ⊢; nlinarith
  unfold math.fp.Bps.apply_to_wad_floor math.fp.Bps.to_wad
  have hex : IsHalfUp (rate.val * 10^18) (10^4) (rate.val * 10^14) := by
    have := IsHalfUp.mul_self (rate.val * 10^14) (10^4) (by norm_num)
    rw [show rate.val * 10^14 * 10^4 = rate.val * 10^18 by ring] at this; exact this
  step with mul_div_half_up_spec env rate constants.production.shared.WAD
    constants.production.shared.BPS hr (by rw [WAD_val]; norm_num) (by rw [BPS_val]; norm_num)
    ⟨rate.val * 10^14, by rw [WAD_val, BPS_val]; exact hex, by
      nlinarith⟩ as ⟨w, hw⟩
  rw [WAD_val, BPS_val] at hw
  have hwe : w.val = rate.val * 10^14 := IsHalfUp.unique (by norm_num) hw hex
  try simp only [bind_ok]
  unfold math.fp.Wad.mul_floor
  obtain ⟨q, hq⟩ := isFloor_exists (v.val * rate.val) (10^4) (by norm_num)
  have hq' := (isFloor_pos (by norm_num)).mp hq
  have hconv : ∀ x, IsFloor (v.val * rate.val) (10^4) x ↔
      IsFloor (v.val * w.val) (10^18) x := by
    intro x
    rw [isFloor_pos (by norm_num), isFloor_pos (by norm_num), hwe]
    constructor <;> rintro ⟨h1, h2⟩ <;> constructor <;> nlinarith
  have hq0 : 0 ≤ q := IsFloor.nonneg_of (by norm_num) hq (mul_nonneg hv hr)
  have hqM : q < 2^127 := by
    by_contra h
    have : 2^127 * 10^4 ≤ q * 10^4 := mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    linarith [hq'.1]
  step with mul_div_floor_spec env v w constants.production.shared.WAD
    (by rw [WAD_val]; norm_num) ⟨q, by rw [WAD_val]; exact (hconv q).mp hq, by omega, hqM⟩
    as ⟨r, hr'⟩
  rw [WAD_val] at hr'
  exact (hconv r.val).mpr hr'

end ControllerAeneas.Proofs
