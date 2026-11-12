/-
`liquidation::math::process_excess_payment`: the backward trimming loop.

The loop walks the repayment legs from the last one backwards. Each visited
leg is kept as is (non-positive amount or zero recorded value), trimmed
(kept amount lowered, the rest refunded), or removed (its whole amount
refunded). Legs before the point where the loop stopped are untouched.
`LegFate` describes one visited leg; `ExcessShape` the whole output.
-/
import ControllerAeneas.Proofs.RepayPlanArith

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

/-! ## Per-leg arithmetic of the partial branch -/

/-- Kept amount with `keep_within_quote` (insolvent account): the floor of the
backed value in tokens, capped at the leg amount. It is in `[0, amount]` and
its exact value `a·10^(18−dec)·p/W` is at most the kept USD `usd − rem`. -/
theorem keep_amount_spec (env : Env) (usd rem price : math.fp.Wad) (dec : Std.U32)
    (amount : Std.I128)
    (hrem : 0 ≤ rem.val) (hlt : rem.val < usd.val) (hp : 0 < price.val) (hdec : dec.val ≤ 18)
    (hfit : usd.val * 10^18 ≤ (2^127 - 1) * price.val) (ham : 0 < amount.val) :
    (do
      let w ← math.fp.Wad.checked_sub usd env rem
      let w1 ← math.fp.Wad.div_floor w env price
      let i ← math.fp.Wad.to_token_floor w1 env dec
      ok (core.cmp.impls.OrdI128.min i amount)) ⦃ a =>
        0 ≤ a.val ∧ a.val ≤ amount.val ∧
        a.val * 10^(18 - dec.val) * price.val ≤ (usd.val - rem.val) * 10^18 ⦄ := by
  unfold math.fp.Wad.checked_sub
  step with checked_sub_nonneg_spec env usd rem hrem hlt.le as ⟨w, hw⟩
  have hw0 : 0 ≤ w.val := by linarith
  step with rp_div_floor_spec env w price hw0 hp (by rw [hw]; nlinarith) as ⟨w1, hw1⟩
  have hw1' := (isFloor_pos hp).mp hw1
  have hw10 : 0 ≤ w1.val := IsFloor.nonneg_of hp hw1 (by positivity)
  step with rp_to_token_floor_spec env w1 dec hdec hw10 as ⟨i, hi⟩
  have hS : (0 : Int) < 10^(18 - dec.val) := by positivity
  have hi' := (isFloor_pos hS).mp hi
  have hi0 : 0 ≤ i.val := IsFloor.nonneg_of hS hi hw10
  simp only [WP.spec_ok, core.cmp.impls.OrdI128.min_val]
  generalize (10:Int)^(18 - dec.val) = S at hS hi' ⊢
  refine ⟨le_min hi0 ham.le, min_le_right _ _, ?_⟩
  have h1 : min i.val amount.val * S ≤ i.val * S :=
    mul_le_mul_of_nonneg_right (min_le_left _ _) hS.le
  have h2 : min i.val amount.val * S * price.val ≤ i.val * S * price.val :=
    mul_le_mul_of_nonneg_right h1 hp.le
  have h3 : i.val * S * price.val ≤ w1.val * price.val :=
    mul_le_mul_of_nonneg_right hi'.1 hp.le
  rw [← hw]; linarith [hw1'.1]

/-- Kept amount without `keep_within_quote` (solvent account, or a release of
unbacked repayment): the ceiling of the backed value in tokens (`floor`, or
`floor + 1` when `floor` does not cover it), capped at the leg amount. It is
in `[1, amount]`, and either it is the whole amount or its exact value
`a·10^(18−dec)·p/W` covers the kept USD `usd − rem`. -/
theorem ceil_amount_spec (env : Env) (usd rem price : math.fp.Wad) (dec : Std.U32)
    (amount : Std.I128)
    (hrem : 0 < rem.val) (hlt : rem.val < usd.val) (hp : 0 < price.val) (hdec : dec.val ≤ 18)
    (hfit : usd.val * 10^18 ≤ (2^127 - 1) * price.val) (ham : 0 < amount.val) :
    (do
      let kept_usd ← math.fp.Wad.checked_sub usd env rem
      let w ← math.fp.Wad.div_floor kept_usd env price
      let floor ← math.fp.Wad.to_token_floor w env dec
      let w1 ← math.fp.Wad.from_token env floor dec
      let w2 ← math.fp.Wad.mul_floor w1 env price
      let covered ← core.cmp.PartialOrd.ge.trait_default math.fp.Wad.Insts.CoreCmpPartialOrdWad
        w2 kept_usd
      let floor1 ← if covered then ok floor else floor + 1#i128
      ok (core.cmp.impls.OrdI128.min floor1 amount)) ⦃ a =>
        0 < a.val ∧ a.val ≤ amount.val ∧
        (a.val = amount.val ∨ (usd.val - rem.val) * 10^18 ≤ a.val * 10^(18 - dec.val) * price.val) ⦄ := by
  unfold math.fp.Wad.checked_sub
  step with checked_sub_nonneg_spec env usd rem hrem.le hlt.le as ⟨k, hk⟩
  have hk0 : 0 < k.val := by linarith
  have hkW : k.val * 10^18 < (2^127 - 1) * price.val := by
    have hku : k.val < usd.val := by linarith
    have : k.val * 10^18 < usd.val * 10^18 := mul_lt_mul_of_pos_right hku (by norm_num)
    linarith
  step with rp_div_floor_spec env k price hk0.le hp (by nlinarith) as ⟨w, hw⟩
  have hw' := (isFloor_pos hp).mp hw
  have hw0 : 0 ≤ w.val := IsFloor.nonneg_of hp hw (by positivity)
  have hwhi : w.val < 2^127 - 1 := by
    by_contra h
    have : (2^127 - 1) * price.val ≤ w.val * price.val :=
      mul_le_mul_of_nonneg_right (not_lt.mp h) hp.le
    linarith [hw'.1]
  step with rp_to_token_floor_spec env w dec hdec hw0 as ⟨f, hf⟩
  have hS : (0 : Int) < 10^(18 - dec.val) := by positivity
  have hf' := (isFloor_pos hS).mp hf
  have hf0 : 0 ≤ f.val := IsFloor.nonneg_of hS hf hw0
  have hfS : f.val * 10^(18 - dec.val) ≤ w.val := hf'.1
  step with from_token_spec env f dec hdec (by constructor <;> nlinarith) as ⟨w1, hw1⟩
  have hw10 : 0 ≤ w1.val := by rw [hw1]; positivity
  have hw1p : w1.val * price.val < 2^127 * 10^18 := by
    have : w1.val * price.val ≤ w.val * price.val := by
      rw [hw1]; exact mul_le_mul_of_nonneg_right hfS hp.le
    have hkb := k.hBounds
    simp only [IScalarTy.numBits] at hkb
    have : k.val * 10^18 < 2^127 * 10^18 := mul_lt_mul_of_pos_right (by omega) (by norm_num)
    linarith [hw'.1]
  step with rp_mul_floor_spec env w1 price hw10 hp.le hw1p as ⟨w2, hw2⟩
  have hw2' := (isFloor_pos (by norm_num : (0:Int) < 10^18)).mp hw2
  simp only [wad_ge, bind_ok]
  generalize hSdef : (10:Int)^(18 - dec.val) = S at hS hf' hfS hw1
  by_cases hcov : k.val ≤ w2.val
  · simp only [hcov, decide_true, ↓reduceIte, bind_ok, WP.spec_ok,
      core.cmp.impls.OrdI128.min_val]
    have hf1 : 1 ≤ f.val := by
      by_contra h
      have hf00 : f.val = 0 := by omega
      rw [hf00, zero_mul] at hw1
      rw [hw1, zero_mul] at hw2'
      have : w2.val * 10^18 ≤ 0 := hw2'.1
      have : w2.val ≤ 0 := by nlinarith
      omega
    refine ⟨lt_min (by omega) ham, min_le_right _ _, ?_⟩
    rcases le_total f.val amount.val with hle | hle
    · right
      rw [min_eq_left hle, ← hk]
      have : k.val * 10^18 ≤ w2.val * 10^18 := mul_le_mul_of_nonneg_right hcov (by norm_num)
      rw [← hw1]; linarith [hw2'.1]
    · left; exact min_eq_right hle
  · simp only [hcov, decide_false, ↓reduceIte, Bool.false_eq_true]
    step as ⟨f1, hf1⟩
    simp only [core.cmp.impls.OrdI128.min_val]
    refine ⟨lt_min (by omega) ham, min_le_right _ _, ?_⟩
    rcases le_total f1.val amount.val with hle | hle
    · right
      rw [min_eq_left hle, ← hk, hf1]
      -- k·W < (w+1)·p ≤ (f+1)·S·p
      have h1 : w.val + 1 ≤ (f.val + 1) * S := by linarith [hf'.2]
      have h2 : (w.val + 1) * price.val ≤ (f.val + 1) * S * price.val :=
        mul_le_mul_of_nonneg_right h1 hp.le
      linarith [hw'.2]
    · left; exact min_eq_right hle

/-! ## Pure arithmetic of the kept amounts -/

/-- `keep_within_quote`: `a = min(⌊⌊kept·W/p⌋/S⌋, amount)` is in `[0, amount]`
and `a·S·p ≤ kept·W`. -/
theorem keep_amount_arith {S p usd rem kw w0 fi amount : Int} (hS : 0 < S) (hp : 0 < p)
    (hkw : kw = usd - rem) (hw0 : IsFloor (kw * 10^18) p w0) (hfi : IsFloor w0 S fi)
    (hw0nn : 0 ≤ w0) (ham : 0 < amount) :
    0 ≤ min fi amount ∧ min fi amount ≤ amount ∧ min fi amount * S * p ≤ (usd - rem) * 10^18 := by
  have hw1' := (isFloor_pos hp).mp hw0
  have hi' := (isFloor_pos hS).mp hfi
  have hi0 : 0 ≤ fi := IsFloor.nonneg_of hS hfi hw0nn
  refine ⟨le_min hi0 ham.le, min_le_right _ _, ?_⟩
  have h1 : min fi amount * S ≤ fi * S := mul_le_mul_of_nonneg_right (min_le_left _ _) hS.le
  have h2 : min fi amount * S * p ≤ fi * S * p := mul_le_mul_of_nonneg_right h1 hp.le
  have h3 : fi * S * p ≤ w0 * p := mul_le_mul_of_nonneg_right hi'.1 hp.le
  rw [← hkw]; linarith [hw1'.1]

/-- Without `keep_within_quote`, covered case: `a = min(f, amount)` with
`⌊f·S·p/W⌋ ≥ kept > 0`; then `a ≥ 1` and either `a = amount` or `kept·W ≤ a·S·p`. -/
theorem ceil_covered_arith {S p usd rem kw f v1 v2 amount : Int} (hS : 0 < S) (hp : 0 < p)
    (hkw : kw = usd - rem) (hk0 : 0 < kw) (hf0 : 0 ≤ f) (hv1 : v1 = f * S)
    (hv2 : v2 * 10^18 ≤ v1 * p ∧ v1 * p < v2 * 10^18 + 10^18) (hcv : kw ≤ v2) (ham : 0 < amount) :
    0 < min f amount ∧ min f amount ≤ amount ∧
      (min f amount = amount ∨ (usd - rem) * 10^18 ≤ min f amount * S * p) := by
  have hf1 : 1 ≤ f := by
    by_contra h
    have hf00 : f = 0 := by omega
    rw [hf00, zero_mul] at hv1
    rw [hv1, zero_mul] at hv2
    have : v2 * 10^18 ≤ 0 := hv2.1
    have : v2 ≤ 0 := by nlinarith
    omega
  refine ⟨lt_min (by omega) ham, min_le_right _ _, ?_⟩
  rcases le_total f amount with hle | hle
  · right
    rw [min_eq_left hle, ← hkw]
    have : kw * 10^18 ≤ v2 * 10^18 := mul_le_mul_of_nonneg_right hcv (by norm_num)
    rw [← hv1]; linarith [hv2.1]
  · left; exact min_eq_right hle

/-- Without `keep_within_quote`, uncovered case: `a = min(f + 1, amount)`; then
`a ≥ 1` and either `a = amount` or `kept·W ≤ a·S·p` (the ceiling covers). -/
theorem ceil_uncovered_arith {S p usd rem kw w0 f amount : Int} {f1 : Int} (hS : 0 < S) (hp : 0 < p)
    (hkw : kw = usd - rem) (hw : w0 * p ≤ kw * 10^18 ∧ kw * 10^18 < w0 * p + p)
    (hf : f * S ≤ w0 ∧ w0 < f * S + S) (hf0 : 0 ≤ f) (hf1 : f1 = f + 1) (ham : 0 < amount) :
    0 < min f1 amount ∧ min f1 amount ≤ amount ∧
      (min f1 amount = amount ∨ (usd - rem) * 10^18 ≤ min f1 amount * S * p) := by
  refine ⟨lt_min (by omega) ham, min_le_right _ _, ?_⟩
  rcases le_total f1 amount with hle | hle
  · right
    rw [min_eq_left hle, ← hkw, hf1]
    have h1 : w0 + 1 ≤ (f + 1) * S := by linarith [hf.2]
    have h2 : (w0 + 1) * p ≤ (f + 1) * S * p := mul_le_mul_of_nonneg_right h1 hp.le
    linarith [hw.2]
  · left; exact min_eq_right hle

/-! ## Half-up helpers -/

/-- Half-up quotient lower bound: `f·d ≤ n → f ≤ hu(n/d)`. -/
theorem rp_huD_ge_of {n d f : Int} (hd : 0 < d) (h : f * d ≤ n) : f ≤ huD n d := by
  obtain ⟨q, hq⟩ := IsHalfUp.exists n d hd
  rw [← isHalfUp_eq hd hq]; exact IsHalfUp.ge_of hd hq h

/-- Half-up quotient upper bound: `n ≤ c·d → hu(n/d) ≤ c`. -/
theorem rp_huD_le_of {n d c : Int} (hd : 0 < d) (h : n ≤ c * d) : huD n d ≤ c := by
  obtain ⟨q, hq⟩ := IsHalfUp.exists n d hd
  rw [← isHalfUp_eq hd hq]; exact IsHalfUp.le_of hd hq h

/-- Half-up rounding is monotone in the numerator. -/
theorem rp_huD_mono {n₁ n₂ d : Int} (hd : 0 < d) (h : n₁ ≤ n₂) : huD n₁ d ≤ huD n₂ d := by
  unfold huD; exact Int.ediv_le_ediv (by omega) (by omega)

/-- Half-up rounding of a non-negative numerator is non-negative. -/
theorem rp_huD_nonneg {n d : Int} (hd : 0 < d) (h : 0 ≤ n) : 0 ≤ huD n d := by
  unfold huD; exact Int.ediv_nonneg (by omega) (by omega)

/-! ## Shape of the output -/

/-- What the backward loop did to one visited leg `e`: `o` is what is left of
it in the output (`none` = removed) and `rs` the refunds it appended.

* `keep`: left unchanged, no refund (a leg with a non-positive amount or a
  zero recorded value is skipped);
* `trim`: amount lowered to `a ∈ (0, amount]` and value to `u ∈ [0, usd]`,
  every other field unchanged, one refund of exactly `amount − a` of the
  leg's asset;
* `drop`: removed, one refund of its whole (positive) amount. -/
inductive LegFate (e : types.RepayEntry) : Option types.RepayEntry → List types.PaymentTuple → Prop
  | keep : LegFate e (some e) []
  | trim (a u r : Std.I128) : 0 < a.val → a.val ≤ e.amount.val → 0 ≤ u.val → u.val ≤ e.usd_wad.val →
      r.val = e.amount.val - a.val →
      LegFate e (some { e with amount := a, usd_wad := u }) [{ asset := e.hub_asset.asset, amount := r }]
  | drop (r : Std.I128) : 0 < e.amount.val → r.val = e.amount.val →
      LegFate e none [{ asset := e.hub_asset.asset, amount := r }]

/-- One record per visited leg: the leg, what is left of it, its refunds. -/
abbrev Fates := List (types.RepayEntry × Option types.RepayEntry × List types.PaymentTuple)

/-- **Shape of the output of `process_excess_payment`.** For input legs `L`
and initial refunds `r0`: there is a cut `k ≤ |L|` such that the first `k`
legs are untouched and the legs `L[k..]` were visited, each with a
`LegFate`; the output legs are `L[..k]` followed by what is left of the
visited legs, in their original order; the refunds are `r0` followed by the
visited legs' refunds, last leg first (the loop runs backwards). -/
def ExcessShape (L : List types.RepayEntry) (r0 : List types.PaymentTuple) (k : Nat)
    (cur : List types.RepayEntry) (rf : List types.PaymentTuple) : Prop :=
  k ≤ L.length ∧ ∃ F : Fates,
    F.map (·.1) = L.drop k ∧ (∀ f ∈ F, LegFate f.1 f.2.1 f.2.2) ∧
    cur = L.take k ++ F.filterMap (·.2.1) ∧ rf = r0 ++ F.reverse.flatMap (·.2.2)

/-- Before the loop: nothing visited, the cut is at the end of the list. -/
theorem shape_init (L : List types.RepayEntry) (r0 : List types.PaymentTuple) :
    ExcessShape L r0 L.length L r0 :=
  ⟨le_refl _, [], by simp, by simp, by simp, by simp⟩

/-- A shape with cut `i + 1` exposes leg `L[i]` at index `i` of the current list. -/
theorem shape_cur {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {i : Nat}
    {cur : List types.RepayEntry} {rf : List types.PaymentTuple}
    (h : ExcessShape L r0 (i+1) cur rf) :
    ∃ e T, L[i]? = some e ∧ cur = L.take i ++ e :: T := by
  obtain ⟨hk, F, -, -, hcur, -⟩ := h
  have hi : i < L.length := by omega
  refine ⟨L[i], F.filterMap (·.2.1), List.getElem?_eq_getElem hi, ?_⟩
  rw [hcur, List.take_succ_eq_append_getElem hi]
  simp only [List.append_assoc, List.singleton_append]

/-- Visiting leg `L[i]` with fate `(o, rs)` moves the cut to `i`. -/
theorem shape_step {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {i : Nat}
    {e : types.RepayEntry} {T : List types.RepayEntry} {rf : List types.PaymentTuple}
    {o : Option types.RepayEntry} {rs : List types.PaymentTuple}
    (h : ExcessShape L r0 (i+1) (L.take i ++ e :: T) rf) (he : L[i]? = some e)
    (hf : LegFate e o rs) :
    ExcessShape L r0 i (L.take i ++ (o.toList ++ T)) (rf ++ rs) := by
  obtain ⟨hk, F, hF, hfate, hcur, hrf⟩ := h
  have hi : i < L.length := by omega
  have hLi : L[i] = e := by rw [List.getElem?_eq_getElem hi] at he; exact Option.some.inj he
  have hT : T = F.filterMap (·.2.1) := by
    rw [List.take_succ_eq_append_getElem hi] at hcur
    simp only [List.append_assoc, List.singleton_append, hLi, List.append_cancel_left_eq,
      List.cons.injEq, true_and] at hcur
    exact hcur
  refine ⟨by omega, (e, o, rs) :: F, ?_, ?_, ?_, ?_⟩
  · rw [List.map_cons, hF, List.drop_eq_getElem_cons hi, hLi]
  · intro f hfm
    rcases List.mem_cons.mp hfm with rfl | hm
    · exact hf
    · exact hfate f hm
  · rw [hT]; cases o <;> simp
  · rw [hrf]; simp

/-- The untouched prefix has exactly `k` legs. -/
theorem shape_take_len {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {k : Nat}
    {cur : List types.RepayEntry} {rf : List types.PaymentTuple}
    (h : ExcessShape L r0 k cur rf) : (L.take k).length = k := by
  have := h.1; simp; omega

/-! ## Sums -/

theorem rp_usdSum_append (P Q : List types.RepayEntry) : usdSum (P ++ Q) = usdSum P + usdSum Q := by
  simp [usdSum]

/-! ## The loop -/

/-- Conditions on one input leg (`keep` = `keep_within_quote`). The recorded
value is non-negative. A leg the loop may trim (positive amount and value)
has a positive price, at most 18 decimals, a value with `usd·W ≤ (2^127−1)·p`
(the token quotient fits `i128`), an amount whose WAD upscale and USD value
fit `i128`, and, without `keep`, a recorded value at most the half-up value
of its amount (`usd ≤ tokUsd(amount)`, true for every leg
`calculate_repayment_amounts` records, see `RepayPlan.lean`). -/
def ExcessLegOK (keep : Bool) (e : types.RepayEntry) : Prop :=
  0 ≤ e.usd_wad.val ∧
  (0 < e.amount.val → 0 < e.usd_wad.val →
    0 < e.feed.price_wad.val ∧ e.feed.asset_decimals.val ≤ 18 ∧
    e.usd_wad.val * 10^18 ≤ (2^127 - 1) * e.feed.price_wad.val ∧
    e.amount.val * 10^(18 - e.feed.asset_decimals.val) < 2^127 ∧
    tokUsd e.amount.val e.feed.asset_decimals.val e.feed.price_wad.val < 2^127 ∧
    (keep = false →
      e.usd_wad.val ≤ tokUsd e.amount.val e.feed.asset_decimals.val e.feed.price_wad.val))

/-- Loop invariant of `process_excess_payment` at index `k` with remaining
excess `rem`: the shape (`ExcessShape`), `rem ≥ 0`, the recorded value never
grew, the value removed so far is at most `E − rem` without `keep` and at
least `E − rem` with `keep`, and while excess remains every visited leg still
present was skipped (non-positive amount or zero value). -/
def ExcessInv (L : List types.RepayEntry) (r0 : List types.PaymentTuple) (E : Int) (keep : Bool)
    (k : Nat) (cur : List types.RepayEntry) (rf : List types.PaymentTuple) (rem : Int) : Prop :=
  ExcessShape L r0 k cur rf ∧ 0 ≤ rem ∧ usdSum cur ≤ usdSum L ∧
  (keep = false → usdSum L - E + rem ≤ usdSum cur) ∧
  (keep = true → usdSum cur ≤ usdSum L - E + rem) ∧
  (0 < rem → ∀ x ∈ cur.drop k, x.amount.val ≤ 0 ∨ x.usd_wad.val = 0)

/-- What `process_excess_payment` guarantees about its output. -/
def ExcessPost (L : List types.RepayEntry) (r0 : List types.PaymentTuple) (E : Int) (keep : Bool)
    (out : List types.RepayEntry × List types.PaymentTuple) : Prop :=
  ∃ k, ExcessShape L r0 k out.1 out.2 ∧ usdSum out.1 ≤ usdSum L ∧
    (keep = false → usdSum L - E ≤ usdSum out.1) ∧
    (keep = true → usdSum out.1 ≤ usdSum L - E ∨
      (k = 0 ∧ ∀ x ∈ out.1, x.amount.val ≤ 0 ∨ x.usd_wad.val = 0))

/-- One iteration of the generated loop, with the recursive call folded. -/
theorem excess_loop_unfold (env : Env) (cur : List types.RepayEntry) (rf : List types.PaymentTuple)
    (keep : Bool) (rem : math.fp.Wad) (idx : Std.U32) :
    positions.liquidation.math.process_excess_payment_loop env cur rf keep rem idx =
      (do
        let r ← positions.liquidation.math.process_excess_payment_loop.body env keep cur rf rem idx
        match r with
        | ControlFlow.cont (a, b, c, d) =>
          positions.liquidation.math.process_excess_payment_loop env a b keep c d
        | ControlFlow.done x => ok x) := by
  conv => lhs; unfold positions.liquidation.math.process_excess_payment_loop; rw [loop]
  dsimp only
  congr 1; funext r
  cases r with
  | cont x => obtain ⟨a, b, c, d⟩ := x; rfl
  | done x => rfl

/-- When the loop stops (no excess left, or index 0) the invariant gives the postcondition. -/
theorem excess_done {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {E : Int}
    {keep : Bool} {k : Nat} {cur : List types.RepayEntry} {rf : List types.PaymentTuple}
    {rem : Int} (h : ExcessInv L r0 E keep k cur rf rem) (hstop : rem ≤ 0 ∨ k = 0) :
    ExcessPost L r0 E keep (cur, rf) := by
  obtain ⟨hsh, hrem0, hle, hnk, hk, hz⟩ := h
  refine ⟨k, hsh, hle, fun h => by have := hnk h; linarith, fun h => ?_⟩
  rcases hstop with hs | hs
  · left; have := hk h; linarith
  · by_cases hr : 0 < rem
    · right; refine ⟨hs, ?_⟩; have := hz hr; subst hs; simpa using this
    · left; have := hk h; linarith

/-- `usdSum (e :: T) = usd(e) + usdSum T`. -/
theorem rp_usdSum_cons' (e : types.RepayEntry) (T : List types.RepayEntry) :
    usdSum (e :: T) = e.usd_wad.val + usdSum T := usdSum_cons e T

/-- Dropping the prefix and the visited leg leaves the tail. -/
theorem rp_drop_mid {P T : List types.RepayEntry} {e : types.RepayEntry} {i : Nat}
    (h : P.length = i) : (P ++ e :: T).drop (i + 1) = T := by
  subst h; simp

/-- Dropping the prefix leaves the tail. -/
theorem rp_drop_mid' {P T : List types.RepayEntry} {i : Nat}
    (h : P.length = i) : (P ++ T).drop i = T := by
  subst h; simp

/-- A skipped leg (non-positive amount or zero value): nothing changes. -/
theorem inv_skip {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {E : Int} {keep : Bool}
    {i : Nat} {e : types.RepayEntry} {T : List types.RepayEntry} {rf : List types.PaymentTuple}
    {rem : Int} (h : ExcessInv L r0 E keep (i+1) (L.take i ++ e :: T) rf rem)
    (he : L[i]? = some e) (hs : e.amount.val ≤ 0 ∨ e.usd_wad.val = 0) :
    ExcessInv L r0 E keep i (L.take i ++ e :: T) rf rem := by
  obtain ⟨hsh, hrem0, hle, hnk, hk, hz⟩ := h
  have hlen := shape_take_len hsh
  have hlen' : (L.take i).length = i := by simp; have := hsh.1; omega
  refine ⟨?_, hrem0, hle, hnk, hk, ?_⟩
  · have := shape_step (o := some e) (rs := []) hsh he LegFate.keep
    simpa using this
  · intro hr x hx
    rw [show L.take i ++ e :: T = L.take i ++ (e :: T) from rfl, rp_drop_mid' hlen'] at hx
    rcases List.mem_cons.mp hx with rfl | hm
    · exact hs
    · exact hz hr x (by rw [rp_drop_mid hlen']; exact hm)

/-- A partial trim (the leg left as `o`, value `u = usdSum o.toList`) after
which no excess remains. -/
theorem inv_partial {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {E : Int}
    {keep : Bool} {i : Nat} {e : types.RepayEntry} {T : List types.RepayEntry}
    {rf : List types.PaymentTuple} {rem : Int} {o : Option types.RepayEntry}
    {rs : List types.PaymentTuple}
    (h : ExcessInv L r0 E keep (i+1) (L.take i ++ e :: T) rf rem)
    (he : L[i]? = some e) (hf : LegFate e o rs)
    (hu : usdSum o.toList ≤ e.usd_wad.val)
    (hkeep : keep = true → usdSum o.toList ≤ e.usd_wad.val - rem)
    (hnkeep : keep = false → e.usd_wad.val - rem ≤ usdSum o.toList) :
    ExcessInv L r0 E keep i (L.take i ++ (o.toList ++ T)) (rf ++ rs) 0 := by
  obtain ⟨hsh, hrem0, hle, hnk, hk, hz⟩ := h
  refine ⟨shape_step hsh he hf, le_refl _, ?_, ?_, ?_, fun h => absurd h (lt_irrefl _)⟩
  · rw [rp_usdSum_append, rp_usdSum_append] at *; rw [rp_usdSum_cons'] at hle; linarith
  · intro hkf; have := hnk hkf; have := hnkeep hkf
    rw [rp_usdSum_append, rp_usdSum_append] at *; rw [rp_usdSum_cons'] at *; linarith
  · intro hkt; have := hk hkt; have := hkeep hkt
    rw [rp_usdSum_append, rp_usdSum_append] at *; rw [rp_usdSum_cons'] at *; linarith

/-- A whole leg removed, its value `usd ≤ rem` taken off the remaining excess. -/
theorem inv_remove {L : List types.RepayEntry} {r0 : List types.PaymentTuple} {E : Int}
    {keep : Bool} {i : Nat} {e : types.RepayEntry} {T : List types.RepayEntry}
    {rf : List types.PaymentTuple} {rem : Int} {rs : List types.PaymentTuple}
    (h : ExcessInv L r0 E keep (i+1) (L.take i ++ e :: T) rf rem)
    (he : L[i]? = some e) (hf : LegFate e none rs) (hu0 : 0 ≤ e.usd_wad.val)
    (hur : e.usd_wad.val ≤ rem) :
    ExcessInv L r0 E keep i (L.take i ++ T) (rf ++ rs) (rem - e.usd_wad.val) := by
  obtain ⟨hsh, hrem0, hle, hnk, hk, hz⟩ := h
  have hlen' : (L.take i).length = i := by simp; have := hsh.1; omega
  have hs := shape_step hsh he hf
  simp only [Option.toList_none, List.nil_append] at hs
  refine ⟨hs, by linarith, ?_, ?_, ?_, ?_⟩
  · rw [rp_usdSum_append] at *; rw [rp_usdSum_cons'] at hle; linarith
  · intro hkf; have := hnk hkf
    rw [rp_usdSum_append] at *; rw [rp_usdSum_cons'] at *; linarith
  · intro hkt; have := hk hkt
    rw [rp_usdSum_append] at *; rw [rp_usdSum_cons'] at *; linarith
  · intro hr x hx
    rw [rp_drop_mid' hlen'] at hx
    exact hz (by linarith) x (by rw [rp_drop_mid hlen']; exact hx)

/-- **Loop invariant of `process_excess_payment`.** From any state satisfying `ExcessInv` at index `k` (with `idx = k`), the generated loop returns (never aborts) an output satisfying `ExcessPost`. Induction on `k`; each iteration keeps, trims or removes leg `L[k−1]`. -/
theorem excess_loop_spec (env : Env) (L : List types.RepayEntry) (r0 : List types.PaymentTuple)
    (E : Int) (keep : Bool) (hok : ∀ e ∈ L, ExcessLegOK keep e) :
    ∀ (k : Nat) (cur : List types.RepayEntry) (rf : List types.PaymentTuple) (rem : math.fp.Wad)
      (idx : Std.U32), idx.val = k → ExcessInv L r0 E keep k cur rf rem.val →
      positions.liquidation.math.process_excess_payment_loop env cur rf keep rem idx
        ⦃ out => ExcessPost L r0 E keep out ⦄ := by
  intro k
  induction k with
  | zero =>
    intro cur rf rem idx hidx hinv
    rw [excess_loop_unfold]
    have hidx0 : ¬ (idx > 0#u32) := by simp [UScalar.lt_equiv]; omega
    simp only [positions.liquidation.math.process_excess_payment_loop.body, wad_gt, rp_zero_val,
      hidx0, ↓reduceIte, bind_ok]
    split <;> simp only [bind_ok, WP.spec_ok] <;> exact excess_done hinv (Or.inr rfl)
  | succ i ih =>
    intro cur rf rem idx hidx hinv
    have hinv0 := hinv
    obtain ⟨hsh, hrem0, hle, hnk, hk, hz⟩ := hinv
    obtain ⟨e, T, he, hcur⟩ := shape_cur hsh
    subst hcur
    have hiL : i < L.length := by have := hsh.1; omega
    have hlen : (L.take i).length = i := by simp; omega
    have heL : e ∈ L := List.mem_of_getElem? he
    obtain ⟨husd0, hpart⟩ := hok e heL
    rw [excess_loop_unfold]
    simp only [positions.liquidation.math.process_excess_payment_loop.body, wad_gt, rp_zero_val]
    by_cases hr : 0 < rem.val
    swap
    · simp only [hr, decide_false, Bool.false_eq_true, ↓reduceIte, bind_ok, WP.spec_ok]
      exact excess_done hinv0 (Or.inl (by linarith))
    have hidx0 : idx > 0#u32 := by simp [UScalar.lt_equiv]; omega
    simp only [hr, decide_true, ↓reduceIte, hidx0, bind_ok]
    step as ⟨idx1, hidx1⟩
    have hi1 : idx1.val = i := by omega
    have hget : aeneas_soroban_shim.Vec.get (L.take i ++ e :: T) idx1 = ok (some e) := by
      simp only [aeneas_soroban_shim.Vec.get, hi1]
      rw [← hlen]; simp
    simp only [hget, bind_ok, validation.expect_invariant, core.option.Option.unwrap_or_else,
      wad_from_same]
    have hrm : aeneas_soroban_shim.Vec.remove (L.take i ++ e :: T) idx1 =
        ok (some (), L.take i ++ T) := by
      simp only [aeneas_soroban_shim.Vec.remove, hi1]
      rw [List.eraseIdx_append_of_length_le (by omega)]
      simp [hlen]
    have hset : ∀ x, aeneas_soroban_shim.Vec.set (L.take i ++ e :: T) idx1 x =
        ok (L.take i ++ x :: T) := by
      intro x
      simp only [aeneas_soroban_shim.Vec.set, hi1]
      rw [List.set_append_right _ _ (by omega)]
      simp [hlen]
    by_cases ham : e.amount.val ≤ 0
    · have hc : e.amount ≤ 0#i128 := by simp [IScalar.le_equiv]; omega
      simp only [hc, ↓reduceIte, bind_ok]
      exact ih _ rf rem idx1 hi1 (inv_skip hinv0 he (Or.inl ham))
    have hc : ¬ (e.amount ≤ 0#i128) := by simp [IScalar.le_equiv]; omega
    simp only [hc, ↓reduceIte, math.fp.Wad.Insts.CoreCmpPartialEqWad.eq, bind_ok]
    by_cases hu0 : e.usd_wad.val = 0
    · have hz0 : e.usd_wad = math.fp.Wad.ZERO := by
        apply IScalar.eq_of_val_eq; rw [hu0, rp_zero_val]
      simp only [hz0, decide_true, ↓reduceIte, bind_ok]
      exact ih _ rf rem idx1 hi1 (inv_skip hinv0 he (Or.inr hu0))
    have hne : ¬ (e.usd_wad = math.fp.Wad.ZERO) := fun h => hu0 (by rw [h, rp_zero_val])
    simp only [hne, decide_false, Bool.false_eq_true, ↓reduceIte]
    have hamp : 0 < e.amount.val := by omega
    have hup : 0 < e.usd_wad.val := lt_of_le_of_ne husd0 (Ne.symm hu0)
    obtain ⟨hp, hdec, hfitU, hfitA, hfitT, hback⟩ := hpart hamp hup
    have hS : (0:Int) < 10^(18 - e.feed.asset_decimals.val) := by positivity
    by_cases hlt : rem.val < e.usd_wad.val
    swap
    · -- the whole leg is excess: remove it and refund its amount
      simp only [hlt, decide_false, Bool.false_eq_true, ↓reduceIte, bind_ok,
        aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone, aeneas_soroban_shim.Vec.push_back,
        hrm]
      unfold math.fp.Wad.checked_sub
      step with checked_sub_nonneg_spec env rem e.usd_wad husd0 (by linarith) as ⟨r', hr'⟩
      have hi := inv_remove hinv0 he (LegFate.drop e.amount hamp rfl) husd0 (by linarith)
      rw [← hr'] at hi
      exact ih _ _ r' idx1 hi1 hi
    -- partial trim
    simp only [hlt, decide_true, ↓reduceIte]
    cases keep with
    | true =>
      simp only [↓reduceIte]
      unfold math.fp.Wad.checked_sub
      step with checked_sub_nonneg_spec env e.usd_wad rem hrem0 hlt.le as ⟨kw, hkw⟩
      have hkw0 : 0 ≤ kw.val := by linarith
      step with rp_div_floor_spec env kw e.feed.price_wad hkw0 hp
        (by rw [hkw]; nlinarith) as ⟨w0, hw0f⟩
      have hw0nn : 0 ≤ w0.val := IsFloor.nonneg_of hp hw0f (by positivity)
      step with rp_to_token_floor_spec env w0 e.feed.asset_decimals hdec hw0nn as ⟨fi, hfi⟩
      obtain ⟨ha0, hale, haS⟩ := keep_amount_arith hS hp hkw hw0f hfi hw0nn hamp
      rw [← core.cmp.impls.OrdI128.min_val] at ha0 hale haS
      generalize core.cmp.impls.OrdI128.min fi e.amount = a at ha0 hale haS ⊢
      have haS' : a.val * 10^(18 - e.feed.asset_decimals.val) ≤
          e.amount.val * 10^(18 - e.feed.asset_decimals.val) :=
        mul_le_mul_of_nonneg_right hale hS.le
      step with from_token_spec env a e.feed.asset_decimals hdec
        (by constructor <;> nlinarith) as ⟨w, hw⟩
      have hw0 : 0 ≤ w.val := by rw [hw]; positivity
      have htokle : huD (w.val * e.feed.price_wad.val) (10^18) ≤
          tokUsd e.amount.val e.feed.asset_decimals.val e.feed.price_wad.val := by
        rw [hw]; unfold tokUsd; apply rp_huD_mono (by norm_num)
        exact mul_le_mul_of_nonneg_right haS' hp.le
      step with rp_mul_spec env w e.feed.price_wad hw0 hp.le (by linarith) as ⟨w1, hw1⟩
      have hw1le : w1.val ≤ e.usd_wad.val - rem.val := by
        rw [hw1, hw]; apply rp_huD_le_of (by norm_num); linarith [haS]
      have hw10 : 0 ≤ w1.val := by
        rw [hw1]; apply rp_huD_nonneg (by norm_num); positivity
      have hnlt : ¬ (e.usd_wad.val < w1.val) := by linarith
      simp only [wad_min, hnlt, ↓reduceIte, bind_ok,
        aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone, aeneas_soroban_shim.Vec.push_back,
        math.fp.Wad.raw]
      step as ⟨r, hr⟩
      by_cases ha00 : a = 0#i128
      · have haz : a.val = 0 := by rw [ha00]; rfl
        simp only [ha00, ↓reduceIte, hrm, bind_ok]
        step with checked_sub_nonneg_spec env e.usd_wad w1 hw10 (by linarith) as ⟨rm, hrm'⟩
        have hge : rem.val ≤ rm.val := by rw [hrm']; linarith
        simp only [wad_ge, hge, decide_true, ↓reduceIte, bind_ok]
        have hi := inv_partial (o := none) hinv0 he
          (LegFate.drop r hamp (by rw [hr, haz]; ring))
          (by simp [usdSum]; linarith) (fun _ => by simp [usdSum]; linarith) (fun h => by cases h)
        simp only [Option.toList_none, List.nil_append] at hi
        rw [← rp_zero_val] at hi
        exact ih _ _ _ idx1 hi1 hi
      · have ha0' : 0 < a.val := by
          have : a.val ≠ 0 := fun h => ha00 (IScalar.eq_of_val_eq (by rw [h]; rfl))
          omega
        simp only [ha00, ↓reduceIte, hset, bind_ok]
        step with checked_sub_nonneg_spec env e.usd_wad w1 hw10 (by linarith) as ⟨rm, hrm'⟩
        have hge : rem.val ≤ rm.val := by rw [hrm']; linarith
        simp only [wad_ge, hge, decide_true, ↓reduceIte, bind_ok]
        have hi := inv_partial hinv0 he
          (LegFate.trim a w1 r ha0' hale hw10 (by linarith) hr)
          (by simp [usdSum]; linarith) (fun _ => by simp [usdSum]; linarith) (fun h => by cases h)
        simp only [Option.toList_some, List.singleton_append] at hi
        rw [← rp_zero_val] at hi
        exact ih _ _ _ idx1 hi1 hi
    | false =>
      simp only [Bool.false_eq_true, ↓reduceIte]
      unfold math.fp.Wad.checked_sub
      step with checked_sub_nonneg_spec env e.usd_wad rem hrem0 hlt.le as ⟨kw, hkw⟩
      have hk0 : 0 < kw.val := by linarith
      have hkW : kw.val * 10^18 < (2^127 - 1) * e.feed.price_wad.val := by
        have hku : kw.val < e.usd_wad.val := by linarith
        have : kw.val * 10^18 < e.usd_wad.val * 10^18 := mul_lt_mul_of_pos_right hku (by norm_num)
        linarith
      step with rp_div_floor_spec env kw e.feed.price_wad hk0.le hp (by nlinarith) as ⟨w0, hw0f⟩
      have hw' := (isFloor_pos hp).mp hw0f
      have hw0nn : 0 ≤ w0.val := IsFloor.nonneg_of hp hw0f (by positivity)
      have hwhi : w0.val < 2^127 - 1 := by
        by_contra h
        have : (2^127 - 1) * e.feed.price_wad.val ≤ w0.val * e.feed.price_wad.val :=
          mul_le_mul_of_nonneg_right (not_lt.mp h) hp.le
        linarith [hw'.1]
      step with rp_to_token_floor_spec env w0 e.feed.asset_decimals hdec hw0nn as ⟨f, hf⟩
      have hf' := (isFloor_pos hS).mp hf
      have hf0 : 0 ≤ f.val := IsFloor.nonneg_of hS hf hw0nn
      have hfS : f.val * 10^(18 - e.feed.asset_decimals.val) ≤ w0.val := hf'.1
      step with from_token_spec env f e.feed.asset_decimals hdec
        (by constructor <;> nlinarith) as ⟨v1, hv1⟩
      have hv10 : 0 ≤ v1.val := by rw [hv1]; positivity
      have hv1p : v1.val * e.feed.price_wad.val < 2^127 * 10^18 := by
        have : v1.val * e.feed.price_wad.val ≤ w0.val * e.feed.price_wad.val := by
          rw [hv1]; exact mul_le_mul_of_nonneg_right hfS hp.le
        have hkb := kw.hBounds
        simp only [IScalarTy.numBits] at hkb
        have : kw.val * 10^18 < 2^127 * 10^18 := mul_lt_mul_of_pos_right (by omega) (by norm_num)
        linarith [hw'.1]
      step with rp_mul_floor_spec env v1 e.feed.price_wad hv10 hp.le hv1p as ⟨v2, hv2⟩
      have hv2' := (isFloor_pos (by norm_num : (0:Int) < 10^18)).mp hv2
      simp only [wad_ge]
      have hfw : f.val ≤ w0.val := by nlinarith
      by_cases hcv : kw.val ≤ v2.val
      on_goal 1 =>
        simp only [hcv, decide_true, ↓reduceIte, bind_ok]
        obtain ⟨ha1, hale, hcov⟩ := ceil_covered_arith hS hp hkw hk0 hf0 hv1 hv2' hcv hamp
        rw [← core.cmp.impls.OrdI128.min_val] at ha1 hale hcov
        generalize core.cmp.impls.OrdI128.min f e.amount = a at ha1 hale hcov ⊢
      on_goal 2 =>
        simp only [hcv, decide_false, Bool.false_eq_true, ↓reduceIte, bind_ok]
        step as ⟨f1, hf1⟩
        obtain ⟨ha1, hale, hcov⟩ := ceil_uncovered_arith hS hp hkw hw' hf' hf0 hf1 hamp
        rw [← core.cmp.impls.OrdI128.min_val] at ha1 hale hcov
        generalize core.cmp.impls.OrdI128.min f1 e.amount = a at ha1 hale hcov ⊢
      all_goals
        have haS' : a.val * 10^(18 - e.feed.asset_decimals.val) ≤
            e.amount.val * 10^(18 - e.feed.asset_decimals.val) :=
          mul_le_mul_of_nonneg_right hale hS.le
        step with from_token_spec env a e.feed.asset_decimals hdec
          (by constructor <;> nlinarith) as ⟨w, hw⟩
        have hw0 : 0 ≤ w.val := by rw [hw]; positivity
        have htokle : huD (w.val * e.feed.price_wad.val) (10^18) ≤
            tokUsd e.amount.val e.feed.asset_decimals.val e.feed.price_wad.val := by
          rw [hw]; unfold tokUsd; apply rp_huD_mono (by norm_num)
          exact mul_le_mul_of_nonneg_right haS' hp.le
        step with rp_mul_spec env w e.feed.price_wad hw0 hp.le (by linarith) as ⟨w1, hw1⟩
        have hw1ge : e.usd_wad.val - rem.val ≤ w1.val := by
          rcases hcov with heq | hcv
          · have := hback rfl
            rw [hw1, hw, heq]; unfold tokUsd at this; linarith
          · rw [hw1, hw]; exact rp_huD_ge_of (by norm_num) hcv
        have hw10 : 0 ≤ w1.val := by
          rw [hw1]; apply rp_huD_nonneg (by norm_num); positivity
        have ha00 : ¬ (a = 0#i128) := fun h => by rw [h] at ha1; simp at ha1
        rcases lt_or_ge e.usd_wad.val w1.val with hlt2 | hlt2
        · simp only [wad_min, hlt2, ↓reduceIte, bind_ok,
            aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone,
            aeneas_soroban_shim.Vec.push_back, math.fp.Wad.raw]
          step as ⟨r, hr⟩
          simp only [ha00, ↓reduceIte, hset, bind_ok]
          have hi := inv_partial hinv0 he
            (LegFate.trim a e.usd_wad r ha1 hale husd0 (le_refl _) hr)
            (by simp [usdSum]) (fun h => by cases h) (fun _ => by simp [usdSum]; linarith)
          simp only [Option.toList_some, List.singleton_append] at hi
          rw [← rp_zero_val] at hi
          exact ih _ _ _ idx1 hi1 hi
        · have hnlt : ¬ (e.usd_wad.val < w1.val) := not_lt.mpr hlt2
          simp only [wad_min, hnlt, ↓reduceIte, bind_ok,
            aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone,
            aeneas_soroban_shim.Vec.push_back, math.fp.Wad.raw]
          step as ⟨r, hr⟩
          simp only [ha00, ↓reduceIte, hset, bind_ok]
          have hi := inv_partial hinv0 he
            (LegFate.trim a w1 r ha1 hale hw10 hlt2 hr)
            (by simp [usdSum]; linarith) (fun h => by cases h) (fun _ => by simp [usdSum]; linarith)
          simp only [Option.toList_some, List.singleton_append] at hi
          rw [← rp_zero_val] at hi
          exact ih _ _ _ idx1 hi1 hi

/-! ## `process_excess_payment` -/

/-- **`process_excess_payment`: shape, exact refunds, value bounds, no abort.**
For input legs `L` (fewer than `2^32`, the host `Vec` limit), initial refunds
`r0`, excess `E ≥ 0` and either mode, if every leg satisfies `ExcessLegOK`,
the call returns (never aborts) and its output `(legs, refunds)` satisfies
`ExcessPost`:

* `ExcessShape`: a cut `k`; legs before it untouched, every leg from it on
  kept, trimmed (amount lowered to `a ∈ (0, amount]`, value to
  `u ∈ [0, usd]`, refund exactly `amount − a`) or removed (refund exactly its
  amount); output order preserved; refunds appended after `r0`;
* the recorded value never grows: `Σ usd(out) ≤ Σ usd(L)`;
* without `keep_within_quote` it removes at most the excess:
  `Σ usd(out) ≥ Σ usd(L) − E`;
* with `keep_within_quote` it removes at least the excess
  (`Σ usd(out) ≤ Σ usd(L) − E`), unless it visited every leg and every leg
  left has a non-positive amount or a zero value. -/
theorem process_excess_payment_spec (env : Env) (L : List types.RepayEntry)
    (r0 : List types.PaymentTuple) (E : math.fp.Wad) (keep : Bool)
    (hlen : L.length < 2^32) (hE : 0 ≤ E.val) (hok : ∀ e ∈ L, ExcessLegOK keep e) :
    positions.liquidation.math.process_excess_payment env L r0 E keep
      ⦃ out => ExcessPost L r0 E.val keep out ⦄ := by
  unfold positions.liquidation.math.process_excess_payment
  simp only [aeneas_soroban_shim.Vec.len, UScalar.tryMk]
  have hk := UScalar.tryMkOpt_eq .U32 L.length
  cases hko : UScalar.tryMkOpt .U32 L.length with
  | none =>
    exfalso; rw [hko] at hk; apply hk
    simp [UScalar.inBounds, UScalarTy.numBits]; omega
  | some n =>
    rw [hko] at hk; obtain ⟨hn, -⟩ := hk
    simp only [Result.ofOption, bind_ok]
    apply excess_loop_spec env L r0 E.val keep hok L.length L r0 E n hn
    refine ⟨shape_init L r0, hE, le_refl _, fun _ => by linarith, fun _ => by linarith, ?_⟩
    intro _ x hx; simp at hx

/-! ## Consequences -/

/-- Sum of the token amounts of a list of repayment legs. -/
def amtSum (l : List types.RepayEntry) : Int := (l.map (fun e => e.amount.val)).sum

/-- Sum of the token amounts of a list of refunds. -/
def refSum (l : List types.PaymentTuple) : Int := (l.map (fun r => r.amount.val)).sum

/-- Per visited leg: kept amount plus refunded amount equals the leg amount. -/
theorem legFate_amount {e : types.RepayEntry} {o : Option types.RepayEntry}
    {rs : List types.PaymentTuple} (h : LegFate e o rs) :
    amtSum o.toList + refSum rs = e.amount.val := by
  cases h with
  | keep => simp [amtSum, refSum]
  | trim a u r _ _ _ _ hr => simp [amtSum, refSum, hr]
  | drop r _ hr => simp [amtSum, refSum, hr]

/-- Summed over the visited legs: kept plus refunded equals the visited amounts. -/
theorem fates_amount (F : Fates) (h : ∀ f ∈ F, LegFate f.1 f.2.1 f.2.2) :
    amtSum (F.filterMap (·.2.1)) + refSum (F.reverse.flatMap (·.2.2)) = amtSum (F.map (·.1)) := by
  induction F with
  | nil => simp [amtSum, refSum]
  | cons f F ih =>
    have h1 := legFate_amount (h f (by simp))
    have h2 := ih (fun x hx => h x (by simp [hx]))
    obtain ⟨e, o, rs⟩ := f
    simp only at h1
    have hfm : (((e, o, rs) :: F).filterMap (·.2.1)) = o.toList ++ F.filterMap (·.2.1) := by
      cases o <;> simp
    rw [hfm]
    simp only [List.reverse_cons, List.flatMap_append, List.map_cons]
    simp [amtSum, refSum] at h1 h2 ⊢
    linarith

/-- **Tokens are conserved exactly.** The refunds appended by
`process_excess_payment` and the kept amounts add up to the input amounts:
`Σ amount(out) + Σ amount(new refunds) = Σ amount(L)` (per leg: kept +
refunded = offered, see `LegFate`). -/
theorem excess_tokens_conserved {L : List types.RepayEntry} {r0 : List types.PaymentTuple}
    {k : Nat} {out : List types.RepayEntry} {rf : List types.PaymentTuple}
    (h : ExcessShape L r0 k out rf) :
    ∃ new, rf = r0 ++ new ∧ amtSum out + refSum new = amtSum L := by
  obtain ⟨hk, F, hF, hfate, hout, hrf⟩ := h
  refine ⟨_, hrf, ?_⟩
  have := fates_amount F hfate
  rw [hF] at this
  have hL : amtSum L = amtSum (L.take k) + amtSum (L.drop k) := by
    conv => lhs; rw [← List.take_append_drop k L]
    simp [amtSum]
  rw [hout, hL]
  simp only [amtSum, List.map_append, List.sum_append] at this ⊢
  linarith

/-- **The output is no longer than the input.** -/
theorem excess_length_le {L : List types.RepayEntry} {r0 : List types.PaymentTuple}
    {k : Nat} {out : List types.RepayEntry} {rf : List types.PaymentTuple}
    (h : ExcessShape L r0 k out rf) : out.length ≤ L.length := by
  obtain ⟨hk, F, hF, -, hout, -⟩ := h
  have h1 : (F.filterMap (·.2.1)).length ≤ F.length := List.length_filterMap_le _ _
  have h2 : F.length = (L.drop k).length := by rw [← hF, List.length_map]
  rw [hout, List.length_append]
  simp only [List.length_drop, List.length_take] at h2 ⊢
  omega

/-- **Removed legs are exactly the fully refunded ones.** A visited leg is
removed iff its refund is its whole amount; a trimmed leg's refund is
strictly below its amount; a kept leg has no refund. -/
theorem legFate_removed_iff {e : types.RepayEntry} {o : Option types.RepayEntry}
    {rs : List types.PaymentTuple} (h : LegFate e o rs) :
    (o = none ↔ ∃ r, rs = [r] ∧ r.amount.val = e.amount.val) ∧
    (∀ x, o = some x → ∀ r ∈ rs, r.amount.val < e.amount.val) := by
  cases h with
  | keep => simp
  | trim a u r ha _ _ _ hr =>
    refine ⟨by simp; omega, ?_⟩
    intro x _ r' hr'; simp at hr'; subst hr'; simp; omega
  | drop r _ hr => simp [hr]

/-- **No leg goes negative.** If every input amount is non-negative, every
output amount and every appended refund is non-negative, and every output
leg's value is non-negative when the input values are. -/
theorem excess_nonneg {L : List types.RepayEntry} {r0 : List types.PaymentTuple}
    {k : Nat} {out : List types.RepayEntry} {rf : List types.PaymentTuple}
    (h : ExcessShape L r0 k out rf) (hL : ∀ e ∈ L, 0 ≤ e.amount.val ∧ 0 ≤ e.usd_wad.val) :
    (∀ x ∈ out, 0 ≤ x.amount.val ∧ 0 ≤ x.usd_wad.val) ∧
    ∃ new, rf = r0 ++ new ∧ ∀ r ∈ new, 0 ≤ r.amount.val := by
  obtain ⟨hk, F, hF, hfate, hout, hrf⟩ := h
  have hmemF : ∀ f ∈ F, f.1 ∈ L := by
    intro f hf
    have : f.1 ∈ L.drop k := by rw [← hF]; exact List.mem_map_of_mem hf
    exact List.mem_of_mem_drop this
  refine ⟨?_, _, hrf, ?_⟩
  · intro x hx
    rw [hout] at hx
    rcases List.mem_append.mp hx with hx | hx
    · exact hL x (List.mem_of_mem_take hx)
    · obtain ⟨f, hf, hfx⟩ := List.mem_filterMap.mp hx
      have he := hL f.1 (hmemF f hf)
      have := hfate f hf
      obtain ⟨e, o, rs⟩ := f
      simp only at hfx this he
      cases this with
      | keep => simp at hfx; subst hfx; exact he
      | trim a u r ha _ hu _ _ => simp at hfx; subst hfx; exact ⟨ha.le, hu⟩
      | drop => simp at hfx
  · intro r hr
    obtain ⟨f, hf, hrs⟩ := List.mem_flatMap.mp hr
    have he := hL f.1 (hmemF f (List.mem_reverse.mp hf))
    have := hfate f (List.mem_reverse.mp hf)
    obtain ⟨e, o, rs⟩ := f
    simp only at hrs this he
    cases this with
    | keep => simp at hrs
    | trim a u r' _ hae _ _ hr' => simp at hrs; subst hrs; simp; omega
    | drop r' _ hr' => simp at hrs; subst hrs; simp; omega

/-- Each output leg comes from an input leg: either unchanged, or with the
same asset, feed and index snapshot and a smaller positive amount and a
smaller non-negative value. -/
def LegLe (e x : types.RepayEntry) : Prop :=
  x = e ∨ ∃ a u : Std.I128, x = { e with amount := a, usd_wad := u } ∧ 0 < a.val ∧
    a.val ≤ e.amount.val ∧ 0 ≤ u.val ∧ u.val ≤ e.usd_wad.val

/-- A `LegLe` pair has the same hub asset. -/
theorem LegLe.hub_asset {e x : types.RepayEntry} (h : LegLe e x) : x.hub_asset = e.hub_asset := by
  rcases h with rfl | ⟨a, u, rfl, -⟩ <;> rfl

/-- A `LegLe` output leg has at most the input amount. -/
theorem LegLe.amount_le {e x : types.RepayEntry} (h : LegLe e x) : x.amount.val ≤ e.amount.val := by
  rcases h with rfl | ⟨a, u, rfl, _, h, -⟩
  · exact le_refl _
  · exact h

/-- A `LegLe` output leg has at most the input value. -/
theorem LegLe.usd_le {e x : types.RepayEntry} (h : LegLe e x) : x.usd_wad.val ≤ e.usd_wad.val := by
  rcases h with rfl | ⟨a, u, rfl, _, _, _, h⟩
  · exact le_refl _
  · exact h

/-- **The output is a pointwise-smaller sublist of the input.** There is a
sublist `S` of `L` (the legs not removed, in order) with `LegLe` pairing each
of its legs with the corresponding output leg. -/
theorem excess_sublist {L : List types.RepayEntry} {r0 : List types.PaymentTuple}
    {k : Nat} {out : List types.RepayEntry} {rf : List types.PaymentTuple}
    (h : ExcessShape L r0 k out rf) :
    ∃ S, S.Sublist L ∧ List.Forall₂ LegLe S out := by
  obtain ⟨hk, F, hF, hfate, hout, -⟩ := h
  have key : ∀ F : Fates, (∀ f ∈ F, LegFate f.1 f.2.1 f.2.2) →
      ∃ S, S.Sublist (F.map (·.1)) ∧ List.Forall₂ LegLe S (F.filterMap (·.2.1)) := by
    intro F
    induction F with
    | nil => intro _; exact ⟨[], List.Sublist.slnil, List.Forall₂.nil⟩
    | cons f F ih =>
      intro hf
      obtain ⟨S, hS, h2⟩ := ih (fun x hx => hf x (by simp [hx]))
      have := hf f (by simp)
      obtain ⟨e, o, rs⟩ := f
      simp only at this
      cases this with
      | keep =>
        exact ⟨e :: S, hS.cons_cons e,
          by simpa using (List.Forall₂.cons (R := LegLe) (Or.inl rfl) h2 :
            List.Forall₂ LegLe (e :: S) (e :: F.filterMap (·.2.1)))⟩
      | trim a u r ha hae hu hue _ =>
        exact ⟨e :: S, hS.cons_cons e,
          by simpa using (List.Forall₂.cons (R := LegLe) (Or.inr ⟨a, u, rfl, ha, hae, hu, hue⟩) h2 :
            List.Forall₂ LegLe (e :: S) ({ e with amount := a, usd_wad := u } :: F.filterMap (·.2.1)))⟩
      | drop =>
        exact ⟨S, hS.cons e, by simpa using h2⟩
  obtain ⟨S, hS, h2⟩ := key F hfate
  refine ⟨L.take k ++ S, ?_, ?_⟩
  · conv => rhs; rw [← List.take_append_drop k L]
    rw [hF] at hS
    exact (List.Sublist.refl _).append hS
  · rw [hout]
    have h1 : List.Forall₂ LegLe (L.take k) (L.take k) :=
      List.forall₂_same.mpr (fun x _ => Or.inl rfl)
    exact List.rel_append h1 h2

end ControllerAeneas.Proofs
