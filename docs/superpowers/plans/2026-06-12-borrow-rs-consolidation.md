# borrow.rs Structural Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure `contracts/controller/src/positions/borrow.rs` per the approved spec (`docs/superpowers/specs/2026-06-11-borrow-rs-consolidation-design.md`): top-down ordering, one name per concept, cache-last parameters, shared merge helper, strategy borrows routed through the real validation pipeline (gaining the position-limit gate — a bug fix), and `require_within_ltv` owning its RedStone prefetch.

**Architecture:** Pure single-file restructure plus three small ripples: a `From<&PoolStrategyMutation>` impl in `common`, one prefetch line in `validation.rs`, a 2-line caller update in `strategies/helpers.rs`, and mechanical renames in `supply.rs`/`validation.rs`/`withdraw.rs`. Behavior deltas are intentional and pre-mainnet-approved: panic *selection* may change on multi-fault plans, and strategy borrows newly hit `PositionLimitExceeded`.

**Tech Stack:** Rust / Soroban SDK (no_std contract), cargo workspace, `stellar` CLI for WASM builds, test-harness integration tests.

**Verification commands (used throughout):**
- `cargo check --workspace --all-targets` (from repo root `/Users/mihaieremia/GitHub/rs-lending-xlm`)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p controller --lib` (38 tests)
- `cargo test -p test-harness` (integration suites)

**Known pre-existing failures (NOT gates for this plan):** `budget_breakdown::budget_withdraw_5_collateral_double_pass` in `-p test-harness --test meta` (shadow-budget cliff); `-p defindex-strategy --test strategy` requires a fresh `target/wasm32v1-none/release/pool.wasm` (rebuild with `stellar contract build --manifest-path contracts/pool/Cargo.toml` if it fails with `MismatchingParameterLen`).

**IMPORTANT — `cd` is broken in this environment:** every `cargo`/`git` command below uses `--manifest-path` / `-C` instead of `cd` (a shell hook makes `cd` exit 1).

---

### Task 0: Commit the pending working-tree refactor (clean slate)

The tree carries an already-verified uncommitted refactor of the positions module (dedup of settle helpers, comment trims — green on check/clippy/tests earlier this session). Commit it so each task below can commit its own files cleanly.

**Files:**
- Modify: nothing (commit only)

- [ ] **Step 1: Confirm the tree state matches expectation**

Run: `git -C /Users/mihaieremia/GitHub/rs-lending-xlm status --short`
Expected: modified files limited to `configs/networks.json`, `contracts/controller/src/positions/*.rs`, `contracts/controller/src/strategies/helpers.rs`. If anything else is modified, STOP and ask the user.

- [ ] **Step 2: Verify green before committing**

Run: `cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings && cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: clippy clean; `test result: ok. 38 passed`

- [ ] **Step 3: Commit the positions refactor (NOT configs/networks.json)**

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/positions contracts/controller/src/strategies/helpers.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): dedup positions flows, settle helpers, comment trims"
```

- [ ] **Step 4: Record the WASM size baseline for Task 8**

```bash
stellar contract build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/contracts/controller/Cargo.toml > /dev/null 2>&1
stat -f%z /Users/mihaieremia/GitHub/rs-lending-xlm/target/wasm32v1-none/release/controller.wasm | tee /tmp/borrow_wasm_baseline.txt
```
Expected: a byte count (~137730).

---

### Task 1: `merge_borrow_result` helper + `From<&PoolStrategyMutation>` impl

Dedups the 10-line pool-result merge block that appears in `create_borrow_strategy` and `execute_borrow_plan`. Pure code motion; tests stay green.

**Files:**
- Modify: `common/src/types/pool.rs` (after the `PoolStrategyMutation` struct, ~line 356)
- Modify: `contracts/controller/src/positions/borrow.rs`

- [ ] **Step 1: Add the `From` impl in `common/src/types/pool.rs`**

Insert immediately after the closing brace of `pub struct PoolStrategyMutation { ... }`:

```rust
impl From<&PoolStrategyMutation> for PoolPositionMutation {
    fn from(m: &PoolStrategyMutation) -> Self {
        Self {
            position: m.position.clone(),
            market_index: m.market_index.clone(),
            market_state: m.market_state.clone(),
            actual_amount: m.actual_amount,
        }
    }
}
```

- [ ] **Step 2: Add `PoolPositionMutation` to borrow.rs imports**

In `contracts/controller/src/positions/borrow.rs`, change:

```rust
use common::types::{
    Account, AccountPositionType, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    PoolBorrowEntry,
};
```

to:

```rust
use common::types::{
    Account, AccountPositionType, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    PoolBorrowEntry, PoolPositionMutation,
};
```

- [ ] **Step 3: Add `merge_borrow_result` to borrow.rs**

Add this function after `execute_borrow_plan`:

```rust
/// Merges one pool borrow result into the account and event buffers.
fn merge_borrow_result(
    account: &mut Account,
    asset: &Address,
    action: common::events::PositionAction,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);
    let position: DebtPosition = (&result.position).into();
    cache.record_debt_position_update(
        action,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, asset, &position);
}
```

- [ ] **Step 4: Use it in `execute_borrow_plan`'s merge loop**

Replace the loop body:

```rust
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        cache.record_market_update(&result.market_state);
        let position: DebtPosition = (&result.position).into();
        cache.record_debt_position_update(
            common::events::PositionAction::Borrow,
            &entry.action.asset,
            result.market_index.borrow_index_ray,
            result.actual_amount,
            &position,
        );
        update_or_remove_debt_position(account, &entry.action.asset, &position);
    }
```

with:

```rust
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        merge_borrow_result(
            account,
            &entry.action.asset,
            common::events::PositionAction::Borrow,
            &result,
            cache,
        );
    }
```

- [ ] **Step 5: Use it in `create_borrow_strategy`**

Replace:

```rust
    cache.record_market_update(&result.market_state);
    let position: DebtPosition = (&result.position).into();
    cache.record_debt_position_update(
        common::events::PositionAction::Multiply,
        debt_token,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, debt_token, &position);

    result.amount_received
```

with:

```rust
    let mutation: PoolPositionMutation = (&result).into();
    merge_borrow_result(
        account,
        debt_token,
        common::events::PositionAction::Multiply,
        &mutation,
        cache,
    );

    result.amount_received
```

- [ ] **Step 6: Verify and commit**

Run: `cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings && cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml && cargo test -p test-harness --test strategy --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: clippy clean, 38 lib tests pass, strategy suite passes.

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add common/src/types/pool.rs contracts/controller/src/positions/borrow.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): extract merge_borrow_result shared by plan and strategy borrows"
```

---

### Task 2: Naming and parameter conventions inside borrow.rs

Mechanical renames: one name per concept (`plan`), cache-last, `validate_asset_borrowable`. No behavior change.

**Files:**
- Modify: `contracts/controller/src/positions/borrow.rs`

- [ ] **Step 1: Rename `validate_borrow_asset_preflight` → `validate_asset_borrowable` with the new parameter order**

Replace the whole function:

```rust
fn validate_borrow_asset_preflight(
    env: &Env,
    cache: &mut Cache,
    asset_config: &AssetConfig,
    asset: &Address,
    account: &Account,
) {
```

with:

```rust
/// Account-level borrowability for one asset: isolation, e-mode, borrow flag.
fn validate_asset_borrowable(
    env: &Env,
    account: &Account,
    asset: &Address,
    asset_config: &AssetConfig,
    cache: &mut Cache,
) {
```

(body unchanged). Update both callers:
- in `create_borrow_strategy`: `validate_borrow_asset_preflight(env, cache, &debt_config, debt_token, account);` → `validate_asset_borrowable(env, account, debt_token, &debt_config, cache);`
- in `prepare_borrow_plan`: `validate_borrow_asset_preflight(env, cache, &asset_config, &asset, account);` → `validate_asset_borrowable(env, account, &asset, &asset_config, cache);`

- [ ] **Step 2: Reorder `validate_siloed_borrow_set` parameters and rename internals**

New signature and body (replaces the old function entirely):

```rust
/// Siloed assets must be an account's only borrow; checks the union of
/// existing debt and the incoming plan.
fn validate_siloed_borrow_set(env: &Env, account: &Account, plan: &Vec<Payment>, cache: &mut Cache) {
    let mut union: Vec<Address> = Vec::new(env);
    for asset in account.borrow_positions.keys() {
        utils::push_unique_address(&mut union, asset);
    }
    for (asset, _) in plan {
        utils::push_unique_address(&mut union, asset);
    }

    if union.len() <= 1 {
        return;
    }

    for asset in union {
        let config = cache.cached_asset_config(&asset);
        assert_with_error!(
            env,
            !config.is_siloed_borrowing,
            CollateralError::NotBorrowableSiloed
        );
    }
}
```

Update both callers:
- in `create_borrow_strategy`: `validate_siloed_borrow_set(env, cache, account, &new_borrows);` → `validate_siloed_borrow_set(env, account, &new_borrows, cache);`
- in `prepare_borrow_plan`: `validate_siloed_borrow_set(env, cache, account, assets);` → `validate_siloed_borrow_set(env, account, plan, cache);` (after Step 3's rename)

- [ ] **Step 3: Cache-last + `plan` naming on the plan-stage functions**

`prepare_borrow_plan`: signature `(env: &Env, account: &Account, assets: &Vec<Payment>, cache: &mut Cache, effective_configs: &Map<Address, AssetConfigRaw>)` → `(env: &Env, account: &Account, plan: &Vec<Payment>, effective_configs: &Map<Address, AssetConfigRaw>, cache: &mut Cache)`; rename every `assets` in the body to `plan`.

`execute_borrow_plan`: signature `(env: &Env, caller: &Address, account: &mut Account, assets: &Vec<Payment>, cache: &mut Cache, effective_configs: &Map<Address, AssetConfigRaw>)` → `(env: &Env, caller: &Address, account: &mut Account, plan: &Vec<Payment>, effective_configs: &Map<Address, AssetConfigRaw>, cache: &mut Cache)`; rename `assets` → `plan` in the body.

`process_borrow_plan`: rename its `borrow_plan` parameter to `plan` and update its two calls to match the new argument order:

```rust
    prepare_borrow_plan(env, account, plan, &effective_configs, cache);
    execute_borrow_plan(env, caller, account, plan, &effective_configs, cache);
```

In `process_borrow`, rename the local `borrow_plan` to `plan` (and its two uses: the `process_borrow_plan` call and `utils::plan_assets(env, &plan)`).

- [ ] **Step 4: Verify and commit**

Run: `cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings && cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: clean / 38 passed.

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/positions/borrow.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): borrow.rs naming and cache-last parameter conventions"
```

---

### Task 3: Consolidate `prepare_borrow_plan` into one per-asset pass

Behavior delta #1 (approved): per-asset checks interleave; a multi-fault plan may surface a different (equally valid) error code. No repo test asserts the old ordering (verified by grep — zero hits for `NotBorrowableSiloed`/`AssetNotBorrowable`/`NotBorrowableIsolation` in test assertions).

**Files:**
- Modify: `contracts/controller/src/positions/borrow.rs`

- [ ] **Step 1: Replace the `prepare_borrow_plan` body**

Replace:

```rust
    validation::require_non_empty_payments(env, plan);

    validation::validate_bulk_position_limits(env, account, AccountPositionType::Borrow, plan);
    for (asset, _) in plan {
        validation::require_market_active(env, cache, &asset);
    }
    validate_siloed_borrow_set(env, account, plan, cache);

    for (asset, amount) in plan {
        let asset_config = super::effective_config(env, effective_configs, &asset);
        validate_asset_borrowable(env, account, &asset, &asset_config, cache);

        add_isolated_debt(env, cache, account, &asset, amount);
    }
```

with:

```rust
    validation::require_non_empty_payments(env, plan);
    validation::validate_bulk_position_limits(env, account, AccountPositionType::Borrow, plan);
    validate_siloed_borrow_set(env, account, plan, cache);

    for (asset, amount) in plan {
        validation::require_market_active(env, cache, &asset);
        let asset_config = super::effective_config(env, effective_configs, &asset);
        validate_asset_borrowable(env, account, &asset, &asset_config, cache);
        add_isolated_debt(env, cache, account, &asset, amount);
    }
```

Also update the comment above the function to:

```rust
// Pre-pool gates only: emptiness, position limits, siloed set, then per-asset
// market-active, borrowability, and isolated-debt ceilings. LTV valuation runs
// post-pool in `require_within_ltv` to reuse the borrow's cached market index.
```

- [ ] **Step 2: Verify (full integration suites) and commit**

Run: `cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml && cargo test -p test-harness --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml 2>&1 | grep -E "^test result|FAILED"`
Expected: all `ok` except the known `budget_withdraw_5_collateral_double_pass`. If any OTHER test fails on an error-code assertion, update that assertion to the new first-failing code and note it in the commit body.

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/positions/borrow.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): single per-asset gate pass in prepare_borrow_plan"
```

---

### Task 4: `borrow_for_strategy` through the shared pipeline (TDD)

Strategy borrows gain the position-limit and non-empty gates (bug fix). Rename `create_borrow_strategy` → `borrow_for_strategy`, cache-last.

**Files:**
- Test: `verification/test-harness/tests/strategy/edge/multiply.rs`
- Modify: `contracts/controller/src/positions/borrow.rs`
- Modify: `contracts/controller/src/strategies/helpers.rs:72`

- [ ] **Step 1: Write the failing test**

Append to `verification/test-harness/tests/strategy/edge/multiply.rs`:

```rust
#[test]
fn test_multiply_respects_borrow_position_limit() {
    use test_harness::{assert_contract_error, errors, xlm_preset, BOB};

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(xlm_preset())
        .build();

    // Liquidity for the second strategy's debt leg.
    t.supply(BOB, "XLM", 10_000.0);

    // First multiply: 1 ETH debt -> 3000 USDC collateral (one borrow position).
    t.fund_router("USDC", 3000.0);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // Cap borrow positions at the count the account already holds.
    t.set_position_limits(8, 1);

    // A second multiply into the same account with a different debt asset
    // would open a second borrow position and must hit the limit gate.
    t.fund_router("USDC", 10.0);
    let steps2 =
        build_aggregator_swap(&t, "XLM", "USDC", apply_flash_fee(1_000_000_000), 10_0000000);
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");
    let result = match t.ctrl_client().try_multiply(
        &alice,
        &account_id,
        &0u32,
        &usdc,
        &1_000_000_000i128,
        &xlm,
        &common::types::PositionMode::Multiply,
        &steps2,
        &None,
        &None,
    ) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(err),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(
        result,
        errors::CollateralError::PositionLimitExceeded as u32,
    );
}
```

Note: if `Ok(Err(err)) => Err(err)` fails to compile with a type mismatch, use `Err(err.into())` (the conversion variant used by `try_borrow` in `verification/test-harness/src/ops/borrow.rs:50`).

- [ ] **Step 2: Run the test — expect RED**

Run: `cargo test -p test-harness --test strategy test_multiply_respects_borrow_position_limit --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: FAIL with panic `expected contract error 109 but got Ok(...)` — today the strategy path skips the limit gate.

- [ ] **Step 3: Replace `create_borrow_strategy` with `borrow_for_strategy`**

Replace the entire function (including its doc comment) with:

```rust
/// Creates strategy debt in the pool through the shared borrow gates and
/// returns the asset amount received by the controller.
pub fn borrow_for_strategy(
    env: &Env,
    account: &mut Account,
    debt_token: &Address,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    let mut plan: Vec<Payment> = Vec::new(env);
    plan.push_back((debt_token.clone(), amount));
    let effective_configs = super::effective_configs_for_plan(env, account, &plan, cache);
    prepare_borrow_plan(env, account, &plan, &effective_configs, cache);

    let debt_config = super::effective_config(env, &effective_configs, debt_token);
    let flash_fee = debt_config.flashloan_fee.apply_to(env, amount);
    let borrow_position = account.get_or_create_debt_position(debt_token);

    let pool_addr = cache.cached_pool_address();
    let action = PoolAction {
        position: (&borrow_position).into(),
        amount,
        asset: debt_token.clone(),
    };
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        action,
        flash_fee,
        debt_config.borrow_cap,
    );
    let mutation: PoolPositionMutation = (&result).into();
    merge_borrow_result(
        account,
        debt_token,
        common::events::PositionAction::Multiply,
        &mutation,
        cache,
    );

    result.amount_received
}
```

This deletes the old hand-rolled `require_market_active` call, the direct `emode::active_e_mode_category`/`effective_asset_config` resolution, the `new_borrows` vec, and the direct `validate_siloed_borrow_set`/`validate_asset_borrowable`/`add_isolated_debt` calls — `prepare_borrow_plan` now does all of it. If `emode` is no longer referenced anywhere in borrow.rs after this, remove `use crate::emode;` (it is still used by `validate_asset_borrowable`, so it stays).

- [ ] **Step 4: Update the caller in `strategies/helpers.rs`**

At `contracts/controller/src/strategies/helpers.rs:72`, replace:

```rust
    borrow::create_borrow_strategy(env, cache, account, asset, amount)
```

with:

```rust
    borrow::borrow_for_strategy(env, account, asset, amount, cache)
```

- [ ] **Step 5: Run the new test — expect GREEN — then the full strategy suite**

Run: `cargo test -p test-harness --test strategy --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: all pass, including `test_multiply_respects_borrow_position_limit`.

- [ ] **Step 6: Full check and commit**

Run: `cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings && cargo test -p test-harness --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml 2>&1 | grep -E "^test result|FAILED"`
Expected: clippy clean; suites ok except the known budget test.

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/positions/borrow.rs contracts/controller/src/strategies/helpers.rs verification/test-harness/tests/strategy/edge/multiply.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "fix(controller): strategy borrows go through shared gates incl. position limit"
```

---

### Task 5: `require_within_ltv` owns its RedStone prefetch

**Files:**
- Modify: `contracts/controller/src/validation.rs:73-94`
- Modify: `contracts/controller/src/positions/borrow.rs` (delete the call-site prefetch block)

- [ ] **Step 1: Add the feed prefetch inside the gate**

In `validation.rs`, replace:

```rust
    // Union prefetch so the supply and debt valuations below share a single
    // bulk index call instead of one per side.
    let mut index_assets = account.supply_positions.keys();
    index_assets.append(&account.borrow_positions.keys());
    cache.prefetch_market_indexes(&index_assets);
```

with:

```rust
    // The gate owns its data: prefetch RedStone feeds and market indexes for
    // the union so the supply and debt valuations below share one bulk call
    // per side.
    let mut index_assets = account.supply_positions.keys();
    index_assets.append(&account.borrow_positions.keys());
    crate::oracle::prefetch_redstone_feeds(cache, &index_assets);
    cache.prefetch_market_indexes(&index_assets);
```

- [ ] **Step 2: Delete the call-site block in `process_borrow`**

Remove these lines from borrow.rs:

```rust
    // require_within_ltv and require_healthy_account price the full
    // supply+borrow set before the HF-body prefetch in
    // calculate_account_totals_body can fire; prefetch the union here so those
    // reads hit the cache. Plan assets are already in borrow_positions after
    // the pool call.
    let mut priced_assets = account.supply_positions.keys();
    priced_assets.append(&account.borrow_positions.keys());
    crate::oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);
```

- [ ] **Step 3: Verify (oracle suites exercise prefetch counting) and commit**

Run: `cargo test -p test-harness --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml 2>&1 | grep -E "^test result|FAILED"`
Expected: ok except the known budget test. The oracle suite (`tests/oracle/redstone_bulk.rs`) counts adapter calls — if a count assertion fails, the gate now bulk-prefetches where lazy per-feed reads happened before; update the expected count and note it in the commit body (fewer oracle calls is the improvement, not a regression).

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/validation.rs contracts/controller/src/positions/borrow.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): require_within_ltv prefetches its own redstone feeds"
```

---

### Task 6: Inline `process_borrow_plan`, reorder the file, fix docs

**Files:**
- Modify: `contracts/controller/src/positions/borrow.rs` (whole-file rewrite to the canonical layout)

- [ ] **Step 1: Rewrite borrow.rs to the final canonical content**

The file must end up exactly as follows (this is the spec's target shape; all pieces already exist from Tasks 1-5 — this step inlines `process_borrow_plan` into `process_borrow`, reorders functions, and updates the module + function docs):

```rust
//! Borrow and strategy-internal borrow flows.
//!
//! Borrows use `OraclePolicy::RiskIncreasing`, update scaled debt shares, and
//! increment isolated-debt counters when the account is isolated. LTV and
//! health gates run post-pool against the market indexes the pool borrow
//! writes into the cache.

use common::errors::{CollateralError, EModeError};
use common::types::{
    Account, AccountPositionType, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    PoolBorrowEntry, PoolPositionMutation,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::cross_contract::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::emode;
use crate::helpers::{require_no_borrow_dust_for_assets, update_or_remove_debt_position};
use crate::oracle::policy::OraclePolicy;
use crate::positions::isolated_debt::add_isolated_debt;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>) {
        process_borrow(&env, &caller, account_id, &borrows);
    }
}

/// Borrows one or more assets; LTV and health validation run post-pool so the
/// valuation reuses the market indexes the borrow itself wrote into the cache.
pub fn process_borrow(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);
    // Dedup once at the entrypoint so every downstream stage, including the
    // post-flight dust scope, sees one entry per asset.
    let plan = utils::aggregate_positive_payments(env, borrows);

    let effective_configs = super::effective_configs_for_plan(env, &account, &plan, &mut cache);
    prepare_borrow_plan(env, &account, &plan, &effective_configs, &mut cache);
    execute_borrow_plan(env, caller, &mut account, &plan, &effective_configs, &mut cache);

    // A failure in either gate panics and reverts the atomic tx.
    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Scope the dust gate to borrowed assets only: borrow never mutates supply,
    // so it must not be blocked by pre-existing positions that drifted under the floor.
    require_no_borrow_dust_for_assets(env, &mut cache, &account, &utils::plan_assets(env, &plan));

    storage::set_debt_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

// Pre-pool gates only: emptiness, position limits, siloed set, then per-asset
// market-active, borrowability, and isolated-debt ceilings. LTV valuation runs
// post-pool in `require_within_ltv` to reuse the borrow's cached market index.
fn prepare_borrow_plan(
    env: &Env,
    account: &Account,
    plan: &Vec<Payment>,
    effective_configs: &Map<Address, AssetConfigRaw>,
    cache: &mut Cache,
) {
    validation::require_non_empty_payments(env, plan);
    validation::validate_bulk_position_limits(env, account, AccountPositionType::Borrow, plan);
    validate_siloed_borrow_set(env, account, plan, cache);

    for (asset, amount) in plan {
        validation::require_market_active(env, cache, &asset);
        let asset_config = super::effective_config(env, effective_configs, &asset);
        validate_asset_borrowable(env, account, &asset, &asset_config, cache);
        add_isolated_debt(env, cache, account, &asset, amount);
    }
}

fn execute_borrow_plan(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    plan: &Vec<Payment>,
    effective_configs: &Map<Address, AssetConfigRaw>,
    cache: &mut Cache,
) {
    // Build the whole plan's entries, make ONE pool call, then merge results
    // input-ordered — one cross-contract frame instead of one per asset.
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (asset, amount) in plan {
        let asset_config = super::effective_config(env, effective_configs, &asset);
        let borrow_position = account.get_or_create_debt_position(&asset);
        entries.push_back(PoolBorrowEntry {
            action: PoolAction {
                position: (&borrow_position).into(),
                amount,
                asset: asset.clone(),
            },
            borrow_cap: asset_config.borrow_cap,
        });
    }
    let pool_addr = cache.cached_pool_address();
    let results = pool_borrow_call(env, &pool_addr, caller, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        merge_borrow_result(
            account,
            &entry.action.asset,
            common::events::PositionAction::Borrow,
            &result,
            cache,
        );
    }
}

/// Merges one pool borrow result into the account and event buffers.
fn merge_borrow_result(
    account: &mut Account,
    asset: &Address,
    action: common::events::PositionAction,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);
    let position: DebtPosition = (&result.position).into();
    cache.record_debt_position_update(
        action,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, asset, &position);
}

/// Account-level borrowability for one asset: isolation, e-mode, borrow flag.
fn validate_asset_borrowable(
    env: &Env,
    account: &Account,
    asset: &Address,
    asset_config: &AssetConfig,
    cache: &mut Cache,
) {
    if account.is_isolated && !asset_config.can_borrow_in_isolation() {
        panic_with_error!(env, EModeError::NotBorrowableIsolation);
    }

    emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, asset);
    emode::ensure_e_mode_compatible_with_asset(env, asset_config, account.e_mode_category_id);

    assert_with_error!(
        env,
        asset_config.is_borrowable,
        CollateralError::AssetNotBorrowable
    );
}

/// Siloed assets must be an account's only borrow; checks the union of
/// existing debt and the incoming plan.
fn validate_siloed_borrow_set(env: &Env, account: &Account, plan: &Vec<Payment>, cache: &mut Cache) {
    let mut union: Vec<Address> = Vec::new(env);
    for asset in account.borrow_positions.keys() {
        utils::push_unique_address(&mut union, asset);
    }
    for (asset, _) in plan {
        utils::push_unique_address(&mut union, asset);
    }

    if union.len() <= 1 {
        return;
    }

    for asset in union {
        let config = cache.cached_asset_config(&asset);
        assert_with_error!(
            env,
            !config.is_siloed_borrowing,
            CollateralError::NotBorrowableSiloed
        );
    }
}

/// Creates strategy debt in the pool through the shared borrow gates and
/// returns the asset amount received by the controller.
pub fn borrow_for_strategy(
    env: &Env,
    account: &mut Account,
    debt_token: &Address,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    let mut plan: Vec<Payment> = Vec::new(env);
    plan.push_back((debt_token.clone(), amount));
    let effective_configs = super::effective_configs_for_plan(env, account, &plan, cache);
    prepare_borrow_plan(env, account, &plan, &effective_configs, cache);

    let debt_config = super::effective_config(env, &effective_configs, debt_token);
    let flash_fee = debt_config.flashloan_fee.apply_to(env, amount);
    let borrow_position = account.get_or_create_debt_position(debt_token);

    let pool_addr = cache.cached_pool_address();
    let action = PoolAction {
        position: (&borrow_position).into(),
        amount,
        asset: debt_token.clone(),
    };
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        action,
        flash_fee,
        debt_config.borrow_cap,
    );
    let mutation: PoolPositionMutation = (&result).into();
    merge_borrow_result(
        account,
        debt_token,
        common::events::PositionAction::Multiply,
        &mutation,
        cache,
    );

    result.amount_received
}
```

- [ ] **Step 2: Verify and commit**

Run: `cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings && cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml && cargo test -p test-harness --test strategy --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: clean / 38 passed / strategy suite ok.

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/positions/borrow.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): borrow.rs top-down layout, inline plan wrapper, doc fixes"
```

---

### Task 7: Ripple conventions to siblings

**Files:**
- Modify: `contracts/controller/src/positions/supply.rs`
- Modify: `contracts/controller/src/validation.rs` (`validate_bulk_position_limits` param rename)
- Modify: `contracts/controller/src/positions/withdraw.rs` (stale comment only)

- [ ] **Step 1: supply.rs — `plan` naming and cache-last**

Mechanical renames (signatures + bodies + call sites; no logic change):
- `process_supply` local `deposit_plan` → `plan`.
- `resolve_supply_account(env, caller, account_id, e_mode_category, assets: &Vec<Payment>, cache)` → param `assets` renamed `plan`.
- `create_account_for_first_asset(env, caller, e_mode_category, assets, cache)` → param `assets` renamed `plan`.
- `process_deposit(env, caller, account, deposit_plan, cache)` → param `deposit_plan` renamed `plan` (pub fn — param rename only, no ABI impact).
- `prepare_deposit_plan(env, account, assets, cache, effective_configs)` → `prepare_deposit_plan(env, account, plan, effective_configs, cache)`; body `assets` → `plan`.
- `execute_deposit_plan(env, caller, account, assets, cache, effective_configs)` → `execute_deposit_plan(env, caller, account, plan, effective_configs, cache)`; body `assets` → `plan`.
- `validate_bulk_isolation(env, account, assets, cache)` → param `assets` renamed `plan`; body updated.
- Update the two call sites inside `process_deposit` to the new argument order.

- [ ] **Step 2: validation.rs — `validate_bulk_position_limits` param rename**

Rename its `assets: &Vec<Payment>` parameter to `plan` and the two body uses (`for (asset, _) in plan.iter()`).

- [ ] **Step 3: withdraw.rs — fix the stale oracle-scope comment**

Replace the comment block in `process_withdraw` that begins `// When the account has debt, the withdrawal loop prices the withdrawn` and ends `// reads are cache hits.)` with:

```rust
    // When the account has debt, the post-pool gates (LTV, health) price the
    // full supply+borrow set; prefetch the union here so those reads and any
    // mid-merge risk-param refresh hit the cache. When there is no debt, the
    // gates early-return and only the dust gate prices the withdrawn assets —
    // scope the prefetch to plan assets so no unread feeds are fetched.
```

(Keep the `dust_assets` / `priced_assets` code below it unchanged.)

- [ ] **Step 4: Verify and commit**

Run: `cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings && cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml`
Expected: clean / 38 passed.

```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm add contracts/controller/src/positions/supply.rs contracts/controller/src/positions/withdraw.rs contracts/controller/src/validation.rs
git -C /Users/mihaieremia/GitHub/rs-lending-xlm commit -m "refactor(controller): ripple plan naming and cache-last conventions to siblings"
```

---

### Task 8: Full verification sweep

**Files:** none (verification only)

- [ ] **Step 1: Full workspace gates**

```bash
cargo check --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml
cargo clippy --workspace --all-targets --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml -- -D warnings
cargo test --workspace --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml 2>&1 | grep -E "^test result|FAILED"
cargo test -p controller --lib --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/Cargo.toml
```
Expected: all ok; the only allowed failure is `budget_withdraw_5_collateral_double_pass` (pre-existing). If `-p defindex-strategy --test strategy` fails with `MismatchingParameterLen`, rebuild the pool fixture first: `stellar contract build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/contracts/pool/Cargo.toml`.

- [ ] **Step 2: WASM size delta vs the Task 0 baseline**

```bash
stellar contract build --manifest-path /Users/mihaieremia/GitHub/rs-lending-xlm/contracts/controller/Cargo.toml > /dev/null 2>&1
NEW=$(stat -f%z /Users/mihaieremia/GitHub/rs-lending-xlm/target/wasm32v1-none/release/controller.wasm)
echo "baseline=$(cat /tmp/borrow_wasm_baseline.txt) new=$NEW delta=$((NEW - $(cat /tmp/borrow_wasm_baseline.txt)))"
```
Expected: delta ≤ 0 (spec requirement). If positive, report the number — do not chase bytes without discussing.

- [ ] **Step 3: Report**

Summarize per the repo's output discipline: `Changed:` (commits list), `Verified:` (commands + results incl. WASM delta), `Notes:` (behavior deltas shipped: panic selection on multi-fault plans, strategy position-limit gate).
