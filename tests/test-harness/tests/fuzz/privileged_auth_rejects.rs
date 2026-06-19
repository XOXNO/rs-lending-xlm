use crate::config::config;
use controller::types::InterestRateModel;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Symbol, Vec as SVec};
use test_harness::LendingTest;

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

fn sample_asset_config(env: &soroban_sdk::Env) -> controller::types::AssetConfigRaw {
    controller::types::AssetConfigRaw {
        loan_to_value_bps: 7500,
        liquidation_threshold_bps: 8000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 100,
        is_collateralizable: true,
        is_borrowable: true,

        is_flashloanable: true,
        flashloan_fee_bps: 9,
        e_mode_categories: soroban_sdk::Vec::new(env),
        borrow_cap: i128::MAX,
        supply_cap: i128::MAX,
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
        first_upper_ratio_bps: 10_100,
        first_lower_ratio_bps: 9_901,
        last_upper_ratio_bps: 10_200,
        last_lower_ratio_bps: 9_804,
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
        let cfg = sample_asset_config(&env);
        let oracle_cfg = sample_oracle_cfg(&t);
        let limits = sample_position_limits();
        let usdc = t.resolve_asset("USDC");
        let random_addr = Address::generate(&env);
        let role_kp = Symbol::new(&env, "KEEPER");
        let role_rev = Symbol::new(&env, "REVENUE");
        let role_oracle = Symbol::new(&env, "ORACLE");

        expect_rejected("pause", || ctrl.set_auths(&no_auths).try_pause()).unwrap();
        expect_rejected("unpause", || ctrl.set_auths(&no_auths).try_unpause()).unwrap();
        expect_rejected("grant_role(KEEPER)", || {
            ctrl.set_auths(&no_auths).try_grant_role(&random_addr, &role_kp)
        }).unwrap();
        expect_rejected("grant_role(REVENUE)", || {
            ctrl.set_auths(&no_auths).try_grant_role(&random_addr, &role_rev)
        }).unwrap();
        expect_rejected("grant_role(ORACLE)", || {
            ctrl.set_auths(&no_auths).try_grant_role(&random_addr, &role_oracle)
        }).unwrap();
        expect_rejected("revoke_role", || {
            ctrl.set_auths(&no_auths).try_revoke_role(&random_addr, &role_kp)
        }).unwrap();
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
        expect_rejected("edit_asset_config", || {
            ctrl.set_auths(&no_auths).try_edit_asset_config(&usdc, &cfg)
        }).unwrap();
        expect_rejected("set_position_limits", || {
            ctrl.set_auths(&no_auths).try_set_position_limits(&limits)
        }).unwrap();
        let _ = (max_supply, max_borrow);

        expect_rejected("add_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_add_e_mode_category()
        }).unwrap();
        expect_rejected("remove_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_remove_e_mode_category(&category_id)
        }).unwrap();
        expect_rejected("add_asset_to_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_add_asset_to_e_mode_category(
                &usdc, &category_id, &can_collateral, &can_borrow, &ltv, &threshold, &bonus,
            )
        }).unwrap();
        expect_rejected("edit_asset_in_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_edit_asset_in_e_mode_category(
                &usdc, &category_id, &can_collateral, &can_borrow, &ltv, &threshold, &bonus,
            )
        }).unwrap();
        expect_rejected("remove_asset_from_e_mode", || {
            ctrl.set_auths(&no_auths).try_remove_asset_from_e_mode(&usdc, &category_id)
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
            max_borrow_rate_ray: 0,
            base_borrow_rate_ray: 0,
            slope1_ray: 0,
            slope2_ray: 0,
            slope3_ray: 0,
            mid_utilization_ray: 0,
            optimal_utilization_ray: 0,
            max_utilization_ray: controller::constants::RAY * 95 / 100,
            reserve_factor_bps: 0,
        };
        expect_rejected("upgrade_liquidity_pool_params", || {
            ctrl.set_auths(&no_auths).try_upgrade_liquidity_pool_params(&usdc, &zero_model)
        }).unwrap();
        expect_rejected("create_liquidity_pool", || {
            let params = controller::types::MarketParamsRaw {
                max_borrow_rate_ray: 0,
                base_borrow_rate_ray: 0,
                slope1_ray: 0,
                slope2_ray: 0,
                slope3_ray: 0,
                mid_utilization_ray: 0,
                optimal_utilization_ray: 0,
                max_utilization_ray: controller::constants::RAY * 95 / 100,
                reserve_factor_bps: 0,
                asset_id: usdc.clone(),
                asset_decimals: 7,
            };
            ctrl.set_auths(&no_auths).try_create_liquidity_pool(&usdc, &params, &cfg)
        }).unwrap();

        let empty_assets: SVec<Address> = SVec::new(&env);
        let empty_ids: SVec<u64> = SVec::new(&env);

        expect_rejected("update_indexes (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_update_indexes(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("clean_bad_debt (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_clean_bad_debt(&random_addr, &0u64)
        }).unwrap();
        expect_rejected("update_account_threshold (KEEPER)", || {
            ctrl.set_auths(&no_auths)
                .try_update_account_threshold(&random_addr, &false, &empty_ids)
        }).unwrap();
        expect_rejected("claim_revenue (REVENUE)", || {
            ctrl.set_auths(&no_auths).try_claim_revenue(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("add_rewards (REVENUE)", || {
            let rewards: SVec<(Address, i128)> = SVec::new(&env);
            ctrl.set_auths(&no_auths).try_add_rewards(&random_addr, &rewards)
        }).unwrap();
        expect_rejected("set_market_oracle_config", || {
            ctrl.set_auths(&no_auths).try_set_market_oracle_config(&usdc, &oracle_cfg)
        }).unwrap();
        expect_rejected("set_oracle_tolerance", || {
            let tolerance = sample_tolerance();
            ctrl.set_auths(&no_auths).try_set_oracle_tolerance(&usdc, &tolerance)
        }).unwrap();
        expect_rejected("disable_token_oracle (ORACLE)", || {
            ctrl.set_auths(&no_auths).try_disable_token_oracle(&random_addr, &usdc)
        }).unwrap();
    }

    // Governance timelock proposers + immediate emergency/meta entrypoints: every
    // privileged surface must reject when no auth is presented, before any
    // validation or scheduling. Protocol and governance-self admin both route
    // through `propose_*` (PROPOSER auth); `pause`/`unpause` stay owner-immediate.
    #[test]
    fn prop_governance_endpoints_reject_unauthed(
        ltv in 0u32..10_000,
        threshold in 0u32..10_000,
        bonus in 0u32..2_000,
        category_id in 1u32..100,
        can_collateral in any::<bool>(),
        can_borrow in any::<bool>(),
        seed in any::<u8>(),
    ) {
        let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        let env = t.env.clone();
        let gov = t.gov_client();
        let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];
        let cfg = sample_asset_config(&env);
        let limits = sample_position_limits();
        let usdc = t.resolve_asset("USDC");
        let random_addr = Address::generate(&env);
        let role_kp = Symbol::new(&env, "KEEPER");
        let role_executor = Symbol::new(&env, "EXECUTOR");
        let salt = dummy_bytes_n(&env, seed);

        // Timelock proposers: PROPOSER auth must gate every controller-targeted
        // schedule. Address / hash setters.
        expect_rejected("gov.propose_set_aggregator", || {
            gov.set_auths(&no_auths).try_propose_set_aggregator(&random_addr, &random_addr, &salt)
        }).unwrap();
        expect_rejected("gov.propose_set_accumulator", || {
            gov.set_auths(&no_auths).try_propose_set_accumulator(&random_addr, &random_addr, &salt)
        }).unwrap();
        expect_rejected("gov.propose_set_pool_template", || {
            gov.set_auths(&no_auths)
                .try_propose_set_pool_template(&random_addr, &dummy_bytes_n(&env, seed), &salt)
        }).unwrap();

        // Market / asset configuration.
        expect_rejected("gov.propose_edit_asset_config", || {
            gov.set_auths(&no_auths).try_propose_edit_asset_config(&random_addr, &usdc, &cfg, &salt)
        }).unwrap();
        expect_rejected("gov.propose_set_position_limits", || {
            gov.set_auths(&no_auths).try_propose_set_position_limits(&random_addr, &limits, &salt)
        }).unwrap();
        expect_rejected("gov.propose_approve_token", || {
            gov.set_auths(&no_auths).try_propose_approve_token(&random_addr, &usdc, &salt)
        }).unwrap();
        expect_rejected("gov.propose_revoke_token", || {
            gov.set_auths(&no_auths).try_propose_revoke_token(&random_addr, &usdc, &salt)
        }).unwrap();
        expect_rejected("gov.propose_create_liquidity_pool", || {
            let params = controller::types::MarketParamsRaw {
                max_borrow_rate_ray: 0,
                base_borrow_rate_ray: 0,
                slope1_ray: 0,
                slope2_ray: 0,
                slope3_ray: 0,
                mid_utilization_ray: 0,
                optimal_utilization_ray: 0,
                max_utilization_ray: controller::constants::RAY * 95 / 100,
                reserve_factor_bps: 0,
                asset_id: usdc.clone(),
                asset_decimals: 7,
            };
            gov.set_auths(&no_auths)
                .try_propose_create_liquidity_pool(&random_addr, &usdc, &params, &cfg, &salt)
        }).unwrap();
        expect_rejected("gov.propose_upgrade_pool_params", || {
            let zero_model = InterestRateModel {
                max_borrow_rate_ray: 0,
                base_borrow_rate_ray: 0,
                slope1_ray: 0,
                slope2_ray: 0,
                slope3_ray: 0,
                mid_utilization_ray: 0,
                optimal_utilization_ray: 0,
                max_utilization_ray: controller::constants::RAY * 95 / 100,
                reserve_factor_bps: 0,
            };
            gov.set_auths(&no_auths)
                .try_propose_upgrade_pool_params(&random_addr, &usdc, &zero_model, &salt)
        }).unwrap();

        // E-mode management.
        expect_rejected("gov.propose_add_e_mode_category", || {
            gov.set_auths(&no_auths)
                .try_propose_add_e_mode_category(&random_addr, &salt)
        }).unwrap();
        expect_rejected("gov.propose_remove_e_mode_category", || {
            gov.set_auths(&no_auths)
                .try_propose_remove_e_mode_category(&random_addr, &category_id, &salt)
        }).unwrap();
        expect_rejected("gov.propose_add_asset_to_e_mode", || {
            gov.set_auths(&no_auths).try_propose_add_asset_to_e_mode(
                &random_addr, &usdc, &category_id, &can_collateral, &can_borrow,
                &ltv, &threshold, &bonus, &salt,
            )
        }).unwrap();
        expect_rejected("gov.propose_edit_asset_in_e_mode", || {
            gov.set_auths(&no_auths).try_propose_edit_asset_in_e_mode(
                &random_addr, &usdc, &category_id, &can_collateral, &can_borrow,
                &ltv, &threshold, &bonus, &salt,
            )
        }).unwrap();
        expect_rejected("gov.propose_remove_asset_from_e_mode", || {
            gov.set_auths(&no_auths)
                .try_propose_remove_asset_from_e_mode(&random_addr, &usdc, &category_id, &salt)
        }).unwrap();

        // Deployment / upgrade / lifecycle. `deploy_controller` stays
        // owner-immediate; governance-self and controller upgrades are timelocked.
        expect_rejected("gov.deploy_controller", || {
            gov.set_auths(&no_auths).try_deploy_controller(&dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("gov.propose_deploy_pool", || {
            gov.set_auths(&no_auths).try_propose_deploy_pool(&random_addr, &salt)
        }).unwrap();
        expect_rejected("gov.propose_upgrade_pool", || {
            gov.set_auths(&no_auths)
                .try_propose_upgrade_pool(&random_addr, &dummy_bytes_n(&env, seed), &salt)
        }).unwrap();
        expect_rejected("gov.propose_upgrade_controller", || {
            gov.set_auths(&no_auths)
                .try_propose_upgrade_controller(&random_addr, &dummy_bytes_n(&env, seed), &salt)
        }).unwrap();
        expect_rejected("gov.propose_migrate_controller", || {
            gov.set_auths(&no_auths).try_propose_migrate_controller(&random_addr, &2u32, &salt)
        }).unwrap();
        expect_rejected("gov.propose_governance_upgrade", || {
            gov.set_auths(&no_auths)
                .try_propose_governance_upgrade(&random_addr, &dummy_bytes_n(&env, seed), &salt)
        }).unwrap();
        expect_rejected("gov.propose_update_delay", || {
            gov.set_auths(&no_auths).try_propose_update_delay(&random_addr, &60u32, &salt)
        }).unwrap();
        expect_rejected("gov.pause", || gov.set_auths(&no_auths).try_pause()).unwrap();
        expect_rejected("gov.unpause", || gov.set_auths(&no_auths).try_unpause()).unwrap();

        // Role and ownership management.
        expect_rejected("gov.propose_grant_controller_role", || {
            gov.set_auths(&no_auths)
                .try_propose_grant_controller_role(&random_addr, &random_addr, &role_kp, &salt)
        }).unwrap();
        expect_rejected("gov.propose_revoke_controller_role", || {
            gov.set_auths(&no_auths)
                .try_propose_revoke_controller_role(&random_addr, &random_addr, &role_kp, &salt)
        }).unwrap();
        expect_rejected("gov.propose_transfer_ctrl_ownership", || {
            gov.set_auths(&no_auths)
                .try_propose_transfer_ctrl_ownership(&random_addr, &random_addr, &1_000_000u32, &salt)
        }).unwrap();
        expect_rejected("gov.propose_grant_governance_role", || {
            gov.set_auths(&no_auths)
                .try_propose_grant_governance_role(&random_addr, &random_addr, &role_executor, &salt)
        }).unwrap();
        expect_rejected("gov.propose_revoke_governance_role", || {
            gov.set_auths(&no_auths)
                .try_propose_revoke_governance_role(&random_addr, &random_addr, &role_executor, &salt)
        }).unwrap();
        expect_rejected("gov.propose_transfer_gov_own", || {
            gov.set_auths(&no_auths)
                .try_propose_transfer_gov_own(&random_addr, &random_addr, &1_000_000u32, &salt)
        }).unwrap();

        // Oracle proposers: PROPOSER auth gates the oracle schedules too.
        expect_rejected("gov.propose_configure_market_oracle", || {
            let input = test_harness::reflector_single_spot_config(&t.mock_reflector, &usdc, 100, 200);
            gov.set_auths(&no_auths)
                .try_propose_configure_market_oracle(&random_addr, &usdc, &input, &salt)
        }).unwrap();
        expect_rejected("gov.propose_edit_oracle_tolerance", || {
            gov.set_auths(&no_auths)
                .try_propose_edit_oracle_tolerance(&random_addr, &usdc, &100u32, &200u32, &salt)
        }).unwrap();
    }

    #[test]
    fn prop_wrong_role_rejected(case_idx in 0u8..6) {
        use soroban_sdk::testutils::MockAuth;
        use soroban_sdk::testutils::MockAuthInvoke;
        use soroban_sdk::IntoVal;

        let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        let env = t.env.clone();
        let ctrl = t.ctrl_client();
        let usdc = t.resolve_asset("USDC");
        let empty_assets: SVec<Address> = SVec::new(&env);

        let (granted, target, endpoint) = match case_idx {
            0 => ("KEEPER", "REVENUE", "claim_revenue"),
            1 => ("KEEPER", "ORACLE", "disable_token_oracle"),
            2 => ("REVENUE", "ORACLE", "disable_token_oracle"),
            3 => ("REVENUE", "KEEPER", "update_indexes"),
            4 => ("ORACLE", "KEEPER", "update_indexes"),
            5 => ("ORACLE", "REVENUE", "claim_revenue"),
            _ => unreachable!(),
        };

        let caller = Address::generate(&env);
        ctrl.grant_role(&caller, &Symbol::new(&env, granted));

        let args: soroban_sdk::Vec<soroban_sdk::Val> = match endpoint {
            "claim_revenue" => (caller.clone(), empty_assets.clone()).into_val(&env),
            "disable_token_oracle" => (caller.clone(), usdc.clone()).into_val(&env),
            "update_indexes" => (caller.clone(), empty_assets.clone()).into_val(&env),
            _ => unreachable!(),
        };
        let invoke = MockAuthInvoke {
            contract: &t.controller,
            fn_name: endpoint,
            args,
            sub_invokes: &[],
        };
        let auths = [MockAuth {
            address: &caller,
            invoke: &invoke,
        }];

        let res = match endpoint {
            "claim_revenue" => ctrl
                .mock_auths(&auths)
                .try_claim_revenue(&caller, &empty_assets)
                .map(|inner| inner.map(|_| ()))
                .map_err(|e| std::format!("{:?}", e)),
            "disable_token_oracle" => ctrl
                .mock_auths(&auths)
                .try_disable_token_oracle(&caller, &usdc)
                .map(|inner| inner.map(|_| ()))
                .map_err(|e| std::format!("{:?}", e)),
            "update_indexes" => ctrl
                .mock_auths(&auths)
                .try_update_indexes(&caller, &empty_assets)
                .map(|inner| inner.map(|_| ()))
                .map_err(|e| std::format!("{:?}", e)),
            _ => unreachable!(),
        };
        prop_assert!(
            !matches!(res, Ok(Ok(_))),
            "CRITICAL: {} role called {} endpoint {}",
            granted, target, endpoint
        );
    }
}
