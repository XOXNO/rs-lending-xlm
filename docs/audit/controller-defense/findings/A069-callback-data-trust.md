# A069 — Callback `data` / swap Bytes size and trust

- Agent: A069
- Theme: T4 (input validation); adjacency T3 (opaque swap trust / A056) and DoS.5 (payload size)
- Severity: low (controller Bytes size unbound = fee/resource hygiene); trust opacity of swap contents = **same class as A056 medium residual**, owned there for quantitative slippage; flash `data` opacity = **info / by design**
- Status: partial
- Paths:
  - Flash callback `data`: `contracts/controller/src/lib.rs` (`flash_loan`, `flash_position`); `strategies/flash_loan.rs`; `strategies/flash_position.rs` (`invoke_receiver`); `external/pool.rs` (`pool_flash_loan_call`); `contracts/pool/src/ops/flash.rs` (`apply` / `invoke_receiver`)
  - Strategy swap `Bytes`: `common/src/types/shared.rs` (`StrategySwap = Bytes`); `strategies/swap.rs` (`swap_tokens`, `swap_tokens_or_passthrough`); callers `multiply.rs`, `swap_debt.rs`, `swap_collateral.rs`, `repay_debt_with_collateral.rs` (+ `convert_swap: Option<Bytes>`)
  - Aggregator bounds (downstream, not controller): `contracts/swap-aggregator/src/lib.rs` (`execute_strategy` / `FromXdr`); `program.rs` (`MAX_OPS`, `MAX_WEIGHTS`, `MAX_PROGRAM_BYTES`, `MAX_ASSETS`, `MAX_AMOUNTS`); `execute/mod.rs` (`total_min_out`)
  - Types / interfaces: `interfaces/controller/src/lib.rs`, `interfaces/swap-aggregator/src/lib.rs`, `interfaces/pool/src/lib.rs`
- Defense: Flash `data` is an opaque initiator→receiver channel; money safety does **not** depend on decoding it (INV-FLASH-01/02, Wasm receiver, measured collateral / pullback). Strategy `swap` is opaque at the controller; emptiness is gated when a swap is required vs forbidden on passthrough/net; router call is flash-guarded; return discarded; settlement by balance Δ + `received > 0` (INV-STRAT-01/02). Honest aggregator additionally bounds packed `ops` and registry lengths and enforces payload `total_min_out`.
- Gap: (1) **No controller- or pool-side `data.len()` / `swap.len()` cap** — unlike view Vecs (`MAX_VIEW_INPUTS = 256`) or aggregator `Program::decode` constants. (2) Controller never decodes `StrategySwap`; quantitative floors and route structure live only inside untrusted aggregator Bytes (A056 / threat-model K1). (3) Flash `data` may be empty or arbitrarily large; neither emptiness nor schema is checked. (4) STRIDE DoS.5 / threat-model “route payload limits” describe **aggregator** parser bounds, not a controller pre-check on `flash_*` `data` or strategy `Bytes` before handoff.
- Impact: Oversized caller-supplied Bytes is **self-funded fee/CPU grief** inside the caller’s own transaction (Soroban resource metering), not cross-account inventory theft. Economic loss from opaque swap contents (dust `min_out`, malicious upgrade ignoring payload) is account-local ≤ in-flight swapped notional subject to post-gate HF — owned by A056/A048. Malicious flash `data` can only harm a receiver that trusts that blob; pool/controller fund safety is independent of contents.
- Evidence: INV-FLASH-01/02, INV-STRAT-01/02; ADR-0011 / ADR-0018 / ADR-0020; threat-model flash receiver + slippage known gap; STRIDE Tamper.4 / DoS.5 / Spoof.4; harness empty-swap / flash / reentrancy suites; aggregator `program_decode` size pins; peers A007, A019, A044, A045, A056, A062, A070.
- Opinion: Treat flash `data` opacity as intentional and correctly outsourced to repayment/measure defenses. Treat strategy Bytes opacity as the same accepted trust design A056 already rates **partial/medium** for slippage — this file adds the **size** strand and the flash-callback inventory. A soft controller `MAX_STRATEGY_BYTES` / `MAX_FLASH_DATA_BYTES` would only close DoS.5 hygiene drift vs STRIDE wording; it would **not** close K1. Prefer A056’s explicit `min_out` for economic trust.

## Scope

Audit every controller (and pool flash) surface that accepts opaque `Bytes`:

1. `flash_loan(..., data: Bytes)` — forwarded through pool to `execute_flash_loan`.
2. `flash_position(..., data: Bytes)` — forwarded to `execute_flash_position`.
3. Strategy routes: `multiply` (`swap`, `convert_swap`), `swap_debt`, `swap_collateral`, `repay_debt_with_collateral` — typed as `StrategySwap = Bytes`, passed to `SwapAggregatorClient::execute_strategy`.

Questions:

- Is **length** bounded at the controller / pool?
- Is **emptiness** validated, and consistently with whether a swap is required?
- Who is trusted to **interpret** the blob, and what protocol invariants still hold if the interpreter is malicious or the blob is garbage?
- How do aggregator-side ADR-0018 bounds relate to controller claims in STRIDE DoS.5?

Out of scope as primary claims: quantitative slippage / `verify_router_output` positivity (A056), flash reentrancy flag lifecycle (A007), Wasm receiver gate (A019), flash pullback / collateral measure (A044/A045), refund allowlists (A070), Vec length (A062), destination `to` (A057).

Method: source read of controller strategies + pool flash + aggregator decode; docs of record; peer findings; harness empty-payload and flash adversarial pins.

---

## Verdict

**Partial.**

| Strand | Judgment |
|---|---|
| Flash `data` trust (contents) | **Defended by design** — protocol never interprets; money safety is repayment / measure / still-open |
| Flash `data` size | **Undefended at controller/pool** — host metering only; Low hygiene |
| Strategy `swap` emptiness gating | **Defended** — non-empty when swap required; empty when passthrough/net |
| Strategy `swap` size | **Undefended at controller** — aggregator bounds after `FromXdr` + `Program::decode`; Low hygiene pre-handoff |
| Strategy `swap` semantic trust (`min_out`, route) | **Accepted gap** — same residual as A056 (medium); not re-scored here |

No novel Critical/High fund-theft via Bytes contents or size alone while INV-FLASH-01/02 and INV-STRAT-01/02 hold.

---

## 1. Inventory of opaque `Bytes` entrypoints

| Entrypoint | Param | Consumer | Controller inspects? |
|---|---|---|---|
| `flash_loan` | `data: Bytes` | Receiver `execute_flash_loan` (via pool) | No — forward only |
| `flash_position` | `data: Bytes` | Receiver `execute_flash_position` | No — `data.clone()` into invoke |
| `multiply` | `swap: Bytes` | Aggregator `execute_strategy` | Emptiness via `swap_tokens(_or_passthrough)` only |
| `multiply` | `convert_swap: Option<Bytes>` | Same, when initial payment is a third asset | Presence (`ConvertStepsRequired`); then emptiness via `swap_tokens` |
| `swap_debt` | `swap: Bytes` | Aggregator | Emptiness only |
| `swap_collateral` | `swap: Bytes` | Aggregator | Emptiness only |
| `repay_debt_with_collateral` | `swap: Bytes` | Aggregator **or** must be empty on same-market net | Emptiness polarity by branch |

Type alias (`common/src/types/shared.rs`):

```9:10:common/src/types/shared.rs
/// Encoded swap route passed to the aggregator router's `execute_strategy` entry point.
pub type StrategySwap = Bytes;
```

There is **no** controller helper akin to `require_view_inputs_bound` for Bytes length. `common/src/validation.rs` has amount / payments / Wasm helpers only — no `require_bytes_*`.

---

## 2. Flash callback `data`

### 2.1 `flash_loan` path

```18:37:contracts/controller/src/strategies/flash_loan.rs
pub(crate) fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    require_authorized_caller(env, caller);
    require_positive_amount(env, amount);
    config::require_hub_active(env, hub_asset.hub_id);
    require_wasm_receiver(env, receiver);
    // ...
    let fee = storage::with_flash_guard(env, || {
        pool_flash_loan_call(env, &pool_addr, hub_asset, caller, receiver, amount, data)
    });
```

| Check on `data` | Present? |
|---|---|
| Non-empty | **No** |
| Max length | **No** |
| Schema / XDR decode | **No** |
| Equality / hash binding | **No** |

Pool `ops/flash.rs::apply` likewise takes `data: Bytes` and passes it unchanged into `execute_flash_loan`. Pool money defenses (three SAC brackets, allowance `transfer_from` of principal+fee) do **not** read `data` (A044).

Mock receivers optionally `FromXdr` a `FlashLoanRequest` from `data` for test modes; production protocol code never does.

### 2.2 `flash_position` path

```311:337:contracts/controller/src/strategies/flash_position.rs
fn invoke_receiver(
    env: &Env,
    receiver: &Address,
    initiator: &Address,
    account_id: u64,
    asset: &Address,
    amount: i128,
    amount_received: i128,
    controller: &Address,
    data: &Bytes,
) {
    env.invoke_contract::<()>(
        receiver,
        &Symbol::new(env, "execute_flash_position"),
        (
            initiator.clone(),
            account_id,
            asset.clone(),
            amount,
            0i128,
            amount_received,
            controller.clone(),
            data.clone(),
        )
            .into_val(env),
    );
}
```

Same opacity: no length / emptiness / schema check. `data.clone()` copies the host Bytes object into the callback arg list — cost scales with size (paid by the invoking transaction).

Post-callback defenses that **do not** depend on `data`:

| Defense | Role |
|---|---|
| `require_wasm_receiver` | Spoof.4 / A019 |
| Ban controller/pool as receiver | Measurement stranding (A045) |
| `with_flash_guard` over forward + callback | INV-FLASH-02 / A007 |
| Declared collaterals + `min_amount` floors | Controller-owned receipt floor (contrast A056) |
| Listed unique `refund_assets` | A070 |
| `require_flash_position_still_open` ×2 + `strategy_finalize` | INV-STRAT-04 |

ADR-0020 / `endpoints.md` treat the receiver as caller-chosen Wasm that returns measured collateral; `data` is the private channel for that contract’s own instructions (quotes, venue calldata, auth blobs). Protocol invariants remain if `data` is empty, garbage, or adversarially large — failure modes are callback panic (full revert) or collateral floors / still-open asserts.

### 2.3 Trust model for flash `data`

| Actor | Trust assumption |
|---|---|
| Caller / initiator | Supplies `data`; typically same economic party as receiver author |
| Receiver Wasm | May interpret `data` arbitrarily; may be malicious |
| Controller / pool | **Must not** need correct `data` for fund safety |

Threat-model actor table: “Flash receiver — Runs arbitrary callback code.” Controls listed are Wasm gate, exact balances, allowance repay, flash flag — **not** `data` validation. Correct.

**Residual:** A compromised or buggy receiver that *trusts* malicious `data` from a third-party initiator can hurt itself (bad swaps, wrong approvals). That is receiver-application risk, not a lending-book invariant break. Cross-account grief via stuffing huge `data` into *another* user’s transaction is unavailable — each tx carries its own args.

---

## 3. Strategy swap `Bytes`

### 3.1 Controller surface (`swap.rs`)

```15:24:contracts/controller/src/strategies/swap.rs
pub(crate) fn swap_tokens(...) -> i128 {
    require_positive_amount(env, amount_in);
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
    // ... authorize exact amount_in, with_flash_guard → execute_strategy, overspend checks,
    // leftover refund, verify_router_output (received > 0)
}
```

```64:78:contracts/controller/src/strategies/swap.rs
pub(crate) fn swap_tokens_or_passthrough(...) -> i128 {
    if token_in == token_out {
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        amount_in
    } else {
        swap_tokens(env, refund_to, token_in, amount_in, token_out, swap)
    }
}
```

| Controller check on `swap` | Enforced? | Meaning |
|---|---|---|
| `!is_empty()` when assets differ | Yes | Forces *some* payload; does not validate structure |
| `is_empty()` when same asset / net settle | Yes | Forbids junk that would hit the router |
| `len() <= N` | **No** | — |
| Decode `StrategyPayload` / read `total_min_out` | **No** | Opaque passthrough |
| Compare payload `token_in`/`token_out` to strategy assets | **No** | Wrong-asset output → flat expected balance → `NoSwapOutput` if Δ=0 |

`call_router_with_reentrancy_guard` discards the aggregator return (`INV-STRAT-01`). Settlement is measured controller `token_out` Δ.

### 3.2 Emptiness matrix (all strategy callers)

| Path | Predicate | Empty `swap` | Non-empty |
|---|---|---|---|
| `multiply` debt→collateral (different) | `swap_tokens_or_passthrough` | Reject `InvalidPayments` | Router |
| `multiply` same-asset mode | passthrough arm | Required | Reject |
| `multiply` `convert_swap` missing on third-asset payment | `ConvertStepsRequired` | n/a | — |
| `multiply` `convert_swap: Some(empty)` | `swap_tokens` | Reject | Router |
| `swap_debt` / `swap_collateral` | always cross-asset in practice | Reject via `swap_tokens` | Router |
| RDWC same market | `swap.is_empty()` assert | Required | Reject |
| RDWC cross asset | `swap_tokens` | Reject | Router |

Harness pins: `test_swap_debt_empty_swap_payload_rolls_back_new_debt`, `test_strategy_empty_swap_payload_multiply`, `test_repay_debt_with_collateral_*_empty_swap_*`, fuzz `empty_swap_payload_reverts_without_state_or_guard_leak`.

### 3.3 Aggregator-side size & structure (downstream)

Controller passes raw `Bytes` to `execute_strategy`. Aggregator:

1. `StrategyPayload::from_xdr` — panics `InvalidRouteXdr` on malformed outer XDR.
2. `Program::decode` on packed `ops` with registry lengths:

| Constant | Value | Role |
|---|---|---|
| `MAX_OPS` | 48 | Instruction cap |
| `MAX_WEIGHTS` | 32 | Split-weight cap |
| `MAX_PROGRAM_BYTES` | `10 + 5×48 + 3×32` = **346** | Stack decode buffer for `ops` |
| `MAX_ASSETS` | 256 | Address registry |
| `MAX_AMOUNTS` | `MODE_PPM_BASE - MODE_FIXED_BASE` = **126** | Amount registry |

ADR-0018: compact registries + indexed instructions; parser validates bounds before venue dispatch. Unit pins in `contracts/swap-aggregator/tests/unit/program_decode.rs` (oversized program, maximal legal buffer).

**Important asymmetry:** these bounds apply **inside the aggregator after** the controller has already entered `with_flash_guard` and authorized `amount_in`. A huge but invalid outer XDR still incurs decode/CPU work on the aggregator (and thus on the strategy tx) before revert. Caller pays; no durable controller state should commit on revert. This is **not** a controller-enforced size gate.

Semantic trust (`total_min_out > 0`, `total_out >= total_min_out`) is also aggregator-only — A056 owns the controller residual that `verify_router_output` is positivity only.

### 3.4 What the controller trusts vs rejects

| Claim in Bytes | Controller stance |
|---|---|
| “I will deliver ≥ X out” | **Ignored** — not read; A056 gap |
| “Route hops / venues / pools” | **Ignored** — not read |
| “Referral id / fees” | **Ignored** |
| Non-empty when swap required | **Enforced** |
| Empty when passthrough/net | **Enforced** |
| Positive measured `token_out` Δ | **Enforced** (`NoSwapOutput`) |
| No input overspend vs auth | **Enforced** (`RouterOverspend`) |
| Post-strategy HF when debt remains | **Enforced** (`strategy_finalize`) |

---

## 4. Size / DoS analysis (DoS.5 adjacency)

### 4.1 STRIDE / threat-model wording vs live controller

STRIDE DoS.5 residual text claims “route payloads are parsed with bounds…”. That is accurate for **`Program::decode`**, inaccurate if read as “controller entrypoints bound `Bytes` length before external handoff.”

Threat-model availability section: “Bounded position counts and **route payload limits** can reject a valid large action” — again aggregator/parser rejection, not a controller `data.len()` check.

A062 already noted uncapped mutator Vecs vs view 256-cap. A069 is the **Bytes** sibling: flash `data` and strategy `swap` join that hygiene class.

### 4.2 Blast radius of oversized Bytes

| Scenario | Effect |
|---|---|
| Caller attaches multi-MB `data` / `swap` | Tx fails resource budget or burns caller fees; atomic revert |
| Delegate submits oversized/dust-min swap on owner account | Fee grief + possible economic slippage (A056); not pool share mint |
| Invalid XDR after large allocate in aggregator | Revert under flash guard; no committed strategy settlement if outer tx rolls back |
| Empty flash `data` | Allowed; honest receivers that require XDR may panic → full revert |

**Not achievable via Bytes alone:** silent share inflation, unpaid flash principal, skipping `require_flash_position_still_open`, or bypassing flash guard.

### 4.3 Contrast: where the codebase *does* bound inputs

| Surface | Bound |
|---|---|
| Views | `MAX_VIEW_INPUTS = 256` |
| Delegates | `MAX_DELEGATES = 16` |
| Position slots | `POSITION_LIMIT_MAX = 5` |
| Flash collaterals / refunds | `len <= max_supply_positions` + uniqueness (A070/A062) |
| Aggregator `ops` | `MAX_PROGRAM_BYTES` / `MAX_OPS` / … |
| Controller `Bytes` args | **None** |

---

## 5. Cross-path trust summary

```
                    ┌─────────────────────────────────────────┐
                    │ Caller-supplied Bytes                    │
                    └─────────────────────────────────────────┘
                      │ flash data          │ StrategySwap
                      ▼                     ▼
              receiver callback      swap-aggregator
              (untrusted Wasm)       (untrusted Wasm / owner)
                      │                     │
                      │ contents ignored    │ FromXdr + Program::decode
                      │ by controller/pool  │ + total_min_out (honest)
                      ▼                     ▼
              INV-FLASH measure/      INV-STRAT measure Δ,
              repay / still-open      overspend, HF gate;
                                      positivity only at controller
```

Flash and strategy share the pattern: **opaque blob + untrusted interpreter + controller-owned settlement assertions that do not decode the blob.**

Difference: flash_position already has controller-owned **quantitative** collateral floors (`min_amount`); strategy swaps do not have a controller-owned `min_out` (A056). Size unbound is common to both.

---

## 6. Findings inventory

| ID | Finding | Severity | Status |
|---|---|---|---|
| F1 | No `data.len()` / `swap.len()` cap on controller or pool flash path | low | open (hygiene) |
| F2 | Flash `data` never validated for emptiness or schema — intentional opacity | info | accepted |
| F3 | Strategy `StrategySwap` never decoded at controller; structure/`min_out` trust = aggregator | medium residual | accepted / owned by A056 for economics |
| F4 | Emptiness polarity (required vs forbidden) is consistent across strategy callers | info | defended |
| F5 | Aggregator ADR-0018 bounds exist but only after controller handoff / flash guard | low | documented |
| F6 | STRIDE DoS.5 / threat-model “payload bounds” over-read if applied to controller `Bytes` args | info | docs drift |
| F7 | `data.clone()` on flash_position scales with size inside guarded window | info | metering-mitigated |

No Critical/High novel hole in this scope.

---

## 7. Tests and evidence map

| Evidence | What it shows |
|---|---|
| Pool `flows.rs` flash suite | Empty `Bytes::new` succeeds on happy path; repayment independent of `data` |
| Harness `controller/flash_loan*.rs`, `strategy/flash_position*.rs` | Wasm / reentry / measure; not size caps |
| `test_*_empty_swap_payload_*` | Emptiness rejects cross-asset; allows same-asset RDWC |
| Fuzz `empty_swap_payload_reverts_without_state_or_guard_leak` | Atomic reject + no guard leak |
| Aggregator `program_decode.rs` | Oversized `ops` → `InvalidRouteXdr`; max legal buffer |
| Certora strategy directional rules | Bounds/HF, not Bytes length |
| A056 harness `strategy/router.rs` | Adversarial router amount claims — semantic trust, not size |

**Missing tests (for A108 adjacency):** no harness case that asserts controller rejection of oversized `data`/`swap` (because no such gate exists); optional pin that a max-legal aggregator payload still settles under controller positivity.

---

## 8. Remediation options (ordered)

1. **Do nothing on size** if product accepts host metering as the only bound — document explicitly that DoS.5 “payload bounds” are aggregator-local, and that flash `data` is intentionally unchecked. Update STRIDE residual note to avoid overclaim.
2. **Soft controller caps** (e.g. `MAX_FLASH_CALLBACK_DATA`, `MAX_STRATEGY_SWAP_BYTES`) checked before pool/router handoff — closes F1/F5 hygiene; pick limits above legitimate ADR-0018 max XDR (registries up to 256 addresses dominate size, not the 346-byte `ops` blob).
3. **Economic trust (preferred for F3):** A056 remediation — explicit controller `min_out` (or decode-and-check `total_min_out` vs measured Δ). Size caps do **not** substitute.
4. **Do not** make the controller a full ADR-0018 parser solely for size — couples upgrades and duplicates aggregator logic (A056 already warns against decode-coupling).

---

## 9. Peer cross-links

| Peer | Relationship |
|---|---|
| A056 | Owns quantitative slippage / opaque `min_out`; explicitly outscoped Bytes **size** to A069 |
| A048 / A047 / A046 | Money-flow; list slippage Gap(1) — same opacity class |
| A007 / A030 | Flash guard around callback and `execute_strategy` |
| A019 | Wasm receiver — prerequisite so `data` reaches code, not an EOA |
| A044 / A045 | Flash money safety independent of `data` contents |
| A070 | Flash declaration lists — structured inputs, not Bytes |
| A062 | Vec length hygiene sibling; DoS.5 adjacency |
| A102 | Listed A069 unfiled; this file closes that wave-4 hole for size/trust |
| A107 / A110 | Tamper.4 / RB-03 economic residual stays with A056; A069 size → optional P3 hygiene |

**Disagreement:** none material with A056 — this file does not re-rank slippage severity; it inventories size and flash `data` trust.

---

## 10. Bottom line for synthesis

- **Flash `data`:** opaque by design; fund safety defended without parsing; size unbound = Low self-DoS hygiene only.
- **Strategy `Bytes`:** emptiness defended; length unbound at controller; semantic trust = known aggregator trust-root (A056).
- **Do not** treat missing controller Bytes caps as a Critical gap or as closing K1.
- **Do** note STRIDE DoS.5 wording drift and optional soft caps if hygiene is desired.
)
