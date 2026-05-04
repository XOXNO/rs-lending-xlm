extern crate std;

use super::*;
use common::errors::{EModeError, GenericError};
use common::types::{
    Account, AssetConfig, ControllerKey, EModeAssetConfig, EModeCategory, ExchangeSource,
    MarketConfig, MarketStatus, OraclePriceFluctuation, OracleProviderConfig, OracleType,
    PositionLimits, PositionMode, ReflectorAssetKind,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env, Map, Symbol};

struct TestSetup {
    env: Env,
    contract: Address,
    asset: Address,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract = env.register(crate::Controller, (admin.clone(),));
        let asset = Address::generate(&env);

        Self {
            env,
            contract,
            asset,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }

    fn sample_asset_config(&self) -> AssetConfig {
        AssetConfig {
            loan_to_value_bps: 7_500,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            e_mode_categories: soroban_sdk::Vec::from_array(&self.env, [1u32]),
            is_isolated_asset: false,
            is_siloed_borrowing: false,
            is_flashloanable: true,
            isolation_borrow_enabled: true,
            isolation_debt_ceiling_usd_wad: 1_000_000,
            flashloan_fee_bps: 9,
            borrow_cap: 2_000_000,
            supply_cap: 3_000_000,
        }
    }

    fn sample_oracle_config(&self) -> OracleProviderConfig {
        OracleProviderConfig {
            base_asset: self.asset.clone(),
            oracle_type: OracleType::Normal,
            exchange_source: ExchangeSource::SpotVsTwap,
            asset_decimals: 7,
            tolerance: OraclePriceFluctuation {
                first_upper_ratio_bps: 10_200,
                first_lower_ratio_bps: 9_800,
                last_upper_ratio_bps: 11_000,
                last_lower_ratio_bps: 9_000,
            },
            max_price_stale_seconds: 900,
        }
    }

    fn sample_market_config(&self) -> MarketConfig {
        MarketConfig {
            status: MarketStatus::Active,
            asset_config: self.sample_asset_config(),
            pool_address: Address::generate(&self.env),
            oracle_config: self.sample_oracle_config(),
            cex_oracle: Some(Address::generate(&self.env)),
            cex_asset_kind: ReflectorAssetKind::Other,
            cex_symbol: Symbol::new(&self.env, "XLM"),
            cex_decimals: 14,
            dex_oracle: Some(Address::generate(&self.env)),
            dex_asset_kind: ReflectorAssetKind::Stellar,
            dex_symbol: Symbol::new(&self.env, ""),
            dex_decimals: 14,
            twap_records: 3,
        }
    }

    fn sample_account(&self) -> Account {
        Account {
            owner: Address::generate(&self.env),
            is_isolated: false,
            e_mode_category_id: 1,
            mode: PositionMode::Normal,
            isolated_asset: None,
            supply_positions: Map::new(&self.env),
            borrow_positions: Map::new(&self.env),
        }
    }
}

#[test]
fn test_instance_storage_round_trip_and_counters() {
    let t = TestSetup::new();

    t.as_contract(|| {
        let template = BytesN::from_array(&t.env, &[7; 32]);
        let aggregator = Address::generate(&t.env);
        let accumulator = Address::generate(&t.env);
        let limits = PositionLimits {
            max_supply_positions: 6,
            max_borrow_positions: 3,
        };

        set_pool_template(&t.env, &template);
        set_aggregator(&t.env, &aggregator);
        set_accumulator(&t.env, &accumulator);
        set_position_limits(&t.env, &limits);
        set_flash_loan_ongoing(&t.env, true);
        bump_instance(&t.env);

        assert_eq!(get_pool_template(&t.env), template);
        assert_eq!(get_aggregator(&t.env), aggregator);
        assert_eq!(get_accumulator(&t.env), accumulator);
        assert_eq!(get_position_limits(&t.env).max_supply_positions, 6);
        assert!(is_flash_loan_ongoing(&t.env));
        assert!(has_accumulator(&t.env));
        assert_eq!(get_account_nonce(&t.env), 0);
        assert_eq!(increment_account_nonce(&t.env), 1);
        assert_eq!(increment_account_nonce(&t.env), 2);
        assert_eq!(get_last_emode_category_id(&t.env), 0);
        assert_eq!(increment_emode_category_id(&t.env), 1);
        assert_eq!(increment_emode_category_id(&t.env), 2);
    });
}

#[test]
fn test_market_account_and_emode_round_trips() {
    let t = TestSetup::new();

    t.as_contract(|| {
        let market = t.sample_market_config();
        let mut account = t.sample_account();
        let emode = EModeCategory {
            loan_to_value_bps: 8_500,
            liquidation_threshold_bps: 9_000,
            liquidation_bonus_bps: 200,
            is_deprecated: false,
            assets: soroban_sdk::Map::new(&t.env),
        };
        let emode_asset = EModeAssetConfig {
            is_collateralizable: true,
            is_borrowable: false,
        };

        // Seed the market-level e-mode category membership before persisting.
        let mut market = market;
        market.asset_config.e_mode_categories = soroban_sdk::Vec::from_array(&t.env, [1u32, 2u32]);

        set_market_config(&t.env, &t.asset, &market);
        set_account(&t.env, 9, &account);
        set_emode_category(&t.env, 1, &emode);
        set_emode_asset(&t.env, 1, &t.asset, &emode_asset);
        set_isolated_debt(&t.env, &t.asset, 42);
        add_to_pools_list(&t.env, &t.asset, &market.pool_address);

        assert!(has_market_config(&t.env, &t.asset));
        assert_eq!(
            get_market_config(&t.env, &t.asset).pool_address,
            market.pool_address
        );
        assert_eq!(
            try_get_market_config(&t.env, &t.asset)
                .unwrap()
                .oracle_config
                .oracle_type,
            OracleType::Normal
        );

        assert_eq!(get_account(&t.env, 9).owner, account.owner);
        account.is_isolated = true;
        set_account(&t.env, 9, &account);
        assert!(try_get_account(&t.env, 9).unwrap().is_isolated);
        bump_account(&t.env, 9);
        remove_account_entry(&t.env, 9);
        assert!(try_get_account(&t.env, 9).is_none());

        assert_eq!(get_emode_category(&t.env, 1).loan_to_value_bps, 8_500);
        assert_eq!(
            try_get_emode_category(&t.env, 1)
                .unwrap()
                .liquidation_threshold_bps,
            9_000
        );
        assert!(!get_emode_asset(&t.env, 1, &t.asset).unwrap().is_borrowable);
        // Reverse-index lives on the market entry; no separate read.
        assert_eq!(
            get_market_config(&t.env, &t.asset)
                .asset_config
                .e_mode_categories
                .len(),
            2
        );
        remove_emode_asset(&t.env, 1, &t.asset);
        assert!(get_emode_asset(&t.env, 1, &t.asset).is_none());

        assert_eq!(get_isolated_debt(&t.env, &t.asset), 42);
        assert_eq!(get_pools_count(&t.env), 1);
        assert_eq!(get_pools_list_entry(&t.env, 0), t.asset);
        bump_pools_list(&t.env);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn test_get_pool_template_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_pool_template(&t.env);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #27)")]
fn test_get_aggregator_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_aggregator(&t.env);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #28)")]
fn test_get_accumulator_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_accumulator(&t.env);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #29)")]
fn test_get_position_limits_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        t.env
            .storage()
            .instance()
            .remove(&ControllerKey::PositionLimits);
        let _ = get_position_limits(&t.env);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_get_market_config_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_market_config(&t.env, &t.asset);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn test_get_account_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_account(&t.env, 404);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #300)")]
fn test_get_emode_category_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_emode_category(&t.env, 77);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")]
fn test_get_pools_list_entry_panics_when_missing() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let _ = get_pools_list_entry(&t.env, 0);
    });
}

#[test]
fn test_error_codes_match_expected_contract_ranges() {
    assert_eq!(GenericError::TemplateNotSet as u32, 26);
    assert_eq!(EModeError::EModeCategoryNotFound as u32, 300);
}
