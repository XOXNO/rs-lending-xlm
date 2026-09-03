# A056 — Swap aggregator min-out / slippage defenses from controller

- Agent: A056
- Theme: T3 (money movement / router trust boundary)
- Severity: medium (documented trust-root residual; not a novel critical hole)
- Status: partial
- Paths:
  - `contracts/controller/src/strategies/swap.rs` (`swap_tokens`, `swap_tokens_or_passthrough`, `call_router_with_reentrancy_guard`, `verify_router_output`)
  - Callers: `strategies/multiply.rs` (primary + `convert_swap`), `strategies/swap_debt.rs`, `strategies/swap_collateral.rs` via `mod.rs::withdraw_and_swap_from_supply`, `strategies/repay_debt_with_collateral.rs`
  - Contrast (controller-enforced floor): `strategies/flash_position.rs` (`collect_collateral_deposits` / `min_amount`)
  - Aggregator-side floor (untrusted): `contracts/swap-aggregator/src/execute/mod.rs` (`total_min_out`), `program.rs` header `min_out` index
  - Types: `common/src/types/shared.rs` (`StrategySwap = Bytes`); interface `interfaces/swap-aggregator/src/lib.rs`
- Defense: Exact pull auth for `amount_in`; flash-guard around `execute_strategy`; discard router return value; reject input overspend / balance increase (`RouterOverspend`); refund unspent `token_in` to `refund_to`; measure controller `token_out` Δ and require `received > 0` (`NoSwapOutput`); empty payload rejected when a swap is required; same-asset passthrough requires empty payload; post-strategy `strategy_finalize` → `require_post_pool_risk_gates` (HF ≥ 1 WAD when debt remains). Honest aggregator additionally enforces payload `total_min_out > 0` and `total_out >= total_min_out` after fees.
- Gap: Controller never reads or enforces a quantitative minimum output. `verify_router_output` is only positivity. Real slippage/`total_min_out` lives inside opaque caller `Bytes` and is enforced only inside the swap aggregator — a declared untrusted trust root (threat-model known gap). No controller entrypoint takes an explicit `min_out` for strategy swaps (unlike `flash_position` collateral floors). Compromised / maliciously upgraded router, or a caller/quote that embeds `min_out = 1`, can settle dust `token_out` while keeping residual input value; loss sticks iff post-gate HF still holds.
- Impact: Per-invocation token theft from the controller is capped by authorized `amount_in` (cannot over-pull). Economic loss for an in-flight strategy can approach the full swapped notional of that leg whenever the account remains solvent after dust output — typically largest on `swap_collateral` (and multiply with spare HF / large initial payment). Protocol share/cash books stay consistent on measured legs; loss is account-local (caller/owner/delegate), not a silent pool-wide mint. Cross-ref unbounded-loss class in threat-model §“The controller does not bound slippage”.
- Evidence: INV-STRAT-01, INV-STRAT-02; ADR-0011; threat-model known gap; STRIDE Tamper.4 (partially overclaims “meet minimums” at controller); harness `strategy/router.rs` (OverPull / Refund / UnderPull / OutputShortfall); aggregator unit `execute_strategy` SlippageExceeded; peers A047/A048/A055/A007/A045; Certora directional bounds (not quantitative min-out).
- Opinion: Custody defenses in `swap.rs` correctly treat the router as untrusted for **amount claims and overspend**. They do **not** close quantitative slippage. Closing the gap requires a controller-side floor (explicit `min_out` arg or decoded-and-checked payload field enforced via measured Δ), not promoting aggregator `total_min_out` alone. Agree with A047/A048 residuals; this file owns the cross-caller inventory and the flash_position contrast.

## Scope

Audit **controller-side** min-out / slippage defenses around the swap-aggregator handoff in `strategies/swap.rs`, including every production caller of `swap_tokens` / `swap_tokens_or_passthrough`, how the opaque `StrategySwap` payload carries `total_min_out`, what the honest aggregator enforces, and how post-strategy solvency bounds residual loss.

Out of scope as primary claims: full money-flow of each strategy (A046–A049), lying-token taxonomy (A055), flash-guard lifecycle (A007), destination `to` hijack (A057), Bytes size limits (A069), aggregator venue math correctness, governance `set_swap_aggregator` (A009).

Method: source read of controller + aggregator execute path; docs of record (invariants, threat-model, ADR-0011, endpoints); peer findings A045/A047/A048/A055; harness adversarial router modes; Certora strategy rules skim.

---

## Verdict

**Partial.** Input-side and zero-output defenses are strong and tested. Quantitative slippage is **not** a controller defense: it is either (a) caller intent encoded in an opaque payload enforced by an untrusted contract, or (b) absent under router compromise. Post-pool HF ≥ 1 is a solvency floor, not a price/slippage floor. This matches the threat-model known gap; it is not a novel critical finding relative to SEED/peers.

---

## 1. Trust model (docs of record)

| Source | Claim |
|---|---|
| ADR-0011 | Router untrusted; balances authoritative; return value does not establish success; output must be demonstrably **positive**; solvency after strategies |
| INV-STRAT-01 | Exact pull auth; **discard** router return (`swap.rs:91`) |
| INV-STRAT-02 | Measured output, residue return, same risk gates as ordinary ops — **enforced as `received > 0`**, not min-out |
| Threat-model | Explicit known gap: “The controller does not bound slippage”; malicious router can return one unit and keep the rest; HF permits any loss that leaves the account healthy; treat router compromise as **unbounded-loss** for in-flight strategies |
| Threat-model | Router owner is an out-of-governance trust root (`upgrade`, `sweep_balance`, … immediate) |
| Endpoints.md | Controller treats aggregator as untrusted; balance-delta settlement |

**STRIDE note (docs drift):** Tamper.4 says output must be “positive **and meet minimums**.” At the **controller** boundary, only positivity is enforced. Minimums exist inside the aggregator payload/execute path. Treat STRIDE wording as overclaim relative to live `verify_router_output`; threat-model is accurate.

---

## 2. Controller surface: `swap.rs` (full defense inventory)

### 2.1 `swap_tokens` sequence

```15:58:contracts/controller/src/strategies/swap.rs
pub(crate) fn swap_tokens(...) -> i128 {
    require_positive_amount(env, amount_in);
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
    // snapshot in_before, out_before
    authorize_transfer_as_current(env, token_in, &controller, &router_addr, amount_in);
    call_router_with_reentrancy_guard(env, &router, amount_in, swap);
    // RouterOverspend: in_after <= in_before; actual_spent <= amount_in
    // leftover -> refund_to
    verify_router_output(env, token_out, out_before)
}
```

| Check | Error | Slippage relevance |
|---|---|---|
| `amount_in > 0` | amount validation | Sizes max pull, not min out |
| `!swap.is_empty()` | `InvalidPayments` (#16) | Forces a payload when assets differ; does **not** decode min-out |
| Snapshot both balances **before** external work | — | Baseline for spend + receipt |
| Exact `transfer(from=controller,to=router,amount_in)` auth | auth failure / OverPull | Caps input extraction |
| Flash guard around `execute_strategy` | `FlashLoanOngoing` on reentry | Reentrancy, not slippage |
| Discard `execute_strategy` return | — | Lies about out ignored |
| `in_after <= in_before` | `RouterOverspend` (#501) | Blocks gift-of-input / weird refund inflate |
| `actual_spent <= amount_in` | `RouterOverspend` | Double-check vs allowance |
| Leftover `amount_in - actual_spent` → `refund_to` | — | Underspend returns to caller wallet |
| `balance_delta_since(token_out) > 0` | `NoSwapOutput` (#502) | **Only** output floor |

### 2.2 `verify_router_output` — the entire controller “slippage” check

```98:106:contracts/controller/src/strategies/swap.rs
fn verify_router_output(env: &Env, token_out: &Address, balance_before: i128) -> i128 {
    let received = balance_delta_since(...);
    assert_with_error!(env, received > 0, StrategyError::NoSwapOutput);
    received
}
```

No comparison to quote, oracle fair value, BPS band, or payload field. Any positive integer of the **caller-selected** `token_out` address passes.

### 2.3 What the controller does *not* do

1. Decode `StrategyPayload` / program header `min_out` index / `amounts[min_out]`.
2. Accept a separate entrypoint `min_out: i128` for multiply / swap_* / repay-with-collateral.
3. Compare measured Δ to aggregator return value (return discarded — correct for lies; also means cannot reuse honest return as a floor without measuring).
4. Assert that payload `token_in` / `token_out` match the strategy’s expected assets (measurement is on the strategy-chosen `token_out`; paying a wrong asset yields `NoSwapOutput` if expected balance flat).
5. Price the swap in WAD/USD or enforce max loss vs oracle (prices are prefetched for **post** HF, not for swap fairness).

### 2.4 Passthrough path (no router, no slippage surface)

```64:77:contracts/controller/src/strategies/swap.rs
pub(crate) fn swap_tokens_or_passthrough(...) -> i128 {
    if token_in == token_out {
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        amount_in
    } else {
        swap_tokens(...)
    }
}
```

Same underlying Address → empty swap required; amount unchanged. Used for cross-hub same-asset refinance/migrate-style legs. No aggregator min-out applies (none needed).

---

## 3. Call-site inventory (who depends on `swap_tokens`)

| Strategy | Call site | `token_in` / `token_out` | `amount_in` source | Uses measured out as |
|---|---|---|---|---|
| `multiply` | `swap_tokens_or_passthrough` after borrow | debt → collateral | net borrow + optional debt-side initial payment | deposit amount (+ optional collateral payment) |
| `multiply` | `swap_tokens` in `collect_initial_multiply_payment` | third-asset payment → collateral | measured pull from caller | collateral_amount |
| `swap_debt` | `swap_tokens_or_passthrough` | new debt → existing debt | `borrow_into_controller` receipt | `debt_available` for repay |
| `swap_collateral` | via `withdraw_and_swap_from_supply` | current → new | measured withdraw | `process_deposit` amount |
| `repay_debt_with_collateral` | via `withdraw_and_swap_from_supply` (cross-asset) | collateral → debt | measured withdraw | repay `debt_available` |
| Same-market repay | empty swap + `net_settle` | n/a | n/a | no swap |
| `flash_position` | **does not** call `swap_tokens` | receiver does external work | — | controller `min_amount` on collateral Δ |
| `migrate_from_blend` | **no** `swap_tokens` | Blend submit path | — | errors.md listing `#502` for migrate is **stale** |

Entrypoints pass `swap: Bytes` / `convert_swap: Option<Bytes>` (`lib.rs` multiply / swap_debt / swap_collateral / repay). Type alias `StrategySwap = Bytes` — opaque to controller.

---

## 4. Aggregator-side min-out (honest path only)

Wire: `StrategyPayload { amounts, assets, ops }`. Program header byte `[3] = min_out` indexes `amounts[]`.

```70:131:contracts/swap-aggregator/src/execute/mod.rs
let total_min_out = amounts.get_unchecked(program.min_out);
if total_min_out <= 0 { panic SlippageExceeded }
// ... hops, optional fees on in or out ...
let total_out = vault.balance_of(&output_token);
if total_out < total_min_out { panic SlippageExceeded }
// transfer total_out to sender; return total_out
```

Properties of the **honest** build:

- `total_min_out` must be strictly positive (dust floor of 1 is legal).
- Check is after fee skim → fee reduces deliverable vs venue out.
- Min-out is inclusive (`>=`).
- Controller never observes this check; if Wasm is replaced by a malicious owner upgrade, the check can vanish while controller still accepts `received > 0`.

Caller/quote responsibility: off-chain builders (SDK `mapQuoteResponseToStrategySwap`, harness `build_aggregator_swap`) embed the floor. A user who signs `min_out = 1` against an honest router **self-authorizes** near-max slippage; controller will not save them.

---

## 5. Contrast: `flash_position` controller-enforced floors

`flash_position` takes `collaterals: Vec<(HubAssetKey, i128)>` where `i128` is an explicit **slippage floor** (endpoints.md; A045):

- Pre: `min_amount >= 0`; ≥1 positive min; uniqueness on underlying asset.
- Post-callback: `delta >= min_amount` → `CollateralMinimumNotMet` (#504); at least one positive deposit.

That is the pattern strategy swaps lack: a **controller-owned** quantitative bound on measured receipt. Strategy routes instead trust payload-in-aggregator for the bound.

---

## 6. Residual loss model (quantified)

### 6.1 Hard caps (defended)

- Router cannot pull more than `amount_in` without failing auth / `RouterOverspend` (tested: `BadMode::OverPull`).
- Zero `token_out` Δ → `NoSwapOutput` (tested: `BadMode::OutputShortfall`).
- Input balance increase during call → `RouterOverspend` (tested: `BadMode::Refund`).
- Underspend refunds leftover to `refund_to` (tested: UnderPull on multiply / swap_collateral / repay).
- Atomic tx: failed HF / any panic rolls back pool + controller mutations.

### 6.2 Soft / economic bound (partial)

After a successful dust-out swap, `require_post_pool_risk_gates` requires (when debt remains):

- `ltv_collateral >= total_debt`
- `health_factor >= Wad::ONE`
- optional min-borrow-collateral floor

So extractable loss ≈ **value of authorized input − dust output**, clipped so the **resulting account** still meets HF ≥ 1.

| Path | Dust-out likely sticks? | Why |
|---|---|---|
| `swap_collateral` | Often yes if spare HF | Withdraw valuable collateral; deposit dust new asset; remaining supplies may still cover debt |
| `multiply` (new account, no payment) | Usually reverts | New debt + dust collateral → HF < 1 |
| `multiply` (large initial collateral / existing spare HF) | Possible | Dust from debt leg absorbed by spare collateral |
| `swap_debt` | Usually reverts | Full new debt minted; old barely repaid |
| `repay_with_collateral` | Often reverts | Collateral withdrawn; dust repay worsens HF |
| Honest router + `min_out=1` in payload | Same as dust if venues cooperate | Controller still only checks `> 0` |

Blast radius: **single account / single strategy notional**, not protocol TVL mint. Share books remain measured. Agree with A048 impact framing.

### 6.3 Compromised router owner (same gap, stronger adversary)

Immediate `upgrade` can:

- Ignore payload `total_min_out`.
- Transfer dust `token_out` and retain `token_in`.
- Optionally `sweep_balance` other router holdings (ops concern; not controller measurement).

Controller still prevents over-pull beyond `amount_in` and zero-out. Threat-model equates this gap with the ownership deployment gate.

---

## 7. Secondary interactions (not primary gaps)

| Topic | Note | Owner |
|---|---|---|
| Leftover / refund transfers after guard clear | Listed-token hook reentry residual | A007 / A055 |
| Measured Δ vs lying/FOT tokens | Listing trust | A055 / A041 |
| Fee on honest aggregator | Reduces out before min-out; still aggregator-enforced | aggregator |
| Empty swap when assets differ | `InvalidPayments`; harness edge + fuzz | validation |
| Certora | Directional bounds / sanity; **no** rule that measured out ≥ symbolic min | A108 backlog candidate |
| Harness mock | `MockAggregator` pays exactly `payload.min_out`; does not model SlippageExceeded | tests assume floor in payload |
| `errors.md` migrate + `#502` | migrate does not call `swap_tokens` | docs drift |

---

## 8. Evidence matrix

| Claim | Evidence |
|---|---|
| Only `received > 0` on controller | `swap.rs:105`; threat-model §slippage |
| Return value discarded | `swap.rs:91`; INV-STRAT-01 |
| Aggregator enforces `total_min_out` | `execute/mod.rs:70-73,128-131`; unit SlippageExceeded tests |
| Overspend / zero / underspend covered | `tests/test-harness/tests/strategy/router.rs` |
| No controller dust-vs-min test | No harness asserting measured out ≥ caller min on controller (mock pays min_out; BadMode has no “pay 1 vs large min”) |
| Flash position has controller floor | `flash_position.rs:346-355`; A045 |
| Peers agree residual | A047 Gap(1); A048 Gap(1) / Opinion |

---

## 9. Findings list (scoped)

| ID | Finding | Severity | Status |
|---|---|---|---|
| F1 | Controller slippage floor is positivity-only (`NoSwapOutput`) | medium | partial / known |
| F2 | Quantitative min-out lives only in untrusted aggregator payload/execute | medium | accepted trust design |
| F3 | No strategy entrypoint `min_out` parallel to `flash_position` floors | medium | gap vs peer pattern |
| F4 | Post-gate HF ≥ 1 bounds solvency, not swap fairness — enables dust settlement when spare HF exists | medium | by design |
| F5 | Exact pull + measured out + residue refund defended | info | defended |
| F6 | STRIDE Tamper.4 “meet minimums” overclaims controller layer | info | docs drift |
| F7 | `errors.md` attributes `#502` to `migrate_from_blend` incorrectly | info | docs drift |

No novel critical gap beyond the documented unbounded-loss-under-router-trust class (SEED / threat-model / A047 / A048).

---

## 10. Remediation directions (for A110 backlog; not implemented here)

1. **Preferred:** add explicit `min_out: i128` (or per-leg) on multiply / swap_debt / swap_collateral / repay-with-collateral / convert_swap; assert `verify_router_output` result `>= min_out` (mirror flash_position).
2. **Alternative:** controller decodes payload header + amounts registry and asserts measured Δ ≥ `amounts[min_out]` — couples controller to wire format (ADR-0018 / program version); fragile across aggregator upgrades.
3. **Do not:** trust `execute_strategy` return as the floor without measurement (violates ADR-0011).
4. **Ops:** keep router ownership on intended multisig; treat upgrade as live trust downgrade for every in-flight strategy.
5. **Docs:** align STRIDE Tamper.4 and `errors.md` migrate/`#502` with code; optionally document that strategy UX must set meaningful `total_min_out` even though controller does not re-check it.
6. **Tests:** adversarial router that returns `1` while payload claims large `min_out` — today only honest aggregator would revert; controller path should gain an explicit min once (1) lands.

---

## 11. Cross-links

- Agree: A047, A048 (slippage residual), A055 (measurement vs lying tokens), A007 (guard windows), A045 (controller min pattern), A009/A029 (aggregator pointer trust), A020 (threat-model external trust roots).
- Synthesis inputs: A101 (money-movement gaps), A106 (max loss), A108 (missing tests/rules for F1).
