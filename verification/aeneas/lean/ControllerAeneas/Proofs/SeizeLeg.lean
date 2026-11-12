/-
One collateral leg of `calculate_seized_collateral`
(`contracts/controller/src/positions/liquidation/math.rs`): the exact closed
form of every quantity the loop body computes, and the body's total
correctness on the production domain.

Notation (all integers): `R = 10^27`, `W = 10^18`, `BPS = 10^4`. For a leg
with scaled shares `s`, supply index `i`, price `p` (WAD), decimals `dec`,
liquidation fee rate `f` (BPS), and the plan's total collateral `C`,
seizure budget `B` (`total_seizure_usd`) and `opb` (`1 + bonus` in WAD):
`Dd = 10^(27−dec)` (one token in RAY units),
`A = huD(s·i, R)` (`actual_ray`), `V = huv(s, i, p)` (`asset_value`),
`Sh = huD(V·W, C)` (`share`), `L = huD(B·Sh, W)` (`seizure_for_asset_usd`),
`Q = huD(L·W, p)` (`seizure_amount_wad`), `S0 = min(Q·10^9, i128::MAX)`
(`seizure_ray`), and so on down to the emitted entry.

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.SeizeBasic

open Aeneas Aeneas.Std Result
open ControllerAeneas.Spec ControllerAeneas.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

/-- The inputs of one leg of the seizure loop, as integers. -/
structure SzLeg where
  /-- scaled shares held -/
  s : Int
  /-- supply index (RAY) -/
  i : Int
  /-- price (WAD) -/
  p : Int
  /-- token decimals -/
  dec : Nat
  /-- stamped liquidation fee rate (BPS) -/
  f : Int
  /-- total collateral of the account (WAD) -/
  C : Int
  /-- seizure budget `total_seizure_usd` (WAD) -/
  B : Int
  /-- `1 + bonus` (WAD) -/
  opb : Int
  /-- `repayment.repays_all_debt` -/
  rad : Bool
  /-- `repayment.seize_all` -/
  sa : Bool

namespace SzLeg

/-- One token in RAY units: `10^(27 − dec)`. -/
def Dd (g : SzLeg) : Int := 10 ^ (27 - g.dec)
/-- `actual_ray = Ray::mul(s, i)`. -/
def A (g : SzLeg) : Int := huD (g.s * g.i) (10^27)
/-- `asset_value = position_value(s, i, p)`. -/
def V (g : SzLeg) : Int := huv g.s g.i g.p
/-- `share = asset_value.div(total_collateral)`. -/
def Sh (g : SzLeg) : Int := huD (g.V * 10^18) g.C
/-- `seizure_for_asset_usd = total_seizure_usd.mul(share)`: the leg's budget. -/
def L (g : SzLeg) : Int := huD (g.B * g.Sh) (10^18)
/-- `seizure_amount_wad = seizure_for_asset_usd.div(price)`: tokens (WAD). -/
def Q (g : SzLeg) : Int := huD (g.L * 10^18) g.p
/-- `seizure_ray`: the saturating upscale `min(Q·10^9, i128::MAX)`. -/
def S0 (g : SzLeg) : Int := min (g.Q * 10^9) (2^127 - 1)
/-- The whole-unit branch is taken: not seize-all, decimals below
`MIN_BORROWABLE_ASSET_DECIMALS = 3`, request below the held balance. -/
def WU (g : SzLeg) : Prop := g.sa = false ∧ g.dec < 3 ∧ g.S0 < g.A
instance (g : SzLeg) : Decidable g.WU := by unfold WU; infer_instance
/-- `seizure_ray` after the whole-unit branch. -/
def S1 (g : SzLeg) : Int :=
  if g.WU then (if g.rad then min (clD g.S0 g.Dd * g.Dd) g.A else g.S0 / g.Dd * g.Dd) else g.S0
/-- `seizure_amount_wad` after the whole-unit branch. -/
def A1 (g : SzLeg) : Int := if g.WU then huD g.S1 (10^9) else g.Q
/-- USD value of the whole units seized (floor branch only). -/
def WUsd (g : SzLeg) : Int := huD (g.A1 * g.p) (10^18)
/-- The leg's contribution to `unseized_usd`. -/
def Uns (g : SzLeg) : Int :=
  if g.WU ∧ g.rad = false then (if g.WUsd < g.L then g.L - g.WUsd else 0) else 0
/-- `capped_ray`. -/
def Capped (g : SzLeg) : Int := if g.sa then g.A else min g.S1 g.A
/-- `base_ray = min(⌊A1·W/opb⌋·10^9, i128::MAX)`. -/
def Base (g : SzLeg) : Int := min (g.A1 * 10^18 / g.opb * 10^9) (2^127 - 1)
/-- `bonus_ray`. -/
def Bonus (g : SzLeg) : Int := if g.Base < g.Capped then g.Capped - g.Base else 0
/-- `protocol_fee_ray = huD(bonus·f, BPS)`. -/
def FeeR (g : SzLeg) : Int := huD (g.Bonus * g.f) (10^4)
/-- `seized_scaled`. -/
def Seized (g : SzLeg) : Int := if g.Capped = g.A then g.s else g.Capped * 10^27 / g.i
/-- `bonus_scaled`. -/
def Bsc (g : SzLeg) : Int := min (g.Bonus * 10^27 / g.i) g.Seized
/-- `capped_amount` (token units). -/
def Amt (g : SzLeg) : Int := if g.Capped = g.A then huD g.Capped g.Dd else g.Capped / g.Dd
/-- `pool_gross` (the paid amount of `resolve_withdrawal`). -/
def PG (g : SzLeg) : Int :=
  if huD g.A g.Dd ≤ g.Amt then g.s * g.i / 10^27 / g.Dd else g.Amt
/-- `fee_asset`. -/
def FeeA (g : SzLeg) : Int := g.FeeR / g.Dd
/-- `bumped_fee`. -/
def Bumped (g : SzLeg) : Int := if 0 < g.FeeR ∧ g.FeeA = 0 then 1 else g.FeeA
/-- `realised_excess`. -/
def Real (g : SzLeg) : Int :=
  if g.Base < g.PG * g.Dd then (g.PG * g.Dd - g.Base) / g.Dd else 0
/-- `protocol_fee`. -/
def PFee (g : SzLeg) : Int := min g.Bumped g.Real
/-- The leg pushes an entry (none of the four `continue` checks fires). -/
def Emit (g : SzLeg) : Prop := 0 < g.S1 ∧ 0 < g.Capped ∧ 0 < g.Seized ∧ 0 < g.Amt

end SzLeg

/-- The production domain of one leg (see `docs/controller-seizure.md`):
* `0 ≤ s ≤ 2^126` and `s·i ≤ 2^126·R` (held shares and RAY value fit with a
  factor-2 margin), `VD s i p` (the USD value fits, `Totals.lean`);
* `0 < i` (supply index ≥ its floor), `0 < p` (oracle prices are positive),
  `dec ≤ 18` (`MAX_ASSET_DECIMALS`), `0 ≤ f ≤ BPS` (stamped fee rate);
* `0 < C` (the loop runs only then) and `V ≤ C` (production passes
  `C = Σ V` over the same legs);
* `0 ≤ B` and `B·W ≤ 2^126·p`: the budget expressed in tokens of this asset
  fits `i128` (otherwise `Wad::div` by the price aborts, see the doc);
* `W ≤ opb` (`1 + bonus` with a non-negative bonus). -/
structure SzLegOK (g : SzLeg) : Prop where
  s0 : 0 ≤ g.s
  sB : g.s ≤ 2^126
  i0 : 0 < g.i
  p0 : 0 < g.p
  dec18 : g.dec ≤ 18
  f0 : 0 ≤ g.f
  fB : g.f ≤ 10^4
  vd : VD g.s g.i g.p
  siB : g.s * g.i ≤ 2^126 * 10^27
  C0 : 0 < g.C
  VC : g.V ≤ g.C
  B0 : 0 ≤ g.B
  BQ : g.B * 10^18 ≤ 2^126 * g.p
  opbW : 10^18 ≤ g.opb

namespace SzLegOK

variable {g : SzLeg} (h : SzLegOK g)
include h

theorem Dd_pos : 0 < g.Dd := by unfold SzLeg.Dd; positivity
theorem Dd_le : g.Dd ≤ 10^27 := by
  unfold SzLeg.Dd; exact pow_le_pow_right₀ (by norm_num) (by omega)
theorem Dd_ge : 10^9 ≤ g.Dd := by
  unfold SzLeg.Dd; exact pow_le_pow_right₀ (by norm_num) (by have := h.dec18; omega)

theorem si0 : 0 ≤ g.s * g.i := mul_nonneg h.s0 h.i0.le

theorem A0 : 0 ≤ g.A := sz_huD_nonneg _ _ (by norm_num) h.si0
theorem A_two : 2 * g.A * 10^27 ≤ 2 * (g.s * g.i) + 10^27 := (sz_huD_bounds _ _ (by norm_num)).1
theorem A_two' : 2 * (g.s * g.i) + 10^27 < 2 * g.A * 10^27 + 2 * 10^27 :=
  (sz_huD_bounds _ _ (by norm_num)).2
theorem AB : g.A ≤ 2^126 := by
  have := h.A_two; have := h.siB
  by_contra hc
  have : (2^126 + 1) * 10^27 ≤ g.A * 10^27 := mul_le_mul_of_nonneg_right (by omega) (by norm_num)
  nlinarith

theorem V0 : 0 ≤ g.V := by
  obtain ⟨hs, hi, hp, -, -⟩ := h.vd
  unfold SzLeg.V huv
  apply sz_huD_nonneg _ _ (by norm_num)
  apply mul_nonneg _ hp
  exact sz_huD_nonneg _ _ (by norm_num) (sz_huD_nonneg _ _ (by norm_num) (mul_nonneg hs hi))

theorem Sh0 : 0 ≤ g.Sh := sz_huD_nonneg _ _ h.C0 (mul_nonneg h.V0 (by norm_num))
theorem ShW : g.Sh ≤ 10^18 := sz_huD_le_of _ _ _ h.C0 (by nlinarith [h.VC])
theorem L0 : 0 ≤ g.L := sz_huD_nonneg _ _ (by norm_num) (mul_nonneg h.B0 h.Sh0)
theorem LB : g.L ≤ g.B := sz_huD_le_of _ _ _ (by norm_num) (by nlinarith [h.ShW, h.B0])
theorem Q0 : 0 ≤ g.Q := sz_huD_nonneg _ _ h.p0 (mul_nonneg h.L0 (by norm_num))
theorem QB : g.Q ≤ 2^126 := sz_huD_le_of _ _ _ h.p0 (by nlinarith [h.LB, h.BQ])
theorem S0_le : g.S0 ≤ g.Q * 10^9 := min_le_left _ _
theorem S0_max : g.S0 ≤ 2^127 - 1 := min_le_right _ _
theorem S00 : 0 ≤ g.S0 := le_min (by have := h.Q0; omega) (by norm_num)

theorem S1_0 : 0 ≤ g.S1 := by
  have hD := h.Dd_pos
  unfold SzLeg.S1
  split_ifs
  · exact le_min (mul_nonneg (sz_clD_nonneg _ _ hD h.S00) hD.le) h.A0
  · exact mul_nonneg (sz_div_nonneg _ _ hD h.S00) hD.le
  · exact h.S00

theorem S1_le : g.S1 ≤ max g.S0 g.A := by
  have hD := h.Dd_pos
  unfold SzLeg.S1
  split_ifs
  · exact le_trans (min_le_right _ _) (le_max_right _ _)
  · exact le_trans (sz_div_bounds g.S0 g.Dd hD).1 (le_max_left _ _)
  · exact le_max_left _ _

theorem S1B : g.S1 ≤ 2^127 - 1 := by
  have h1 := h.S1_le; have h3 := h.AB
  exact le_trans h1 (max_le h.S0_max (by omega))

theorem A1_0 : 0 ≤ g.A1 := by
  unfold SzLeg.A1; split_ifs
  · exact sz_huD_nonneg _ _ (by norm_num) h.S1_0
  · exact h.Q0

theorem A1B : g.A1 ≤ 2^126 := by
  unfold SzLeg.A1; split_ifs
  · exact sz_huD_le_of _ _ _ (by norm_num) (by have := h.S1B; omega)
  · exact h.QB

theorem Capped0 : 0 ≤ g.Capped := by
  unfold SzLeg.Capped; split_ifs
  · exact h.A0
  · exact le_min h.S1_0 h.A0

theorem CappedA : g.Capped ≤ g.A := by
  unfold SzLeg.Capped; split_ifs
  · exact le_refl _
  · exact min_le_right _ _

theorem Base0 : 0 ≤ g.Base := by
  unfold SzLeg.Base
  have := h.opbW
  exact le_min (mul_nonneg (sz_div_nonneg _ _ (by omega) (mul_nonneg h.A1_0 (by norm_num)))
    (by norm_num)) (by norm_num)

theorem Bonus0 : 0 ≤ g.Bonus := by
  unfold SzLeg.Bonus; split_ifs <;> omega

theorem BonusC : g.Bonus ≤ g.Capped := by
  have := h.Base0; have := h.Capped0
  unfold SzLeg.Bonus; split_ifs <;> omega

/-- The USD value of any whole-unit request `x ≤ A` fits `i128`
(`Wad::mul(to_wad(x), price)` cannot abort). -/
theorem wusd_fit (x : Int) (hx0 : 0 ≤ x) (hxA : x ≤ g.A) :
    huD (huD x (10^9) * g.p) (10^18) ≤ 2^127 - 1 := by
  obtain ⟨-, -, hp, -, hB⟩ := h.vd
  have h1 := sz_huD_bounds x (10^9) (by norm_num)
  have h2 := h.A_two
  have h3 : huD x (10^9) * (10^27 * 10^9) ≤ g.s * g.i + 2 * 10^27 * 10^9 := by nlinarith
  have h4 : huD x (10^9) * (10^27 * 10^9) * g.p ≤ (g.s * g.i + 2 * 10^27 * 10^9) * g.p :=
    mul_le_mul_of_nonneg_right h3 hp
  apply sz_huD_le_of _ _ _ (by norm_num)
  nlinarith

/-- A partial request (`x < A`) converts to strictly fewer shares than held:
`⌊x·R/i⌋ ≤ s`. -/
theorem part_le (x : Int) (hxA : x < g.A) : x * 10^27 / g.i ≤ g.s := by
  apply sz_div_le_of _ _ _ h.i0
  have := h.A_two
  nlinarith

/-- Any `0 ≤ x ≤ A` converts to shares that fit `i128`: `⌊x·R/i⌋ ≤ s + R`. -/
theorem div_fit (x : Int) (hxA : x ≤ g.A) : x * 10^27 / g.i ≤ g.s + 10^27 := by
  apply sz_div_le_of _ _ _ h.i0
  have := h.A_two; have := h.i0; have := h.s0
  nlinarith

theorem WU_A1 (hw : g.WU) : g.A1 = huD g.S1 (10^9) := by unfold SzLeg.A1; rw [if_pos hw]
theorem nWU_A1 (hw : ¬ g.WU) : g.A1 = g.Q := by unfold SzLeg.A1; rw [if_neg hw]
theorem nWU_S1 (hw : ¬ g.WU) : g.S1 = g.S0 := by unfold SzLeg.S1; rw [if_neg hw]
theorem nWU_Uns (hw : ¬ g.WU) : g.Uns = 0 := by
  unfold SzLeg.Uns; rw [if_neg (fun c => hw c.1)]

theorem Uns0 : 0 ≤ g.Uns := by
  unfold SzLeg.Uns; split_ifs <;> omega

theorem UnsL : g.Uns ≤ g.L := by
  have := h.L0
  have hw : 0 ≤ g.WUsd := sz_huD_nonneg _ _ (by norm_num) (mul_nonneg h.A1_0 h.p0.le)
  unfold SzLeg.Uns; split_ifs <;> omega

/-! ### Per-entry properties (INV-LIQ-02) -/

/-- Seized shares never exceed held shares: `0 < Seized ≤ s` (an emitted leg). -/
theorem seized_le (he : g.Emit) : 0 < g.Seized ∧ g.Seized ≤ g.s := by
  refine ⟨he.2.2.1, ?_⟩
  unfold SzLeg.Seized
  split_ifs with c
  · exact le_refl _
  · exact h.part_le _ (lt_of_le_of_ne h.CappedA c)

/-- Bonus shares lie between 0 and the seized shares. -/
theorem bsc_le (he : g.Emit) : 0 ≤ g.Bsc ∧ g.Bsc ≤ g.Seized := by
  have := h.Bonus0
  refine ⟨le_min (sz_div_nonneg _ _ h.i0 (by positivity)) he.2.2.1.le, min_le_right _ _⟩

/-- The floor held amount is at most the half-up held amount. -/
theorem F_le_H : g.s * g.i / 10^27 / g.Dd ≤ huD g.A g.Dd := by
  have hD := h.Dd_pos
  have h1 := sz_div_bounds (g.s * g.i) (10^27) (by norm_num)
  have h2 := h.A_two'
  have hfa : g.s * g.i / 10^27 ≤ g.A := by
    by_contra c
    have : (g.A + 1) * 10^27 ≤ (g.s * g.i / 10^27) * 10^27 :=
      mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    nlinarith
  have h3 := sz_div_bounds (g.s * g.i / 10^27) g.Dd hD
  have h4 := sz_huD_bounds g.A g.Dd hD
  by_contra c
  have : (huD g.A g.Dd + 1) * g.Dd ≤ (g.s * g.i / 10^27 / g.Dd) * g.Dd :=
    mul_le_mul_of_nonneg_right (by omega) hD.le
  nlinarith

theorem PG_le : 0 ≤ g.PG ∧ g.PG ≤ g.Amt := by
  have hD := h.Dd_pos
  have hF := h.F_le_H
  have hF0 : 0 ≤ g.s * g.i / 10^27 / g.Dd :=
    sz_div_nonneg _ _ hD (sz_div_nonneg _ _ (by norm_num) h.si0)
  have hA0 : 0 ≤ g.Amt := by
    unfold SzLeg.Amt; split_ifs
    · exact sz_huD_nonneg _ _ hD h.Capped0
    · exact sz_div_nonneg _ _ hD h.Capped0
  unfold SzLeg.PG; split_ifs <;> omega

/-- The protocol fee is between 0 and the transferred amount, which is positive. -/
theorem fee_le (he : g.Emit) : 0 < g.Amt ∧ 0 ≤ g.PFee ∧ g.PFee ≤ g.Amt := by
  have hD := h.Dd_pos
  have hpg := h.PG_le
  have hb := h.Base0
  have hfr : 0 ≤ g.FeeR := sz_huD_nonneg _ _ (by norm_num) (mul_nonneg h.Bonus0 h.f0)
  have hfa : 0 ≤ g.FeeA := sz_div_nonneg _ _ hD hfr
  have hbu : 0 ≤ g.Bumped := by unfold SzLeg.Bumped; split_ifs <;> omega
  have hre : 0 ≤ g.Real ∧ g.Real ≤ g.PG := by
    unfold SzLeg.Real; split_ifs
    · exact ⟨sz_div_nonneg _ _ hD (by omega),
        sz_div_le_of _ _ _ hD (by nlinarith)⟩
    · exact ⟨le_refl _, hpg.1⟩
  refine ⟨he.2.2.2, le_min hbu hre.1, ?_⟩
  exact le_trans (min_le_right _ _) (le_trans hre.2 hpg.2)

/-- The transferred amount is at most the half-up held amount
`huD(A, 10^(27−dec))` (`unscale_supply`, what `resolve_withdrawal` compares
against): exactly it on a full close, at most `⌊A/10^(27−dec)⌋` otherwise. -/
theorem amt_le : g.Amt ≤ huD g.A g.Dd := by
  have hD := h.Dd_pos
  unfold SzLeg.Amt
  split_ifs with c
  · rw [c]
  · have h1 := sz_div_bounds g.Capped g.Dd hD
    have h2 := sz_huD_bounds g.A g.Dd hD
    have := h.CappedA
    by_contra c'
    have : (huD g.A g.Dd + 1) * g.Dd ≤ (g.Capped / g.Dd) * g.Dd :=
      mul_le_mul_of_nonneg_right (by omega) hD.le
    nlinarith

/-- The seized shares are worth at most the capped RAY amount plus half a RAY
unit: `2·Seized·i ≤ R·(2·Capped + 1)`. -/
theorem seized_val : 2 * (g.Seized * g.i) ≤ 10^27 * (2 * g.Capped + 1) := by
  unfold SzLeg.Seized
  split_ifs with c
  · have := h.A_two'; rw [c]; nlinarith
  · have := sz_div_bounds (g.Capped * 10^27) g.i h.i0
    nlinarith

/-- Outside seize-all the capped request is at most the (whole-unit
adjusted) request `S1`, and always at most the held `A`. -/
theorem capped_le (hsa : g.sa = false) : g.Capped ≤ g.S1 := by
  unfold SzLeg.Capped; rw [hsa]; exact min_le_left _ _

/-- The request after the whole-unit branch exceeds the pro-rata request
`S0 ≤ Q·10^9` by less than one whole token, and only when the plan repays all
debt (rounded up to a whole unit); otherwise it is at most `S0`. -/
theorem S1_le_S0 : g.S1 ≤ g.S0 + (if g.WU ∧ g.rad = true then g.Dd - 1 else 0) := by
  have hD := h.Dd_pos
  unfold SzLeg.S1
  by_cases hw : g.WU
  · by_cases hr : g.rad = true
    · rw [if_pos hw, if_pos hr, if_pos ⟨hw, hr⟩]
      have := sz_clD_bounds g.S0 g.Dd hD
      exact le_trans (min_le_left _ _) (by omega)
    · rw [if_pos hw, if_neg hr, if_neg (fun c => hr c.2)]
      have := sz_div_bounds g.S0 g.Dd hD
      omega
  · rw [if_neg hw, if_neg (fun c => hw c.1)]; omega

/-- **Value of one leg's seizure against its budget.** For an emitted leg
outside seize-all, with `X = Dd − 1` in the whole-unit round-up branch and
`0` otherwise, the exact value of the seized shares (`Seized·i·p`, scaled
by `R·10^9·W = 10^54`) satisfies
`2·Seized·i·p ≤ 2·10^54·L + 10^27·p·(10^9 + 1 + 2·X)`:
the leg's budget `L` plus half a token-wei of rounding (`Wad::div` by the
price, half up), half a RAY unit (share conversion), and on the round-up
branch one whole token. -/
theorem seized_value_le_budget (hsa : g.sa = false) :
    2 * (g.Seized * g.i * g.p) ≤ 2 * 10^54 * g.L +
      10^27 * g.p * (10^9 + 1 + 2 * (if g.WU ∧ g.rad = true then g.Dd - 1 else 0)) := by
  have hp := h.p0
  have h1 := h.seized_val
  have h2 := h.capped_le hsa
  have h3 := h.S1_le_S0
  have h4 := h.S0_le
  have h5 := sz_huD_bounds (g.L * 10^18) g.p hp
  have hQ : g.Q = huD (g.L * 10^18) g.p := rfl
  rw [← hQ] at h5
  generalize (if g.WU ∧ g.rad = true then g.Dd - 1 else 0) = X at h3 ⊢
  have h6 : 2 * (g.Seized * g.i) * g.p ≤ 10^27 * (2 * g.Capped + 1) * g.p :=
    mul_le_mul_of_nonneg_right h1 hp.le
  have h7 : g.Capped * g.p ≤ (g.Q * 10^9 + X) * g.p :=
    mul_le_mul_of_nonneg_right (by omega) hp.le
  nlinarith

end SzLegOK

end ControllerAeneas.Proofs
