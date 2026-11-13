/-
StableSwap invariant solver, `common/src/oracle/lp_stable.rs`: the Newton
loop of `solve_stable_d` never traps on the inputs it admits, and every `D` it
returns is within `[D* - 1, D* + 5]` of the exact invariant root `D*`; the
stable LP pricer `fair_stable_lp_price_wad` therefore never traps and returns
`⌊D · min_price / S⌋`.

Notation (all naturals, as in the code): `x ≤ y` the sorted WAD reserves,
`S = x + y`, `a = ann = 2·amp`, `K = 4xy`. The two-coin invariant the
iteration solves, multiplied through by `4xy`, is

  `G(D) = D³ + K·(a - 1)·D - K·a·S = 0`

(equivalently `a·S + D = a·D + D³/(4xy)`, Curve's form with `Ann = amp·n`).
`G` is strictly increasing, `G(0) < 0 ≤ G(S)`, so it has one real root
`D* ∈ (0, S]`. One iteration from `d` computes
`p = ⌊⌊d²/(2x)⌋·d/(2y)⌋` (≈ `d³/K`) and
`d' = ⌊(a·S + 2p)·d / ((a - 1)·d + 3p)⌋`, Newton's step for `G`.
-/
import OracleAeneas.Proofs.LpStable

open Aeneas Aeneas.Std Result
open OracleAeneas.Spec
open oracle_aeneas

set_option maxRecDepth 20000

namespace OracleAeneas.Proofs

/-! ## The invariant and the Newton step -/

/-- The two-coin StableSwap invariant scaled by `K = 4xy`:
`G(D) = D³ + K(a-1)D - K·a·S`. Its unique real root is the exact `D*`. -/
def stableG (K a S D : Int) : Int := D ^ 3 + K * (a - 1) * D - K * a * S

/-- One Newton iteration of `solve_stable_d`, in the code's operation order:
`d_p = ⌊⌊d·d/(x·2)⌋·d/(y·2)⌋`, then
`⌊(ann·sum + 2·d_p)·d / ((ann-1)·d + 3·d_p)⌋`. -/
def stableNewton (x y a d : Nat) : Nat :=
  let p := d * d / (x * 2) * d / (y * 2)
  (a * (x + y) + 2 * p) * d / ((a - 1) * d + (2 + 1) * p)

/-- `G` is monotone (strictly increasing in fact) in `D`. -/
theorem stableG_mono {K a S u v : Int} (hK : 0 ≤ K) (ha : 1 ≤ a) (huv : u ≤ v) :
    stableG K a S u ≤ stableG K a S v := by
  unfold stableG
  have h1 : 0 ≤ v ^ 2 + u * v + u ^ 2 + K * (a - 1) := by
    nlinarith [sq_nonneg (u + v), sq_nonneg u, sq_nonneg v, mul_nonneg hK (by linarith : (0:Int) ≤ a - 1)]
  have h2 := mul_nonneg (sub_nonneg.2 huv) h1
  nlinarith

/-- `G` is negative at every `D ≤ 0`. -/
theorem stableG_neg {K a S u : Int} (hK : 0 < K) (ha : 1 ≤ a) (hS : 1 ≤ S) (hu : u ≤ 0) :
    stableG K a S u < 0 := by
  unfold stableG
  have h1 : u ^ 3 ≤ 0 := by nlinarith [sq_nonneg u]
  have h2 : K * (a - 1) * u ≤ 0 :=
    mul_nonpos_of_nonneg_of_nonpos (mul_nonneg hK.le (by linarith)) hu
  have h3 : 0 < K * a * S := by positivity
  linarith

/-- **A rounded-down Newton step never lands below the root's floor.** If `d > 0`
and the integer `p` does not exceed `d³/K` (`K·p ≤ d³`), every `s` above the
real quotient `(a·S + 2p)·d / ((a-1)·d + 3p)` has `G(s) > 0`. Applied with
`s = d' + 1` this says `d' + 1 > D*`, i.e. `d' ≥ ⌊D*⌋`. The proof is the
tangent-line (convexity) argument in polynomial form:
`d·G(s) > d·s³ - K·p·(3s - 2d) ≥ d·(s - d)²·(s + 2d) ≥ 0`. -/
theorem newton_lower (K a S d p s : Int) (hd : 0 < d) (hK : 0 < K)
    (hKp : K * p ≤ d ^ 3) (hp : 0 ≤ p) (hs : 0 ≤ s)
    (hlt : (a * S + 2 * p) * d < s * ((a - 1) * d + 3 * p)) : 0 < stableG K a S s := by
  have key : d * stableG K a S s = d * s ^ 3 - K * p * (3 * s - 2 * d) +
      K * (s * ((a - 1) * d + 3 * p) - (a * S + 2 * p) * d) := by
    unfold stableG; ring
  have hpos : 0 < K * (s * ((a - 1) * d + 3 * p) - (a * S + 2 * p) * d) :=
    mul_pos hK (by linarith)
  have hmain : 0 ≤ d * s ^ 3 - K * p * (3 * s - 2 * d) := by
    by_cases h3 : 2 * d ≤ 3 * s
    · have h4 : K * p * (3 * s - 2 * d) ≤ d ^ 3 * (3 * s - 2 * d) :=
        mul_le_mul_of_nonneg_right hKp (by linarith)
      have h5 : 0 ≤ d * ((s - d) ^ 2 * (s + 2 * d)) :=
        mul_nonneg hd.le (mul_nonneg (sq_nonneg _) (by linarith))
      nlinarith
    · have h4 : K * p * (3 * s - 2 * d) ≤ 0 :=
        mul_nonpos_of_nonneg_of_nonpos (mul_nonneg hK.le hp) (by linarith)
      have h5 : 0 ≤ d * s ^ 3 := mul_nonneg hd.le (pow_nonneg hs 3)
      linarith
  have : 0 < d * stableG K a S s := by rw [key]; linarith
  exact pos_of_mul_pos_right this hd.le

/-- **The Newton quotient stays below `2S`.** For `d ≤ 2S`, `a ≥ 2` and `p ≥ 0`,
`(a·S + 2p)·d ≤ 2S·((a-1)·d + 3p)`: the quotient is a weighted mediant of
`a·S/(a-1) ≤ 2S` and `2d/3 ≤ 2S`. -/
theorem newton_upper (a S d p : Int) (ha : 2 ≤ a) (hS : 0 ≤ S) (hd : 0 ≤ d) (hd2 : d ≤ 2 * S)
    (hp : 0 ≤ p) : (a * S + 2 * p) * d ≤ 2 * S * ((a - 1) * d + 3 * p) := by
  nlinarith [mul_nonneg (mul_nonneg hS hd) (by linarith : (0:Int) ≤ a - 2),
    mul_nonneg hp (by linarith : (0:Int) ≤ 6 * S - 2 * d)]

/-- **The stopping rule pins `d` within 4 of the root.** If the step from `d`
moved down by at most one (`d' ≥ d - 1`, so the real quotient is at least
`d - 1`), `p` is the code's rounded `d³/K` (`K·(p+1) + 2xd > d³`), and
`d ≤ 2S`, `1 ≤ x ≤ y`, `a ≥ 2`, then `G(d - 4) ≤ 0`, i.e. `d - 4 ≤ D*`.
Proof: the stopping inequality gives `G(d) ≤ 3d² + K(a + 2)`, while
`G(d) - G(d-4) = 12d² - 48d + 64 + 4K(a-1)`, so
`G(d-4) ≤ -(3d - 8)² - 3K(a - 2) ≤ 0`. -/
theorem newton_stop_accuracy (x y a d p : Int) (hx : 1 ≤ x) (hxy : x ≤ y) (ha : 2 ≤ a)
    (hd2 : d ≤ 2 * (x + y))
    (hp : d ^ 3 < 4 * x * y * (p + 1) + 2 * x * d)
    (hstop : (d - 1) * ((a - 1) * d + 3 * p) ≤ (a * (x + y) + 2 * p) * d) :
    stableG (4 * x * y) a (x + y) (d - 4) ≤ 0 := by
  have hK : 0 < 4 * x * y := by nlinarith
  by_cases h3 : d ≤ 3
  · exact (stableG_neg hK (by linarith) (by linarith) (by linarith)).le
  have h3 : 4 ≤ d := by omega
  set K := 4 * x * y with hKdef
  set S := x + y with hSdef
  have h1 : K * (a - 1) * d ^ 2 - K * (a - 1) * d + K * p * (d - 3) ≤ K * a * S * d := by
    have := mul_le_mul_of_nonneg_left hstop hK.le
    nlinarith
  have h2 : (d ^ 3 - 2 * x * d - K) * (d - 3) ≤ K * p * (d - 3) :=
    mul_le_mul_of_nonneg_right (by nlinarith) (by linarith)
  have h4 : 2 * x * d ≤ 2 * K := by nlinarith
  have hG : d * stableG K a S d ≤ d * (3 * d ^ 2 + K * (a + 2)) := by
    unfold stableG
    nlinarith [mul_nonneg (by linarith : (0:Int) ≤ 2 * K - 2 * x * d) (by linarith : (0:Int) ≤ d),
      mul_nonneg (mul_nonneg (by linarith : (0:Int) ≤ x) (by linarith : (0:Int) ≤ d)) (by linarith : (0:Int) ≤ 6),
      hK]
  have hG' : stableG K a S d ≤ 3 * d ^ 2 + K * (a + 2) :=
    le_of_mul_le_mul_left hG (by linarith)
  have h5 : stableG K a S d - stableG K a S (d - 4) = 12 * d ^ 2 - 48 * d + 64 + 4 * K * (a - 1) := by
    unfold stableG; ring
  nlinarith [sq_nonneg (3 * d - 8), mul_nonneg hK.le (by linarith : (0:Int) ≤ a - 2)]

/-! ## Facts about one iteration, on naturals -/

/-- The code's rounded `d³/K`: `p = ⌊⌊d²/(2x)⌋·d/(2y)⌋` satisfies
`K·p ≤ d³ < K·(p+1) + 2xd` and, for `d ≤ 2S` with `x ≤ y`, `p ≤ 2d²`. -/
theorem stable_p_bounds (x y d : Nat) (hx : 1 ≤ x) (hxy : x ≤ y) (hd2 : d ≤ 2 * (x + y)) :
    4 * x * y * (d * d / (x * 2) * d / (y * 2)) ≤ d ^ 3 ∧
    d ^ 3 < 4 * x * y * (d * d / (x * 2) * d / (y * 2) + 1) + 2 * x * d ∧
    d * d / (x * 2) * d / (y * 2) ≤ 2 * (d * d) := by
  set q := d * d / (x * 2) with hq
  set p := q * d / (y * 2) with hp
  have hx2 : 0 < x * 2 := by omega
  have hy2 : 0 < y * 2 := by omega
  have hq1 : q * (x * 2) ≤ d * d := Nat.div_mul_le_self _ _
  have hq2 : d * d < (q + 1) * (x * 2) := by
    have := Nat.div_add_mod (d * d) (x * 2)
    have := Nat.mod_lt (d * d) hx2
    rw [← hq] at *; nlinarith
  have hp1 : p * (y * 2) ≤ q * d := Nat.div_mul_le_self _ _
  have hp2 : q * d < (p + 1) * (y * 2) := by
    have := Nat.div_add_mod (q * d) (y * 2)
    have := Nat.mod_lt (q * d) hy2
    rw [← hp] at *; nlinarith
  have hqd : q ≤ d * d := by nlinarith
  refine ⟨?_, ?_, ?_⟩
  · have : 4 * x * y * p = (p * (y * 2)) * (x * 2) := by ring
    rw [this]
    calc p * (y * 2) * (x * 2) ≤ q * d * (x * 2) := Nat.mul_le_mul_right _ hp1
      _ = (q * (x * 2)) * d := by ring
      _ ≤ d * d * d := Nat.mul_le_mul_right _ hq1
      _ = d ^ 3 := by ring
  · have h1 : q * d * (x * 2) < (p + 1) * (y * 2) * (x * 2) := Nat.mul_lt_mul_of_pos_right hp2 hx2
    have h2 : d * d * d ≤ (q * (x * 2)) * d + x * 2 * d := by
      have : d * d ≤ q * (x * 2) + x * 2 := by nlinarith
      nlinarith
    have : d ^ 3 = d * d * d := by ring
    rw [this]; nlinarith
  · have h1 : p * (y * 2) ≤ d * d * (y * 2) * 2 := by
      calc p * (y * 2) ≤ q * d := hp1
        _ ≤ d * d * d := Nat.mul_le_mul_right _ hqd
        _ ≤ d * d * (2 * (x + y)) := Nat.mul_le_mul_left _ hd2
        _ ≤ d * d * (y * 2) * 2 := by nlinarith
    have : p * (y * 2) ≤ (2 * (d * d)) * (y * 2) := by nlinarith
    exact Nat.le_of_mul_le_mul_right this hy2

/-- **One iteration of the `solve_stable_d` loop, on naturals.** For
`1 ≤ x ≤ y`, `a ≥ 2` and an iterate `1 ≤ d ≤ 2S`, the next iterate
`D = stableNewton x y a d` satisfies `1 ≤ D ≤ 2S` and `G(D + 1) > 0`
(`D ≥ ⌊D*⌋`); and if the stopping test `D ≥ d - 1` holds then `G(d - 4) ≤ 0`
(`d ≤ D* + 4`). -/
theorem stableNewton_facts (x y a d : Nat) (hx : 1 ≤ x) (hxy : x ≤ y) (ha : 2 ≤ a)
    (hd1 : 1 ≤ d) (hd2 : d ≤ 2 * (x + y)) :
    1 ≤ stableNewton x y a d ∧ stableNewton x y a d ≤ 2 * (x + y) ∧
    0 < stableG (4 * x * y) a (x + y) ((stableNewton x y a d : Int) + 1) ∧
    (d ≤ stableNewton x y a d + 1 → stableG (4 * x * y) a (x + y) ((d : Int) - 4) ≤ 0) := by
  obtain ⟨hp1, hp2, -⟩ := stable_p_bounds x y d hx hxy hd2
  unfold stableNewton
  set p := d * d / (x * 2) * d / (y * 2) with hp
  set num := (a * (x + y) + 2 * p) * d with hnum
  set den := (a - 1) * d + (2 + 1) * p with hden
  have hden0 : 0 < den := by
    have : 1 ≤ (a - 1) * d := Nat.one_le_iff_ne_zero.mpr (Nat.mul_ne_zero (by omega) (by omega))
    omega
  have hq1 : num / den * den ≤ num := Nat.div_mul_le_self _ _
  have hq2 : num < (num / den + 1) * den := by
    have := Nat.div_add_mod num den
    have := Nat.mod_lt num hden0
    nlinarith
  set D := num / den with hD
  have hdenI : (den : Int) = ((a : Int) - 1) * d + 3 * p := by
    rw [hden]; push_cast [Nat.cast_sub (by omega : 1 ≤ a)]; ring
  have hnumI : (num : Int) = ((a : Int) * (x + y) + 2 * p) * d := by
    rw [hnum]; push_cast; ring
  have hlow : 0 < stableG (4 * x * y) a (x + y) ((D : Int) + 1) := by
    apply newton_lower (4 * x * y) a (x + y) d p ((D : Int) + 1) (by exact_mod_cast hd1)
      (by have : (1:Int) ≤ x := by exact_mod_cast hx
          have : (1:Int) ≤ y := by exact_mod_cast (le_trans hx hxy)
          nlinarith)
      (by exact_mod_cast hp1) (by positivity) (by positivity)
    rw [← hnumI, ← hdenI]; exact_mod_cast hq2
  refine ⟨?_, ?_, hlow, ?_⟩
  · -- `D ≥ 1`, since `G(1) < 0 < G(D + 1)`
    by_contra h0
    have hD0 : D = 0 := by simp only [not_le, Nat.lt_one_iff] at h0; exact h0
    rw [hD0] at hlow
    have hx' : (1:Int) ≤ x := by exact_mod_cast hx
    have hy' : (1:Int) ≤ y := by exact_mod_cast (le_trans hx hxy)
    have ha' : (2:Int) ≤ a := by exact_mod_cast ha
    unfold stableG at hlow
    push_cast at hlow
    have hK : (4:Int) ≤ 4 * x * y := by nlinarith
    have hKa : 0 ≤ (4 * x * y : Int) * a := by positivity
    have h1 := mul_le_mul_of_nonneg_left (by linarith : (2:Int) ≤ x + y) hKa
    nlinarith
  · apply Nat.div_le_of_le_mul
    have h := newton_upper a (x + y) d p (by exact_mod_cast ha) (by positivity) (by positivity)
      (by exact_mod_cast hd2) (by positivity)
    rw [← hnumI, ← hdenI] at h
    have : (num : Int) ≤ ((den * (2 * (x + y)) : Nat) : Int) := by push_cast; linarith
    exact_mod_cast this
  · intro hstop
    have hst : ((d : Int) - 1) * den ≤ num := by
      have h1 : ((d : Int) - 1) ≤ D := by omega
      have h2 : ((d : Int) - 1) * den ≤ (D : Int) * den :=
        mul_le_mul_of_nonneg_right h1 (by positivity)
      have h3 : ((D * den : Nat) : Int) ≤ num := by exact_mod_cast hq1
      push_cast at h3; linarith
    rw [hnumI, hdenI] at hst
    exact newton_stop_accuracy x y a d p (by exact_mod_cast hx) (by exact_mod_cast hxy)
      (by exact_mod_cast ha) (by exact_mod_cast hd2) (by exact_mod_cast hp2) hst

/-! ## The loop -/

/-- What `solve_stable_d`'s loop guarantees about its result, for sorted
reserves `x ≤ y` and `a = ann`: either `Err(InvalidPrice)`, or `Ok(D)` with
`1 ≤ D ≤ 2S`, `G(D + 1) > 0` and `G(D - 5) ≤ 0` (so `D* - 1 < D ≤ D* + 5`),
and `D` is one Newton step from an iterate `d` with `|D - d| ≤ 1` (the code's
stopping test). -/
def StableDPost (x y a : Nat) (r : core.result.Result aeneas_oracle_shim.U256 errors.OracleError) :
    Prop :=
  match r with
  | .Ok D => 1 ≤ D.val ∧ D.val ≤ 2 * (x + y) ∧
      0 < stableG (4 * x * y) a (x + y) ((D.val : Int) + 1) ∧
      stableG (4 * x * y) a (x + y) ((D.val : Int) - 5) ≤ 0 ∧
      ∃ d : Nat, D.val = stableNewton x y a d ∧ d ≤ D.val + 1 ∧ D.val ≤ d + 1
  | .Err e => e = .InvalidPrice

/-- `Clone` on `u32` is the identity (for the range iterator spec). -/
theorem clone_u32 (v : Std.U32) : core.clone.CloneU32.clone v = ok v := rfl

/-- `<` on `u32` (for the range iterator spec). -/
theorem lt_u32 (u v : Std.U32) : core.cmp.PartialOrdU32.lt u v = ok (decide (u.val < v.val)) := by
  simp [core.cmp.impls.PartialOrdU32.lt]

/-- **The Newton loop of `solve_stable_d` never traps and returns a near-root.**
With `n = 2`, `one = 1`, sorted reserves `1 ≤ x ≤ y ≤ 10^34`, `sum = x + y`,
`2 ≤ ann ≤ 2·10^6`, and a start iterate `1 ≤ d ≤ 2·sum`, the loop returns
(no host trap, whatever the iteration counter) either `Err(InvalidPrice)` (a
`checked_mul` overflowed, or the counter ran out) or `Ok(D)` satisfying
`StableDPost`.

Loop invariant: every iterate satisfies `1 ≤ d ≤ 2S`. It is preserved because
the Newton quotient is a mediant of `a·S/(a-1) ≤ 2S` and `2d/3`
(`newton_upper`) and never drops below `⌊D*⌋ ≥ 1` (`newton_lower`). From it:
`d·d ≤ 16·10^68`, `d_p ≤ 2d² ≤ 32·10^68`, `ann·sum ≤ 4·10^40`,
`(ann-1)·d ≤ 8·10^40`, so every plain `mul`/`add` stays far below `2^256`,
`sub` never underflows (`ann ≥ 2`, and the convergence test subtracts the
smaller value), and every divisor is positive (`x·2`, `y·2`, and the
denominator `≥ (ann-1)·d ≥ 1`). Termination: the `0..255` range counter. -/
theorem solve_stable_d_loop_spec (n one xs xl sum ann : aeneas_oracle_shim.U256)
    (hn : n.val = 2) (hone : one.val = 1) (hx : 1 ≤ xs.val) (hxy : xs.val ≤ xl.val)
    (hxl : xl.val ≤ 10 ^ 34) (hsum : sum.val = xs.val + xl.val)
    (ha : 2 ≤ ann.val) (ha' : ann.val ≤ 2 * 10 ^ 6)
    (iter : core.ops.range.Range Std.U32) (d : aeneas_oracle_shim.U256)
    (hd1 : 1 ≤ d.val) (hd2 : d.val ≤ 2 * sum.val) :
    oracle.lp_stable.solve_stable_d_loop iter n one xs xl sum ann d ⦃ r =>
      StableDPost xs.val xl.val ann.val r ⦄ := by
  unfold oracle.lp_stable.solve_stable_d_loop
  apply loop.spec_decr_nat
    (fun st : core.ops.range.Range Std.U32 × aeneas_oracle_shim.U256 =>
      st.1.end.val - st.1.start.val)
    (fun st => 1 ≤ st.2.val ∧ st.2.val ≤ 2 * sum.val)
  · rintro ⟨it, d⟩ ⟨hd1, hd2⟩
    simp only at hd1 hd2
    unfold oracle.lp_stable.solve_stable_d_loop.body
    by_cases hlt : it.start.val < it.end.val
    · step with core.iter.range.IteratorRange.next_UScalar_some_spec clone_u32 lt_u32 it hlt
        as ⟨o, it1, ho, hit1, hend⟩
      subst ho
      simp only
      have hdB : d.val ≤ 4 * 10 ^ 34 := by omega
      have hdd : d.val * d.val ≤ 2 ^ 256 - 1 := by
        have := Nat.mul_le_mul hdB hdB
        have h2 : 4 * 10 ^ 34 * (4 * 10 ^ 34) ≤ 2 ^ 256 - 1 := by norm_num
        omega
      step as ⟨u, hu⟩
      step as ⟨u1, hu1⟩
      step as ⟨dp, hdp⟩
      step as ⟨o1, ho1, ho1'⟩
      by_cases hov : dp.val * d.val ≤ 2 ^ 256 - 1
      · obtain ⟨c, rfl, hc⟩ := ho1 hov
        simp only [core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch, bind_ok]
        have hdpv : dp.val = d.val * d.val / (xs.val * 2) := by rw [hdp, hu, hu1, hn]
        obtain ⟨hp1, hp2, hp3⟩ := stable_p_bounds xs.val xl.val d.val hx hxy (by omega)
        have hpB : d.val * d.val / (xs.val * 2) * d.val / (xl.val * 2) ≤ 32 * 10 ^ 68 := by
          have := Nat.mul_le_mul hdB hdB; omega
        have hAS : ann.val * sum.val ≤ 4 * 10 ^ 40 := by
          have := Nat.mul_le_mul ha' (show sum.val ≤ 2 * 10 ^ 34 by omega); omega
        have hAD : (ann.val - 1) * d.val ≤ 8 * 10 ^ 40 := by
          have := Nat.mul_le_mul (show ann.val - 1 ≤ 2 * 10 ^ 6 by omega) hdB; omega
        have hAD1 : 1 ≤ (ann.val - 1) * d.val :=
          Nat.one_le_iff_ne_zero.mpr (Nat.mul_ne_zero (by omega) (by omega))
        step as ⟨u2, hu2⟩
        step as ⟨p, hp⟩
        have hpv : p.val = d.val * d.val / (xs.val * 2) * d.val / (xl.val * 2) := by
          rw [hp, hc, hdpv, hu2, hn]
        step as ⟨dprev, hdprev⟩
        step as ⟨u3, hu3⟩
        step as ⟨u4, hu4⟩
        step as ⟨u5, hu5⟩
        step as ⟨o2, ho2, ho2'⟩
        by_cases hov2 : u5.val * d.val ≤ 2 ^ 256 - 1
        · obtain ⟨c2, rfl, hc2⟩ := ho2 hov2
          simp only [bind_ok]
          step as ⟨u6, hu6⟩
          step as ⟨u7, hu7⟩
          step as ⟨u8, hu8⟩
          step as ⟨u9, hu9⟩
          step as ⟨den, hden⟩
          step as ⟨d1, hd1v⟩
          have hnewt : d1.val = stableNewton xs.val xl.val ann.val d.val := by
            rw [hd1v, hc2, hden, hu5, hu3, hu4, hu7, hu9, hu6, hu8, hn, hone, hpv, hsum]
            rfl
          obtain ⟨hD1, hD2, hDlow, hDacc⟩ :=
            stableNewton_facts xs.val xl.val ann.val d.val hx hxy ha hd1 (by omega)
          rw [← hnewt] at hD1 hD2 hDlow hDacc
          subst dprev
          simp only [aeneas_oracle_shim.U256.ge_eq, bind_ok]
          have hpost : d.val ≤ d1.val + 1 → d1.val ≤ d.val + 1 →
              StableDPost xs.val xl.val ann.val (.Ok d1) := by
            intro h1 h2
            refine ⟨hD1, hD2, hDlow, ?_, d.val, hnewt, h1, h2⟩
            have hK : (0:Int) ≤ 4 * xs.val * xl.val := by positivity
            exact le_trans (stableG_mono hK (by omega) (by omega)) (hDacc h1)
          by_cases hb : d.val ≤ d1.val
          · simp only [hb, decide_true, ↓reduceIte]
            step as ⟨u10, hu10⟩
            simp only [aeneas_oracle_shim.U256.le_eq, bind_ok]
            by_cases hcv : u10.val ≤ one.val
            · simp only [hcv, decide_true, ↓reduceIte, WP.spec_ok]
              exact hpost (by omega) (by rw [hu10, hone] at hcv; omega)
            · simp only [hcv, decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
              refine ⟨hD1, by omega, ?_⟩
              rw [hend]; omega
          · simp only [hb, decide_false, Bool.false_eq_true, ↓reduceIte]
            step as ⟨u10, hu10⟩
            simp only [aeneas_oracle_shim.U256.le_eq, bind_ok]
            by_cases hcv : u10.val ≤ one.val
            · simp only [hcv, decide_true, ↓reduceIte, WP.spec_ok]
              exact hpost (by rw [hu10, hone] at hcv; omega) (by omega)
            · simp only [hcv, decide_false, Bool.false_eq_true, ↓reduceIte, WP.spec_ok]
              refine ⟨hD1, by omega, ?_⟩
              rw [hend]; omega
        · rw [ho2' (by omega)]
          simp [core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
            core.convert.FromSame.from, StableDPost]
      · rw [ho1' (by omega)]
        simp [core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
          core.convert.FromSame.from, StableDPost]
    · step with core.iter.range.IteratorRange.next_UScalar_none_spec lt_u32 it (by omega)
        as ⟨o, it1, ho, hit1⟩
      subst ho
      simp [StableDPost]
  · exact ⟨hd1, hd2⟩

/-! ## `solve_stable_d` -/

/-- **`solve_stable_d` never traps on the domain it admits, and every `D` it
returns is within `[D* - 1, D* + 5]` of the exact invariant.** For
`amp ∈ [1, 10^6]` and WAD reserves `xa, xb ∈ (0, 10^34]` (exactly the inputs
its guards let into the loop), `solve_stable_d(xa, xb, amp)` returns (never
aborts) `Err(InvalidPrice)` or `Ok(D)` with `StableDPost x y (2·amp)`, where
`x = min(xa, xb)`, `y = max(xa, xb)`: `1 ≤ D ≤ 2(x + y)`, `G(D + 1) > 0`,
`G(D - 5) ≤ 0` for `G(D) = D³ + 4xy(2amp - 1)D - 4xy·2amp·(x + y)`, and `D`
is a Newton step from an iterate within 1 of it (the code's stopping test
`|D_{k+1} - D_k| ≤ 1`). `Err` is returned when a `checked_mul` overflows
(extreme imbalance) or 255 rounds pass without the stopping test holding;
this theorem does not exclude the second case (see `docs/oracle.md`). -/
theorem solve_stable_d_spec (env : aeneas_oracle_shim.Env) (xa xb : Std.I128) (amp : Std.U128)
    (ha0 : 0 < xa.val) (hb0 : 0 < xb.val) (ha1 : xa.val ≤ 10 ^ 34) (hb1 : xb.val ≤ 10 ^ 34)
    (hamp : 1 ≤ amp.val ∧ amp.val ≤ 1000000) :
    oracle.lp_stable.solve_stable_d env xa xb amp ⦃ r =>
      StableDPost (min xa.val xb.val).toNat (max xa.val xb.val).toNat (2 * amp.val) r ⦄ := by
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have ha' : ¬ (xa ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have hb' : ¬ (xb ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have hca := hcast_u128_val xa (by omega)
  have hcb := hcast_u128_val xb (by omega)
  obtain ⟨M, hM, hMv⟩ := max_normalized_reserve_eq
  have h4 : ¬ (IScalar.hcast .U128 xa > M) := by
    simp only [gt_iff_lt, UScalar.lt_equiv]; omega
  have h5 : ¬ (IScalar.hcast .U128 xb > M) := by
    simp only [gt_iff_lt, UScalar.lt_equiv]; omega
  unfold oracle.lp_stable.solve_stable_d
  simp only [ha', hb', ↓reduceIte, core.ops.range.RangeInclusive.new, bind_ok, amp_contains, hamp,
    lift, hM, h4, h5]
  have hN : (oracle.lp_stable.N_COINS).val = 2 := by unfold oracle.lp_stable.N_COINS; rfl
  simp only [and_self, decide_true, ↓reduceIte]
  step as ⟨n, hn⟩
  step as ⟨one, hone⟩
  have hmin := core.cmp.impls.OrdI128.min_val xa xb
  have hmax := core.cmp.impls.OrdI128.max_val xa xb
  have hcmin := hcast_u128_val (core.cmp.impls.OrdI128.min xa xb) (by rw [hmin]; omega)
  have hcmax := hcast_u128_val (core.cmp.impls.OrdI128.max xa xb) (by rw [hmax]; omega)
  step as ⟨xs, hxs⟩
  step as ⟨xl, hxl⟩
  have hxsv : xs.val = (min xa.val xb.val).toNat := by omega
  have hxlv : xl.val = (max xa.val xb.val).toNat := by omega
  step with aeneas_oracle_shim.U256.add_spec xs xl (by omega) as ⟨sum, hsum⟩
  step as ⟨u, hu⟩
  step with aeneas_oracle_shim.U256.mul_spec u n (by rw [hu, hn, hN]; omega) as ⟨ann, hann⟩
  step as ⟨d, hd⟩
  subst d
  have hn2 : n.val = 2 := by rw [hn]; exact hN
  have hone1 : one.val = 1 := by rw [hone]
  have hannv : ann.val = 2 * amp.val := by rw [hann, hu, hn2]; ring
  apply WP.spec_mono (solve_stable_d_loop_spec n one xs xl sum ann hn2 hone1 (by omega) (by omega)
    (by omega) hsum (by omega) (by omega) _ sum (by omega) (by omega))
  intro r hr
  rw [hxsv, hxlv, hannv] at hr
  exact hr

/-- **An `Ok(D)` from `solve_stable_d` comes from the admitted domain and
satisfies the near-root bounds.** If `solve_stable_d(xa, xb, amp)` returns
`Ok(D)`, then `xa, xb ∈ (0, 10^34]`, `amp ∈ [1, 10^6]` and `D` satisfies
`StableDPost` (in particular `1 ≤ D ≤ 2(xa + xb) ≤ 4·10^34`). -/
theorem solve_stable_d_ok_post (env : aeneas_oracle_shim.Env) (xa xb : Std.I128) (amp : Std.U128)
    (D : aeneas_oracle_shim.U256) (h : oracle.lp_stable.solve_stable_d env xa xb amp = ok (.Ok D)) :
    0 < xa.val ∧ 0 < xb.val ∧ xa.val ≤ 10 ^ 34 ∧ xb.val ≤ 10 ^ 34 ∧
      (1 ≤ amp.val ∧ amp.val ≤ 1000000) ∧
      StableDPost (min xa.val xb.val).toNat (max xa.val xb.val).toNat (2 * amp.val) (.Ok D) := by
  by_cases hamp : amp.val < 1 ∨ 1000000 < amp.val
  · rw [solve_stable_d_rejects_amp env xa xb amp hamp] at h; simp at h
  by_cases hr : xa.val ≤ 0 ∨ xb.val ≤ 0 ∨ 10 ^ 34 < xa.val ∨ 10 ^ 34 < xb.val
  · rw [solve_stable_d_rejects_reserve env xa xb amp hr] at h; simp at h
  have hs := solve_stable_d_spec env xa xb amp (by omega) (by omega) (by omega) (by omega) (by omega)
  rw [h, WP.spec_ok] at hs
  exact ⟨by omega, by omega, by omega, by omega, by omega, hs⟩

/-- **`solve_stable_d` never traps, for any input.** For every pair of `i128`
reserves and every `u128` amplification, `solve_stable_d` returns (no host
trap, no panic) either `Ok(_)` or `Err(InvalidPrice)`: outside the admitted
domain by its guards (`solve_stable_d_rejects_amp`, `_rejects_reserve`),
inside it by `solve_stable_d_spec`. -/
theorem solve_stable_d_fails_closed (env : aeneas_oracle_shim.Env) (xa xb : Std.I128)
    (amp : Std.U128) :
    oracle.lp_stable.solve_stable_d env xa xb amp ⦃ r =>
      match r with
      | .Ok _ => True
      | .Err e => e = .InvalidPrice ⦄ := by
  by_cases hamp : amp.val < 1 ∨ 1000000 < amp.val
  · rw [solve_stable_d_rejects_amp env xa xb amp hamp]; simp
  by_cases hr : xa.val ≤ 0 ∨ xb.val ≤ 0 ∨ 10 ^ 34 < xa.val ∨ 10 ^ 34 < xb.val
  · rw [solve_stable_d_rejects_reserve env xa xb amp hr]; simp
  apply WP.spec_mono (solve_stable_d_spec env xa xb amp (by omega) (by omega) (by omega)
    (by omega) (by omega))
  intro r hr
  cases r with
  | Ok _ => trivial
  | Err e => exact hr

/-! ## `fair_stable_lp_price_wad` -/

/-- **The stable LP price is `⌊D · min_price / S⌋`, or fails closed.** For
positive reserves, prices and share supply, with `try_amount_to_wad` giving
`Ok xa`, `Ok xb` for the reserves and `Ok S` for the share supply, and
`solve_stable_d(xa, xb, amp) = Ok D`, the pricer returns (no trap) either
`Ok p` with `p = ⌊D · min(price_a, price_b) / S⌋` (stated as `IsFloorPos`),
or `Err(InvalidPrice)` exactly when that floor is `≥ 2^127`. Why nothing
traps: `D ≤ 2(xa + xb) ≤ 4·10^34` (`solve_stable_d_ok_post`) and
`min_price < 2^127`, so `D·min_price < 2^256`; `S > 0`
(`try_amount_to_wad_pos`), so the division is defined and the
`share_supply_wad <= 0` branch is unreachable. `D` itself satisfies
`StableDPost` (`solve_stable_d_ok_post`): `D* - 1 < D ≤ D* + 5`. -/
theorem fair_stable_lp_price_wad_value (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) (amp : Std.U128)
    (hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧ 0 < b.price_wad.val ∧
      0 < s.total_shares.val)
    (xa xb S : Std.I128) (D : aeneas_oracle_shim.U256)
    (ha : oracle.observation.try_amount_to_wad a.reserve a.decimals = ok (.Ok xa))
    (hb : oracle.observation.try_amount_to_wad b.reserve b.decimals = ok (.Ok xb))
    (hD : oracle.lp_stable.solve_stable_d env xa xb amp = ok (.Ok D))
    (hS : oracle.observation.try_amount_to_wad s.total_shares s.decimals = ok (.Ok S)) :
    oracle.lp_stable.fair_stable_lp_price_wad env a b s amp ⦃ r =>
      match r with
      | .Ok p => IsFloorPos ((D.val : Int) * min a.price_wad.val b.price_wad.val) S.val p.val
      | .Err e => e = .InvalidPrice ∧
          ∀ q : Int, IsFloorPos ((D.val : Int) * min a.price_wad.val b.price_wad.val) S.val q →
            2^127 ≤ q ⦄ := by
  have hSpos := try_amount_to_wad_pos s.total_shares s.decimals S hpos.2.2.2.2 hS
  obtain ⟨hxa0, hxb0, hxa1, hxb1, -, hpost⟩ := solve_stable_d_ok_post env xa xb amp D hD
  have hDle : D.val ≤ 4 * 10 ^ 34 := by
    have := hpost.2.1; omega
  have hSb := S.hBounds
  have hpa := a.price_wad.hBounds
  have hpb := b.price_wad.hBounds
  simp only [IScalarTy.numBits] at hSb hpa hpb
  unfold oracle.lp_stable.fair_stable_lp_price_wad
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g6 : ¬ (S ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  simp only [g1, g2, g3, g4, g5, ↓reduceIte, ha, hb, hD, hS, bind_ok,
    core.result.Result.Insts.CoreOpsTry.branch, lift, g6]
  have hmin := core.cmp.impls.OrdI128.min_val a.price_wad b.price_wad
  have hcm := hcast_u128_val (core.cmp.impls.OrdI128.min a.price_wad b.price_wad) (by omega)
  have hcS := hcast_u128_val S (by omega)
  step as ⟨mp, hmp⟩
  step with aeneas_oracle_shim.U256.mul_spec D mp (by
    have h1 : mp.val < 2 ^ 127 := by omega
    have := Nat.mul_le_mul hDle h1.le
    have h2 : 4 * 10 ^ 34 * 2 ^ 127 ≤ 2 ^ 256 - 1 := by norm_num
    omega) as ⟨m, hm⟩
  step as ⟨sd, hsd⟩
  step as ⟨f, hf⟩
  step with try_u256_to_i128_spec f as ⟨o, ho⟩
  have hmpI : (mp.val : Int) = min a.price_wad.val b.price_wad.val := by omega
  have hsdI : (sd.val : Int) = S.val := by omega
  have hfl := floorPos_of_ediv ((D.val : Int) * min a.price_wad.val b.price_wad.val) S.val hSpos
  have hcastq : (f.val : Int) = (D.val : Int) * min a.price_wad.val b.price_wad.val / S.val := by
    rw [hf, hm, ← hmpI, ← hsdI]; push_cast; rfl
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

/-- **The stable pricer never traps, for any input.** For every pair of legs,
share supply and amplification, `fair_stable_lp_price_wad` returns (no host
trap, no panic) `Ok _` or `Err(InvalidPrice)`. This closes the gap left by the
structural theorems: the Newton loop is trap-free on everything
`solve_stable_d` admits (`solve_stable_d_spec`), and `D·min_price` fits
`U256` (`fair_stable_lp_price_wad_value`). Unit test
`absurd_reserve_errors_not_panics`, here for every input. -/
theorem fair_stable_lp_price_wad_fails_closed (env : aeneas_oracle_shim.Env)
    (a b : oracle.lp.LpLeg) (s : oracle.lp.LpSupply) (amp : Std.U128) :
    oracle.lp_stable.fair_stable_lp_price_wad env a b s amp ⦃ r =>
      match r with
      | .Ok _ => True
      | .Err e => e = .InvalidPrice ⦄ := by
  by_cases hdeg : a.reserve.val ≤ 0 ∨ b.reserve.val ≤ 0 ∨ a.price_wad.val ≤ 0 ∨
      b.price_wad.val ≤ 0 ∨ s.total_shares.val ≤ 0
  · rw [fair_stable_lp_price_wad_rejects_degenerate env a b s amp hdeg]; simp
  have hpos : 0 < a.reserve.val ∧ 0 < b.reserve.val ∧ 0 < a.price_wad.val ∧
      0 < b.price_wad.val ∧ 0 < s.total_shares.val := by omega
  have h0 : (0#i128 : Std.I128).val = 0 := by simp
  have g1 : ¬ (a.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g2 : ¬ (b.reserve ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g3 : ¬ (a.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g4 : ¬ (b.price_wad ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  have g5 : ¬ (s.total_shares ≤ 0#i128) := by rw [IScalar.le_equiv, h0]; omega
  rcases try_amount_to_wad_cases a.reserve a.decimals with ⟨xa, ha⟩ | ha
  · rcases try_amount_to_wad_cases b.reserve b.decimals with ⟨xb, hb⟩ | hb
    · obtain ⟨r, hr, hP⟩ := spec_ok_eq (solve_stable_d_fails_closed env xa xb amp)
      cases r with
      | Ok D =>
        rcases try_amount_to_wad_cases s.total_shares s.decimals with ⟨S, hS⟩ | hS
        · apply WP.spec_mono (fair_stable_lp_price_wad_value env a b s amp hpos xa xb S D ha hb hr hS)
          intro r hr'
          cases r with
          | Ok _ => trivial
          | Err e => exact hr'.1
        · unfold oracle.lp_stable.fair_stable_lp_price_wad
          simp [g1, g2, g3, g4, g5, ha, hb, hr, hS, core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
            core.convert.FromSame.from, lift]
      | Err e =>
        simp only at hP
        unfold oracle.lp_stable.fair_stable_lp_price_wad
        simp [g1, g2, g3, g4, g5, ha, hb, hr, hP, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
          core.convert.FromSame.from]
    · unfold oracle.lp_stable.fair_stable_lp_price_wad
      simp [g1, g2, g3, g4, g5, ha, hb, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
        core.convert.FromSame.from]
  · unfold oracle.lp_stable.fair_stable_lp_price_wad
    simp [g1, g2, g3, g4, g5, ha, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTry_traitFromResidualResult.from_residual,
      core.convert.FromSame.from]

end OracleAeneas.Proofs
