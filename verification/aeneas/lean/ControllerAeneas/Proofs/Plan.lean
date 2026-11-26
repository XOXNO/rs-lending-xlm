/-
`build_liquidation_plan` (`contracts/controller/src/positions/liquidation/plan.rs`):
a liquidation plan never seizes more shares than the account holds
(INV-LIQ-02).

The theorems here are about a call that RETURNED (partial correctness):
`Rets m P` says "if `m` returns `r`, then `P r`". They need no value domain.
`calculate_seized_collateral_entries` (`SeizeBudget.lean`) proves the same
per-entry bound for `calculate_seized_collateral` under `SzLegOK` for every
leg, a served context and a bounded plan. Those hypotheses make the call
return; the bound itself does not need them:

* `Ray::mul` (`actual_ray`) returns only for a non-negative position and
  index, with `actual_ray = huD(s·i, RAY)`;
* `capped_ray ≤ actual_ray` on both branches (`math.rs`, `capped_ray`);
* a full close (`capped_ray == actual_ray`) seizes exactly the held shares;
* a partial close seizes `⌊capped_ray·RAY/i⌋` with `capped_ray < actual_ray`,
  which is at most the held shares;
* an entry is pushed only when `seized_scaled > 0`.

`build_liquidation_plan` passes its account to that call and returns the
call's list unchanged (`build_liquidation_plan_seized_computed`).

A plain (non-`module`) file.
-/
import ControllerAeneas.Proofs.SeizeBudget
import ControllerAeneas.Proofs.EstimateAbort

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000

namespace ControllerAeneas.Proofs

/-! ## Partial correctness -/

/-- `Rets m P`: if `m` returns `r` (does not abort), then `P r`. -/
def Rets {α : Type} (m : Result α) (P : α → Prop) : Prop := ∀ r, m = ok r → P r

/-- A bind that returned: its first computation returned `v` and the
continuation on `v` returned the same result. -/
theorem pl_bind_ok {α β : Type} {m : Result α} {k : α → Result β} {r : β}
    (h : Std.bind m k = ok r) : ∃ v, m = ok v ∧ k v = ok r := by
  obtain ⟨v, hv⟩ := est_bind_eq_ok_inv h
  refine ⟨v, hv, ?_⟩
  rw [hv, bind_ok] at h
  exact h

/-- A total-correctness triple gives its postcondition for the value returned. -/
theorem pl_spec_ok {α : Type} {m : Result α} {P : α → Prop} (h : m ⦃ v => P v ⦄) {r : α}
    (hr : m = ok r) : P r := by
  obtain ⟨v, hv, hP⟩ := spec_imp_eq_ok h
  rw [hr] at hv
  have e : r = v := by simpa using hv
  rw [e]; exact hP

/-- A returned value satisfies `P`. -/
theorem Rets.ok {α : Type} {a : α} {P : α → Prop} (h : P a) : Rets (ok a) P := by
  intro r hr
  have e : a = r := by simpa using hr
  rw [← e]; exact h

/-- Sequencing: `Q` holds for what `m` returns, and the continuation
satisfies `P` on every value with `Q`. -/
theorem Rets.bind {α β : Type} {m : Result α} {k : α → Result β} {P : β → Prop} (Q : α → Prop)
    (hm : Rets m Q) (hk : ∀ a, Q a → Rets (k a) P) : Rets (Std.bind m k) P := by
  intro r hr
  obtain ⟨v, hv, hkv⟩ := pl_bind_ok hr
  exact hk v (hm v hv) r hkv

/-- Both branches of an `if`. -/
theorem Rets.ite {α : Type} {c : Prop} [Decidable c] {a b : Result α} {P : α → Prop}
    (ha : c → Rets a P) (hb : ¬ c → Rets b P) : Rets (if c then a else b) P := by
  split
  · exact ha ‹_›
  · exact hb ‹_›

/-- Sequencing when nothing is needed about what `m` returns. -/
theorem Rets.bind_any {α β : Type} {m : Result α} {k : α → Result β} {P : β → Prop}
    (hk : ∀ a, Rets (k a) P) : Rets (Std.bind m k) P :=
  Rets.bind (fun _ => True) (fun _ _ => trivial) (fun a _ => hk a)

/-- Proves `Rets m P` when `tac` closes every returning branch of `m`: skips
each bind (its value is arbitrary) and splits each `if`. -/
macro "rets_all " t:tacticSeq : tactic =>
  `(tactic| repeat' (first | $t | refine Rets.bind_any (fun _ => ?_) | refine Rets.ite (fun _ => ?_) (fun _ => ?_) | split))

/-- `Ray::mul` that returned: both operands are non-negative and the result is
`huD(a·b, RAY)` (`mul_div_half_up` aborts on a negative operand). -/
theorem pl_ray_mul_ok (env : Env) (a b A : math.fp.Ray) (h : math.fp.Ray.mul a env b = ok A) :
    0 ≤ a.val ∧ 0 ≤ b.val ∧ A.val = huD (a.val * b.val) (10^27) := by
  have h1 : math.fp_core.mul_div_half_up env a b constants.production.shared.RAY ⦃ _ => True ⦄ := by
    unfold math.fp.Ray.mul at h
    obtain ⟨v, hv, _⟩ := pl_bind_ok h
    rw [hv]; exact (WP.spec_ok _).mpr trivial
  obtain ⟨ha, hb, _, q, hq, hlt⟩ := (mul_div_half_up_returns_iff env a b _).mp h1
  rw [RAY_val] at hq
  have hfit : huD (a.val * b.val) (10^27) < 2^127 := by
    rw [← isHalfUp_eq (by norm_num) hq]; exact hlt
  exact ⟨ha, hb, pl_spec_ok (sz_ray_mul_val env a b ha hb hfit) h⟩

/-! ## One leg -/

/-- A seizure entry `x` against the supply leg `e` it was computed from: same
hub asset, and a positive share amount at most the shares the leg holds. -/
def Within {α : Type} (key : α → types.HubAssetKey) (held : α → Int) (e : α)
    (x : types.SeizeEntry) : Prop :=
  x.hub_asset = key e ∧ 0 < x.scaled_amount.val ∧ x.scaled_amount.val ≤ held e

/-- **One loop iteration of `calculate_seized_collateral`, when it returns.**
For EVERY input (no domain hypothesis): the iteration continues with the rest
of the legs, and the seizure list is unchanged or extended by exactly one
entry for this leg's hub asset whose `scaled_amount` is positive and at most
the leg's held `scaled_amount`. -/
theorem pl_body_rets (env : Env) (tc opb tsu : math.fp.Wad) (rad sa : Bool)
    (k : types.HubAssetKey) (pos : types.AccountPosition)
    (rest : List (types.HubAssetKey × types.AccountPosition)) (ctx : context.Context)
    (seized : List types.SeizeEntry) (uns : math.fp.Wad) :
    Rets (positions.liquidation.math.calculate_seized_collateral_loop.body env tc rad sa opb tsu
        ((k, pos) :: rest) ctx seized uns)
      (fun r => ∃ ctx' seized' uns', r = ControlFlow.cont (rest, ctx', seized', uns') ∧
        (seized' = seized ∨ ∃ x, seized' = seized ++ [x] ∧
          Within Prod.fst (fun e => e.2.scaled_amount.val) (k, pos) x)) := by
  unfold positions.liquidation.math.calculate_seized_collateral_loop.body
  simp only [sz_next_cons, bind_ok]
  refine Rets.bind_any fun x => ?_
  obtain ⟨feed, c1⟩ := x
  refine Rets.bind_any fun x => ?_
  obtain ⟨mi, c2⟩ := x
  refine Rets.bind (fun A => 0 ≤ pos.scaled_amount.val ∧ 0 ≤ mi.supply_index.val ∧
    A.val = huD (pos.scaled_amount.val * mi.supply_index.val) (10^27))
    (fun A hA => pl_ray_mul_ok _ _ _ _ hA) ?_
  rintro A ⟨hs0, hi0, hAv⟩
  refine Rets.bind_any fun v => ?_
  refine Rets.bind_any fun sh => ?_
  refine Rets.bind_any fun L => ?_
  refine Rets.bind_any fun q => ?_
  refine Rets.bind_any fun i => ?_
  -- the whole-unit block returns the same iterator
  refine Rets.bind (fun t => t.2.2.1 = rest) ?_ ?_
  · rets_all (exact Rets.ok rfl)
  rintro ⟨c3, u1, it2, a1, s1⟩ ht
  simp only at ht
  subst ht
  simp only [sz_ray_le, sz_ray_zero_val, sz_ray_eq, bind_ok, decide_eq_true_eq]
  refine Rets.ite (fun _ => ?_) (fun _ => ?_)
  · refine Rets.ok ?_; exact ⟨_, _, _, rfl, Or.inl rfl⟩
  refine Rets.bind (fun cap : math.fp.Ray => cap.val ≤ A.val) ?_ ?_
  · refine Rets.ite (fun _ => ?_) (fun _ => ?_)
    · exact Rets.ok le_rfl
    · rw [sz_ray_min]; refine Rets.ok ?_; split <;> omega
  intro cap hcap
  refine Rets.ite (fun _ => ?_) (fun hcap0 => ?_)
  · refine Rets.ok ?_; exact ⟨_, _, _, rfl, Or.inl rfl⟩
  refine Rets.bind_any fun w => ?_
  refine Rets.bind_any fun i3 => ?_
  refine Rets.bind_any fun i4 => ?_
  refine Rets.bind_any fun base => ?_
  refine Rets.bind_any fun b4 => ?_
  -- `seized_scaled ≤ position.scaled_amount`
  refine Rets.bind (fun ss : math.fp.Ray => ss.val ≤ pos.scaled_amount.val) ?_ ?_
  · refine Rets.ite (fun _ => Rets.ok le_rfl) (fun hne => ?_)
    intro x hx
    have hA2 := (sz_huD_bounds (pos.scaled_amount.val * mi.supply_index.val) (10^27) (by norm_num)).1
    rw [← hAv] at hA2
    have hi : 0 < mi.supply_index.val := by
      rcases hi0.lt_or_eq with h | h
      · exact h
      · exfalso
        have h0 : pos.scaled_amount.val * mi.supply_index.val = 0 := by rw [← h]; ring
        rw [h0] at hA2; omega
    have hle : cap.val * 10^27 ≤ pos.scaled_amount.val * mi.supply_index.val := by omega
    have hF0 : 0 ≤ cap.val * 10^27 / mi.supply_index.val :=
      sz_div_nonneg _ _ hi (mul_nonneg (by omega) (by norm_num))
    have hFs : cap.val * 10^27 / mi.supply_index.val ≤ pos.scaled_amount.val :=
      sz_div_le_of _ _ _ hi hle
    have hb := pos.scaled_amount.hBounds
    simp only [IScalarTy.numBits] at hb
    have hx' := pl_spec_ok (sz_ray_div_floor_val env cap mi.supply_index hi ⟨by omega, by omega⟩) hx
    rw [hx']; exact hFs
  intro ss hss
  refine Rets.bind_any fun r => ?_
  refine Rets.bind_any fun bsc => ?_
  refine Rets.ite (fun _ => ?_) (fun hss0 => ?_)
  · refine Rets.ok ?_; exact ⟨_, _, _, rfl, Or.inl rfl⟩
  simp only [math.fp.Ray.raw, aeneas_soroban_shim.Vec.push_back, bind_ok]
  have hpos : 0 < ss.val := by omega
  rets_all first
    | (refine Rets.ok ?_; exact ⟨_, _, _, rfl, Or.inl rfl⟩)
    | (refine Rets.ok ?_; exact ⟨_, _, _, rfl, Or.inr ⟨_, rfl, rfl, hpos, hss⟩⟩)

/-! ## The loop and the function -/

/-- The seizure list built from the legs `l`, in order: each leg contributes
no entry, or exactly one entry `Within` it. -/
def SeizedFrom {α : Type} (key : α → types.HubAssetKey) (held : α → Int) :
    List α → List types.SeizeEntry → Prop
  | [], out => out = []
  | e :: l, out => SeizedFrom key held l out ∨
      ∃ x out', out = x :: out' ∧ Within key held e x ∧ SeizedFrom key held l out'

/-- No leg may contribute. -/
theorem SeizedFrom.nil {α : Type} {key : α → types.HubAssetKey} {held : α → Int} :
    ∀ l : List α, SeizedFrom key held l []
  | [] => rfl
  | _ :: l => Or.inl (SeizedFrom.nil l)

/-- Every entry of the list comes from one of the legs. -/
theorem SeizedFrom.mem {α : Type} {key : α → types.HubAssetKey} {held : α → Int} :
    ∀ {l : List α} {out : List types.SeizeEntry}, SeizedFrom key held l out →
      ∀ x ∈ out, ∃ e ∈ l, Within key held e x := by
  intro l
  induction l with
  | nil => intro out h x hx; rw [show out = [] from h] at hx; simp at hx
  | cons e l ih =>
    intro out h x hx
    rcases h with h | ⟨y, out', rfl, hy, h⟩
    · obtain ⟨e', he', hw⟩ := ih h x hx
      exact ⟨e', by simp [he'], hw⟩
    · rcases List.mem_cons.mp hx with rfl | hx
      · exact ⟨e, by simp, hy⟩
      · obtain ⟨e', he', hw⟩ := ih h x hx
        exact ⟨e', by simp [he'], hw⟩

/-- The hub assets of the list are, in order, a sub-list of the legs' hub
assets: a leg contributes at most one entry. -/
theorem SeizedFrom.keys_sublist {α : Type} {key : α → types.HubAssetKey} {held : α → Int} :
    ∀ {l : List α} {out : List types.SeizeEntry}, SeizedFrom key held l out →
      (out.map (fun x => x.hub_asset)).Sublist (l.map key) := by
  intro l
  induction l with
  | nil => intro out h; rw [show out = [] from h]; exact List.Sublist.slnil
  | cons e l ih =>
    intro out h
    rcases h with h | ⟨y, out', rfl, hy, h⟩
    · exact (ih h).cons _
    · simp only [List.map_cons]
      rw [hy.1]
      exact (ih h).cons_cons _

/-- Transport of `SeizedFrom` along an entry-wise relation that preserves the
hub asset and the held amount. -/
theorem SeizedFrom.transport {α β : Type} {key : α → types.HubAssetKey} {held : α → Int}
    {key' : β → types.HubAssetKey} {held' : β → Int} {R : β → α → Prop}
    (hR : ∀ b a, R b a → key a = key' b ∧ held a = held' b) :
    ∀ {m : List β} {l : List α}, List.Forall₂ R m l →
      ∀ out, SeizedFrom key held l out → SeizedFrom key' held' m out := by
  intro m l hf
  induction hf with
  | nil => intro out h; exact h
  | @cons b a m l hba _ ih =>
    intro out h
    obtain ⟨e1, e2⟩ := hR b a hba
    rcases h with h | ⟨y, out', rfl, ⟨h1, h2, h3⟩, h⟩
    · exact Or.inl (ih out h)
    · exact Or.inr ⟨y, out', rfl, ⟨by rw [h1, e1], h2, by rw [← e2]; exact h3⟩, ih out' h⟩

/-- **The seizure loop, when it returns.** For every input, the returned list
is the input list followed by entries `SeizedFrom` the legs. -/
theorem pl_loop_rets (env : Env) (tc opb tsu : math.fp.Wad) (rad sa : Bool) :
    ∀ (L : List (types.HubAssetKey × types.AccountPosition)) (ctx : context.Context)
      (seized : List types.SeizeEntry) (uns : math.fp.Wad),
      Rets (positions.liquidation.math.calculate_seized_collateral_loop L env tc rad sa ctx seized
          opb tsu uns)
        (fun r => ∃ out, r.2.1 = seized ++ out ∧
          SeizedFrom Prod.fst (fun e => e.2.scaled_amount.val) L out) := by
  intro L
  induction L with
  | nil =>
    intro ctx seized uns
    rw [sz_loop_nil]
    refine Rets.ok ?_
    exact ⟨[], by simp, rfl⟩
  | cons e L ih =>
    intro ctx seized uns
    obtain ⟨k, pos⟩ := e
    unfold positions.liquidation.math.calculate_seized_collateral_loop
    rw [loop]
    dsimp only
    refine Rets.bind _ (pl_body_rets env tc opb tsu rad sa k pos L ctx seized uns) ?_
    rintro r ⟨ctx', seized', uns', rfl, hstep⟩
    simp only
    have := ih ctx' seized' uns'
    unfold positions.liquidation.math.calculate_seized_collateral_loop at this
    intro res hres
    obtain ⟨out, hs, hout⟩ := this res hres
    rcases hstep with rfl | ⟨x, rfl, hx⟩
    · exact ⟨out, hs, Or.inl hout⟩
    · exact ⟨x :: out, by rw [hs]; simp, Or.inr ⟨x, out, rfl, hx, hout⟩⟩

/-- The held scaled amount of a raw supply entry. -/
def heldRaw (e : types.HubAssetKey × types.AccountPositionRaw) : Int := e.2.scaled_amount.val

/-- **`calculate_seized_collateral` never seizes more shares than held.** For
EVERY input on which the call returns: the seizures are `SeizedFrom` the
account's supply map, in map order: at most one entry per supply entry, with
that entry's hub asset and `0 < scaled_amount ≤` its `scaled_amount`.
No hypothesis: no value domain, no served context, no bound on the plan. -/
theorem calculate_seized_collateral_seized_le_held (env : Env) (acc : types.Account)
    (tc : math.fp.Wad) (rep : positions.liquidation.math.NormalizedRepaymentPlan)
    (ctx : context.Context) :
    Rets (positions.liquidation.math.calculate_seized_collateral env acc tc rep ctx)
      (fun r => SeizedFrom Prod.fst heldRaw
        (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)) r.1.1) := by
  unfold positions.liquidation.math.calculate_seized_collateral
  simp only [aeneas_soroban_shim.Vec.new, wad_le, bind_ok, decide_eq_true_eq]
  refine Rets.ite (fun _ => ?_) (fun _ => ?_)
  · refine Rets.ok ?_; exact SeizedFrom.nil _
  refine Rets.bind_any fun w => ?_
  refine Rets.bind_any fun o => ?_
  refine Rets.bind_any fun t => ?_
  refine Rets.bind (fun L => List.Forall₂ TypedOf acc.supply_positions L)
    (fun L hL => pl_spec_ok (iter_typed_positions_spec acc.supply_positions) hL) ?_
  intro L hL
  refine Rets.bind _
    (pl_loop_rets env tc o t rep.repays_all_debt rep.seize_all L ctx [] math.fp.Wad.ZERO) ?_
  rintro ⟨c1, s1, u⟩ ⟨out, hs, hout⟩
  refine Rets.bind_any fun w1 => ?_
  refine Rets.ok ?_
  simp only [List.nil_append] at hs
  show SeizedFrom Prod.fst heldRaw _ s1
  rw [hs]
  exact SeizedFrom.transport (R := TypedOf)
    (fun b a h => ⟨h.1, by simp only [heldRaw]; rw [h.2.1]⟩) hL out hout

/-! ## The plan builder -/

/-- **The plan's seizures are computed by `calculate_seized_collateral`.** When
`build_liquidation_plan` returns a plan, its `seized` list is exactly the
list one call of `calculate_seized_collateral` returned, on the same account,
with the total collateral that `calculate_account_risk_totals` returned for
the account's own position maps. The glue between that call and the return
(`release_unbacked_repayment`, the seizure-flag loop, `validate`) reads the
list and never changes it. -/
theorem build_liquidation_plan_seized_computed (env : Env) (acc : types.Account)
    (pays : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128)) (ctx : context.Context) :
    Rets (positions.liquidation.plan.build_liquidation_plan env acc pays ctx)
      (fun r => ∃ (totals : risk.totals.AccountRiskTotals) (c1 c2 : context.Context)
          (rep : positions.liquidation.math.NormalizedRepaymentPlan) (c : context.Context)
          (unb : math.fp.Wad) (c' : context.Context),
        risk.totals.calculate_account_risk_totals env c1 acc.supply_positions
          acc.borrow_positions = ok (totals, c2) ∧
        positions.liquidation.math.calculate_seized_collateral env acc totals.total_collateral
          rep c = ok ((r.1.seized, unb), c')) := by
  unfold positions.liquidation.plan.build_liquidation_plan
  refine Rets.bind_any fun b => ?_
  refine Rets.bind_any fun _ => ?_
  refine Rets.bind_any fun iter => ?_
  refine Rets.bind_any fun cache1 => ?_
  refine Rets.bind (fun v => risk.totals.calculate_account_risk_totals env cache1
    acc.supply_positions acc.borrow_positions = ok v) (fun _ hv => hv) ?_
  rintro ⟨totals, cache2⟩ htot
  refine Rets.bind_any fun b1 => ?_
  refine Rets.bind_any fun _ => ?_
  refine Rets.bind_any fun x => ?_
  obtain ⟨⟨ps, bb⟩, cache3⟩ := x
  refine Rets.bind_any fun x => ?_
  obtain ⟨sc, cache4⟩ := x
  refine Rets.bind_any fun curve => ?_
  refine Rets.bind_any fun x => ?_
  obtain ⟨repayment, cache5⟩ := x
  refine Rets.bind (fun v => positions.liquidation.math.calculate_seized_collateral env acc
    totals.total_collateral repayment cache5 = ok v) (fun _ hv => hv) ?_
  rintro ⟨⟨seized, unb⟩, cache6⟩ hsz
  rets_all (refine Rets.ok ?_; exact ⟨_, _, _, _, _, _, _, htot, hsz⟩)

/-- **`build_liquidation_plan`, when it returns.** The plan's seizures are
`SeizedFrom` the supply map of the account the builder was given. -/
theorem pl_build_rets (env : Env) (acc : types.Account)
    (pays : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128)) (ctx : context.Context) :
    Rets (positions.liquidation.plan.build_liquidation_plan env acc pays ctx)
      (fun r => SeizedFrom Prod.fst heldRaw
        (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw))
        (r.1.seized : List types.SeizeEntry)) := by
  intro r hr
  obtain ⟨totals, -, -, rep, c, unb, c', -, h⟩ :=
    build_liquidation_plan_seized_computed env acc pays ctx r hr
  exact calculate_seized_collateral_seized_le_held env acc totals.total_collateral rep c _ h

/-- In a map with distinct keys, `Map::get` of an entry's key returns that
entry's value. -/
theorem mapLookup_of_mem {K V : Type} {m : List (K × V)} (hm : (m.map Prod.fst).Nodup)
    {e : K × V} (he : e ∈ m) : mapLookup m e.1 = some e.2 := by
  induction m with
  | nil => simp at he
  | cons a m ih =>
    rw [List.map_cons, List.nodup_cons] at hm
    rcases List.mem_cons.mp he with rfl | he'
    · simp [mapLookup]
    · have hne : ¬ a.1 = e.1 := fun h => hm.1 (h ▸ List.mem_map_of_mem he')
      have h1 := ih hm.2 he'
      unfold mapLookup at h1 ⊢
      rw [List.find?_cons_of_neg (by simpa using hne)]
      exact h1

/-- **Every seizure of a liquidation plan is within one supply entry.** For
every input on which `build_liquidation_plan` returns a plan (no hypothesis):
each seizure entry has the hub asset of some entry of the account's supply
map, and `0 < scaled_amount ≤` that entry's `scaled_amount`. -/
theorem build_liquidation_plan_seized_within (env : Env) (acc : types.Account)
    (pays : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128)) (ctx ctx' : context.Context)
    (plan : positions.liquidation.math.LiquidationPlan)
    (hret : positions.liquidation.plan.build_liquidation_plan env acc pays ctx = ok (plan, ctx')) :
    ∀ x ∈ (plan.seized : List types.SeizeEntry),
      ∃ e ∈ (acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)),
        x.hub_asset = e.1 ∧ 0 < x.scaled_amount.val ∧
        x.scaled_amount.val ≤ e.2.scaled_amount.val :=
  fun x hx => SeizedFrom.mem (pl_build_rets env acc pays ctx (plan, ctx') hret) x hx

/-- **A liquidation plan never seizes more than the account holds
(INV-LIQ-02).** For every input on which `build_liquidation_plan` returns a
plan, on an account whose supply map has distinct keys (a Soroban `Map`):

* every seizure entry names a hub asset the account supplies
  (`Map::get` returns a position, `map_get_eq`), with
  `0 < entry.scaled_amount ≤ position.scaled_amount`;
* no two seizure entries name the same hub asset, so the total seized per
  position is also at most what the position holds.

No other hypothesis: no value domain, no served context, no bound on the
payments, and every opaque host function (spoke flags, spoke config, oracle,
pool, `repays_every_debt_leg`) is arbitrary. -/
theorem build_liquidation_plan_seized_le_held (env : Env) (acc : types.Account)
    (pays : aeneas_soroban_shim.Vec (types.HubAssetKey × Std.I128)) (ctx ctx' : context.Context)
    (plan : positions.liquidation.math.LiquidationPlan)
    (hkeys : ((acc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).map
      Prod.fst).Nodup)
    (hret : positions.liquidation.plan.build_liquidation_plan env acc pays ctx = ok (plan, ctx')) :
    (∀ x ∈ (plan.seized : List types.SeizeEntry), ∃ pos : types.AccountPositionRaw,
        mapLookup acc.supply_positions x.hub_asset = some pos ∧
        0 < x.scaled_amount.val ∧ x.scaled_amount.val ≤ pos.scaled_amount.val) ∧
    ((plan.seized : List types.SeizeEntry).map (fun x => x.hub_asset)).Nodup := by
  have h := pl_build_rets env acc pays ctx (plan, ctx') hret
  refine ⟨fun x hx => ?_, (SeizedFrom.keys_sublist h).nodup hkeys⟩
  obtain ⟨e, he, hk, h0, hle⟩ := SeizedFrom.mem h x hx
  exact ⟨e.2, by rw [hk]; exact mapLookup_of_mem hkeys he, h0, hle⟩

/-! ## Non-vacuity

A concrete call that returns a non-empty seizure list, so the theorems above
do not hold only because no call returns. -/

/-- Hub asset 0 of the token with handle 7. -/
def exKey : types.HubAssetKey := { hub_id := 0#u32, asset := (7 : Nat) }

/-- The one supply position: `10^30` shares (1000 tokens at index `RAY`),
stamped fee rate 10%. -/
def exPos : types.AccountPositionRaw :=
  { scaled_amount := 1000000000000000000000000000000#i128, liquidation_threshold := 8000#u32,
    liquidation_bonus := 500#u32, loan_to_value := 7500#u32, liquidation_fees := 1000#u32 }

/-- An account with that single supply position. -/
def exAcc : types.Account :=
  { owner := (1 : Nat), spoke_id := 1#u32, supply_positions := [(exKey, exPos)],
    borrow_positions := [] }

/-- The token's cached feed: price 1 USD, 7 decimals. -/
def exFeed : types.PriceFeedRaw :=
  { price_wad := 1000000000000000000#i128, asset_decimals := 7#u32, timestamp := 0#u64 }

/-- The market's cached indexes: both `RAY`. -/
def exIndex : types.MarketIndexRaw :=
  { borrow_index := 1000000000000000000000000000#i128,
    supply_index := 1000000000000000000000000000#i128 }

/-- A context that serves the token. -/
def exCtx : context.Context :=
  { env := (), token_prices := [((7 : Nat), exFeed)], market_indexes := [(exKey, exIndex)] }

/-- A repayment plan worth 100 USD at a 5% bonus. -/
def exRep : positions.liquidation.math.NormalizedRepaymentPlan :=
  { repaid := [], refunds := [], repay_usd := 100000000000000000000#i128, bonus := 500#i128,
    full_close := false, repays_all_debt := false, seize_all := false }

/-- Total collateral 1000 USD: the half-up value of the position. -/
def exTc : math.fp.Wad := 1000000000000000000000#i128

/-- The integer inputs of the example's leg. -/
def exLeg : SzLeg :=
  ⟨10^30, 10^27, 10^18, 7, 1000, 10^21, szBudget (10^20) 500, szOpb 500, false, false⟩

/-- The example's leg, read from the example account, context and plan. -/
theorem exLeg_eq : planLeg exCtx exTc.val exRep (exKey, exPos) = exLeg := by
  simp [planLeg, legOfRaw, exLeg, supplyIndexOf, priceOf, decOf, mapLookup, exCtx, exKey, exPos,
    exRep, exTc, exFeed, exIndex]

/-- The example's leg is in the production domain `SzLegOK`. -/
theorem exLeg_ok : SzLegOK exLeg := by
  constructor <;>
    simp only [exLeg, SzLeg.V, VD, huv, huD, szBudget, szOpb] <;> norm_num

/-- The example's leg emits an entry. -/
theorem exLeg_emit : exLeg.Emit := by
  simp only [exLeg, SzLeg.Emit, SzLeg.Seized, SzLeg.Capped, SzLeg.S1, SzLeg.WU, SzLeg.S0,
    SzLeg.Q, SzLeg.L, SzLeg.Sh, SzLeg.V, SzLeg.A, SzLeg.Amt, SzLeg.Dd, szBudget, szOpb, huv, huD]
  norm_num

/-- The example's leg seizes `105·10^27` shares. -/
theorem exLeg_seized : exLeg.Seized = 105 * 10^27 := by
  simp only [exLeg, SzLeg.Seized, SzLeg.Capped, SzLeg.S1, SzLeg.WU, SzLeg.S0, SzLeg.Q,
    SzLeg.L, SzLeg.Sh, SzLeg.V, SzLeg.A, szBudget, szOpb, huv, huD]
  norm_num

/-- The example's leg leaves no unseized USD. -/
theorem exLeg_uns : exLeg.Uns = 0 := by
  simp only [exLeg, SzLeg.Uns, SzLeg.WU]
  norm_num

/-- **The bound is not vacuous.** On a concrete account (one supply position
of `10^30` shares, distinct keys), context and plan (100 USD at 5%),
`calculate_seized_collateral` returns, and it returns exactly one seizure
entry, for the account's hub asset, of `105·10^27` shares: positive and below
the `10^30` held. So `calculate_seized_collateral_seized_le_held` constrains
a real run, and the key hypothesis of
`build_liquidation_plan_seized_le_held` is satisfiable. -/
theorem seized_le_held_nonvacuous :
    ((exAcc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)).map
      Prod.fst).Nodup ∧
    ∃ r, positions.liquidation.math.calculate_seized_collateral () exAcc exTc exRep exCtx = ok r ∧
      ∃ x, r.1.1 = [x] ∧ x.hub_asset = exKey ∧ x.scaled_amount.val = 105 * 10^27 ∧
        heldRaw (exKey, exPos) = 10^30 := by
  refine ⟨by simp [exAcc], ?_⟩
  have hlegs : ∀ e ∈ (exAcc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)),
      Serves exCtx e.1 ∧ SzLegOK (planLeg exCtx exTc.val exRep e) := by
    intro e he
    have he' : e = (exKey, exPos) := by simpa [exAcc] using he
    subst he'
    refine ⟨⟨⟨exFeed, by simp [mapLookup, exCtx, exKey]⟩, ⟨exIndex, by simp [mapLookup, exCtx]⟩⟩, ?_⟩
    rw [exLeg_eq]; exact exLeg_ok
  have huns : lsum (fun e => (planLeg exCtx exTc.val exRep e).Uns)
      (exAcc.supply_positions : List (types.HubAssetKey × types.AccountPositionRaw)) < 2^127 := by
    simp only [exAcc, lsum, List.map_cons, List.map_nil, List.sum_cons, List.sum_nil, exLeg_eq,
      exLeg_uns]
    norm_num
  obtain ⟨r, hr, -, hout, -⟩ := spec_imp_eq_ok
    (calculate_seized_collateral_spec () exAcc exTc exRep exCtx (by simp [exTc])
      (by simp [exRep]) (by simp [exRep]) (by simp [exRep])
      (by simp only [exRep, szBudget, szOpb, huD]; norm_num) hlegs huns)
  refine ⟨r, hr, ?_⟩
  have hout' : SzOut (planLeg exCtx exTc.val exRep) Prod.fst exCtx [(exKey, exPos)] r.1.1 := hout
  unfold SzOut at hout'
  rw [exLeg_eq, ite_eq_left exLeg_emit] at hout'
  obtain ⟨x, out', hx, hent, hnil⟩ := hout'
  have hnil' : out' = [] := hnil
  subst hnil'
  refine ⟨x, hx, hent.1, ?_, ?_⟩
  · rw [hent.2.2.2.1, exLeg_seized]
  · simp [heldRaw, exPos]

end ControllerAeneas.Proofs
