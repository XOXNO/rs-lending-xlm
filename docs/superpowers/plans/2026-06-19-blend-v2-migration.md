# Blend V2 → XOXNO Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. This plan is written for execution by the context-holding author (inline); a fresh worker should first read the design spec `docs/superpowers/specs/2026-06-19-blend-v2-migration-design.md` and the cited template files.

**Goal:** Add one controller entrypoint `migrate_from_blend` that atomically moves a user's full Blend V2 position (collateral, non-collateral supply, debt) into our protocol in a single transaction at zero flash-loan fee.

**Architecture:** A new in-controller strategy (`strategies/migrate_blend.rs`) composing existing primitives: a fee=0 `create_strategy` borrow (the "flash loan"), one Blend `submit` (Repay all + WithdrawCollateral all + Withdraw all), `process_deposit` of the swept assets into the user's account, and `strategy_finalize` for the single end-state health gate. The Blend ABI surface is **just `submit`** — the explicit `max_debt` cap is used as the repay amount and Blend's over-repay refund is reconciled back into the user's new debt, eliminating any Blend `get_positions`/`get_reserve` reads.

**Tech Stack:** Rust, `soroban-sdk = 26.1.0`, OZ stellar-* `0.7.2`, workspace edition 2021.

## Global Constraints

- Verification bar (per repo memory `verification-bar-no-all-features`): use `--workspace`, **never** `--all-features` (enables `certora`, breaks linking). Gate for controller-only feature-unified tests: `cargo test -p controller --lib`.
- `cargo fmt` must use `--edition 2021` (memory `sec-gov-split-low-hardenings`); fmt of `-p controller` reformats cfg-gated certora `#[path]` files — revert unrelated churn (memory `cargo-fmt-formats-cfg-gated-path-modules`).
- `bash cd` trips a gvm hook (memory `bash-cd-trips-gvm-hook`): use absolute paths / `cargo --manifest-path`, never `cd` in tool calls.
- WASM cap: controller deploys ~134 KB vs 140 KB cap; the cap checks wasm+100B entry (memory `wasm-cap-entry-overhead-strip-levers`). Each `contracterror` variant ≈ 57 B (memory `contracterror-variants-cost-wasm`) — reuse variants, add only what's new.
- `#[cfg]`-gated contract methods need their own `#[contractimpl]` block (memory `cfg-gated-method-needs-own-contractimpl`).
- `authorize_as_current_contract` covers only the NEXT sub-invocation tree (memory `defindex-strategy-e2e-and-auth-fix`): emit it immediately before `blend.submit`, with no cross-call in between.
- Only SAC tokens are safe (no fee-on-transfer defense, memory `transfer-measure-received-name-lies`); migration assumes exact-amount SAC semantics.
- New controller types live in `controller_interface::types`, not `common` (memory `controller-types-live-in-interface-crate`). Shared throwing guards live in `common::validation` (memory `common-validation-shared-guards`).
- ABI parity: strategies appear in the `ControllerInterface` trait (`interfaces/controller/src/lib.rs`); the harness `tests/integration` and `governance-interface` mirror production entrypoints (memories `governance-interface-crate`, `integration-harness-abi-repair-gov-e2e`). Adding a production entrypoint may require updating those mirrors / count assertions.

## File Structure

| File | Responsibility |
|------|----------------|
| `contracts/controller/src/external/blend.rs` | **New.** Blend `Request`/`Positions` mirrors, `BlendPoolClient` (`submit` only), `blend_submit_call` wrapper, request-type consts. |
| `contracts/controller/src/external/mod.rs` | Register `mod blend;` with the certora harness `#[path]` swap (mirroring `pool`/`sac`). |
| `certora/controller/harness/external/blend.rs` | **New.** Certora harness stub for the Blend client (no-op / minimal), mirroring `harness/external/pool.rs`. |
| `contracts/controller/src/positions/borrow.rs` | Add `borrow_for_migration` (fee=0) by extracting a fee+action-parameterized inner from `borrow_for_strategy`. |
| `contracts/controller/src/strategies/positions.rs` | Add `open_migration_borrow` wrapper. |
| `contracts/controller/src/strategies/mod.rs` | `mod migrate_blend;` + export `open_migration_borrow`. |
| `contracts/controller/src/strategies/migrate_blend.rs` | **New.** `migrate_from_blend` entrypoint + `process_migrate_blend` + the Blend auth/submit helper. |
| `contracts/controller/src/events.rs` | Add `PositionAction::Migrate = 13` + `BlendMigrationEvent`. |
| `contracts/controller/src/strategies/migrate_blend_mock.rs` (or test module) | **New, test-only.** Blend mock pool faithfully mimicking `submit` semantics for unit tests. |
| `interfaces/controller/src/lib.rs` | Add `migrate_from_blend` to `ControllerInterface`. |
| `common/src/errors.rs` | Add `MigrationDebtCapExceeded` (+ reuse existing for unsupported/empty). |
| `tests/integration/flows/migrate_blend.sh` | **New (Task 6).** E2E lane against a deployed Blend mock. |

---

### Task 1: Blend external interface (`external/blend.rs`)

**Files:**
- Create: `contracts/controller/src/external/blend.rs`
- Create: `certora/controller/harness/external/blend.rs`
- Modify: `contracts/controller/src/external/mod.rs`

**Interfaces:**
- Produces: `blend::{BlendRequest, BlendPositions, BlendPoolClient, blend_submit_call, REQ_WITHDRAW, REQ_WITHDRAW_COLLATERAL, REQ_REPAY}`.

- [ ] **Step 1: Write `external/blend.rs`**

```rust
//! Blend V2 pool client for one-click migration.
//!
//! Mirrors only the Blend ABI surface migration uses: `submit`. `Request` and
//! `Positions` field NAMES must match Blend (`#[contracttype]` map encoding);
//! see blend pool `actions.rs:13-17` and `user.rs:8-15`. We never read Blend
//! positions/reserves — the migration uses Blend's over-repay refund instead.

use soroban_sdk::{contractclient, contracttype, Address, Env, Map, Vec};

/// Blend `RequestType` discriminants (blend `actions.rs:22-33`). Only these three are emitted.
pub const REQ_WITHDRAW: u32 = 1; // sweep non-collateral supply
pub const REQ_WITHDRAW_COLLATERAL: u32 = 3; // sweep collateral
pub const REQ_REPAY: u32 = 5; // clear debt

/// Mirror of blend `pool/src/pool/actions.rs:13-17`.
#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

/// Mirror of blend `pool/src/pool/user.rs:8-15`. Returned by `submit`; decoded
/// for type-fidelity then discarded (migration measures balance deltas instead).
#[contracttype]
#[derive(Clone)]
pub struct BlendPositions {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

#[allow(dead_code)] // Generates the Soroban client proxy.
#[contractclient(name = "BlendPoolClient")]
pub trait BlendPool {
    fn submit(
        env: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<BlendRequest>,
    ) -> BlendPositions;
}

/// Calls Blend `submit`. The caller MUST have emitted `authorize_as_current_contract`
/// for the controller's `spender` legs immediately before this call (no intervening
/// cross-call), and the user must have authorized the `from` leg in the tx auth tree.
pub(crate) fn blend_submit_call(
    env: &Env,
    blend_pool: &Address,
    from: &Address,
    spender: &Address,
    to: &Address,
    requests: &Vec<BlendRequest>,
) -> BlendPositions {
    BlendPoolClient::new(env, blend_pool).submit(from, spender, to, requests)
}
```

- [ ] **Step 2: Write the certora harness stub** `certora/controller/harness/external/blend.rs`

Mirror the shape of `certora/controller/harness/external/pool.rs` (read it first). Re-export the same public items so `crate::external::blend::*` resolves under the `certora` feature; `blend_submit_call` may return a default `BlendPositions { liabilities: Map::new(env), collateral: Map::new(env), supply: Map::new(env) }` without a real cross-call. Keep the type mirrors identical.

- [ ] **Step 3: Register the module** — edit `contracts/controller/src/external/mod.rs`, appending:

```rust
#[cfg(not(feature = "certora"))]
pub(crate) mod blend;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/blend.rs"]
pub(crate) mod blend;
```

- [ ] **Step 4: Compile**

Run: `cargo check -p controller`
Expected: PASS (dead-code allowed on the client trait).

- [ ] **Step 5: Commit**

```bash
git add contracts/controller/src/external/blend.rs contracts/controller/src/external/mod.rs certora/controller/harness/external/blend.rs
git commit -m "feat(migration): add Blend V2 pool client (submit-only)"
```

---

### Task 2: Blend mock pool (test fixture)

**Files:**
- Create: `contracts/controller/src/strategies/migrate_blend_mock.rs` (gated `#[cfg(test)]`)

**Interfaces:**
- Produces: a registrable mock contract `BlendMock` with `submit(from, spender, to, requests) -> Positions` and a seeding helper `seed(env, pool_id, user, asset, kind, amount)` where `kind ∈ {Collateral, Supply, Liability}`.

**Mock semantics to reproduce (from blend `actions.rs`/`submit.rs`):**
- `spender.require_auth()`; if `from != spender`, `from.require_auth()`.
- Per request, by `request_type`:
  - `REQ_REPAY (5)`: pull `amount` from `spender` (`token.transfer(spender, pool, amount)`); reduce `from`'s liability by `min(amount, debt)`; if `amount > debt`, refund `amount - debt` to `to` (`token.transfer(pool, to, excess)`).
  - `REQ_WITHDRAW_COLLATERAL (3)`: `out = min(amount, collateral[from,asset])`; reduce; `token.transfer(pool, to, out)`.
  - `REQ_WITHDRAW (1)`: same against the `supply` balance.
- Store balances in a `Map<(Address,Address), i128>` per kind (use `#[contracttype]` enum keys).
- Final health check: skip when `from` has no remaining liabilities (mirrors Blend `has_liabilities()` gate) — enough for our tests since migration always clears all debt before withdrawing.

- [ ] **Step 1: Write the mock contract** with the semantics above (SAC transfers via `soroban_sdk::token::Client`; the pool must hold the underlying liquidity, seeded in tests). Include the seeding helper.

- [ ] **Step 2: Self-test the mock**

```rust
// In the mock's #[cfg(test)] section: seed collateral, call submit WithdrawCollateral(MAX),
// assert the user's tokens land at `to` and the collateral balance is zeroed; seed debt +
// fund spender, call Repay(over-amount), assert debt cleared and excess refunded to `to`.
```

Run: `cargo test -p controller --lib migrate_blend_mock`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add contracts/controller/src/strategies/migrate_blend_mock.rs contracts/controller/src/strategies/mod.rs
git commit -m "test(migration): add Blend mock pool fixture"
```

---

### Task 3: Zero-fee migration borrow + event + action

**Files:**
- Modify: `contracts/controller/src/positions/borrow.rs:148-185`
- Modify: `contracts/controller/src/strategies/positions.rs`
- Modify: `contracts/controller/src/strategies/mod.rs`
- Modify: `contracts/controller/src/events.rs:291-305` (enum) + new event

**Interfaces:**
- Produces: `borrow::borrow_for_migration(env, account, debt_token, amount, cache) -> i128`; `strategies::open_migration_borrow(...)`; `events::PositionAction::Migrate`; `events::BlendMigrationEvent`.

- [ ] **Step 1: Parameterize the strategy borrow.** In `borrow.rs`, replace `borrow_for_strategy` with a thin wrapper over a fee+action-parameterized inner (preserves existing behavior byte-for-byte for `borrow_for_strategy`):

```rust
pub fn borrow_for_strategy(
    env: &Env, account: &mut Account, debt_token: &Address, amount: i128, cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(env, account, debt_token, amount, cache, None, events::PositionAction::Multiply)
}

/// Zero-fee strategy borrow for migration: the opened debt is the user's
/// permanent migrated debt, not a flash loan, so no flash fee is charged.
pub fn borrow_for_migration(
    env: &Env, account: &mut Account, debt_token: &Address, amount: i128, cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(env, account, debt_token, amount, cache, Some(0), events::PositionAction::Migrate)
}

fn borrow_strategy_inner(
    env: &Env, account: &mut Account, debt_token: &Address, amount: i128, cache: &mut Cache,
    fee_override: Option<i128>, action: events::PositionAction,
) -> i128 {
    let mut payments: AggregatedPayments = Vec::new(env);
    payments.push_back((debt_token.clone(), amount));
    let aggregated = utils::aggregate_positive_payments(env, &payments);
    let configs = AggregatedConfigs::resolve(env, account, &aggregated, cache);
    validate_borrow(env, account, &aggregated, &configs, cache);

    let debt_config = configs.get(env, debt_token);
    let flash_fee = fee_override
        .unwrap_or_else(|| debt_config.flashloan_fee.flash_loan_fee_on(env, amount));
    let borrow_position = account.get_or_create_debt_position(debt_token);

    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&borrow_position, amount, debt_token.clone());
    let result = pool_create_strategy_call(
        env, &pool_addr, &env.current_contract_address(), pool_action, flash_fee, debt_config.borrow_cap,
    );
    let mutation: PoolPositionMutation = PoolPositionMutation::from(&result);
    merge_borrow_result(account, debt_token, action, &mutation, cache);
    result.amount_received
}
```

- [ ] **Step 2: Add `PositionAction::Migrate = 13`** in `events.rs:291-305` (append after `CloseWd = 12`). Note: additive enum variant; sdk-js decoder follow-up (memory `lending-types-sdk-release-chain`).

- [ ] **Step 3: Add the event** in `events.rs` (mirroring `InitialMultiplyPaymentEvent`):

```rust
#[contractevent(topics = ["strategy", "blend_migration"])]
#[derive(Clone, Debug)]
pub struct BlendMigrationEvent {
    pub account_id: u64,
    pub blend_pool: Address,
    pub collateral_count: u32,
    pub supply_count: u32,
    pub debt_count: u32,
}
```

- [ ] **Step 4: Add `open_migration_borrow`** in `strategies/positions.rs`:

```rust
pub(crate) fn open_migration_borrow(
    env: &Env, cache: &mut Cache, account: &mut Account, asset: &Address, amount: i128,
) -> i128 {
    borrow::borrow_for_migration(env, account, asset, amount, cache)
}
```
Export it from `strategies/mod.rs` alongside `open_strategy_borrow`.

- [ ] **Step 5: Compile + existing tests unaffected**

Run: `cargo check -p controller && cargo test -p controller --lib`
Expected: PASS (no behavior change to `borrow_for_strategy`).

- [ ] **Step 6: Commit**

```bash
git add contracts/controller/src/positions/borrow.rs contracts/controller/src/strategies/positions.rs contracts/controller/src/strategies/mod.rs contracts/controller/src/events.rs
git commit -m "feat(migration): zero-fee migration borrow + Migrate action/event"
```

---

### Task 4: `migrate_from_blend` strategy + ABI + tests

**Files:**
- Create: `contracts/controller/src/strategies/migrate_blend.rs`
- Modify: `interfaces/controller/src/lib.rs` (trait)
- Modify: `common/src/errors.rs` (`MigrationDebtCapExceeded`)

**Interfaces:**
- Consumes: `blend::{blend_submit_call, BlendRequest, REQ_*}`, `open_migration_borrow`, `repay_debt_from_controller`/`StrategyRepay`, `supply::process_deposit`, `strategy_finalize`, `prefetch_strategy_oracles`, `balance_delta`, `helpers::create_account`, `validation::*`, `Cache`.
- Produces: entrypoint `Controller::migrate_from_blend(...) -> u64`.

**Entrypoint signature (add to `ControllerInterface` trait verbatim):**
```rust
fn migrate_from_blend(
    env: Env,
    caller: Address,
    account_id: u64,
    e_mode_category: u32,
    blend_pool: Address,
    collateral_assets: Vec<Address>,
    supply_assets: Vec<Address>,
    debt_caps: Vec<(Address, i128)>,
) -> u64;
```

**`process_migrate_blend` flow (the deliverable code):**
1. `caller.require_auth()`; `validation::require_not_flash_loaning(env)`.
2. Validate not-all-empty (`collateral_assets`+`supply_assets`+`debt_caps`) → else `GenericError::InvalidPayments`. Validate `blend_pool` is a Wasm contract via `validation::require_wasm_receiver(env, &blend_pool)`. Validate no asset appears in more than one role (build a `Map<Address,bool> seen`; panic `GenericError::AssetsAreTheSame` on overlap).
3. `Cache::new(env, OraclePolicy::RiskIncreasing)`.
4. Load/create account (mode `PositionMode::Normal`): `account_id == 0 ? helpers::create_account(env, caller, e_mode_category, PositionMode::Normal) : (storage::get_account + require_account_owner_match)`.
5. For each involved asset: `validation::require_market_active(env, &mut cache, &asset)` (also the implicit "supported" signal); supply/collateral assets additionally require `cache.cached_asset_config(asset).can_supply()` else `CollateralError::NotCollateral`.
6. Build `all_assets` (collateral ∪ supply ∪ debt) and `prefetch_strategy_oracles(&mut cache, &account, &all_assets)`.
7. Snapshot `B0[asset] = token.balance(controller)` for every involved asset (pre-borrow).
8. For each `(debt_asset, max)` in `debt_caps`: `require_positive_amount(env, max)`; `open_migration_borrow(env, &mut cache, &mut account, debt_asset, max)` (controller now holds `max`).
9. Build the Blend `Vec<BlendRequest>`: `Repay(debt, max)` ∀ debt, then `WithdrawCollateral(c, i128::MAX)` ∀ collateral, then `Withdraw(s, i128::MAX)` ∀ supply.
10. `authorize_blend_submit(env, &blend_pool, caller, &requests, &debt_caps)` — builds the `authorize_as_current_contract` tree (submit-as-spender + nested `transfer(controller, blend_pool, max)` per debt) **immediately** before:
11. `blend_submit_call(env, &blend_pool, caller, &controller, &controller, &requests)` (controller = `env.current_contract_address()`).
12. For each collateral/supply asset `a`: `delta = balance_delta(env, &token(a), B0[a])`; if `delta > 0` push `(a, delta)` to `deposit_assets`. `supply::process_deposit(env, &controller, &mut account, &deposit_assets, &mut cache)` (skip the call if empty).
13. For each `debt_asset`: `excess = balance_delta(env, &token(debt), B0[debt])` (Blend's over-repay refund); if `excess > 0`, `repay_debt_from_controller(env, &mut account, &mut cache, caller, StrategyRepay { debt_token: debt, debt_available: excess, debt_pos: &<loaded DebtPosition>, action: PositionAction::Migrate })` → nets the user's new debt down to the exact amount that cleared Blend.
14. `strategy_finalize(env, account_id, &mut account, &mut cache)` (the single health gate).
15. `BlendMigrationEvent { account_id, blend_pool, collateral_count, supply_count, debt_count }.publish(env)`.
16. Return `account_id`.

**`authorize_blend_submit` (mirror `swap::pre_authorize_router_pull`):**
```rust
fn authorize_blend_submit(
    env: &Env, blend_pool: &Address, user: &Address,
    requests: &Vec<BlendRequest>, debt_caps: &Vec<(Address, i128)>,
) {
    let controller = env.current_contract_address();
    let mut sub: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for (debt_asset, max) in debt_caps.iter() {
        sub.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: debt_asset.clone(),
                fn_name: symbol_short!("transfer"),
                args: (controller.clone(), blend_pool.clone(), max).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }));
    }
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: blend_pool.clone(),
            fn_name: symbol_short!("submit"),
            args: (user.clone(), controller.clone(), controller.clone(), requests.clone()).into_val(env),
        },
        sub_invocations: sub,
    });
    env.authorize_as_current_contract(soroban_sdk::vec![env, entry]);
}
```

- [ ] **Step 1: Add `MigrationDebtCapExceeded`** to `common/src/errors.rs` (next free discriminant in the relevant error enum; reuse `GenericError::AssetNotSupported`/`PairNotActive` for unsupported markets, `InvalidPayments` for empty, `AssetsAreTheSame` for role overlap). `cargo check -p common`.

- [ ] **Step 2: Add the trait method** to `interfaces/controller/src/lib.rs` (signature above). `cargo check -p controller-interface`.

- [ ] **Step 3: Write the failing test (debt-free flow)** in `migrate_blend.rs` `#[cfg(test)]`, using the Task-2 mock + the existing pool wasm fixture + controller. Seed a Blend collateral + supply position; call `client.migrate_from_blend(caller, 0, 0, blend, [coll], [sup], [])`; assert: account created, our supply positions credited with the swept amounts, Blend balances zeroed, `health_factor == i128::MAX` (no debt).

Run: `cargo test -p controller --lib migrate_blend::tests::debt_free`
Expected: FAIL (not implemented).

- [ ] **Step 4: Implement `migrate_from_blend` + `process_migrate_blend` + `authorize_blend_submit`** per the flow above (entrypoint in its own `#[contractimpl] impl Controller` block).

Run: `cargo test -p controller --lib migrate_blend::tests::debt_free`
Expected: PASS

- [ ] **Step 5: Add the remaining tests** (one behavior each):
  - `debt_flow`: seed XLM collateral + USDC debt on Blend; `debt_caps = [(USDC, blend_debt + buffer)]`; assert end state = XLM collateral + USDC debt here, user debt == actual Blend debt (refund reconciled), Blend emptied, healthy.
  - `debt_cap_exceeded`: `debt_caps` max < actual Blend debt → Blend partial-repay + collateral withdraw → mock health check reverts (tx reverts). Assert revert.
  - `unsupported_asset`: an involved asset has no active market here → revert `PairNotActive`/`AssetNotSupported`.
  - `role_overlap`: same asset in collateral and debt → revert `AssetsAreTheSame`.
  - `unhealthy_end_state`: debt too large vs migrated collateral → revert at `strategy_finalize` (`InsufficientCollateral`).
  - `auth_shape`: with targeted `mock_auths` (NOT `mock_all_auths`, per memory `defindex-strategy-e2e-and-auth-fix`), prove `caller` signs `migrate_from_blend` + nested `submit(from=caller,…)`, and the controller's invoker-auth covers `submit`(spender) + the repay `transfer`.

Run: `cargo test -p controller --lib migrate_blend`
Expected: PASS (all)

- [ ] **Step 6: Commit**

```bash
git add contracts/controller/src/strategies/migrate_blend.rs contracts/controller/src/strategies/mod.rs interfaces/controller/src/lib.rs common/src/errors.rs
git commit -m "feat(migration): migrate_from_blend strategy (debt-free + flash-borrow debt flow)"
```

---

### Task 5: ABI parity + full verification

**Files:** possibly `interfaces/governance/`, `tests/integration/*` (if parity assertions exist).

- [ ] **Step 1: ABI-parity check.** Search for production-entrypoint count assertions / mirrors:
  `grep -rn "migrate_from_blend\|propose_\|production entrypoint\|trait methods" interfaces/governance tests/integration` and check the governance-interface mirror (memory `governance-interface-crate`). If a count/mirror requires the new method, update it (migration is NOT governance-callable, so likely only a count needs adjusting — confirm with the failing test).
- [ ] **Step 2: Full verification bar.**

```bash
cargo check --workspace --all-targets
cargo clippy -p controller -p controller-interface -p common --all-targets -- -D warnings
cargo test -p controller --lib
cargo test --workspace
```
Expected: PASS (workspace test caveat: production-only oracle tests are cfg-gated under feature-unification — memory `workspace-test-feature-unification-failures`; the gate is `-p controller --lib`).

- [ ] **Step 3: WASM build + size.**

```bash
stellar contract build
ls -la target/wasm32v1-none/release/controller.wasm
```
Expected: builds; size under the 140 KB cap. If over, apply strip levers (memory `wasm-cap-entry-overhead-strip-levers` / `wasm-size-trial-matrix`).

- [ ] **Step 4: `cargo fmt --edition 2021`** on touched files; revert any cfg-gated certora `#[path]` churn.

- [ ] **Step 5: Commit** any parity/format fixes.

```bash
git add -A && git commit -m "chore(migration): ABI parity + verification fixes"
```

---

### Task 6 (optional): Integration E2E flow

**Files:** Create `tests/integration/flows/migrate_blend.sh` + register in the harness runner.

- [ ] Deploy a Blend mock + controller/pool on the chosen lane; seed a Blend position; run both flows; assert balances/positions/events; gate helpers as in existing lanes (memory `agg-lane-e2e-repair-local-aggregator`, `defindex-strategy-e2e-and-auth-fix`). Commit.

---

## Self-Review

**Spec coverage:** §1 goal → Task 4 flow. §2 D1 generic → caller asset lists. D2 cap → Task 4 step 8/13 (cap = borrow, refund-reconcile; note this **refines** spec §8's exact-ceil approach to a refund-reconcile that needs no Blend reads — update spec §5/§8). D3 supply → Task 4 step 12 (deposited as collateral; §10 consequence holds). D4 fee=0 → Task 3. D5 in-controller → all tasks. §9 auth → `authorize_blend_submit` + user tx-tree signature. §11 errors → Task 4 step 1. §12 tests → Task 4 step 5 / Task 6. §13 risks: Reserve-mirror risk #2 **eliminated** (submit-only interface); WASM risk #3 → Task 5 step 3; live-auth risk #1 → Task 6 + `auth_shape` unit test.

**Placeholder scan:** none — code blocks are complete for production files; the mock + tests are fully specified (semantics + assertions). The only deliberate "look up the exact discriminant" is the new error variant (codebase-specific free slot) and parity assertion (Task 5 step 1, gated on a real failing test).

**Type consistency:** `BlendRequest`/`BlendPositions` field names match Blend; `borrow_for_migration`/`open_migration_borrow` names consistent across Tasks 3–4; `PositionAction::Migrate` used in both the borrow and the reconciliation repay; `blend_submit_call(env, pool, from, spender, to, requests)` signature consistent.

**Refinement to record in the spec:** update `2026-06-19-blend-v2-migration-design.md` §5 (Blend client = `submit` only) and §8 (refund-reconcile, not Reserve-read ceil) to match this plan.
