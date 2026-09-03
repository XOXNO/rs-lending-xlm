# A045 — Flash position: debt mint, collateral measure, refunds

- Agent: A045
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/strategies/flash_position.rs` (full: `process_flash_position`, `mint_and_forward`, `collect_collateral_deposits`, `refund_listed_assets`, `require_flash_position_still_open`, validators)
  - `contracts/controller/src/positions/debt.rs:248-298` (`borrow_into_controller`, `charge_fee = false`)
  - `contracts/controller/src/positions/supply.rs:106-155` (`process_deposit` / `settle_supply` measured pool push)
  - `contracts/controller/src/strategies/legs.rs:220-233` (`refund_controller_balance_delta`)
  - `contracts/controller/src/strategies/mod.rs:41-55,71-80` (`snapshot_balances`, `strategy_finalize`)
  - `contracts/controller/src/payments.rs:14-24` (`balance_delta_since`)
  - `common/src/token.rs:19-34` (`transfer_amount_measured`)
  - `contracts/pool/src/ops/strategy.rs` (`create_strategy` / `charge_fee`)
  - `contracts/controller/src/lib.rs:194-222` (entrypoint)
- Defense: Zero-fee strategy debt is minted only through the owner-gated pool, double-measured at the controller (pool report == controller balance delta), then forward-measured to the Wasm receiver. Collaterals are pre-validated, snapshotted inside the flash guard, credited only on post-callback measured increases that meet declared minima, then deposited via measured controller→pool transfers. Refunds move only positive deltas of pre-listed, non-collateral assets to `caller`. The account must keep live debt in the flashed asset plus at least one supply position through `strategy_finalize` (INV-STRAT-04). Undeclared leftovers are stranded and unstealable because every refund/deposit uses a pre-callback baseline.
- Gap: (1) Post-guard listed-token transfer hooks during `process_deposit` / refund (shared with A007 §5) — listing trust, not a measurement hole. (2) Accepted: zero origination fee vs `multiply`; dust `min_amount` on an already-healthy account ≈ `borrow(..., to)` (ADR-0020). (3) Accepted: no repay leg; returned debt tokens refund to caller. (4) Accepted: silent refunds / no admin sweep for stranded undeclared tokens. (5) `refund_assets` listing is keyed by `debt.hub_id` (Address-only list) — multi-hub listing asymmetry is a validation quirk (A070), not a fund-theft vector. (6) Refund leg uses raw `transfer` of measured excess, not `transfer_amount_measured` — FOT under-delivers to caller only; cannot inflate positions.
- Impact: A successful call leaves open, solvent strategy debt backed by measured collateral; pool cash and debt shares stay consistent with the mint. A malicious receiver cannot convert the path into a free cash flash loan, steal pre-existing controller balances, auto-repay by pushing the debt token, or credit undeclared assets as collateral. Blast radius of callback mischief is the caller’s own account solvency / their own pushed tokens. Market-wide loss requires a governance-listed non-SAC token bypassing measurement assumptions (A055), not a hole in this flow’s delta discipline.
- Evidence: INV-STRAT-04; INV-ACCT-03; INV-FLASH-02; ADR-0020; ADR-0013; STRIDE Spoof.4 / Tamper.5 / I8; threat-model “Flash position” + “pays no origination fee”; `docs/reference/endpoints.md` flash_position section; errors #401/#503/#504/#505; harness `strategy/flash_position.rs`, `flash_position_adversarial.rs`, `flash_position_mode_and_asset_edges.rs`; unit `contracts/controller/tests/strategies/flash_position.rs`; pool `test_create_strategy_fee_free_when_charge_fee_false`. Cross-ref A007, A018, A019, A030, A032, A041, A044, A055, A070, A082.
- Opinion: Money-movement core for `flash_position` is defended. Keep the triple chain — pool report equality → forward measure → collateral/refund baselines — intact. Do not add a same-call repay or a gross-balance sweep; both would break the “open position, measured delta only” confinement that justifies `charge_fee = false`.

## Method

1. Read `COORDINATION.md`, `SEED.md`, README finding format, ADR-0020, INV-STRAT-04 / INV-ACCT-03 / INV-FLASH-02, endpoints flash_position section, threat-model rows.
2. Traced `lib.rs::flash_position` → `process_flash_position` end-to-end: gates → mint/forward → snapshots → callback → deposit → refund → still-open → finalize → still-open → event.
3. Decomposed debt mint (`borrow_into_controller` + pool `create_strategy` with `charge_fee=false`), collateral measure (`collect_collateral_deposits` + `process_deposit`), refund (`refund_listed_assets` + `refund_controller_balance_delta`).
4. Cross-checked peer findings A007, A018, A019, A030, A032, A041, A044, A055, A082; harness + unit tests for FOT, refund, round-trip denial, still-open arms.
5. Searched for novel critical gaps (auto-repay, baseline theft, unmeasured credit, fee bypass of debt booking). None found beyond accepted ADR/threat-model residuals.

---

## 1. End-to-end money sequence

```
pre-callback (flash guard held)
  1. borrow_into_controller(debt, amount, charge_fee=false)
       pool mints debt for `amount`, sends amount_received (= amount) → controller
       merge_debt_leg from pool mutation (actual_amount = principal)
  2. assert controller Δ(debt.asset) == amount_received; Δ > 0
  3. transfer_amount_measured(controller → receiver, measured) → amount_received'
  4. snapshot controller balances for each collateral asset
  5. snapshot controller balances for each refund asset
  6. invoke execute_flash_position(..., amount, fee=0, amount_received', ...)

post-callback (guard cleared)
  7. for each collateral: Δ = balance - baseline; require Δ ≥ min; deposit Δ > 0
  8. process_deposit: measured controller → pool supply; merge supply legs
  9. for each refund asset: if Δ > 0, transfer Δ to caller
 10. require_flash_position_still_open (live debt in `debt` + ≥1 supply)
 11. strategy_finalize (restamp LTV, post-pool risk, persist, remove_if_empty)
 12. require_flash_position_still_open again
 13. FlashPositionEvent { amount, amount_received', fee: 0 }
```

Order is load-bearing: snapshots are taken **after** mint/forward and **before** the callback, so mint proceeds are not mistaken for collateral/refund deltas, and callback pushes are the only credited increases.

---

## 2. Debt mint (zero-fee strategy debt)

### 2.1 Pool side (`charge_fee = false`)

```58:101:contracts/pool/src/ops/strategy.rs
// mint_debt(amount); fee = 0 when !charge_fee;
// amount_to_send = amount - fee; debit_cash(amount_to_send);
// mutation: actual_amount = amount, amount_received = amount_to_send
```

With `charge_fee = false`, `amount_received == amount`; cash leaves the pool 1:1 with minted principal. Fee-charging multiply paths withhold `flashloan_fee` bps as revenue; flash position deliberately does not (ADR-0020; threat-model “pays no origination fee”). Confinement that justifies zero fee is INV-STRAT-04 (open position), not a same-call repay.

Controller also requires `is_flashloanable` on the debt market before mint (`flash_position.rs:90-94`) — caller-chosen receiver custody is exactly what that flag denies for cash flash loans; `multiply` stays ungated because funds only reach the governance router (ADR-0020).

### 2.2 Controller custody measure (defense in depth)

`mint_and_forward`:

| Step | Check | Failure |
|---|---|---|
| Snapshot controller debt-token balance | — | — |
| `borrow_into_controller(..., false, FlashPos)` | Inner measure == `result.amount_received`; `measured > 0`; entry gates; nested flash guard on pool transfer | `InternalError` / amount / gate errors |
| Outer `balance_delta_since` vs returned measured | `measured == reported` | `InternalError` |
| `transfer_amount_measured` to receiver | Recipient Δ returned; `forwarded > 0` | `AmountMustBePositive` |

Inner (`debt.rs:271-284`) and outer (`flash_position.rs:283-307`) both subtract a pre-borrow controller balance. Between the two snapshots only validation / position create runs — no token movement — so the equality is redundant but intentional (A082: do not remove). Debt shares follow pool `new_scaled` / `actual_amount`; tokens credited for forwarding follow `amount_received`. With fee=0 those coincide.

Fee-on-transfer debt: pool→controller short delivery fails the measure==report assert (adversarial `test_flash_position_fee_on_transfer_debt_fails_closed`). Controller→receiver FOT reduces `amount_received` in the event/callback; debt stays at full minted principal — borrower bears FOT, protocol books do not shrink.

### 2.3 What debt mint is not

- Not a flash loan: no principal pullback, no fee book (contrast A044 / INV-FLASH-01).
- Not auto-repay: returning the debt token to the controller never calls repay; if listed in `refund_assets`, it is refunded to `caller` at full minted size (`test_flash_position_returning_debt_token_does_not_repay`). Protocol revenue unchanged.
- Not unbound: borrow entry gates, spoke borrowable flags, caps, min-borrow collateral, and final solvency still apply (`strategy_finalize` → `require_post_pool_risk_gates`).

---

## 3. Collateral measure and deposit

### 3.1 Declaration = trust + work bound

`collaterals: Vec<(HubAssetKey, i128)>` — `i128` is a **slippage floor**, not a credit amount.

Pre-callback `validate_collaterals`:

- Non-empty; `len() ≤ max_supply_positions`
- `min_amount ≥ 0`; uniqueness on **underlying asset** (stronger than full `HubAssetKey`)
- `require_can_supply` (listing, collateralizable, halt flags)
- At least one `min_amount > 0` else `CollateralRequired` (#503)
- `validate_position_entry_gates` for Deposit (caps / position limits)

Undeclared pushes have no snapshot → never credited (INV-ACCT-03 discipline).

### 3.2 `collect_collateral_deposits`

```339:366:contracts/controller/src/strategies/flash_position.rs
// baseline from pre-callback map (missing key → InternalError)
// delta = balance_delta_since(controller, baseline)
// require delta >= min_amount (#504)
// push (hub_asset, delta) if delta > 0
// require deposits non-empty (#504)
```

| Receiver delivery | Result |
|---|---|
| Δ ≥ min, Δ > 0 | Full Δ deposited (over-delivery kept as collateral) |
| Δ < min | Revert #504 |
| Δ = 0 and min = 0 | Leg skipped |
| All legs empty after filter | Revert #504 |

Pre-existing controller inventory sits inside the baseline — cannot be stolen as “collateral credit” by a later caller (endpoints.md; same pattern as refunds). Mid-callback donations to a **declared** collateral increase Δ and are deposited — that is the intended push path.

### 3.3 Deposit settlement

`process_deposit(env, &controller, ...)` → `settle_supply` transfers each aggregated amount from the **controller** to the pool with `transfer_amount_measured`, then pool supply uses the **received** amount at the pool (A041). FOT on collateral:

- Receiver→controller: only net Δ is eligible (`test_flash_position_fee_on_transfer_collateral_credits_net` / `_misses_min`).
- Controller→pool: shares follow measured pool receipt; no unbacked share mint from requested Δ.

Same-asset borrow-then-supply is allowed (ADR-0020; `test_flash_position_same_asset_loop`); solvency is the gate.

### 3.4 Prices

`prefetch_strategy_prices` runs before the callback. Finalize uses the in-`Cache` snapshot (ADR-0020 “Prices used at finalize are the pre-callback snapshot”). Callback cannot refresh monetary state through controller mutators (flash guard). View mid-state during callback is an accepted composability residual (threat-model), not an accounting bug.

---

## 4. Refunds

### 4.1 Rules (`validate_refund_assets`)

| Rule | Effect |
|---|---|
| `len() ≤ max_supply_positions` | Gas/work bound |
| No duplicate Addresses | No double refund of same Δ |
| `require_listed_active_config(spoke, HubAssetKey { hub_id: debt.hub_id, asset })` | Post-guard `token::Client` only on governance-listed contracts |
| No overlap with any collateral `.asset` | Prevents deposit-then-refund of the same push |

Debt asset **may** appear in `refund_assets` (explicitly allowed). Overlap check is collateral-only.

Listing key uses **debt hub_id** because `refund_assets` carries bare `Address`. An asset listed only under another hub in the same spoke fails `AssetNotInSpoke`. That is stricter than “any spoke listing”; money-safe, possibly UX-surprising — track under A070 allowlist semantics, not as theft.

### 4.2 Execution

Snapshots taken inside the guard with collaterals. After deposit:

```388:400:contracts/controller/src/strategies/flash_position.rs
// refund_controller_balance_delta(asset, baseline, caller)
```

```220:233:contracts/controller/src/strategies/legs.rs
// excess = balance_delta_since(...); if excess > 0 { transfer(controller → refund_to, excess) }
```

Properties:

- **Delta only** — never gross controller balance; stranded prior inventory is inside baseline and unstealable.
- **Recipient is `caller`**, not receiver / owner-of-record (owner usually equals caller; delegates refund to the delegate address).
- **No repay** — debt-token refund restores caller wallet cash while debt shares remain (`test_flash_position_returning_debt_token_does_not_repay`).
- **Silent** — no refund field on `FlashPositionEvent`; only token `transfer` events (endpoints.md accepted).
- **Raw transfer** of excess — under FOT, caller receives less; positions already finalized on measured collateral path; no credit inflation.

Undeclared leftovers remain permanently stranded; no admin sweep by design (measured-delta discipline vs rescue primitive — endpoints.md).

### 4.3 Post-guard hook residual

Refund (and collateral `process_deposit`) run with the flash flag clear (A007 §5, A030). Mitigation: assets must be listed. Severity stays low under SAC/listing trust; would rise if a listed token gains arbitrary reentrant hooks racing unpersisted in-memory account state before `strategy_finalize` (A032).

---

## 5. Round-trip / closed-position denial (INV-STRAT-04)

`require_flash_position_still_open` (before and after `strategy_finalize`):

1. Account not empty and not debt-free
2. Borrow map has entry for **this** `debt` key
3. That entry `scaled_amount > 0` and `supply_positions` non-empty

Unit tests pin each arm (`contracts/controller/tests/strategies/flash_position.rs`). Harness: keep-funds without collateral fails; dust collateral fails solvency; push-debt-back without meeting collateral min fails #504; successful path leaves full ETH debt with zero fee.

`strategy_finalize` may drop zero-scaled supply via LTV restamp / `remove_if_empty` — second still-open check catches “finalize emptied the position.”

During the callback, monetary controller entrypoints are flash-blocked; pool strategy/repay is owner-only — receiver cannot clear controller-tracked debt except by failing later asserts.

---

## 6. Receiver and auth gates (money-adjacent)

| Gate | Money role |
|---|---|
| `require_authorized_caller` / `AccountGuard::Multiply` | Only owner/delegate (or self-create) opens leverage (A003, A018) |
| `require_wasm_receiver` | No EOA repayment games (A019, Spoof.4) |
| `receiver ∉ {controller, pool}` | Avoids stranded measurement / false Δ (A019, A055) |
| `#[when_not_paused]` | No new flash positions while halted |
| Outer `with_flash_guard` around forward+callback | INV-FLASH-02; nests with inner borrow guard (A007, A030) |

---

## 7. Evidence matrix

| Claim | Evidence |
|---|---|
| Zero-fee mint matches full cash send | pool `compute_fee` / `test_create_strategy_fee_free_when_charge_fee_false`; harness `test_flash_position_opens_healthy_account_without_fee` |
| Controller measures pool receipt | `borrow_into_controller` + `mint_and_forward` equality asserts |
| Forward is measured | `transfer_amount_measured`; event `amount_received` |
| Collateral credited = measured Δ ≥ min | `collect_collateral_deposits`; #503/#504 tests; FOT collateral tests |
| Deposit to pool re-measures | `settle_supply` + A041 |
| Refunds are positive deltas to caller | `refund_controller_balance_delta`; `test_flash_position_refunds_undeclared_push` |
| Debt return ≠ repay | `test_flash_position_returning_debt_token_does_not_repay`; revenue unchanged |
| Cannot close via round-trip | INV-STRAT-04; still-open unit arms; adversarial keep-funds |
| Unlisted refund rejected | `test_flash_position_rejects_unlisted_refund_asset` |
| Baseline theft impossible | endpoints.md stranded section; claim_revenue dust test pattern (A015) |
| `is_flashloanable` required | `flash_position.rs:90-94`; mode/asset edge tests |

---

## 8. Residuals (not novel critical gaps)

| Residual | Severity | Disposition |
|---|---|---|
| Post-guard deposit/refund transfer hooks | low | Shared A007; listing trust |
| Zero fee vs multiply | info | Threat-model accepted; `is_flashloanable` narrows substitute markets |
| Dust min on healthy account extracts debt to receiver | info | ADR-0020 = ordinary borrow |
| Silent refunds / stranded undeclared tokens | info | Deliberate; no sweep |
| Refund list keyed by `debt.hub_id` | info | A070 allowlist detail |
| Refund raw transfer (no recipient measure) | info | Cannot inflate protocol credit |
| Non-SAC / rebasing if listed | medium (A055) | Governance listing boundary |

---

## 9. Verdict

**Status: defended** for A045 scope (debt mint, collateral measure, refunds).

The path is a measured leverage open with an external callback, not a cash flash loan. Pool debt/cash consistency, controller double-measure on mint, measured forward, baseline-diff collateral credit, listed delta-only refunds, and dual still-open checks jointly enforce INV-STRAT-04 and INV-ACCT-03. No undefended fund-theft or unmeasured credit path was found in this surface.

Remediation from this audit alone: none required on production Rust. Optional later hygiene (not blockers): (a) hold flash guard through deposit+refund until finalize if listing trust is questioned (A007); (b) A070 may document debt-hub listing key for refunds; (c) keep equality asserts in `mint_and_forward` / `borrow_into_controller` (A082).
