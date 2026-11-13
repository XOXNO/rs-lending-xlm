/-
StableSwap LP pricing, `common/src/oracle/lp_stable.rs`: structural
properties of `solve_stable_d` and `fair_stable_lp_price_wad` that hold for
every input, stated as equalities of the whole computation (same value, same
error, or the same abort).
-/
import OracleAeneas.Proofs.Lp

open Aeneas Aeneas.Std Result
open OracleAeneas.Spec
open oracle_aeneas

set_option maxRecDepth 20000

namespace OracleAeneas.Proofs

/-- `i128::min` is commutative. -/
theorem i128_min_comm (x y : Std.I128) :
    core.cmp.impls.OrdI128.min x y = core.cmp.impls.OrdI128.min y x := by
  apply IScalar.eq_of_val_eq
  simp only [core.cmp.impls.OrdI128.min_val]; omega

/-- `i128::max` is commutative. -/
theorem i128_max_comm (x y : Std.I128) :
    core.cmp.impls.OrdI128.max x y = core.cmp.impls.OrdI128.max y x := by
  apply IScalar.eq_of_val_eq
  simp only [core.cmp.impls.OrdI128.max_val]; omega

/-- `RangeInclusive::contains` on `MIN_AMP..=MAX_AMP` is `1 ≤ amp ≤ 1_000_000`. -/
theorem amp_contains (amp : Std.U128) :
    core.ops.range.RangeInclusive.contains core.cmp.PartialOrdU128 core.cmp.PartialOrdU128
      core.cmp.PartialOrdU128 ⟨oracle.lp_stable.MIN_AMP, oracle.lp_stable.MAX_AMP, false⟩ amp
      = ok (decide (1 ≤ amp.val ∧ amp.val ≤ 1000000)) := by
  have h1 : oracle.lp_stable.MIN_AMP = 1#u128 := by unfold oracle.lp_stable.MIN_AMP; rfl
  have h2 : oracle.lp_stable.MAX_AMP = 1000000#u128 := by unfold oracle.lp_stable.MAX_AMP; rfl
  rw [h1, h2]
  unfold core.ops.range.RangeInclusive.contains
  by_cases ha : 1 ≤ amp.val <;> by_cases hb : amp.val ≤ 1000000 <;>
    simp [core.cmp.impls.PartialOrdU128.le, ha, hb]

/-- `MAX_NORMALIZED_RESERVE_WAD` (`10u128.pow(34)`) evaluates to `10^34`. -/
theorem max_normalized_reserve_eq :
    ∃ M : Std.U128, oracle.lp_stable.MAX_NORMALIZED_RESERVE_WAD = ok M ∧ M.val = 10^34 := by
  unfold oracle.lp_stable.MAX_NORMALIZED_RESERVE_WAD core.num.U128.pow
  have h10 : (10#u128 : Std.U128).val = 10 := by simp
  have h34 : (34#u32 : Std.U32).val = 34 := by simp
  rw [h10, h34]
  generalize hx : (10 : Nat) ^ 34 = x
  have hxb : x < 2^128 := by rw [← hx]; norm_num
  have hp := UScalar.tryMkOpt_eq .U128 x
  simp only [UScalar.tryMk]
  cases ho : UScalar.tryMkOpt .U128 x with
  | none =>
    exfalso; rw [ho] at hp; apply hp
    simp only [UScalar.inBounds, UScalarTy.numBits]; exact hxb
  | some M =>
    rw [ho] at hp
    exact ⟨M, by simp [Result.ofOption], hp.1⟩

/-- **`solve_stable_d` does not depend on the order of the reserves.** For
every pair of reserves and every amplification, `solve_stable_d(xa, xb, amp)`
and `solve_stable_d(xb, xa, amp)` are the same computation: same `D`, same
error, same abort. The function sorts the reserves into `min`/`max` before
the Newton loop; this is the property the doc comment ("The result does not
depend on the order of the two reserves") and the unit test
`imbalanced_pool_solves_exactly_in_either_leg_order` state. -/
theorem solve_stable_d_symmetric (env : aeneas_oracle_shim.Env) (xa xb : Std.I128)
    (amp : Std.U128) :
    oracle.lp_stable.solve_stable_d env xa xb amp = oracle.lp_stable.solve_stable_d env xb xa amp := by
  unfold oracle.lp_stable.solve_stable_d
  simp only [core.ops.range.RangeInclusive.new, bind_ok, amp_contains, lift]
  by_cases h1 : xa ≤ 0#i128 <;> by_cases h2 : xb ≤ 0#i128 <;> simp only [h1, h2, ↓reduceIte]
  by_cases h3 : 1 ≤ amp.val ∧ amp.val ≤ 1000000 <;> simp only [h3, decide_false,
    ↓reduceIte, Bool.false_eq_true]
  obtain ⟨M, hM, _⟩ := max_normalized_reserve_eq
  rw [hM]
  simp only [bind_ok]
  by_cases h4 : IScalar.hcast .U128 xa > M <;> by_cases h5 : IScalar.hcast .U128 xb > M <;>
    simp only [h4, h5, ↓reduceIte]
  rw [i128_min_comm, i128_max_comm]

/-- **Out-of-range amplification is rejected by `solve_stable_d`.** For every
pair of reserves and every `amp` outside `[MIN_AMP, MAX_AMP] = [1, 10^6]`,
`solve_stable_d` returns `Err(InvalidPrice)` without entering the Newton loop
and without aborting. -/
theorem solve_stable_d_rejects_amp (env : aeneas_oracle_shim.Env) (xa xb : Std.I128)
    (amp : Std.U128) (hamp : amp.val < 1 ∨ 1000000 < amp.val) :
    oracle.lp_stable.solve_stable_d env xa xb amp = ok (.Err .InvalidPrice) := by
  unfold oracle.lp_stable.solve_stable_d
  have h3 : ¬ (1 ≤ amp.val ∧ amp.val ≤ 1000000) := by omega
  simp only [core.ops.range.RangeInclusive.new, bind_ok, amp_contains, h3, decide_false,
    Bool.false_eq_true, ↓reduceIte]
  split <;> [rfl; split <;> rfl]

/-- **Non-positive or over-cap reserves are rejected by `solve_stable_d`.**
If either WAD-normalised reserve is zero or negative, or above
`MAX_NORMALIZED_RESERVE_WAD = 10^34`, `solve_stable_d` returns
`Err(InvalidPrice)` without entering the Newton loop and without aborting,
for every amplification. (Unit test `absurd_reserve_errors_not_panics`,
cap case.) -/
theorem solve_stable_d_rejects_reserve (env : aeneas_oracle_shim.Env) (xa xb : Std.I128)
    (amp : Std.U128)
    (h : xa.val ≤ 0 ∨ xb.val ≤ 0 ∨ 10^34 < xa.val ∨ 10^34 < xb.val) :
    oracle.lp_stable.solve_stable_d env xa xb amp = ok (.Err .InvalidPrice) := by
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  by_cases ha : xa ≤ 0#i128
  · unfold oracle.lp_stable.solve_stable_d; simp [ha]
  by_cases hb : xb ≤ 0#i128
  · unfold oracle.lp_stable.solve_stable_d; simp [ha, hb]
  by_cases hamp : amp.val < 1 ∨ 1000000 < amp.val
  · exact solve_stable_d_rejects_amp env xa xb amp hamp
  rw [IScalar.le_equiv, h0] at ha hb
  have hca := hcast_u128_val xa (by omega)
  have hcb := hcast_u128_val xb (by omega)
  unfold oracle.lp_stable.solve_stable_d
  have ha' : ¬ (xa ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have hb' : ¬ (xb ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have h3 : 1 ≤ amp.val ∧ amp.val ≤ 1000000 := by omega
  obtain ⟨M, hM, hMv⟩ := max_normalized_reserve_eq
  simp only [ha', hb', ↓reduceIte, core.ops.range.RangeInclusive.new, bind_ok, amp_contains, h3,
    lift, hM]
  by_cases h4 : IScalar.hcast .U128 xa > M
  · simp [h4]
  · have h5 : IScalar.hcast .U128 xb > M := by
      simp only [gt_iff_lt, UScalar.lt_equiv] at h4 ⊢
      omega
    simp [h4, h5]

/-- **Degenerate inputs are rejected by the stable pricer.** If either
reserve, either price or the share supply is zero or negative,
`fair_stable_lp_price_wad` returns `Err(InvalidPrice)` (does not abort), for
every amplification. Matches the Certora rule
`stable_lp_price_rejects_degenerate_inputs`. -/
theorem fair_stable_lp_price_wad_rejects_degenerate (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) (amp : Std.U128)
    (h : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨ b.price_wad.val ≤ 0 ∨
      s.total_shares.val ≤ 0) :
    oracle.lp_stable.fair_stable_lp_price_wad env a b s amp = ok (.Err .InvalidPrice) := by
  unfold oracle.lp_stable.fair_stable_lp_price_wad
  simp only [IScalar.le_equiv]
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  rw [h0]
  rcases h with h | h | h | h | h <;> simp [h]

/-- **Out-of-range amplification is rejected by the stable pricer.** For
every pair of legs, every share supply and every `amp` outside `[1, 10^6]`,
`fair_stable_lp_price_wad` returns `Err(InvalidPrice)` and does not abort.
Matches the Certora rule `stable_lp_price_rejects_out_of_range_amp` (there for
one fixed pool) and the unit test `out_of_range_amp_errors`, here for every
input. -/
theorem fair_stable_lp_price_wad_rejects_amp (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) (amp : Std.U128)
    (hamp : amp.val < 1 ∨ 1000000 < amp.val) :
    oracle.lp_stable.fair_stable_lp_price_wad env a b s amp = ok (.Err .InvalidPrice) := by
  by_cases hdeg : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨
      b.price_wad.val ≤ 0 ∨ s.total_shares.val ≤ 0
  · exact fair_stable_lp_price_wad_rejects_degenerate env a b s amp hdeg
  unfold oracle.lp_stable.fair_stable_lp_price_wad
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  simp only [g1, g2, g3, g4, g5, ↓reduceIte]
  rcases try_amount_to_wad_cases a.reserve a.decimals with ⟨xa, ha⟩ | ha <;>
  rcases try_amount_to_wad_cases b.reserve b.decimals with ⟨xb, hb⟩ | hb <;>
    simp [ha, hb, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
      core.convert.FromSame.from, solve_stable_d_rejects_amp env _ _ amp hamp]

/-- **The stable pricer is symmetric under swapping the legs.** For every
pair of legs, share supply and amplification,
`fair_stable_lp_price_wad(a, b, s, amp)` and `fair_stable_lp_price_wad(b, a,
s, amp)` are the same computation (same price, same error, same abort).
Matches the Certora rule `stable_lp_price_is_symmetric_under_leg_swap`, here
without its input bounds. -/
theorem fair_stable_lp_price_wad_symmetric (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) (amp : Std.U128) :
    oracle.lp_stable.fair_stable_lp_price_wad env a b s amp =
      oracle.lp_stable.fair_stable_lp_price_wad env b a s amp := by
  by_cases hdeg : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨
      b.price_wad.val ≤ 0 ∨ s.total_shares.val ≤ 0
  · rw [fair_stable_lp_price_wad_rejects_degenerate env a b s amp hdeg,
      fair_stable_lp_price_wad_rejects_degenerate env b a s amp (by omega)]
  unfold oracle.lp_stable.fair_stable_lp_price_wad
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  simp only [g1, g2, g3, g4, g5, ↓reduceIte]
  rcases try_amount_to_wad_cases a.reserve a.decimals with ⟨xa, ha⟩ | ha <;>
  rcases try_amount_to_wad_cases b.reserve b.decimals with ⟨xb, hb⟩ | hb <;>
    simp only [ha, hb, bind_ok, core.result.Result.Insts.CoreOpsTry.branch]
  rw [solve_stable_d_symmetric env xa xb amp, i128_min_comm]

/-- **The stable price tracks only the cheaper leg.** Two pools that differ
only in the price of leg `b`, where that price is at least leg `a`'s price in
both, give the same result (same price, same error, same abort), for every
reserve, decimal count, share supply and amplification: the price enters
only through `min(price_a, price_b)` and the positivity guard. Matches the
Certora rule `stable_lp_price_tracks_only_the_cheaper_leg`, here without its
input bounds. -/
theorem fair_stable_lp_price_wad_tracks_cheaper_leg (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (p₁ p₂ : Std.I128) (s : oracle.lp.LpSupply) (amp : Std.U128)
    (h₁ : a.price_wad.val ≤ p₁.val) (h₂ : a.price_wad.val ≤ p₂.val) :
    oracle.lp_stable.fair_stable_lp_price_wad env a { b with price_wad := p₁ } s amp =
      oracle.lp_stable.fair_stable_lp_price_wad env a { b with price_wad := p₂ } s amp := by
  have hmin₁ : core.cmp.impls.OrdI128.min a.price_wad p₁ = a.price_wad := by
    apply IScalar.eq_of_val_eq; simp only [core.cmp.impls.OrdI128.min_val]; omega
  have hmin₂ : core.cmp.impls.OrdI128.min a.price_wad p₂ = a.price_wad := by
    apply IScalar.eq_of_val_eq; simp only [core.cmp.impls.OrdI128.min_val]; omega
  unfold oracle.lp_stable.fair_stable_lp_price_wad
  simp only [hmin₁, hmin₂]
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  by_cases ha : a.price_wad ≤ 0#i128
  · simp [ha]
  · have g1 : ¬ (p₁ ≤ 0#i128) := by rw [IScalar.le_equiv, h0] at ha ⊢; omega
    have g2 : ¬ (p₂ ≤ 0#i128) := by rw [IScalar.le_equiv, h0] at ha ⊢; omega
    simp only [ha, g1, g2, ↓reduceIte]

end OracleAeneas.Proofs
