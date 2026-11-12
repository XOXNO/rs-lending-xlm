/-
Constant-product LP pricing, `common/src/oracle/lp.rs`, and the observation
helpers it uses (`common/src/oracle/observation.rs`).
-/
import OracleAeneas.Proofs.Isqrt
import OracleAeneas.Proofs.HalfUp
import OracleAeneas.Proofs.Fp

open Aeneas Aeneas.Std Result
open OracleAeneas.Spec
open oracle_aeneas

set_option maxRecDepth 20000

namespace OracleAeneas.Proofs

abbrev OErr := errors.OracleError

/-! ## `try_u256_to_i128` -/

/-- **`try_u256_to_i128` is the exact conversion.** For every `U256` value
`v`, it returns (never aborts) `Some q` with `q = v` when `v ≤ i128::MAX`
(`v < 2^127`), and `None` otherwise. -/
theorem try_u256_to_i128_spec (v : aeneas_oracle_shim.U256) :
    oracle.observation.try_u256_to_i128 v ⦃ o =>
      match o with
      | some q => q.val = v.val ∧ v.val < 2^127
      | none => 2^127 ≤ v.val ⦄ := by
  unfold oracle.observation.try_u256_to_i128
  step as ⟨o, ho⟩
  have htm := UScalar.tryMkOpt_eq .U128 v.val
  rw [← ho] at htm
  cases o with
  | none =>
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual, WP.spec_ok]
    simp only [UScalar.inBounds, UScalarTy.numBits] at htm
    omega
  | some w =>
    obtain ⟨hw, _⟩ := htm
    simp only [core.option.Option.Insts.CoreOpsTry_traitTry.branch, bind_ok, lift,
      core.bool.Bool.then_some, WP.spec_ok]
    have hmax : (IScalar.hcast .U128 core.num.I128.MAX).val = 2^127 - 1 := by
      simp [core.num.I128.MAX, IScalar.hcast_val_eq, I128.rMax]
    by_cases hle : w.val ≤ 2^127 - 1
    · have hd : w ≤ IScalar.hcast UScalarTy.U128 core.num.I128.MAX := by
        rw [UScalar.le_equiv, hmax]; exact hle
      have hc := UScalar.hcast_inBounds_spec .I128 w
        (by simp [I128.max, I128.numBits]; omega)
      simp only [lift, WP.spec_ok] at hc
      simp only [hd, decide_true, ↓reduceIte]
      refine ⟨?_, by omega⟩
      rw [hc, hw]
    · have hd : ¬ (w ≤ IScalar.hcast UScalarTy.U128 core.num.I128.MAX) := by
        rw [UScalar.le_equiv, hmax]; exact hle
      simp only [hd, decide_false, Bool.false_eq_true, ↓reduceIte]
      omega

/-! ## `try_amount_to_wad` -/

/-- **`try_amount_to_wad` scales exactly or fails closed.** For every `i128`
amount and `u32` decimal count it returns (never aborts) either
`Ok(amount * 10^(18 - decimals))`, only when `amount ≥ 0` and
`decimals ≤ 18`, or `Err(InvalidPrice)`, only when the amount is negative,
`decimals > 18`, or the scaled amount does not fit `i128`. Since #224 the
function takes no `env` and goes through `Wad::try_from_token_upscale`
(`Wad_try_from_token_upscale_spec`); the result is the same as before. -/
theorem try_amount_to_wad_spec (amount : Std.I128) (dec : Std.U32) :
    oracle.observation.try_amount_to_wad amount dec ⦃ r =>
      match r with
      | .Ok q => 0 ≤ amount.val ∧ dec.val ≤ 18 ∧ q.val = amount.val * 10^(18 - dec.val)
      | .Err e => e = .InvalidPrice ∧
          (amount.val < 0 ∨ 18 < dec.val ∨ 2^127 ≤ amount.val * 10^(18 - dec.val)) ⦄ := by
  unfold oracle.observation.try_amount_to_wad
  step with Wad_try_from_token_upscale_spec amount dec as ⟨o, ho⟩
  cases o with
  | none =>
    simp only [core.option.Option.map_none, bind_ok, core.option.Option.ok_or, WP.spec_ok]
    exact ⟨trivial, ho⟩
  | some w =>
    simp only [core.option.Option.map_some, P.Insts.CoreOpsFunctionFnOnceTupleWadI128,
      P.Insts.CoreOpsFunctionFnOnceTupleWadI128.call_once, math.fp.Wad.raw, bind_tc_ok,
      bind_ok, core.option.Option.ok_or, WP.spec_ok]
    exact ho

/-! ## `reserve_value_wad` -/

/-- **`reserve_value_wad` is the half-up leg value or fails closed.** For
every leg it returns (never aborts) either `Ok(v)` with
`v = reserve * price_wad / 10^decimals` rounded half up (stated as
`IsHalfUp`), only when the reserve and price are not negative and
`10^decimals` fits `i128`, or `Err(InvalidPrice)`, only when `10^decimals`
overflows `i128`, the reserve or the price is negative, or the value does not
fit `i128`. Since #224 it goes through `Wad::try_value_of_token`
(`Wad_try_value_of_token_spec`), which computes the same single half-up
rounding of `reserve * price_wad` by `10^decimals`: the statement is
unchanged. -/
theorem reserve_value_wad_spec (env : aeneas_oracle_shim.Env) (leg : oracle.lp.LpLeg) :
    oracle.lp.reserve_value_wad env leg ⦃ r =>
      match r with
      | .Ok v => 0 ≤ leg.reserve.val ∧ 0 ≤ leg.price_wad.val ∧ (10 : Int)^leg.decimals.val < 2^127 ∧
          IsHalfUp (leg.reserve.val * leg.price_wad.val) (10^leg.decimals.val) v.val
      | .Err e => e = .InvalidPrice ∧
          ((2 : Int)^127 ≤ 10^leg.decimals.val ∨ leg.reserve.val < 0 ∨ leg.price_wad.val < 0 ∨
            ∀ q : Int, IsHalfUp (leg.reserve.val * leg.price_wad.val) (10^leg.decimals.val) q →
              2^127 ≤ q) ⦄ := by
  unfold oracle.lp.reserve_value_wad
  simp only [math.fp.Wad.from, core.convert.IntoFrom.into, core.convert.FromSame.from,
    bind_ok]
  step with Wad_try_value_of_token_spec leg.price_wad env leg.reserve leg.decimals as ⟨o, ho⟩
  cases o with
  | none =>
    simp only [core.option.Option.map_none, bind_ok, core.option.Option.ok_or, WP.spec_ok]
    exact ⟨trivial, ho⟩
  | some w =>
    simp only [core.option.Option.map_some, P.Insts.CoreOpsFunctionFnOnceTupleWadI128,
      P.Insts.CoreOpsFunctionFnOnceTupleWadI128.call_once, math.fp.Wad.raw, bind_tc_ok,
      bind_ok, core.option.Option.ok_or, WP.spec_ok]
    exact ho

/-! ## Equational forms -/

/-- A total-correctness spec pins the computation to `ok v` for some `v`. -/
theorem spec_ok_eq {α : Type} {m : Result α} {P : α → Prop} (h : m ⦃ P ⦄) :
    ∃ v, m = ok v ∧ P v := by
  cases m with
  | ret r => exact ⟨r, rfl, (WP.spec_ok r).mp h⟩
  | vis i k => exact absurd h (by simp [WP.spec_vis])
  | div => exact absurd h (by simp [WP.spec_div])

/-- `isqrt_of_product` as an equation: it returns `Nat.sqrt (a*b)`. -/
theorem isqrt_of_product_eq (env : aeneas_oracle_shim.Env) (a b : Std.U128) :
    oracle.lp.isqrt_of_product env a b =
      ok (Nat.sqrt (a.val * b.val) : aeneas_oracle_shim.U256) := by
  obtain ⟨r, hr, h1, h2⟩ := spec_ok_eq (isqrt_of_product_spec env a b)
  rw [hr]
  have : r.val = Nat.sqrt (a.val * b.val) := Nat.eq_sqrt.mpr ⟨h1, h2⟩
  exact congrArg ok this

/-- `isqrt_of_product` does not depend on the order of its arguments. -/
theorem isqrt_of_product_comm (env : aeneas_oracle_shim.Env) (a b : Std.U128) :
    oracle.lp.isqrt_of_product env a b = oracle.lp.isqrt_of_product env b a := by
  rw [isqrt_of_product_eq, isqrt_of_product_eq, Nat.mul_comm]

/-- `reserve_value_wad` always returns `Ok` or `Err(InvalidPrice)`. -/
theorem reserve_value_wad_cases (env : aeneas_oracle_shim.Env) (leg : oracle.lp.LpLeg) :
    (∃ v, oracle.lp.reserve_value_wad env leg = ok (.Ok v)) ∨
      oracle.lp.reserve_value_wad env leg = ok (.Err .InvalidPrice) := by
  obtain ⟨r, hr, hP⟩ := spec_ok_eq (reserve_value_wad_spec env leg)
  cases r with
  | Ok v => exact Or.inl ⟨v, hr⟩
  | Err e => right; rw [hr, hP.1]

/-! ## `fair_lp_price_wad` -/

/-- **Degenerate inputs are rejected, not aborted.** If either reserve, either
price or the share supply is zero or negative, `fair_lp_price_wad` returns
`Err(InvalidPrice)` (it does not abort). Matches the Certora rule
`lp_price_rejects_degenerate_inputs`, here for every decimal count. -/
theorem fair_lp_price_wad_rejects_degenerate (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply)
    (h : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨ b.price_wad.val ≤ 0 ∨
      s.total_shares.val ≤ 0) :
    oracle.lp.fair_lp_price_wad env a b s = ok (.Err .InvalidPrice) := by
  unfold oracle.lp.fair_lp_price_wad
  simp only [IScalar.le_equiv]
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  rw [h0]
  rcases h with h | h | h | h | h <;> simp [h]

/-- **Symmetric under swapping the legs.** For every pair of legs and every
share supply, `fair_lp_price_wad(a, b, s)` and `fair_lp_price_wad(b, a, s)`
are the same computation result: the same `Ok` price, or the same error.
Matches the Certora rule `lp_price_is_symmetric_under_leg_swap`, here without
its input bounds. -/
theorem fair_lp_price_wad_symmetric (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) :
    oracle.lp.fair_lp_price_wad env a b s = oracle.lp.fair_lp_price_wad env b a s := by
  by_cases hdeg : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨
      b.price_wad.val ≤ 0 ∨ s.total_shares.val ≤ 0
  · rw [fair_lp_price_wad_rejects_degenerate env a b s hdeg,
      fair_lp_price_wad_rejects_degenerate env b a s (by omega)]
  · unfold oracle.lp.fair_lp_price_wad
    have h0 : (0#i128 : Std.I128).val = 0 := by simp
    have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
    have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
    have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
    have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
    have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
    simp only [g1, g2, g3, g4, g5, ↓reduceIte]
    rcases reserve_value_wad_cases env a with ⟨va, ha⟩ | ha <;>
    rcases reserve_value_wad_cases env b with ⟨vb, hb⟩ | hb <;>
      simp only [ha, hb, bind_ok, core.result.Result.Insts.CoreOpsTry.branch, lift]
    rw [isqrt_of_product_comm]

/-- A leg value returned as `Ok` is not negative. -/
theorem reserve_value_wad_nonneg (env : aeneas_oracle_shim.Env) (leg : oracle.lp.LpLeg)
    (v : Std.I128) (h : oracle.lp.reserve_value_wad env leg = ok (.Ok v)) : 0 ≤ v.val := by
  have hs := reserve_value_wad_spec env leg
  rw [h, WP.spec_ok] at hs
  obtain ⟨hr, hp, _, hh⟩ := hs
  have hn : 0 ≤ leg.reserve.val * leg.price_wad.val := Int.mul_nonneg hr hp
  have hd : 0 < (10 : Int) ^ leg.decimals.val := by positivity
  unfold IsHalfUp at hh
  by_contra hneg
  have : v.val * 10 ^ leg.decimals.val ≤ -(10 ^ leg.decimals.val) := by nlinarith
  nlinarith

/-- A non-negative `i128` cast to `u128` keeps its value. -/
theorem hcast_u128_val (v : Std.I128) (h : 0 ≤ v.val) :
    ((IScalar.hcast .U128 v).val : Int) = v.val := by
  have hb := v.hBounds
  have hc := IScalar.hcast_inBounds_spec .U128 v
    ⟨h, by simp [IScalarTy.numBits, U128.max, U128.numBits] at hb ⊢; omega⟩
  simp only [lift, WP.spec_ok] at hc
  exact hc

/-- **The constant-product LP price formula.** When every reserve, price and
the share supply are positive, both legs value to `Ok(va)`, `Ok(vb)` under
`reserve_value_wad`, and the share supply scales to `Ok(S)` with `S > 0`
under `try_amount_to_wad`, then `fair_lp_price_wad` returns (never aborts)
`Ok(p)` with `p = ⌊2·⌊√(va·vb)⌋·WAD / S⌋` (stated as `IsFloorPos`), or
`Err(InvalidPrice)` exactly when that quotient does not fit `i128`. This is
the formula of `docs/reference/formulas.md` ("LP share prices"). Every
intermediate `U256` step fits (`va, vb < 2^127`), so the host never traps. -/
theorem fair_lp_price_wad_value (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply)
    (hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧ 0 < b.price_wad.val ∧
      0 < s.total_shares.val)
    (va vb S : Std.I128)
    (ha : oracle.lp.reserve_value_wad env a = ok (.Ok va))
    (hb : oracle.lp.reserve_value_wad env b = ok (.Ok vb))
    (hS : oracle.observation.try_amount_to_wad s.total_shares s.decimals = ok (.Ok S))
    (hSpos : 0 < S.val) :
    oracle.lp.fair_lp_price_wad env a b s ⦃ r =>
      match r with
      | .Ok p => IsFloorPos (2 * (Nat.sqrt (va.val.toNat * vb.val.toNat) : Int) * 10^18) S.val p.val
      | .Err e => e = .InvalidPrice ∧
          ∀ q : Int, IsFloorPos (2 * (Nat.sqrt (va.val.toNat * vb.val.toNat) : Int) * 10^18)
            S.val q → 2^127 ≤ q ⦄ := by
  have hva := reserve_value_wad_nonneg env a va ha
  have hvb := reserve_value_wad_nonneg env b vb hb
  have hvab := va.hBounds
  have hvbb := vb.hBounds
  have hSb := S.hBounds
  simp only [IScalarTy.numBits] at hvab hvbb hSb
  unfold oracle.lp.fair_lp_price_wad
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g6 : ¬ (S ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  simp only [g1, g2, g3, g4, g5, ↓reduceIte, ha, hb, hS, bind_ok,
    core.result.Result.Insts.CoreOpsTry.branch, lift, isqrt_of_product_eq, g6]
  have hca := hcast_u128_val va hva
  have hcb := hcast_u128_val vb hvb
  have hcS := hcast_u128_val S (by omega)
  -- the root
  set r := Nat.sqrt ((IScalar.hcast .U128 va).val * (IScalar.hcast .U128 vb).val) with hr
  have hrv : r = Nat.sqrt (va.val.toNat * vb.val.toNat) := by
    rw [hr]; congr 2 <;> omega
  have hr1 := Nat.sqrt_le ((IScalar.hcast .U128 va).val * (IScalar.hcast .U128 vb).val)
  rw [← hr] at hr1
  have hprod : (IScalar.hcast .U128 va).val * (IScalar.hcast .U128 vb).val < 2^254 := by
    have h1 : (IScalar.hcast .U128 va).val < 2^127 := by omega
    have h2 : (IScalar.hcast .U128 vb).val < 2^127 := by omega
    calc _ < 2^127 * 2^127 := Nat.mul_lt_mul'' h1 h2
      _ = 2^254 := by norm_num
  have hrlt : r < 2^127 := by
    by_contra hc
    have : 2^127 * 2^127 ≤ r * r := Nat.mul_le_mul (by omega) (by omega)
    have h2 : (2:Nat)^127 * 2^127 = 2^254 := by norm_num
    omega
  step as ⟨two, htwo⟩
  step as ⟨tv, htv⟩
  · show r * aeneas_oracle_shim.U256.val two ≤ 2^256 - 1
    rw [htwo]; omega
  have hrr : aeneas_oracle_shim.U256.val r = r := rfl
  rw [hrr, htwo] at htv
  have hW : (IScalar.hcast .U128 constants.production.shared.WAD).val = 10^18 := by
    unfold constants.production.shared.WAD; simp [IScalar.hcast_val_eq]
  step as ⟨w, hw⟩
  rw [hW] at hw
  step as ⟨m, hm⟩
  step as ⟨sd, hsd⟩
  step as ⟨f, hf⟩
  step with try_u256_to_i128_spec f as ⟨o, ho⟩
  rw [hf, hm, htv, hw, hsd] at ho
  have hSn : ((IScalar.hcast .U128 S).val : Int) = S.val := hcS
  -- the exact floor
  have hfl := floorPos_of_ediv (2 * (r : Int) * 10^18) S.val hSpos
  have hcastq : (((r * 2 * 10^18 / (IScalar.hcast .U128 S).val : Nat)) : Int) =
      2 * (r : Int) * 10^18 / S.val := by
    rw [← hSn]; push_cast; ring_nf
  rw [← hrv]
  cases o with
  | none =>
    simp only [core.option.Option.ok_or, WP.spec_ok]
    refine ⟨trivial, fun q hq => ?_⟩
    have := IsFloorPos.unique hSpos hq hfl
    rw [this, ← hcastq]; exact_mod_cast ho
  | some q =>
    simp only [core.option.Option.ok_or, WP.spec_ok]
    obtain ⟨hq, _⟩ := ho
    rw [hq, hcastq]; exact hfl

/-- **A balanced pool prices at its reserve value.** When both legs value to
the same `Ok(v)` under `reserve_value_wad` (and the conditions of
`fair_lp_price_wad_value` hold), the LP price is `⌊2·v·WAD / S⌋`: the pool is
worth exactly the sum of its two reserve values, because `⌊√(v·v)⌋ = v`.
The unit test `balanced_pool_prices_at_reserve_value` is the instance
`v = 10^20`, `S = 10^20` (price `2·10^18`). -/
theorem fair_lp_price_wad_balanced (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply)
    (hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧ 0 < b.price_wad.val ∧
      0 < s.total_shares.val)
    (v S : Std.I128)
    (ha : oracle.lp.reserve_value_wad env a = ok (.Ok v))
    (hb : oracle.lp.reserve_value_wad env b = ok (.Ok v))
    (hS : oracle.observation.try_amount_to_wad s.total_shares s.decimals = ok (.Ok S))
    (hSpos : 0 < S.val) :
    oracle.lp.fair_lp_price_wad env a b s ⦃ r =>
      match r with
      | .Ok p => IsFloorPos (2 * v.val * 10^18) S.val p.val
      | .Err e => e = .InvalidPrice ∧
          ∀ q : Int, IsFloorPos (2 * v.val * 10^18) S.val q → 2^127 ≤ q ⦄ := by
  have hv := reserve_value_wad_nonneg env a v ha
  have hsq : ((Nat.sqrt (v.val.toNat * v.val.toNat) : Nat) : Int) = v.val := by
    rw [Nat.sqrt_eq]; omega
  apply WP.spec_mono (fair_lp_price_wad_value env a b s hpos v v S ha hb hS hSpos)
  intro r hr
  rw [hsq] at hr
  exact hr

/-- `try_amount_to_wad` always returns `Ok` or `Err(InvalidPrice)`. -/
theorem try_amount_to_wad_cases (amount : Std.I128)
    (dec : Std.U32) :
    (∃ v, oracle.observation.try_amount_to_wad amount dec = ok (.Ok v)) ∨
      oracle.observation.try_amount_to_wad amount dec = ok (.Err .InvalidPrice) := by
  obtain ⟨r, hr, hP⟩ := spec_ok_eq (try_amount_to_wad_spec amount dec)
  cases r with
  | Ok v => exact Or.inl ⟨v, hr⟩
  | Err e => right; rw [hr, hP.1]

/-- A positive amount scales to a positive WAD amount. -/
theorem try_amount_to_wad_pos (amount : Std.I128) (dec : Std.U32)
    (S : Std.I128) (hpos : 0 < amount.val)
    (h : oracle.observation.try_amount_to_wad amount dec = ok (.Ok S)) : 0 < S.val := by
  have hs := try_amount_to_wad_spec amount dec
  rw [h, WP.spec_ok] at hs
  rw [hs.2.2]; positivity

/-- When the legs value fine but the share supply does not scale
(`try_amount_to_wad` returns `Err`), `fair_lp_price_wad` returns
`Err(InvalidPrice)` without aborting: the root and `2·root` computed before
the share conversion never trap. -/
theorem fair_lp_price_wad_share_err (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply)
    (hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧ 0 < b.price_wad.val ∧
      0 < s.total_shares.val)
    (va vb : Std.I128)
    (ha : oracle.lp.reserve_value_wad env a = ok (.Ok va))
    (hb : oracle.lp.reserve_value_wad env b = ok (.Ok vb))
    (hS : oracle.observation.try_amount_to_wad s.total_shares s.decimals =
      ok (.Err .InvalidPrice)) :
    oracle.lp.fair_lp_price_wad env a b s ⦃ r => r = .Err .InvalidPrice ⦄ := by
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have hva := reserve_value_wad_nonneg env a va ha
  have hvb := reserve_value_wad_nonneg env b vb hb
  have hvab := va.hBounds
  have hvbb := vb.hBounds
  simp only [IScalarTy.numBits] at hvab hvbb
  unfold oracle.lp.fair_lp_price_wad
  simp only [g1, g2, g3, g4, g5, ↓reduceIte, ha, hb, hS, bind_ok,
    core.result.Result.Insts.CoreOpsTry.branch, lift, isqrt_of_product_eq]
  have hca := hcast_u128_val va hva
  have hcb := hcast_u128_val vb hvb
  set r := Nat.sqrt ((IScalar.hcast .U128 va).val * (IScalar.hcast .U128 vb).val) with hr
  have hr1 := Nat.sqrt_le ((IScalar.hcast .U128 va).val * (IScalar.hcast .U128 vb).val)
  rw [← hr] at hr1
  have hprod : (IScalar.hcast .U128 va).val * (IScalar.hcast .U128 vb).val < 2^254 := by
    have h1 : (IScalar.hcast .U128 va).val < 2^127 := by omega
    have h2 : (IScalar.hcast .U128 vb).val < 2^127 := by omega
    calc _ < 2^127 * 2^127 := Nat.mul_lt_mul'' h1 h2
      _ = 2^254 := by norm_num
  have hrlt : r < 2^127 := by
    by_contra hc
    have : 2^127 * 2^127 ≤ r * r := Nat.mul_le_mul (by omega) (by omega)
    have h2 : (2:Nat)^127 * 2^127 = 2^254 := by norm_num
    omega
  step as ⟨two, htwo⟩
  step as ⟨tv, htv⟩
  · show r * aeneas_oracle_shim.U256.val two ≤ 2^256 - 1
    rw [htwo]; omega
  simp [core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
    core.convert.FromSame.from]

/-- **Share decimals past WAD are rejected, not aborted.** If the share
supply declares more than 18 decimals, `fair_lp_price_wad` returns
`Err(InvalidPrice)` for every reserve, price and share amount. Matches the
Certora rule `lp_price_rejects_share_decimals_past_wad` (there for one pool
and decimals up to 40), here for every input. -/
theorem fair_lp_price_wad_rejects_share_decimals (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) (hdec : 18 < s.decimals.val) :
    oracle.lp.fair_lp_price_wad env a b s ⦃ r => r = .Err .InvalidPrice ⦄ := by
  by_cases hdeg : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨
      b.price_wad.val ≤ 0 ∨ s.total_shares.val ≤ 0
  · rw [fair_lp_price_wad_rejects_degenerate env a b s hdeg]; simp
  have hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧
      0 < b.price_wad.val ∧ 0 < s.total_shares.val := by omega
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  rcases reserve_value_wad_cases env a with ⟨va, ha⟩ | ha
  · rcases reserve_value_wad_cases env b with ⟨vb, hb⟩ | hb
    · rcases try_amount_to_wad_cases s.total_shares s.decimals with ⟨S, hS⟩ | hS
      · have hs := try_amount_to_wad_spec s.total_shares s.decimals
        rw [hS, WP.spec_ok] at hs
        omega
      · exact fair_lp_price_wad_share_err env a b s hpos va vb ha hb hS
    · unfold oracle.lp.fair_lp_price_wad
      simp [g1, g2, g3, g4, g5, ha, hb, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
        core.convert.FromSame.from]
  · unfold oracle.lp.fair_lp_price_wad
    simp [g1, g2, g3, g4, g5, ha, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
      core.convert.FromSame.from]

/-- **`fair_lp_price_wad` fails closed and never aborts.** For every input
(any reserves, prices, decimals and share supply) it returns, without a host
trap or panic, either `Ok` or `Err(InvalidPrice)`. Generalises the unit test
`absurd_reserve_errors_not_panics` (INV-ORACLE-01). -/
theorem fair_lp_price_wad_fails_closed (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) :
    oracle.lp.fair_lp_price_wad env a b s ⦃ r =>
      match r with
      | .Ok _ => True
      | .Err e => e = .InvalidPrice ⦄ := by
  by_cases hdeg : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨
      b.price_wad.val ≤ 0 ∨ s.total_shares.val ≤ 0
  · rw [fair_lp_price_wad_rejects_degenerate env a b s hdeg]; simp
  have hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧
      0 < b.price_wad.val ∧ 0 < s.total_shares.val := by omega
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  rcases reserve_value_wad_cases env a with ⟨va, ha⟩ | ha
  · rcases reserve_value_wad_cases env b with ⟨vb, hb⟩ | hb
    · rcases try_amount_to_wad_cases s.total_shares s.decimals with ⟨S, hS⟩ | hS
      · by_cases hSpos : 0 < S.val
        · apply WP.spec_mono (fair_lp_price_wad_value env a b s hpos va vb S ha hb hS hSpos)
          intro r hr
          cases r with
          | Ok _ => trivial
          | Err e => exact hr.1
        · exact absurd (try_amount_to_wad_pos s.total_shares s.decimals S hpos.2.2.2.2 hS) hSpos
      · apply WP.spec_mono (fair_lp_price_wad_share_err env a b s hpos va vb ha hb hS)
        intro r hr; rw [hr]
    · unfold oracle.lp.fair_lp_price_wad
      simp [g1, g2, g3, g4, g5, ha, hb, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
        core.convert.FromSame.from]
  · unfold oracle.lp.fair_lp_price_wad
    simp [g1, g2, g3, g4, g5, ha, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
      core.convert.FromSame.from]

