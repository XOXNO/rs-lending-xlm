/-
Flash-loan arithmetic of `contracts/pool/src/ops/flash.rs`: the fee and the
expected balances `terms` computes (INV-FLASH-01).

The token calls and the receiver callback are outside the model; the theorem
is about the numbers the balance checks compare against.
-/
import PoolAeneas.Proofs.Interest
import PoolAeneas.Proofs.Cash

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec PoolAeneas.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- The half-up quotient exists (`Int.ediv` of `2n + d` by `2d`). -/
theorem isHalfUp_exists (n d : Int) (hd : 0 < d) : ∃ q, IsHalfUp n d q :=
  ⟨(2 * n + d) / (2 * d), (IsHalfUp.iff_isFloor hd).mpr (isFloor_ediv _ _ (by omega))⟩

/-- `BPS` is 10000. -/
theorem bps_const_val : (constants.shared.BPS).val = 10000 := by
  unfold constants.shared.BPS; simp

/-- `Bps::flash_loan_fee_on(amount)` for `0 ≤ bps ≤ 10000` and `amount ≥ 0`:
`amount * bps / 10000` rounded half up, or 1 when the rate is positive and
that rounds to zero. Never aborts. -/
theorem flash_loan_fee_on_spec (env : Env) (k amount : Std.I128)
    (hk0 : 0 ≤ k.val) (hk : k.val ≤ 10000) (hamt : 0 ≤ amount.val) :
    math.fp.Bps.flash_loan_fee_on k env amount ⦃ f => ∃ h : Int,
      IsHalfUp (amount.val * k.val) 10000 h ∧ 0 ≤ h ∧ h ≤ amount.val ∧
      f.val = (if 0 < k.val ∧ h = 0 then 1 else h) ⦄ := by
  unfold math.fp.Bps.flash_loan_fee_on math.fp.Bps.apply_to math.fp.Bps.raw
  obtain ⟨h, hh⟩ := isHalfUp_exists (amount.val * k.val) 10000 (by norm_num)
  have hh' := hh; unfold IsHalfUp at hh'
  have hh0 : 0 ≤ h := by nlinarith
  have hhle : h ≤ amount.val := by nlinarith
  have hab := i128_bounds amount
  step with mul_div_half_up_spec env amount k constants.shared.BPS hamt hk0
    (by rw [bps_const_val]; norm_num) ⟨h, by rw [bps_const_val]; exact hh, by omega⟩ as ⟨f, hf⟩
  rw [bps_const_val] at hf
  have hfh : f.val = h := IsHalfUp.unique (by norm_num) hf hh
  by_cases hkp : k > 0#i128
  · have hkp' : 0 < k.val := by simpa [IScalar.lt_equiv] using hkp
    simp only [hkp, ↓reduceIte]
    by_cases hf0 : f = 0#i128
    · have : h = 0 := by rw [← hfh, hf0]; simp
      simp only [hf0, ↓reduceIte, WP.spec_ok]
      exact ⟨h, hh, hh0, hhle, by simp [hkp', this]⟩
    · have : h ≠ 0 := by
        intro h0; apply hf0; apply IScalar.eq_of_val_eq; rw [hfh, h0]; simp
      simp only [hf0, ↓reduceIte, WP.spec_ok]
      exact ⟨h, hh, hh0, hhle, by simp [this, hfh]⟩
  · have hkp' : ¬ 0 < k.val := by simpa [IScalar.lt_equiv] using hkp
    simp only [hkp, ↓reduceIte, WP.spec_ok]
    exact ⟨h, hh, hh0, hhle, by simp [hkp', hfh]⟩

/-- **Flash-loan terms are exact (INV-FLASH-01).** For principal `P > 0`,
fee rate `bps ≤ 10000` and pre-loan pool balance `B ≥ P` with `B + P` fitting
`i128`, `terms` returns (never aborts) with

* fee `F = h`, where `h` is `P * bps / 10000` rounded half up, except that a
  positive rate that rounds to zero charges `F = 1` (`flash_loan_fee_on`);
  `0 ≤ F ≤ P`, and `F > 0` whenever `bps > 0`;
* `total_repayment = P + F`;
* `balance_after_payout = B - P` and `balance_after_repayment = B + F`, so
  the post-repayment balance is the post-payout balance plus exactly the
  amount pulled: `(B - P) + (P + F) = B + F`.

Hypotheses: `P > 0` (`prepare` calls `require_positive_amount`); `bps ≤ 10000`
(market params enforce `flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`);
`P ≤ B`: the principal is paid out of the pool's balance `B`, so the
payout transfer fails otherwise; `B + P < 2^127` (token supplies are far
below). -/
theorem terms_spec (env : Env) (amount : Std.I128) (fee_bps : Std.U32) (pre : Std.I128)
    (hamt : 0 < amount.val) (hbps : fee_bps.val ≤ 10000) (hpre : amount.val ≤ pre.val)
    (hfit : pre.val + amount.val < 2^127) :
    ops.flash.terms env amount fee_bps pre ⦃ t =>
      ∃ h : Int, IsHalfUp (amount.val * fee_bps.val) 10000 h ∧
        t.fee.val = (if 0 < fee_bps.val ∧ h = 0 then 1 else h) ∧
        0 ≤ t.fee.val ∧ t.fee.val ≤ amount.val ∧ (0 < fee_bps.val → 0 < t.fee.val) ∧
        t.total_repayment.val = amount.val + t.fee.val ∧
        t.balance_after_payout.val = pre.val - amount.val ∧
        t.balance_after_repayment.val = pre.val + t.fee.val ∧
        t.balance_after_payout.val + t.total_repayment.val = t.balance_after_repayment.val ⦄ := by
  unfold ops.flash.terms
  -- i128::from(fee_bps)
  simp only [I128.Insts.CoreConvertFromU32.from, IScalar.tryMk]
  generalize hx : (fee_bps.val : Int) = x
  have hk := IScalar.tryMkOpt_eq IScalarTy.I128 x
  cases hko : IScalar.tryMkOpt IScalarTy.I128 x with
  | none =>
    rw [hko] at hk; exfalso; apply hk
    simp only [IScalar.inBounds, IScalarTy.numBits]; omega
  | some k =>
    rw [hko] at hk
    have hkv : k.val = fee_bps.val := by rw [hk.1, ← hx]
    simp only [Result.ofOption, bind_ok]
    subst hx
    simp only [math.fp.Bps.from, core.convert.IntoFrom, core.convert.IntoFrom.into,
      core.convert.FromSame, core.convert.FromSame.from, bind_ok]
    step with flash_loan_fee_on_spec env k amount (by rw [hkv]; omega) (by rw [hkv]; omega) (by omega) as ⟨F, h, hh, hh0, hhle, hF⟩
    rw [hkv] at hF hh
    have hF0 : 0 ≤ F.val := by rw [hF]; split <;> omega
    have hFle : F.val ≤ amount.val := by rw [hF]; split <;> omega
    have hab := i128_bounds amount
    have hpb := i128_bounds pre
    simp only [lift]
    have s1 := I128.checked_add_bv_spec amount F
    have s2 := I128.checked_sub_bv_spec pre amount
    have s3 := I128.checked_add_bv_spec pre F
    rw [show I128.checked_add amount F = core.num.checked_add_IScalar amount F from rfl,
      show I128.checked_sub pre amount = core.num.checked_sub_IScalar pre amount from rfl,
      show I128.checked_add pre F = core.num.checked_add_IScalar pre F from rfl]
    cases h1 : core.num.checked_add_IScalar amount F with
    | none => rw [h1] at s1; simp only [I128.min_eq, I128.max_eq] at s1; exfalso; apply s1; omega
    | some t1 =>
    cases h2 : core.num.checked_sub_IScalar pre amount with
    | none => rw [h2] at s2; simp only [I128.min_eq, I128.max_eq] at s2; exfalso; apply s2; omega
    | some t2 =>
    cases h3 : core.num.checked_add_IScalar pre F with
    | none => rw [h3] at s3; simp only [I128.min_eq, I128.max_eq] at s3; exfalso; apply s3; omega
    | some t3 =>
    rw [h1] at s1; rw [h2] at s2; rw [h3] at s3
    simp only [bind_ok, core.option.Option.unwrap_or_else, WP.spec_ok]
    have hcast : (0 : Int) < ((fee_bps.val : Nat) : Int) ↔ 0 < fee_bps.val := Int.natCast_pos
    have hF' : F.val = (if 0 < fee_bps.val ∧ h = 0 then 1 else h) := by
      simp only [hcast] at hF; exact hF
    refine ⟨h, hh, hF', hF0, hFle, fun hp => by rw [hF]; split <;> omega,
      s1.2.2.1, s2.2.2.1, s3.2.2.1, by rw [s1.2.2.1, s2.2.2.1, s3.2.2.1]; ring⟩

/-- **`book_fee` books the flash fee into cash and revenue.** For a fee
`F ≥ 0` in token units, the market's `cash` grows by exactly `F` and
`revenue` and `supplied` both grow by the same `s ≥ 0` shares (the fee's
RAY value `F * 10^(27-dec)` converted by `add_protocol_revenue`, floor
rounded and capped); nothing else changes and INV-ACCT-01 is preserved.
Never aborts under the hypotheses.

Hypotheses: INV-ACCT-01, `supply_index > 0`, `decimals < 27` (listed assets
have at most 18), the fee's RAY value and `cash + F` fit `i128`. -/
theorem book_fee_spec (c : market_cache.Cache) (fee : Std.I128)
    (hinv : SharesInv c) (hidx : 0 < c.supply_index.val) (hfee : 0 ≤ fee.val)
    (hdec : c.params.asset_decimals.val < 27)
    (hA : fee.val * 10^(27 - c.params.asset_decimals.val) < 2^127)
    (hcash : c.cash.val + fee.val < 2^127) :
    ops.flash.book_fee c fee ⦃ c' =>
      SharesInv c' ∧ c'.cash.val = c.cash.val + fee.val ∧
      ∃ s : Int, 0 ≤ s ∧ c'.revenue.val = c.revenue.val + s ∧
        c'.supplied.val = c.supplied.val + s ∧
        c' = { c with cash := c'.cash, revenue := c'.revenue, supplied := c'.supplied } ⦄ := by
  unfold ops.flash.book_fee
  simp only [market_cache.Cache.impl.env, market_cache.Cache.impl.params, bind_ok]
  have hp : (0 : Int) ≤ 10^(27 - c.params.asset_decimals.val) := by positivity
  step with from_asset_spec c.env fee c.params.asset_decimals hdec
    ⟨by nlinarith, hA⟩ as ⟨pf, hpf⟩
  step with add_protocol_revenue_spec c pf hinv hidx (by rw [hpf]; positivity) as ⟨c1, hc1, hz, hnz⟩
  have hc1eq : c1.cash = c.cash ∧ (∃ s : Int, 0 ≤ s ∧ c1.revenue.val = c.revenue.val + s ∧
      c1.supplied.val = c.supplied.val + s ∧
      c1 = { c with revenue := c1.revenue, supplied := c1.supplied }) := by
    by_cases h0 : pf.val = 0
    · rw [hz h0]; exact ⟨rfl, 0, le_refl _, by simp, by simp, rfl⟩
    · obtain ⟨s, hs0, _, hr, hs, heq⟩ := hnz h0
      refine ⟨by rw [heq], s, hs0, hr, hs, heq⟩
  obtain ⟨hcc, s, hs0, hr, hs, heq⟩ := hc1eq
  have hok : ∃ c2, market_cache.cash.Cache.credit_cash c1 fee = ok c2 :=
    (credit_cash_spec c1 fee).1.mpr ⟨hfee, by rw [hcc]; exact hcash⟩
  obtain ⟨c2, hc2⟩ := hok
  obtain ⟨hcv, hc2eq⟩ := (credit_cash_spec c1 fee).2 c2 hc2
  rw [hc2]
  simp only [WP.spec_ok]
  have hrev : c2.revenue = c1.revenue := by rw [hc2eq]
  have hsup : c2.supplied = c1.supplied := by rw [hc2eq]
  have hbor : c2.borrowed = c1.borrowed := by rw [hc2eq]
  refine ⟨?_, by rw [hcv, hcc], s, hs0, by rw [hrev, hr], by rw [hsup, hs], ?_⟩
  · unfold SharesInv at *; rw [hrev, hsup, hbor]; exact hc1
  · rw [hc2eq, heq]

end PoolAeneas.Proofs
