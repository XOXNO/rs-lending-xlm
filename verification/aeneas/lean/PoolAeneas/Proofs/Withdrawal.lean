-- Copied from RatesAeneas/Proofs/Withdrawal.lean (2026-11-12 snapshot) for the pool
-- extraction, with `RatesAeneas`/`rates_aeneas`/`aeneas_soroban_shim` renamed to
-- `PoolAeneas`/`pool_aeneas`/`aeneas_pool_shim` and `constants.production.` to
-- `constants.`; imports point at the pool copies (`RatesHalfUpWrappers`, `RatesIndex`, whose
-- lemmas live in `PoolAeneas.RatesProofs`, opened below). Only `resolve_withdrawal_spec` and
-- its two arithmetic lemmas are copied (the abort characterisation is not needed here).
/-
`rates::scaling::resolve_withdrawal` (common/src/rates/scaling.rs): the
withdrawal resolver. Unbounded counterpart of the Kani harness
`withdrawal_never_overpays` (`verification/kani/src/proofs/scaling.rs`).

The function compares the request with the half-up value
`unscale_supply(pos)` (two half-up roundings: `Ray::mul` then
`Ray::to_asset`), pays the floor value `unscale_supply_floor(pos)` on a full
withdrawal, and otherwise burns `calculate_scaled_supply_ceil(amount)` and
pays `amount`.

Notation: `RAY = 10^27`, `k = 27 - decimals`, `D = 10^k`, `P = RAY * D`
(one asset unit in `scaled * index` units), `N = pos * index`,
`r = hu(N, RAY)` (`Ray::mul`), `H = hu(r, D)` (`Ray::to_asset`, the branch
value), `F = ⌊N/P⌋` (`unscale_supply_floor`).

The subtle case (`amount < H`, the ceiling burn could round past the
position) is safe: `amount ≤ H - 1` gives
`2*amount*D + D ≤ 2*r` and `2*r*RAY ≤ 2*N + RAY`, so
`2*amount*D*RAY ≤ 2*N + RAY - D*RAY ≤ 2*N` (as `D ≥ 1`), i.e.
`amount*P ≤ N`, hence `⌈amount*P/index⌉ ≤ pos` and `amount ≤ F`.
-/
import PoolAeneas.Proofs.FloorCeilResolve
import PoolAeneas.Proofs.RatesHalfUpWrappers
import PoolAeneas.Proofs.RatesIndex

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec PoolAeneas.HalfUpSpec
open pool_aeneas
open PoolAeneas.RatesProofs (ray_mul_spec ray_to_asset_spec)

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- Arithmetic core of the partial branch: a request strictly below the
double half-up value `H = hu(hu(N, RAY), D)` is worth at most `N`:
`amount*D*RAY ≤ N`. Needs only `D ≥ 1`. -/
theorem withdrawal_partial_le {N r H amount D : Int} (hD : 1 ≤ D)
    (hr : IsHalfUp N (10^27) r) (hH : IsHalfUp r D H) (hlt : amount < H) :
    amount * D * 10^27 ≤ N := by
  have h1 := hr.mul_le
  have h2 := hH.mul_le
  have h3 : (amount + 1) * D ≤ H * D := mul_le_mul_of_nonneg_right (by omega) (by omega)
  -- 2*amount*D + D ≤ 2*r
  have h4 : 2 * (amount * D) + D ≤ 2 * r := by nlinarith
  have h5 : (2 * (amount * D) + D) * 10^27 ≤ 2 * r * 10^27 :=
    mul_le_mul_of_nonneg_right h4 (by norm_num)
  nlinarith

/-- Arithmetic core of the full branch: the floor value is at most the double
half-up value, `F ≤ H`, for `N ≥ 0`, `D ≥ 1`. -/
theorem withdrawal_floor_le_halfup {N r H F D : Int} (hD : 1 ≤ D)
    (hr : IsHalfUp N (10^27) r) (hH : IsHalfUp r D H)
    (hF : IsFloor N (10^27 * D) F) : F ≤ H := by
  have hP : (0:Int) < 10^27 * D := by positivity
  unfold IsFloor at hF; rw [if_pos hP] at hF
  have h1 := hr.lt_mul
  have h2 := hH.lt_mul
  -- F*D ≤ r
  have hFD : F * D ≤ r := by
    by_contra h
    have : (r + 1) * 10^27 ≤ F * D * 10^27 :=
      mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    nlinarith
  by_contra h
  have : (H + 1) * D ≤ F * D := mul_le_mul_of_nonneg_right (by omega) (by omega)
  nlinarith

/-- **`resolve_withdrawal` never aborts, never burns more shares than held,
never pays more than the floor-valued claim, and partial burns favour the
protocol (ADR-0003).** With `N = pos*index`, `r = hu(N, RAY)`,
`H = hu(r, 10^k)` (the half-up value `unscale_supply` compares against) and
`F = ⌊N/P⌋` (the floor value `unscale_supply_floor` pays), the call returns
(does not abort) a pair `(burn, paid)` with:

* always: `0 ≤ burn ≤ pos` (never burns more shares than held, including the
  case `amount < H` where the ceiling burn could in principle round past the
  position), `0 ≤ paid ≤ F` (never pays more than the floor-valued claim) and
  `paid ≤ amount` (never pays more than requested);
* full withdrawal (`H ≤ amount`): `burn = pos` and `paid = F`;
* partial withdrawal (`amount < H`): `paid = amount`,
  `burn = ⌈amount*10^k*RAY/index⌉` (`calculate_scaled_supply_ceil`), so
  `amount*10^k*RAY ≤ burn*index` (the burned shares are worth at least the
  payout), and `amount*10^k*RAY ≤ pos*index` (the request is covered by the
  position's exact value).

Hypotheses and why they hold in production:
* `decimals < 27`: listed assets have at most `MAX_ASSET_DECIMALS = 18`
  decimals (`Ray::to_asset` needs a downscale).
* `0 < index`: the supply index is at least `SUPPLY_INDEX_FLOOR_RAW`
  (INV-IDX-02, ADR-0012). No upper bound and no `≥ 10^24` is needed.
* `0 ≤ pos`: share balances are non-negative (INV-ACCT-01).
* `0 ≤ amount`: the withdraw entry rejects non-positive amounts
  (INV-ACCT-05); `amount = 0` is allowed here.
* `hu(pos*index, RAY)` fits `i128`: this is exactly the abort condition of
  the first call (`Ray::mul` in `unscale_supply`), see
  `resolve_withdrawal_aborts`. Every other fit (the floor product, `A =
  amount*10^k`, the ceiling burn) is derived. -/
theorem resolve_withdrawal_spec (env : Env) (amount pos index : Std.I128) (dec : Std.U32)
    (hdec : dec.val < 27) (hidx : 0 < index.val) (hpos : 0 ≤ pos.val) (hamt : 0 ≤ amount.val)
    (hfit : ∃ q : Int, IsHalfUp (pos.val * index.val) (10^27) q ∧ q < 2^127) :
    rates.scaling.resolve_withdrawal env amount pos index dec ⦃ p => ∀ r H F : Int,
      IsHalfUp (pos.val * index.val) (10^27) r →
      IsHalfUp r (10^(27 - dec.val)) H →
      IsFloor (pos.val * index.val) (10^27 * 10^(27 - dec.val)) F →
      (0 ≤ p.1.val ∧ p.1.val ≤ pos.val) ∧
      (0 ≤ p.2.val ∧ p.2.val ≤ F ∧ p.2.val ≤ amount.val) ∧
      (H ≤ amount.val → p.1 = pos ∧ p.2.val = F) ∧
      (amount.val < H → p.2 = amount ∧
        IsCeil (amount.val * 10^(27 - dec.val) * 10^27) index.val p.1.val ∧
        amount.val * 10^(27 - dec.val) * 10^27 ≤ p.1.val * index.val ∧
        amount.val * 10^(27 - dec.val) * 10^27 ≤ pos.val * index.val) ⦄ := by
  have hk := pow_pos (show (0:Int) < 10 by norm_num) (27 - dec.val)
  have hD1 : (1:Int) ≤ 10^(27 - dec.val) := by omega
  have hP : (0:Int) < 10^27 * 10^(27 - dec.val) := by positivity
  have hposb := pos.hBounds
  have hamtb := amount.hBounds
  simp only [IScalarTy.numBits] at hposb hamtb
  have hN : 0 ≤ pos.val * index.val := by positivity
  obtain ⟨q0, hq0, hq0hi⟩ := hfit
  -- the floor intermediate `⌊N/RAY⌋` fits: it lies in `[0, q0]`
  obtain ⟨f0, hf0⟩ := exists_isFloor (pos.val * index.val) (10^27) (by norm_num)
  have hf0c := hf0
  unfold IsFloor at hf0c; rw [if_pos (by norm_num)] at hf0c
  have hf0nn : 0 ≤ f0 := by
    by_contra h
    have : (f0 + 1) * 10^27 ≤ 0 := by nlinarith
    omega
  have hf0le : f0 ≤ q0 := by
    have := hq0.lt_mul
    by_contra h
    have : (q0 + 1) * 10^27 ≤ f0 * 10^27 :=
      mul_le_mul_of_nonneg_right (by omega) (by norm_num)
    nlinarith
  unfold rates.scaling.resolve_withdrawal rates.scaling.unscale_supply
    rates.scaling.scaled_to_original
  step with ray_mul_spec env pos index hpos (by omega) ⟨q0, hq0, hq0hi⟩ as ⟨r0, hr0⟩
  have hr0nn : 0 ≤ r0.val := IsHalfUp.nonneg (by norm_num) hN hr0
  step with ray_to_asset_spec env r0 dec hdec as ⟨H0, hH0⟩
  have hH0' : IsHalfUp r0.val (10^(27 - dec.val)) H0.val := by
    unfold IsHalfAway at hH0; rw [if_pos hr0nn] at hH0; exact hH0
  step with unscale_supply_floor_spec env pos index dec (by omega) hN
    ⟨f0, hf0, by omega, by omega⟩ as ⟨F0, hF0⟩
  have hFH : F0.val ≤ H0.val := withdrawal_floor_le_halfup hD1 hr0 hH0' hF0
  have hF0c := hF0
  unfold IsFloor at hF0c; rw [if_pos hP] at hF0c
  have hF0nn : 0 ≤ F0.val := by
    by_contra h
    have : (F0.val + 1) * (10^27 * 10^(27 - dec.val)) ≤ 0 :=
      mul_nonpos_of_nonpos_of_nonneg (by omega) (by omega)
    linarith
  by_cases hge : H0.val ≤ amount.val
  · have hge' : (amount >= H0) := by simp [IScalar.le_equiv]; omega
    simp only [hge', ↓reduceIte, WP.spec_ok]
    intro r H F hr hH hF
    have hre : r = r0.val := IsHalfUp.unique (by norm_num) hr hr0
    subst hre
    have hHe : H = H0.val := IsHalfUp.unique (by omega) hH hH0'
    have hFe : F = F0.val := IsFloor.unique (by omega) hF hF0
    subst hHe; subst hFe
    exact ⟨hpos, le_refl _, hF0nn, le_refl _, by omega, fun _ => ⟨trivial, rfl⟩,
      fun h => absurd h (by omega)⟩
  · have hlt' : ¬ (amount >= H0) := by simp [IScalar.le_equiv]; omega
    simp only [hlt', ↓reduceIte]
    have hAN : amount.val * 10^(27 - dec.val) * 10^27 ≤ pos.val * index.val :=
      withdrawal_partial_le hD1 hr0 hH0' (by omega)
    -- `A = amount*10^k` fits: `2*A + D ≤ 2*r0` and `r0 ≤ q0 < 2^127`
    have hr0q : r0.val = q0 := IsHalfUp.unique (by norm_num) hr0 hq0
    have hA0 : 0 ≤ amount.val * 10^(27 - dec.val) := by positivity
    have hAlt : amount.val * 10^(27 - dec.val) < 2^127 := by
      have h2 := hH0'.mul_le
      have h3 : (amount.val + 1) * 10^(27 - dec.val) ≤ H0.val * 10^(27 - dec.val) :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    obtain ⟨c, hc⟩ := exists_isCeil (amount.val * 10^(27 - dec.val) * 10^27) index.val (by omega)
    have hcc := hc
    unfold IsCeil at hcc; rw [if_pos hidx] at hcc
    have hc0 : 0 ≤ c := by
      by_contra h
      have : c * index.val < 0 := mul_neg_of_neg_of_pos (by omega) hidx
      nlinarith
    have hcpos : c ≤ pos.val := by
      by_contra h
      have : (pos.val + 1) * index.val ≤ c * index.val :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    step with calculate_scaled_supply_ceil_spec env amount dec index hdec (by omega)
      ⟨by omega, hAlt⟩ ⟨c, hc, by omega, by omega⟩ as ⟨s, hs⟩
    have hsc : s.val = c := IsCeil.unique (by omega) hs hc
    have hsc' := hs
    unfold IsCeil at hsc'; rw [if_pos hidx] at hsc'
    -- `amount ≤ F`: `amount*P ≤ N < (F+1)*P`
    have hamtF : amount.val ≤ F0.val := by
      by_contra h
      have : (F0.val + 1) * (10^27 * 10^(27 - dec.val)) ≤
          amount.val * (10^27 * 10^(27 - dec.val)) :=
        mul_le_mul_of_nonneg_right (by omega) (by omega)
      nlinarith
    intro r H F hr hH hF
    have hre : r = r0.val := IsHalfUp.unique (by norm_num) hr hr0
    subst hre
    have hHe : H = H0.val := IsHalfUp.unique (by omega) hH hH0'
    have hFe : F = F0.val := IsFloor.unique (by omega) hF hF0
    subst hHe; subst hFe
    exact ⟨by omega, by omega, hamt, hamtF, le_refl _, fun h => absurd h (by omega),
      fun _ => ⟨hs, hsc'.1, hAN⟩⟩

end PoolAeneas.Proofs
