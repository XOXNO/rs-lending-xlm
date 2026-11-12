/-
Share-total bookkeeping of `contracts/pool/src/cache/shares.rs`.

INV-ACCT-01: supply, debt and revenue share totals stay non-negative and
revenue never exceeds total supply. `SharesInv` states it; each mint and burn
below preserves it on every successful return.
-/
import PoolAeneas.Proofs.Basic

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec
open pool_aeneas

namespace PoolAeneas.Proofs

/-- INV-ACCT-01 on one market cache: `0 ≤ revenue ≤ supplied` and
`0 ≤ borrowed` (all scaled RAY share totals). (`0 ≤ supplied` follows.) -/
def SharesInv (c : market_cache.Cache) : Prop :=
  0 ≤ c.revenue.val ∧ c.revenue.val ≤ c.supplied.val ∧ 0 ≤ c.borrowed.val

/-- `require_revenue_backed` returned: `revenue ≤ supplied` held. -/
theorem require_revenue_backed_eq_ok (c : market_cache.Cache) (u : Unit)
    (h : market_cache.shares.Cache.require_revenue_backed c = ok u) :
    c.revenue.val ≤ c.supplied.val := by
  unfold market_cache.shares.Cache.require_revenue_backed at h
  rw [ray_le_eq] at h
  by_cases hle : c.revenue.val ≤ c.supplied.val
  · exact hle
  · simp [hle, massert] at h

/-- **`mint_supply` preserves INV-ACCT-01.** A successful `mint_supply(s)`
adds exactly `s` to `supplied` and changes nothing else; with the invariant
before and `s ≥ 0` it holds after. Hypothesis `s ≥ 0`: every caller mints
the output of `calculate_scaled_supply` on a non-negative amount (`supply`
validates `amount ≥ 0`), a floor of a non-negative quotient. -/
theorem mint_supply_preserves (c c' : market_cache.Cache) (s : math.fp.Ray)
    (hinv : SharesInv c) (hs : 0 ≤ s.val)
    (h : market_cache.shares.Cache.mint_supply c s = ok c') :
    SharesInv c' ∧ c'.supplied.val = c.supplied.val + s.val ∧
      c' = { c with supplied := c'.supplied } := by
  unfold market_cache.shares.Cache.mint_supply at h
  obtain ⟨r, hr, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; subst h
  have := ray_checked_add_eq_ok _ _ _ _ hr
  unfold SharesInv at *
  refine ⟨⟨hinv.1, by simp; omega, hinv.2.2⟩, this, rfl⟩

/-- **`burn_supply` preserves INV-ACCT-01, unconditionally.** A successful
`burn_supply(s)` had `0 ≤ s ≤ supplied`, subtracts exactly `s` from
`supplied`, changes nothing else, and leaves `revenue ≤ supplied` (it
asserts it). No hypothesis on `s`: the checked subtraction and the assert
reject every input that would break the invariant. -/
theorem burn_supply_preserves (c c' : market_cache.Cache) (s : math.fp.Ray)
    (hinv : SharesInv c)
    (h : market_cache.shares.Cache.burn_supply c s = ok c') :
    SharesInv c' ∧ 0 ≤ s.val ∧ s.val ≤ c.supplied.val ∧
      c'.supplied.val = c.supplied.val - s.val ∧
      c' = { c with supplied := c'.supplied } := by
  unfold market_cache.shares.Cache.burn_supply at h
  obtain ⟨r, hr, h⟩ := bind_eq_ok h
  obtain ⟨u, hu, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; subst h
  have hs := ray_checked_sub_eq_ok _ _ _ _ hr
  have hb := require_revenue_backed_eq_ok _ _ hu
  unfold SharesInv at *
  simp only at hb
  exact ⟨⟨hinv.1, hb, hinv.2.2⟩, hs.2.1, hs.2.2.1, hs.2.2.2, rfl⟩

/-- **`mint_debt` preserves INV-ACCT-01.** It adds exactly `s` to `borrowed`
and changes nothing else. Hypothesis `s ≥ 0`: callers mint
`calculate_scaled_borrow` of a validated non-negative amount. -/
theorem mint_debt_preserves (c c' : market_cache.Cache) (s : math.fp.Ray)
    (hinv : SharesInv c) (hs : 0 ≤ s.val)
    (h : market_cache.shares.Cache.mint_debt c s = ok c') :
    SharesInv c' ∧ c'.borrowed.val = c.borrowed.val + s.val ∧
      c' = { c with borrowed := c'.borrowed } := by
  unfold market_cache.shares.Cache.mint_debt at h
  obtain ⟨r, hr, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; subst h
  have := ray_checked_add_eq_ok _ _ _ _ hr
  unfold SharesInv at *
  refine ⟨⟨hinv.1, hinv.2.1, by simp; omega⟩, this, rfl⟩

/-- **`burn_debt` preserves INV-ACCT-01, unconditionally.** A successful
`burn_debt(s)` had `0 ≤ s ≤ borrowed` and subtracts exactly `s`. -/
theorem burn_debt_preserves (c c' : market_cache.Cache) (s : math.fp.Ray)
    (hinv : SharesInv c)
    (h : market_cache.shares.Cache.burn_debt c s = ok c') :
    SharesInv c' ∧ 0 ≤ s.val ∧ s.val ≤ c.borrowed.val ∧
      c'.borrowed.val = c.borrowed.val - s.val ∧
      c' = { c with borrowed := c'.borrowed } := by
  unfold market_cache.shares.Cache.burn_debt at h
  obtain ⟨r, hr, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; subst h
  have hs := ray_checked_sub_eq_ok _ _ _ _ hr
  unfold SharesInv at *
  exact ⟨⟨hinv.1, hinv.2.1, by simp; omega⟩, hs.2.1, hs.2.2.1, hs.2.2.2, rfl⟩

/-- **`accrue_revenue` preserves INV-ACCT-01 and mints revenue into supply.**
A successful `accrue_revenue(s)` adds exactly `s` to both `revenue` and
`supplied` ("revenue minting increases revenue and total supply equally")
and changes nothing else. Hypothesis `s ≥ 0`: the callers pass
`protocol_fee_shares` (a floor of a non-negative fee, capped by the
non-negative headroom) or the accrual step's revenue shares. -/
theorem accrue_revenue_preserves (c c' : market_cache.Cache) (s : math.fp.Ray)
    (hinv : SharesInv c) (hs : 0 ≤ s.val)
    (h : market_cache.shares.Cache.accrue_revenue c s = ok c') :
    SharesInv c' ∧ c'.revenue.val = c.revenue.val + s.val ∧
      c'.supplied.val = c.supplied.val + s.val ∧
      c' = { c with revenue := c'.revenue, supplied := c'.supplied } := by
  unfold market_cache.shares.Cache.accrue_revenue at h
  obtain ⟨r, hr, h⟩ := bind_eq_ok h
  obtain ⟨r1, hr1, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; subst h
  have h1 := ray_checked_add_eq_ok _ _ _ _ hr
  have h2 := ray_checked_add_eq_ok _ _ _ _ hr1
  unfold SharesInv at *
  refine ⟨⟨by simp; omega, by simp; omega, hinv.2.2⟩, h1, h2, rfl⟩

/-- **`absorb_supply_as_revenue` preserves INV-ACCT-01.** A successful call
adds exactly `s` to `revenue`, leaves `supplied` unchanged ("reclassifying
seized supply increases revenue without increasing total supply") and has
checked `revenue ≤ supplied`. Hypothesis `s ≥ 0` (the seized position's
scaled amount, validated non-negative by the seize leg). -/
theorem absorb_supply_as_revenue_preserves (c c' : market_cache.Cache) (s : math.fp.Ray)
    (hinv : SharesInv c) (hs : 0 ≤ s.val)
    (h : market_cache.shares.Cache.absorb_supply_as_revenue c s = ok c') :
    SharesInv c' ∧ c'.revenue.val = c.revenue.val + s.val ∧
      c' = { c with revenue := c'.revenue } := by
  unfold market_cache.shares.Cache.absorb_supply_as_revenue at h
  obtain ⟨r, hr, h⟩ := bind_eq_ok h
  obtain ⟨u, hu, h⟩ := bind_eq_ok h
  simp only [ok.injEq] at h; subst h
  have h1 := ray_checked_add_eq_ok _ _ _ _ hr
  have hb := require_revenue_backed_eq_ok _ _ hu
  unfold SharesInv at *
  simp only at hb
  refine ⟨⟨by simp; omega, hb, hinv.2.2⟩, h1, rfl⟩

/-! ## `burn_claimable_revenue` -/

set_option maxRecDepth 20000

/-- The outcome of `burn_claimable_revenue` returning `(amt, c')`, where
`T` is the floor value of the revenue shares in asset units:

* `amt ≤ cash`, `amt ≤ T`, `amt ≥ 0`;
* `amt = 0` exactly when nothing is claimable (`cash ≤ 0` or `T ≤ 0`), and
  then the cache is unchanged;
* a positive `amt` is `min(cash, T)` and burns `burned > 0` revenue shares
  (at most all of them) from both `revenue` and `supplied`, nothing else
  changes, and the burned shares are worth at least the payout:
  `amt * RAY * 10^(27-dec) ≤ burned * supply_index`;
* a full claim (`amt = T`) burns all revenue; a cash-limited one burns
  `⌈revenue * amt / T⌉` (`IsCeil`), the protocol-favouring ceiling. -/
def ClaimResult (c : market_cache.Cache) (T : Int) (amt : Int) (c' : market_cache.Cache) : Prop :=
  amt ≤ c.cash.val ∧ amt ≤ T ∧ 0 ≤ amt ∧
  (amt = 0 ↔ (c.cash.val ≤ 0 ∨ T ≤ 0)) ∧
  (0 < amt → amt = min c.cash.val T) ∧
  (amt = 0 → c' = c) ∧
  (0 < amt → ∃ burned : Int,
    0 < burned ∧ burned ≤ c.revenue.val ∧
    c'.revenue.val = c.revenue.val - burned ∧ c'.supplied.val = c.supplied.val - burned ∧
    c' = { c with revenue := c'.revenue, supplied := c'.supplied } ∧
    amt * (10^27 * 10^(27 - c.params.asset_decimals.val)) ≤ burned * c.supply_index.val ∧
    (amt = T → burned = c.revenue.val) ∧
    (amt < T → IsCeil (c.revenue.val * amt) T burned))

/-- **`burn_claimable_revenue` (INV-ACCT-06, INV-ACCT-01).** It never aborts
and its result satisfies `ClaimResult` for `T = ⌊revenue * supply_index /
(RAY * 10^(27-dec))⌋`. In particular the `scaled_to_burn != Ray::ZERO`
`InternalError` assert is unreachable: every positive payout burns at least
one share.

Hypotheses, with why they hold in production:
* `supply_index > 0`: the index is floored at `SUPPLY_INDEX_FLOOR_RAW =
  RAY/1000` (`apply_bad_debt_to_supply_index`) and never decreases otherwise;
* `0 ≤ revenue ≤ supplied`: INV-ACCT-01 (`SharesInv`), preserved by every
  share mutation (theorems above);
* `cash ≥ 0`: INV-ACCT-02, `credit_cash`/`debit_cash` reject negative
  amounts and insufficient reserves (`Cash.lean`);
* `decimals ≤ 27`: listed assets have at most 18 decimals;
* `revenue * supply_index < 2^127 * RAY`: the revenue's RAY value fits
  `i128`, otherwise `mul_floor` aborts with `MathOverflow`. -/
theorem burn_claimable_revenue_spec (c : market_cache.Cache)
    (hidx : 0 < c.supply_index.val) (hrev : 0 ≤ c.revenue.val)
    (hcash : 0 ≤ c.cash.val) (hle : c.revenue.val ≤ c.supplied.val)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hfit : c.revenue.val * c.supply_index.val < 2^127 * 10^27) :
    market_cache.shares.Cache.burn_claimable_revenue c ⦃ res =>
      ∃ T : Int, IsFloor (c.revenue.val * c.supply_index.val)
          (10^27 * 10^(27 - c.params.asset_decimals.val)) T ∧
        ClaimResult c T res.1.val res.2 ⦄ := by
  unfold market_cache.shares.Cache.burn_claimable_revenue market_cache.scale.Cache.unscale_supply_floor
  have hnn : 0 ≤ c.revenue.val * c.supply_index.val := by positivity
  step with unscale_supply_floor_spec c.env c.revenue c.supply_index c.params.asset_decimals hdec hnn
    (floor_fits _ _ (by norm_num) hnn hfit) as ⟨T, hT⟩
  simp only [lift, bind_ok]
  generalize hD : (10 : Int) ^ 27 * 10 ^ (27 - c.params.asset_decimals.val) = D at hT ⊢
  have hDpos : 0 < D := by rw [← hD]; positivity
  have hT0 : 0 ≤ T.val := IsFloor.nonneg hDpos hnn hT
  have hTD : T.val * D ≤ c.revenue.val * c.supply_index.val := by
    unfold IsFloor at hT; rw [if_pos hDpos] at hT; exact hT.1
  generalize hm : core.cmp.impls.OrdI128.min c.cash T = m
  have hmv : m.val = min c.cash.val T.val := by
    rw [← hm]; simp only [core.cmp.impls.OrdI128.min]
    split <;> rename_i h <;> simp only [IScalar.lt_equiv] at h <;> omega
  by_cases hpos : m ≤ 0#i128
  · simp only [hpos, ↓reduceIte, WP.spec_ok]
    have hpos' : m.val ≤ 0 := by simpa [IScalar.le_equiv] using hpos
    refine ⟨T.val, hT, ?_⟩
    have h0 : ((0#i128 : Std.I128).val) = 0 := by simp
    unfold ClaimResult
    simp only [h0]
    refine ⟨by omega, by omega, le_refl _, ⟨fun _ => by omega, fun _ => by simp⟩,
      fun h => by omega, fun _ => by simp, fun h => by omega⟩
  · simp only [hpos, ↓reduceIte]
    have hm0 : 0 < m.val := by simp [IScalar.le_equiv] at hpos; omega
    -- the revenue is positive: T ≥ m > 0
    have hTpos : 0 < T.val := by omega
    have hrevpos : 0 < c.revenue.val := by
      rcases lt_or_eq_of_le hrev with h | h
      · exact h
      · exfalso; rw [← h] at hTD; simp at hTD; nlinarith
    by_cases hfull : m ≥ T
    · have hmT : m.val = T.val := by simp [IScalar.le_equiv] at hfull; omega
      simp only [hfull, ↓reduceIte, bind_ok, ray_ne_eq, zero_val]
      have hne : decide (c.revenue.val ≠ 0) = true := by simp; omega
      simp only [hne, massert, ↓reduceIte, bind_ok]
      step with ray_checked_sub_spec c.env c.revenue c.revenue hrev (le_refl _) as ⟨r, hr⟩
      step with ray_checked_sub_spec c.env c.supplied c.revenue hrev hle as ⟨r1, hr1⟩
      refine ⟨T.val, hT, ?_⟩
      unfold ClaimResult
      simp only
      refine ⟨by omega, by omega, by omega, ⟨fun h => by omega, fun h => by omega⟩,
        fun _ => by omega, fun h => by omega, fun _ => ⟨c.revenue.val, hrevpos, le_refl _,
          hr, hr1, by simp, ?_, fun _ => rfl, fun h => by omega⟩⟩
      rw [hmT, hD]; nlinarith
    · have hmT : m.val < T.val := by simp [IScalar.le_equiv] at hfull; omega
      simp only [hfull, ↓reduceIte]
      unfold math.fp.Ray.mul_ratio_ceil
      obtain ⟨q, hq⟩ := isCeil_exists (c.revenue.val * m.val) T.val hTpos
      have hq' := hq
      unfold IsCeil at hq'; rw [if_pos hTpos] at hq'
      have hqpos : 0 < q := by
        by_contra hc; have hc := not_lt.mp hc
        have : 0 < c.revenue.val * m.val := by positivity
        nlinarith
      have hqle : q ≤ c.revenue.val := by
        by_contra hc; have hc := not_le.mp hc
        have : (c.revenue.val + 1) * T.val ≤ q * T.val := by nlinarith
        nlinarith
      step with mul_div_ceil_spec c.env c.revenue m T (by omega)
        ⟨q, hq, by omega, by have := i128_bounds c.revenue; omega⟩ as ⟨b, hb⟩
      have hbq : b.val = q := IsCeil.unique (by omega) hb hq
      have hne : decide (b.val ≠ 0) = true := by simp; omega
      simp only [ray_ne_eq, zero_val, hne, massert, ↓reduceIte, bind_ok]
      step with ray_checked_sub_spec c.env c.revenue b (by omega) (by omega) as ⟨r, hr⟩
      step with ray_checked_sub_spec c.env c.supplied b (by omega) (by omega) as ⟨r1, hr1⟩
      refine ⟨T.val, hT, ?_⟩
      unfold ClaimResult
      simp only
      refine ⟨by omega, by omega, by omega, ⟨fun h => by omega, fun h => by omega⟩,
        fun _ => by omega, fun h => by omega, fun _ => ⟨b.val, by omega, by omega,
          hr, hr1, by simp, ?_, fun h => by omega, fun _ => hb⟩⟩
      -- m * D ≤ b * idx, from b*T ≥ rev*m and T*D ≤ rev*idx
      have h1 : c.revenue.val * m.val ≤ b.val * T.val := by rw [hbq]; exact hq'.1
      have h2 : c.revenue.val * (m.val * D) ≤ c.revenue.val * (b.val * c.supply_index.val) := by
        nlinarith
      rw [hD]; exact le_of_mul_le_mul_left h2 hrevpos
/-- **`burn_claimable_revenue` preserves INV-ACCT-01.** Under the hypotheses
of `burn_claimable_revenue_spec` (which include the invariant itself), the
returned cache again satisfies `SharesInv`. -/
theorem burn_claimable_revenue_preserves (c : market_cache.Cache) (hinv : SharesInv c)
    (hidx : 0 < c.supply_index.val) (hcash : 0 ≤ c.cash.val)
    (hdec : c.params.asset_decimals.val ≤ 27)
    (hfit : c.revenue.val * c.supply_index.val < 2^127 * 10^27) :
    market_cache.shares.Cache.burn_claimable_revenue c ⦃ res => SharesInv res.2 ⦄ := by
  apply WP.spec_mono (burn_claimable_revenue_spec c hidx hinv.1 hcash hinv.2.1 hdec hfit)
  rintro ⟨amt, c'⟩ ⟨T, _, h⟩
  obtain ⟨_, _, hnn, _, _, hz, hp⟩ := h
  rcases lt_or_eq_of_le hnn with hpos | h0
  · obtain ⟨b, hb0, hbr, hr, hs, heq, _⟩ := hp hpos
    unfold SharesInv at *
    have hb' : c'.borrowed = c.borrowed := by simp only at heq; rw [heq]
    refine ⟨by omega, by omega, by rw [hb']; exact hinv.2.2⟩
  · rw [hz h0.symm]; exact hinv

end PoolAeneas.Proofs
