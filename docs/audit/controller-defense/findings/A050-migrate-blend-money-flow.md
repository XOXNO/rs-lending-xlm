# A050 — Migrate-from-blend money flow and leftover repay

- Agent: A050
- Theme: T3 (money movement), T4 (post-move risk gates), residual T1 (flash windows on Blend submit)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/strategies/migrate_blend.rs` (full: `process_migrate_blend`, `execute_migration_debt_leg`, `reconcile_debt_refunds`, `deposit_withdrawn`, validators)
  - `contracts/controller/src/external/blend.rs` (`blend_repay_all`, `blend_sweep_all`, `authorize_repay_pulls`, `guarded_submit`)
  - `contracts/controller/src/positions/debt.rs:248-298` (`borrow_into_controller`, `charge_fee = false`)
  - `contracts/controller/src/strategies/legs.rs:49-89,220-233` (`repay_debt_from_controller`, `refund_controller_balance_delta`)
  - `contracts/controller/src/positions/supply.rs:106-155` (`process_deposit` / measured controller→pool)
  - `contracts/controller/src/strategies/mod.rs:41-80` (`snapshot_balances`, `strategy_finalize`)
  - `contracts/controller/src/payments.rs:14-24` (`balance_delta_since`)
  - `contracts/controller/src/lib.rs:342-372` (`migrate_from_blend` entrypoint)
- Defense: Migration is a single atomic tx: zero-fee hub borrow into controller custody → capped Blend repay pulls → **leftover hub repay of measured controller Δ** (not a caller refund of gross cash) → Blend collateral/supply sweep → measured deposit of sweep Δ → `strategy_finalize` solvency. Blend pool is allowlisted; repay auth is per-asset capped at `debt_caps`; every controller balance read is baseline+`checked_sub`; pre-existing controller balances are untouched; debt assets are unique; withdraw assets must be supplyable before any Blend call.
- Gap: (1) Approved Blend pool is a trust boundary — hostile upgrade/behavior can consume borrowed tokens or under-deliver sweeps; HF gate + tx atomicity limit protocol loss to the migrator’s own attempt (threat-model / Tamper.8 / INV-STRAT-03). (2) Post-guard listed-token hooks on leftover `repay_debt_from_controller` and `process_deposit` — shared A007 §5 / A055 residual. (3) Oversized `debt_caps` briefly mint full-cap hub debt before leftover repay; utilization/spoke-borrow caps can fail the call (user sizing footgun, not theft). (4) `collateral_assets` / `supply_assets` are not de-duplicated at the request layer (only the deposit list is); duplicate same-kind Blend requests no-op after empty — low hygiene. (5) `reentrancy_matrix` omits `migrate_from_blend` (code still gates; A007 coverage gap).
- Impact: No path found that credits hub supply/debt from unmeasured Blend reports, steals pre-existing controller balances, leaves successful migrants with hub debt equal to the **cap** rather than Blend’s consumed liability, or lets an unapproved pool run. Successful leftover reconciliation nets hub debt ≈ Blend liability (not the cap). Cap-too-low and unhealthy end-states revert with Blend/controller positions intact. Blast radius of Blend mischief is the caller’s own tokens/account inside one reverting or self-harming tx.
- Evidence: INV-STRAT-03, INV-ACCT-03, INV-FLASH-02, INV-AUTH-02, INV-RISK-01; STRIDE Tamper.8 / I9 / R.1; threat-model “Blend migration”; `docs/reference/endpoints.md` migrate + unlisted-leftover discipline; harness `tests/test-harness/tests/strategy/migrate_blend.rs`, fuzz `tests/fuzz/migrate_blend.rs` / `prop_migrate_blend_reconciles_same_asset`; unit `contracts/controller/tests/strategies/mod.rs`, `tests/external/blend.rs`. Cross-ref A003, A007, A032, A041, A045, A047, A055, A072.
- Opinion: Leftover handling is the load-bearing correctness property and is implemented correctly: excess borrowed cash is **repaid into hub debt** via measured Δ, not transferred to the caller as free tokens. That is the right dual of swap leftover (which returns `token_in` to `refund_to`). Do not replace leftover repay with a caller refund — that would leave inflated hub debt. Keep `charge_fee = false`, capped `authorize_repay_pulls`, and baseline snapshots.

## Method

1. Read `COORDINATION.md`, `SEED.md`, README format, INV-STRAT-02/03, INV-ACCT-03, INV-FLASH-02, threat-model Blend row, STRIDE Tamper.8, endpoints migrate/leftover sections.
2. Traced `lib.rs::migrate_from_blend` → `process_migrate_blend` end-to-end: gates → debt leg → sweep/deposit → finalize → event.
3. Decomposed money hops: `borrow_into_controller` (fee-free), `blend_repay_all` + auth caps, `reconcile_debt_refunds` → `repay_debt_from_controller`, `blend_sweep_all` → `deposit_withdrawn` → `process_deposit`.
4. Cross-checked MockBlend repay (pull-cap + refund) vs measured leftover semantics; harness refund / exact-cap / zero-liability / same-asset / multi-debt / preexisting-balance / cap-too-low tests; fuzz hygiene (controller leftover ≤ 4, debt ≠ cap).
5. Cross-checked peers A003 (caller Blend vs account hub), A007 (flash windows), A032 (finalize), A041/A045/A047 (custody measure patterns), A055 (lying tokens). No novel critical fund-theft gap beyond documented Blend allowlist trust.

## Scope

Audit of **token and position money movement** on `migrate_from_blend`, with emphasis on leftover borrowed-amount reconciliation after Blend repay.

In scope: `migrate_blend.rs`, `external/blend.rs`, and the shared legs those call (`borrow_into_controller`, `repay_debt_from_controller`, `process_deposit`, snapshots, finalize).

Out of scope for depth (peer agents): entrypoint macro/pause inventory (A001), auth predicate detail (A003), flash-guard storage lifecycle (A030), strategy finalize batching abstractly (A032), lying-token taxonomy (A055), spoke-usage index math (A076/A082), NFT mint on account create (A004).

## Verdict

**Defended.** Cash and share flows are measured at the controller custody boundary, leftover borrow is burned back against hub debt rather than paid out, Blend interaction is allowlisted and pull-capped, and post-migration solvency uses the shared strategy gate. Residuals are approved-pool trust, listing-hook reentrancy, and caller sizing of `debt_caps` — not silent controller-balance theft or cap-as-debt persistence on the happy path.

---

## 1. End-to-end money sequence

```
gates (no token move)
  1. require_authorized_caller; when_not_paused (entrypoint)
  2. require_hub_active; non-empty request; is_blend_pool_approved
  3. unique debt assets; load_or_create_account(AccountGuard::Migrate)
  4. prefetch prices; require_can_supply for every withdraw asset

debt leg (execute_migration_debt_leg) — skipped if debt_caps empty
  5. snapshot controller balances for each debt asset  → before_debt
  6. for each (asset, max): borrow_into_controller(..., max, charge_fee=false, Migrate)
       pool create_strategy → controller; assert Δ == amount_received > 0
       merge_debt_leg Entry (hub debt = max principal)
  7. authorize_repay_pulls(asset → blend_pool, amount=max) per cap
  8. guarded_submit Blend REQ_REPAY(amount=max) for caller; spender=to=controller
  9. reconcile_debt_refunds:
       refund = Δ(controller, asset) since before_debt
       if refund > 0: repay_debt_from_controller(debt_available=refund)
         → measured controller→pool transfer; burn hub debt by received;
           any pool overpay Δ forwarded to caller

sweep / deposit — skipped if no withdraw assets
 10. snapshot controller balances for withdraw_assets → before_withdraw
 11. guarded_submit Blend withdraw-collateral / withdraw (amount=i128::MAX)
 12. deposit_withdrawn: for each asset, received=Δ since before_withdraw;
       if received > 0 push (hub, received); process_deposit(controller → pool)

finalize
 13. strategy_finalize: restamp LTV → require_post_pool_risk_gates → persist Both
 14. BlendMigrationEvent (counts only; not an accounting SoT)
```

Order is load-bearing:

- **Debt before sweep** — Blend (and MockBlend) reject collateral withdrawal while liabilities remain; clearing Blend debt first is required for a successful sweep.
- **Leftover repay before sweep snapshot** — `before_withdraw` is taken *after* reconcile, so debt-leg residue is not mis-credited as supply.
- **Baselines before external calls** — pre-existing controller balances sit inside the baseline and cannot be swept into the migrator’s positions (`endpoints.md` unlisted/stranded discipline; harness preexisting test).

---

## 2. Debt mint into controller (`charge_fee = false`)

Each `debt_caps` entry calls `borrow_into_controller` with `charge_fee: false` and `PositionAction::Migrate`.

| Property | Enforcement |
|---|---|
| Positive cap | `require_positive_amount` on each `max` |
| Unique debt assets | `require_unique_debt_assets` → `#7 AssetsAreTheSame` |
| Entry gates | `validate_position_entry_gates` (borrowable, not frozen entry, spoke borrow cap, …) |
| Custody measure | snapshot → `pool_create_strategy_call` under flash guard → `measured == amount_received` and `measured > 0` |
| Fee | `false` so cash available for Blend equals minted principal (no flashloan_fee skim that would leave Blend repay short vs auth cap) |
| Persistence | merge only; durable write at `strategy_finalize` (A032) |

If the pool delivered less than `max` while auth still allowed Blend to pull `max`, the subsequent Blend `transfer(controller → pool, max)` would fail closed (insufficient balance). Happy path: measured == max.

Temporary state after the borrow loop and before leftover repay: hub debt equals **sum of caps**, not Blend liability. That is intentional headroom; step 9 collapses it.

---

## 3. Blend repay pulls (capped auth)

```67:118:contracts/controller/src/external/blend.rs
// blend_repay_all: REQ_REPAY with amount=max per debt_caps
// authorize_repay_pulls: InvokerContractAuthEntry transfer(controller, blend_pool, max)
// guarded_submit: with_flash_guard; submit(from=caller, spender=controller, to=controller)
```

Defenses:

1. **Allowlist** — `validate_migration_request` → `is_blend_pool_approved` (`#42`) before any borrow (INV-STRAT-03).
2. **Pull cap** — auth amount equals each `max`; Blend cannot pull more than the caller declared.
3. **Flash guard** — `guarded_submit` holds INV-FLASH-02 across the cross-contract `submit` so a Blend hook cannot reenter monetary controller verbs mid-call (A007).
4. **Caller-keyed Blend book** — `from = caller`; hub positions attach to `account_id`. A003 notes a delegate can migrate *their* Blend book onto the owner’s account — still INV-AUTH-02 gated, not a stranger vector.

MockBlend behavior (harness model of over-cap repay): pulls full `req.amount`, applies `min(amount, liability)`, refunds excess to `to` (controller). Real Blend may only pull `min(cap, liability)`. **Net controller Δ after repay is the same class**: borrowed − consumed_by_Blend ≥ 0 under honest pull-≤-auth semantics.

Hostile approved pool that consumes the full pull without clearing a matching liability (or without refunding) is the documented trust residual: migrator may end over-indebted on the hub or fail HF and revert the whole tx. Governance must treat Blend approval as high trust (threat-model: approved pool can later upgrade).

---

## 4. Leftover repay (core of this scope)

```247:286:contracts/controller/src/strategies/migrate_blend.rs
// reconcile_debt_refunds: refund = balance_delta_since(before_debt);
// if refund > 0 → repay_debt_from_controller(..., debt_available: refund, ...)
```

### 4.1 What “leftover” means

| Scenario | Controller Δ after Blend repay (vs `before_debt`) | Hub debt after reconcile |
|---|---|---|
| Cap > Blend liability | `cap − liability` (excess) | ≈ liability |
| Cap == liability | `0` (no refund path) | ≈ liability |
| Cap > 0, Blend liability = 0 | `cap` (full refund) | `0` (net flat) |
| Cap < liability | Blend health/repay fails → **tx reverts** | unchanged |

Leftover is **not** sent to `caller` as free tokens. It is repaid into the just-minted hub debt via the shared strategy repay leg (contrast swap leftover → `refund_to`). That is what makes `prop_migrate_blend_reconciles_same_asset` assert hub debt ≈ Blend liability **not the cap**.

### 4.2 Measured repay mechanics

`repay_debt_from_controller`:

1. `transfer_amount_measured(controller → pool, refund)` — credited burn amount is pool receipt (INV-ACCT-03).
2. `execute_repayment` with in-memory `DebtPosition` loaded after the migrate borrows.
3. Snapshot controller balance **after** the transfer / **before** pool repay mutation; any pool overpay increase is `transfer`’d to `caller` via `refund_controller_balance_delta` (caller residue only; not a gross sweep).

`refund > 0` gate ignores non-positive Δ (no attempt to repay from baseline or from a balance drop).

### 4.3 Pre-existing controller balances

`before_debt` includes any stuck inventory. After borrow+Blend+reconcile, Δ-based leftover equals only migration residue; stuck inventory remains. Harness: `test_migrate_refund_ignores_preexisting_controller_balance` (stuck ETH unchanged; debt still reconciles to ~0.5).

Same discipline as flash_position / swap: **no path transfers the controller’s gross balance** (`endpoints.md`).

### 4.4 Same-asset debt+collateral loop

`test_migrate_same_asset_loop` and fuzz `prop_migrate_blend_reconciles_same_asset`: USDC as Blend liability and collateral.

Sequence still separates phases: leftover repay restores controller USDC to baseline *before* `before_withdraw`; sweep Δ alone becomes hub supply. Hub debt ≈ 400 not 500 cap; supply ≈ collateral swept. No double-credit of leftover as both debt burn and supply.

### 4.5 Multi-debt

`reconcile_debt_refunds` iterates each `debt_caps` asset independently with its own baseline entry (from `snapshot_balances`, which de-dupes keys). `test_migrate_multi_debt_refund_reconciles_each` covers ETH+USDC caps above liability.

### 4.6 Exact cap / zero liability

- `test_migrate_debt_cap_exact_no_refund` — refund path skipped; controller debt-token balance 0; hub debt ≈ liability.
- `test_migrate_zero_blend_liability_cap_nets_zero_debt` — full leftover repay nets zero hub debt; collateral sweep still lands.

### 4.7 Cap too low

`test_migrate_debt_cap_too_low_reverts` / fuzz `prop_migrate_blend_cap_too_low_reverts`: MockBlend `#1 HealthCheckFailed` when collateral withdraw would leave liability; controller leftover 0; Blend positions intact. No partial-repay-then-sweep hole under atomicity.

---

## 5. Sweep and measured deposit

```215:245:contracts/controller/src/strategies/migrate_blend.rs
// deposit_withdrawn: received = balance_delta_since(before); only received > 0 deposited
```

| Step | Defense |
|---|---|
| Pre-check | `require_can_supply` for every unique withdraw asset **before** Blend funds move |
| Sweep amounts | `i128::MAX` per listed collateral/supply asset (full exit of that Blend slot kind) |
| Guard | `guarded_submit` flash window |
| Credit | Only positive controller Δ since `before_withdraw`; zero Blend balance → no deposit (`test_migrate_ignores_listed_asset_with_zero_blend_balance`) |
| Pool push | `process_deposit` with `caller = controller`; `transfer_amount_measured` into pool; shares follow pool mutation (A041) |

Collateral and supply kinds share one deduped deposit list (`push_unique_address`) but remain separate Blend request types — correct for Blend’s split position maps.

Under-delivery by an approved Blend pool → fewer hub shares (migrator loss) or HF revert. Over-delivery → extra shares to the migrator from Blend’s own balances (approved-pool trust), not from other users’ controller baselines.

---

## 6. Finalization and risk

`strategy_finalize` restamps listed supply LTV, runs `require_post_pool_risk_gates` (solvency / HF), persists both sides with `remove_if_empty` (A032 / A072).

`test_migrate_unhealthy_end_state_reverts` — insufficient migrated collateral vs debt → `#100`; atomic rollback.

Empty account edge: zero-balance listed withdraw can create-then-empty an account shell depending on cleanup flags; harness asserts `!account_exists` when nothing was deposited — not a money-flow theft issue (A004/A036 territory).

---

## 7. Auth, pause, and reentrancy (money-flow relevant)

| Control | Where |
|---|---|
| `#[when_not_paused]` | `lib.rs` entrypoint |
| `require_authorized_caller` | `process_migrate_blend:44` before Cache/account work |
| `AccountGuard::Migrate` | owner/delegate or self-create |
| Flash on Blend submit | `guarded_submit` |
| Flash on pool borrow | `borrow_into_controller` |
| Post-guard leftover repay / deposit | flag clear — A007 residual under listing trust |

Adversarial: `test_migrate_blend_submit_hook_cannot_reenter`; flash-loan receiver `ReenterMigrateBlend`. Matrix coverage gap for migrate noted in A007 — code-gated, test-incomplete.

---

## 8. Attack / failure matrix

| Attempt | Outcome |
|---|---|
| Unapproved Blend pool | `#42` before borrow |
| Empty request | `#16 InvalidPayments` |
| Duplicate debt asset | `#7` |
| Zero cap | `# AmountMustBePositive` |
| Unlisted / non-supplyable withdraw asset | oracle/spoke errors before Blend |
| Cap < Blend liability | Blend health fail; full revert; no hub leftover |
| Cap > liability | Leftover repaid; hub debt ≈ liability |
| Steal stuck controller inventory via Δ | Impossible — baselines include stuck |
| Treat leftover as caller cash refund | Not implemented; would inflate hub debt — correctly avoided |
| Claim Blend-reported amounts without Δ | Not used; deposits/refunds are balance Δ only |
| Reenter during Blend submit | Flash guard → `#400` |
| Hostile approved Blend consumes pull | Migrator self-harm and/or HF revert; protocol cash book still consistent with pool mutations |
| Fee-on-transfer / lying debt or coll token | Listing trust (A055); measurement still prevents unmeasured share mint on controller→pool legs |

---

## 9. Test evidence map

| Concern | Evidence |
|---|---|
| Collateral / supply only | `test_migrate_collateral_only`, `test_migrate_supply_only` |
| Debt+collateral reconcile to liability not cap | `test_migrate_debt_and_collateral` |
| Same-asset loop | `test_migrate_same_asset_loop`; fuzz `prop_migrate_blend_reconciles_same_asset` |
| Exact cap / zero liability / multi-debt | `test_migrate_debt_cap_exact_no_refund`, `…_zero_blend_liability_…`, `…_multi_debt_refund_…` |
| Preexisting controller balance | `test_migrate_refund_ignores_preexisting_controller_balance` |
| Cap too low / unhealthy / unapproved | corresponding `test_migrate_*_reverts` + fuzz |
| Controller dust hygiene | fuzz `assert_hygiene` leftover ≤ 4; exact-cap balance 0 |
| Reentrancy | `strategy/adversarial.rs::test_migrate_blend_submit_hook_cannot_reenter` |
| Unit validators | `process_migrate_blend_rejects_empty_request`, `…_unapproved_pool` |

---

## 10. Peer agreement / non-claims

- **Agrees with A003** — Blend `from=caller` vs hub `account_id` is intentional delegate economics, not an auth hole.
- **Agrees with A007** — Blend submit is guarded; leftover repay / deposit sit in the post-guard listed-token residual class.
- **Agrees with A032 / A041 / A047** — in-memory merge then finalize; measure at controller custody; leftover on migrate is debt-burn, unlike swap `token_in` refund.
- **Agrees with A045** — `charge_fee=false` + measured custody; migrate’s confinement is allowlisted Blend + leftover repay + HF, not INV-STRAT-04’s “must stay open”.
- **Does not claim** a novel critical gap against INV-STRAT-03 beyond the documented approved-pool upgrade residual.

---

## 11. Residuals (accepted / tracking)

1. **Approved Blend trust (Tamper.8 residual Low)** — governance allowlist; pool owner upgrade can change behavior until approval revoked.
2. **Post-guard transfer hooks** — leftover repay and deposit (A007/A055).
3. **debt_caps sizing** — temporary full-cap borrow can hit utilization / spoke borrow caps; callers should set modest buffers (fuzz uses up to +50% and still expects reconcile).
4. **Request-list hygiene** — debt uniqueness enforced; collateral/supply duplicate entries not rejected (sweep no-ops after empty).
5. **Test matrix** — extend `reentrancy_matrix` for `migrate_from_blend` (A007 tracking).

No production code change recommended from this audit alone. If remediation is later scoped: prefer documentation/runbook guidance on `debt_caps` buffers and matrix coverage over changing leftover-repay into a caller transfer.
