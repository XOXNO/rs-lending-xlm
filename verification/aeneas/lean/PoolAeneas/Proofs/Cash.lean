/-
Cash reserve bookkeeping of `contracts/pool/src/cache/cash.rs` (INV-ACCT-02).

`cash` is the market's accounting reserve book, separate from the token
balance. Credits and debits reject negative amounts; credits reject
overflow; debits reject insufficient reserves. Each changes only `cash`.
-/
import PoolAeneas.Proofs.Basic

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec
open pool_aeneas

namespace PoolAeneas.Proofs

/-- `validation::require_nonneg_amount` passes exactly on `amount ≥ 0`. -/
theorem require_nonneg_amount_iff (env : Env) (a : Std.I128) :
    (∃ u, validation.require_nonneg_amount env a = ok u) ↔ 0 ≤ a.val := by
  unfold validation.require_nonneg_amount
  by_cases h : 0 ≤ a.val
  · have : a >= 0#i128 := by simp [IScalar.le_equiv]; omega
    simp [this, massert, h]
  · have : ¬ (a >= 0#i128) := by simp [IScalar.le_equiv]; omega
    simp [this, massert, h]

/-- **`require_reserves` (INV-ACCT-02).** Passes exactly when `cash ≥ amount`. -/
theorem require_reserves_iff (c : market_cache.Cache) (a : Std.I128) :
    (∃ u, market_cache.cash.Cache.require_reserves c a = ok u) ↔ a.val ≤ c.cash.val := by
  unfold market_cache.cash.Cache.require_reserves
  by_cases h : a.val ≤ c.cash.val
  · have : c.cash >= a := by simp [IScalar.le_equiv]; omega
    simp [this, massert, h]
  · have : ¬ (c.cash >= a) := by simp [IScalar.le_equiv]; omega
    simp [this, massert, h]

/-- **`credit_cash` (INV-ACCT-02).** It returns exactly when `amount ≥ 0` and
`cash + amount` fits `i128`; then the new cache is the old one with
`cash + amount` and nothing else changed. No hypotheses. -/
theorem credit_cash_spec (c : market_cache.Cache) (a : Std.I128) :
    ((∃ c', market_cache.cash.Cache.credit_cash c a = ok c') ↔
        (0 ≤ a.val ∧ c.cash.val + a.val < 2^127)) ∧
    (∀ c', market_cache.cash.Cache.credit_cash c a = ok c' →
        c'.cash.val = c.cash.val + a.val ∧ c' = { c with cash := c'.cash }) := by
  have hcb := i128_bounds c.cash
  have hab := i128_bounds a
  unfold market_cache.cash.Cache.credit_cash validation.require_nonneg_amount
  have hs := I128.checked_add_bv_spec c.cash a
  rw [show I128.checked_add c.cash a = core.num.checked_add_IScalar c.cash a from rfl]
  by_cases hn : 0 ≤ a.val
  swap
  · have : ¬ (a >= 0#i128) := by simp [IScalar.le_equiv]; omega
    simp [this, massert, hn]
  have hn' : a >= 0#i128 := by simp [IScalar.le_equiv]; omega
  simp only [hn', massert, ↓reduceIte, bind_ok, lift]
  cases hc : core.num.checked_add_IScalar c.cash a with
  | none =>
    rw [hc] at hs
    simp only [I128.min_eq, I128.max_eq] at hs
    simp [core.option.Option.unwrap_or_else,
      market_cache.cash.Cache.credit_cash.closure.Insts.CoreOpsFunctionFnOnceTupleI128.call_once]
    omega
  | some z =>
    rw [hc] at hs
    simp only [I128.min_eq, I128.max_eq] at hs
    simp only [core.option.Option.unwrap_or_else, bind_ok]
    refine ⟨⟨fun _ => ⟨hn, by omega⟩, fun _ => ⟨_, rfl⟩⟩, ?_⟩
    intro c' h; simp only [ok.injEq] at h; rw [← h]
    exact ⟨hs.2.2.1, rfl⟩

/-- **`debit_cash` (INV-ACCT-02).** It returns exactly when
`0 ≤ amount ≤ cash`; then the new cache is the old one with `cash - amount`
(so `cash` stays non-negative) and nothing else changed. No hypotheses. -/
theorem debit_cash_spec (c : market_cache.Cache) (a : Std.I128) :
    ((∃ c', market_cache.cash.Cache.debit_cash c a = ok c') ↔
        (0 ≤ a.val ∧ a.val ≤ c.cash.val)) ∧
    (∀ c', market_cache.cash.Cache.debit_cash c a = ok c' →
        c'.cash.val = c.cash.val - a.val ∧ 0 ≤ c'.cash.val ∧ c' = { c with cash := c'.cash }) := by
  have hcb := i128_bounds c.cash
  have hab := i128_bounds a
  unfold market_cache.cash.Cache.debit_cash validation.require_nonneg_amount
    market_cache.cash.Cache.require_reserves
  have hs := I128.checked_sub_bv_spec c.cash a
  rw [show I128.checked_sub c.cash a = core.num.checked_sub_IScalar c.cash a from rfl]
  by_cases hn : 0 ≤ a.val
  swap
  · have : ¬ (a >= 0#i128) := by simp [IScalar.le_equiv]; omega
    simp [this, massert, hn]
  have hn' : a >= 0#i128 := by simp [IScalar.le_equiv]; omega
  simp only [hn', massert, ↓reduceIte, bind_ok, lift]
  by_cases hr : a.val ≤ c.cash.val
  swap
  · have : ¬ (c.cash >= a) := by simp [IScalar.le_equiv]; omega
    simp [this, hr]
  have hr' : c.cash >= a := by simp [IScalar.le_equiv]; omega
  simp only [hr', ↓reduceIte, bind_ok]
  cases hc : core.num.checked_sub_IScalar c.cash a with
  | none =>
    rw [hc] at hs
    simp only [I128.min_eq, I128.max_eq] at hs
    exfalso; apply hs; omega
  | some z =>
    rw [hc] at hs
    simp only [I128.min_eq, I128.max_eq] at hs
    simp only [core.option.Option.unwrap_or_else, bind_ok]
    refine ⟨⟨fun _ => ⟨hn, hr⟩, fun _ => ⟨_, rfl⟩⟩, ?_⟩
    intro c' h; simp only [ok.injEq] at h; rw [← h]
    exact ⟨hs.2.2.1, by simp; omega, rfl⟩

end PoolAeneas.Proofs
