use controller::types::InterestRateModel;
use controller::types::SpokeAssetArgs;
use governance_interface::AdminOperation;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{Address, BytesN, InvokeError, Vec as SVec};
use test_harness::{hub_asset, HubAssetKey, LendingTest, HARNESS_HUB};

/// Checks that an unauthenticated privileged call fails with a host error, not a contract error.
///
/// Every failure arrives in the outer `Err`. A host error of any type except
/// `ScErrorType::Contract` passes. A `Contract` error means the call got past the auth
/// check. A bare `InvokeError` or any `Ok` result fails.
/// The host narrows every non-contract failure to `Error(Context, InvalidAction)`, so a
/// pass cannot tell an auth rejection from any other host trap.
fn expect_rejected<F, R, InnerErr>(label: &str, call: F) -> Result<(), String>
where
    F: FnOnce() -> Result<Result<R, InnerErr>, Result<soroban_sdk::Error, InvokeError>>,
{
    match call() {
        Err(Ok(err)) if !err.is_type(ScErrorType::Contract) => Ok(()),
        Err(Ok(err)) => Err(format!(
            "CRITICAL: {label} passed the auth gate and was stopped by contract logic: {err:?}"
        )),
        Err(Err(invoke)) => Err(format!(
            "CRITICAL: {label} failed with a bare InvokeError, not a host error value: {invoke:?}"
        )),
        Ok(Ok(_)) => Err(format!(
            "CRITICAL: {label} executed successfully without auth"
        )),
        Ok(Err(_)) => Err(format!(
            "CRITICAL: {label} executed without auth (only the return value failed to convert)"
        )),
    }
}

fn sample_position_limits() -> controller::types::PositionLimits {
    controller::types::PositionLimits {
        max_supply_positions: 5,
        max_borrow_positions: 5,
    }
}

fn dummy_bytes_n(env: &soroban_sdk::Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

#[test]
fn owner_only_endpoints_reject_unauthed_before_validation() {
    let ltv = 7_500;
    let threshold = 8_000;
    let bonus = 500;
    let category_id = 1;
    let can_collateral = true;
    let can_borrow = true;
    let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    let env = t.env.clone();
    let ctrl = t.ctrl_client();
    // A hash the host can actually resolve: the builder already uploaded it.
    let real_wasm = t.position_nft_wasm_hash.clone();
    let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];
    let limits = sample_position_limits();
    let usdc = t.resolve_asset("USDC");
    let random_addr = Address::generate(&env);

    expect_rejected("pause", || ctrl.set_auths(&no_auths).try_pause()).unwrap();
    expect_rejected("unpause", || ctrl.set_auths(&no_auths).try_unpause()).unwrap();
    expect_rejected("transfer_ownership", || {
        ctrl.set_auths(&no_auths)
            .try_transfer_ownership(&random_addr, &1_000_000u32)
    })
    .unwrap();
    expect_rejected("set_aggregator", || {
        ctrl.set_auths(&no_auths)
            .try_set_swap_aggregator(&random_addr)
    })
    .unwrap();
    expect_rejected("set_accumulator", || {
        ctrl.set_auths(&no_auths).try_set_accumulator(&random_addr)
    })
    .unwrap();
    expect_rejected("set_position_limits", || {
        ctrl.set_auths(&no_auths).try_set_position_limits(&limits)
    })
    .unwrap();
    expect_rejected("create_hub", || ctrl.set_auths(&no_auths).try_create_hub()).unwrap();
    expect_rejected("add_spoke", || ctrl.set_auths(&no_auths).try_add_spoke()).unwrap();
    expect_rejected("remove_spoke_category", || {
        ctrl.set_auths(&no_auths).try_remove_spoke(&category_id)
    })
    .unwrap();
    expect_rejected("add_asset_to_spoke", || {
        ctrl.set_auths(&no_auths)
            .try_add_asset_to_spoke(&SpokeAssetArgs {
                liquidation_fees: 0,
                hub_id: HARNESS_HUB,
                asset: usdc.clone(),
                spoke_id: category_id,
                can_collateral,
                can_borrow,
                paused: false,
                frozen: false,
                no_seize: false,
                ltv,
                threshold,
                bonus,
                supply_cap: 0,
                borrow_cap: 0,
            })
    })
    .unwrap();
    expect_rejected("edit_asset_in_spoke", || {
        ctrl.set_auths(&no_auths)
            .try_edit_asset_in_spoke(&SpokeAssetArgs {
                liquidation_fees: 0,
                hub_id: HARNESS_HUB,
                asset: usdc.clone(),
                spoke_id: category_id,
                can_collateral,
                can_borrow,
                paused: false,
                frozen: false,
                no_seize: false,
                ltv,
                threshold,
                bonus,
                supply_cap: 0,
                borrow_cap: 0,
            })
    })
    .unwrap();
    expect_rejected("remove_asset_from_spoke", || {
        ctrl.set_auths(&no_auths)
            .try_remove_asset_from_spoke(&hub_asset(usdc.clone()), &category_id)
    })
    .unwrap();
    expect_rejected("set_spoke_asset_flags", || {
        ctrl.set_auths(&no_auths).try_set_spoke_asset_flags(
            &category_id,
            &hub_asset(usdc.clone()),
            &true,
            &true,
            &true,
        )
    })
    .unwrap();
    let live_epoch = ctrl.get_spoke_asset_flags_epoch(&category_id, &hub_asset(usdc.clone()));
    expect_rejected("relax_spoke_asset_flags", || {
        ctrl.set_auths(&no_auths).try_relax_spoke_asset_flags(
            &category_id,
            &hub_asset(usdc.clone()),
            &live_epoch,
            &false,
            &false,
            &false,
        )
    })
    .unwrap();
    expect_rejected("set_spoke_liquidation_curve", || {
        ctrl.set_auths(&no_auths).try_set_spoke_liquidation_curve(
            &category_id,
            &(controller::constants::WAD + controller::constants::WAD / 50),
            &(controller::constants::WAD / 2),
            &8_000u32,
        )
    })
    .unwrap();
    expect_rejected("upgrade", || {
        ctrl.set_auths(&no_auths).try_upgrade(&real_wasm)
    })
    .unwrap();
    expect_rejected("migrate", || ctrl.set_auths(&no_auths).try_migrate(&2u32)).unwrap();
    expect_rejected("force_socialize_bad_debt", || {
        ctrl.set_auths(&no_auths)
            .try_force_socialize_bad_debt(&1u64)
    })
    .unwrap();
    expect_rejected("upgrade_pool", || {
        ctrl.set_auths(&no_auths).try_upgrade_pool(&real_wasm)
    })
    .unwrap();
    // The builder already deployed the pool, so without the auth check this call
    // fails with `PoolAlreadyDeployed`, a contract error.
    expect_rejected("deploy_pool", || {
        ctrl.set_auths(&no_auths).try_deploy_pool(&real_wasm)
    })
    .unwrap();
    let zero_model = InterestRateModel {
        max_borrow_rate: 0,
        base_borrow_rate: 0,
        slope1: 0,
        slope2: 0,
        slope3: 0,
        mid_utilization: 0,
        optimal_utilization: 0,
        max_utilization: controller::constants::RAY * 95 / 100,
        reserve_factor: 0,
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    expect_rejected("upgrade_liquidity_pool_params", || {
        ctrl.set_auths(&no_auths)
            .try_upgrade_liquidity_pool_params(&hub_asset(usdc.clone()), &zero_model)
    })
    .unwrap();
    expect_rejected("create_liquidity_pool", || {
        let params = controller::types::MarketParamsRaw {
            max_borrow_rate: 0,
            base_borrow_rate: 0,
            slope1: 0,
            slope2: 0,
            slope3: 0,
            mid_utilization: 0,
            optimal_utilization: 0,
            max_utilization: controller::constants::RAY * 95 / 100,
            reserve_factor: 0,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: usdc.clone(),
            asset_decimals: 7,
        };

        ctrl.set_auths(&no_auths)
            .try_create_liquidity_pool(&HARNESS_HUB, &usdc, &params)
    })
    .unwrap();

    let empty_assets: SVec<HubAssetKey> = SVec::new(&env);
    let empty_ids: SVec<u64> = SVec::new(&env);

    expect_rejected("update_indexes (caller auth)", || {
        ctrl.set_auths(&no_auths)
            .try_update_indexes(&random_addr, &empty_assets)
    })
    .unwrap();
    expect_rejected("clean_bad_debt (caller auth)", || {
        ctrl.set_auths(&no_auths)
            .try_clean_bad_debt(&random_addr, &0u64)
    })
    .unwrap();
    expect_rejected("update_account_threshold (caller auth)", || {
        ctrl.set_auths(&no_auths)
            .try_update_account_threshold(&random_addr, &false, &empty_ids)
    })
    .unwrap();
    expect_rejected("claim_revenue (caller auth)", || {
        ctrl.set_auths(&no_auths)
            .try_claim_revenue(&random_addr, &empty_assets)
    })
    .unwrap();
    expect_rejected("set_price_aggregator", || {
        ctrl.set_auths(&no_auths)
            .try_set_price_aggregator(&random_addr)
    })
    .unwrap();
    expect_rejected("approve_blend_pool", || {
        ctrl.set_auths(&no_auths)
            .try_approve_blend_pool(&random_addr)
    })
    .unwrap();
    expect_rejected("revoke_blend_pool", || {
        ctrl.set_auths(&no_auths)
            .try_revoke_blend_pool(&random_addr)
    })
    .unwrap();
    expect_rejected("set_min_borrow_collateral_usd", || {
        ctrl.set_auths(&no_auths)
            .try_set_min_borrow_collateral_usd(&1)
    })
    .unwrap();
    expect_rejected("set_position_manager", || {
        ctrl.set_auths(&no_auths)
            .try_set_position_manager(&random_addr, &true)
    })
    .unwrap();
    expect_rejected("deploy_position_nft", || {
        ctrl.set_auths(&no_auths).try_deploy_position_nft(
            &real_wasm,
            &soroban_sdk::String::from_str(&env, "u"),
            &soroban_sdk::String::from_str(&env, "n"),
            &soroban_sdk::String::from_str(&env, "s"),
        )
    })
    .unwrap();
    expect_rejected("upgrade_position_nft", || {
        ctrl.set_auths(&no_auths)
            .try_upgrade_position_nft(&real_wasm)
    })
    .unwrap();
}

#[test]
fn governance_endpoints_reject_unauthed_before_validation() {
    let seed = 0;
    let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    let env = t.env.clone();
    let gov = t.gov_client();
    let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];
    let limits = sample_position_limits();
    let random_addr = Address::generate(&env);
    let salt = dummy_bytes_n(&env, seed);
    let real_wasm = t.position_nft_wasm_hash.clone();

    expect_rejected("gov.propose(SetPositionLimits)", || {
        gov.set_auths(&no_auths).try_propose(
            &random_addr,
            &AdminOperation::SetPositionLimits(limits),
            &salt,
        )
    })
    .unwrap();

    expect_rejected("gov.propose(UpdateGovDelay)", || {
        gov.set_auths(&no_auths).try_propose(
            &random_addr,
            &AdminOperation::UpdateGovDelay(60u32),
            &salt,
        )
    })
    .unwrap();

    // The builder already set the controller, so without the auth check this call
    // fails with `PoolAlreadyDeployed`, a contract error.
    expect_rejected("gov.deploy_controller", || {
        gov.set_auths(&no_auths).try_deploy_controller(&real_wasm)
    })
    .unwrap();
    expect_rejected("gov.pause", || {
        gov.set_auths(&no_auths).try_pause(&random_addr)
    })
    .unwrap();
}
