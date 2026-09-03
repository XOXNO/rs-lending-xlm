# A049 — Repay-with-collateral: same-market netting vs cross-market swap

- Agent: A049
- Theme: T3 (money movement), T4 (input validation on branch predicate / swap payload)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs:313–339` (`repay_debt_with_collateral` entrypoint)
  - `contracts/controller/src/strategies/repay_debt_with_collateral.rs` (orchestrator)
  - `contracts/controller/src/strategies/legs.rs:49–89,154–218` (`repay_debt_from_controller`, `net_settle_collateral_against_debt`)
  - `contracts/controller/src/strategies/mod.rs:71–111` (`strategy_finalize`, `withdraw_and_swap_from_supply`)
  - `contracts/controller/src/strategies/swap.rs:61–78` (`swap_tokens_or_passthrough`)
  - `contracts/pool/src/ops/net_settle.rs` + `common/src/rates/scaling.rs:131–159` (`resolve_net_settle`)
  - `common/src/types/pool.rs:462–468` (`HubAssetKey` Eq)
- Defense: Branch on full `HubAssetKey` equality (same market). Same-market path forbids any swap bytes and calls pool `net_settle` (no cash / no token movement). Cross-market path fail-fast checks debt, withdraws under flash guard, swaps or passthroughs by **asset address**, then measured controller→pool repay with excess refund. Both paths share `strategy_finalize` solvency + persist.
- Gap: (1) Entrypoint docstring says “when the assets match” but the predicate is market identity (`hub_id`+`asset`), not `Address` alone — cross-hub same-token correctly uses withdraw+passthrough+repay. (2) No harness case for same-market + non-empty swap rejection, nor for cross-hub same-token `repay_debt_with_collateral`. (3) Shared residuals: A080 spoke-exit no-op; A007 flash-guard absence on the repay transfer leg (withdraw leg is guarded). (4) Pool allows zero-settle no-op when conservative floors are 0 (dust); controller does not re-assert `settled_amount > 0`.
- Impact: No fund-control hole found. Wrong branch selection cannot mint debt, move foreign collateral, or bypass router emptiness checks. Same-market netting cannot drain pool cash (cash unchanged). Cross-hub same-token unwind moves cash hub→hub only as a normal withdraw+repay composition under the user’s own positions. Blast radius if the branch were wrong would be market-local cash / share desync or forced router trust on a no-swap path — neither is present.
- Evidence: INV-AUTH-02 (A003), INV-HALT-01 pause gate (A001), pool `net_settle_*` Certora suite, controller `net_settle_pivot_never_leaves_zero_scaled_records` / `usage_strategy_net_settle_tracks_scaled_delta`, harness `strategy/happy.rs` + `strategy/edge/rejections.rs` same-token suite, `docs/reference/endpoints.md:383–390`, peers A025/A032/A041/A055/A072/A080/A082.
- Opinion: The netting-vs-swap split is the right money-path design. Keep the predicate on `HubAssetKey`, keep empty-swap fail-closed on both same-market and same-`Address` passthrough, and treat amount semantics asymmetry (settle-cap vs withdraw-then-refund) as intentional rather than a bug.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git ops; no production Rust edits).
2. Trace `Controller::repay_debt_with_collateral` → `process_repay_debt_with_collateral` → branch → `close_remaining_collateral_if_requested` → `strategy_finalize`.
3. Compare money, cash, rounding, events, spoke usage, and trust boundaries for:
   - same `HubAssetKey` (net settle),
   - different `HubAssetKey` / different `Address` (withdraw + router swap + repay),
   - different `HubAssetKey` / same `Address` (withdraw + passthrough + repay).
4. Cross-check pool `net_settle`, `resolve_net_settle`, swap emptiness gates, auth/pause/flags, tests, Certora, and peer findings A003/A007/A025/A032/A041/A055/A072/A080/A082.
5. Out of scope as primary claims: aggregator min-out depth (A056), lying-token listing policy beyond noting measured repay (A055), bare `repay` storage shape (A025), bad-debt same-market redesign ADR-0021 (socialization path, not this strategy).

---

## Verdict

**Defended.** Same-market repay-with-collateral never touches the router and never moves tokens; cross-market repay always withdraws first, never nets across hubs, and only passthroughs when token addresses already match with an empty swap. Post-path risk gates and atomic tx rollback close mid-leg failure modes. Residuals are documentation precision and test/Certora coverage skew, not an undefended money bug.

---

## 1. Entrypoint surface

| Gate | Where | Effect |
|---|---|---|
| `#[when_not_paused]` | `lib.rs:317` | Global pause blocks strategy unwind (intentional; bare `withdraw`/`repay` remain — A001 N2) |
| `require_authorized_caller` | `repay_debt_with_collateral.rs:48` | Caller auth + flash-loan reentrancy gate (A007) |
| `require_positive_amount(collateral_amount)` | `:50` | Zero/negative rejected before any position load |
| `require_hub_active` on **both** hubs | `:51–52` | Inactive hub cannot be used on either leg |
| `require_owner_or_delegate` | `:55` | Stranger cannot force-close / net foreign accounts (A003) |
| `Cache::new` + `prefetch_strategy_prices` | `:56–59` | Instance TTL renew + oracle prefetch for finalize HF |

Permissionless surface: not listed — owner/delegate only. Pause-gated strategy verb.

---

## 2. Branch predicate (load-bearing)

```61:84:contracts/controller/src/strategies/repay_debt_with_collateral.rs
    if collateral == debt {
        // Same market on both legs: net supply against debt on the pool with no
        // tokens moving, so no conversion is needed or allowed.
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        net_settle_collateral_against_debt(
            ...
            events::PositionAction::RpColNet,
        );
    } else {
        repay_via_collateral_swap(...);
    }
```

`HubAssetKey` is `Eq` on `(hub_id, asset)` (`common/src/types/pool.rs:462–468`). Therefore:

| Caller keys | Branch | Swap rule |
|---|---|---|
| Identical hub+asset | **Net settle** | Must be empty (`InvalidPayments` otherwise) |
| Different hubs, same `Address` | **Swap path** | Empty → `swap_tokens_or_passthrough` passthrough; non-empty → `InvalidPayments` at passthrough |
| Different `Address` | **Swap path** | Non-empty required (`swap_tokens` asserts `!swap.is_empty()`) |

This matches `endpoints.md` (“same **market** … netted directly”) more precisely than the entrypoint rustdoc (“when the **assets** match”). The code is correct; the rustdoc is the soft gap.

Cross-hub same-token **must not** net-settle: hubs are separate cash/index books. Withdraw from collateral hub + repay into debt hub is the only sound unwind (see GH-21 loop in `strategy/cross_hub_same_asset_loop.rs`, which today unwinds via bare `repay`+`withdraw`, not this verb).

---

## 3. Same-market netting path (no tokens)

### 3.1 Controller leg

`net_settle_collateral_against_debt` (`legs.rs:154–218`):

1. `FreezePolicy::AllowOnExit` on the single listing (paused blocks; frozen allowed).
2. Panic if supply or debt row missing (`get_*_position_or_panic`).
3. Owner-only pool `net_settle` with caller’s current scaled rows + requested amount.
4. Merge **both** sides from pool result: `merge_withdraw_leg` then `merge_debt_leg(Exit)`, same `settled_amount`, same `market_index`, action `RpColNet`.
5. Spoke usage: supply exit + borrow exit from scaled deltas (Certora `usage_strategy_net_settle_tracks_scaled_delta`).
6. Zero-scaled records removed via `update_or_remove_*_position` (Certora pivot rule intent).

No `token::transfer`, no router, no controller custody, no flash-guard wrap (nothing for a transfer hook to hitch onto).

### 3.2 Pool arithmetic

`resolve_net_settle`: settle = `min(requested, floor(supply), ceil(debt))`; burns directed (`ceil` supply / `floor` debt) with full-side close only when the conservative value is exhausted (`common/src/rates/scaling.rs:131–159`). Positive settle that would burn zero shares on either side → `NetSettleRoundsToZeroShares`. Cash and utilization: pool README proves healthy markets do not raise utilization via net settle; `require_solvent_withdraw_state` still runs; **no utilization max gate** (documented intentional).

Cash invariance is harness-pinned: `test_repay_debt_with_collateral_same_token_succeeds_at_zero_cash` — succeeds when idle cash &lt; settle amount; cash unchanged.

### 3.3 Amount / excess semantics

Requested `collateral_amount` is a **settle cap**, not a withdraw size. Excess over `min(supply_floor, debt_ceil)` stays as supply (`test_repay_debt_with_collateral_same_token_leaves_excess_as_supply`). Unlike the swap path, unused collateral is **not** sent to the caller.

### 3.4 Dust / zero-settle residual

If both positions exist but conservative floors yield `settle <= 0`, pool returns zero burns (allowed). Controller merges no-ops and still finalizes. User pays fees for a no-op; no share inflation / cash theft. Severity: info coverage/UX, not fund loss.

---

## 4. Cross-market swap path (tokens move)

### 4.1 Order of operations

`repay_via_collateral_swap` (`repay_debt_with_collateral.rs:95–132`):

1. **Fail-fast** `get_debt_position_or_panic` before any collateral exit (missing debt → no withdraw; harness `test_repay_debt_with_collateral_missing_debt_rejects`).
2. `withdraw_and_swap_from_supply` → measured withdraw into controller under `with_flash_guard` (`legs.rs:103–115`) → `swap_tokens_or_passthrough`.
3. `repay_debt_from_controller`: `transfer_amount_measured` controller→pool, `execute_repayment` with **received** amount, refund only post-repay controller balance **increase** to caller (pre-existing controller inventory not swept — `test_repay_debt_with_collateral_refund_only_uses_repay_excess`).

Events: withdraw `RpColWd`, repay `RpColR` (distinct from netting’s single `RpColNet`).

### 4.2 Passthrough vs router

```64:78:contracts/controller/src/strategies/swap.rs
pub(crate) fn swap_tokens_or_passthrough(...) -> i128 {
    if token_in == token_out {
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        amount_in
    } else {
        swap_tokens(...)  // requires !swap.is_empty()
    }
}
```

Same-`Address` cross-hub: empty swap passthrough (no DEX). Different assets: empty swap rejected (`test_repay_debt_with_collateral_non_same_token_empty_swap_rejects`). Router underspend refunded (`strategy/router.rs`). Reentering router blocked by flash guard (adversarial suite).

### 4.3 Amount / excess semantics (contrast with netting)

Withdraw amount is a **collateral exit size**. Oversized / `i128::MAX` resolves toward full supply (`resolve_withdrawal` when `amount >= current_supply_actual`), swaps, repays, refunds debt-asset excess (`extreme_amount_inputs.rs`). That is the dual of netting’s settle-cap: here unused value leaves as token refund, not residual supply.

Swap path **requires pool cash** on the collateral market (real withdraw). Netting does not. Choosing the wrong keys (cross-hub when same-hub positions exist) cannot “steal” via netting; it can only force a cash-dependent unwind.

### 4.4 Measurement

Repay burns follow measured pool receipt (A041/A082 pattern). Fee-on-transfer / lying tokens remain a listing-trust residual (A055), not a missing measure on this custody leg.

---

## 5. Shared tail

| Step | Behavior |
|---|---|
| `close_remaining_collateral_if_requested` | If `close_position`: require `borrow_positions.is_empty()` (`CannotCloseWithRemainingDebt`), then `execute_withdraw_all` → caller. Residual paused collateral blocks close (`pause_bypass.rs`). |
| `strategy_finalize` | Restamp listed supply LTV → `require_post_pool_risk_gates` (HF / LTV / min collateral) → `finalize_position_flow(..., Both, true)` (A032/A072). Debt-free accounts skip HF. Empty account cleanup on persist. |

Soroban tx atomicity: mid-leg failure after pool mutate reverts pool + controller (A032). No durable half-state.

HF after same-asset netting typically improves (remove full-price debt vs LTV-weighted collateral at one oracle price). Cross-asset HF regressions revert (`test_repay_debt_with_collateral_health_factor_guard`).

Risk-param refresh on residual supply after net settle: `test_repay_debt_with_collateral_same_token_refreshes_risk_params`.

---

## 6. Flags, pause, frozen

| Condition | Net settle | Swap path |
|---|---|---|
| Listing `paused` | Blocked (`AllowOnExit`) | Withdraw and/or repay blocked |
| Listing `frozen` | Allowed (exit) | Allowed on exits |
| Hub inactive | Blocked at entry (both hub ids) | Same |
| Global pause | Blocked by attribute | Same |

Paused **debt** on cross-asset path: harness `test_repay_debt_with_collateral_paused_debt_reverts`. Same-market pause inherits the single-listing exit flag (one check covers both books).

---

## 7. Comparison matrix

| Property | Same `HubAssetKey` | Diff hub, same `Address` | Diff `Address` |
|---|---|---|---|
| Branch | Net settle | Withdraw+passthrough+repay | Withdraw+swap+repay |
| Tokens move | No | Yes (hub A cash → controller → hub B) | Yes (+ router) |
| Needs collateral-hub cash | No | Yes | Yes |
| Swap bytes | Must be empty | Must be empty | Must be non-empty |
| Amount meaning | Settle cap | Withdraw size | Withdraw size |
| Excess disposition | Remains supply | Debt-asset refund after repay | Same + router in-underspend refund |
| Events | `RpColNet` (both legs) | `RpColWd` + `RpColR` | Same |
| Flash guard | N/A (no transfer) | Withdraw yes; repay transfer no (shared A007 note) | Same |

---

## 8. Evidence inventory

**Harness (representative):**

- Same-token netting / cash / excess / risk refresh: `tests/test-harness/tests/strategy/happy.rs` (`test_repay_debt_with_collateral_same_token_*`, `test_same_token_net_settle_*`).
- Empty vs non-empty swap gates, HF, close, auth: `strategy/edge/rejections.rs`.
- Pause: `strategy/edge/pause_bypass.rs`.
- Router refund / missing legs: `strategy/router.rs`.
- `i128::MAX` withdraw-all on swap path: `strategy/extreme_amount_inputs.rs`.
- Reentrancy via router: `strategy/adversarial.rs` (`test_rdwc_router_reenter_*`).

**Coverage holes (not runtime holes):**

- Same-market + **non-empty** swap → expect `InvalidPayments` (code-clear; no dedicated test).
- Cross-hub same-token `repay_debt_with_collateral` with empty swap (loop unwind via this verb).
- Same-market `i128::MAX` settle-cap (math caps; untested at strategy layer).

**Certora:**

- Pool: `net_settle_keeps_revenue_backed`, `net_settle_never_persists_supply_drained_with_debt`, `net_settle_conserves_cash_and_both_scaled_totals`, additivity rule.
- Controller: `usage_strategy_net_settle_tracks_scaled_delta`, `net_settle_pivot_never_leaves_zero_scaled_records`, `repay_with_collateral_never_increases_positions` (cross-token assumes `collateral_token != debt_token`).
- Note: pivot rule seeds `nonempty_strategy_swap()` (= `nondet_bytes1()`). Successful traces require empty bytes; nonempty same-market calls hit `InvalidPayments`. Rule is not a substitute for an explicit emptiness assert rule.

**Docs / STRIDE:** `endpoints.md` same-market language; events `RpColWd`/`RpColR`/`RpColNet`; errors `NetSettleRoundsToZeroShares`; STRIDE I8 strategies auth; pool README net_settle cash-free design.

**Peers:** Agree with A003 (owner gate), A001 (pause), A032 (finalize batch), A041/A082 (measured repay), A072 (post-pool risk). Inherit A080 (spoke exit missing-row no-op) and A007 (unguarded repay transfer) as shared residuals — not introduced by the branch split.

---

## 9. Attack / misuse sketches (closed)

| Sketch | Outcome |
|---|---|
| Stranger nets Alice’s same-asset books | `require_owner_or_delegate` → reject (A003) |
| Same-market with junk swap to force router | `swap.is_empty()` assert → `InvalidPayments` |
| Cross-asset with empty swap | `swap_tokens` emptiness assert → `InvalidPayments` |
| Cross-hub same-token with nonempty swap | Passthrough emptiness assert → `InvalidPayments` |
| Net settle to drain cash / bypass illiquidity | No cash movement; zero-cash test passes |
| Withdraw collateral then fail swap/repay | Whole tx reverts; positions restored |
| Inflate debt burn above tokens received | Measured `received` feeds repay |
| Sweep unrelated controller inventory on refund | Refund = delta since pre-repay snapshot only |
| `close_position` with leftover debt | `CannotCloseWithRemainingDebt` |
| Use netting across hubs by Address confusion | Predicate is `HubAssetKey`; cross-hub never nets |

---

## 10. Opinion / follow-ups (non-blocking)

1. Tighten `lib.rs` rustdoc to “same **market** (`HubAssetKey`)” to match code and `endpoints.md`.
2. Add harness: same-market nonempty swap reject; cross-hub same-token RDWC empty-swap unwind.
3. Optional: assert `settled_amount > 0` after net settle if product wants fail-closed on dust no-ops.
4. Do not “optimize” by branching on `asset` Address alone — that would incorrectly net distinct hub books or skip required cash movement.

**Status remains defended** for A049’s money-path scope.
