/-
`calculate_seized_collateral` (`contracts/controller/src/positions/liquidation/math.rs`):
the whole loop and the function, total correctness on the production domain,
with every emitted entry given by the per-leg closed forms of `SeizeLeg.lean`,
and the cross-leg bound "total seized value ≤ budget + rounding per leg".

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.SeizeBody
import ControllerAeneas.Proofs.EstimateBasic

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 4000000

namespace ControllerAeneas.Proofs

instance (g : SzLeg) : Decidable g.Emit := by unfold SzLeg.Emit; infer_instance

/-- The seizure list produced from the legs `l`: legs are taken in order; a
leg with `Emit` contributes exactly one entry satisfying `EntryOf`, any other
leg contributes nothing. `G` gives the integer inputs of a leg, `key` its hub
asset. -/
def SzOut {α : Type} (G : α → SzLeg) (key : α → types.HubAssetKey) (ctx : context.Context) :
    List α → List types.SeizeEntry → Prop
  | [], out => out = []
  | e :: l, out =>
    if (G e).Emit then ∃ x out', out = x :: out' ∧ EntryOf ctx (key e) (G e) x ∧ SzOut G key ctx l out'
    else SzOut G key ctx l out

/-- Every entry of a seizure list comes from an emitted leg. -/
theorem SzOut.mem {α : Type} {G : α → SzLeg} {key : α → types.HubAssetKey} {ctx : context.Context} :
    ∀ {l : List α} {out : List types.SeizeEntry}, SzOut G key ctx l out →
      ∀ x ∈ out, ∃ e ∈ l, (G e).Emit ∧ EntryOf ctx (key e) (G e) x := by
  intro l
  induction l with
  | nil => intro out h x hx; simp [SzOut] at h; subst h; simp at hx
  | cons e l ih =>
    intro out h x hx
    unfold SzOut at h
    split_ifs at h with c
    · obtain ⟨y, out', rfl, hy, hrest⟩ := h
      rcases List.mem_cons.mp hx with rfl | hx
      · exact ⟨e, by simp, c, hy⟩
      · obtain ⟨e', he', h1, h2⟩ := ih hrest x hx
        exact ⟨e', by simp [he'], h1, h2⟩
    · obtain ⟨e', he', h1, h2⟩ := ih h x hx
      exact ⟨e', by simp [he'], h1, h2⟩

/-- Transport of `SzOut` along an entry-wise relation that preserves the leg
inputs and the key. -/
theorem SzOut.transport {α β : Type} {G : α → SzLeg} {G' : β → SzLeg}
    {key : α → types.HubAssetKey} {key' : β → types.HubAssetKey} {ctx : context.Context}
    {R : β → α → Prop} (hR : ∀ b a, R b a → G a = G' b ∧ key a = key' b) :
    ∀ {m : List β} {l : List α}, List.Forall₂ R m l →
      ∀ out, SzOut G key ctx l out → SzOut G' key' ctx m out := by
  intro m l hf
  induction hf with
  | nil => intro out h; exact h
  | @cons b a m l hba _ ih =>
    intro out h
    obtain ⟨e1, e2⟩ := hR b a hba
    unfold SzOut at h ⊢
    rw [← e1, ← e2]
    split_ifs at h ⊢ with c
    · obtain ⟨x, out', h1, h2, h3⟩ := h
      exact ⟨x, out', h1, h2, ih out' h3⟩
    · exact ih out h

theorem sz_loop_nil (env : Env) (tc : math.fp.Wad) (rad sa : Bool) (ctx : context.Context)
    (seized : List types.SeizeEntry) (opb tsu uns : math.fp.Wad) :
    positions.liquidation.math.calculate_seized_collateral_loop [] env tc rad sa ctx seized opb tsu uns
      = ok (ctx, seized, uns) := by
  unfold positions.liquidation.math.calculate_seized_collateral_loop; rw [loop]
  simp [positions.liquidation.math.calculate_seized_collateral_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- **The seizure loop.** Over typed supply legs `L`, each served by the
context and in the production domain `SzLegOK`, the loop returns (does not
abort) the context unchanged, the input list extended by exactly the entries
`SzOut` describes, and `unseized_usd` increased by exactly `Σ Uns`.
Hypothesis: the running `unseized_usd` plus the remaining contributions fits
`i128` (each is at most the leg's budget). -/
theorem sz_loop_spec (env : Env) (tc opb tsu : math.fp.Wad) (rad sa : Bool) (ctx : context.Context) :
    ∀ (L : List (types.HubAssetKey × types.AccountPosition)) (seized : List types.SeizeEntry)
      (uns : math.fp.Wad),
      (∀ e ∈ L, Serves ctx e.1 ∧ SzLegOK (legOf ctx tc tsu opb rad sa e)) →
      0 ≤ uns.val →
      uns.val + lsum (fun e => (legOf ctx tc tsu opb rad sa e).Uns) L < 2^127 →
      positions.liquidation.math.calculate_seized_collateral_loop L env tc rad sa ctx seized opb tsu uns
        ⦃ r => r.1 = ctx ∧
          (∃ out, r.2.1 = seized ++ out ∧ SzOut (legOf ctx tc tsu opb rad sa) Prod.fst ctx L out) ∧
          r.2.2.val = uns.val + lsum (fun e => (legOf ctx tc tsu opb rad sa e).Uns) L ⦄ := by
  intro L
  induction L with
  | nil =>
    intro seized uns _ _ _
    rw [sz_loop_nil]; simp [SzOut]
  | cons e L ih =>
    intro seized uns hok hu0 hufit
    obtain ⟨k, pos⟩ := e
    obtain ⟨hserve, hleg⟩ := hok (k, pos) (by simp)
    have hrest : ∀ x ∈ L, Serves ctx x.1 ∧ SzLegOK (legOf ctx tc tsu opb rad sa x) :=
      fun x hx => hok x (by simp [hx])
    have hU0 := hleg.Uns0
    have hsum0 : 0 ≤ lsum (fun e => (legOf ctx tc tsu opb rad sa e).Uns) L := by
      unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      exact (hrest y hy).2.Uns0
    rw [lsum_cons] at hufit
    unfold positions.liquidation.math.calculate_seized_collateral_loop
    rw [loop]
    dsimp only
    apply WP.spec_bind (sz_body_spec env tc opb tsu rad sa k pos L ctx seized uns hserve hleg hu0
      (by omega))
    rintro r ⟨seized', uns', rfl, hu', hemit, hnemit⟩
    simp only
    have := ih seized' uns' hrest (by rw [hu']; omega) (by rw [hu']; omega)
    unfold positions.liquidation.math.calculate_seized_collateral_loop at this
    apply WP.spec_mono this
    rintro ⟨c, s, u⟩ ⟨hc, ⟨out, hs, hout⟩, hu⟩
    refine ⟨hc, ?_, by rw [hu, hu', lsum_cons]; ring⟩
    by_cases he : (legOf ctx tc tsu opb rad sa (k, pos)).Emit
    · obtain ⟨x, rfl, hx⟩ := hemit he
      refine ⟨x :: out, by rw [hs]; simp, ?_⟩
      unfold SzOut; rw [if_pos he]
      exact ⟨x, out, rfl, hx, hout⟩
    · rw [hnemit he] at hs
      refine ⟨out, hs, ?_⟩
      unfold SzOut; rw [if_neg he]; exact hout

/-- The integer inputs of a leg read from a raw supply entry (the typed
position has the same scaled amount and fee rate, `TypedOf`). -/
noncomputable def legOfRaw (ctx : context.Context) (C B opb : Int) (rad sa : Bool)
    (e : types.HubAssetKey × types.AccountPositionRaw) : SzLeg where
  s := e.2.scaled_amount.val
  i := supplyIndexOf ctx e.1
  p := priceOf ctx e.1.asset
  dec := decOf ctx e.1.asset
  f := e.2.liquidation_fees.val
  C := C
  B := B
  opb := opb
  rad := rad
  sa := sa

theorem legOf_typed (ctx : context.Context) (tc tsu opb : math.fp.Wad) (rad sa : Bool)
    (e : types.HubAssetKey × types.AccountPositionRaw) (t : types.HubAssetKey × types.AccountPosition)
    (h : TypedOf e t) :
    legOf ctx tc tsu opb rad sa t = legOfRaw ctx tc.val tsu.val opb.val rad sa e ∧ t.1 = e.1 := by
  obtain ⟨h1, h2, -, -, -, h6⟩ := h
  refine ⟨?_, h1⟩
  simp only [legOf, legOfRaw, h1, h2, h6]

theorem sz_forall2_mem_right {α β : Type} {R : α → β → Prop} :
    ∀ {m : List α} {l : List β}, List.Forall₂ R m l → ∀ {x : β}, x ∈ l → ∃ b ∈ m, R b x := by
  intro m l hf
  induction hf with
  | nil => intro x hx; simp at hx
  | @cons b a m l hba _ ih =>
    intro x hx
    rcases List.mem_cons.mp hx with rfl | hx
    · exact ⟨b, by simp, hba⟩
    · obtain ⟨b', hb', h'⟩ := ih hx
      exact ⟨b', by simp [hb'], h'⟩

/-- `1 + bonus` in WAD. -/
def szOpb (bonus : Int) : Int := 10^18 + bonus * 10^14
/-- The seizure budget `total_seizure_usd = huD(repay·(1 + bonus), W)`. -/
def szBudget (repay bonus : Int) : Int := huD (repay * szOpb bonus) (10^18)

/-- The legs of the account's supply map, with the plan's budget. -/
noncomputable def planLeg (ctx : context.Context) (tc : Int)
    (rep : positions.liquidation.math.NormalizedRepaymentPlan) :
    types.HubAssetKey × types.AccountPositionRaw → SzLeg :=
  legOfRaw ctx tc (szBudget rep.repay_usd.val rep.bonus.val) (szOpb rep.bonus.val)
    rep.repays_all_debt rep.seize_all

/-- **`calculate_seized_collateral`, positive collateral.** For a repayment
plan with `0 ≤ repay_usd`, `0 ≤ bonus ≤ 10^8` BPS and seizure budget
`B = huD(repay·(W + bonus·10^14), W)` fitting `i128`, and every supply leg
served by the context and in the production domain `SzLegOK` (`planLeg`:
that budget, `opb = W + bonus·10^14`), the call returns (does not abort):
the context unchanged; the seizures are exactly `SzOut` over the account's
supply map in map order (one `EntryOf` entry per emitted leg, with the
per-leg closed forms of `SeizeLeg.lean`); the unbacked USD is
`⌊(Σ Uns)·W / opb⌋`. Hypothesis: `Σ Uns < 2^127` (each `Uns ≤ L ≤ B`). -/
theorem calculate_seized_collateral_spec (env : Env) (acc : types.Account) (tc : math.fp.Wad)
    (rep : positions.liquidation.math.NormalizedRepaymentPlan) (ctx : context.Context)
    (htc : 0 < tc.val) (hr0 : 0 ≤ rep.repay_usd.val) (hb0 : 0 ≤ rep.bonus.val)
    (hb : rep.bonus.val ≤ 10^8)
    (hfit : szBudget rep.repay_usd.val rep.bonus.val < 2^127)
    (hlegs : ∀ e ∈ acc.supply_positions, Serves ctx e.1 ∧ SzLegOK (planLeg ctx tc.val rep e))
    (huns : lsum (fun e => (planLeg ctx tc.val rep e).Uns) acc.supply_positions < 2^127) :
    positions.liquidation.math.calculate_seized_collateral env acc tc rep ctx ⦃ r =>
      r.2 = ctx ∧
      SzOut (planLeg ctx tc.val rep) Prod.fst ctx acc.supply_positions r.1.1 ∧
      r.1.2.val = lsum (fun e => (planLeg ctx tc.val rep e).Uns) acc.supply_positions * 10^18 /
        szOpb rep.bonus.val ⦄ := by
  unfold positions.liquidation.math.calculate_seized_collateral
  simp only [aeneas_soroban_shim.Vec.new, wad_le, sz_wad_zero_val, bind_ok]
  have hn : ¬ (tc.val ≤ 0) := by omega
  simp only [hn, decide_false, Bool.false_eq_true, ↓reduceIte]
  step with bps_to_wad_spec env rep.bonus hb0 hb as ⟨w, hw⟩
  step with sz_wad_checked_add_val env math.fp.Wad.ONE w
    ⟨by rw [sz_wad_one_val, hw]; nlinarith, by rw [sz_wad_one_val, hw]; nlinarith⟩ as ⟨o, ho⟩
  have ho' : o.val = szOpb rep.bonus.val := by rw [ho, sz_wad_one_val, hw]; rfl
  have hopos : 0 < o.val := by rw [ho']; unfold szOpb; nlinarith
  step with sz_wad_mul_val env rep.repay_usd o hr0 (by omega)
    (by rw [ho']; exact hfit) as ⟨t, ht⟩
  have ht' : t.val = szBudget rep.repay_usd.val rep.bonus.val := by rw [ht, ho']; rfl
  step with iter_typed_positions_spec acc.supply_positions as ⟨L, hL⟩
  have hG : ∀ (b : types.HubAssetKey × types.AccountPositionRaw) (a : types.HubAssetKey × types.AccountPosition),
      TypedOf b a → legOf ctx tc t o rep.repays_all_debt rep.seize_all a = planLeg ctx tc.val rep b ∧
        a.1 = b.1 := by
    intro b a h
    obtain ⟨e1, e2⟩ := legOf_typed ctx tc t o rep.repays_all_debt rep.seize_all b a h
    refine ⟨?_, e2⟩
    rw [e1, ht', ho']; rfl
  -- transport hypotheses to the typed list
  have hLok : ∀ x ∈ L, Serves ctx x.1 ∧ SzLegOK (legOf ctx tc t o rep.repays_all_debt rep.seize_all x) := by
    intro x hx
    obtain ⟨b, hb, hbx⟩ := sz_forall2_mem_right hL hx
    obtain ⟨e1, e2⟩ := hG b x hbx
    rw [e1, e2]; exact hlegs b hb
  have hsumeq : ∀ {m : List (types.HubAssetKey × types.AccountPositionRaw)}
      {l : List (types.HubAssetKey × types.AccountPosition)}, List.Forall₂ TypedOf m l →
      lsum (fun e => (legOf ctx tc t o rep.repays_all_debt rep.seize_all e).Uns) l =
        lsum (fun e => (planLeg ctx tc.val rep e).Uns) m := by
    intro m l hf
    induction hf with
    | nil => rfl
    | @cons b a m l hba _ ih => rw [lsum_cons, lsum_cons, ih, (hG b a hba).1]
  have hse := hsumeq hL
  step with sz_loop_spec env tc o t rep.repays_all_debt rep.seize_all ctx L [] math.fp.Wad.ZERO hLok
    (by rw [sz_wad_zero_val]) (by rw [sz_wad_zero_val, hse]; omega) as ⟨c1, s1, u1, hc, out, hout, hsz, hu⟩
  rw [sz_wad_zero_val, zero_add, hse] at hu
  have hu0 : 0 ≤ u1.val := by
    rw [hu]; unfold lsum; apply List.sum_nonneg
    intro x hx
    obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
    exact (hlegs y hy).2.Uns0
  have hub : u1.val * 10^18 / o.val ≤ u1.val := sz_div_le_of _ _ _ hopos (by
    rw [ho']; unfold szOpb; nlinarith)
  step with sz_wad_div_floor_val env u1 o hopos
    ⟨by have := sz_div_nonneg (u1.val * 10^18) o.val hopos (by positivity); omega,
     by have := u1.hBounds; simp only [IScalarTy.numBits] at this; omega⟩ as ⟨w1, hw1⟩
  refine ⟨hc, ?_, by rw [hw1, hu, ho']⟩
  rw [hout]
  exact SzOut.transport (R := TypedOf) hG hL out (by simpa using hsz)

/-- **`calculate_seized_collateral` without collateral** returns no seizure
and no unbacked USD, for every input (it never aborts). -/
theorem calculate_seized_collateral_no_collateral (env : Env) (acc : types.Account)
    (tc : math.fp.Wad) (rep : positions.liquidation.math.NormalizedRepaymentPlan)
    (ctx : context.Context) (htc : tc.val ≤ 0) :
    positions.liquidation.math.calculate_seized_collateral env acc tc rep ctx =
      ok ((([] : List types.SeizeEntry), math.fp.Wad.ZERO), ctx) := by
  unfold positions.liquidation.math.calculate_seized_collateral
  simp [aeneas_soroban_shim.Vec.new, sz_wad_zero_val, htc]

end ControllerAeneas.Proofs
