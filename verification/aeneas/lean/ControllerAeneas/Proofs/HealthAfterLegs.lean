/-
Per-leg rounding facts for the health factor after a liquidation: how far
the three valuation chains of `common/src/rates/value.rs` (floor `fv`,
half-up `huv`, ceiling `cv`, `Totals.lean`) can be from the exact value
`s·i·p` (scale `K = R·10^9·W = 10^54`), how much threshold-weighted floor
collateral a leg loses when `σ` shares are removed, and how much ceiling
debt a leg sheds when `a` tokens are repaid through `resolve_repay`.

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.SeizeBudget

open FixedPoint.Spec FixedPoint.HalfUpSpec

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## Distance of the chains from the exact value -/

/-- The floor chain never overstates: `fv·K ≤ s·i·p`. -/
theorem ha_fv_le (s i p : Int) (hsi : 0 ≤ s * i) (hp : 0 ≤ p) :
    fv s i p * 10^54 ≤ s * i * p := by
  unfold fv
  have h1 := sz_div_bounds (s * i) (10^27) (by norm_num)
  have h2 := sz_div_bounds (s * i / 10^27) (10^9) (by norm_num)
  have h3 := sz_div_bounds (s * i / 10^27 / 10^9 * p) (10^18) (by norm_num)
  have h20 : 0 ≤ s * i / 10^27 / 10^9 := sz_div_nonneg _ _ (by norm_num) (sz_div_nonneg _ _ (by norm_num) hsi)
  have a1 : s * i / 10^27 / 10^9 * 10^9 * 10^27 ≤ s * i := by nlinarith
  have a2 : s * i / 10^27 / 10^9 * 10^36 * p ≤ s * i * p := by nlinarith
  nlinarith

/-- The floor chain understates by less than `1 + 2·p/10^18` WAD units:
`s·i·p < fv·K + K + 2·10^36·p`. -/
theorem ha_fv_ge (s i p : Int) (hsi : 0 ≤ s * i) (hp : 0 ≤ p) :
    s * i * p < fv s i p * 10^54 + 10^54 + 2 * 10^36 * p := by
  unfold fv
  have h1 := sz_div_bounds (s * i) (10^27) (by norm_num)
  have h2 := sz_div_bounds (s * i / 10^27) (10^9) (by norm_num)
  have h3 := sz_div_bounds (s * i / 10^27 / 10^9 * p) (10^18) (by norm_num)
  have a1 : s * i < (s * i / 10^27 / 10^9) * 10^36 + 10^36 + 10^27 := by nlinarith
  have a2 : s * i * p ≤ ((s * i / 10^27 / 10^9) * 10^36 + 10^36 + 10^27) * p :=
    mul_le_mul_of_nonneg_right a1.le hp
  nlinarith

/-- The ceiling chain never understates: `s·i·p ≤ cv·K`. -/
theorem ha_cv_ge (s i p : Int) (hsi : 0 ≤ s * i) (hp : 0 ≤ p) :
    s * i * p ≤ cv s i p * 10^54 := by
  unfold cv
  have h1 := sz_clD_bounds (s * i) (10^27) (by norm_num)
  have h2 := sz_clD_bounds (clD (s * i) (10^27)) (10^9) (by norm_num)
  have h3 := sz_clD_bounds (clD (clD (s * i) (10^27)) (10^9) * p) (10^18) (by norm_num)
  have a1 : s * i ≤ clD (clD (s * i) (10^27)) (10^9) * 10^36 := by nlinarith
  have a2 : s * i * p ≤ clD (clD (s * i) (10^27)) (10^9) * 10^36 * p :=
    mul_le_mul_of_nonneg_right a1 hp
  nlinarith

/-- The ceiling chain overstates by less than `1 + 2·p/10^18` WAD units:
`cv·K < s·i·p + K + 2·10^36·p`. -/
theorem ha_cv_le (s i p : Int) (hsi : 0 ≤ s * i) (hp : 0 ≤ p) :
    cv s i p * 10^54 < s * i * p + 10^54 + 2 * 10^36 * p := by
  unfold cv
  have h1 := sz_clD_bounds (s * i) (10^27) (by norm_num)
  have h2 := sz_clD_bounds (clD (s * i) (10^27)) (10^9) (by norm_num)
  have h3 := sz_clD_bounds (clD (clD (s * i) (10^27)) (10^9) * p) (10^18) (by norm_num)
  have a1 : clD (clD (s * i) (10^27)) (10^9) * 10^36 < s * i + 10^36 + 10^27 := by nlinarith
  have a2 : clD (clD (s * i) (10^27)) (10^9) * 10^36 * p ≤ (s * i + 10^36 + 10^27) * p :=
    mul_le_mul_of_nonneg_right a1.le hp
  nlinarith

/-- The half-up chain overstates by less than `1 + 2·p/10^18` WAD units. -/
theorem ha_huv_le (s i p : Int) (hsi : 0 ≤ s * i) (hp : 0 ≤ p) :
    huv s i p * 10^54 < s * i * p + 10^54 + 2 * 10^36 * p := by
  unfold huv
  have h1 := sz_huD_bounds (s * i) (10^27) (by norm_num)
  have h2 := sz_huD_bounds (huD (s * i) (10^27)) (10^9) (by norm_num)
  have h3 := sz_huD_bounds (huD (huD (s * i) (10^27)) (10^9) * p) (10^18) (by norm_num)
  have a1 : huD (huD (s * i) (10^27)) (10^9) * 10^36 < s * i + 10^36 + 10^27 := by nlinarith
  have a2 : huD (huD (s * i) (10^27)) (10^9) * 10^36 * p ≤ (s * i + 10^36 + 10^27) * p :=
    mul_le_mul_of_nonneg_right a1.le hp
  nlinarith

/-! ## One collateral leg -/

/-- **Weighted collateral lost by removing `σ` shares.** For a supply leg
with `0 ≤ σ ≤ s`, index `i ≥ 0`, price `p ≥ 0` and threshold
`0 ≤ LT ≤ BPS`, the threshold-weighted floor value `⌊fv·LT/BPS⌋` (the leg's
term of `weighted_collateral`) drops by at most the exact value of the
removed shares, weighted, plus rounding:
`BPS·K·(⌊fv(s)·LT/BPS⌋ − ⌊fv(s−σ)·LT/BPS⌋) ≤ LT·(σ·i·p + K + 2·10^36·p) + BPS·K`. -/
theorem ha_coll_leg (s σ i p LT : Int) (hσ : 0 ≤ σ) (hσs : σ ≤ s) (hi : 0 ≤ i) (hp : 0 ≤ p)
    (hLT : 0 ≤ LT) :
    10^4 * 10^54 * (fv s i p * LT / 10^4 - fv (s - σ) i p * LT / 10^4) ≤
      LT * (σ * i * p + 10^54 + 2 * 10^36 * p) + 10^4 * 10^54 := by
  have hs : 0 ≤ s * i := mul_nonneg (by omega) hi
  have hs' : 0 ≤ (s - σ) * i := mul_nonneg (by omega) hi
  have e1 := ha_fv_le s i p hs hp
  have e2 := ha_fv_ge (s - σ) i p hs' hp
  have q1 := sz_div_bounds (fv s i p * LT) (10^4) (by norm_num)
  have q2 := sz_div_bounds (fv (s - σ) i p * LT) (10^4) (by norm_num)
  have d : 10^54 * (fv s i p - fv (s - σ) i p) ≤ σ * i * p + 10^54 + 2 * 10^36 * p := by
    have : (s - σ) * i * p = s * i * p - σ * i * p := by ring
    nlinarith
  have d' : 10^54 * ((fv s i p - fv (s - σ) i p) * LT) ≤ (σ * i * p + 10^54 + 2 * 10^36 * p) * LT := by
    have := mul_le_mul_of_nonneg_right d hLT; nlinarith
  nlinarith

/-- **Half-up value against weighted floor value.** For a supply leg with
`0 ≤ LT ≤ BPS`: `K·LT·huv ≤ K·BPS·(⌊fv·LT/BPS⌋ + 1) + LT·(2K + 4·10^36·p)`. -/
theorem ha_huv_weighted (s i p LT : Int) (hsi : 0 ≤ s * i) (hp : 0 ≤ p) (hLT : 0 ≤ LT) :
    10^54 * (LT * huv s i p) ≤
      10^54 * 10^4 * (fv s i p * LT / 10^4 + 1) + LT * (2 * 10^54 + 4 * 10^36 * p) := by
  have e1 := ha_fv_ge s i p hsi hp
  have e2 := ha_huv_le s i p hsi hp
  have q := sz_div_bounds (fv s i p * LT) (10^4) (by norm_num)
  have d : 10^54 * huv s i p ≤ 10^54 * fv s i p + 2 * 10^54 + 4 * 10^36 * p := by linarith
  have d' := mul_le_mul_of_nonneg_left d hLT
  nlinarith

/-! ## One debt leg -/

/-- How a debt leg changes in the post-liquidation model. Debt scaled `t`,
borrow index `ib`, price `p`, decimals `dec`, repaid tokens `a`, credited USD
`u`, post scaled `t'`:
* untouched: `a = 0`, `u = 0`, `t' = t`;
* partial (`0 < a < D`, `D = ⌈t·ib/(R·10^(27−dec))⌉`): `u ≤ huD(a·10^(18−dec)·p, W)`
  (`usd_value_wad`, or less after trimming) and
  `t' = t − ⌊a·10^(27−dec)·R/ib⌋` (`resolve_repay`, floor burn);
* full (`D ≤ a`): `u ≤ cv(t, ib, p)` (production credits `fv ≤ cv`) and `t' = 0`. -/
def DebtStep (t ib p : Int) (dec : Nat) (a u t' : Int) : Prop :=
  (a = 0 ∧ u = 0 ∧ t' = t) ∨
  (0 < a ∧ a < clD (t * ib) (10^27 * 10^(27 - dec)) ∧
    u ≤ huD (a * 10^(18 - dec) * p) (10^18) ∧ t' = t - a * 10^(27 - dec) * 10^27 / ib) ∨
  (clD (t * ib) (10^27 * 10^(27 - dec)) ≤ a ∧ u ≤ cv t ib p ∧ t' = 0)

/-- **Ceiling debt shed by a repayment.** For a debt leg with `t ≥ 0`,
`0 < ib ≤ 10^36` (borrow index between `RAY` and its cap), `p ≥ 0`,
`dec ≤ 18`, following `DebtStep`: `0 ≤ t'` and
`2K·(cv(t) − cv(t')) ≥ 2K·u − 3K − 6·10^36·p`: the debt value drops by at
least the credited USD minus `1.5 + 3·p/10^18` WAD units. -/
theorem ha_debt_leg (t ib p : Int) (dec : Nat) (a u t' : Int) (ht : 0 ≤ t) (hib : 0 < ib)
    (hibB : ib ≤ 10^36) (hp : 0 ≤ p) (hdec : dec ≤ 18) (h : DebtStep t ib p dec a u t') :
    0 ≤ t' ∧ t' ≤ t ∧
    2 * 10^54 * u - 3 * 10^54 - 6 * 10^36 * p ≤ 2 * 10^54 * (cv t ib p - cv t' ib p) := by
  have hti : 0 ≤ t * ib := mul_nonneg ht hib.le
  rcases h with ⟨_, hu, rfl⟩ | ⟨ha, hlt, hu, rfl⟩ | ⟨_, hu, rfl⟩
  · refine ⟨ht, le_refl _, ?_⟩; rw [hu]; nlinarith
  · have hK : (0:Int) < 10^(27 - dec) := by positivity
    have hK18 : (10:Int)^(27 - dec) = 10^9 * 10^(18 - dec) := by
      rw [← pow_add]; congr 1; omega
    set bu := a * 10^(27 - dec) * 10^27 / ib with hbu
    have hb := sz_div_bounds (a * 10^(27 - dec) * 10^27) ib hib
    rw [← hbu] at hb
    have hb0 : 0 ≤ bu := sz_div_nonneg _ _ hib (by positivity)
    -- `bu ≤ t`: the request is below the ceiling debt
    have hD := sz_clD_bounds (t * ib) (10^27 * 10^(27 - dec)) (by positivity)
    have hbt : bu ≤ t := by
      have h1 : a * (10^27 * 10^(27 - dec)) < t * ib := by
        have : a * (10^27 * 10^(27 - dec)) ≤ (clD (t * ib) (10^27 * 10^(27 - dec)) - 1) *
            (10^27 * 10^(27 - dec)) := mul_le_mul_of_nonneg_right (by omega) (by positivity)
        nlinarith
      exact sz_div_le_of _ _ _ hib (by nlinarith)
    refine ⟨by omega, by omega, ?_⟩
    have ht' : 0 ≤ (t - bu) * ib := mul_nonneg (by omega) hib.le
    have c1 := ha_cv_ge t ib p hti hp
    have c2 := ha_cv_le (t - bu) ib p ht' hp
    have hu2 := sz_huD_bounds (a * 10^(18 - dec) * p) (10^18) (by norm_num)
    -- `2·bu·ib·p ≥ 2K·u − K − 2·ib·p`
    have e1 : (a * 10^(27 - dec) * 10^27 - ib) * p ≤ bu * ib * p :=
      mul_le_mul_of_nonneg_right (by linarith [hb.2]) hp
    have e2 : a * 10^(27 - dec) * 10^27 * p = 10^36 * (a * 10^(18 - dec) * p) := by
      rw [hK18]; ring
    have e3 : 2 * 10^54 * u ≤ 10^36 * (2 * (a * 10^(18 - dec) * p) + 10^18) := by nlinarith
    have e4 : ib * p ≤ 10^36 * p := mul_le_mul_of_nonneg_right hibB hp
    have e5 : (t - bu) * ib * p = t * ib * p - bu * ib * p := by ring
    nlinarith
  · refine ⟨le_refl _, ht, ?_⟩
    have : cv 0 ib p = 0 := by unfold cv clD; norm_num
    rw [this]
    have := ha_cv_ge t ib p hti hp
    nlinarith

end ControllerAeneas.Proofs
