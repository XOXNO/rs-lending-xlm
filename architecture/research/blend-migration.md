# One-Click Migration: Blend → rs-lending-xlm

Status: research / design. No code written. Feasibility: **YES, in two tiers.**
Date: 2026-06-03.

Citations: `pool/...` = Blend v2 (`github.com/blend-capital/blend-contracts-v2`); bare
`contracts/...` = this repo.

---

## 1. Verdict

| Migration class | Feasible | Mechanism |
|---|---|---|
| **Collateral-only** (user has Blend supply/collateral, no debt) | Yes | Blend `submit` withdraw → our on-behalf `supply`. No flash/borrow needed. |
| **Debt-carrying** (user has Blend collateral + debt) | Yes | New strategy entry point on our controller using `pool.create_strategy` (the internal flash-loan-equivalent) → adapter repays Blend + reclaims collateral → `process_deposit` credits user → health check at finalize. |

Two hard prerequisites gate **implementation** (not the design):
1. **Soroban auth-tree construction** for Blend's `submit(from=U, …)` — the one piece the existing `multiply` analogy does *not* cover. Must be de-risked with a testnet spike first.
2. **Fee decision**: `create_strategy` charges `flashloan_fee` on the full borrowed principal. Migrated debt would pay a flash fee on the entire migrated amount — a migration disincentive. Needs a fee-exempt (or reduced) migration borrow leg.

The user's "diamond contract" intuition maps to an **orchestrating adapter/router**, not an EIP-2535 upgradeable-facet proxy. On Soroban the facet pattern isn't the point; we build a thin Blend adapter that occupies the same "untrusted router" role the swap aggregator plays in `multiply` today.

---

## 2. Blend mechanics (relevant subset)

### 2.1 The one entry point: `submit`
```rust
fn submit(e, from: Address, spender: Address, to: Address, requests: Vec<Request>) -> Positions
// pool/src/contract.rs:116, auth at :459-465
struct Request { request_type: u32, address: Address, amount: i128 } // actions.rs:11
// RequestType: Withdraw=1, WithdrawCollateral=3, Repay=5 (actions.rs:21-34)
```
- `from` = whose positions change. `spender` = who funds tokens in. `to` = who receives tokens out (**`to` is never authenticated**).
- Auth: `spender.require_auth()`; **`from.require_auth()` whenever `from != spender`** (contract.rs:461-463). **No delegate/operator model — U's signature is mandatory.** This is the central constraint.
- Token routing is flexible: `from=U, spender=adapter, to=adapter` → adapter funds the repay, adapter receives the withdrawn collateral.

### 2.2 Three properties that make migration clean
1. **Health checked once, at the end, and skipped if no liabilities remain** (submit.rs:159-196). A full exit (repay all debt + withdraw all collateral) in one `submit` ends with empty `liabilities` ⇒ `has_liabilities()` false ⇒ HF and `min_collateral` checks skipped entirely. Transient mid-sequence "insolvency" is never evaluated. Partial migrations must end HF ≥ `1.0000100`.
2. **Over-repay / over-withdraw are safe** — Blend caps to actual debt/balance and refunds the excess underlying to `to` (actions.rs:372-377, 426-434). The migration adapter can pass `Repay = debt + buffer` and `WithdrawCollateral = i128::MAX`; Blend settles to exact and refunds dust. **This is what rescues the auth-amount-pinning problem** (§5.3): U signs upper-bound amounts that survive interest accrual between sign and execute.
3. **Repay/Withdraw/WithdrawCollateral are not gated by pool status** (pool.rs:75-78) — migration works even on a frozen/distressed Blend pool. Caveat: an active liquidation auction on U blocks `submit` (#1212) and can only be cancelled (type 9) when pool status > 1.

### 2.3 What does NOT help
- Blend's own `flash_loan` only authenticates a single `from` (contract.rs:491) — it cannot let our adapter act on U. Use plain `submit`.
- **Repay capital must be sourced upfront.** Blend settles all transfers *after* validation (submit.rs:55-59), so the withdrawn collateral is not available mid-call to fund the repay. The repay capital comes from our `create_strategy` borrow (§4.2).

### 2.4 Reading positions (detection)
- `get_positions(e, U) -> Positions` (contract.rs:448) → `{ liabilities, collateral, supply }`, each `Map<reserveIndex(u32), shares>`.
- `get_reserve_list() -> Vec<Address>` (contract.rs:439) — **vec index == reserve index** in the maps.
- `get_reserve(e, asset) -> Reserve` (contract.rs:443) — returns live `b_rate`/`d_rate` (12-dec) to convert shares→underlying. Simulate this off-chain for current values; stored `ResData` is stale until accrued.

---

## 3. Our protocol mechanics (relevant subset)

- **controller** = sole user entry point + risk/strategy/oracle. **pool** = per-asset custody + accounting, every mutator `#[only_owner]` (owner = controller). Positions keyed by `account_id: u64`, `AccountMeta.owner: Address` (`contracts/controller/src/storage/account.rs`). No operator/delegate model.
- **Public `supply`** ties payer = creditee = `caller` (`supply.rs:44,242`). **Public `borrow`/`withdraw`** require `account.owner == caller` (`borrow.rs:99`, `withdraw.rs:61`). So **no public entry point lets a middleman build U's position** — by design.
- **But the payer≠creditee split already exists internally**: `process_deposit(caller=payer, account=creditee, Vec<Payment>, cache)` (`supply.rs:103`) is multi-asset and is used by `multiply` with **controller-as-payer, user-as-creditee** (`multiply.rs:151-159`).
- **`pool.create_strategy`** (`pool/src/lib.rs:287`) is the internal flash-loan-equivalent: mints scaled debt onto a position, sends `amount − fee` to the controller, records `fee` as revenue, enforces `require_reserves` + `require_utilization_below_max` + `enforce_borrow_cap`. **It does NOT set the flash-loan reentrancy guard** — which is exactly why `multiply` can borrow-then-supply in one tx.
- **Public `flash_loan` cannot be used.** It sets `FlashLoanOngoing`; every position verb opens with `require_not_flash_loaning`, so re-entering `supply`/`borrow` inside the callback panics (error 400). Test-verified: `flash_loan_tests.rs:522-548`. The naive "flash-loan → repay → pull → supply → borrow" loop is structurally impossible inside our public flash loan.
- **`multiply` is the template** (`multiply.rs:36-175`): one `caller.require_auth()` → load/create account → `open_strategy_borrow` (proceeds to controller, no owner-match, no per-call health) → external router swap with balance-delta verification (ADR 0005) → `process_deposit` (controller as payer) → `strategy_finalize` re-checks HF/LTV/dust. **Health deferred to finalize** — exactly migration's shape.
- **Fee**: `create_borrow_strategy` applies `debt_config.flashloan_fee` to the full `amount` (`borrow.rs:65`) → recorded as revenue. ⚠️ migrated debt would pay this on the entire principal.
- **Liquidity sizing**: pool view `reserves()` (`pool/src/views.rs:29`) = live token balance; borrow bounded by `require_reserves` AND `require_utilization_below_max` AND `borrow_cap` (`pool/src/lib.rs:302-312`).

---

## 4. Architecture

### 4.0 Components
1. **Blend adapter** (new contract, the "diamond"/router): stateless, holds the Blend `submit` call construction. Occupies the untrusted-router role; the controller verifies its effect by balance-delta, never by trusting its return. Keeps the controller Blend-agnostic and preserves the ADR boundary (pools never call external; controller calls external routers).
2. **New controller entry point `migrate`** (strategy-style): orchestrates the borrow-equivalent + adapter call + on-behalf deposit + finalize. Modeled 1:1 on `multiply`.
3. **Frontend detection + auth-tree builder** (the gating spike).

### 4.1 Tier 1 — collateral-only (ship first)
No debt, no `create_strategy`, no fee question. De-risks the auth-tree work in isolation.

Single tx, one user signature covering the whole auth tree:
1. Adapter calls Blend `submit(from=U, spender=adapter, to=adapter, [WithdrawCollateral(asset, MAX) for each collateral])` → collateral underlying lands at adapter (or pass `to = our pool` if SAC transfer auth allows; simplest is `to = controller`).
2. Controller verifies received amounts by balance-delta.
3. Controller `process_deposit(payer = controller, account = U's, [(asset, amount)…])` credits U.
4. `strategy_finalize` (no debt ⇒ trivially healthy).

New public entry point shape:
```rust
fn migrate_collateral(env, user: Address, account_id: u64, e_mode_category: u32,
                      blend_pool: Address, adapter: Address,
                      collateral: Vec<Address>) -> u64
// user.require_auth() once; rest of the tree authorized beneath it.
```

### 4.2 Tier 2 — debt-carrying (the borrow-strategy loop)
Mirrors `multiply`, replacing the swap with a Blend round-trip. Order matters: **borrow-equivalent first, repay+pull second, deposit third, health last.**

For each (debt asset D, total collateral set):
1. `open_strategy_borrow(account=U, D, amount = blendDebt(D) + buffer)` via `create_borrow_strategy` → `pool.create_strategy`. Mints U's **migrated debt** in our protocol; sends D underlying to the controller. No reentrancy guard, no per-call health.
2. Controller routes D to the **adapter**, which calls Blend `submit(from=U, spender=adapter, to=adapter, [Repay(D, amount), WithdrawCollateral(C, MAX) for each C])`. Blend repays U's debt, refunds dust to adapter, releases U's collateral to adapter.
3. Controller verifies reclaimed collateral by balance-delta (ADR 0005), refunds/repays the D dust against U's fresh debt (or sweeps as revenue — decision §6).
4. `process_deposit(payer = controller, account = U, reclaimed collateral set)` credits U's collateral here.
5. `strategy_finalize` — now U has both collateral and debt locally; HF/LTV/dust validated. Must end HF ≥ our minimum.

Net result: U's debt + collateral now live in our protocol, owned by U, in one atomic tx. The migrated debt is the `create_strategy` debt — backed by the just-deposited collateral, exactly the "borrow strategy where it remains as debt in the user position" the user described.

```rust
fn migrate(env, user: Address, account_id: u64, e_mode_category: u32,
           blend_pool: Address, adapter: Address,
           debt: Vec<(Address, i128)>,        // asset -> repay upper bound
           collateral: Vec<Address>) -> u64
// user.require_auth() once.
```

**v1 scope recommendation:** `process_deposit` is already multi-collateral; `create_borrow_strategy` is single-asset (loop per debt asset). Support multi-collateral + multi-debt by iterating the borrow leg. If schedule-constrained, ship single-debt/single-collateral first, but the Vec shapes above are the target.

---

## 5. The hard part: Soroban auth tree

The `multiply` analogy does **not** cover auth. Multiply's external router trades the *controller's* funds (`authorize_as_current_contract`); there is no foreign `require_auth` inside it. Blend's `submit` calls **`U.require_auth()` inside the external call** — a different beast and the riskiest piece of the feature.

### 5.1 What U must sign
A pre-built `SorobanAuthorizationEntry` for U whose authorized invocation matches Blend's `submit` with **exact args**: `from=U, spender=adapter, to=adapter`, and the **full `requests` Vec including amounts**. Soroban matches the signed auth entry against the actual sub-invocation argument-for-argument.

### 5.2 What the controller/adapter authorize themselves
- `spender = adapter` leg and the nested token `transfer`/`transfer_from` to Blend: authorized via `env.authorize_as_current_contract(InvokerContractAuthEntry::Contract(...))` (pattern already in `strategies/helpers.rs:270-287` and `flash-loan-receiver/src/lib.rs:134-158`).
- The controller **cannot** authorize U's leg. Only U's signature does.

### 5.3 Amount pinning (solved by Blend's capping)
Interest accrues between sign-time and execute-time, so the exact debt is unknown when U signs. Pin **upper bounds**: `Repay = debt + buffer`, `WithdrawCollateral = i128::MAX`. Blend caps to actual and refunds excess (§2.2). The signed upper-bound args remain valid as the true amounts drift upward with accrual. These two findings — Blend's capping and the auth-pinning need — are the same mechanism; rely on it explicitly.

### 5.4 Mandatory de-risk
**Build a testnet spike that simulates the full auth tree (controller `migrate` → adapter → Blend `submit` with U's signed entry + the controller's self-authorized legs) before committing the contract design.** This is the thing most likely to not "just work." Spike first, then implement.

---

## 6. Open decisions (recommended defaults)

| Decision | Recommendation |
|---|---|
| **Migration borrow fee** | Add a **fee-exempt migration borrow leg** (or a separate, low `migration_fee` Bps distinct from `flashloan_fee`). Charging the full flash fee on migrated principal is a direct disincentive. Default: fee-exempt for the migrate path; revisit if abused. |
| **`to` routing on Blend** | Route to **controller** (`to = controller`) for uniform balance-delta accounting; simpler than `to = pool` and avoids per-asset SAC auth surprises. |
| **Repay dust refund** | Re-apply the refunded D dust against U's fresh local debt (cleanest for the user) rather than sweeping as revenue. |
| **Tiering** | Ship **Tier 1 (collateral-only) first** — de-risks auth tree without the borrow/fee complexity. Tier 2 second. |
| **Multi-asset v1** | Target multi-collateral + multi-debt (loop the borrow leg). Single/single only if schedule forces it. |
| **"Diamond" framing** | Orchestrating **adapter/router**, not an upgradeable-facet proxy. Confirmed deliberate. |
| **Pool discovery (frontend)** | Maintain a registry of canonical Blend pools (or read pool-factory). For each, `get_positions(U)`; gate migrate on: collateral asset listed here AND (for debt) debt asset listed + liquid (`reserves()` ≥ amount, under max-util, under `borrow_cap`). |

---

## 7. Constraints / blockers checklist

- U's signature is **non-negotiable** (Blend has no delegation). "Migrate someone else's position permissionlessly" is impossible — fine for our use case.
- Debt asset must be **listed + liquid** in our pool; collateral asset must be **listed** here. Gate in frontend.
- Active Blend liquidation auction on U blocks `submit` (only cancellable when Blend pool status > 1). Surface as "not migratable now."
- Blend positions keyed by **reserve index**, not asset — resolve via `get_reserve_list()` before reading.
- Our public `flash_loan` is unusable for this (reentrancy guard). Use `create_strategy` only.
- `from`/`spender`/`to` cannot be the Blend pool address (#1200).

---

## 8. Build sequence

1. **Spike**: testnet auth-tree simulation for Tier 1 (collateral-only) round-trip. Gate everything on this.
2. Blend adapter contract (stateless `submit` builder) + frontend detection (`get_positions` + reserve-list + rate conversion).
3. Controller `migrate_collateral` entry point (Tier 1) reusing `process_deposit` (controller-as-payer) + `strategy_finalize`.
4. Tier 2: `migrate` entry point reusing `create_borrow_strategy` (with fee-exempt leg) + adapter repay/pull + `process_deposit` + `strategy_finalize`; loop for multi-debt.
5. **Certora harness**: new heavy strategy entry points need a nondet summary (see project memory `risk_premium_phase1`); after refactors, **re-sync the certora tree** (mirrors prod ABI — memory `certora_build_broken`).
6. Verify: `cargo check/clippy/test` per-crate (multi-crate clippy quirk, memory `clippy_multicrate_quirk`); fuzz reference + certora mirror for the new strategy.

---

## 9. Citations index
Blend: `pool/src/contract.rs:116,439-465`; `pool/src/pool/actions.rs:11-63,372-434`; `pool/src/pool/submit.rs:55-196`; `pool/src/pool/user.rs:9-15`; `pool/src/pool/status.rs:542-560`.
Ours: `contracts/controller/src/positions/supply.rs:103-249`; `.../positions/borrow.rs:42-99`; `.../positions/withdraw.rs:61`; `.../strategies/multiply.rs:36-175`; `.../strategies/helpers.rs:149-197,270-287`; `contracts/pool/src/lib.rs:124-312`; `contracts/flash-loan-receiver/src/lib.rs:74-208`; `verification/test-harness/tests/flash_loan_tests.rs:522-548`; ADRs 0001/0005/0006.
