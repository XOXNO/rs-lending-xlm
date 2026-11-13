/-
`interest::accrue_chunk` and `interest::global_sync`
(contracts/pool/src/interest.rs). INV-IDX-04 (bounded chunks), INV-IDX-01/02,
INV-ACCT-01.

`global_sync` is reached through the `NonZeroU64` models in
`FunsExternal.lean` and the `postextract.sh` field renaming (docs/pool.md).
-/
import PoolAeneas.Proofs.Accrual

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
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

/-- The stored-state invariants accrual relies on, on a cache (hypotheses of
`accrue_step_spec`, stated on the cache fields): a validated rate model,
INV-ACCT-01 and indexes in range. There is no hypothesis on the size of the
market values: accrual holds interest at `MAX_MARKET_VALUE_RAY` instead of
overflowing (INV-IDX-06). -/
structure AccrualDomain (c : market_cache.Cache) : Prop where
  model : RatesProofs.ValidRateModel c.params
  shares : SharesInv c
  bi_nonneg : 0 ≤ c.borrow_index.val
  bi_le : c.borrow_index.val ≤ 10^36
  si_pos : 0 < c.supply_index.val
  si_le : c.supply_index.val ≤ 10^36

/-- What accrual does to a cache `c`, as a relation to the result `c'`:
INV-ACCT-01 holds, the borrow and supply indexes do not decrease and stay
capped at `10^36` (INV-IDX-01/02), `revenue` and `supplied` grow by the same
`sh ≥ 0` revenue shares (INV-ACCT-01 booking), and nothing else changes
(timestamps, `borrowed`, `cash`, parameters). The relation is reflexive on
`AccrualDomain` and transitive, so it describes one chunk and any number of
chunks alike. -/
def ChunkPost (c c' : market_cache.Cache) : Prop :=
  SharesInv c' ∧
  c.borrow_index.val ≤ c'.borrow_index.val ∧ c'.borrow_index.val ≤ 10^36 ∧
  c.supply_index.val ≤ c'.supply_index.val ∧ c'.supply_index.val ≤ 10^36 ∧
  ∃ sh : Int, 0 ≤ sh ∧ c'.revenue.val = c.revenue.val + sh ∧
    c'.supplied.val = c.supplied.val + sh ∧
    c' = { c with borrow_index := c'.borrow_index, supply_index := c'.supply_index,
                  revenue := c'.revenue, supplied := c'.supplied }

/-- No accrual: a cache in `AccrualDomain` is related to itself. -/
theorem ChunkPost.refl {c : market_cache.Cache} (hD : AccrualDomain c) : ChunkPost c c :=
  ⟨hD.shares, le_refl _, hD.bi_le, le_refl _, hD.si_le, 0, le_refl _, by omega, by omega, rfl⟩

/-- Chunks compose: the indexes stay monotone and capped, and the revenue
shares add up. -/
theorem ChunkPost.trans {c c' c'' : market_cache.Cache} (h1 : ChunkPost c c')
    (h2 : ChunkPost c' c'') : ChunkPost c c'' := by
  obtain ⟨_, hb1, _, hs1, _, sh1, hsh1, hr1, hsu1, he1⟩ := h1
  obtain ⟨hinv, hb2, hbM, hs2, hsM, sh2, hsh2, hr2, hsu2, he2⟩ := h2
  refine ⟨hinv, by omega, hbM, by omega, hsM, sh1 + sh2, by omega, by omega, by omega, ?_⟩
  rw [he2, he1]

/-- A chunk keeps the cache in `AccrualDomain`: the parameters do not change,
INV-ACCT-01 holds after, and the indexes stay in range. -/
theorem AccrualDomain.of_chunk {c c' : market_cache.Cache} (hD : AccrualDomain c)
    (h : ChunkPost c c') : AccrualDomain c' := by
  obtain ⟨hinv, hb1, hbM, hs1, hsM, _, _, _, _, he⟩ := h
  have hpar : c'.params = c.params := by rw [he]
  have hbn := hD.bi_nonneg
  have hsp := hD.si_pos
  exact ⟨hpar ▸ hD.model, hinv, by omega, hbM, by omega, hsM⟩

/-- **`accrue_chunk` never aborts in the accrual domain and satisfies
`ChunkPost`.** For a chunk `delta_ms ≤ MAX_COMPOUND_DELTA_MS` on a cache in
`AccrualDomain`, whatever the size of its market values: indexes monotone and
capped (INV-IDX-01/02), revenue minted into `revenue` and `supplied` equally
with `supplied + sh ≤ i128::MAX`, so INV-ACCT-01 is preserved, and no other
field changes. The returned flag says whether the value ceiling held the
chunk's interest. -/
theorem accrue_chunk_spec (env : Env) (c : market_cache.Cache) (k : Std.U64)
    (hD : AccrualDomain c) (hk : k.val ≤ 31556926000) :
    interest.accrue_chunk env c k ⦃ y => ChunkPost c y.2 ⦄ := by
  obtain ⟨hp, hinv, hbi0, hbiM, hsi0, hsiM⟩ := hD
  have hinv' := hinv
  unfold SharesInv at hinv'
  unfold interest.accrue_chunk
  simp only [market_cache.Cache.impl.params, market_cache.Cache.impl.borrowed,
    market_cache.Cache.impl.supplied, market_cache.Cache.impl.borrow_index,
    market_cache.Cache.impl.supply_index, bind_ok]
  step with accrue_step_spec env c.params c.borrowed c.supplied c.borrow_index c.supply_index k
    hp hinv'.2.2 (by omega) hbi0 hbiM hsi0 hsiM (by omega)
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
sum to the elapsed time, and all but the last are full. The loop also carries
the "ceiling held a chunk" flag, which no chunk reads. -/
theorem global_sync_loop_spec (env : Env) (Inv : Nat → market_cache.Cache → Prop)
    (hstep : ∀ (r : Nat) (k : Std.U64) (c : market_cache.Cache), Inv r c → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env c k ⦃ y => Inv (r - k.val) y.2 ⦄)
    (r : Std.U64) (c : market_cache.Cache) (vc : Bool) (h : Inv r.val c) :
    interest.global_sync_loop r env c vc ⦃ y => Inv 0 y.1 ⦄ := by
  unfold interest.global_sync_loop
  apply loop.spec_decr_nat (fun p => p.1.val) (fun p => Inv p.1.val p.2.1) _ _ (r, c, vc) _ h
  rintro ⟨r1, c1, v1⟩ hinv
  simp only [interest.global_sync_loop.body, core.num.nonzero.NonZero.new,
    core.num.nonzero.NonZero.get, bind_ok]
  by_cases h0 : r1.val = 0
  · simp only [h0, ↓reduceIte, WP.spec_ok]
    simpa [h0] using hinv
  · simp only [h0, ↓reduceIte, lift, bind_ok, max_compound_delta_val]
    have hk : (core.cmp.impls.OrdU64.min r1 31556926000#u64).val = min r1.val 31556926000 := by
      rw [min_u64_val]; rfl
    step with hstep r1.val _ c1 hinv (by omega) hk as ⟨b, c2, hc2⟩
    rw [hk] at hc2
    cases b <;>
      simp only [Bool.false_eq_true, ↓reduceIte, bind_ok, WP.spec_ok] <;>
      rw [sat_sub_u64_val, hk] <;>
      exact ⟨hc2, by omega⟩

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
satisfying `Inv 0` (the cache after all chunks). The value-ceiling event, when
emitted, changes nothing in the cache. -/
theorem global_sync_spec_of_inv (env : Env) (Inv : Nat → market_cache.Cache → Prop)
    (hstep : ∀ (r : Nat) (k : Std.U64) (c : market_cache.Cache), Inv r c → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env c k ⦃ y => Inv (r - k.val) y.2 ⦄)
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
  step with global_sync_loop_spec env Inv hstep _ c false (by rw [sat_sub_u64_val]; exact h)
    as ⟨x, vc, hx⟩
  simp only [market_cache.Cache.mark_accrued, bind_ok]
  cases vc <;>
    simp only [Bool.false_eq_true, ↓reduceIte, market_cache.Cache.impl.hub_asset,
      events.emit_market_value_ceiling, bind_ok, WP.spec_ok] <;>
    exact Or.inr ⟨by omega, x, hx, rfl⟩

/-- **`global_sync` never aborts, for any elapsed time, and preserves the
accrual invariants (INV-IDX-06, INV-IDX-01/02/04, INV-ACCT-01; audit finding
S-04).** For a cache in `AccrualDomain`, whatever the elapsed time and the
size of the market values: with no elapsed time the cache is unchanged;
otherwise the elapsed time is accrued in chunks of at most
`MAX_COMPOUND_DELTA_MS`, the result `x` satisfies `ChunkPost c x` (indexes
monotone and capped, revenue booked into `revenue` and `supplied` equally,
INV-ACCT-01 kept) and is again in `AccrualDomain`, and `last_timestamp` is set
to `current_timestamp`. Every pool leg accrues first, so no stored state that
satisfies the invariants can make an exit path abort in accrual. -/
theorem global_sync_spec (env : Env) (c : market_cache.Cache) (hD : AccrualDomain c) :
    interest.global_sync env c ⦃ c' =>
      (c.current_timestamp.val ≤ c.last_timestamp.val ∧ c' = c) ∨
      (c.last_timestamp.val < c.current_timestamp.val ∧
        c'.last_timestamp = c.current_timestamp ∧
        ∃ x, ChunkPost c x ∧ AccrualDomain x ∧
          c' = { x with last_timestamp := x.current_timestamp }) ⦄ := by
  let Inv : Nat → market_cache.Cache → Prop := fun _ x => ChunkPost c x ∧ AccrualDomain x
  have hstep : ∀ (r : Nat) (k : Std.U64) (x : market_cache.Cache), Inv r x → 0 < r →
      k.val = min r 31556926000 →
      interest.accrue_chunk env x k ⦃ y => Inv (r - k.val) y.2 ⦄ := by
    rintro r k x ⟨hcx, hDx⟩ _ hk
    apply WP.spec_mono (accrue_chunk_spec env x k hDx (by omega))
    intro y hy
    exact ⟨hcx.trans hy, hDx.of_chunk hy⟩
  apply WP.spec_mono (global_sync_spec_of_inv env Inv hstep c ⟨ChunkPost.refl hD, hD⟩)
  rintro c' (h | ⟨hlt, x, ⟨hx, hDx⟩, rfl⟩)
  · exact Or.inl h
  · refine Or.inr ⟨hlt, ?_, x, hx, hDx, rfl⟩
    obtain ⟨_, _, _, _, _, _, _, _, _, hxe⟩ := hx
    rw [hxe]
