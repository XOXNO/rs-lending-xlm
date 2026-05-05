//! Contract-level property test: admin / role auth matrix.
//!
//! Enumerates privileged controller endpoints and verifies that calling them
//! **without** the required auth
//! (via `ControllerClient::set_auths(&[])` -- no signed auth in the env)
//! fails at the host layer.
//!
//! Semantics:
//!   * `LendingTest::build()` calls `env.mock_all_auths()` so normal tests
//!     succeed. `set_auths(&[])` bypasses that mock **per-call** and demands
//!     a real signature -- none exists, so the host aborts.
//!   * `try_<method>` on the generated client returns
//!     `Result<Result<Ret, ContractErr>, Result<Err, InvokeError>>`.
//!     An **outer Err** signals host-level rejection (auth failure / panic).
//!     An **outer Ok** (inner Ok or Err) means the endpoint body ran past
//!     the auth gate, indicating a missing authorization gate.
//!
//! Each endpoint runs against 64 random inputs (role string / asset / amount),
//! paired with a random caller address that holds no role and does not own
//! the contract.

use common::types::InterestRateModel;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Symbol, Vec as SVec};
use test_harness::{eth_preset, usdc_preset, wbtc_preset, LendingTest};

/// Run `call` with auth mocking disabled and assert host-layer rejection
/// (outer `Err`). Returns `Ok(())` on correct rejection, `Err(msg)` if the
/// auth gate is missing.
fn expect_rejected<F, R, InnerErr, OuterErr>(label: &str, call: F) -> Result<(), String>
where
    F: FnOnce() -> Result<Result<R, InnerErr>, OuterErr>,
    InnerErr: core::fmt::Debug,
{
    match call() {
        Err(_) => Ok(()), // correct: host-level auth failure
        Ok(Ok(_)) => Err(format!(
            "CRITICAL: {} executed successfully without auth -- endpoint is NOT gated",
            label
        )),
        Ok(Err(contract_err)) => Err(format!(
            "CRITICAL: {} executed past auth gate and returned contract error {:?} \
             (auth should have rejected first)",
            label, contract_err
        )),
    }
}

fn build_ctx() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build()
}

// Blanket AssetConfig + MarketOracleConfigInput builders with plausible shapes.
// Values need not be valid -- the auth gate must reject before the body runs.
fn sample_asset_config(env: &soroban_sdk::Env) -> common::types::AssetConfig {
    common::types::AssetConfig {
        loan_to_value_bps: 7500,
        liquidation_threshold_bps: 8000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 100,
        is_collateralizable: true,
        is_borrowable: true,
        e_mode_categories: soroban_sdk::Vec::new(env),
        is_isolated_asset: false,
        is_siloed_borrowing: false,
        is_flashloanable: true,
        isolation_borrow_enabled: false,
        isolation_debt_ceiling_usd_wad: 0,
        flashloan_fee_bps: 9,
        borrow_cap: i128::MAX,
        supply_cap: i128::MAX,
    }
}

fn sample_oracle_cfg(t: &LendingTest) -> common::types::MarketOracleConfigInput {
    common::types::MarketOracleConfigInput {
        exchange_source: common::types::ExchangeSource::SpotOnly,
        max_price_stale_seconds: 900,
        first_tolerance_bps: 100,
        last_tolerance_bps: 200,
        cex_oracle: t.mock_reflector.clone(),
        cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(&t.env, ""),
        dex_oracle: None,
        dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        dex_symbol: Symbol::new(&t.env, ""),
        twap_records: 3,
    }
}

fn sample_position_limits() -> common::types::PositionLimits {
    common::types::PositionLimits {
        max_supply_positions: 5,
        max_borrow_positions: 5,
    }
}

fn dummy_bytes_n(env: &soroban_sdk::Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

// ---------------------------------------------------------------------------
// Matrix of privileged endpoints, each returning a Result<(), String>.
// "CRITICAL: ..." means the auth gate is absent / weak.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    // ---- only_owner endpoints -------------------------------------------

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
        let t = build_ctx();
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

        // pause / unpause (only_owner)
        expect_rejected("pause", || ctrl.set_auths(&no_auths).try_pause())
            .unwrap();
        expect_rejected("unpause", || ctrl.set_auths(&no_auths).try_unpause())
            .unwrap();

        // grant_role / revoke_role (only_owner)
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

        // transfer_ownership (only_owner)
        expect_rejected("transfer_ownership", || {
            ctrl.set_auths(&no_auths).try_transfer_ownership(&random_addr, &1_000_000u32)
        }).unwrap();

        // set_aggregator / set_accumulator / set_liquidity_pool_template
        expect_rejected("set_aggregator", || {
            ctrl.set_auths(&no_auths).try_set_aggregator(&random_addr)
        }).unwrap();
        expect_rejected("set_accumulator", || {
            ctrl.set_auths(&no_auths).try_set_accumulator(&random_addr)
        }).unwrap();
        expect_rejected("set_liquidity_pool_template", || {
            ctrl.set_auths(&no_auths).try_set_liquidity_pool_template(&dummy_bytes_n(&env, seed))
        }).unwrap();

        // edit_asset_config (only_owner)
        expect_rejected("edit_asset_config", || {
            ctrl.set_auths(&no_auths).try_edit_asset_config(&usdc, &cfg)
        }).unwrap();

        // set_position_limits (only_owner)
        expect_rejected("set_position_limits", || {
            ctrl.set_auths(&no_auths).try_set_position_limits(&limits)
        }).unwrap();
        let _ = (max_supply, max_borrow);

        // E-mode category management: all endpoints require owner auth.
        expect_rejected("add_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_add_e_mode_category(&ltv, &threshold, &bonus)
        }).unwrap();
        // edit_e_mode_category must be owner-gated.
        expect_rejected("edit_e_mode_category (C-01)", || {
            ctrl.set_auths(&no_auths)
                .try_edit_e_mode_category(&category_id, &ltv, &threshold, &bonus)
        }).unwrap();
        expect_rejected("remove_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_remove_e_mode_category(&category_id)
        }).unwrap();
        expect_rejected("add_asset_to_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_add_asset_to_e_mode_category(
                &usdc, &category_id, &can_collateral, &can_borrow,
            )
        }).unwrap();
        expect_rejected("edit_asset_in_e_mode_category", || {
            ctrl.set_auths(&no_auths).try_edit_asset_in_e_mode_category(
                &usdc, &category_id, &can_collateral, &can_borrow,
            )
        }).unwrap();
        expect_rejected("remove_asset_from_e_mode", || {
            ctrl.set_auths(&no_auths).try_remove_asset_from_e_mode(&usdc, &category_id)
        }).unwrap();
        expect_rejected("remove_asset_e_mode_category", || {
            ctrl.set_auths(&no_auths)
                .try_remove_asset_e_mode_category(&usdc, &category_id)
        }).unwrap();

        // approve_token_wasm / revoke_token_wasm (only_owner)
        expect_rejected("approve_token_wasm", || {
            ctrl.set_auths(&no_auths).try_approve_token_wasm(&usdc)
        }).unwrap();
        expect_rejected("revoke_token_wasm", || {
            ctrl.set_auths(&no_auths).try_revoke_token_wasm(&usdc)
        }).unwrap();

        // upgrade / upgrade_pool / upgrade_pool_params (only_owner)
        expect_rejected("upgrade", || {
            ctrl.set_auths(&no_auths).try_upgrade(&dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("upgrade_pool", || {
            ctrl.set_auths(&no_auths).try_upgrade_pool(&usdc, &dummy_bytes_n(&env, seed))
        }).unwrap();
        expect_rejected("upgrade_liquidity_pool", || {
            ctrl.set_auths(&no_auths).try_upgrade_liquidity_pool(
                &usdc,
                &dummy_bytes_n(&env, seed),
            )
        }).unwrap();
        let zero_model = InterestRateModel {
            max_borrow_rate_ray: 0,
            base_borrow_rate_ray: 0,
            slope1_ray: 0,
            slope2_ray: 0,
            slope3_ray: 0,
            mid_utilization_ray: 0,
            optimal_utilization_ray: 0,
            reserve_factor_bps: 0,
        };
        expect_rejected("upgrade_pool_params", || {
            ctrl.set_auths(&no_auths).try_upgrade_pool_params(&usdc, &zero_model)
        }).unwrap();
        expect_rejected("upgrade_liquidity_pool_params", || {
            ctrl.set_auths(&no_auths).try_upgrade_liquidity_pool_params(&usdc, &zero_model)
        }).unwrap();

        // create_liquidity_pool (only_owner)
        expect_rejected("create_liquidity_pool", || {
            let params = common::types::MarketParams {
                max_borrow_rate_ray: 0, base_borrow_rate_ray: 0,
                slope1_ray: 0, slope2_ray: 0, slope3_ray: 0,
                mid_utilization_ray: 0, optimal_utilization_ray: 0,
                reserve_factor_bps: 0,
                asset_id: usdc.clone(), asset_decimals: 7,
            };
            ctrl.set_auths(&no_auths).try_create_liquidity_pool(&usdc, &params, &cfg)
        }).unwrap();

        // ---- only_role endpoints ---------------------------------------

        // Empty Vec<Address> is fine -- auth is checked before body runs.
        let empty_assets: SVec<Address> = SVec::new(&env);
        let empty_ids: SVec<u64> = SVec::new(&env);

        // KEEPER role
        expect_rejected("update_indexes (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_update_indexes(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("keepalive_shared_state (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_keepalive_shared_state(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("keepalive_accounts (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_keepalive_accounts(&random_addr, &empty_ids)
        }).unwrap();
        expect_rejected("keepalive_pools (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_keepalive_pools(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("clean_bad_debt (KEEPER)", || {
            ctrl.set_auths(&no_auths).try_clean_bad_debt(&random_addr, &0u64)
        }).unwrap();
        expect_rejected("update_account_threshold (KEEPER)", || {
            ctrl.set_auths(&no_auths)
                .try_update_account_threshold(&random_addr, &usdc, &false, &empty_ids)
        }).unwrap();

        // REVENUE role
        expect_rejected("claim_revenue (REVENUE)", || {
            ctrl.set_auths(&no_auths).try_claim_revenue(&random_addr, &empty_assets)
        }).unwrap();
        expect_rejected("add_rewards (REVENUE)", || {
            let rewards: SVec<(Address, i128)> = SVec::new(&env);
            ctrl.set_auths(&no_auths).try_add_rewards(&random_addr, &rewards)
        }).unwrap();

        // ORACLE role
        expect_rejected("configure_market_oracle (ORACLE)", || {
            ctrl.set_auths(&no_auths)
                .try_configure_market_oracle(&random_addr, &usdc, &oracle_cfg)
        }).unwrap();
        expect_rejected("edit_oracle_tolerance (ORACLE)", || {
            ctrl.set_auths(&no_auths)
                .try_edit_oracle_tolerance(&random_addr, &usdc, &100u32, &200u32)
        }).unwrap();
        expect_rejected("disable_token_oracle (ORACLE)", || {
            ctrl.set_auths(&no_auths).try_disable_token_oracle(&random_addr, &usdc)
        }).unwrap();
    }

    // ---- Cross-role rejection: KEEPER role cannot call ORACLE endpoints ----
    //
    // Grant the random caller the WRONG role, mock auths for THAT address
    // only, then call endpoints gated on a different role. Host auth accepts
    // the caller's `require_auth`, but the contract-level role check
    // (only_role) must reject.

    #[test]
    fn prop_wrong_role_rejected(
        // Sweep the 6 (granted_role, target_role) pairs where granted != target,
        // so each case grants ONE role and probes an endpoint gated on a
        // different role:
        //   0 = KEEPER  -> REVENUE  (claim_revenue)
        //   1 = KEEPER  -> ORACLE   (disable_token_oracle)
        //   2 = REVENUE -> ORACLE   (disable_token_oracle)
        //   3 = REVENUE -> KEEPER   (update_indexes)
        //   4 = ORACLE  -> KEEPER   (update_indexes)
        //   5 = ORACLE  -> REVENUE  (claim_revenue)
        case_idx in 0u8..6,
    ) {
        use soroban_sdk::testutils::MockAuth;
        use soroban_sdk::testutils::MockAuthInvoke;
        use soroban_sdk::IntoVal;

        let t = build_ctx();
        let env = t.env.clone();
        let ctrl = t.ctrl_client();
        let usdc = t.resolve_asset("USDC");
        let empty_assets: SVec<Address> = SVec::new(&env);

        // (granted_role, target_role, target_endpoint).
        let (granted, target, endpoint) = match case_idx {
            0 => ("KEEPER", "REVENUE", "claim_revenue"),
            1 => ("KEEPER", "ORACLE", "disable_token_oracle"),
            2 => ("REVENUE", "ORACLE", "disable_token_oracle"),
            3 => ("REVENUE", "KEEPER", "update_indexes"),
            4 => ("ORACLE", "KEEPER", "update_indexes"),
            5 => ("ORACLE", "REVENUE", "claim_revenue"),
            _ => unreachable!(),
        };

        // Grant the caller exactly one role; admin holds all three from the
        // constructor, so `grant_role` under `mock_all_auths` succeeds.
        let caller = Address::generate(&env);
        ctrl.grant_role(&caller, &Symbol::new(&env, granted));

        // only_role macros run `require_auth(caller)` first, then check the
        // role. Provide a valid MockAuth for `caller` so role check is what
        // rejects (not the host auth gate).
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
            "CRITICAL: {}-only address succeeded against {} endpoint {}",
            granted, target, endpoint
        );
    }
}
