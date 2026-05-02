//! Coverage for keeper / admin entry points exposed by the controller
//! (`controller/src/router.rs`) and the user-visible deprecated-eMode reject
//! path (`controller/src/positions/emode.rs:95`).
//!
//! Each test is intentionally narrow:
//!   - happy keepalive paths drive the keeper-signature branches and the
//!     per-asset bumps inside `keepalive_shared_state` and `keepalive_pools`;
//!   - skip-on-missing branches hit the `!has_market_config` early-continue
//!     in both loops by appending an unregistered asset address;
//!   - `upgrade_pool` is exercised against a known-good wasm hash;
//!   - `TemplateEmpty` is reached on a freshly-registered controller that has
//!     no pool template set;
//!   - the deprecated-eMode reject runs the full `add -> remove -> supply`
//!     sequence so that `active_e_mode_category` panics with #301.
extern crate std;

use common::errors::{EModeError, GenericError};
use common::types::{ControllerKey, EModeCategory};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Vec};
use test_harness::{
    assert_contract_error, eth_preset, usdc_preset, wbtc_preset, LendingTest, ALICE,
    STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. keepalive_pools -- happy path with multiple assets + skip-on-missing.
//    Covers router.rs lines 41-44 (entry + signature), 373-383 (loop body)
//    and the `!has_market_config` skip at line 376-378.
// ---------------------------------------------------------------------------

#[test]
fn test_keepalive_pools_iterates_and_skips_unknown() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Build an asset list that mixes registered markets and a stray address
    // with no market config so the loop hits both branches.
    let stray = Address::generate(&t.env);
    let mut assets: Vec<Address> = Vec::new(&t.env);
    assets.push_back(t.resolve_asset("USDC"));
    assets.push_back(stray);
    assets.push_back(t.resolve_asset("ETH"));
    assets.push_back(t.resolve_asset("WBTC"));

    // Must not panic; the keeper signature is satisfied and the loop must
    // tolerate a missing market config without aborting.
    t.ctrl_client().keepalive_pools(&t.keeper, &assets);
}

// ---------------------------------------------------------------------------
// 2. keepalive_shared_state -- exercises the deeper per-asset branches:
//    `Market` and `IsolatedDebt` always; `AssetEModes`, `EModeCategory`,
//    `EModeAssets` only when the asset belongs to at least one e-mode
//    category. Covers router.rs lines 338-362 and the `!has_market_config`
//    skip at 344-346.
// ---------------------------------------------------------------------------

#[test]
fn test_keepalive_shared_state_bumps_emode_keys() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Stray address triggers the skip-on-missing branch.
    let stray = Address::generate(&t.env);
    let mut assets: Vec<Address> = Vec::new(&t.env);
    assets.push_back(t.resolve_asset("USDC")); // member of e-mode 1
    assets.push_back(t.resolve_asset("ETH")); // no e-mode -> AssetEModes empty
    assets.push_back(stray); // no market config -> skip

    t.ctrl_client().keepalive_shared_state(&t.keeper, &assets);
}

// ---------------------------------------------------------------------------
// 3. upgrade_pool -- admin path. Covers router.rs line 67-69 (entry) and
//    252-257 (body: require_asset_supported + get_market_config +
//    pool_client.upgrade). The same wasm hash already stored as the pool
//    template is re-used: the Soroban host accepts a no-op upgrade for a
//    known-uploaded hash, so we drive the full code path without needing a
//    second wasm blob.
// ---------------------------------------------------------------------------

#[test]
fn test_upgrade_pool_admin_path() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Read the template hash that build() set on the controller.
    let template_hash: BytesN<32> = t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .instance()
            .get(&ControllerKey::PoolTemplate)
            .expect("pool template must be set after build()")
    });

    // Drive the admin-gated upgrade entry point. With the controller's
    // own template hash this is a no-op upgrade that exercises every line
    // of `upgrade_liquidity_pool` without altering pool behavior.
    let asset = t.resolve_asset("USDC");
    t.ctrl_client().upgrade_pool(&asset, &template_hash);
}

// ---------------------------------------------------------------------------
// 4. TemplateEmpty -- create_liquidity_pool must panic with
//    GenericError::TemplateEmpty (#5) when no pool template is set.
//    Covers router.rs line 142-144.
//
//    A fresh controller registered outside the LendingTest builder gives us
//    a state where `has_pool_template == false` while still allowing us to
//    pre-approve a token contract and reach the template check.
// ---------------------------------------------------------------------------

#[test]
fn test_create_liquidity_pool_panics_when_template_unset() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();

    let admin = Address::generate(&env);
    let controller = env.register(controller::Controller, (admin.clone(),));
    let ctrl = controller::ControllerClient::new(&env, &controller);

    // Mirror the post-deploy operator runbook so we land on the template
    // check rather than the pause / role gates.
    ctrl.unpause();

    // A real SAC token satisfies the decimals + symbol + allow-list probes
    // inside `create_liquidity_pool` so the template check is the next step.
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    ctrl.approve_token_wasm(&asset);

    let preset = usdc_preset();
    let params = preset.params.to_market_params(&asset, preset.decimals);
    let config = preset.config.to_asset_config();

    let result = match ctrl.try_create_liquidity_pool(&asset, &params, &config) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, GenericError::TemplateEmpty as u32);
}

// ---------------------------------------------------------------------------
// 5. Deprecated e-mode reject on the user path. Covers
//    `controller/src/positions/emode.rs:95`. Sequence:
//      a) admin opens an e-mode category and adds USDC to it;
//      b) ALICE opens an account in that category (still active);
//      c) admin removes (deprecates) the category;
//      d) ALICE attempts a fresh supply on the same account -- supply
//         calls `active_e_mode_category(env, account.e_mode_category_id)`,
//         which panics with EModeCategoryDeprecated (#301).
//
//    The account is created via the harness storage shim while the category
//    is still active (the shim asserts non-deprecated, mirroring the
//    on-chain `create_account` validation), so the reject must come from
//    the supply path, not from account creation.
// ---------------------------------------------------------------------------

#[test]
fn test_supply_panics_on_deprecated_emode_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Open an account in category 1 while it is still active.
    let account_id = t.create_emode_account(ALICE, 1);

    // Sanity check: the account's stored category id is 1.
    let stored_id: u32 = t.env.as_contract(&t.controller_address(), || {
        let meta: common::types::AccountMeta = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::AccountMeta(account_id))
            .expect("account meta must exist");
        meta.e_mode_category_id
    });
    assert_eq!(stored_id, 1, "account must be in e-mode category 1");

    // Deprecate the category.
    t.remove_e_mode_category(1);

    // Confirm the category is now flagged deprecated in storage.
    let deprecated: bool = t.env.as_contract(&t.controller_address(), || {
        let cat: EModeCategory = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::EModeCategory(1))
            .expect("category must still exist (only flagged)");
        cat.is_deprecated
    });
    assert!(deprecated, "category 1 must be flagged deprecated");

    // The next supply on the same account must panic with
    // EModeCategoryDeprecated (#301) from `active_e_mode_category`.
    let alice_addr = t.users.get(ALICE).unwrap().address.clone();
    let asset_addr = t.resolve_asset("USDC");
    let market = t.resolve_market("USDC");
    let amount = test_harness::f64_to_i128(100.0, market.decimals);
    market.token_admin.mint(&alice_addr, &amount);

    let payments: soroban_sdk::Vec<(Address, i128)> =
        soroban_sdk::vec![&t.env, (asset_addr, amount)];
    let ctrl = t.ctrl_client();
    let result = match ctrl.try_supply(&alice_addr, &account_id, &0u32, &payments) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(err),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, EModeError::EModeCategoryDeprecated as u32);
}
