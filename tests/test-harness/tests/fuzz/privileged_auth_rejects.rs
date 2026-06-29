use crate::config::config;
use controller::types::SpokeAssetArgs;
use controller::types::InterestRateModel;
use governance_interface::AdminOperation;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Vec as SVec};
use test_harness::{HARNESS_HUB, hub_asset, HubAssetKey, LendingTest};

fn expect_rejected<F, R, InnerErr, OuterErr>(label: &str, call: F) -> Result<(), String>
where
    F: FnOnce() -> Result<Result<R, InnerErr>, OuterErr>,
    InnerErr: core::fmt::Debug,
{
    match call() {
        Err(_) => Ok(()),
        Ok(Ok(_)) => Err(format!(
            "CRITICAL: {} executed successfully without auth",
            label
        )),
        Ok(Err(contract_err)) => Err(format!(
            "CRITICAL: {} passed auth gate with contract error {:?}",
            label, contract_err
        )),
    }
}

// Auth is rejected before any field is read, so a minimal resolved shape
// (mock-reflector constants, 100/200 BPS bands) suffices.
fn sample_oracle_cfg(t: &LendingTest) -> controller::types::MarketOracleConfig {
    let asset = t.resolve_market("USDC").asset.clone();
    controller::types::MarketOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: sample_tolerance(),
        strategy: controller::types::OracleStrategy::Single,
        primary: controller::types::OracleSourceConfig::Reflector(
            controller::types::ReflectorSourceConfig {
                contract: t.mock_reflector.clone(),
                asset: controller::types::OracleAssetRef::Stellar(asset),
                read_mode: controller::types::OracleReadMode::Spot,
                decimals: 14,
                resolution_seconds: 300,
                base: controller::types::ReflectorBase::Usd,
            },
        ),
        anchor: controller::types::OracleSourceConfigOption::None,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: controller::constants::MAX_REASONABLE_PRICE_WAD,
    }
}

fn sample_tolerance() -> controller::types::OraclePriceFluctuation {
    controller::types::OraclePriceFluctuation {
        upper_ratio_bps: 10_200,
        lower_ratio_bps: 9_804,
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

proptest! {
    #![proptest_config(config(64))]

    #[test]
    fn prop_owner_only_endpoints_reject_unauthed(
        ltv in 0u32..10_000,
        threshold in 0u32..10_000,
        bonus in 0u32..2_000,
        category_id in 1u32..100,
        can_collateral in any::<bool>(),
        can_borrow in any::<bool>(),
        seed in any::<u8>(),
        max_supply in 1u32..20,
        max_borrow in 1u32..20,
    ) {
        let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        let env = t.env.clone();
        let ctrl = t.ctrl_client();
        let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];
        let oracle_cfg = sample_oracle_cfg(&t);
        let limits = sample_position_limits();
        let usdc = t.resolve_asset("USDC");
        let random_addr = Address::generate(&env);

        expect_rejected("pause", || ctrl.set_auths(&no_auths).try_pause()).unwrap();
        expect_rejected("unpause", || ctrl.set_auths(&no_auths).try_unpause()).unwrap();
        expect_rejected("transfer_ownership", || {
            ctrl.set_auths(&no_auths).try_transfer_ownership(&random_addr, &1_000_000u32)
        }).unwrap();
        expect_rejected("set_aggregator", || {
            ctrl.set_auths(&no_auths).try_set_aggregator(&random_addr)
        }).unwrap();
        expect_rejected("set_accumulator", || {
            ctrl.set_auths(&no_auths).try_set_accumulator(&random_addr)
        }).unwrap();
        expect_rejected("set_liquidity_pool_template", || {
            ctrl.set_auths(&no_auths).try_set_liquidity_pool_template(&dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("set_position_limits", || {
            ctrl.set_auths(&no_auths).try_set_position_limits(&limits)
        }).unwrap();
        let _ = (max_supply, max_borrow);

        expect_rejected("add_spoke", || {
            ctrl.set_auths(&no_auths).try_add_spoke()
        }).unwrap();
        expect_rejected("remove_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_remove_spoke(&category_id)
        }).unwrap();
        expect_rejected("add_asset_to_spoke", || {
            ctrl.set_auths(&no_auths)
                .try_add_asset_to_spoke(&SpokeAssetArgs {
                    hub_id: HARNESS_HUB,
                    asset: usdc.clone(),
                    spoke_id: category_id,
                    can_collateral,
                    can_borrow,
                    ltv,
                    threshold,
                    bonus,
                    supply_cap: 0,
                    borrow_cap: 0,
                })
        }).unwrap();
        expect_rejected("edit_asset_in_spoke", || {
            ctrl.set_auths(&no_auths)
                .try_edit_asset_in_spoke(&SpokeAssetArgs {
                    hub_id: HARNESS_HUB,
                    asset: usdc.clone(),
                    spoke_id: category_id,
                    can_collateral,
                    can_borrow,
                    ltv,
                    threshold,
                    bonus,
                    supply_cap: 0,
                    borrow_cap: 0,
                })
        }).unwrap();
        expect_rejected("remove_asset_from_e_mode", || {
            ctrl.set_auths(&no_auths)
                .try_remove_asset_from_spoke(&hub_asset(usdc.clone()), &category_id)
        }).unwrap();
        expect_rejected("approve_token", || {
            ctrl.set_auths(&no_auths).try_approve_token(&usdc)
        }).unwrap();
        expect_rejected("revoke_token", || {
            ctrl.set_auths(&no_auths).try_revoke_token(&usdc)
        }).unwrap();
        expect_rejected("upgrade", || {
            ctrl.set_auths(&no_auths).try_upgrade(&dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("upgrade_pool", || {
            ctrl.set_auths(&no_auths).try_upgrade_pool(&dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("deploy_pool", || {
            ctrl.set_auths(&no_auths).try_deploy_pool()
        }).unwrap();
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
        };
        expect_rejected("upgrade_liquidity_pool_params", || {
            ctrl.set_auths(&no_auths).try_upgrade_liquidity_pool_params(&hub_asset(usdc.clone()), &zero_model)
        }).unwrap();
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
                supply_cap: 0,
                borrow_cap: 0,
                is_flashloanable: false,
                flashloan_fee: 0,
                asset_id: usdc.clone(),
                asset_decimals: 7,
            };
            // Auth is rejected before any params are read; any shape suffices.
            ctrl.set_auths(&no_auths).try_create_liquidity_pool(&HARNESS_HUB, &usdc, &params)
        }).unwrap();

        let empty_assets: SVec<HubAssetKey> = SVec::new(&env);
        let empty_ids: SVec<u64> = SVec::new(&env);

        expect_rejected("update_indexes (caller auth)", || {
            ctrl.set_auths(&no_auths).try_update_indexes(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("clean_bad_debt (caller auth)", || {
            ctrl.set_auths(&no_auths).try_clean_bad_debt(&random_addr, &0u64)
        }).unwrap();
        expect_rejected("update_account_threshold (caller auth)", || {
            ctrl.set_auths(&no_auths)
                .try_update_account_threshold(&random_addr, &false, &empty_ids)
        }).unwrap();
        expect_rejected("claim_revenue (caller auth)", || {
            ctrl.set_auths(&no_auths).try_claim_revenue(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("add_rewards (caller auth)", || {
            let rewards: SVec<(HubAssetKey, i128)> = SVec::new(&env);
            ctrl.set_auths(&no_auths).try_add_rewards(&random_addr, &rewards)
        }).unwrap();
        expect_rejected("set_market_oracle_config", || {
            ctrl.set_auths(&no_auths).try_set_market_oracle_config(&hub_asset(usdc.clone()), &oracle_cfg)
        }).unwrap();
        expect_rejected("set_oracle_tolerance", || {
            let tolerance = sample_tolerance();
            ctrl.set_auths(&no_auths).try_set_oracle_tolerance(&usdc, &tolerance)
        }).unwrap();
        expect_rejected("disable_token_oracle (owner)", || {
            ctrl.set_auths(&no_auths).try_disable_token_oracle(&usdc)
        }).unwrap();
    }

    // Governance timelock proposers + immediate emergency/meta entrypoints: every
    // privileged surface must reject when no auth is presented, before any
    // validation or scheduling. Protocol and governance-self admin both route
    // through `propose` (PROPOSER auth); `pause`/`unpause` stay owner-immediate.
    #[test]
    fn prop_governance_endpoints_reject_unauthed(
        seed in any::<u8>(),
    ) {
        let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        let env = t.env.clone();
        let gov = t.gov_client();
        let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];
        let limits = sample_position_limits();
        let usdc = t.resolve_asset("USDC");
        let random_addr = Address::generate(&env);
        let salt = dummy_bytes_n(&env, seed);

        // Test the unified `propose` endpoint with representative operations to prove PROPOSER role check is enforced.
        expect_rejected("gov.propose(SetPositionLimits)", || {
            gov.set_auths(&no_auths).try_propose(
                &random_addr,
                &AdminOperation::SetPositionLimits(limits),
                &salt,
            )
        }).unwrap();

        expect_rejected("gov.propose(UpdateGovDelay)", || {
            gov.set_auths(&no_auths).try_propose(
                &random_addr,
                &AdminOperation::UpdateGovDelay(60u32),
                &salt,
            )
        }).unwrap();

        expect_rejected("gov.propose(DisableTokenOracle)", || {
            gov.set_auths(&no_auths).try_propose(
                &random_addr,
                &AdminOperation::DisableTokenOracle(usdc),
                &salt,
            )
        }).unwrap();

        // Immediate admin actions
        expect_rejected("gov.deploy_controller", || {
            gov.set_auths(&no_auths).try_deploy_controller(&dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("gov.pause", || gov.set_auths(&no_auths).try_pause()).unwrap();
        expect_rejected("gov.unpause", || gov.set_auths(&no_auths).try_unpause()).unwrap();
    }

    #[test]
    fn prop_non_owner_cannot_disable_token_oracle(_case in 0u8..2) {
        let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        let ctrl = t.ctrl_client();
        let usdc = t.resolve_asset("USDC");
        let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];

        let res = ctrl
            .set_auths(&no_auths)
            .try_disable_token_oracle(&usdc)
            .map(|inner| inner.map(|_| ()))
            .map_err(|e| std::format!("{:?}", e));
        prop_assert!(
            !matches!(res, Ok(Ok(_))),
            "CRITICAL: non-owner must not disable_token_oracle without owner auth"
        );
    }
}
