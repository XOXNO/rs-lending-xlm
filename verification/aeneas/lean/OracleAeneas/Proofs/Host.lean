/-
`@[step]` specifications for the host `U256` and `I256` models and the Rust
core functions in `FunsExternal.lean`, so the `step` tactic can walk through
generated code that calls them. Each is a direct consequence of the definition
it describes. The `I256` part is the same as `RatesAeneas/Proofs/Host.lean`.
-/
import OracleAeneas.Funs

open Aeneas Aeneas.Std Result
open oracle_aeneas

namespace aeneas_oracle_shim.U256

/-- The natural number a host `U256` stands for. A plain (not reducible)
definition on purpose: `U256` is `Nat` underneath, but terms of type `U256`
are invisible to `omega`; `x.val` has type `Nat`. -/
def val (x : aeneas_oracle_shim.U256) : Nat := x

/-- `MAX` is `2^256 - 1`. -/
theorem MAX_eq : MAX = 2^256 - 1 := rfl

/-- `U256::from_u128` keeps the value. -/
@[step] theorem from_u128_spec (env : aeneas_oracle_shim.Env) (u : Std.U128) :
    from_u128 env u ⦃ v => v.val = u.val ⦄ := by
  simp [from_u128, val]

/-- `U256::from_u32` keeps the value. -/
@[step] theorem from_u32_spec (env : aeneas_oracle_shim.Env) (u : Std.U32) :
    from_u32 env u ⦃ v => v.val = u.val ⦄ := by
  simp [from_u32, val]

/-- `Clone` on `U256` returns the same value. -/
@[step] theorem clone_spec (a : aeneas_oracle_shim.U256) :
    Insts.CoreCloneClone.clone a ⦃ c => c = a ⦄ := by
  simp [Insts.CoreCloneClone.clone]

/-- `U256::mul` is the exact product when it fits 256 bits. -/
@[step] theorem mul_spec (a b : aeneas_oracle_shim.U256) (h : a.val * b.val ≤ 2^256 - 1) :
    mul a b ⦃ c => c.val = a.val * b.val ⦄ := by
  unfold mul checked
  split
  · simp [val]
  · rename_i hc; exact absurd (by rw [MAX_eq]; exact h) hc

/-- `U256::add` is the exact sum when it fits 256 bits. -/
@[step] theorem add_spec (a b : aeneas_oracle_shim.U256) (h : a.val + b.val ≤ 2^256 - 1) :
    add a b ⦃ c => c.val = a.val + b.val ⦄ := by
  unfold add checked
  split
  · simp [val]
  · rename_i hc; exact absurd (by rw [MAX_eq]; exact h) hc

/-- `U256::sub` is the exact difference when it does not underflow. -/
@[step] theorem sub_spec (a b : aeneas_oracle_shim.U256) (h : b.val ≤ a.val) :
    sub a b ⦃ c => c.val = a.val - b.val ⦄ := by
  unfold sub
  split
  · simp [val]
  · rename_i hc; exact absurd h hc

/-- `U256::div` is floor division by a nonzero divisor. -/
@[step] theorem div_spec (a b : aeneas_oracle_shim.U256) (h : b.val ≠ 0) :
    div a b ⦃ c => c.val = a.val / b.val ⦄ := by
  unfold div
  split
  · rename_i hc; exact absurd hc h
  · simp [val]

/-- `U256::checked_mul`: `Some` exact product when it fits 256 bits, else `None`. -/
@[step] theorem checked_mul_spec (a b : aeneas_oracle_shim.U256) :
    checked_mul a b ⦃ o => (a.val * b.val ≤ 2^256 - 1 → ∃ c, o = some c ∧ c.val = a.val * b.val) ∧
      (2^256 - 1 < a.val * b.val → o = none) ⦄ := by
  unfold checked_mul
  have hM : (MAX : Nat) = 2^256 - 1 := MAX_eq
  by_cases h : a.val * b.val ≤ 2^256 - 1
  · have h' : (a : Nat) * b ≤ MAX := by rw [hM]; exact h
    rw [ite_eq_left_of_eq_true _ _ (eq_true h')]
    simp only [WP.spec_ok]
    exact ⟨fun _ => ⟨_, rfl, rfl⟩, fun h2 => absurd h (by omega)⟩
  · have h' : ¬ ((a : Nat) * b ≤ MAX) := by rw [hM]; exact h
    rw [ite_eq_right_of_eq_false _ _ (eq_false h')]
    simp only [WP.spec_ok]
    exact ⟨fun h2 => absurd h2 h, fun _ => trivial⟩

/-- `U256::to_u128`: `Some` exactly when the value fits `u128`. -/
@[step] theorem to_u128_spec (v : aeneas_oracle_shim.U256) :
    to_u128 v ⦃ o => o = UScalar.tryMkOpt .U128 v.val ⦄ := by
  simp [to_u128, val]

/-- `<` on host `U256` values, through the generated `PartialOrd` instance. -/
@[simp] theorem lt_eq (a b : aeneas_oracle_shim.U256) :
    core.cmp.PartialOrd.lt.trait_default oracle_aeneas.aeneas_oracle_shim.U256.Insts.CoreCmpPartialOrdU256 a b
      = ok (decide (a.val < b.val)) := by
  simp [core.cmp.PartialOrd.lt.trait_default, core.cmp.PartialOrd.lt.default,
    core.cmp.PartialOrd.lt_body, val,
    Insts.CoreCmpPartialOrdU256.partial_cmp, compare_lt_iff_lt]
  rfl

/-- `<=` on host `U256` values. -/
@[simp] theorem le_eq (a b : aeneas_oracle_shim.U256) :
    core.cmp.PartialOrd.le.trait_default oracle_aeneas.aeneas_oracle_shim.U256.Insts.CoreCmpPartialOrdU256 a b
      = ok (decide (a.val ≤ b.val)) := by
  simp [core.cmp.PartialOrd.le.trait_default, core.cmp.PartialOrd.le.default,
    core.cmp.PartialOrd.le_body, val,
    Insts.CoreCmpPartialOrdU256.partial_cmp, compare_lt_iff_lt]
  simp only [Nat.le_iff_lt_or_eq (n := a), Bool.decide_or]

/-- `>=` on host `U256` values. -/
@[simp] theorem ge_eq (a b : aeneas_oracle_shim.U256) :
    core.cmp.PartialOrd.ge.trait_default oracle_aeneas.aeneas_oracle_shim.U256.Insts.CoreCmpPartialOrdU256 a b
      = ok (decide (b.val ≤ a.val)) := by
  simp [core.cmp.PartialOrd.ge.trait_default, core.cmp.PartialOrd.ge.default,
    core.cmp.PartialOrd.ge_body, val,
    Insts.CoreCmpPartialOrdU256.partial_cmp, compare_gt_iff_gt]
  simp only [Nat.le_iff_lt_or_eq (n := b), Bool.decide_or, eq_comm (a := b)]

end aeneas_oracle_shim.U256

/-- `u128::isqrt` is `Nat.sqrt`. -/
@[step] theorem core.num.U128.isqrt_spec (a : Std.U128) :
    core.num.U128.isqrt a ⦃ s => s.val = Nat.sqrt a.val ⦄ := by
  unfold core.num.U128.isqrt
  have h := UScalar.tryMkOpt_eq .U128 (Nat.sqrt a.val)
  simp only [UScalar.tryMk]
  cases ho : UScalar.tryMkOpt .U128 (Nat.sqrt a.val) with
  | none =>
    exfalso; rw [ho] at h; apply h
    have := a.hBounds
    have := Nat.sqrt_le_self a.val
    simp only [UScalar.inBounds]; omega
  | some s =>
    rw [ho] at h; simp [Result.ofOption, h.1]

/-- `saturating_add` on `u128` is the exact sum when it fits. -/
theorem core.num.U128.saturating_add_val (x y : Std.U128) (h : x.val + y.val ≤ U128.max) :
    (core.num.U128.saturating_add x y).val = x.val + y.val := by
  have hmax : U128.max = 2^128 - 1 := by simp [U128.max, U128.numBits]
  simp only [core.num.U128.saturating_add, UScalar.saturating_add]
  show (BitVec.ofNat _ (Min.min (UScalar.max UScalarTy.U128) (x.val + y.val))).toNat = _
  rw [BitVec.toNat_ofNat]
  have hm : Min.min (UScalar.max UScalarTy.U128) (x.val + y.val) = x.val + y.val := by
    have : UScalar.max UScalarTy.U128 = U128.max := by simp [U128.max, UScalarTy.numBits, U128.numBits]
    rw [this]; exact Nat.min_eq_right h
  rw [hm]
  apply Nat.mod_eq_of_lt
  simp only [UScalarTy.numBits]; omega

namespace aeneas_oracle_shim.I256

/-- In the host range. -/
abbrev InRange (v : Int) : Prop := MIN ≤ v ∧ v ≤ MAX

/-- The constant `MIN` unfolded. -/
theorem MIN_eq : MIN = -2^255 := rfl
/-- The constant `MAX` unfolded. -/
theorem MAX_eq : MAX = 2^255 - 1 := rfl

/-- `I256::from_i128` keeps the value. -/
@[step] theorem from_i128_spec (env : aeneas_oracle_shim.Env) (i : Std.I128) :
    from_i128 env i ⦃ v => (v : Int) = i.val ⦄ := by
  simp [from_i128]

/-- `I256::mul` is the exact product when it fits the signed 256-bit range. -/
@[step] theorem mul_spec (a b : aeneas_oracle_shim.I256) (h : InRange ((a : Int) * b)) :
    mul a b ⦃ c => (c : Int) = (a : Int) * b ⦄ := by
  simp [mul, checked, h.1, h.2]

/-- `I256::add` is the exact sum when it fits the signed 256-bit range. -/
@[step] theorem add_spec (a b : aeneas_oracle_shim.I256) (h : InRange ((a : Int) + b)) :
    add a b ⦃ c => (c : Int) = (a : Int) + b ⦄ := by
  simp [add, checked, h.1, h.2]

/-- `I256::div` truncates, for a nonzero divisor and an in-range quotient. -/
@[step] theorem div_spec (a b : aeneas_oracle_shim.I256) (hb : (b : Int) ≠ 0)
    (h : InRange (Int.tdiv a b)) :
    div a b ⦃ q => (q : Int) = Int.tdiv a b ⦄ := by
  simp [div, checked, hb, h.1, h.2]

/-- `to_i128`: `Some` exactly when the value fits `i128`. -/
@[step] theorem to_i128_spec (v : aeneas_oracle_shim.I256) :
    to_i128 v ⦃ o => o = IScalar.tryMkOpt .I128 v ⦄ := by
  simp [to_i128]

end aeneas_oracle_shim.I256
