/-
`contracts/controller/src/positions/liquidation/apply.rs` and the seizure
budget of `math.rs::calculate_seized_collateral`:

* `apply_liquidation_repayments` credits only measured receipts: each leg's
  credited USD is its planned value when the pool received at least the
  planned amount, else `⌊usd · received / amount⌋`; the debt repaid on the
  pool is the received amount (INV-LIQ-03, connected below to
  `scale_seizures_to_received`).
* `credited_shares` is the liquidator part of `split_seized_shares`.
* `check_bad_debt_after_liquidation` invokes bad-debt socialization exactly
  when debt remains and `coll < debt ∧ coll ≤ 5 WAD` (INV-LIQ-04).
* The seizure budget `repay_usd · (1 + bonus)` of
  `calculate_seized_collateral` is rounded half up (INV-LIQ-02), with a
  concrete case where it exceeds the exact product.
-/
import ControllerAeneas.Proofs.Repayment
import ControllerAeneas.Proofs.Shares
import ControllerAeneas.Proofs.Curve
import ControllerAeneas.Proofs.Seizure

open Aeneas Aeneas.Std Result
open FixedPoint.Spec FixedPoint.HalfUpSpec
open controller_aeneas

set_option maxRecDepth 20000
set_option maxHeartbeats 2000000

namespace ControllerAeneas.Proofs

/-! ## `credited_shares` -/

/-- **`credited_shares` is the liquidator's exact remainder.** For an entry
with `0 ≤ bonus_scaled ≤ scaled_amount` and fee rate below 100% (what
`LiquidationPlan::validate` enforces), the credited shares are
`S − ⌈bonus·fee/BPS⌉`, between 0 and `S`; never aborts. -/
theorem credited_shares_spec (env : Env) (e : types.SeizeEntry)
    (hb0 : 0 ≤ e.bonus_scaled.val) (hbS : e.bonus_scaled.val ≤ e.scaled_amount.val)
    (hf : (e.liquidation_fees.val : Int) < 10^4) :
    positions.liquidation.apply.credited_shares env e ⦃ c =>
      ∃ fee, IsCeil (e.bonus_scaled.val * e.liquidation_fees.val) (10^4) fee ∧
        c.val = e.scaled_amount.val - fee ∧ 0 ≤ c.val ∧ c.val ≤ e.scaled_amount.val ⦄ := by
  unfold positions.liquidation.apply.credited_shares
  simp only [ray_from_same, bind_ok]
  step with split_seized_shares_spec env e.scaled_amount e.bonus_scaled e.liquidation_fees hb0 hbS hf
    as ⟨r1, r2, h1, h2, h3, h4, h5, h6⟩
  exact ⟨r1.val, h1, by omega, h5, h6⟩

/-! ## `check_bad_debt_after_liquidation` (INV-LIQ-04) -/

/-- **Bad-debt socialization is explicit and exactly conditioned.**
`check_bad_debt_after_liquidation` behaves as follows (an equation, so it
holds whatever the external effects do):

* no debt position left: it only runs the empty-account cleanup;
* otherwise, when `total_collateral < total_debt` and
  `total_collateral ≤ 5 WAD` (`BAD_DEBT_USD_THRESHOLD`), it runs
  `execute_bad_debt_cleanup` on the whole account with the given totals
  (which seizes all remaining collateral and socializes all remaining debt);
* otherwise it changes nothing.

No other path reaches the socialization. -/
theorem check_bad_debt_after_liquidation_eq (env : Env) (ctx : context.Context) (id : Std.U64)
    (acc : types.Account) (totals : risk.totals.AccountRiskTotals) :
    positions.liquidation.apply.check_bad_debt_after_liquidation env ctx id acc totals =
      if (acc.borrow_positions : List _) = [] then
        (do account_store.cleanup_account_if_empty env acc id; ok ctx)
      else if totals.total_collateral.val < totals.total_debt.val ∧
          totals.total_collateral.val ≤ 5 * 10^18 then
        positions.liquidation.bad_debt.execute_bad_debt_cleanup env ctx id acc totals
      else ok ctx := by
  unfold positions.liquidation.apply.check_bad_debt_after_liquidation
  simp only [aeneas_soroban_shim.Map.is_empty, bind_ok, List.isEmpty_iff]
  by_cases he : (acc.borrow_positions : List _) = []
  · simp only [he, ↓reduceIte]
  · simp only [he, ↓reduceIte]
    have hs := is_socializable_bad_debt_spec totals.total_debt totals.total_collateral
    cases hb : positions.liquidation.curve.is_socializable_bad_debt totals.total_debt
        totals.total_collateral with
    | ret b =>
      rw [hb] at hs
      change (ok b) ⦃ b => (b = true ↔ _) ⦄ at hs
      rw [WP.spec_ok] at hs
      simp only [bind_ok]
      by_cases hc : totals.total_collateral.val < totals.total_debt.val ∧
          totals.total_collateral.val ≤ 5 * 10^18
      · rw [if_pos hc, if_pos (hs.mpr hc)]
      · have : b = false := by
          cases b
          · rfl
          · exact absurd (hs.mp rfl) hc
        rw [if_neg hc, this]; rfl
    | vis e k => rw [hb] at hs; simp at hs
    | div => rw [hb] at hs; simp at hs

/-! ## `apply_liquidation_repayments`: credit only measured receipts -/

/-- Receipt measured for a repayment leg when the liquidator transfers the
planned amount to the pool (partial-close plans, `offered = None`). -/
noncomputable def rcvOf (liq : aeneas_soroban_shim.Address) (e : types.RepayEntry) : Std.I128 :=
  transferReceived e.hub_asset.asset liq poolAddress e.amount

/-- Credited USD of a leg: the planned value if at least the planned amount
arrived, else `⌊usd · received / amount⌋`. -/
def recvLeg (usd amount rcv : Int) : Int := if amount ≤ rcv then usd else usd * rcv / amount

/-- Credited USD of a leg for a given liquidator. -/
noncomputable def legRecv (liq : aeneas_soroban_shim.Address) (e : types.RepayEntry) : Int :=
  recvLeg e.usd_wad.val e.amount.val (rcvOf liq e).val

/-- Conditions on a planned repayment leg: the spoke flags allow the exit,
the planned amount is positive, the recorded USD value is non-negative, the
measured receipt is non-negative, and the account has the debt position. -/
def ApplyLegOK (acc : types.Account) (liq : aeneas_soroban_shim.Address) (e : types.RepayEntry) : Prop :=
  spokeFlagsAllow acc.spoke_id e.hub_asset .AllowOnExit = true ∧ 0 < e.amount.val ∧
  0 ≤ e.usd_wad.val ∧ 0 ≤ (rcvOf liq e).val ∧
  ∃ d, mapLookup acc.borrow_positions e.hub_asset = some d

/-- The credited value is between 0 and the planned value, and proportional
with the floor in the protocol's favour: `credited · amount ≤ usd · received`
on a short receipt. -/
theorem recvLeg_bounds (usd amount rcv : Int) (hu : 0 ≤ usd) (ha : 0 < amount) (hr : 0 ≤ rcv) :
    0 ≤ recvLeg usd amount rcv ∧ recvLeg usd amount rcv ≤ usd ∧
    (rcv < amount → recvLeg usd amount rcv * amount ≤ usd * rcv) := by
  unfold recvLeg
  split
  · exact ⟨hu, le_refl _, fun h => by omega⟩
  · rename_i h
    refine ⟨Int.ediv_nonneg (mul_nonneg hu hr) ha.le, ?_, fun _ => Int.ediv_mul_le _ (by omega)⟩
    apply Int.ediv_le_of_le_mul ha
    have : usd * rcv ≤ usd * amount := mul_le_mul_of_nonneg_left (by omega) hu
    linarith

/-- A pool action recorded for a leg repays the debt position of the leg's
hub asset with the received amount. -/
def ActionOf (liq : aeneas_soroban_shim.Address) (e : types.RepayEntry) (a : types.PoolAction) : Prop :=
  a.hub_asset = e.hub_asset ∧ a.amount = rcvOf liq e

theorem apply_repay_loop_nil (env : Env) (liq : aeneas_soroban_shim.Address) (acc : types.Account)
    (ctx : context.Context) (pool : aeneas_soroban_shim.Address) (acts : List types.PoolAction)
    (u : math.fp.Wad) :
    positions.liquidation.apply.apply_liquidation_repayments_loop [] env liq acc none ctx pool acts u =
      ok (acc, ctx, acts, u) := by
  unfold positions.liquidation.apply.apply_liquidation_repayments_loop; rw [loop]
  simp [positions.liquidation.apply.apply_liquidation_repayments_loop.body,
    aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next]

/-- The repayment loop (partial-close plan): returns the account and context
unchanged (all effects happen in the batch call after the loop), one pool
action per leg carrying the received amount, and the credited USD
`u + Σ legRecv`. -/
theorem apply_repay_loop_spec (env : Env) (liq : aeneas_soroban_shim.Address) (acc : types.Account)
    (ctx : context.Context) :
    ∀ (L : List types.RepayEntry) (acts : List types.PoolAction) (u : math.fp.Wad),
      (∀ e ∈ L, ApplyLegOK acc liq e) → 0 ≤ u.val →
      u.val + lsum (fun e : types.RepayEntry => e.usd_wad.val) L < 2^127 →
      positions.liquidation.apply.apply_liquidation_repayments_loop L env liq acc none ctx poolAddress acts u
        ⦃ r => r.1 = acc ∧ r.2.1 = ctx ∧ r.2.2.2.val = u.val + lsum (legRecv liq) L ∧
          ∃ xs, r.2.2.1 = acts ++ xs ∧ List.Forall₂ (ActionOf liq) L xs ⦄ := by
  intro L
  induction L with
  | nil => intro acts u _ _ _; rw [apply_repay_loop_nil]; simp
  | cons e L ih =>
    intro acts u hok hu hfit
    obtain ⟨hflag, ha, hus, hrcv, ⟨d, hd⟩⟩ := hok e (by simp)
    have hrest : ∀ x ∈ L, ApplyLegOK acc liq x := fun x hx => hok x (by simp [hx])
    have hsum0 : 0 ≤ lsum (fun e : types.RepayEntry => e.usd_wad.val) L := by
      unfold lsum; apply List.sum_nonneg
      intro x hx
      obtain ⟨y, hy, rfl⟩ := List.mem_map.mp hx
      exact (hrest y hy).2.2.1
    rw [lsum_cons] at hfit
    obtain ⟨lb0, lbu, lbp⟩ := recvLeg_bounds e.usd_wad.val e.amount.val (rcvOf liq e).val hus ha hrcv
    have lb0' : 0 ≤ legRecv liq e := lb0
    have lbu' : legRecv liq e ≤ e.usd_wad.val := lbu
    unfold positions.liquidation.apply.apply_liquidation_repayments_loop
    rw [loop]
    dsimp only
    rw [positions.liquidation.apply.apply_liquidation_repayments_loop.body]
    simp only [aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator.next, bind_ok]
    simp [positions.enforce_spoke_asset_flags, hflag, core.option.Option.map_or,
      payment_legs.transfer_amount_measured, ha, wad_from_same]
    have hR : transferReceived e.hub_asset.asset liq poolAddress e.amount = rcvOf liq e := rfl
    rw [hR]
    have hpush : ∀ (u1 : math.fp.Wad), u1.val = u.val + legRecv liq e →
        (do
          let x_1 ← types.HubAssetKey.Insts.CoreCloneClone.clone e.hub_asset
          let x_2 ← aeneas_soroban_shim.Map.get acc.borrow_positions x_1
          let x_3 ← validation.expect_invariant env x_2
          let x_4 ← types.DebtPosition.Insts.CoreConvertFromShared0DebtPositionRaw.from x_3
          let x_5 ← positions.make_pool_action
            (core.convert.IntoFrom types.ScaledPositionRaw.Insts.CoreConvertFromShared0DebtPosition) x_4
            (rcvOf liq e) e.hub_asset
          let x_6 ← aeneas_soroban_shim.Vec.push_back acts x_5
          loop
            (fun x => positions.liquidation.apply.apply_liquidation_repayments_loop.body env liq acc none
              poolAddress x.1 x.2.1 x.2.2.1 x.2.2.2)
            (L, ctx, x_6, u1)) ⦃ r =>
          r.1 = acc ∧ r.2.1 = ctx ∧ r.2.2.2.val = u.val + (legRecv liq e + lsum (legRecv liq) L) ∧
          ∃ xs, r.2.2.1 = acts ++ xs ∧ List.Forall₂ (ActionOf liq) (e :: L) xs ⦄ := by
      intro u1 hu1
      simp [types.HubAssetKey.Insts.CoreCloneClone.clone, lift, core.clone.impls.CloneU32.clone,
        aeneas_soroban_shim.Address.Insts.CoreCloneClone.clone, map_get_eq, hd,
        validation.expect_invariant, core.option.Option.unwrap_or_else,
        types.DebtPosition.Insts.CoreConvertFromShared0DebtPositionRaw.from, positions.make_pool_action,
        core.convert.IntoFrom.into, types.ScaledPositionRaw.Insts.CoreConvertFromShared0DebtPosition.from,
        math.fp.Ray.raw, aeneas_soroban_shim.Vec.push_back]
      let pa : types.PoolAction :=
        { position := { scaled_amount := d.scaled_amount }, amount := rcvOf liq e, hub_asset := e.hub_asset }
      have := ih (acts ++ [pa]) u1 hrest (by rw [hu1]; omega) (by rw [hu1]; omega)
      unfold positions.liquidation.apply.apply_liquidation_repayments_loop at this
      dsimp only at this
      apply WP.spec_mono this
      rintro ⟨a1, c1, xs1, r1⟩ ⟨h1, h2, h3, ⟨xs, hxs, hf⟩⟩
      refine ⟨h1, h2, by rw [h3, hu1]; ring, ⟨pa :: xs, by rw [hxs]; simp, ?_⟩⟩
      exact List.Forall₂.cons ⟨rfl, rfl⟩ hf
    by_cases hge : e.amount.val ≤ (rcvOf liq e).val
    · simp only [hge, ↓reduceIte, bind_ok, math.fp.Wad.checked_add]
      have hL : legRecv liq e = e.usd_wad.val := by simp [legRecv, recvLeg, hge]
      step with checked_add_raw_spec env u e.usd_wad (by constructor <;> omega) as ⟨u1, hu1⟩
      exact hpush u1 (by rw [hu1, hL])
    · simp only [hge, ↓reduceIte, bind_ok, math.fp.Wad.checked_add, math.fp.Wad.mul_ratio_floor]
      have hL : legRecv liq e = e.usd_wad.val * (rcvOf liq e).val / e.amount.val := by
        simp [legRecv, recvLeg, hge]
      have hfl : IsFloor (e.usd_wad.val * (rcvOf liq e).val) e.amount.val (legRecv liq e) := by
        rw [hL, isFloor_pos ha]
        exact ⟨Int.ediv_mul_le _ (by omega), by
          have := Int.lt_ediv_add_one_mul_self (e.usd_wad.val * (rcvOf liq e).val) ha; linarith⟩
      step with mul_div_floor_spec env e.usd_wad (rcvOf liq e) e.amount (by omega)
        ⟨legRecv liq e, hfl, by omega, by omega⟩ as ⟨w, hw⟩
      have hwv : w.val = legRecv liq e := IsFloor.unique (by omega) hw hfl
      step with checked_add_raw_spec env u w (by rw [hwv]; constructor <;> omega) as ⟨u1, hu1⟩
      exact hpush u1 (by rw [hu1, hwv])

/-- **`apply_liquidation_repayments` credits only measured receipts
(partial-close plan, `offered = None`).** For legs satisfying `ApplyLegOK`
whose recorded values sum below `2^127`, the function is exactly: build one
pool action per leg repaying that leg's debt position with the *received*
amount, then run the (external) batch repay; it returns the credited USD
`u = Σ legRecv`, where each leg contributes its planned `usd_wad` if the pool
received at least the planned amount and `⌊usd_wad · received / amount⌋`
otherwise. Consequently `0 ≤ u ≤ Σ usd_wad` (`legRecv_le_planned`). -/
theorem apply_liquidation_repayments_eq (env : Env) (liq : aeneas_soroban_shim.Address)
    (acc : types.Account) (repaid : List types.RepayEntry) (ctx : context.Context)
    (hok : ∀ e ∈ repaid, ApplyLegOK acc liq e)
    (hfit : lsum (fun e : types.RepayEntry => e.usd_wad.val) repaid < 2^127) :
    ∃ (u : math.fp.Wad) (acts : List types.PoolAction),
      u.val = lsum (legRecv liq) repaid ∧ List.Forall₂ (ActionOf liq) repaid acts ∧
      positions.liquidation.apply.apply_liquidation_repayments env liq acc repaid none ctx =
        (do
          let (_, acc2, ctx2) ← positions.apply_repay_batch env acc liq events.PositionAction.LiqRepay acts ctx
          ok (u, acc2, ctx2)) := by
  have h0 : (math.fp.Wad.ZERO : Std.I128).val = 0 := by unfold math.fp.Wad.ZERO; rfl
  have hs := apply_repay_loop_spec env liq acc ctx repaid [] math.fp.Wad.ZERO hok (by omega)
    (by rw [h0]; omega)
  obtain ⟨r, hr, hP⟩ : ∃ r, positions.liquidation.apply.apply_liquidation_repayments_loop repaid env liq
      acc none ctx poolAddress [] math.fp.Wad.ZERO = ok r ∧ _ := by
    cases hm : positions.liquidation.apply.apply_liquidation_repayments_loop repaid env liq acc none ctx
        poolAddress [] math.fp.Wad.ZERO with
    | ret x => rw [hm] at hs; exact ⟨x, rfl, (WP.spec_ok x).mp hs⟩
    | vis e k => rw [hm] at hs; simp at hs
    | div => rw [hm] at hs; simp at hs
  obtain ⟨ha, hc, hu, ⟨xs, hxs, hf⟩⟩ := hP
  obtain ⟨a1, c1, acts1, u1⟩ := r
  simp only at ha hc hu hxs
  refine ⟨u1, xs, by rw [hu, h0, zero_add], hf, ?_⟩
  unfold positions.liquidation.apply.apply_liquidation_repayments
  simp [context.Context.cached_pool_address, aeneas_soroban_shim.Vec.new,
    aeneas_soroban_shim.Vec.iter, hr, ha, hc, hxs]

/-- The credited USD is at most the planned USD, leg by leg and in total. -/
theorem legRecv_le_planned (liq : aeneas_soroban_shim.Address) (acc : types.Account)
    (repaid : List types.RepayEntry) (hok : ∀ e ∈ repaid, ApplyLegOK acc liq e) :
    0 ≤ lsum (legRecv liq) repaid ∧
    lsum (legRecv liq) repaid ≤ lsum (fun e : types.RepayEntry => e.usd_wad.val) repaid := by
  induction repaid with
  | nil => simp
  | cons e L ih =>
    obtain ⟨-, ha, hus, hrcv, -⟩ := hok e (by simp)
    obtain ⟨b0, bu, -⟩ := recvLeg_bounds e.usd_wad.val e.amount.val (rcvOf liq e).val hus ha hrcv
    have := ih (fun x hx => hok x (by simp [hx]))
    simp only [lsum_cons]
    have b0' : 0 ≤ legRecv liq e := b0
    have bu' : legRecv liq e ≤ e.usd_wad.val := bu
    omega

/-- **INV-LIQ-03 end to end (under-delivery reduces seizure).** Let `u` be the
USD that `apply_liquidation_repayments` credits (`apply_liquidation_repayments_eq`)
and `planned = Σ usd_wad` the plan's repayment (what `sum_repaid_usd`
returns and `NormalizedRepaymentPlan::validate` requires to equal
`repay_usd`). Then `u ≤ planned`, and `scale_seizures_to_received(seized, u,
planned)` returns the planned seizures unchanged when everything arrived and
otherwise floors every entry by `u / planned` (`ScaledEntry`: shares, bonus
shares, amount and fee each at most their planned value and proportional to
`u / planned`). -/
theorem underdelivery_scales_seizures (env : Env) (liq : aeneas_soroban_shim.Address)
    (acc : types.Account) (repaid : List types.RepayEntry) (seized : List types.SeizeEntry)
    (hok : ∀ e ∈ repaid, ApplyLegOK acc liq e) (u planned : math.fp.Wad)
    (hu : u.val = lsum (legRecv liq) repaid)
    (hp : planned.val = lsum (fun e : types.RepayEntry => e.usd_wad.val) repaid)
    (hent : u.val < planned.val → ∀ e ∈ seized, SeizeEntryOk e) :
    u.val ≤ planned.val ∧
    positions.liquidation.math.scale_seizures_to_received env seized u planned ⦃ out =>
      (planned.val ≤ u.val → out = seized) ∧
      (u.val < planned.val → List.Forall₂ (ScaledEntry u.val planned.val) seized out) ⦄ := by
  obtain ⟨h0, hle⟩ := legRecv_le_planned liq acc repaid hok
  refine ⟨by rw [hu, hp]; exact hle, ?_⟩
  apply WP.spec_mono (scale_seizures_to_received_spec env seized u planned (by rw [hu]; exact h0) hent)
  rintro out ⟨h1, h2⟩
  refine ⟨fun h => h1 (Or.inr h), fun h => h2 ⟨by omega, h⟩⟩

/-! ## The seizure budget of `calculate_seized_collateral` (INV-LIQ-02) -/

/-- `Bps::to_wad` is exact for a non-negative rate: `b · 10^14`. -/
theorem liq_bps_to_wad_val (env : Env) (b : math.fp.Bps) (hb : 0 ≤ b.val) (hbB : b.val ≤ 10^4) :
    math.fp.Bps.to_wad b env ⦃ w => w.val = b.val * 10^14 ⦄ := by
  unfold math.fp.Bps.to_wad
  have hW : (constants.production.shared.WAD : Std.I128).val = 10^18 := WAD_val
  have hB : (constants.production.shared.BPS : Std.I128).val = 10^4 := BPS_val
  have hex : IsHalfUp (b.val * 10^18) (10^4) (b.val * 10^14) := by
    have := IsHalfUp.mul_self (b.val * 10^14) (10^4) (by norm_num)
    rw [show b.val * 10^14 * 10^4 = b.val * 10^18 by ring] at this; exact this
  step with mul_div_half_up_spec env b constants.production.shared.WAD constants.production.shared.BPS
    hb (by rw [hW]; norm_num) (by rw [hB]; norm_num) ⟨b.val * 10^14, by rw [hW, hB]; exact hex,
      by nlinarith⟩ as ⟨w, hw⟩
  rw [hW, hB] at hw
  exact IsHalfUp.unique (by norm_num) hw hex

/-- **The seizure budget is the repayment times `1 + bonus`, rounded half
up.** The first three steps of `calculate_seized_collateral`
(`one_plus_bonus = WAD + bonus.to_wad()`,
`total_seizure_usd = repay_usd.mul(one_plus_bonus)`) compute
`hu(repay · (BPS + bonus) / BPS)` for every non-negative repayment and bonus
`≤ 100%` whose result fits `i128`. Half-up means the budget can exceed the
exact `repay · (1 + bonus)` by up to half a WAD unit (`10^-18` USD), see
`seizure_budget_exceeds_exact`. -/
theorem seizure_budget_spec (env : Env) (repay : math.fp.Wad) (bonus : math.fp.Bps)
    (hr : 0 ≤ repay.val) (hb : 0 ≤ bonus.val) (hbB : bonus.val ≤ 10^4)
    (hfit : huD (repay.val * (10^4 + bonus.val)) (10^4) < 2^127) :
    (do
      let w ← math.fp.Bps.to_wad bonus env
      let opb ← math.fp.Wad.checked_add math.fp.Wad.ONE env w
      math.fp.Wad.mul repay env opb) ⦃ t =>
        IsHalfUp (repay.val * (10^4 + bonus.val)) (10^4) t.val ∧
        t.val = huD (repay.val * (10^4 + bonus.val)) (10^4) ⦄ := by
  have hW : (constants.production.shared.WAD : Std.I128).val = 10^18 := WAD_val
  step with liq_bps_to_wad_val env bonus hb hbB as ⟨w, hw⟩
  unfold math.fp.Wad.checked_add
  have hone : (math.fp.Wad.ONE : Std.I128).val = 10^18 := by unfold math.fp.Wad.ONE; exact hW
  step with checked_add_raw_spec env math.fp.Wad.ONE w (by rw [hone, hw]; constructor <;> nlinarith)
    as ⟨o, ho⟩
  unfold math.fp.Wad.mul
  have ho' : o.val = (10^4 + bonus.val) * 10^14 := by rw [ho, hone, hw]; ring
  obtain ⟨q, hq⟩ := IsHalfUp.exists (repay.val * o.val) (10^18) (by norm_num)
  have hconv : ∀ x, IsHalfUp (repay.val * o.val) (10^18) x ↔
      IsHalfUp (repay.val * (10^4 + bonus.val)) (10^4) x := by
    intro x; rw [ho']; unfold IsHalfUp; constructor <;> rintro ⟨h1, h2⟩ <;> constructor <;> nlinarith
  have hqe : q = huD (repay.val * (10^4 + bonus.val)) (10^4) :=
    isHalfUp_eq (by norm_num) ((hconv q).mp hq)
  step with mul_div_half_up_spec env repay o constants.production.shared.WAD hr
    (by rw [ho']; nlinarith) (by rw [hW]; norm_num) ⟨q, by rw [hW]; exact hq, by rw [hqe]; exact hfit⟩
    as ⟨t, ht⟩
  rw [hW] at ht
  have := (hconv t.val).mp ht
  exact ⟨this, isHalfUp_eq (by norm_num) this⟩

/-- **Concrete case: the budget exceeds the exact product.** With
`repay_usd = 10` (WAD units, `10^-17` USD) and a 5% bonus, the exact budget is
`10.5` units; the code budgets `hu(10.5) = 11`. The excess is below one
`10^-18` USD unit per liquidation and is then distributed pro rata, so it is
economically irrelevant, but the budget is not rounded in the protocol's
favour. -/
theorem seizure_budget_exceeds_exact :
    huD (10 * (10^4 + 500)) (10^4) = 11 ∧ 11 * 10^4 > 10 * (10^4 + 500) := by
  unfold huD; norm_num

end ControllerAeneas.Proofs
