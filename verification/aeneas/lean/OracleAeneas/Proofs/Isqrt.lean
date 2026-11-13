/-
`oracle::lp::isqrt_of_product(a, b)` returns exactly `⌊√(a·b)⌋` for every pair
of `u128` inputs, and its Newton loop always terminates.

The claim is stated without a square root or a division:
`r*r ≤ a*b < (r+1)*(r+1)`.
-/
import OracleAeneas.Proofs.Host

open Aeneas Aeneas.Std Result
open oracle_aeneas

namespace OracleAeneas.Proofs

/-! ## Newton-step arithmetic on naturals

Throughout, `r` is the integer square root of `n`: `r*r ≤ n < (r+1)*(r+1)`. -/

/-- One integer Newton step never undershoots the root: for every `x > 0`,
`⌊(x + ⌊n/x⌋)/2⌋ ≥ ⌊√n⌋` (integer AM-GM). -/
theorem newton_ge_root (n r x : Nat) (hr1 : r * r ≤ n) (hx : 0 < x) :
    r ≤ (x + n / x) / 2 := by
  have hq : n < (n / x + 1) * x := by
    have := Nat.lt_div_mul_add (a := n) hx
    nlinarith
  by_contra hc
  have hlt : x + n / x + 1 ≤ 2 * r := by omega
  -- (q+1)*x ≤ (2r - x)*x ≤ r*r ≤ n, contradiction
  have : (n / x + 1) * x ≤ r * r := by
    zify at hlt ⊢
    nlinarith [sq_nonneg ((r : Int) - x)]
  omega

/-- Above the root, one Newton step strictly decreases. -/
theorem newton_lt_of_gt_root (n r x : Nat) (hr2 : n < (r + 1) * (r + 1)) (hx : r < x) :
    (x + n / x) / 2 < x := by
  have hxx : n < x * x := by nlinarith
  have hq : n / x < x := by
    rw [Nat.div_lt_iff_lt_mul (by omega)]; exact hxx
  omega

/-- The quotient in a Newton step from any `x ≥ r ≥ 1` is at most `r + 2`. -/
theorem newton_quot_le (n r x : Nat) (hr2 : n < (r + 1) * (r + 1)) (hr : 1 ≤ r)
    (hx : r ≤ x) : n / x ≤ r + 2 := by
  calc n / x ≤ n / r := Nat.div_le_div_left hx hr
    _ ≤ r + 2 := by
      apply Nat.div_le_of_le_mul
      nlinarith

/-- The root of a number below `2^256` is below `2^128`. -/
theorem root_lt (n r : Nat) (hr1 : r * r ≤ n) (hn : n < 2^256) : r < 2^128 := by
  by_contra h
  have : 2^128 * 2^128 ≤ r * r := Nat.mul_le_mul (by omega) (by omega)
  have h2 : (2:Nat)^128 * 2^128 = 2^256 := by norm_num
  omega

/-! ## The loop -/

/-- **The Newton loop of `isqrt_of_product` terminates at the root.** Started
from any `x` with `⌊√n⌋ ≤ x ≤ 2^128` and `y` the first Newton step from `x`,
the loop returns (does not abort, does not diverge) exactly `⌊√n⌋`.

Hypotheses: `n ≥ 2` (so the root is at least 1; `isqrt_of_product` returns
early for `n ≤ 1`), `n < 2^256` (the product of two `u128`), and `two = 2`.
Termination: `x` strictly decreases while the loop runs and never drops
below the root. -/
theorem isqrt_loop_spec (n two : aeneas_oracle_shim.U256) (htwo : two.val = 2)
    (r : Nat) (hr1 : r * r ≤ n.val) (hr2 : n.val < (r + 1) * (r + 1)) (hr : 1 ≤ r)
    (hn : n.val < 2^256)
    (x y : aeneas_oracle_shim.U256) (hx : r ≤ x.val) (hxb : x.val ≤ 2^128)
    (hy : y.val = (x.val + n.val / x.val) / 2) :
    oracle.lp.isqrt_of_product_loop n two x y ⦃ res => res.val = r ⦄ := by
  have hrb := root_lt n.val r hr1 hn
  unfold oracle.lp.isqrt_of_product_loop
  apply loop.spec_decr_nat (fun p : aeneas_oracle_shim.U256 × aeneas_oracle_shim.U256 => p.1.val)
    (fun p => r ≤ p.1.val ∧ p.1.val ≤ 2^128 ∧ p.2.val = (p.1.val + n.val / p.1.val) / 2)
  · rintro ⟨x, y⟩ ⟨hx, hxb, hy⟩
    simp only at hx hxb hy
    unfold oracle.lp.isqrt_of_product_loop.body
    simp only [aeneas_oracle_shim.U256.lt_eq, bind_ok]
    have hyr : r ≤ y.val := by rw [hy]; exact newton_ge_root n.val r x.val hr1 (by omega)
    by_cases hlt : y.val < x.val
    · simp only [hlt, decide_true, ↓reduceIte]
      step as ⟨x1, hx1⟩
      subst x1
      step as ⟨u, hu⟩
      step as ⟨u1, hu1⟩
      · have := newton_quot_le n.val r y.val hr2 hr hyr
        rw [hu]; omega
      step as ⟨y1, hy1⟩
      refine ⟨hyr, by omega, ?_, hlt⟩
      rw [hy1, hu1, hu, htwo]
    · simp only [hlt, decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
      by_contra hne
      exact hlt (by rw [hy]; exact newton_lt_of_gt_root n.val r x.val hr2 (by omega))
  · exact ⟨hx, hxb, hy⟩

/-! ## `isqrt_of_product` -/

/-- `Nat.sqrt` of a `u128` is below `2^64`. -/
theorem sqrt_u128_lt (a : Std.U128) : Nat.sqrt a.val < 2^64 := by
  have ha := a.hBounds
  have h := Nat.sqrt_le a.val
  by_contra hc
  have : 2^64 * 2^64 ≤ Nat.sqrt a.val * Nat.sqrt a.val := Nat.mul_le_mul (by omega) (by omega)
  have h2 : (2:Nat)^64 * 2^64 = 2^128 := by norm_num
  simp only [UScalarTy.numBits] at ha
  omega

/-- **`isqrt_of_product` is the exact integer square root.** For every pair of
`u128` values `a`, `b`, `isqrt_of_product(a, b)` returns (does not abort, and
its Newton loop terminates) the `U256` value `r` with
`r*r ≤ a*b < (r+1)*(r+1)`, that is `r = ⌊√(a·b)⌋`.

Hypotheses: none beyond the types; the domain is all of `u128 × u128`.
Why nothing traps: `a*b < 2^256` fits `U256`; the seed
`(isqrt(a)+1)*(isqrt(b)+1)` is at most `2^128` (so `saturating_add` never
saturates) and strictly above the root (since `(isqrt(x)+1)^2 > x`); every
Newton step stays between the root and the seed, so the divisor is never zero
and `x + n/x` stays below `2^130`.
This is the claim of the Certora rule `isqrt_is_the_integer_floor_of_the_root`
(`certora/common/spec/lp_math_rules.rs`), which the prover cannot check
(`certora/README.md`, "Known prover limits"); here it holds for the whole
`u128` domain, not only the rule's `u64` inputs. -/
theorem isqrt_of_product_spec (env : aeneas_oracle_shim.Env) (a b : Std.U128) :
    oracle.lp.isqrt_of_product env a b ⦃ r =>
      r.val * r.val ≤ a.val * b.val ∧ a.val * b.val < (r.val + 1) * (r.val + 1) ⦄ := by
  have ha := a.hBounds
  have hb := b.hBounds
  simp only [UScalarTy.numBits] at ha hb
  have hab : a.val * b.val ≤ (2^128 - 1) * (2^128 - 1) := Nat.mul_le_mul (by omega) (by omega)
  have habM : a.val * b.val < 2^256 := by
    have : ((2:Nat)^128 - 1) * (2^128 - 1) < 2^256 := by norm_num
    omega
  unfold oracle.lp.isqrt_of_product
  step as ⟨u, hu⟩
  step as ⟨u1, hu1⟩
  step as ⟨n, hn⟩
  rw [hu, hu1] at hn
  step as ⟨one, hone⟩
  simp only [aeneas_oracle_shim.U256.le_eq, bind_ok]
  by_cases h1 : n.val ≤ one.val
  · simp only [h1, decide_true, ↓reduceIte, WP.spec_ok]
    have : one.val = 1 := by rw [hone]
    rw [← hn]
    have hn1 : n.val = 0 ∨ n.val = 1 := by omega
    rcases hn1 with h | h <;> rw [h] <;> norm_num
  · simp only [h1, decide_false, Bool.false_eq_true, ↓reduceIte]
    have hone1 : one.val = 1 := by rw [hone]
    have hn2 : 2 ≤ n.val := by omega
    step as ⟨two, htwo⟩
    step as ⟨sa, hsa⟩
    simp only [lift, bind_ok]
    step as ⟨u2, hu2⟩
    step as ⟨sb, hsb⟩
    step as ⟨u3, hu3⟩
    have hsa_lt := sqrt_u128_lt a
    have hsb_lt := sqrt_u128_lt b
    have hmax : U128.max = 2^128 - 1 := by simp [U128.max, U128.numBits]
    rw [core.num.U128.saturating_add_val _ _ (by rw [hsa, hmax]; simp; omega), hsa] at hu2
    rw [core.num.U128.saturating_add_val _ _ (by rw [hsb, hmax]; simp; omega), hsb] at hu3
    have h1v : (1#u128 : Std.U128).val = 1 := by simp
    rw [h1v] at hu2 hu3
    -- the root and the seed
    set r := Nat.sqrt n.val with hr
    have hr1 : r * r ≤ n.val := Nat.sqrt_le n.val
    have hr2 : n.val < (r + 1) * (r + 1) := Nat.lt_succ_sqrt n.val
    have hr : 1 ≤ r := by
      by_contra h0
      have : r = 0 := by omega
      rw [this] at hr2; omega
    have hseed_le : u2.val * u3.val ≤ 2^128 := by
      rw [hu2, hu3]
      calc (Nat.sqrt a.val + 1) * (Nat.sqrt b.val + 1) ≤ 2^64 * 2^64 :=
            Nat.mul_le_mul (by omega) (by omega)
        _ = 2^128 := by norm_num
    have hseed_gt : r < u2.val * u3.val := by
      have hA := Nat.lt_succ_sqrt a.val
      have hB := Nat.lt_succ_sqrt b.val
      have hprod : a.val * b.val < (u2.val * u3.val) * (u2.val * u3.val) := by
        rw [hu2, hu3]
        calc a.val * b.val
            < ((Nat.sqrt a.val).succ * (Nat.sqrt a.val).succ) *
              ((Nat.sqrt b.val).succ * (Nat.sqrt b.val).succ) :=
              Nat.mul_lt_mul'' hA hB
          _ = _ := by simp only [Nat.succ_eq_add_one]; ring
      rw [← hn] at hprod
      exact Nat.mul_self_lt_mul_self_iff.mp (by omega)
    step as ⟨seed, hseed⟩
    rw [← hseed] at hseed_le hseed_gt
    step as ⟨q, hq⟩
    step as ⟨s, hs⟩
    · have := newton_quot_le n.val r seed.val hr2 hr (by omega)
      rw [hq]; omega
    step as ⟨y, hy⟩
    have htwo2 : two.val = 2 := htwo
    apply WP.spec_mono (isqrt_loop_spec n two htwo2 r hr1 hr2 hr (by omega) seed y
      (by omega) hseed_le (by rw [hy, hs, hq, htwo2]))
    intro res hres
    rw [hres, ← hn]
    exact ⟨hr1, hr2⟩

end OracleAeneas.Proofs
