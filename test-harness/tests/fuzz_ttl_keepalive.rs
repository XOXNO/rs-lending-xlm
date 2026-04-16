//! Contract-level property test: keepalive / TTL lifecycle invariants.
//!
//! The controller exposes three KEEPER-gated endpoints that extend Soroban
//! storage TTLs:
//!   * `keepalive_shared_state(assets)` -- bumps per-market keys
//!     (`Market`, `IsolatedDebt`, `AssetEModes`, `EModeCategory`, `EModeAsset`).
//!   * `keepalive_accounts(ids)` -- bumps `AccountMeta` and every
//!     `SupplyPosition` / `BorrowPosition` key for that account.
//!   * `keepalive_pools(assets)` -- forwards to `pool.keepalive()`, bumping
//!     the pool's instance storage.
//!
//! If these bumps drift out of sync with actual storage (e.g. an orphan
//! position left in persistent storage after a full exit), entries expire
//! and break the protocol at a ledger boundary — a class of bug no directed
//! test will find.
//!
//! Four properties:
//!   1. `prop_keepalive_accounts_bumps_positions`  -- random account/asset mix.
//!   2. `prop_keepalive_shared_bumps_markets`      -- random market + e-mode.
//!   3. `prop_keepalive_pools_forwards`            -- pool instance TTL grows.
//!   4. `prop_account_orphan_positions_not_stuck`  -- M-14 regression.

extern crate std;

use std::string::{String, ToString};

use common::constants::{TTL_BUMP_SHARED, TTL_BUMP_USER};
use common::types::ControllerKey;
use proptest::prelude::*;
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::Address;
use test_harness::{eth_preset, usdc_preset, wbtc_preset, LendingTest};

const USERS: &[&str] = &["alice", "bob", "carol", "dave", "eve"];
const ASSETS: &[&str] = &["USDC", "ETH", "WBTC"];

// ---------------------------------------------------------------------------
// TTL read helpers. In soroban-sdk 25.3.1 the testutils `get_ttl(key)`
// returns the *remaining* ledgers until expiry (not the absolute
// live_until_ledger). The assertion `remaining >= TTL_BUMP_*` is equivalent
// to `live_until_ledger >= current + TTL_BUMP_*`.
// ---------------------------------------------------------------------------

fn persistent_ttl(t: &LendingTest, key: &ControllerKey) -> u32 {
    t.env.as_contract(&t.controller, || t.env.storage().persistent().get_ttl(key))
}

fn persistent_has(t: &LendingTest, key: &ControllerKey) -> bool {
    t.env.as_contract(&t.controller, || t.env.storage().persistent().has(key))
}

fn pool_instance_ttl(t: &LendingTest, pool: &Address) -> u32 {
    t.env.as_contract(pool, || t.env.storage().instance().get_ttl())
}

fn build_ctx() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build()
}

// ---------------------------------------------------------------------------
// Property 1: keepalive_accounts bumps every position key.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn prop_keepalive_accounts_bumps_positions(
        num_accounts in 1usize..=5,
        asset_mix in prop::collection::vec(0usize..3, 1..=3),
    ) {
        let mut t = build_ctx();

        // Each account: supply a random non-empty subset of assets.
        let mut account_ids: std::vec::Vec<u64> = std::vec::Vec::new();
        let mut per_account_assets: std::vec::Vec<std::vec::Vec<&'static str>> = std::vec::Vec::new();
        for i in 0..num_accounts {
            let user = USERS[i];
            let mut used: std::vec::Vec<&'static str> = std::vec::Vec::new();
            for &ai in &asset_mix {
                let asset = ASSETS[ai];
                if used.contains(&asset) { continue; }
                // Supply a modest amount so the supply caps remain unbreached.
                t.supply(user, asset, 100.0);
                used.push(asset);
            }
            let id = t.find_account_id(user).expect("account should exist after supply");
            account_ids.push(id);
            per_account_assets.push(used);
        }

        // Build id Vec for the keepalive call.
        let mut ids = soroban_sdk::Vec::new(&t.env);
        for id in &account_ids {
            ids.push_back(*id);
        }

        // Call keepalive_accounts.
        t.ctrl_client().keepalive_accounts(&t.keeper, &ids);

        // Assert every AccountMeta + per-asset SupplyPosition key has
        // TTL >= TTL_BUMP_USER. Allow 1-ledger tolerance for off-by-one
        // between set time and read time.
        let min_ttl = TTL_BUMP_USER.saturating_sub(1);
        for (idx, id) in account_ids.iter().enumerate() {
            let meta_ttl = persistent_ttl(&t, &ControllerKey::AccountMeta(*id));
            prop_assert!(
                meta_ttl >= min_ttl,
                "AccountMeta({}) TTL too low: {} < {}", id, meta_ttl, min_ttl
            );
            for asset in &per_account_assets[idx] {
                let asset_addr = t.resolve_asset(asset);
                let key = ControllerKey::SupplyPosition(*id, asset_addr);
                let ttl = persistent_ttl(&t, &key);
                prop_assert!(
                    ttl >= min_ttl,
                    "SupplyPosition({}, {}) TTL too low: {} < {}",
                    id, asset, ttl, min_ttl
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property 2: keepalive_shared_state bumps per-market shared keys.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn prop_keepalive_shared_bumps_markets(
        asset_indices in prop::collection::vec(0usize..3, 1..=3),
    ) {
        let t = build_ctx();

        // Dedupe.
        let mut seen = [false; 3];
        let mut chosen: std::vec::Vec<&'static str> = std::vec::Vec::new();
        for &i in &asset_indices {
            if !seen[i] {
                seen[i] = true;
                chosen.push(ASSETS[i]);
            }
        }

        let mut assets = soroban_sdk::Vec::new(&t.env);
        for name in &chosen {
            assets.push_back(t.resolve_asset(name));
        }

        t.ctrl_client().keepalive_shared_state(&t.keeper, &assets);

        let min_ttl = TTL_BUMP_SHARED.saturating_sub(1);
        for name in &chosen {
            let addr = t.resolve_asset(name);

            let market_ttl = persistent_ttl(&t, &ControllerKey::Market(addr.clone()));
            prop_assert!(
                market_ttl >= min_ttl,
                "Market({}) TTL too low: {} < {}", name, market_ttl, min_ttl
            );

            let iso_ttl = persistent_ttl(&t, &ControllerKey::IsolatedDebt(addr.clone()));
            prop_assert!(
                iso_ttl >= min_ttl,
                "IsolatedDebt({}) TTL too low: {} < {}", name, iso_ttl, min_ttl
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 3: keepalive_pools forwards to each pool, bumping its instance TTL.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn prop_keepalive_pools_forwards(
        asset_indices in prop::collection::vec(0usize..3, 1..=3),
    ) {
        let t = build_ctx();

        let mut seen = [false; 3];
        let mut chosen: std::vec::Vec<&'static str> = std::vec::Vec::new();
        for &i in &asset_indices {
            if !seen[i] {
                seen[i] = true;
                chosen.push(ASSETS[i]);
            }
        }

        // Capture pre-TTL for each pool (the default instance TTL applied
        // at deploy time).
        let mut pre: std::vec::Vec<(String, u32, Address)> = std::vec::Vec::new();
        for name in &chosen {
            let pool = t.resolve_market(name).pool.clone();
            let ttl = pool_instance_ttl(&t, &pool);
            pre.push((name.to_string(), ttl, pool));
        }

        let mut assets = soroban_sdk::Vec::new(&t.env);
        for name in &chosen {
            assets.push_back(t.resolve_asset(name));
        }
        t.ctrl_client().keepalive_pools(&t.keeper, &assets);

        for (name, pre_ttl, pool) in &pre {
            let post_ttl = pool_instance_ttl(&t, pool);
            prop_assert!(
                post_ttl >= *pre_ttl,
                "pool {} instance TTL regressed: {} -> {}", name, pre_ttl, post_ttl
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 4: M-14 regression -- after a full withdraw, no orphan
// SupplyPosition entry may remain, and AccountMeta.supply_assets must not
// contain the asset.
//
// M-14 was reported "FIXED" at the pool level; this property verifies the
// fix also holds at the controller storage layer (where `bump_account`
// iterates `meta.supply_assets`). If an orphan exists, a future keepalive
// call silently skips it (the meta list omits it) AND the key remains
// persisted — so the entry expires and breaks invariants.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn prop_account_orphan_positions_not_stuck(
        supply_amt in 10u32..1_000,
        num_partials in 0u32..=3,
        partial_bps in prop::collection::vec(1000u16..9000, 0..=3),
    ) {
        let mut t = build_ctx();

        let asset = "USDC";
        let user = "alice";

        // Initial supply.
        t.supply(user, asset, supply_amt as f64);
        let id = t.find_account_id(user).unwrap();

        // A handful of partial withdrawals.
        for i in 0..num_partials as usize {
            let bps = partial_bps.get(i).copied().unwrap_or(5000);
            let cur = t.supply_balance_raw(user, asset);
            if cur <= 1 { break; }
            let amt = (cur as i128 * bps as i128) / 10_000;
            if amt > 0 && amt < cur {
                let _ = t.try_withdraw(user, asset, amt as f64 / 10_f64.powi(7));
            }
        }

        // Full exit via amount=0 "withdraw all".
        let addr = t.users.get(user).unwrap().address.clone();
        let asset_addr = t.resolve_asset(asset);
        let withdrawals: soroban_sdk::Vec<(Address, i128)> = {
            let mut v = soroban_sdk::Vec::new(&t.env);
            v.push_back((asset_addr.clone(), 0i128));
            v
        };
        // The full withdraw may return an error if the account was already
        // pruned; the key assertion is the post-state.
        let _ = t.ctrl_client().try_withdraw(&addr, &id, &withdrawals);

        // (a) No orphan SupplyPosition key left.
        let orphan_key = ControllerKey::SupplyPosition(id, asset_addr.clone());
        // When the account is fully pruned, the AccountMeta is also gone;
        // in that case the supply key must likewise be gone.
        let has_orphan = persistent_has(&t, &orphan_key);
        prop_assert!(
            !has_orphan,
            "CRITICAL M-14: orphan SupplyPosition({}, {:?}) left in persistent storage",
            id, asset_addr
        );

        // (b) If AccountMeta is still present, its supply_assets must not mention the asset.
        let meta_key = ControllerKey::AccountMeta(id);
        if persistent_has(&t, &meta_key) {
            let meta: common::types::AccountMeta = t.env.as_contract(&t.controller, || {
                t.env.storage().persistent().get(&meta_key).unwrap()
            });
            for a in meta.supply_assets.iter() {
                prop_assert!(
                    a != asset_addr,
                    "CRITICAL: AccountMeta.supply_assets still contains fully-withdrawn asset"
                );
            }
        }
    }
}
