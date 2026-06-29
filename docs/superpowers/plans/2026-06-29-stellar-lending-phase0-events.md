# Phase 0 — Contract events carry `hub_id` (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. This is the **blocker phase** of the master plan (`xoxno-az-functions/docs/superpowers/plans/2026-06-29-stellar-lending-independent-architecture.md`, §Phase 0 / Appendix §16). Repo: **rs-lending-xlm** only.

**Goal:** Add `hub_id: u32` to the five `(hub,asset)`-scoped contract events that currently drop it, so the off-chain indexer can disambiguate the same asset across hubs (USDC@hub0 vs USDC@hub1).

**Architecture:** There is one shared pool contract and one controller, so `hub_id` is unrecoverable from the emitting address — it must travel in the event body. Every emit site already holds the hub coordinate (`MarketStateSnapshot.hub_asset`, the pool's `hub_asset: HubAssetKey`, `SpokeAssetArgs.hub_id`, the leg's `hub_asset` in `positions/*`), so each change only stops discarding it. All five additions are additive wire-ABI changes; testnet is fresh, no migration. Oracle events stay asset-only (pricing is token-rooted).

**Tech Stack:** Rust, Soroban SDK (`#[contractevent]`, `#[contracttype]`), `HubAssetKey { hub_id: u32, asset: Address }`.

## Global Constraints

- **Wire-ABI field order is load-bearing** — the off-chain decoder reads positional tuples/struct keys. Add `hub_id` exactly where each task specifies; never reorder existing fields.
- No `unwrap()`/`expect()` outside tests. No magic literals. Conventional Commits, subject ≤72 chars.
- **Verification per task:** `cargo test -p <crate> <test>` for the new test; then `cargo check --all-targets` for the touched crate. **End of phase:** `cargo check --all-targets && cargo clippy --all-targets -- -D warnings && cargo test -p pool -p controller` (no `--all-features` — that enables certora and breaks linking).
- Each task is one commit.

---

## Pre-flight

- [ ] **Baseline green.** Run `cargo check -p pool -p controller --all-targets` and confirm it compiles before starting. Expected: clean (warnings ok).

---

## Task 0.1 — `PoolMarketStateEvent` carries `hub_id`

**Files:**
- Modify: `contracts/pool/src/events.rs:9-35` (struct + `From`)
- Test: `contracts/pool/tests/flows.rs` (append a `#[test]`; this file is the pool test module, included via `#[path]` from `lib.rs:640`)

**Interfaces — Produces:** wire ABI `PoolMarketStateEvent[hub_id, asset, timestamp, supply_index, borrow_index, cash, supplied, borrowed, revenue]` (topic `["market","batch_state_update"]`). No call-site changes — every state emit already passes a `MarketStateSnapshot` (which carries `hub_asset`) via `cache.market_snapshot()`.

- [ ] **Step 1: Write the failing test** — append to `contracts/pool/tests/flows.rs`:

```rust
#[test]
fn pool_market_state_event_carries_hub_id() {
    use common::types::{HubAssetKey, MarketStateSnapshot};
    use crate::events::PoolMarketStateEvent;
    use soroban_sdk::testutils::Address as _;
    let env = Env::default();
    let asset = Address::generate(&env);
    let snap = MarketStateSnapshot {
        hub_asset: HubAssetKey { hub_id: 7, asset: asset.clone() },
        timestamp: 1,
        supply_index: 0,
        borrow_index: 0,
        cash: 0,
        supplied: 0,
        borrowed: 0,
        revenue: 0,
    };
    let ev = PoolMarketStateEvent::from(&snap);
    assert_eq!(ev.0, 7);
    assert_eq!(ev.1, asset);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pool pool_market_state_event_carries_hub_id`
Expected: FAIL — compile error (`PoolMarketStateEvent` has 8 fields; `.0` is `Address`, no `hub_id`).

- [ ] **Step 3: Add the field** — `contracts/pool/src/events.rs`, replace the struct + `From`:

```rust
/// Pool market accounting snapshot emitted after successful pool mutations.
///
/// Field order is wire ABI; do not reorder:
/// `[hub_id, asset, timestamp, supply_index, borrow_index, cash,
///   supplied, borrowed, revenue]`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketStateEvent(
    pub u32,
    pub Address,
    pub u64,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
);

impl From<&MarketStateSnapshot> for PoolMarketStateEvent {
    fn from(s: &MarketStateSnapshot) -> Self {
        Self(
            s.hub_asset.hub_id,
            s.hub_asset.asset.clone(),
            s.timestamp,
            s.supply_index,
            s.borrow_index,
            s.cash,
            s.supplied,
            s.borrowed,
            s.revenue,
        )
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pool pool_market_state_event_carries_hub_id`
Expected: PASS. Then `cargo check -p pool --all-targets` clean.

- [ ] **Step 5: Commit**

```bash
git add contracts/pool/src/events.rs contracts/pool/tests/flows.rs
git commit -m "feat(events): carry hub_id in PoolMarketStateEvent"
```

---

## Task 0.2 — `PoolMarketParamsEvent` carries `hub_id`

**Files:**
- Modify: `contracts/pool/src/events.rs:43-48` (struct) and `:93-97` (`publish_market_params`)
- Modify call-sites: `contracts/pool/src/lib.rs:307`, `:567`, `:577`
- Test: `contracts/pool/tests/flows.rs`

**Interfaces — Produces:** `PoolMarketParamsEvent { hub_id, asset, params }`; `publish_market_params(env, hub_id: u32, asset, params)`.

- [ ] **Step 1: Write the failing test** — append to `contracts/pool/tests/flows.rs` (reuses the existing `market_params` helper at `flows.rs:120`):

```rust
#[test]
fn pool_market_params_event_carries_hub_id() {
    use crate::events::PoolMarketParamsEvent;
    use soroban_sdk::testutils::Address as _;
    let env = Env::default();
    let asset = Address::generate(&env);
    let ev = PoolMarketParamsEvent { hub_id: 5, asset: asset.clone(), params: market_params(&asset) };
    assert_eq!(ev.hub_id, 5);
    assert_eq!(ev.asset, asset);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pool pool_market_params_event_carries_hub_id`
Expected: FAIL — compile error (`PoolMarketParamsEvent` has no field `hub_id`).

- [ ] **Step 3a: Add the field + thread the helper** — `contracts/pool/src/events.rs`:

```rust
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub params: MarketParamsRaw,
}
```
and replace `publish_market_params`:
```rust
/// Emits a single market-params update as a one-element batch.
pub(crate) fn publish_market_params(env: &Env, hub_id: u32, asset: Address, params: MarketParamsRaw) {
    let mut updates = Vec::new(env);
    updates.push_back(PoolMarketParamsEvent { hub_id, asset, params });
    publish_market_params_batch(env, updates);
}
```

- [ ] **Step 3b: Thread the three call-sites** — `contracts/pool/src/lib.rs` (each has `hub_asset: HubAssetKey` in scope):
  - `:307` (create_market): `events::publish_market_params(&env, hub_asset.hub_id, asset, params);`
  - `:567` (update_params): `events::publish_market_params(&env, hub_asset.hub_id, asset, params);`
  - `:577` (update_caps): `events::publish_market_params(&env, hub_asset.hub_id, asset, params);`

- [ ] **Step 4: Run test + check**

Run: `cargo test -p pool pool_market_params_event_carries_hub_id`
Expected: PASS. Then `cargo check -p pool --all-targets` clean (the three call-sites compile).

- [ ] **Step 5: Commit**

```bash
git add contracts/pool/src/events.rs contracts/pool/src/lib.rs contracts/pool/tests/flows.rs
git commit -m "feat(events): carry hub_id in PoolMarketParamsEvent"
```

---

## Task 0.3 — `CreateMarketEvent` carries `hub_id`

**Files:**
- Modify: `contracts/controller/src/events.rs:244-258` (struct)
- Modify emit: `contracts/controller/src/setup/mod.rs:39-52` (fn already has `hub_id: u32` param)
- Modify existing test literal: `contracts/controller/tests/events.rs:213-225`
- Test: `contracts/controller/tests/events.rs` (new `#[test]`)

**Interfaces — Produces:** `CreateMarketEvent { hub_id, base_asset, … , market_address }` (topic `["market","create"]`).

- [ ] **Step 1: Write the failing test** — append to `contracts/controller/tests/events.rs`:

```rust
#[test]
fn create_market_event_carries_hub_id() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let ev = CreateMarketEvent {
        hub_id: 2,
        base_asset: asset.clone(),
        max_borrow_rate: 0,
        base_borrow_rate: 0,
        slope1: 0,
        slope2: 0,
        slope3: 0,
        mid_utilization: 0,
        optimal_utilization: 0,
        max_utilization: 0,
        reserve_factor: 0,
        market_address: asset.clone(),
    };
    assert_eq!(ev.hub_id, 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p controller create_market_event_carries_hub_id`
Expected: FAIL — compile error (no field `hub_id`).

- [ ] **Step 3a: Add the field** — `contracts/controller/src/events.rs`, add `pub hub_id: u32,` as the first field of `CreateMarketEvent`:

```rust
#[contractevent(topics = ["market", "create"])]
#[derive(Clone, Debug)]
pub struct CreateMarketEvent {
    pub hub_id: u32,
    pub base_asset: Address,
    pub max_borrow_rate: i128,
    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,
    pub mid_utilization: i128,
    pub optimal_utilization: i128,
    pub max_utilization: i128,
    pub reserve_factor: u32,
    pub market_address: Address,
}
```

- [ ] **Step 3b: Emit it** — `contracts/controller/src/setup/mod.rs:39`, add `hub_id,` as the first field of the `CreateMarketEvent { … }` literal (the enclosing fn `create_liquidity_pool` already binds `hub_id: u32`).

- [ ] **Step 3c: Fix the existing test literal** — `contracts/controller/tests/events.rs:213`, in `emit_helpers_publish_without_panicking`, add `hub_id: 0,` as the first field of the `CreateMarketEvent { base_asset: asset.clone(), … }` literal.

- [ ] **Step 4: Run tests + check**

Run: `cargo test -p controller create_market_event_carries_hub_id emit_helpers_publish_without_panicking`
Expected: PASS (both). Then `cargo check -p controller --all-targets` clean.

- [ ] **Step 5: Commit**

```bash
git add contracts/controller/src/events.rs contracts/controller/src/setup/mod.rs contracts/controller/tests/events.rs
git commit -m "feat(events): carry hub_id in CreateMarketEvent"
```

---

## Task 0.4 — `EventDepositDelta` + `EventBorrowDelta` carry `hub_id` (per leg)

**Files:**
- Modify: `contracts/controller/src/events.rs:300-364` (both tuple structs + `::new`)
- Modify: `contracts/controller/src/context/events.rs:11-44` (`record_position_update`, `record_debt_position_update`)
- Modify call-sites (each has `hub_asset: HubAssetKey` in scope): `contracts/controller/src/positions/supply.rs:184`, `withdraw.rs:238`, `repay.rs:143`, `borrow.rs:170`, `pool_ops/mod.rs:343`
- Modify existing test literal: `contracts/controller/tests/events.rs:243`
- Test: `contracts/controller/tests/events.rs` (new `#[test]`)

**Interfaces — Produces:**
- `EventDepositDelta[action, hub_id, asset, scaled_amount, index_ray, amount, liq_threshold, liq_bonus, ltv]`
- `EventBorrowDelta[action, hub_id, asset, scaled_amount, index_ray, amount]`
- `EventDepositDelta::new(action, hub_id, asset, index_ray, amount, position)`
- `EventBorrowDelta::new(action, hub_id, asset, index_ray, amount, position)`
- `Cache::record_position_update(action, hub_id, asset, index_ray, amount, position)`
- `Cache::record_debt_position_update(action, hub_id, asset, index_ray, amount, position)`

- [ ] **Step 1: Write the failing test** — append to `contracts/controller/tests/events.rs`:

```rust
#[test]
fn position_deltas_carry_hub_id() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let dep = EventDepositDelta(PositionAction::Supply, 4, asset.clone(), 0, 0, 0, 0, 0, 0);
    let bor = EventBorrowDelta(PositionAction::Repay, 9, asset.clone(), 0, 0, 0);
    assert_eq!(dep.1, 4);
    assert_eq!(bor.1, 9);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p controller position_deltas_carry_hub_id`
Expected: FAIL — compile error (arity mismatch; `dep.1` is `Address`).

- [ ] **Step 3a: Add the fields + constructors** — `contracts/controller/src/events.rs`:

```rust
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventDepositDelta(
    pub PositionAction,
    pub u32,
    pub Address,
    pub i128,
    pub i128,
    pub i128,
    pub u32,
    pub u32,
    pub u32,
);

impl EventDepositDelta {
    pub fn new(
        action: PositionAction,
        hub_id: u32,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) -> Self {
        Self(
            action,
            hub_id,
            asset,
            position.scaled_amount.raw(),
            index_ray,
            amount,
            position.liquidation_threshold.raw() as u32,
            position.liquidation_bonus.raw() as u32,
            position.loan_to_value.raw() as u32,
        )
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EventBorrowDelta(
    pub PositionAction,
    pub u32,
    pub Address,
    pub i128,
    pub i128,
    pub i128,
);

impl EventBorrowDelta {
    pub fn new(
        action: PositionAction,
        hub_id: u32,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) -> Self {
        Self(action, hub_id, asset, position.scaled_amount.raw(), index_ray, amount)
    }
}
```
(Keep the existing doc-comments on the two structs.)

- [ ] **Step 3b: Thread the recorders** — `contracts/controller/src/context/events.rs`:

```rust
    pub fn record_position_update(
        &mut self,
        action: PositionAction,
        hub_id: u32,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) {
        self.deposit_updates.push_back(EventDepositDelta::new(
            action,
            hub_id,
            asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    pub fn record_debt_position_update(
        &mut self,
        action: PositionAction,
        hub_id: u32,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) {
        self.borrow_updates.push_back(EventBorrowDelta::new(
            action,
            hub_id,
            asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }
```

- [ ] **Step 3c: Thread the five call-sites** — insert `hub_asset.hub_id,` as the second argument (after the `action`) at each:
  - `positions/supply.rs:184` → `cache.record_position_update(events::PositionAction::Supply, hub_asset.hub_id, &hub_asset.asset, result.market_index.supply_index, entry.action.amount, &position);`
  - `positions/withdraw.rs:238` → `cache.record_position_update(action, hub_asset.hub_id, &hub_asset.asset, result.market_index.supply_index, result.actual_amount, &result_position);`
  - `positions/repay.rs:143` → `cache.record_debt_position_update(action, hub_asset.hub_id, &hub_asset.asset, result.market_index.borrow_index, result.actual_amount, &position);`
  - `positions/borrow.rs:170` → `cache.record_debt_position_update(action, hub_asset.hub_id, &hub_asset.asset, result.market_index.borrow_index, result.actual_amount, &position);`
  - `pool_ops/mod.rs:343` → `cache.record_position_update(events::PositionAction::ParamUpd, hub_asset.hub_id, &hub_asset.asset, market_index.supply_index.raw(), 0, &updated);`

- [ ] **Step 3d: Fix the existing test literal** — `contracts/controller/tests/events.rs:243`, the `EventDepositDelta(PositionAction::Supply, asset.clone(), 0, 0, 0, 0, 0, 0)` becomes:

```rust
        deposits.push_back(EventDepositDelta(
            PositionAction::Supply,
            0,
            asset.clone(),
            0,
            0,
            0,
            0,
            0,
            0,
        ));
```

- [ ] **Step 4: Run tests + check**

Run: `cargo test -p controller position_deltas_carry_hub_id emit_helpers_publish_without_panicking`
Expected: PASS (both). Then `cargo check -p controller --all-targets` clean (all five call-sites + recorders compile).

- [ ] **Step 5: Commit**

```bash
git add contracts/controller/src/events.rs contracts/controller/src/context/events.rs \
  contracts/controller/src/positions/supply.rs contracts/controller/src/positions/withdraw.rs \
  contracts/controller/src/positions/repay.rs contracts/controller/src/positions/borrow.rs \
  contracts/controller/src/pool_ops/mod.rs contracts/controller/tests/events.rs
git commit -m "feat(events): carry hub_id per leg in position deltas"
```

---

## Task 0.5 — `UpdateSpokeAssetEvent` + `RemoveSpokeAssetEvent` carry `hub_id`

**Files:**
- Modify: `contracts/controller/src/events.rs:419-432` (both structs)
- Modify emits: `contracts/controller/src/config/asset.rs:80` (add), `:159` (edit), `:199` (remove)
- Modify existing test literals: `contracts/controller/tests/events.rs:284` and `:303`
- Test: `contracts/controller/tests/events.rs` (new `#[test]`)

**Interfaces — Produces:** `UpdateSpokeAssetEvent { asset, config, spoke_id, hub_id }` (topic `["config","spoke_asset"]`); `RemoveSpokeAssetEvent { asset, spoke_id, hub_id }` (topic `["config","remove_spoke_asset"]`).

- [ ] **Step 1: Write the failing test** — append to `contracts/controller/tests/events.rs`:

```rust
#[test]
fn spoke_asset_events_carry_hub_id() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let upd = UpdateSpokeAssetEvent {
        asset: asset.clone(),
        config: SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: true,
            paused: false,
            frozen: false,
            loan_to_value: 9000,
            liquidation_threshold: 9500,
            liquidation_bonus: 200,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: MarketOracleConfigOption::None,
        },
        spoke_id: 1,
        hub_id: 3,
    };
    let rem = RemoveSpokeAssetEvent { asset, spoke_id: 1, hub_id: 3 };
    assert_eq!(upd.hub_id, 3);
    assert_eq!(rem.hub_id, 3);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p controller spoke_asset_events_carry_hub_id`
Expected: FAIL — compile error (no field `hub_id`).

- [ ] **Step 3a: Add the fields** — `contracts/controller/src/events.rs`:

```rust
#[contractevent(topics = ["config", "spoke_asset"])]
#[derive(Clone, Debug)]
pub struct UpdateSpokeAssetEvent {
    pub asset: Address,
    pub config: SpokeAssetConfig,
    pub spoke_id: u32,
    pub hub_id: u32,
}

#[contractevent(topics = ["config", "remove_spoke_asset"])]
#[derive(Clone, Debug)]
pub struct RemoveSpokeAssetEvent {
    pub asset: Address,
    pub spoke_id: u32,
    pub hub_id: u32,
}
```

- [ ] **Step 3b: Emit at the three sites** — `contracts/controller/src/config/asset.rs`:
  - `:80` (in `add_asset_to_spoke`, `args: &SpokeAssetArgs`) → add `hub_id: args.hub_id,` to the `UpdateSpokeAssetEvent { … }` literal.
  - `:159` (in `edit_asset_in_spoke`) → add `hub_id: args.hub_id,` to the `UpdateSpokeAssetEvent { … }` literal.
  - `:199` (in `remove_asset_from_spoke`, `hub_asset: HubAssetKey`) → add `hub_id: hub_asset.hub_id,`:
    ```rust
    RemoveSpokeAssetEvent {
        asset: hub_asset.asset,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
    ```
    (`hub_id` is a `Copy` `u32`; reading it in the same literal that moves `hub_asset.asset` is valid.)

- [ ] **Step 3c: Fix the existing test literals** — `contracts/controller/tests/events.rs`:
  - `:284` `UpdateSpokeAssetEvent { …, spoke_id: 1 }` → add `hub_id: 0,` after `spoke_id: 1,`.
  - `:303` `RemoveSpokeAssetEvent { asset: asset.clone(), spoke_id: 1 }` → add `hub_id: 0,`.

- [ ] **Step 4: Run tests + check**

Run: `cargo test -p controller spoke_asset_events_carry_hub_id emit_helpers_publish_without_panicking`
Expected: PASS (both). Then `cargo check -p controller --all-targets` clean.

- [ ] **Step 5: Commit**

```bash
git add contracts/controller/src/events.rs contracts/controller/src/config/asset.rs contracts/controller/tests/events.rs
git commit -m "feat(events): carry hub_id in spoke-asset events"
```

---

## Phase verification + redeploy

- [ ] **Full gate:** `cargo check --all-targets && cargo clippy --all-targets -- -D warnings && cargo test -p pool -p controller` — all green. (Do NOT pass `--all-features`; that enables certora and breaks linking.)
- [ ] **Redeploy testnet** (per `GOAL.md` verification): bump `AppVersion`, redeploy controller + pool, `create_hub`×2, create the same asset on both hubs, list it on a spoke per hub.
- [ ] **Confirm on the wire:** fetch a sample via Soroban RPC `getEvents` for a `market:batch_state_update` and a `config:spoke_asset` and verify the decoded body now contains the hub id (state event field 0; spoke-asset `hub_id` key). This is the green light for Phase 2 (indexer).

## Self-review

- **Spec coverage (Appendix §16):** all five events updated — `PoolMarketStateEvent` (0.1), `PoolMarketParamsEvent` (0.2), `CreateMarketEvent` (0.3), `EventDepositDelta`/`EventBorrowDelta` (0.4), `UpdateSpokeAssetEvent`/`RemoveSpokeAssetEvent` (0.5). Oracle events deliberately untouched (token-rooted).
- **Field-order consistency:** `hub_id` is field 0 on `PoolMarketStateEvent`; field index 1 (after `action`) on both deltas; a named `hub_id` key on the three structs — matching each task's Produces block and the decoder's positional expectations.
- **No placeholders:** every struct, call-site (exact file:line + the `hub_asset` var in scope), and existing test literal to fix is concrete. Threading uses `hub_asset.hub_id` (Copy u32) — no borrow-checker hazard at the `remove_asset_from_spoke` partial move.
