# A048 — Swap collateral legs money flow

- Agent: A048
- Theme: T3
- Severity: medium (residual; primary path defended)
- Status: partial
- Paths: `contracts/controller/src/lib.rs:287-311`; `contracts/controller/src/strategies/swap_collateral.rs`; `contracts/controller/src/strategies/mod.rs:71-111` (`strategy_finalize`, `withdraw_and_swap_from_supply`); `contracts/controller/src/strategies/legs.rs:94-118` (`withdraw_collateral_to_controller`); `contracts/controller/src/strategies/swap.rs` (`swap_tokens`, `swap_tokens_or_passthrough`, `verify_router_output`); `contracts/controller/src/positions/supply.rs:106-155,243-301` (`process_deposit`, `settle_supply`, `execute_withdrawal`); `common/src/token.rs:19-34` (`transfer_amount_measured`); `contracts/controller/src/payments.rs:14-24` (`balance_delta_since`)
- Defense: Owner-or-delegate + pause + flash-entry gate → measured withdraw-to-controller → flash-guarded router with exact pull auth, overspend reject, leftover refund, measured `token_out` Δ → measured controller→pool deposit of that Δ → LTV restamp + post-pool solvency → single `strategy_finalize` persist. Same-asset cross-hub uses empty-swap passthrough (no router). Shares and spoke usage follow pool outcomes; swap return values are discarded.
- Gap: (1) Controller does not enforce `min_out` / slippage — only `received > 0`; aggregator-enforced `total_min_out` sits in the untrusted router trust root (threat-model known gap; INV-STRAT-02 residual). (2) Deposit leg runs with flash flag clear — listed-token transfer hooks can reenter monetary entrypoints mid-settlement (A007 residual). (3) Fee-on-transfer / lying listed tokens: legs measure Δ but do not require Δ == requested (A041/A055). (4) Router underspend leftover of *source* collateral is refunded to `caller` (wallet), not re-supplied — intentional; subsumed by delegate complete economic control (threat-model).
- Impact: Compromised or malicious route can convert nearly all withdrawn collateral value into router retention while leaving `>0` destination units and a still-healthy account — loss bounded by that account’s excess health / withdrawn leg notionals, not by protocol TVL. Protocol share/cash books stay consistent on measured legs. FOT/rebasing listed assets: user (or market, if cash book stressed) bears listing-trust loss; not a novel swap_collateral hole.
- Evidence: INV-STRAT-01, INV-STRAT-02, INV-AUTH-02, INV-RISK-01, INV-ACCT measured-receipt language; ADR-0011; Certora `swap_collateral_preserves_directional_bounds`, `swap_collateral_rejects_same_token`, `post_gate_swap_collateral_totals_are_final`, `swap_collateral_sanity`; harness `strategy/happy.rs`, `strategy/router.rs`, `strategy/edge/*`, `strategy/adversarial.rs`, `strategy/extreme_amount_inputs.rs`, `controller/multi_hub.rs::swap_collateral_migrates_collateral_across_hubs`, fuzz `prop_swap_collateral_conserves_position_delta`; peers A003, A007, A032, A041, A055, A072.
- Opinion: Money-flow defenses for swap_collateral are correctly layered for an untrusted router and SAC-like tokens. Do not treat the thin `swap_collateral.rs` wrapper as incomplete — custody measurement lives in shared helpers. Highest residual is the documented no-controller-slippage gap on the withdraw→swap leg; any fix belongs at controller `verify_router_output` (or an explicit `min_out` arg), not by trusting aggregator return values. Cross-hub same-asset passthrough is a first-class money path and is defended without the router.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only).
2. Trace `Controller::swap_collateral` → `process_swap_collateral` → `withdraw_and_swap_from_supply` → `process_deposit` → `strategy_finalize`.
3. For each cash/share leg, record: custody boundary, measurement primitive, flash-guard window, who receives residue, what the account books credit.
4. Cross-check INV-STRAT-01/02, threat-model router/slippage sections, peers A003/A007/A032/A041/A055/A072, Certora + harness coverage.
5. Explicitly separate novel swap_collateral money-flow claims from shared residuals already owned by peers.

Out of scope as primary claims: swap_debt (A047), repay_debt_with_collateral (A049), migrate leftover repay (A050), ordinary withdraw recipient hijack, pool-internal cash books, lying-token taxonomy depth (A055), strategy finalize batching mechanics beyond this verb’s call (A032).

---

## Call graph (money legs)

```
Controller::swap_collateral                 #[when_not_paused]  lib.rs:290-310
  └─ process_swap_collateral                swap_collateral.rs:30-80
       ├─ require_authorized_caller         auth + !flash_loaning
       ├─ current != new                    AssetsAreTheSame (HubAssetKey)
       ├─ require_hub_active(current)       source hub only (early)
       ├─ require_positive_amount(from_amount)
       ├─ get_account + require_owner_or_delegate
       ├─ Cache::new                        instance TTL renew
       ├─ require_can_supply(new)           dest hub active + listed +
       │                                    unpaused/unfrozen + collateralizable
       ├─ prefetch_strategy_prices          current + new + open positions
       ├─ withdraw_and_swap_from_supply     → swapped_amount (token_out Δ)
       │    ├─ get_supply_position_or_panic(current)
       │    ├─ withdraw_collateral_to_controller
       │    │    ├─ snapshot controller balance(current.asset)
       │    │    ├─ with_flash_guard {
       │    │    │    execute_withdrawal → pool_withdraw(to=controller)
       │    │    │    merge_withdraw_leg (SwColWd, memory)
       │    │    │  }
       │    │    └─ return balance_delta_since(controller, current.asset)
       │    └─ swap_tokens_or_passthrough(caller, current.asset → new.asset)
       │         ├─ same Address + empty swap → passthrough amount_in
       │         └─ else swap_tokens:
       │              ├─ snapshot in_before / out_before
       │              ├─ authorize_transfer_as_current(exact amount_in)
       │              ├─ with_flash_guard { router.execute_strategy }
       │              ├─ reject RouterOverspend; refund leftover → caller
       │              └─ verify_router_output: Δ_out > 0
       ├─ process_deposit(caller=controller, [(new, swapped_amount)])
       │    ├─ validate_position_entry_gates (limits + require_can_supply)
       │    └─ settle_supply                **flash flag clear**
       │         ├─ transfer_amount_measured(controller → pool, swapped_amount)
       │         ├─ pool_supply_call
       │         └─ merge_supply_leg (event action Supply; memory)
       └─ strategy_finalize
            ├─ restamp_listed_supply_ltv
            ├─ require_post_pool_risk_gates   skip if debt_free
            └─ finalize_position_flow(Both, remove_if_empty=true)
```

---

## Leg-by-leg money flow

### Leg 0 — Gates before cash moves

| Check | Role |
|---|---|
| `#[when_not_paused]` | Blocks rotation during global pause (bare withdraw/repay remain) |
| `require_authorized_caller` | Caller auth + INV-FLASH-02 entry ban |
| `current != new` | Rejects identical `HubAssetKey` (not merely identical asset Address) |
| `require_hub_active(current)` | Source market must be live |
| `require_positive_amount` | Rejects `from_amount <= 0` |
| `require_owner_or_delegate` | INV-AUTH-02 before any withdraw |
| `require_can_supply(new)` | Destination must be active hub, listed, entry-allowed, collateralizable |

No tokens move until after these gates. Preflight `require_can_supply(new)` is repeated inside `process_deposit` entry gates (TOCTOU-safe within one tx; flags cannot flip mid-invocation without nested entry, which flash/auth block during guarded windows).

### Leg 1 — Withdraw `current` → controller custody (measured)

`withdraw_collateral_to_controller`:

1. Snapshots controller’s `current.asset` balance.
2. Under `with_flash_guard`, calls `execute_withdrawal` with counterparty = controller and action `SwColWd`.
3. Pool pays the recipient (controller); `merge_withdraw_leg` updates in-memory scaled supply + spoke usage Exit from **pool mutation outputs**.
4. Returns `balance_delta_since` — **not** the requested `from_amount`, **not** the pool-reported payout alone as an unchecked trust.

Properties:

- Flash flag held for the pool withdraw + token transfer into controller (hooks during payout see flag set → monetary reentry blocked; A007).
- `i128::MAX` / oversized amounts resolve to full position at the pool (`extreme_amount_inputs::swap_collateral_with_i128_max_amount_means_all`).
- Zero/negative Δ after a “successful” withdraw fail closed on the next leg (`require_positive_amount` in `swap_tokens`, or `transfer_amount_measured` on deposit for passthrough of 0).
- No equality assert `Δ == from_amount` — FOT under-delivery shrinks the swap budget (user loss; protocol books follow pool burn + measured custody). Cross-ref A041/A055.

### Leg 2a — Cross-asset swap (router trust boundary)

`swap_tokens` (when `current.asset != new.asset`):

| Control | Mechanism |
|---|---|
| Exact pull auth | `authorize_transfer_as_current(..., amount_in)` — INV-STRAT-01 |
| Discard router return | `_ = router.execute_strategy(...)` |
| Overspend | `in_after <= in_before` and `actual_spent <= amount_in` → `RouterOverspend` |
| Residue | `leftover = amount_in - actual_spent` transferred to `refund_to` (= strategy `caller`) |
| Output | `verify_router_output`: controller `token_out` Δ must be `> 0` (`NoSwapOutput`) |
| Reentrancy | Entire router call under `with_flash_guard` |

Pinned: `strategy/router.rs::test_swap_collateral_refunds_router_underspend_to_caller` (half input refunded to Alice; controller ends at 0 USDC); adversarial router reentry tests.

**Critical residual (known):** `verify_router_output` does **not** check a minimum out. Slippage / `total_min_out` is only inside the aggregator payload. A malicious or upgraded router can pull full `amount_in` and return 1 unit of `token_out`. Post-gate allows any loss that leaves the account solvent (INV-RISK-01). Threat-model: “unbounded-loss path for in-flight strategies” relative to the swapped notional / excess HF. Severity for this agent: **medium** residual (documented; not a missing measurement bug).

Underspend does **not** steal to the router: unspent `token_in` returns to `caller`. That extracts value from the lending position to the caller’s wallet (tested). For owner = caller this is user intent; for delegate it is covered by “delegate has complete economic control.”

### Leg 2b — Same-asset passthrough (cross-hub migration)

When `current.asset == new.asset` but `current != new` (different `hub_id`):

- Requires **empty** `swap` (`InvalidPayments` otherwise).
- Returns `actual_withdrawn` unchanged — no router, no refund path.
- Tokens stay on controller and are deposited into the **new** hub listing.

Harness: `controller/multi_hub.rs::swap_collateral_migrates_collateral_across_hubs`. This is a real money path (withdraw + redeposit), not a dead branch. Destination still faces `require_can_supply` / caps / risk restamp for the new hub’s params (`edge/swap.rs::test_swap_collateral_applies_spoke_params_to_destination_position`).

Note: `docs/reference/endpoints.md` says “two assets must differ”; runtime equality is on `HubAssetKey`. Doc imprecision only — behavior is intentional and tested.

### Leg 3 — Deposit `new` from controller → pool (measured)

`process_deposit` with `caller = env.current_contract_address()`:

1. Re-runs entry gates (position limits + `require_can_supply`).
2. `transfer_amount_measured(new.asset, from=controller, to=pool, amount=swapped_amount)` — credits pool action with **observed** receipt Δ.
3. `pool_supply_call` → `merge_supply_leg` sets scaled shares from pool result; spoke usage Entry + cap check; risk FullTuple restamp on destination.
4. Buffered event action is `Supply` (not a distinct SwCol deposit tag); withdraw side already recorded `SwColWd`. Observational only (A033).

Flash flag is **clear** during this transfer + pool supply (same pattern as multiply after router — A007 §4.3). A malicious listed `new` token hook could reenter other `#[contractimpl]` monetary entrypoints while the destination merge is still buffered in memory. Same-tx atomicity + entry auth still apply; class is shared residual, not unique theft via swap_collateral’s measured Δ chain.

Controller does not leave intended `swapped_amount` stranded on success: the measured transfer pulls that budget into the pool. Pre-existing unrelated dust of `new.asset` on the controller (if any) is not swept — general dust hygiene, not this verb’s credit bug (only Δ from Leg 2 is deposited).

### Leg 4 — Solvency and persist

`strategy_finalize`:

- Restamps listed supply LTVs after the rotation (destination may have different LTV/LT).
- `require_post_pool_risk_gates`: if any debt, require LTV collateral ≥ debt, HF ≥ 1 WAD, optional min-borrow-collateral floor. Debt-free accounts skip (pinned: `test_swap_collateral_no_borrows` / `no_borrows_skip_hf`).
- `finalize_position_flow(..., PositionSides::Both, remove_if_empty=true)` — spoke usage persist → position maps → batch events (A032). Full close of `current` can free a supply slot before/with dest open (`test_swap_collateral_full_close_frees_slot_at_max_positions`).

Temporary under-collateralization between Leg 1 and Leg 3 exists only in memory; failure anywhere rolls the whole Soroban transaction (pool + controller).

---

## Conservation sketch (successful cross-asset path)

| Location | `current.asset` | `new.asset` | Account supply shares |
|---|---|---|---|
| Before | user in pool | — | `current` scaled S₀ |
| After Leg 1 | +Δ_w on controller; pool paid out | — | `current` → S₁ ≤ S₀ (pool) |
| After Leg 2 | leftover → caller; spent → router | +Δ_out on controller | unchanged |
| After Leg 3 | ~0 intended on controller | Δ_out moved to pool (measured) | `new` scaled ↑ from pool; usage Entry |
| After finalize | durable maps match memory | durable maps match memory | both sides persisted |

Fuzz property `prop_swap_collateral_conserves_position_delta`: successful stablecoin swaps debit/credit exact raw position amounts under honest router. Certora `swap_collateral_preserves_directional_bounds`: source scaled does not increase; dest scaled does not decrease on success.

---

## What cannot happen (defended)

1. **Stranger rotates another account’s collateral** — owner-or-delegate before withdraw (A003; `test_swap_collateral_wrong_account_owner`).
2. **Router pulls more than authorized `amount_in`** — scoped auth + overspend asserts (INV-STRAT-01).
3. **Router return value inflates credited deposit** — return discarded; deposit uses controller balance Δ then measured pool receipt.
4. **Zero-output swap credits destination** — `NoSwapOutput`.
5. **Same HubAssetKey “swap”** — `AssetsAreTheSame` (Certora + harness).
6. **Non-collateral / frozen / paused destination** — `require_can_supply` / entry flags (`test_swap_collateral_non_collateralizable`, `rejects_frozen_destination`, pause_bypass).
7. **Nested strategy/position reentry during withdraw or router** — flash guard + entry `require_not_flash_loaning`.
8. **Skipping post-trade HF with open debt** — `strategy_finalize` gates (Certora `post_gate_swap_collateral_totals_are_final`).
9. **Stranding underspent swap input on controller** — leftover refund path (router underspend test).

---

## Residuals and non-claims

| Residual | Owner / disposition |
|---|---|
| No controller `min_out` | Threat-model known gap; **primary A048 residual** |
| Post-guard token hook reentry on deposit | A007 / A055; shared strategy pattern |
| FOT / rebasing / credit-on-transfer listing | A041 / A055; measure-don’t-trust, no Δ==request |
| Leftover refund to delegate wallet | Intentional; threat-model delegate power |
| Event tag `Supply` on dest leg | A033 observational |
| Finalize batching / pool-before-persist | A032; tx atomicity OK |
| Source-only early `require_hub_active` | Dest still gated via `require_can_supply` → `require_hub_active` |

**No novel critical fund-theft bug** found in the measured withdraw → swap → deposit chain unique to `swap_collateral.rs`. The wrapper correctly composes shared custody primitives.

---

## Tests and formal evidence (swap_collateral-specific)

| Artifact | What it pins |
|---|---|
| `strategy/happy.rs::test_swap_collateral_replaces_supply` | Partial rotation + HF with debt |
| `strategy/happy.rs::test_swap_collateral_no_borrows` | Debt-free path |
| `strategy/router.rs::…refunds_router_underspend…` | Residue → caller; controller clean |
| `strategy/edge/swap.rs` | Dest params, merge/remove, pause, flash, non-collateral, frozen dest |
| `strategy/edge/rejections.rs` | Caps, position limits, spoke category, zero amount, auth, same token |
| `strategy/adversarial.rs` | Router reenter supply; transfer-hook reenter; empty swap under flag |
| `strategy/extreme_amount_inputs.rs` | `i128::MAX` → withdraw all |
| `controller/multi_hub.rs` | Same-asset cross-hub passthrough money path |
| Certora strategy-swap-collateral*.conf + health-post-gate | Directional bounds, same-token revert, post-gate finals |
| Fuzz `prop_swap_collateral_conserves_position_delta` | Exact position Δ under honest route |

---

## Opinion (actionable)

1. Keep “measure at controller custody boundary” on all three legs; do not credit `execute_strategy`’s returned amount.
2. If closing the slippage gap: add an explicit controller-side minimum on `verify_router_output` (or a dedicated `min_out` parameter) — do not promote aggregator-enforced `total_min_out` alone to a trust root.
3. Treat cross-hub passthrough as production money flow in reviews and docs (`endpoints.md` wording).
4. Listing governance remains the outer control for non-SAC tokens on both `current` and `new`.
