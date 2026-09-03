# A057 — Destination `to` option hijack risks

- Agent: A057
- Theme: T3
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs:109-135` (`borrow`, `withdraw` ABI `to: Option<Address>`)
  - `contracts/controller/src/positions/debt.rs:35-76,114-160` (`process_borrow`, recipient resolution, `pool_borrow_call`)
  - `contracts/controller/src/positions/supply.rs:161-238,283-301` (`process_withdraw`, `settle_withdraw`, `apply_withdraw_batch`)
  - `contracts/controller/src/positions/mod.rs:38-50` (`require_external_recipient`)
  - `contracts/controller/src/account.rs:116-143` (`require_owner_or_delegate`)
  - `contracts/controller/src/risk/validation.rs:12-15` (`require_authorized_caller`)
  - `contracts/controller/src/external/pool.rs:33-65` (`pool_borrow_call`, `pool_withdraw_call`)
  - `contracts/pool/src/ops/borrow.rs:25-36`, `contracts/pool/src/ops/withdraw.rs:30-39`, `contracts/pool/src/cache/cash.rs:39-47` (pool pays `receiver` with no second recipient gate)
  - `contracts/controller/src/strategies/flash_position.rs:73-86` (parallel controller/pool ban for flash receivers)
  - `contracts/controller/src/positions/debt.rs:244-298` (`borrow_into_controller` — intentional controller recipient, bypasses the public `to` gate)
- Defense: Both public payout verbs resolve `to.unwrap_or(caller)`, require caller auth + owner-or-delegate **before** any pool transfer, then reject pool and controller as recipient via `require_external_recipient` (`InvalidFlashloanReceiver` #412). Debt/collateral always book to `account_id`; `to` only redirects tokens. Pool is owner-gated (`#[only_owner]`) so strangers cannot set a rogue pool-level receiver.
- Gap: (1) Accepted design — a live delegate (or NFT owner) may set `to` to any non-banned address, including self, draining the account up to post-pool solvency (threat-model “complete economic control”). (2) Residual — other protocol contracts (NFT, governance, price/swap aggregators, accumulator) are not banned; mistaken `to` can strand or expose funds to that contract’s admin surface, without forging controller credit. (3) Observability — position batch events do not record `to`. (4) Shared listing residual (A007/A023) — ordinary borrow/withdraw do not hold the flash flag across `transfer_out`; a hooked listed token could reenter against unpersisted RAM (not a `to`-substitution theft).
- Impact: Cross-account theft by forging `to` without INV-AUTH-02 authority: **none**. Per-account loss equals that account’s withdrawable collateral / borrowable headroom when an authorized party chooses a hostile or mistaken recipient — by design for delegates. Protocol TVL / share books are not inflated by a bad `to` (user paths do not credit measured recipient receipt). Stranding to pool/controller: **closed** before cash moves (GH-17).
- Evidence: INV-AUTH-02; threat-model § “A delegate has complete economic control”; STRIDE I2/I3; endpoints.md `borrow`/`withdraw`; errors.md #412; harness `recipient_is_protocol_contract.rs` (GH-17); `borrow.rs` delegate `to`/`None` routing; `withdraw.rs` third-party `to`; integration `admin.sh` gh17_*; Certora `borrow_rejects_pool_recipient`, `withdraw_rejects_controller_recipient` (+ satisfy twins); skills integrating note. Peers: A003, A005, A011, A019, A023, A024, A041, A044, A055.
- Opinion: The classical “destination hijack” (attacker redirects another user’s payout) is closed. What remains is intentional payout redirection under owner/delegate authority, plus a narrow stranding denylist that correctly targets the two addresses that break protocol cash/measurement invariants. Do not treat delegate-to-self as a novel A057 bug.

---

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git ops).
2. Trace `Controller::{borrow,withdraw}` → recipient resolution → `require_external_recipient` → pool FFI → token `transfer_out`.
3. Ask: who can set `to`? Does `to` require auth? Can `to` forge credit, strand protocol cash, or redirect another account’s funds?
4. Compare internal paths that intentionally pay the controller (`borrow_into_controller`, strategy withdraw-to-controller, liquidation Transfer) and flash-position’s parallel denylist.
5. Cross-check peers A003/A005 (delegate power), A011 (auth vs `to`), A019/A044 (receiver bans), A023/A024 (storage order vs recipient), A041 (no recipient measure on user payouts), A055 (lying tokens), threat-model + GH-17 tests + Certora market-guard rules.

Out of scope as primary claims: swap/router retention (A047/A048), liquidation Credit seize recipients (A013/A052), flash_loan Wasm receiver (A019), strategy finalize batching (A032).

---

## 1. Threat model for “hijack”

| Attack story | Mechanism | Verdict |
|---|---|---|
| Stranger sets victim’s `to` to attacker | Call `borrow`/`withdraw` without owner/delegate | **Blocked** — `#44 NotAuthorized` before recipient use |
| Substitute `to` in a signed invocation | Soroban auth binds invocation args | **Blocked** unless victim/delegate authorized that `to` |
| Delegate / owner sets `to = self` or accomplice | Optional recipient under INV-AUTH-02 | **Allowed** — documented complete economic control |
| `to = pool` or `to = controller` | Strand cash / poison measurement | **Blocked** — `#412` before transfer (GH-17) |
| `to = None` when caller is pool/controller | Default recipient = caller | **Blocked** — same predicate on resolved recipient |
| `to` arbitrary external contract | Payout without recipient `require_auth` | **Allowed** — by design; recipient need not consent to receive |
| Bad `to` inflates shares / debt credit | Controller measures recipient Δ on these paths | **N/A** — shares follow pool mutation outputs, not recipient balance (A041/A082) |
| Front-run changing pool address mid-check | `Cache` reads stored pool once per call | **Not a cross-tx hijack** — single invocation; pool set is admin |

---

## 2. Call graph — public `to` surface

Only two `#[contractimpl]` entrypoints expose optional destination:

```
Controller::borrow(..., to: Option<Address>)     #[when_not_paused]
  └─ process_borrow
       ├─ require_authorized_caller          caller.require_auth + !flash
       ├─ get_account
       ├─ require_owner_or_delegate          INV-AUTH-02
       ├─ recipient = to.unwrap_or(caller)
       ├─ require_external_recipient         ≠ controller ∧ ≠ pool
       ├─ validate_position_entry_gates
       ├─ settle_debt(Borrow { recipient })
       │    └─ pool_borrow_call(pool, recipient, entries)
       │         └─ pool: mint debt, debit cash, transfer_out(recipient)
       │    └─ merge_debt_leg (RAM) from pool mutations
       ├─ enforce_post_pool_solvency
       └─ finalize_position_flow             durable debt (+ supply if LTV restamp)

Controller::withdraw(..., to: Option<Address>)   // not pause-gated
  └─ process_withdraw
       ├─ require_authorized_caller
       ├─ get_account
       ├─ require_owner_or_delegate
       ├─ recipient = to.unwrap_or(caller)
       ├─ require_external_recipient
       ├─ settle_withdraw → apply_withdraw_batch(recipient, Normal, …)
       │    └─ pool_withdraw_call(pool, recipient, false, entries)
       │         └─ pool: burn supply, debit cash, transfer_out(recipient)
       │    └─ merge_withdraw_leg (RAM)
       ├─ enforce_post_pool_solvency
       └─ finalize_position_flow(Supply, remove_if_empty=true)
```

**Invariant of the split:** liability and collateral always attach to `account_id`. `to` never selects which account is debited. A successful `borrow(..., to: Eve)` still leaves debt on Alice’s NFT account; Eve only receives tokens.

Repay has **no** `to` (pulls from `caller`). Liquidation Transfer pays the liquidator (not a user `to`). Strategies that need controller custody use `borrow_into_controller` / `withdraw_collateral_to_controller` and never expose a public `to` on those legs.

---

## 3. `require_external_recipient` — what it does and why

```43:50:contracts/controller/src/positions/mod.rs
pub(crate) fn require_external_recipient(env: &Env, cache: &mut Cache, recipient: &Address) {
    let pool = cache.cached_pool_address();
    assert_with_error!(
        env,
        *recipient != env.current_contract_address() && *recipient != pool,
        FlashLoanError::InvalidFlashloanReceiver
    );
}
```

Documented rationale (module comment + GH-17 harness):

| Banned address | Failure mode if paid |
|---|---|
| **Pool** | Pool has already debited market cash and then `transfer`s to itself; SAC balance may not move while books say cash left — stranded / inconsistent cash story |
| **Controller** | Tokens sit on the controller. Ordinary borrow/withdraw **do not** measure recipient Δ to claim them; strategies that *do* measure would see polluted baselines — funds become unclaimable dust relative to those paths |

Same error discriminant and same two addresses as flash-position’s inline checks (`flash_position.rs:73-86`), reused as `#412 InvalidFlashloanReceiver` (errors.md). UX quirk: the name says “flashloan” on a non-flash path; behavior is still fail-closed.

**Pool does not re-check.** `LiquidityPool::{borrow,withdraw}` take `receiver` and `transfer_out` unconditionally under `#[only_owner]`. The denylist is entirely a controller policy on the public ABI. That is sufficient because only the controller can call pool mutators (INV-AUTH-01).

**Gate ordering:** recipient check runs **after** auth and owner/delegate, **before** aggregation/pool. Harness asserts pool reserves unchanged when borrow-to-pool is rejected.

---

## 4. Auth matrix — who can redirect payouts

| Caller | `to` | Debt/collateral account | Token recipient | Outcome |
|---|---|---|---|---|
| Owner | `None` | Owner’s account | Owner (`caller`) | Happy path |
| Owner | `Some(Bob)` | Owner’s account | Bob | Intentional third-party payout (tested withdraw) |
| Owner | `Some(pool\|controller)` | — | — | `#412` |
| Active delegate | `None` | Owner’s account | **Delegate** | Intentional; harness `test_delegated_borrow_to_none_routes_to_caller` |
| Active delegate | `Some(owner)` | Owner’s account | Owner | Intentional; `test_delegated_borrow_routes_funds_to_owner` |
| Active delegate | `Some(delegate)` | Owner’s account | Delegate | Full drain within HF — threat-model accepted |
| Stranger | any | — | — | `#44` before `to` matters |
| Stale delegate after NFT transfer | any | — | — | `#44` (grant keyed to prior owner; A005 / position_nft tests) |

`to` itself never calls `require_auth`. A011 records this as intentional: funds leave under the authorizing party’s authority. Recipient consent is not part of the safety model (SEP-41 receive is passive).

Pause asymmetry does not create a hijack: `borrow` is `#[when_not_paused]`; `withdraw` stays live so solvent users can exit — still owner/delegate gated, still subject to the same recipient denylist.

---

## 5. Money-flow consequences of a “bad” `to`

### 5.1 Authorized hostile/mistaken recipient (accepted)

Blast radius = that account only:

- **Withdraw to attacker:** collateral leaves; post-pool solvency must still hold if debt remains; attacker holds tokens.
- **Borrow to attacker:** debt stays on the account; attacker holds principal; liquidation risk is the owner’s.

No cross-account contagion; no share inflation. Matches INV-AUTH-02 + threat-model wording that user docs must state the power plainly.

### 5.2 Protocol denylist (defended)

GH-17 suite + integration `gh17_*` + Certora revert rules pin refusal before cash movement. Certora’s assert rules cover `borrow→pool` and `withdraw→controller`; harness covers all four combinations (borrow/withdraw × pool/controller). Satisfy twins show a non-banned recipient can complete on the same fixture.

### 5.3 Other protocol addresses (residual info)

Not banned: position-NFT, governance, price-aggregator, swap-aggregator, revenue accumulator, oracles, random EOAs/contracts.

- **Stranding:** tokens may be irrecoverable if the contract has no sweep (e.g. DeFindex adapter threat-model note) or only an admin rescue.
- **Secondary admin capture:** e.g. paying the **swap aggregator** exposes balances to router `sweep_balance` (threat-model: router owner can move balances above fee reserves to any recipient). That is user/misconfiguration risk under an authorized `to`, not stranger hijack.
- **No fake credit:** controller will not mint supply/debt from “tokens landed on aggregator.”

Widening the denylist is a product/defense-in-depth choice, not required to close theft of foreign accounts.

### 5.4 Controller custody paths intentionally skip the public gate

`borrow_into_controller` pays the controller on purpose (strategy mint), measures Δ, asserts equality with pool report, and runs under `with_flash_guard`. Strategy withdraw-to-controller and liquidation Transfer similarly choose a fixed counterparty. These are not ABI `to` hijacks; they must not call `require_external_recipient` or strategies would brick.

`execute_withdraw_all(destination)` (migrate/close leftover) pays a caller-chosen destination **without** re-invoking `require_external_recipient`. Production callers pass `caller` after an upstream owner/delegate (or account-guard) check (`repay_debt_with_collateral`). Residual: an internal misuse that passed pool/controller as `destination` would not hit the GH-17 gate — worth remembering if new strategy legs are added (defense-in-depth), not an exposed public ABI hole today.

### 5.5 No recipient balance measurement (A041)

Pool→user borrow/withdraw trust pool cash accounting + token transfer. A lying token can under-deliver to `to` while pool has already debited cash (listing trust / A055). That hurts the recipient or stresses cash vs shares; it is not an attacker “hijacking” `to` to steal another user’s position without auth.

### 5.6 Transfer-hook reentrancy (shared residual)

Ordinary `pool_borrow_call` / `pool_withdraw_call` do **not** wrap `with_flash_guard` (unlike `borrow_into_controller`). During `transfer_out` to `to`, a hooked listed token could reenter monetary entrypoints while outer merges are still in RAM (A007 § residual, A023). Auth still required; tx atomicity still applies. Classifying this as destination hijack overstates it — the trigger is the **token**, not a substituted `to` argument. Severity remains listing-trust / low unless a listed asset ships arbitrary hooks.

---

## 6. Observability and integrator footguns

| Item | Observation |
|---|---|
| Events | `UpdatePositionBatchEvent` records amounts/indexes/actions, **not** payout address. Indexers cannot reconstruct `to` from events alone — must use tx args. |
| Error naming | `#412` shared with flash Wasm-receiver failures; integrators must branch on context. |
| Default `None` | Integrators and UIs that omit `to` pay the **caller**. A delegate automation that forgets `Some(owner)` silently pays the bot — pinned by harness. |
| Batch | One `to` for the whole vec; cannot split recipients per asset in one call. |
| Docs / skills | endpoints.md, controller README, integrating skill all state pool/controller ban and owner/delegate + optional `to`. |

---

## 7. Evidence table

| Claim | Evidence |
|---|---|
| Auth before payout | `debt.rs:42-49`, `supply.rs:168-175`; A003 path matrix |
| Denylist before cash | `require_external_recipient`; harness reserves unchanged on borrow-to-pool |
| Delegate may take funds via `to`/`None` | threat-model.md ~285-302; `borrow.rs` delegated tests; A005 impact line |
| Stranger cannot | `refutation_third_party_cannot_borrow_on_victim`; INV-AUTH-02 |
| Certora recipient refusal | `market_guard_rules.rs` `borrow_rejects_pool_recipient`, `withdraw_rejects_controller_recipient` + satisfy twins; confs `market-guard-reverts*.conf` |
| Integration | `tests/integration/flows/admin.sh` `gh17_borrow_to_*` / `gh17_withdraw_to_*` |
| Pool trusts controller-chosen receiver | `pool/src/lib.rs` `#[only_owner]` borrow/withdraw; no recipient assert |
| No recipient in events | `events/` / `docs/reference/events.md` batch schema |

---

## 8. Residual register (not novel criticals)

| ID | Residual | Severity | Disposition |
|---|---|---|---|
| R1 | Delegate/owner drains via `to` | info (accepted) | Document; revoke delegate / transfer NFT |
| R2 | Non-denylisted protocol contracts as `to` | info | Optional denylist widen; UI warnings |
| R3 | Events omit recipient | info | Index tx args; optional event field later |
| R4 | `#412` name overload | info | Docs already list borrow/withdraw |
| R5 | Internal `execute_withdraw_all` skips denylist | info | Keep callers on `caller`; assert if new legs added |
| R6 | Listed-token hook reentrancy on payout | low (shared A007) | Listing trust / optional flash flag on plain borrow-withdraw |
| R7 | FOT under-delivery to `to` | info (A055) | Listing policy |

No R* is a stranger hijack of another account’s `to`.

---

## 9. Cross-links

| Peer | Relation |
|---|---|
| A003 / A005 | Owner-or-delegate is the real authorization boundary for `to` power |
| A011 | Explicit: `to`/`receiver` need not auth when receiving |
| A019 / A044 | Parallel controller/pool bans on flash receivers; cash flash relies on missing callback |
| A023 / A024 | Call out recipient gate; defer hijack analysis to A057 |
| A041 | User payouts unmeasured at recipient — trust pool+token |
| A048 | Explicitly out-of-scopes “ordinary withdraw recipient hijack” to this agent |
| A055 | Lying token / stranding interaction with denylist |

---

## 10. Opinion

**Defended** against the hijack class that matters for protocol safety: unauthorized redirection of someone else’s borrow/withdraw proceeds, and payouts to the two addresses that corrupt pool cash or controller measurement.

The optional `to` is a deliberate composition feature (pay a vault, a router EOA, a payment split contract). Its danger is **confused or malicious authority** (owner, delegate, or a buggy integrator that holds a grant), not an open ABI substitution. Treat R1 as user-education / delegate hygiene, not as an unfixed critical in A057.
