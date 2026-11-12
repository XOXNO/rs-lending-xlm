/-
`interest::accrue_chunk` and `interest::global_sync`
(contracts/pool/src/interest.rs). INV-IDX-04 (bounded chunks), INV-IDX-01/02,
INV-ACCT-01.

`global_sync` is reached through the `NonZeroU64` models in
`FunsExternal.lean` and the `postextract.sh` field renaming (docs/pool.md).
-/
import PoolAeneas.Proofs.Accrual

open Aeneas Aeneas.Std Result
open PoolAeneas.Spec PoolAeneas.HalfUpSpec
open pool_aeneas

set_option maxRecDepth 20000

namespace PoolAeneas.Proofs

/-- `u64::min` is the minimum of the values. -/
theorem min_u64_val (x y : Std.U64) :
    (core.cmp.impls.OrdU64.min x y).val = min x.val y.val := by
  simp only [core.cmp.impls.OrdU64.min]
  split <;> rename_i h <;> simp only [UScalar.lt_equiv] at h <;> omega

/-- `u64::saturating_sub` is truncated subtraction. -/
theorem sat_sub_u64_val (x y : Std.U64) :
    (core.num.U64.saturating_sub x y).val = x.val - y.val := by
  have hx := x.hBounds
  simp only [UScalarTy.numBits] at hx
  show (BitVec.ofNat 64 (max 0 (x.val - y.val))).toNat = x.val - y.val
  rw [BitVec.toNat_ofNat, Nat.zero_max]
  exact Nat.mod_eq_of_lt (by omega)

/-! ## `accrue_chunk` -/

/-- The production domain of one accrual chunk on a cache (hypotheses of
`accrue_step_spec`, stated on the cache fields): a validated rate model,
INV-ACCT-01, indexes in range, and the debt value after 21-fold growth and the
supply value below `2^125` RAY. -/
structure AccrualDomain (c : market_cache.Cache) : Prop where
  model : RatesProofs.ValidRateModel c.params
  shares : SharesInv c
  bi_nonneg : 0 ≤ c.borrow_index.val
  bi_le : c.borrow_index.val ≤ 10^36
  si_pos : 0 < c.supply_index.val
  si_le : c.supply_index.val ≤ 10^36
  debt_fits : c.borrowed.val * (21 * c.borrow_index.val) < 2^125 * 10^27
  supply_fits : c.supplied.val * c.supply_index.val < 2^125 * 10^27

/-- What one chunk does to a cache `c`, as a relation to the result `c'`:
INV-ACCT-01 holds, the borrow and supply indexes do not decrease and stay
capped at `10^36` (INV-IDX-01/02), `revenue` and `supplied` grow by the same
`sh ≥ 0` revenue shares (INV-ACCT-01 booking), and nothing else changes
(timestamps, `borrowed`, `cash`, parameters). -/
def ChunkPost (c c' : market_cache.Cache) : Prop :=
  SharesInv c' ∧
  c.borrow_index.val ≤ c'.borrow_index.val ∧ c'.borrow_index.val ≤ 10^36 ∧
  c.supply_index.val ≤ c'.supply_index.val ∧ c'.supply_index.val ≤ 10^36 ∧
  ∃ sh : Int, 0 ≤ sh ∧ c'.revenue.val = c.revenue.val + sh ∧
    c'.supplied.val = c.supplied.val + sh ∧
    c' = { c with borrow_index := c'.borrow_index, supply_index := c'.supply_index,
                  revenue := c'.revenue, supplied := c'.supplied }

/-- **`accrue_chunk` never aborts in the accrual domain and satisfies
`ChunkPost`.** For a chunk `delta_ms ≤ MAX_COMPOUND_DELTA_MS` on a cache in
`AccrualDomain`: indexes monotone and capped (INV-IDX-01/02), revenue minted
into `revenue` and `supplied` equally with `supplied + sh ≤ i128::MAX`, so
INV-ACCT-01 is preserved, and no other field changes. -/
theorem accrue_chunk_spec (env : Env) (c : market_cache.Cache) (k : Std.U64)
    (hD : AccrualDomain c) (hk : k.val ≤ 31556926000) :
    interest.accrue_chunk env c k ⦃ c' => ChunkPost c c' ⦄ := by
  obtain ⟨hp, hinv, hbi0, hbiM, hsi0, hsiM, hdf, hsf⟩ := hD
  have hinv' := hinv
  unfold SharesInv at hinv'
  unfold interest.accrue_chunk
  simp only [market_cache.Cache.impl.params, market_cache.Cache.impl.borrowed,
    market_cache.Cache.impl.supplied, market_cache.Cache.impl.borrow_index,
    market_cache.Cache.impl.supply_index, bind_ok]
  step with accrue_step_spec env c.params c.borrowed c.supplied c.borrow_index c.supply_index k
    hp hinv'.2.2 (by omega) hbi0 hbiM hsi0 hsiM (by omega) hdf hsf
    as ⟨st, h1, h2, h3, h4, h5, h6, _⟩
  simp only [market_cache.report.Cache.set_borrow_index, market_cache.report.Cache.set_supply_index,
    bind_ok]
  unfold market_cache.shares.Cache.accrue_revenue
  have hrb := i128_bounds c.revenue
  step with ray_checked_add_spec c.env c.revenue st.revenue_shares (by simp; omega)
    (by simp; omega) as ⟨r, hr⟩
  step with ray_checked_add_spec c.env c.supplied st.revenue_shares (by simp; omega)
    (by simp; omega) as ⟨r1, hr1⟩
  refine ⟨⟨by simp; omega, by simp; omega, by simp; omega⟩, by simpa using h1, by simpa using h2,
    by simpa using h3, by simpa using h4, st.revenue_shares.val, h5, by simpa using hr,
    by simpa using hr1, rfl⟩

/-! ## `global_sync` -/

/-- **The `global_sync` loop advances time in chunks of exactly
`min(remaining, MAX_COMPOUND_DELTA_MS)` (INV-IDX-04) and terminates.**
Induction principle: for any property `Inv remaining cache` such that every
chunk `k = min(r, MAX_COMPOUND_DELTA_MS)` taken with `r > 0` remaining maps a
cache satisfying `Inv r` to one satisfying `Inv (r − k)` (and does not abort),
the loop started with `remaining = r` does not abort and ends in a cache
satisfying `Inv 0`. So each chunk is positive and at most one year, the chunks
sum to the elapsed time, and all but the last are full. -/
theorem global_sync_loop_spec (env : Env) (Inv : Nat → market_cache.Cache → Prop)
    (hstep : ∀ (r : Nat) (k : Std.U64) (c : market_cache.Cache), Inv r c → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env c k ⦃ c' => Inv (r - k.val) c' ⦄)
    (r : Std.U64) (c : market_cache.Cache) (h : Inv r.val c) :
    interest.global_sync_loop r env c ⦃ c' => Inv 0 c' ⦄ := by
  unfold interest.global_sync_loop
  apply loop.spec_decr_nat (fun p => p.1.val) (fun p => Inv p.1.val p.2) _ _ (r, c) _ h
  rintro ⟨r1, c1⟩ hinv
  simp only [interest.global_sync_loop.body, core.num.nonzero.NonZero.new,
    core.num.nonzero.NonZero.get, bind_ok]
  by_cases h0 : r1.val = 0
  · simp only [h0, ↓reduceIte, WP.spec_ok]
    simpa [h0] using hinv
  · simp only [h0, ↓reduceIte, lift, bind_ok, max_compound_delta_val]
    have hk : (core.cmp.impls.OrdU64.min r1 31556926000#u64).val = min r1.val 31556926000 := by
      rw [min_u64_val]; rfl
    step with hstep r1.val _ c1 hinv (by omega) hk as ⟨c2, hc2⟩
    rw [sat_sub_u64_val, hk]
    rw [hk] at hc2
    refine ⟨hc2, by omega⟩

/-- `elapsed_ms` is `current_timestamp − last_timestamp`, truncated at 0. -/
theorem elapsed_val (c : market_cache.Cache) :
    market_cache.Cache.elapsed_ms c =
      ok (core.num.U64.saturating_sub c.current_timestamp c.last_timestamp) := rfl

/-- **No elapsed time: `global_sync` is a no-op (INV-IDX-04).** When
`current_timestamp ≤ last_timestamp`, the cache is returned unchanged: no
accrual and no timestamp advance. -/
theorem global_sync_noop (env : Env) (c : market_cache.Cache)
    (h : c.current_timestamp.val ≤ c.last_timestamp.val) :
    interest.global_sync env c = ok c := by
  unfold interest.global_sync market_cache.Cache.needs_accrual
  simp only [elapsed_val, bind_ok]
  have : ¬ (core.num.U64.saturating_sub c.current_timestamp c.last_timestamp > 0#u64) := by
    simp only [gt_iff_lt, UScalar.lt_equiv, sat_sub_u64_val]; simp; omega
  simp [this]

/-- **`global_sync` = chunked accrual, then the timestamp advance
(INV-IDX-04).** With `e = current_timestamp − last_timestamp` (truncated):
for any chunk invariant `Inv` as in `global_sync_loop_spec` with `Inv e c`,
`global_sync` does not abort; if `e = 0` it returns `c` unchanged, otherwise
it returns `x` with `last_timestamp := current_timestamp` for some `x`
satisfying `Inv 0` (the cache after all chunks). -/
theorem global_sync_spec_of_inv (env : Env) (Inv : Nat → market_cache.Cache → Prop)
    (hstep : ∀ (r : Nat) (k : Std.U64) (c : market_cache.Cache), Inv r c → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env c k ⦃ c' => Inv (r - k.val) c' ⦄)
    (c : market_cache.Cache) (h : Inv (c.current_timestamp.val - c.last_timestamp.val) c) :
    interest.global_sync env c ⦃ c' =>
      (c.current_timestamp.val ≤ c.last_timestamp.val ∧ c' = c) ∨
      (c.last_timestamp.val < c.current_timestamp.val ∧
        ∃ x, Inv 0 x ∧ c' = { x with last_timestamp := x.current_timestamp }) ⦄ := by
  by_cases h0 : c.current_timestamp.val ≤ c.last_timestamp.val
  · rw [global_sync_noop env c h0]; simp [h0]
  unfold interest.global_sync market_cache.Cache.needs_accrual
  simp only [elapsed_val, bind_ok]
  have hpos : core.num.U64.saturating_sub c.current_timestamp c.last_timestamp > 0#u64 := by
    simp only [gt_iff_lt, UScalar.lt_equiv, sat_sub_u64_val]; simp; omega
  simp only [hpos, decide_true, ↓reduceIte]
  step with global_sync_loop_spec env Inv hstep _ c (by rw [sat_sub_u64_val]; exact h)
    as ⟨x, hx⟩
  simp only [market_cache.Cache.mark_accrued, WP.spec_ok]
  exact Or.inr ⟨by omega, x, hx, rfl⟩

/-- **`global_sync` over at most one chunk never aborts and preserves the
accrual invariants (INV-IDX-01/02/04, INV-ACCT-01).** For a cache in
`AccrualDomain` whose elapsed time is at most `MAX_COMPOUND_DELTA_MS` (one
year; every market touched at least yearly): with no elapsed time the cache is
unchanged; otherwise exactly one chunk of the whole elapsed time is accrued
(`ChunkPost`: indexes monotone and capped, revenue booked into `revenue` and
`supplied` equally, INV-ACCT-01 kept) and `last_timestamp` is set to
`current_timestamp`. -/
theorem global_sync_one_chunk_spec (env : Env) (c : market_cache.Cache) (hD : AccrualDomain c)
    (he : c.current_timestamp.val - c.last_timestamp.val ≤ 31556926000) :
    interest.global_sync env c ⦃ c' =>
      (c.current_timestamp.val ≤ c.last_timestamp.val ∧ c' = c) ∨
      (c.last_timestamp.val < c.current_timestamp.val ∧
        c'.last_timestamp = c.current_timestamp ∧
        ∃ x, ChunkPost c x ∧ c' = { x with last_timestamp := x.current_timestamp }) ⦄ := by
  let e := c.current_timestamp.val - c.last_timestamp.val
  let Inv : Nat → market_cache.Cache → Prop := fun r x =>
    (r = e ∧ x = c) ∨ (r = 0 ∧ ChunkPost c x)
  have hstep : ∀ (r : Nat) (k : Std.U64) (x : market_cache.Cache), Inv r x → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env x k ⦃ c' => Inv (r - k.val) c' ⦄ := by
    rintro r k x (⟨hr, rfl⟩ | ⟨hr, _⟩) hr0 hk
    · apply WP.spec_mono (accrue_chunk_spec env x k hD (by omega))
      intro c' hc'
      exact Or.inr ⟨by omega, hc'⟩
    · omega
  apply WP.spec_mono (global_sync_spec_of_inv env Inv hstep c (Or.inl ⟨rfl, rfl⟩))
  rintro c' (h | ⟨hlt, x, hx, rfl⟩)
  · exact Or.inl h
  · rcases hx with ⟨h0, _⟩ | ⟨_, hx⟩
    · omega
    · refine Or.inr ⟨hlt, ?_, x, hx, rfl⟩
      obtain ⟨_, _, _, _, _, _, _, _, _, hxe⟩ := hx
      rw [hxe]

