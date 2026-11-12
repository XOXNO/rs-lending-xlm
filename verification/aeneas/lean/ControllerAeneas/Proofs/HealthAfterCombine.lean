/-
The arithmetic core of the post-liquidation health-factor theorem: nine
rounding inequalities (each proved about the production formulas in
`HealthAfter.lean`) combine into "post-HF ≥ pre-HF − rounding". Pure
rational arithmetic, no generated code.
-/
import ControllerAeneas.Proofs.HealthAfterLegs

namespace ControllerAeneas.Proofs

/-- The collateral-side rounding allowance (in WAD units of weighted
collateral) of `ha_combine`. -/
noncomputable def haErrC (rep opb B C n SLT Ec Es Ev : ℚ) : ℚ :=
  rep * opb / (2 * (10^18)^2) + 1/2 + B * (n + Ev / (10^54 * 10^4)) / C +
    (B + 10^18) * SLT / (2 * 10^18 * 10^4) + (Es / 2 + Ec) / (10^4 * 10^54) + n

/-- **Combination lemma.** Writing `W = 10^18`, `K = 10^54`, `BPS = 10^4`:
from
1. `BPS·K·(Wc − Wc') ≤ X + Ec + n·BPS·K` (floor-weighted loss per leg),
2. `2X ≤ 2K·Y + Es` (seized value ≤ leg budget + slack),
3. `4WC·Y ≤ 4BW·Z + 2C(B + W)·SLT` (leg budget ≈ pro-rata share of `B`),
4. `K·Z ≤ K·BPS·(Wc + n) + Ev` (half-up value vs weighted floor value),
5. `2W·B ≤ 2·rep·opb + W` (half-up budget),
6. `2·Wc·W ≤ C·(2·prop + 1)` (half-up proportion),
7. `opb·prop ≤ hf·W` (the HF-preserving bonus cap),
8. `hf·D ≤ Wc·W` (floored health factor),
9. `2K·rep − Ed ≤ 2K·(D − D')` (ceiling debt shed by the repayment),
it follows that `Wc'·D ≥ Wc·D' − Wc·Ed/(2K) − D·ErrC`. -/
theorem ha_combine (Wc Wc' D D' C B rep opb prop hf X Y Z Ec Es Ev Ed n SLT : ℚ)
    (hC : 0 < C) (hD : 0 < D) (hB : 0 ≤ B) (hrep : 0 ≤ rep) (hopb : 0 ≤ opb) (hWc : 0 ≤ Wc) (hWcC : Wc ≤ C)
    (h1 : 10^4 * 10^54 * (Wc - Wc') ≤ X + Ec + n * 10^4 * 10^54)
    (h2 : 2 * X ≤ 2 * 10^54 * Y + Es)
    (h3 : 4 * 10^18 * C * Y ≤ 4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT)
    (h4 : 10^54 * Z ≤ 10^54 * 10^4 * (Wc + n) + Ev)
    (h5 : 2 * 10^18 * B ≤ 2 * rep * opb + 10^18)
    (h6 : 2 * Wc * 10^18 ≤ C * (2 * prop + 1))
    (h7 : opb * prop ≤ hf * 10^18)
    (h8 : hf * D ≤ Wc * 10^18)
    (h9 : 2 * 10^54 * rep - Ed ≤ 2 * 10^54 * (D - D')) :
    Wc * D' - Wc * Ed / (2 * 10^54) - D * haErrC rep opb B C n SLT Ec Es Ev ≤ Wc' * D := by
  -- step 3: Y ≤ B·Z/C + (B + W)·SLT/(2W)
  have hY : Y ≤ B * Z / C + (B + 10^18) * SLT / (2 * 10^18) := by
    have e : 4 * 10^18 * C * (B * Z / C + (B + 10^18) * SLT / (2 * 10^18)) =
        4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT := by field_simp; ring
    have hp : (0:ℚ) < 4 * 10^18 * C := by positivity
    by_contra hc; push_neg at hc
    have := mul_lt_mul_of_pos_left hc hp
    linarith
  -- step 4 times B/C
  have hZ : B * Z / C ≤ B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C := by
    have hz : Z ≤ 10^4 * (Wc + n) + Ev / 10^54 := by
      have e : (10:ℚ)^54 * (10^4 * (Wc + n) + Ev / 10^54) = 10^54 * 10^4 * (Wc + n) + Ev := by
        field_simp
      by_contra hc; push_neg at hc
      have := mul_lt_mul_of_pos_left hc (show (0:ℚ) < 10^54 by positivity)
      linarith
    have : B * Z ≤ B * (10^4 * (Wc + n) + Ev / 10^54) := mul_le_mul_of_nonneg_left hz hB
    have e : B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C =
        B * (10^4 * (Wc + n) + Ev / 10^54) / C := by field_simp; ring
    rw [e]; exact div_le_div_of_nonneg_right this hC.le
  -- the budget-weighted share: B·Wc/C ≤ rep·Wc/D + rep·opb/(2W²) + 1/2
  have hBW : B * Wc / C ≤ rep * Wc / D + rep * opb / (2 * (10^18)^2) + 1/2 := by
    have hu : Wc / C ≤ (2 * prop + 1) / (2 * 10^18) := by
      rw [div_le_div_iff₀ hC (by positivity)]; nlinarith
    have hu1 : Wc / C ≤ 1 := (div_le_one hC).mpr hWcC
    have hu0 : 0 ≤ Wc / C := div_nonneg hWc hC.le
    have hb : B ≤ rep * opb / 10^18 + 1/2 := by
      have e : (10:ℚ)^18 * (rep * opb / 10^18 + 1/2) * 2 = 2 * rep * opb + 10^18 := by field_simp
      nlinarith
    have hro : 0 ≤ rep * opb := mul_nonneg hrep hopb
    have k1 : B * (Wc / C) ≤ (rep * opb / 10^18) * (Wc / C) + 1/2 := by nlinarith
    have k2 : (rep * opb / 10^18) * (Wc / C) ≤ rep * Wc / D + rep * opb / (2 * (10^18)^2) := by
      have := mul_le_mul_of_nonneg_left hu (div_nonneg hro (by positivity : (0:ℚ) ≤ 10^18))
      have e1 : rep * opb / 10^18 * ((2 * prop + 1) / (2 * 10^18)) =
          rep * (opb * prop) / (10^18)^2 + rep * opb / (2 * (10^18)^2) := by field_simp
      have h7' : rep * (opb * prop) ≤ rep * (hf * 10^18) := mul_le_mul_of_nonneg_left h7 hrep
      have h8' : hf ≤ Wc * 10^18 / D := by rw [le_div_iff₀ hD]; linarith
      have h8'' : rep * (hf * 10^18) / (10^18)^2 ≤ rep * Wc / D := by
        have := mul_le_mul_of_nonneg_left h8' hrep
        have e2 : rep * (hf * 10^18) / (10^18)^2 = rep * hf / 10^18 := by field_simp
        have e3 : rep * (Wc * 10^18 / D) / 10^18 = rep * Wc / D := by field_simp
        rw [e2, ← e3]; exact div_le_div_of_nonneg_right this (by positivity)
      have : rep * (opb * prop) / (10^18)^2 ≤ rep * (hf * 10^18) / (10^18)^2 :=
        div_le_div_of_nonneg_right h7' (by positivity)
      linarith
    have e : B * Wc / C = B * (Wc / C) := by ring
    rw [e]; linarith
  -- collateral loss: Wc − Wc' ≤ rep·Wc/D + ErrC
  have hcoll : Wc - Wc' ≤ rep * Wc / D + haErrC rep opb B C n SLT Ec Es Ev := by
    unfold haErrC
    have e1 : (10:ℚ)^4 * 10^54 * (Wc - Wc') / (10^4 * 10^54) = Wc - Wc' := by field_simp
    have hX : X ≤ 10^54 * Y + Es / 2 := by linarith
    have hall : 10^4 * 10^54 * (Wc - Wc') ≤
        10^54 * (B * Z / C + (B + 10^18) * SLT / (2 * 10^18)) + Es / 2 + Ec + n * 10^4 * 10^54 := by
      nlinarith
    have hall2 : 10^4 * 10^54 * (Wc - Wc') ≤
        10^54 * (B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C +
          (B + 10^18) * SLT / (2 * 10^18)) + Es / 2 + Ec + n * 10^4 * 10^54 := by nlinarith
    have e2 : 10^54 * (B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C +
          (B + 10^18) * SLT / (2 * 10^18)) + Es / 2 + Ec + n * 10^4 * 10^54 =
        10^4 * 10^54 * (B * Wc / C + B * (n + Ev / (10^54 * 10^4)) / C +
          (B + 10^18) * SLT / (2 * 10^18 * 10^4) + (Es / 2 + Ec) / (10^4 * 10^54) + n) := by
      field_simp; ring
    rw [e2] at hall2
    have hd : Wc - Wc' ≤ B * Wc / C + B * (n + Ev / (10^54 * 10^4)) / C +
          (B + 10^18) * SLT / (2 * 10^18 * 10^4) + (Es / 2 + Ec) / (10^4 * 10^54) + n := by
      have hp : (0:ℚ) < 10^4 * 10^54 := by positivity
      by_contra hc; push_neg at hc
      have := mul_lt_mul_of_pos_left hc hp
      linarith
    linarith
  -- debt: D − rep ≥ D' − Ed/(2K)
  have hdebt : D' - Ed / (2 * 10^54) ≤ D - rep := by
    have e : 2 * 10^54 * (D' - Ed / (2 * 10^54)) = 2 * 10^54 * D' - Ed := by field_simp
    have hp : (0:ℚ) < 2 * 10^54 := by positivity
    by_contra hc; push_neg at hc
    have := mul_lt_mul_of_pos_left hc hp
    linarith
  -- final
  have hfin : Wc * (D' - Ed / (2 * 10^54)) ≤ Wc * (D - rep) := mul_le_mul_of_nonneg_left hdebt hWc
  have hc2 : D * (Wc - Wc') ≤ D * (rep * Wc / D + haErrC rep opb B C n SLT Ec Es Ev) :=
    mul_le_mul_of_nonneg_left hcoll hD.le
  have e3 : D * (rep * Wc / D) = rep * Wc := by field_simp
  have e4 : Wc * (D' - Ed / (2 * 10^54)) = Wc * D' - Wc * Ed / (2 * 10^54) := by ring
  nlinarith

/-- The part of `haErrC` that does not depend on the bonus cap. -/
noncomputable def haErrC0 (B C n SLT Ec Es Ev : ℚ) : ℚ :=
  B * (n + Ev / (10^54 * 10^4)) / C + (B + 10^18) * SLT / (2 * 10^18 * 10^4) +
    (Es / 2 + Ec) / (10^4 * 10^54) + n

/-- Collateral side alone: from inequalities 1–4 of `ha_combine`,
`Wc − Wc' ≤ B·Wc/C + ErrC0` (the weighted collateral lost is the budget's
pro-rata weighted share plus rounding). -/
theorem ha_coll_bound (Wc Wc' C B X Y Z Ec Es Ev n SLT : ℚ) (hC : 0 < C) (hB : 0 ≤ B)
    (h1 : 10^4 * 10^54 * (Wc - Wc') ≤ X + Ec + n * 10^4 * 10^54)
    (h2 : 2 * X ≤ 2 * 10^54 * Y + Es)
    (h3 : 4 * 10^18 * C * Y ≤ 4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT)
    (h4 : 10^54 * Z ≤ 10^54 * 10^4 * (Wc + n) + Ev) :
    Wc - Wc' ≤ B * Wc / C + haErrC0 B C n SLT Ec Es Ev := by
  unfold haErrC0
  have hY : Y ≤ B * Z / C + (B + 10^18) * SLT / (2 * 10^18) := by
    have e : 4 * 10^18 * C * (B * Z / C + (B + 10^18) * SLT / (2 * 10^18)) =
        4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT := by field_simp; ring
    have hp : (0:ℚ) < 4 * 10^18 * C := by positivity
    by_contra hc; push_neg at hc
    have := mul_lt_mul_of_pos_left hc hp
    linarith
  have hZ : B * Z / C ≤ B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C := by
    have hz : Z ≤ 10^4 * (Wc + n) + Ev / 10^54 := by
      have e : (10:ℚ)^54 * (10^4 * (Wc + n) + Ev / 10^54) = 10^54 * 10^4 * (Wc + n) + Ev := by
        field_simp
      by_contra hc; push_neg at hc
      have := mul_lt_mul_of_pos_left hc (show (0:ℚ) < 10^54 by positivity)
      linarith
    have : B * Z ≤ B * (10^4 * (Wc + n) + Ev / 10^54) := mul_le_mul_of_nonneg_left hz hB
    have e : B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C =
        B * (10^4 * (Wc + n) + Ev / 10^54) / C := by field_simp; ring
    rw [e]; exact div_le_div_of_nonneg_right this hC.le
  have hall2 : 10^4 * 10^54 * (Wc - Wc') ≤
      10^54 * (B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C +
        (B + 10^18) * SLT / (2 * 10^18)) + Es / 2 + Ec + n * 10^4 * 10^54 := by nlinarith
  have e2 : 10^54 * (B * Wc / C * 10^4 + B * (n * 10^4 + Ev / 10^54) / C +
        (B + 10^18) * SLT / (2 * 10^18)) + Es / 2 + Ec + n * 10^4 * 10^54 =
      10^4 * 10^54 * (B * Wc / C + B * (n + Ev / (10^54 * 10^4)) / C +
        (B + 10^18) * SLT / (2 * 10^18 * 10^4) + (Es / 2 + Ec) / (10^4 * 10^54) + n) := by
    field_simp; ring
  rw [e2] at hall2
  have hp : (0:ℚ) < 10^4 * 10^54 := by positivity
  by_contra hc; push_neg at hc
  have := mul_lt_mul_of_pos_left hc hp
  linarith

/-- **Combination lemma, curve-target branch.** With inequalities 1–6 and 9
of `ha_combine`, `δ` the code's seizure rate (`2·opb·prop < 2·δ·W + W`,
from `δ = huD(prop·opb, W)`), the estimate's target property for a quote `r`
(`HitsTarget`: `2T(D − r) − (T − δ) − W < 2(W·Wc − δ·r)`, `δ < T`) and a
repayment `rep ≥ r`, the post-liquidation weighted collateral reaches the
target: `T·D' − ErrT ≤ W·Wc'` with
`ErrT = T·Ed/(2K) + (T − δ + W)/2 + rep/2 + W·ErrC`. -/
theorem ha_combine_target (Wc Wc' D D' C B rep opb prop δ T r X Y Z Ec Es Ev Ed n SLT : ℚ)
    (hC : 0 < C) (hB : 0 ≤ B) (hrep : 0 ≤ rep) (hopb : 0 ≤ opb) (hWc : 0 ≤ Wc) (hWcC : Wc ≤ C)
    (hT0 : 0 ≤ T)
    (h1 : 10^4 * 10^54 * (Wc - Wc') ≤ X + Ec + n * 10^4 * 10^54)
    (h2 : 2 * X ≤ 2 * 10^54 * Y + Es)
    (h3 : 4 * 10^18 * C * Y ≤ 4 * B * 10^18 * Z + 2 * C * (B + 10^18) * SLT)
    (h4 : 10^54 * Z ≤ 10^54 * 10^4 * (Wc + n) + Ev)
    (h5 : 2 * 10^18 * B ≤ 2 * rep * opb + 10^18)
    (h6 : 2 * Wc * 10^18 ≤ C * (2 * prop + 1))
    (h7 : 2 * (opb * prop) < 2 * δ * 10^18 + 10^18)
    (hδT : δ < T) (hrr : r ≤ rep)
    (hT : 2 * (T * (D - r)) - (T - δ) - 10^18 < 2 * (10^18 * Wc - δ * r))
    (h9 : 2 * 10^54 * rep - Ed ≤ 2 * 10^54 * (D - D')) :
    T * D' - (T * Ed / (2 * 10^54) + (T - δ + 10^18) / 2 + rep / 2 +
      10^18 * haErrC rep opb B C n SLT Ec Es Ev) ≤ 10^18 * Wc' := by
  have hc := ha_coll_bound Wc Wc' C B X Y Z Ec Es Ev n SLT hC hB h1 h2 h3 h4
  have hErr : haErrC rep opb B C n SLT Ec Es Ev =
      rep * opb / (2 * (10^18)^2) + 1/2 + haErrC0 B C n SLT Ec Es Ev := by
    unfold haErrC haErrC0; ring
  -- B·Wc/C ≤ rep·(δ + 1/2)/W + rep·opb/(2W²) + 1/2
  have hu : Wc / C ≤ (2 * prop + 1) / (2 * 10^18) := by
    rw [div_le_div_iff₀ hC (by positivity)]; nlinarith
  have hu1 : Wc / C ≤ 1 := (div_le_one hC).mpr hWcC
  have hu0 : 0 ≤ Wc / C := div_nonneg hWc hC.le
  have hb : B ≤ rep * opb / 10^18 + 1/2 := by
    have e : (10:ℚ)^18 * (rep * opb / 10^18 + 1/2) * 2 = 2 * rep * opb + 10^18 := by field_simp
    nlinarith
  have hro : 0 ≤ rep * opb := mul_nonneg hrep hopb
  have k1 : B * (Wc / C) ≤ (rep * opb / 10^18) * (Wc / C) + 1/2 := by nlinarith
  have k2 : (rep * opb / 10^18) * (Wc / C) ≤ rep * (δ + 1/2) / 10^18 + rep * opb / (2 * (10^18)^2) := by
    have := mul_le_mul_of_nonneg_left hu (div_nonneg hro (by positivity : (0:ℚ) ≤ 10^18))
    have e1 : rep * opb / 10^18 * ((2 * prop + 1) / (2 * 10^18)) =
        rep * (opb * prop) / (10^18)^2 + rep * opb / (2 * (10^18)^2) := by field_simp
    have h7' : rep * (opb * prop) ≤ rep * ((δ + 1/2) * 10^18) :=
      mul_le_mul_of_nonneg_left (by linarith) hrep
    have : rep * (opb * prop) / (10^18)^2 ≤ rep * ((δ + 1/2) * 10^18) / (10^18)^2 :=
      div_le_div_of_nonneg_right h7' (by positivity)
    have e2 : rep * ((δ + 1/2) * 10^18) / (10^18)^2 = rep * (δ + 1/2) / 10^18 := by field_simp
    linarith
  have e : B * Wc / C = B * (Wc / C) := by ring
  -- W·(Wc − Wc') ≤ rep·δ + rep/2 + W·ErrC
  have hW : 10^18 * (Wc - Wc') ≤ rep * δ + rep / 2 + 10^18 * haErrC rep opb B C n SLT Ec Es Ev := by
    rw [hErr]
    have : Wc - Wc' ≤ rep * (δ + 1/2) / 10^18 + rep * opb / (2 * (10^18)^2) + 1/2 +
        haErrC0 B C n SLT Ec Es Ev := by linarith
    have e3 : (10:ℚ)^18 * (rep * (δ + 1/2) / 10^18 + rep * opb / (2 * (10^18)^2) + 1/2 +
        haErrC0 B C n SLT Ec Es Ev) = rep * δ + rep / 2 +
          10^18 * (rep * opb / (2 * (10^18)^2) + 1/2 + haErrC0 B C n SLT Ec Es Ev) := by
      field_simp; ring
    nlinarith
  -- debt
  have hdebt : D' - Ed / (2 * 10^54) ≤ D - rep := by
    have e : 2 * 10^54 * (D' - Ed / (2 * 10^54)) = 2 * 10^54 * D' - Ed := by field_simp
    have hp : (0:ℚ) < 2 * 10^54 := by positivity
    by_contra hc; push_neg at hc
    have := mul_lt_mul_of_pos_left hc hp
    linarith
  have hTd : T * (D' - Ed / (2 * 10^54)) ≤ T * (D - rep) := mul_le_mul_of_nonneg_left hdebt hT0
  -- target: W·Wc − δ·rep ≥ T·(D − rep) − (T − δ + W)/2 (extra repayment only helps, δ < T)
  have hx : (T - δ) * (rep - r) ≥ 0 := mul_nonneg (by linarith) (by linarith)
  have e4 : T * (D' - Ed / (2 * 10^54)) = T * D' - T * Ed / (2 * 10^54) := by ring
  nlinarith

end ControllerAeneas.Proofs
