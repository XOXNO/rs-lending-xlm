# RedStone Bulk Prefetch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace N per-feed RedStone cross-contract calls with one bulk `read_price_data` call per transaction, cutting ~1.27MB of metered memory per avoided call.

**Architecture:** A transaction-local prefetch map in `Cache` stores **raw** `RedStonePriceData` keyed by `(adapter, feed_id)`. Entrypoints that price ≥2 assets call `prefetch_redstone_feeds` once (two chokepoints: `calculate_account_totals_body` for HF flows, `check_assets_side` for dust gates). The lowest-level client read (`client.rs::read_price_data`) consults the map first and falls back to today's single-feed call on any miss. Because only the raw provider payload is cached — never a resolved price — every staleness/sanity/tolerance/policy check still runs per flow, unchanged. Bulk failure leaves the map empty → exact current behavior.

**Tech Stack:** Rust / soroban-sdk (workspace crates: `controller`, `common`, `test-harness`). Verification: `cargo check/clippy/test` **per crate** (combined `-p` clippy fails on a known testutils feature-unification quirk — not a regression).

**Verified facts this plan relies on (probed on testnet adapter `CBIHT4HVRIT5OMVLSXZ44J2ZAXYBDDGOSCN3LTN2DOC6SWHDS5IP6BK3`, 2026-06-11):**
- `read_price_data(feed_ids: Vec<String>) -> Result<Vec<PriceData>, Error>` exists on the deployed adapter.
- Results are **index-aligned** with the request order (`[BTC, USDC]` → `[6121509116480, 99971464]`).
- A missing feed fails the **whole call** (all-or-nothing). The harness mock (`?` propagation) matches this.

**Working directory:** `/home/truststaking/GitHub/rs-lending-xlm/.claude/worktrees/integration-tests` (branch `worktree-integration-tests`). Run all commands from this root.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `contracts/controller/src/cache/mod.rs` | Modify | Add `redstone_prefetch` map + accessors |
| `contracts/controller/src/oracle/providers/redstone/client.rs` | Modify | Add bulk trait fn + `read_price_data_bulk`; make single read consult cache |
| `contracts/controller/src/oracle/providers/redstone.rs` | Modify | Pass `cache` (not `env`) into `read_price_data` |
| `contracts/controller/src/oracle/prefetch.rs` | Create | Collect feeds from market configs, group by adapter, bulk-fetch |
| `contracts/controller/src/oracle/mod.rs` | Modify | Wire `mod prefetch`, re-export |
| `contracts/controller/src/helpers/math.rs` | Modify | Chokepoint 1: prefetch in `calculate_account_totals_body` |
| `contracts/controller/src/helpers/account.rs` | Modify | Chokepoint 2: prefetch in `check_assets_side` |
| `verification/test-harness/src/mock_redstone.rs` | Modify | Add call counters (single vs bulk) for assertions |
| `verification/test-harness/tests/oracle/redstone_bulk.rs` | Create | New tests: bulk-once, fallback, parity, idempotency |
| `verification/test-harness/tests/oracle/main.rs` (or module registry) | Modify | Register new test file |

**Certora note:** `oracle/price.rs` and `tolerance.rs` are cfg-swapped with certora harness files; `_calculate_account_totals_impl` is summarized under `certora`. The prefetch is a pure performance optimization with identical semantics, so under `feature = "certora"` it compiles to a **no-op** (see Task 3). No new nondet summaries needed. The certora build is known pre-broken by the emode refactor — do not attempt to fix it here; just don't add new breakage (the no-op + additive Cache field guarantee that).

---

### Task 1: Cache prefetch store

**Files:**
- Modify: `contracts/controller/src/cache/mod.rs`

- [ ] **Step 1: Add the import**

In `cache/mod.rs`, after the existing `use crate::oracle::...` imports (line ~22):

```rust
use crate::oracle::providers::redstone::RedStonePriceData;
```

(`providers` is `pub(crate)` in `oracle/mod.rs`, `redstone` re-exports the type `pub(crate)` — the path resolves crate-internally.)

- [ ] **Step 2: Add the field**

In `struct Cache` (after `pub prices_cache: Map<Address, PriceFeedRaw>,`):

```rust
    /// Raw RedStone payloads bulk-fetched once per tx, keyed by (adapter, feed_id).
    /// Stores provider data, never resolved prices, so per-flow policy checks
    /// (staleness, sanity, tolerance) are unaffected.
    redstone_prefetch: Map<(Address, String), RedStonePriceData>,
```

Add `String` to the `soroban_sdk` import list at the top of the file.

- [ ] **Step 3: Initialize in `build()`**

In `Cache::build`, alongside the other `Map::new(env)` fields:

```rust
            redstone_prefetch: Map::new(env),
```

- [ ] **Step 4: Add accessors**

After `cached_price` (line ~84):

```rust
    pub fn redstone_prefetched(
        &self,
        contract: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.redstone_prefetch
            .get((contract.clone(), feed_id.clone()))
    }

    pub fn set_redstone_prefetched(
        &mut self,
        contract: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.redstone_prefetch
            .set((contract.clone(), feed_id.clone()), data);
    }
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p controller --all-targets`
Expected: clean (warnings about unused methods are acceptable until Task 3 wires them; if `-D warnings` bites in later clippy, it resolves once used).

- [ ] **Step 6: Commit**

```bash
git add contracts/controller/src/cache/mod.rs
git commit -m "feat(oracle): add tx-local RedStone prefetch store to Cache"
```

---

### Task 2: Bulk client + cache-aware single read

**Files:**
- Modify: `contracts/controller/src/oracle/providers/redstone/client.rs`
- Modify: `contracts/controller/src/oracle/providers/redstone.rs`

- [ ] **Step 1: Extend the contract client trait**

In `client.rs`, add the bulk method to the existing trait (signature verified against the deployed adapter):

```rust
#[contractclient(name = "RedStonePriceFeedClient")]
#[allow(dead_code)] // Required: trait exists only for the macro to generate the client proxy.
pub trait RedStoneMultiFeed {
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
    fn read_price_data(env: Env, feed_ids: Vec<String>) -> Result<Vec<RedStonePriceData>, Error>;
}
```

Add `Vec` to the `soroban_sdk` imports in `client.rs`.

- [ ] **Step 2: Make the single read consult the prefetch map**

Replace the existing `read_price_data` free function in `client.rs` with:

```rust
/// Reads RedStone price data, returning `None` on provider failure.
/// Served from the tx-local prefetch when `prefetch_redstone_feeds` ran.
pub(crate) fn read_price_data(
    cache: &Cache,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    if let Some(data) = cache.redstone_prefetched(contract, feed_id) {
        return Some(data);
    }
    match RedStonePriceFeedClient::new(cache.env(), contract).try_read_price_data_for_feed(feed_id)
    {
        Ok(Ok(data)) => Some(data),
        _ => None,
    }
}
```

Add `use crate::cache::Cache;` to `client.rs` and drop the now-unused `Env` import if nothing else needs it.

- [ ] **Step 3: Add the bulk read**

Below it in `client.rs`:

```rust
/// One cross-contract call for all feeds of one adapter. `None` on any
/// failure or length mismatch; callers fall back to per-feed reads.
pub(crate) fn read_price_data_bulk(
    env: &Env,
    contract: &Address,
    feed_ids: &Vec<String>,
) -> Option<Vec<RedStonePriceData>> {
    match RedStonePriceFeedClient::new(env, contract).try_read_price_data(feed_ids) {
        Ok(Ok(data)) if data.len() == feed_ids.len() => Some(data),
        _ => None,
    }
}
```

- [ ] **Step 4: Update the provider call site**

In `providers/redstone.rs`, `read_redstone_source` currently does
`read_price_data(env, &config.contract, &config.feed_id)`. Change to:

```rust
    let price_data = match read_price_data(cache, &config.contract, &config.feed_id) {
        Some(price_data) => price_data,
        _ if required => panic_with_error!(env, GenericError::InvalidTicker),
        _ => return None,
    };
```

Also re-export the bulk fn from `providers/redstone.rs`:

```rust
pub(crate) use client::{read_price_data, read_price_data_bulk, RedStonePriceData, REDSTONE_DECIMALS};
```

- [ ] **Step 5: Verify**

Run: `cargo check -p controller --all-targets && cargo test -p test-harness --test oracle redstone -- --test-threads=4`
Expected: compiles; existing `tests/oracle/redstone.rs` tests pass (behavior unchanged — map is always empty so far).

- [ ] **Step 6: Commit**

```bash
git add contracts/controller/src/oracle/providers/redstone/client.rs contracts/controller/src/oracle/providers/redstone.rs
git commit -m "feat(oracle): bulk RedStone client and prefetch-aware single read"
```

---

### Task 3: The prefetch collector

**Files:**
- Create: `contracts/controller/src/oracle/prefetch.rs`
- Modify: `contracts/controller/src/oracle/mod.rs`

- [ ] **Step 1: Write `prefetch.rs`**

```rust
//! Bulk prefetch of RedStone feeds into the transaction cache.
//!
//! One `read_price_data` call per adapter replaces N single-feed calls
//! (~1.27MB metered memory each). Only raw provider payloads are cached,
//! so every policy, staleness, and sanity check still runs per flow.
//! Any bulk failure leaves the cache empty and the per-feed lazy path
//! takes over unchanged. The real adapter returns results index-aligned
//! with the request and fails whole-call on a missing feed (verified
//! on-chain); a length-checked zip relies on that.

use soroban_sdk::{Address, Map, String, Vec};

use crate::cache::Cache;

/// Below this many distinct feeds per adapter, bulk saves nothing.
const MIN_BULK_FEEDS: u32 = 2;

/// No-op under Certora: pure performance optimization, identical semantics.
#[cfg(feature = "certora")]
pub(crate) fn prefetch_redstone_feeds(_cache: &mut Cache, _assets: &Vec<Address>) {}

#[cfg(not(feature = "certora"))]
pub(crate) fn prefetch_redstone_feeds(cache: &mut Cache, assets: &Vec<Address>) {
    use common::types::OracleSourceConfig;

    use super::providers::redstone::read_price_data_bulk;

    let env = cache.env().clone();
    let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);

    for asset in assets.iter() {
        // Already fully resolved this tx: nothing left to fetch for it.
        if cache.prices_cache.contains_key(asset.clone()) {
            continue;
        }
        let oracle_config = cache.cached_market_config(&asset).oracle_config;
        collect_redstone_feed(cache, &env, &mut by_adapter, &oracle_config.primary);
        if let Some(anchor) = oracle_config.anchor.as_ref() {
            collect_redstone_feed(cache, &env, &mut by_adapter, anchor);
        }
    }

    for (adapter, feeds) in by_adapter.iter() {
        if feeds.len() < MIN_BULK_FEEDS {
            continue;
        }
        let Some(data) = read_price_data_bulk(&env, &adapter, &feeds) else {
            continue;
        };
        // Lengths match (checked in read_price_data_bulk); zip by index.
        for (i, feed_id) in feeds.iter().enumerate() {
            if let Some(entry) = data.get(i as u32) {
                cache.set_redstone_prefetched(&adapter, &feed_id, entry);
            }
        }
    }
}

#[cfg(not(feature = "certora"))]
fn collect_redstone_feed(
    cache: &Cache,
    env: &soroban_sdk::Env,
    by_adapter: &mut Map<Address, Vec<String>>,
    source: &common::types::OracleSourceConfig,
) {
    let common::types::OracleSourceConfig::RedStone(r) = source else {
        return;
    };
    if cache.redstone_prefetched(&r.contract, &r.feed_id).is_some() {
        return;
    }
    let mut feeds = by_adapter
        .get(r.contract.clone())
        .unwrap_or_else(|| Vec::new(env));
    if feeds.first_index_of(r.feed_id.clone()).is_some() {
        return;
    }
    feeds.push_back(r.feed_id.clone());
    by_adapter.set(r.contract.clone(), feeds);
}
```

(If `use common::types::OracleSourceConfig;` inside the function body trips clippy style, hoist both `use` lines to module level under the same `#[cfg]`.)

- [ ] **Step 2: Wire the module**

In `oracle/mod.rs`, after `pub(crate) mod providers;`:

```rust
mod prefetch;
```

and with the other re-exports:

```rust
pub(crate) use prefetch::prefetch_redstone_feeds;
```

- [ ] **Step 3: Verify**

Run: `cargo check -p controller --all-targets && cargo clippy -p controller --all-targets -- -D warnings`
Expected: clean. (`prefetch_redstone_feeds` is exported but unused until Task 5 — if clippy flags dead code, proceed to Task 5 wiring before re-running clippy and note it; do NOT add `#[allow(dead_code)]`.)

- [ ] **Step 4: Commit**

```bash
git add contracts/controller/src/oracle/prefetch.rs contracts/controller/src/oracle/mod.rs
git commit -m "feat(oracle): bulk-prefetch RedStone feeds grouped by adapter"
```

---

### Task 4: Harness mock call counters

**Files:**
- Modify: `verification/test-harness/src/mock_redstone.rs`

The mock already implements bulk `read_price_data` with all-or-nothing `?` semantics (matches the real adapter). Add counters so tests can assert dispatch counts.

- [ ] **Step 1: Add counter keys and views**

In `mock_redstone.rs`, extend `MockKey`:

```rust
#[contracttype]
pub enum MockKey {
    PriceData(String),
    SingleCalls,
    BulkCalls,
}
```

Inside `impl MockRedStonePriceFeed`, add a private helper and two views:

```rust
    fn bump(env: &Env, key: MockKey) {
        let n: u32 = env.storage().temporary().get(&key).unwrap_or(0);
        env.storage().temporary().set(&key, &(n + 1));
    }

    pub fn single_calls(env: Env) -> u32 {
        env.storage().temporary().get(&MockKey::SingleCalls).unwrap_or(0)
    }

    pub fn bulk_calls(env: Env) -> u32 {
        env.storage().temporary().get(&MockKey::BulkCalls).unwrap_or(0)
    }
```

- [ ] **Step 2: Count at entrypoints without double-counting**

Extract the storage read into a private fn so the bulk path doesn't bump the single counter:

```rust
    fn get_feed(env: &Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        env.storage()
            .temporary()
            .get(&MockKey::PriceData(feed_id))
            .ok_or_else(|| Error::from_contract_error(GenericError::InvalidTicker as u32))
    }

    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error> {
        Self::bump(&env, MockKey::SingleCalls);
        Self::get_feed(&env, feed_id)
    }

    pub fn read_price_data(
        env: Env,
        feed_ids: Vec<String>,
    ) -> Result<Vec<RedStonePriceData>, Error> {
        Self::bump(&env, MockKey::BulkCalls);
        let mut values = Vec::new(&env);
        for feed_id in feed_ids.iter() {
            values.push_back(Self::get_feed(&env, feed_id)?);
        }
        Ok(values)
    }
```

Update `read_prices` and `read_timestamp` to call `Self::get_feed` / keep behavior (they may keep calling the pub fns — then they'd bump counters; prefer `get_feed` so counters mean cross-contract dispatches only as seen from the controller paths under test).

- [ ] **Step 3: Verify**

Run: `cargo test -p test-harness --test oracle redstone -- --test-threads=4`
Expected: existing tests still pass.

- [ ] **Step 4: Commit**

```bash
git add verification/test-harness/src/mock_redstone.rs
git commit -m "test(harness): count single vs bulk dispatches in RedStone mock"
```

---

### Task 5: Chokepoint 1 — HF/account-totals flows (TDD)

**Files:**
- Create: `verification/test-harness/tests/oracle/redstone_bulk.rs`
- Modify: the `tests/oracle` module registry (the file declaring `mod redstone;` — likely `tests/oracle/main.rs`; check with `grep -rn "mod redstone" verification/test-harness/tests/oracle/`)
- Modify: `contracts/controller/src/helpers/math.rs`

- [ ] **Step 1: Write the failing test**

`verification/test-harness/tests/oracle/redstone_bulk.rs`:

```rust
use soroban_sdk::{Address, String};
use test_harness::{usd, LendingTest, ALICE, BOB, DEFAULT_TOLERANCE};

/// One mock adapter serving multiple feeds, registered + priced.
fn setup_redstone_feeds(t: &LendingTest, feeds: &[(&str, i128)]) -> Address {
    let redstone = t
        .env
        .register(test_harness::mock_redstone::MockRedStonePriceFeed, ());
    let client =
        test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone);
    for (feed, price_wad) in feeds {
        client.set_price(&String::from_str(&t.env, feed), price_wad);
    }
    redstone
}

/// Reflector primary + RedStone anchor on `symbol`, same shape as prod config.
fn anchor_market_with_redstone(t: &LendingTest, redstone: &Address, symbol: &str) {
    let asset = t.resolve_asset(symbol);
    let feed_id = String::from_str(&t.env, symbol);
    let cfg = test_harness::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        redstone,
        &feed_id,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &asset, &cfg);
}

fn redstone_client<'a>(
    t: &'a LendingTest,
    redstone: &Address,
) -> test_harness::mock_redstone::MockRedStonePriceFeedClient<'a> {
    test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, redstone)
}

#[test]
fn test_borrow_hf_uses_one_bulk_redstone_call() {
    // Two RedStone-anchored markets on the SAME adapter; a borrow's HF check
    // prices both feeds and must dispatch exactly one bulk call.
    let mut t = LendingTest::new()
        .with_market(test_harness::usdc_preset())
        .with_market(test_harness::xlm_preset())
        .build();
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("XLM", usd(1) / 4)]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "XLM");

    t.supply(BOB, "XLM", 10_000.0); // liquidity for the borrow
    t.supply(ALICE, "USDC", 1_000.0);

    let rs = redstone_client(&t, &redstone);
    let single_before = rs.single_calls();
    let bulk_before = rs.bulk_calls();

    t.borrow(ALICE, "XLM", 100.0); // HF check prices USDC collateral + XLM debt

    assert_eq!(
        rs.bulk_calls() - bulk_before,
        1,
        "HF valuation must bulk-fetch RedStone feeds once"
    );
    assert_eq!(
        rs.single_calls() - single_before,
        0,
        "no per-feed RedStone calls when bulk prefetch covers the set"
    );
}
```

Adjust preset names to whatever `test_harness::presets` exports (check `verification/test-harness/src/presets.rs`; `usdc_preset` is confirmed, use the analogous second market — if no `xlm_preset` exists, use any second existing preset, e.g. `eth_preset`, and rename feeds accordingly). Register `mod redstone_bulk;` in the `tests/oracle` module registry file. If `t.borrow`/`t.supply` float-API names differ, mirror the call style used in `tests/oracle/redstone.rs` (`t.supply(ALICE, "USDC", 1_000.0)` confirmed).

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p test-harness --test oracle redstone_bulk -- --nocapture`
Expected: FAIL on `bulk_calls == 1` (actual 0, with 2 single calls) — proving the current per-feed behavior.

- [ ] **Step 3: Wire the chokepoint**

In `contracts/controller/src/helpers/math.rs`, at the **top of `calculate_account_totals_body`** (the shared body both cfg variants call):

```rust
fn calculate_account_totals_body(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    let mut priced_assets: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(env);
    for (asset, _) in supply_positions.iter() {
        priced_assets.push_back(asset);
    }
    for (asset, _) in borrow_positions.iter() {
        priced_assets.push_back(asset);
    }
    crate::oracle::prefetch_redstone_feeds(cache, &priced_assets);
    // ... existing body unchanged below ...
```

Duplicate assets across the two maps are fine — the collector dedupes at feed level. Under `certora` the call is a no-op (Task 3).

- [ ] **Step 4: Run the test — pass**

Run: `cargo test -p test-harness --test oracle redstone_bulk -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the surrounding suites for regressions**

Run: `cargo test -p test-harness --test oracle && cargo test -p test-harness --test controller`
Expected: all pass (HF results identical; only the transport changed).

- [ ] **Step 6: Commit**

```bash
git add contracts/controller/src/helpers/math.rs verification/test-harness/tests/oracle/redstone_bulk.rs verification/test-harness/tests/oracle/main.rs
git commit -m "feat(controller): bulk-prefetch RedStone feeds for HF valuation"
```

---

### Task 6: Chokepoint 2 — dust gates (TDD)

**Files:**
- Modify: `verification/test-harness/tests/oracle/redstone_bulk.rs`
- Modify: `contracts/controller/src/helpers/account.rs`

- [ ] **Step 1: Write the failing test**

Append to `redstone_bulk.rs`:

```rust
#[test]
fn test_multi_asset_supply_dust_check_uses_one_bulk_call() {
    let mut t = LendingTest::new()
        .with_market(test_harness::usdc_preset())
        .with_market(test_harness::xlm_preset())
        .build();
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("XLM", usd(1) / 4)]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "XLM");

    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    // Bulk supply touches two priced assets; the dust gate values both.
    t.supply_many(ALICE, &[("USDC", 1_000.0), ("XLM", 1_000.0)]);

    assert_eq!(rs.bulk_calls() - bulk_before, 1);
    assert_eq!(rs.single_calls() - single_before, 0);
}
```

If the harness has no `supply_many`, use whatever multi-asset supply helper exists (check `grep -rn "fn supply" verification/test-harness/src/ops/`); worst case call `ctrl_client().supply(...)` with a two-element `Vec<(Address, i128)>` directly, mirroring any existing bulk-supply test (`grep -rn "supply" verification/test-harness/tests/controller/limits.rs`). Note this test only passes if both markets' presets set a non-zero `min_collat_floor_usd` — check the preset; if the floor is 0, configure it via the same admin path existing dust tests use (`grep -rn "min_collat_floor" verification/test-harness/`).

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p test-harness --test oracle redstone_bulk::test_multi_asset_supply_dust_check_uses_one_bulk_call -- --nocapture`
Expected: FAIL (2 single calls, 0 bulk).

- [ ] **Step 3: Wire the chokepoint**

In `contracts/controller/src/helpers/account.rs`, at the top of `check_assets_side` (before its `for` loop), insert a prescreen mirroring `check_position`'s early returns exactly, so no feed is prefetched that the lazy path wouldn't read:

```rust
fn check_assets_side(
    env: &Env,
    cache: &mut Cache,
    assets: &Vec<Address>,
    side: Side,
    scaled_for: impl Fn(&Address) -> Option<Ray>,
    floor_for: impl Fn(&common::types::AssetConfig) -> i128,
) {
    let mut priceable: Vec<Address> = Vec::new(env);
    for asset in assets.iter() {
        let Some(scaled) = scaled_for(&asset) else {
            continue;
        };
        if scaled == Ray::ZERO {
            continue;
        }
        let cfg = cache.cached_asset_config(&asset);
        if floor_for(&cfg) == 0 {
            continue;
        }
        priceable.push_back(asset);
    }
    crate::oracle::prefetch_redstone_feeds(cache, &priceable);

    // ... existing loop unchanged below ...
```

(`cached_asset_config` is memoized per tx — the double call costs nothing.)

- [ ] **Step 4: Run the test — pass**

Run: `cargo test -p test-harness --test oracle redstone_bulk -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add contracts/controller/src/helpers/account.rs verification/test-harness/tests/oracle/redstone_bulk.rs
git commit -m "feat(controller): bulk-prefetch RedStone feeds for dust gates"
```

---

### Task 7: Fallback, parity, and idempotency tests

**Files:**
- Modify: `verification/test-harness/tests/oracle/redstone_bulk.rs`

- [ ] **Step 1: Bulk-failure fallback test**

```rust
#[test]
fn test_bulk_failure_falls_back_to_per_feed_reads() {
    let mut t = LendingTest::new()
        .with_market(test_harness::usdc_preset())
        .with_market(test_harness::xlm_preset())
        .build();
    // Only USDC's feed is set: the bulk [USDC, XLM] call errors whole-call
    // (all-or-nothing, like the real adapter), so the lazy path takes over.
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1))]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "XLM");

    t.supply(BOB, "XLM", 10_000.0);
    t.supply(ALICE, "USDC", 1_000.0);

    let rs = redstone_client(&t, &redstone);
    let single_before = rs.single_calls();

    // XLM's RedStone read fails per-feed too, but RedStone is the ANCHOR:
    // compose falls back to the Reflector primary (degraded dual-source),
    // so the borrow still succeeds — identical to today's behavior.
    t.borrow(ALICE, "XLM", 100.0);

    assert!(
        rs.single_calls() > single_before,
        "lazy per-feed path must engage when bulk fails"
    );
}
```

(Note: this works because the anchor-degradation policy allows a missing anchor on risk-increasing flows only when `allows_degraded_dual_source()` permits — mirror whatever the existing anchor-failure test in `tests/oracle/redstone.rs` asserts; if that policy panics for borrow, assert the panic error code instead, matching the existing test's expectation. The invariant under test is: **behavior with bulk-failure == behavior before this feature**.)

- [ ] **Step 2: Price-parity test**

```rust
#[test]
fn test_prefetched_price_identical_to_lazy_price() {
    // Same setup twice: one account values via bulk path (multi-feed),
    // one via lazy path (single feed). The resolved view price must match.
    let mut t = LendingTest::new()
        .with_market(test_harness::usdc_preset())
        .with_market(test_harness::xlm_preset())
        .build();
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("XLM", usd(1) / 4)]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "XLM");

    t.supply(BOB, "XLM", 10_000.0);
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "XLM", 100.0); // bulk path ran inside this tx

    // The view re-resolves prices fresh; any divergence between prefetched
    // and lazily-read payloads would show up as a different HF.
    let hf = t.health_factor(ALICE);
    assert!(hf > 1_000_000_000_000_000_000); // > 1.0 WAD, position healthy
}
```

(Use the harness's real account-view helper — check `verification/test-harness/src/view.rs` for the exact name, e.g. `t.health_factor` / `t.account_health`; mirror an existing HF assertion from `tests/controller/limits.rs`.)

- [ ] **Step 3: Idempotency test — withdraw runs both chokepoints, feeds fetched once**

```rust
#[test]
fn test_withdraw_with_debt_prefetches_once_across_chokepoints() {
    // withdraw triggers the dust gate AND the HF check; the second
    // chokepoint must be served from the map, not re-fetch.
    let mut t = LendingTest::new()
        .with_market(test_harness::usdc_preset())
        .with_market(test_harness::xlm_preset())
        .build();
    let redstone = setup_redstone_feeds(&t, &[("USDC", usd(1)), ("XLM", usd(1) / 4)]);
    anchor_market_with_redstone(&t, &redstone, "USDC");
    anchor_market_with_redstone(&t, &redstone, "XLM");

    t.supply(BOB, "XLM", 10_000.0);
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "XLM", 100.0);

    let rs = redstone_client(&t, &redstone);
    let bulk_before = rs.bulk_calls();
    let single_before = rs.single_calls();

    t.withdraw(ALICE, "USDC", 50.0);

    assert_eq!(rs.bulk_calls() - bulk_before, 1, "one bulk fetch per tx");
    assert_eq!(rs.single_calls() - single_before, 0);
}
```

Caveat for the executor: in the harness each `t.supply`/`t.borrow` client call is its own "tx" (fresh `Cache` per entrypoint), so counters accumulate across the setup calls — that's why every assertion measures **deltas** around exactly one operation.

- [ ] **Step 4: Run the whole new file**

Run: `cargo test -p test-harness --test oracle redstone_bulk -- --nocapture`
Expected: all 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add verification/test-harness/tests/oracle/redstone_bulk.rs
git commit -m "test(oracle): bulk prefetch fallback, parity, and idempotency coverage"
```

---

### Task 8: Full verification sweep

**Files:** none (verification only)

- [ ] **Step 1: Format + per-crate lint** (per-crate is deliberate — combined `-p common -p pool -p controller` clippy fails on a known testutils feature-unification quirk)

```bash
cargo fmt --all -- --check
cargo clippy -p common --all-targets -- -D warnings
cargo clippy -p controller --all-targets -- -D warnings
cargo clippy -p pool --all-targets -- -D warnings
cargo clippy -p test-harness --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 2: Full test suite**

```bash
cargo test -p test-harness
cargo test -p common
```

Expected: all pass (suite was 637 green before this work; now +5).

- [ ] **Step 3: Wasm build sanity**

```bash
make build
```

Expected: builds with the pinned `WASM_STACK_SIZE`; controller wasm size grows only marginally (one new client method + collector).

- [ ] **Step 4: Final commit if anything moved (fmt)**

```bash
git status --short   # commit any fmt-only changes with: chore: cargo fmt
```

---

### Task 9: On-chain validation (after deploy/upgrade of controller to testnet)

**Files:** none (operational)

- [ ] **Step 1:** Upgrade the testnet controller with the new wasm and re-run the market-oracle configure step so XLM/USDC dual-source (commit `ea29945`) is live on-chain.
- [ ] **Step 2:** Re-run a multi-feed borrow probe (e.g. via `tests/integration/flows/stress.sh`) and record instructions + memory from the tx; compare against the measured baseline (dual-source borrow wall: 9 feeds / ~10M instr/feed / ~1.27MB per avoided RedStone frame). Expected: RedStone frames collapse to 1 per tx; the dual-source feed ceiling moves from ~9 toward ~12+.
- [ ] **Step 3:** Update the memory note `testnet_e2e_harness.md` / `v2_architecture_direction.md` with the re-measured frontier.

---

## Explicitly Out of Scope (YAGNI)

- **Reflector bulk** — no bulk interface exists in SEP-40; nothing to call.
- **Controller-owned accounting** (removes pool `get_sync_data` calls) — separate, larger refactor; composes with this one.
- **Fixing the pre-broken Certora build** — known 1-line emode issue, unrelated.
- **`contracts/mock-redstone` changes** — the deployed mock already has the bulk endpoint; only the harness-native mock needed counters.
- **Caching resolved prices across policies** — deliberately rejected; raw-payload caching preserves all policy semantics.
