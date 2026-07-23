use common::errors::{GenericError, SpokeError};
use controller::types::{ControllerKey, SpokeConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN};
use test_harness::{
    assert_contract_error, hub_asset, usdc_preset, HubAssetKey, LendingTest, ALICE,
    STABLECOIN_SPOKE,
};

fn upload_pool_wasm(env: &soroban_sdk::Env) -> BytesN<32> {
    let mut bytes = std::fs::read("target/wasm32v1-none/release/pool.wasm");
    if bytes.is_err() {
        bytes = std::fs::read("../../target/wasm32v1-none/release/pool.wasm");
    }
    let bytes = bytes.expect("Liquidity pool WASM not found. Run 'make build' first.");
    env.deployer()
        .upload_contract_wasm(soroban_sdk::Bytes::from_slice(env, &bytes))
}

// 1. upgrade_pool -- admin path. Re-uploads the pool wasm so the Soroban host
//    accepts a no-op upgrade without altering pool behavior.

#[test]
fn test_upgrade_pool_admin_path() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let reserves_before = t.pool_reserves("USDC");

    let live_hash = upload_pool_wasm(&t.env);
    t.ctrl_client().upgrade_pool(&live_hash);
    assert_eq!(t.pool_reserves("USDC"), reserves_before);
}

// 2. PoolNotInitialized -- create_liquidity_pool must panic with
//     GenericError::PoolNotInitialized (#30) when the global pool has not been
//     deployed yet.

#[test]
fn test_create_liquidity_pool_panics_before_deploy_pool() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();

    let admin = Address::generate(&env);
    let controller = env.register(controller::Controller, (admin.clone(),));
    let ctrl = controller::ControllerClient::new(&env, &controller);

    ctrl.unpause();

    // Register a hub so the mandatory hub-active check passes and the
    // missing-pool check becomes the next step under test.
    let hub = ctrl.create_hub();

    // A real SAC token satisfies the decimals + symbol + allow-list probes
    // inside `create_liquidity_pool` so the missing-pool check is the next
    // step.
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let preset = usdc_preset();
    let params = preset.params.to_market_params(&asset, preset.decimals);

    let result = match ctrl.try_create_liquidity_pool(&hub, &asset, &params) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, GenericError::PoolNotInitialized as u32);
}

// 2b. PoolAlreadyDeployed -- a second deploy_pool must panic with
//     GenericError::PoolAlreadyDeployed (#5); the builder already ran the
//     first deployment.

#[test]
fn test_deploy_pool_panics_on_second_call() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let hash = upload_pool_wasm(&t.env);

    let result = match t.ctrl_client().try_deploy_pool(&hash) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, GenericError::PoolAlreadyDeployed as u32);
}

// 3. Deprecated spoke reject on the user path. Sequence:
//      a) admin opens an spoke category and adds USDC to it;
//      b) ALICE opens an account in that category (still active);
//      c) admin removes (deprecates) the category;
//      d) ALICE attempts a fresh supply on the same account -- supply
//         calls `active_spoke_category(env, account.spoke_id)`,
//         which panics with SpokeDeprecated (#301).
//
//    The account is created via the harness storage shim while the category
//    is still active (the shim asserts non-deprecated, mirroring the
//    on-chain `create_account` validation), so the reject must come from
//    the supply path, not from account creation.

#[test]
fn test_supply_panics_on_deprecated_spoke_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    // Open an account in category 2 while it is still active.
    let account_id = t.create_spoke_account(ALICE, 2);

    // Sanity check: the account's stored category id is 2.
    let stored_id: u32 = t.env.as_contract(&t.controller_address(), || {
        let meta: controller::types::AccountMeta = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::AccountMeta(account_id))
            .expect("account meta must exist");
        meta.spoke_id
    });
    assert_eq!(stored_id, 2, "account must be in spoke category 2");

    // Deprecate the category.
    t.remove_spoke_category(2);

    // Confirm the category is flagged deprecated in storage.
    let deprecated: bool = t.env.as_contract(&t.controller_address(), || {
        let cat: SpokeConfig = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::Spoke(2))
            .expect("category must still exist (only flagged)");
        cat.is_deprecated
    });
    assert!(deprecated, "category 2 must be flagged deprecated");

    // The next supply on the same account must panic with
    // SpokeDeprecated (#301) from `active_spoke_category`.
    let alice_addr = t.users.get(ALICE).unwrap().address.clone();
    let asset_addr = t.resolve_asset("USDC");
    let market = t.resolve_market("USDC");
    let amount = test_harness::f64_to_i128(100.0, market.decimals);
    market.token_admin.mint(&alice_addr, &amount);

    let payments: soroban_sdk::Vec<(HubAssetKey, i128)> =
        soroban_sdk::vec![&t.env, (hub_asset(asset_addr), amount)];
    let ctrl = t.ctrl_client();
    let result = match ctrl.try_supply(&alice_addr, &account_id, &2u32, &payments) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(err),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, SpokeError::SpokeDeprecated as u32);
}
