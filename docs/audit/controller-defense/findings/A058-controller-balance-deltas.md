# A058 — Controller balance delta measurement correctness

- Agent: A058
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/payments.rs:7-24` (`balance_delta_since`); `:26-99` (`aggregate_*` — amount fold only)
  - `common/src/token.rs:19-34` (`transfer_amount_measured`); `:39-55` (`authorize_transfer_as_current`)
  - `contracts/controller/src/strategies/legs.rs` (`repay_debt_from_controller`, `withdraw_collateral_to_controller`, `refund_controller_balance_delta`; siblings `execute_withdraw_all`, `net_settle_collateral_against_debt`)
  - Primary consumers: `positions/{supply,debt,liquidation/apply}.rs`, `keepers.rs`, `strategies/{swap,flash_position,migrate_blend,multiply}.rs`, `strategies/mod.rs` (`snapshot_balances`, `withdraw_and_swap_from_supply`)
- Defense: Two shared primitives implement INV-ACCT-03 / ADR-0013 at every controller custody boundary. `transfer_amount_measured` credits **recipient Δ** after a push; `balance_delta_since` credits **holder Δ** since an explicit baseline. Strategy legs compose them so pool share books follow measured cash, refunds forward only positive post-baseline increases (never gross sweeps), and router/pool return figures are discarded or equality-checked. Aggregate payment folding rejects negatives / empty / non-positive (Rejected) before any transfer.
- Gap: (1) Shared A055 / listing trust — primitive trusts `token.balance` / `transfer`; rebasing or balance-lying listed tokens are outside the SEP-41/SAC assumption. (2) Hygiene — `transfer_amount_measured` does **not** assert measured Δ `> 0` (or even `≥ 0`); zero/negative receipts rely on downstream pool gates or call-site asserts (fail-closed under SAC/FoT; hostile listed token is A055). (3) Overflow panic code asymmetry — measured underflow/overflow in `transfer_amount_measured` uses `#14 AmountMustBePositive`; `balance_delta_since` uses `InternalError`. (4) Accepted — outbound refunds / leftovers use raw `transfer`, not recipient-measured delivery (FOT haircuts the refundee only; A054). (5) Accepted — `claim_revenue` event reports controller receipt of the pool→controller hop; forward `transfer_amount_measured` return is discarded (one outbound FoT hop vs accumulator; harness-pinned). (6) `net_settle` / direct pool→user withdraw intentionally skip controller Δ (no custody / no credit from requested amount).
- Impact: No path found where a requested transfer amount alone mints supply/debt credit, where strategy residue sweeps pre-existing controller inventory, or where FoT into the pool inflates shares beyond cash. Blast radius of a measurement bypass requires a governance-listed non-SAC token that lies about balances (A055) — market-wide desync capped by that market’s TVL, not a hole in these primitives’ arithmetic.
- Evidence: INV-ACCT-02/03/05, INV-STRAT-01/02, INV-LIQ-03; ADR-0011/0013; STRIDE TB7 / Tamper.3; threat-model “Non-standard tokens” + “Controller to router and tokens”; unit `common/tests/token.rs`, `contracts/controller/tests/helpers/utils.rs`, `contracts/controller/tests/events.rs` (FoT liquidation); harness `controller/outbound_transfer_measurement.rs`, strategy FoT/adversarial suites. Cross-ref A016, A041, A045–A050, A054, A055, A061.
- Opinion: Measurement core is defended and correctly centralized. Keep `payments::balance_delta_since` + `common::token::transfer_amount_measured` as the only credit oracles for custody legs. Optional hardening only: assert `measured > 0` (or `≥ 0` with explicit zero policy) inside `transfer_amount_measured`, and align overflow panics to `InternalError`/`MathOverflow`. Do not “fix” by trusting pool/router return values or adding gross-balance sweeps.

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format; confirmed `A058-*.md` absent.
2. Read primitives end-to-end (`payments.rs`, `common/src/token.rs`, `legs.rs`) and every Rust call site of `balance_delta_since` / `transfer_amount_measured`.
3. Cross-checked pool ops that consume measured amounts (`supply`, `repay`, `recapitalize`), ADR-0011/0013, INV-ACCT-03, peers A041/A045–A050/A054/A055/A061.
4. Enumerated zero / negative / FoT / donation / baseline-contamination / overflow behaviors and whether each fails closed.
5. No novel critical fund-theft gap beyond listing-trust residuals already owned by A055.

---

## 1. Primitive contracts

### 1.1 `transfer_amount_measured` — push then measure recipient

```19:34:common/src/token.rs
pub fn transfer_amount_measured(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    non_positive_error: GenericError,
) -> i128 {
    assert_with_error!(env, amount > 0, non_positive_error);
    let tok = token::Client::new(env, asset);
    let pre = tok.balance(to);
    tok.transfer(from, to, &amount);
    let post = tok.balance(to);
    post.checked_sub(pre)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AmountMustBePositive))
}
```

| Property | Behavior |
|---|---|
| Requested amount | Must be `> 0`; caller-chosen panic code |
| Credited figure | `post − pre` at **`to`**, never `amount` |
| FoT / short delivery | Returns less than `amount` → downstream books that value |
| Donation during transfer | Returns more than `amount` → credit follows cash that arrived (INV-ACCT-02 donations still don’t rewrite lendable cash without an accounting path; supply then `credit_cash(received)`) |
| `post < pre` | **Returns a negative i128** — `checked_sub` only fails on i128 overflow, not on sign |
| Overflow of Δ | Panics `#14 AmountMustBePositive` (misnamed vs true overflow) |
| Auth | Does not itself authorize; callers that pull as the contract use `authorize_transfer_as_current` separately (router path) |

Controller re-exports this as `payments::transfer_amount_measured` so position/strategy code has one import surface.

### 1.2 `balance_delta_since` — pure snapshot diff

```14:24:contracts/controller/src/payments.rs
pub(crate) fn balance_delta_since(
    env: &Env,
    asset: &Address,
    holder: &Address,
    before: i128,
) -> i128 {
    token::Client::new(env, asset)
        .balance(holder)
        .checked_sub(before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}
```

| Property | Behavior |
|---|---|
| No transfer | Read-only; caller owns when `before` was taken |
| Negative Δ | Allowed (balance fell) — callers must gate |
| Overflow | `InternalError` (better signal than §1.1) |
| Contaminated baseline | Wrong `before` ⇒ wrong Δ; all in-tree call sites pair the same `holder` and take `before` immediately before the external work |

This is the vocabulary for pool→controller mints, withdraw-to-controller, router output, claim_revenue, migrate leftovers, and refunds.

### 1.3 `aggregate_*` (same file, not a balance oracle)

`aggregate_positive_payments` / `aggregate_payments` fold `HubPayment` legs before any token move. Negative always fatal; `ZeroLeg::Rejected` rejects 0; `ZeroLeg::MeansAll` uses sticky zero as withdraw-all sentinel. Ordering is load-bearing (A061). They do **not** measure balances; they only size subsequent measured transfers.

---

## 2. `legs.rs` — controller-custody composition

### 2.1 `repay_debt_from_controller`

```
1. received = transfer_amount_measured(controller → pool, debt_available)
2. snapshot controller balance AFTER the push / BEFORE pool repay
3. execute_repayment(..., amount: received)   // PoolAction uses measured
4. refund_controller_balance_delta(..., snapshot, caller)
```

Load-bearing details:

- Share burn input is **pool receipt**, not `debt_available` (INV-ACCT-03).
- Refund baseline is **post-transfer**, so FoT haircut on the inbound push is not mistaken for “excess,” and pre-existing controller inventory is outside the Δ.
- Pool overpay refunds to controller-as-payer; step 4 forwards only `max(0, Δ)` to `caller` (A054).

### 2.2 `withdraw_collateral_to_controller`

```
1. balance_before = controller balance(asset)
2. with_flash_guard { execute_withdrawal(..., counterparty=controller) }
3. return balance_delta_since(controller, balance_before)
```

- Does not trust pool-reported payout alone.
- Does not require Δ `> 0` here; `withdraw_and_swap_from_supply` → `swap_tokens` (`require_positive_amount`) or later `transfer_amount_measured` on deposit/repay fail-closes zero/negative under normal paths. Same-asset passthrough returns the raw Δ and the next credit leg still requires `amount > 0`.
- Flash guard covers the pool withdraw + token transfer window (A007).

### 2.3 `refund_controller_balance_delta`

```
excess = balance_delta_since(controller, balance_before)
if excess > 0 { transfer(controller → refund_to, excess) }  // raw, not measured
```

- Positive-only; never sweeps dust that existed at `balance_before`.
- Raw outbound transfer: FOT under-delivers to `refund_to` only; cannot inflate protocol credit (A045 gap #6 / A054 accepted).

### 2.4 Sibling legs (measurement inventory)

| Helper | Token movement | Measurement |
|---|---|---|
| `execute_withdraw_all` | Pool → `destination` directly | None at controller (user/close path; A041 outbound note) |
| `net_settle_collateral_against_debt` | None (share netting) | N/A — uses pool `settled_amount` for share merge only |

Neither invents unmeasured **inbound credit** against a requested transfer amount.

---

## 3. Call-site matrix (who measures what)

| Surface | Snapshot / measure | Credited / returned figure | Positive gate |
|---|---|---|---|
| User/strategy `settle_supply` | `transfer_amount_measured` → pool | `PoolSupplyEntry.amount = received` | Requested `> 0`; pool allows `amount==0` no-op / else `SupplyRoundsToZeroShares` |
| User `repay` / strategy repay | `transfer_amount_measured` → pool | `PoolAction.amount = received` | Same; `RepayRoundsToZeroShares` if positive net burns 0 shares |
| Liquidation repay | Measured into pool; USD floor-scaled if `received < planned` | Seizure coupling uses delivered USD (INV-LIQ-03) | Planned amount `> 0` via transfer assert |
| `recapitalize` | Measured into pool | Pool applies `min(received, shortfall)` | `require_positive_amount` + transfer |
| `claim_revenue` | `balance_delta_since` after pool claim; forward measured amount | Event = controller receipt; accumulator gets forward (possibly FoT-haircut) | Forward only if `received > 0` |
| `borrow_into_controller` | `balance_delta_since` vs pool `amount_received` | Equality + `measured > 0` | Hard assert |
| `flash_position` mint/forward | Outer Δ == reported; `transfer_amount_measured` to receiver | `forwarded > 0` | Triple positive/equality chain (A045) |
| `swap_tokens` output | `verify_router_output` → `balance_delta_since` | Must be `> 0` (`NoSwapOutput`) | Yes |
| Router input leftover | `in_before − in_after` (not the helper, same idea) | Leftover ≤ `amount_in` to `refund_to` | Overspend asserts |
| `migrate_blend` leftover / sweep | `snapshot_balances` + `balance_delta_since` | Repay/deposit only if Δ `> 0` | Yes |
| Multiply initial payment | `transfer_amount_measured` → controller | Fold uses `received` | Requested `> 0` |

**Pattern:** inbound protocol credit always passes through a measured receipt. Strategy “open” paths double-check pool reports (`borrow_into_controller`). Strategy “close/refinance” paths measure controller→pool then delta-refund. Untrusted router returns are discarded (INV-STRAT-01 / ADR-0011).

---

## 4. Failure-mode analysis

### 4.1 Fee-on-transfer (in scope, defended)

WeirdToken / shortfall fixtures exist for liquidation events, claim_revenue, recapitalize refunds, and strategy collateral. Credited shares/cash track **delivered** units; seizures scale down; indexers on `revenue:claim` see controller-measured receipt (not pool intent).

### 4.2 Zero measured receipt

`transfer_amount_measured` can return `0` if a token delivers nothing yet succeeds. Pool supply/repay treat `amount == 0` as an accounting no-op (no share mint/burn required). User loses the FoT haircut to the token contract — protocol does not mint claims against missing cash. Liquidation planned USD scales to 0 via `mul_div_floor` when `received == 0`.

### 4.3 Negative measured receipt (hostile listed token)

Not rejected at the primitive. Under SEP-41/SAC transfers this does not occur. A listed token that drains `to` during `transfer` could return a negative Δ into `make_pool_action` / `credit_cash` — that is **listing compromise** (A055 / Tamper.3 residual), not a missing baseline on honest SAC/FoT assets. Stronger call sites (`borrow_into_controller`, swap verify, flash forward) already require `> 0`.

### 4.4 Baseline contamination / pre-existing balances

Defended wherever refunds or strategy credits use an explicit pre-work snapshot:

- Refunds: Δ since post-push or pre-callback baseline.
- Flash collateral/refunds: snapshots after mint/forward, before callback (A045).
- Migrate: `snapshot_balances` before Blend work; deposit/repay only positive Δ.
- Claim: dust at controller untouched when forwarding measured claim (harness).

No gross `balance(controller)` sweep exists in these primitives.

### 4.5 Reentrancy between `pre` and `post`

Only the token `transfer` runs in the window. Hooks can reenter controller verbs when the flash flag is clear (ordinary supply/repay) — mitigated by listing policy + flash guard on strategy/pool windows (A007). Measurement still reports the recipient’s net balance after the outer transfer returns; nested measured pushes are separate INV-ACCT-03 applications.

### 4.6 Overflow / error hygiene

| Primitive | Overflow / impossible Δ | Code |
|---|---|---|
| `balance_delta_since` | `checked_sub` fail | `InternalError` |
| `transfer_amount_measured` | `checked_sub` fail | `AmountMustBePositive` (#14) |

Doc comment on `transfer_amount_measured` (“cannot be represented in i128”) matches overflow, but the panic code does not. Operational confusion only; not a funds bug.

### 4.7 `from == to` / controller-as-recipient poison

User borrow/withdraw use `require_external_recipient` so payouts are not stranded on controller/pool (measurement poison / stuck funds). Strategy paths that intentionally use controller custody set counterparty to the controller and measure explicitly.

---

## 5. Consistency with peer findings

| Peer | Agreement |
|---|---|
| A041 | Measured deposit/receipt pattern is the hard rule; this file owns the shared helpers that enforce it |
| A045 | Flash triple-measure chain is a correct specialization of these primitives |
| A046–A050 | Strategy money flows reuse legs/payments without a second dialect |
| A054 | Refunds are delta-only; outbound raw transfer is accepted |
| A055 | Lying/rebasing tokens remain the outer residual; measurement is necessary but not sufficient vs balance lies |
| A061 | Aggregate sign/zero rules complement measurement; they size legs, they do not replace Δ |

No disagreement filed. A058 does not claim a novel critical gap beyond those residuals.

---

## 6. Verdict

**Defended.** `balance_delta_since` and `transfer_amount_measured`, as composed by `legs.rs` and the position/keeper/strategy call sites, correctly implement “credit equals measured receipt” and “refund equals positive custody Δ.” Under the protocol’s SEP-41/SAC + listing assumptions, FoT and short delivery cannot inflate shares or steal unrelated controller balances through these helpers.

Treat regressions that (a) credit pool actions from requested amounts, (b) trust router/pool returns without Δ equality or discard, or (c) refund via gross controller balance as **Critical** against INV-ACCT-03 / INV-STRAT-01/02 / ADR-0013.
)
