use super::*;
use controller_interface::types::{
    AssetConfigRaw, MarketConfig, MarketOracleConfig, MarketOracleConfigOption, MarketStatus,
    OracleAssetRef, OraclePriceFluctuation, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, PositionMode, ReflectorBase, ReflectorSourceConfig,
    SpokeAssetConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, vec, BytesN, Vec};

#[contract]
struct TestContract;

fn setup() -> (Env, Address) {
    let env = Env::default();
    let contract = env.register(TestContract, ());
    (env, contract)
}

fn dummy_address(env: &Env) -> Address {
    Address::generate(env)
}

fn dummy_asset_config(env: &Env) -> AssetConfigRaw {
    AssetConfigRaw {
        loan_to_value_bps: 7500,
        liquidation_threshold_bps: 8000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 100,
        is_collateralizable: true,
        is_borrowable: true,
        e_mode_categories: soroban_sdk::Vec::new(env),
        is_flashloanable: true,
        flashloan_fee_bps: 9,
        asset_decimals: 7,
    }
}

fn dummy_market_config(env: &Env) -> MarketConfig {
    let asset = dummy_address(env);
    let oracle = dummy_address(env);
    MarketConfig {
        status: MarketStatus::Active,
        asset_config: dummy_asset_config(env),
        oracle_config: MarketOracleConfig {
            asset_decimals: 7,
            max_price_stale_seconds: 900,
            tolerance: OraclePriceFluctuation {
                upper_ratio_bps: 10_500,
                lower_ratio_bps: 9_500,
            },
            strategy: OracleStrategy::PrimaryWithAnchor,
            primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: oracle.clone(),
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode: OracleReadMode::Twap(12),
                decimals: 14,
                resolution_seconds: 300,
                base: ReflectorBase::Usd,
            }),
            anchor: OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
                ReflectorSourceConfig {
                    contract: oracle,
                    asset: OracleAssetRef::Stellar(asset),
                    read_mode: OracleReadMode::Spot,
                    decimals: 7,
                    resolution_seconds: 300,
                    base: ReflectorBase::Usd,
                },
            )),
            min_sanity_price_wad: 0,
            max_sanity_price_wad: 0,
        },
    }
}

#[test]
fn event_position_mode_eq_and_from() {
    assert_eq!(EventPositionMode::None, EventPositionMode::None);
    assert_ne!(EventPositionMode::Long, EventPositionMode::Short);
    assert_eq!(
        EventPositionMode::from(PositionMode::Normal),
        EventPositionMode::None
    );
    assert_eq!(
        EventPositionMode::from(PositionMode::Multiply),
        EventPositionMode::Multiply
    );
    assert_eq!(
        EventPositionMode::from(PositionMode::Long),
        EventPositionMode::Long
    );
    assert_eq!(
        EventPositionMode::from(PositionMode::Short),
        EventPositionMode::Short
    );
}

#[test]
fn event_oracle_type_eq_and_from() {
    assert_eq!(EventOracleType::None, EventOracleType::None);
    assert_ne!(EventOracleType::None, EventOracleType::Normal);
}

#[test]
fn event_pricing_method_eq_and_from() {
    assert_eq!(EventPricingMethod::None, EventPricingMethod::None);
    assert_ne!(EventPricingMethod::Safe, EventPricingMethod::Instant);
    assert_eq!(
        EventPricingMethod::from(OracleStrategy::Single),
        EventPricingMethod::Instant
    );
    assert_eq!(
        EventPricingMethod::from(OracleStrategy::PrimaryWithAnchor),
        EventPricingMethod::Mix
    );
}

#[test]
fn event_account_attributes_from_account_meta_spoke() {
    let env = Env::default();
    let owner = dummy_address(&env);
    let meta = AccountMeta {
        owner: owner.clone(),
        spoke_id: 3,
        mode: PositionMode::Long,
    };
    let attrs = EventAccountAttributes::from(&meta);
    assert_eq!(attrs.0, owner);
    assert_eq!(attrs.1, 3);
    assert_eq!(attrs.2, EventPositionMode::Long);
}

#[test]
fn event_oracle_provider_from_market_builds_struct() {
    let env = Env::default();
    let market = dummy_market_config(&env);
    let asset = dummy_address(&env);
    let provider = EventOracleProvider::from_market(&env, &asset, &market);
    assert_eq!(
        provider.primary_provider,
        OracleProviderKind::ReflectorSep40 as u32
    );
    assert_eq!(provider.primary_decimals, 14);
    assert_eq!(provider.primary_twap_records, 12);
    assert_eq!(provider.primary_max_stale_seconds, 900);
    assert!(provider.primary_asset.is_some());
    assert_eq!(provider.anchor_decimals, 7);
    assert_eq!(provider.anchor_twap_records, 0);
    assert_eq!(provider.anchor_max_stale_seconds, 900);
    assert!(provider.anchor_contract.is_some());
}

#[test]
fn update_asset_oracle_event_nests_oracle_fields_under_oracle_key() {
    extern crate std;
    use soroban_sdk::testutils::Events;
    use soroban_sdk::xdr::{ContractEventBody, ScVal};
    use std::string::{String, ToString};
    use std::vec::Vec as StdVec;

    fn map_keys(v: &ScVal) -> StdVec<String> {
        match v {
            ScVal::Map(Some(m)) => m
                .iter()
                .filter_map(|e| match &e.key {
                    ScVal::Symbol(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => StdVec::new(),
        }
    }
    fn nested<'a>(v: &'a ScVal, key: &str) -> &'a ScVal {
        match v {
            ScVal::Map(Some(m)) => m
                .iter()
                .find(|e| matches!(&e.key, ScVal::Symbol(s) if s.to_string() == key))
                .map(|e| &e.val)
                .expect("nested key present"),
            _ => panic!("not a map"),
        }
    }

    let (env, contract) = setup();
    env.as_contract(&contract, || {
        let asset = dummy_address(&env);
        let market = dummy_market_config(&env);
        UpdateAssetOracleEvent {
            asset: asset.clone(),
            oracle: EventOracleProvider::from_market(&env, &asset, &market),
        }
        .publish(&env);
    });

    let all = env.events().all();
    let xdr_events = all.events();
    let last = xdr_events.last().expect("event published");
    let ContractEventBody::V0(body) = &last.body;
    let data = &body.data;

    // Event data exposes only `asset` and `oracle` at the top level.
    // Sanity bounds and quote tokens are nested under `oracle`.
    let top = map_keys(data);
    assert!(top.iter().any(|k| k == "oracle"), "top keys: {:?}", top);
    assert!(top.iter().any(|k| k == "asset"));
    assert!(!top.iter().any(|k| k == "min_sanity_price_wad"));
    assert!(!top.iter().any(|k| k == "primary_quote_token"));

    // Sanity bounds and per-source quote tokens live inside `oracle`.
    let oracle_keys = map_keys(nested(data, "oracle"));
    for expected in [
        "min_sanity_price_wad",
        "max_sanity_price_wad",
        "primary_quote_token",
        "anchor_quote_token",
    ] {
        assert!(
            oracle_keys.iter().any(|k| k == expected),
            "missing {expected} in oracle keys: {oracle_keys:?}"
        );
    }
}

#[test]
fn emit_helpers_publish_without_panicking() {
    let (env, contract) = setup();
    env.as_contract(&contract, || {
        let asset = dummy_address(&env);
        let caller = dummy_address(&env);
        let market = dummy_market_config(&env);

        CreateMarketEvent {
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
            config: dummy_asset_config(&env),
        }
        .publish(&env);

        UpdateMarketParamsEvent {
            asset: asset.clone(),
            max_borrow_rate_ray: 0,
            base_borrow_rate_ray: 0,
            slope1_ray: 0,
            slope2_ray: 0,
            slope3_ray: 0,
            mid_utilization_ray: 0,
            optimal_utilization_ray: 0,
            max_utilization_ray: 0,
            reserve_factor_bps: 0,
        }
        .publish(&env);

        let mut deposits = Vec::new(&env);
        deposits.push_back(EventDepositDelta(
            PositionAction::Supply,
            asset.clone(),
            0,
            0,
            0,
            0,
            0,
            0,
        ));
        UpdatePositionBatchEvent {
            account_id: 1,
            account_attributes: EventAccountAttributes(caller.clone(), 0, EventPositionMode::None),
            deposits,
            borrows: Vec::new(&env),
        }
        .publish(&env);

        FlashLoanEvent {
            asset: asset.clone(),
            receiver: caller.clone(),
            caller: caller.clone(),
            amount: 0,
            fee: 0,
        }
        .publish(&env);

        UpdateAssetConfigEvent {
            asset: asset.clone(),
            config: dummy_asset_config(&env),
        }
        .publish(&env);

        UpdateAssetOracleEvent {
            asset: asset.clone(),
            oracle: EventOracleProvider::from_market(&env, &asset, &market),
        }
        .publish(&env);

        UpdateSpokeEvent {
            spoke: EventSpoke {
                spoke_id: 1,
                is_deprecated: false,
            },
        }
        .publish(&env);

        UpdateSpokeAssetEvent {
            asset: asset.clone(),
            config: SpokeAssetConfig {
                is_collateralizable: true,
                is_borrowable: true,
                paused: false,
                frozen: false,
                loan_to_value_bps: 9000,
                liquidation_threshold_bps: 9500,
                liquidation_bonus_bps: 200,
                liquidation_fees_bps: 0,
                supply_cap: 0,
                borrow_cap: 0,
                oracle_override: MarketOracleConfigOption::None,
            },
            spoke_id: 1,
        }
        .publish(&env);

        RemoveSpokeAssetEvent {
            asset: asset.clone(),
            spoke_id: 1,
        }
        .publish(&env);

        CleanBadDebtEvent {
            account_id: 1,
            total_borrow_usd_wad: 0,
            total_collateral_usd_wad: 0,
        }
        .publish(&env);

        InitialMultiplyPaymentEvent {
            token: asset.clone(),
            amount: 0,
            usd_value_wad: 0,
            account_id: 1,
        }
        .publish(&env);

        ApproveTokenEvent {
            wasm_hash: BytesN::from_array(&env, &[0u8; 32]),
            approved: true,
        }
        .publish(&env);

        // Reference vec! to keep it used even if the macro path changes.
        let _ignored: Vec<Address> = vec![&env];
    });
}
