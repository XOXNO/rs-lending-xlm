use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE,
};
use common::errors::{CollateralError, EModeError, GenericError, OracleError};
use common::events::{
    emit_remove_emode_asset, emit_update_asset_config, emit_update_asset_oracle,
    emit_update_emode_asset, emit_update_emode_category, EventOracleProvider,
    RemoveEModeAssetEvent, UpdateAssetConfigEvent, UpdateAssetOracleEvent, UpdateEModeAssetEvent,
    UpdateEModeCategoryEvent,
};
use common::fp_core;
#[cfg(test)]
use common::types::ReflectorConfig;
use common::types::{
    AssetConfig, EModeAssetConfig, EModeCategory, MarketOracleConfigInput, MarketStatus,
    OraclePriceFluctuation, OracleProviderConfig, OracleType, PositionLimits,
};
use soroban_sdk::{panic_with_error, token, Address, BytesN, Env, Executable};

use crate::oracle::reflector::{ReflectorAsset, ReflectorClient};
use crate::{storage, validation};

fn require_contract_address(env: &Env, addr: &Address, error: impl Into<soroban_sdk::Error>) {
    if !addr.exists() || !matches!(addr.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, error);
    }
}

fn require_nonzero_wasm_hash(env: &Env, hash: &BytesN<32>) {
    if hash.to_array() == [0; 32] {
        panic_with_error!(env, GenericError::InvalidPoolTemplate);
    }
}

// ---------------------------------------------------------------------------
// Address configuration
// ---------------------------------------------------------------------------

pub fn set_aggregator(env: &Env, addr: Address) {
    require_contract_address(env, &addr, OracleError::InvalidAggregator);
    storage::set_aggregator(env, &addr);
}

pub fn set_accumulator(env: &Env, addr: Address) {
    require_contract_address(env, &addr, GenericError::NotSmartContract);
    storage::set_accumulator(env, &addr);
}

pub fn set_liquidity_pool_template(env: &Env, hash: BytesN<32>) {
    require_nonzero_wasm_hash(env, &hash);
    storage::set_pool_template(env, &hash);
}

// ---------------------------------------------------------------------------
// Asset configuration
// ---------------------------------------------------------------------------

pub fn edit_asset_config(env: &Env, asset: Address, config: AssetConfig) {
    validation::validate_asset_config(env, &config);

    let mut market = storage::get_market_config(env, &asset);
    let mut next_config = config;
    next_config.e_mode_enabled = market.asset_config.e_mode_enabled;
    market.asset_config = next_config.clone();
    storage::set_market_config(env, &asset, &market);

    emit_update_asset_config(
        env,
        UpdateAssetConfigEvent {
            asset,
            config: next_config,
        },
    );
}

// ---------------------------------------------------------------------------
// Position limits
// ---------------------------------------------------------------------------

pub fn set_position_limits(env: &Env, limits: PositionLimits) {
    // Reject 0 (would brick supply/borrow for every user) and > 32 (would
    // let liquidation iteration exhaust gas).
    if limits.max_supply_positions == 0
        || limits.max_borrow_positions == 0
        || limits.max_supply_positions > 32
        || limits.max_borrow_positions > 32
    {
        panic_with_error!(env, GenericError::InvalidPositionLimits);
    }
    storage::set_position_limits(env, &limits);
}

// ---------------------------------------------------------------------------
// E-Mode categories
// ---------------------------------------------------------------------------

pub fn add_e_mode_category(env: &Env, ltv: i128, threshold: i128, bonus: i128) -> u32 {
    if ltv < 0 || threshold <= ltv || threshold > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
    if !(0..=common::constants::MAX_LIQUIDATION_BONUS).contains(&bonus) {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    let id = storage::increment_emode_category_id(env);
    let cat = EModeCategory {
        category_id: id,
        loan_to_value_bps: ltv,
        liquidation_threshold_bps: threshold,
        liquidation_bonus_bps: bonus,
        is_deprecated: false,
    };
    storage::set_emode_category(env, id, &cat);

    emit_update_emode_category(env, UpdateEModeCategoryEvent { category: cat });

    id
}

pub fn edit_e_mode_category(env: &Env, id: u32, ltv: i128, threshold: i128, bonus: i128) {
    if ltv < 0 || threshold <= ltv || threshold > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
    if !(0..=common::constants::MAX_LIQUIDATION_BONUS).contains(&bonus) {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
    let mut cat = storage::try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    cat.loan_to_value_bps = ltv;
    cat.liquidation_threshold_bps = threshold;
    cat.liquidation_bonus_bps = bonus;
    storage::set_emode_category(env, id, &cat);

    emit_update_emode_category(env, UpdateEModeCategoryEvent { category: cat });
}

pub fn remove_e_mode_category(env: &Env, id: u32) {
    let mut cat = storage::try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    cat.is_deprecated = true;
    storage::set_emode_category(env, id, &cat);

    // Soroban storage is not iterable, so per-asset membership entries
    // survive deprecation. The `is_deprecated` flag blocks new positions
    // from entering this category; runtime checks reject existing entries.

    emit_update_emode_category(env, UpdateEModeCategoryEvent { category: cat });
}

// ---------------------------------------------------------------------------
// E-Mode asset membership
// ---------------------------------------------------------------------------

pub fn add_asset_to_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
) {
    // Validate that the category exists and is not deprecated (single read).
    let cat = storage::try_get_emode_category(env, category_id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    if cat.is_deprecated {
        panic_with_error!(env, EModeError::EModeCategoryDeprecated);
    }

    // Guard: asset must be supported in the primary market.
    if !storage::has_market_config(env, &asset) {
        panic_with_error!(env, GenericError::AssetNotSupported);
    }

    // Guard: asset must not already belong to this category.
    if storage::get_emode_asset(env, category_id, &asset).is_some() {
        panic_with_error!(env, EModeError::AssetAlreadyInEmode);
    }

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    // Maintain the asset -> category index. Enable `e_mode_enabled` on the
    // market config when the asset enters its first e-mode category.
    let mut asset_cats = storage::get_asset_emodes(env, &asset);
    let is_first_category = asset_cats.is_empty();
    if !asset_cats.contains(category_id) {
        asset_cats.push_back(category_id);
        storage::set_asset_emodes(env, &asset, &asset_cats);
    }
    if is_first_category {
        if let Some(mut market) = storage::try_get_market_config(env, &asset) {
            if !market.asset_config.e_mode_enabled {
                market.asset_config.e_mode_enabled = true;
                storage::set_market_config(env, &asset, &market);
            }
        }
    }

    emit_update_emode_asset(
        env,
        UpdateEModeAssetEvent {
            asset,
            config,
            category_id,
        },
    );
}

pub fn edit_asset_in_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
) {
    // Guard: asset must exist in this category.
    if storage::get_emode_asset(env, category_id, &asset).is_none() {
        panic_with_error!(env, EModeError::AssetNotInEmode);
    }

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    emit_update_emode_asset(
        env,
        UpdateEModeAssetEvent {
            asset,
            config,
            category_id,
        },
    );
}

pub fn remove_asset_from_e_mode(env: &Env, asset: Address, category_id: u32) {
    storage::remove_emode_asset(env, category_id, &asset);

    // Remove the asset -> category index entry. Disable `e_mode_enabled`
    // once the asset belongs to no e-mode category.
    let mut asset_cats = storage::get_asset_emodes(env, &asset);
    if let Some(idx) = asset_cats.iter().position(|id| id == category_id) {
        asset_cats.remove(idx as u32);
        storage::set_asset_emodes(env, &asset, &asset_cats);
    }
    if asset_cats.is_empty() {
        if let Some(mut market) = storage::try_get_market_config(env, &asset) {
            if market.asset_config.e_mode_enabled {
                market.asset_config.e_mode_enabled = false;
                storage::set_market_config(env, &asset, &market);
            }
        }
    }

    emit_remove_emode_asset(env, RemoveEModeAssetEvent { asset, category_id });
}

// ---------------------------------------------------------------------------
// Oracle configuration
// ---------------------------------------------------------------------------

fn calculate_tolerance_range(env: &Env, tolerance: i128) -> (i128, i128) {
    let upper_bound = BPS + tolerance;
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

fn validate_and_calculate_tolerances(
    env: &Env,
    first_tolerance: i128,
    last_tolerance: i128,
) -> OraclePriceFluctuation {
    if !(MIN_FIRST_TOLERANCE..=MAX_FIRST_TOLERANCE).contains(&first_tolerance) {
        panic_with_error!(env, OracleError::BadFirstTolerance);
    }
    if !(MIN_LAST_TOLERANCE..=MAX_LAST_TOLERANCE).contains(&last_tolerance) {
        panic_with_error!(env, OracleError::BadLastTolerance);
    }

    validation::validate_oracle_bounds(env, first_tolerance, last_tolerance);

    let (first_upper, first_lower) = calculate_tolerance_range(env, first_tolerance);
    let (last_upper, last_lower) = calculate_tolerance_range(env, last_tolerance);

    OraclePriceFluctuation {
        first_upper_ratio_bps: first_upper,
        first_lower_ratio_bps: first_lower,
        last_upper_ratio_bps: last_upper,
        last_lower_ratio_bps: last_lower,
    }
}

fn validate_oracle_asset(env: &Env, asset: &Address) -> u32 {
    let token_decimals = token::Client::new(env, asset)
        .try_decimals()
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset))
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset));
    if token::Client::new(env, asset).try_symbol().is_err() {
        panic_with_error!(env, GenericError::InvalidAsset);
    }
    token_decimals
}

fn resolve_oracle_decimals(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfigInput,
) -> (u32, u32, u32) {
    if config.twap_records > 12 {
        panic_with_error!(env, OracleError::InvalidOracleTokenType);
    }
    if config.exchange_source == common::types::ExchangeSource::DualOracle
        && config.dex_oracle.is_none()
    {
        panic_with_error!(env, GenericError::InvalidExchangeSrc);
    }

    let asset_decimals = validate_oracle_asset(env, asset);
    let reflector_asset = match config.cex_asset_kind {
        common::types::ReflectorAssetKind::Stellar => ReflectorAsset::Stellar(asset.clone()),
        common::types::ReflectorAssetKind::Other => {
            ReflectorAsset::Other(config.cex_symbol.clone())
        }
    };

    let cex_client = ReflectorClient::new(env, &config.cex_oracle);
    let cex_decimals = cex_client.decimals();
    if cex_client.lastprice(&reflector_asset).is_none() {
        panic_with_error!(env, GenericError::InvalidTicker);
    }

    // Probe the DEX feed with the operator-supplied dex_symbol and
    // dex_asset_kind and reject unresolvable symbols at config time.
    let dex_decimals = if let Some(dex_addr) = config.dex_oracle.clone() {
        let dex_client = ReflectorClient::new(env, &dex_addr);
        let dex_asset = match config.dex_asset_kind {
            common::types::ReflectorAssetKind::Stellar => ReflectorAsset::Stellar(asset.clone()),
            common::types::ReflectorAssetKind::Other => {
                ReflectorAsset::Other(config.dex_symbol.clone())
            }
        };
        if dex_client.lastprice(&dex_asset).is_none() {
            panic_with_error!(env, GenericError::InvalidTicker);
        }
        dex_client.decimals()
    } else {
        0
    };

    (asset_decimals, cex_decimals, dex_decimals)
}

pub fn configure_market_oracle(env: &Env, asset: Address, config: MarketOracleConfigInput) {
    let mut market = match storage::try_get_market_config(env, &asset) {
        Some(m) => m,
        None => panic_with_error!(env, GenericError::AssetNotSupported),
    };

    if !matches!(
        market.status,
        MarketStatus::PendingOracle | MarketStatus::Active | MarketStatus::Disabled
    ) {
        panic_with_error!(env, GenericError::PairNotActive);
    }

    // `ExchangeSource::SpotOnly` has no tolerance, TWAP, or divergence
    // check; forbid it in production builds so a compromised ORACLE key
    // cannot weaken a live market to unprotected pricing. Test builds
    // retain SpotOnly for coverage.
    #[cfg(not(feature = "testing"))]
    if matches!(
        config.exchange_source,
        common::types::ExchangeSource::SpotOnly
    ) {
        panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
    }

    if config.max_price_stale_seconds < 60 || config.max_price_stale_seconds > 86_400 {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }

    let (asset_decimals, cex_decimals, dex_decimals) =
        resolve_oracle_decimals(env, &asset, &config);
    // Persist token precision discovered from the asset contract. Under the
    // `testing` feature, preserve any synthetic precision seeded at market
    // creation because the integration harness uses Soroban's SAC helper
    // (fixed at 7 decimals).
    let persisted_asset_decimals =
        if cfg!(feature = "testing") && market.oracle_config.asset_decimals != 0 {
            market.oracle_config.asset_decimals
        } else {
            asset_decimals
        };

    let oracle_config = OracleProviderConfig {
        base_asset: asset.clone(),
        oracle_type: OracleType::Normal,
        exchange_source: config.exchange_source,
        asset_decimals: persisted_asset_decimals,
        tolerance: validate_and_calculate_tolerances(
            env,
            config.first_tolerance_bps,
            config.last_tolerance_bps,
        ),
        max_price_stale_seconds: config.max_price_stale_seconds,
    };

    market.oracle_config = oracle_config.clone();
    market.status = MarketStatus::Active;
    market.cex_oracle = Some(config.cex_oracle);
    market.cex_asset_kind = config.cex_asset_kind;
    market.cex_symbol = config.cex_symbol;
    market.cex_decimals = cex_decimals;
    market.dex_oracle = config.dex_oracle;
    market.dex_asset_kind = config.dex_asset_kind;
    market.dex_symbol = config.dex_symbol;
    market.dex_decimals = dex_decimals;
    market.twap_records = config.twap_records;
    storage::set_market_config(env, &asset, &market);

    emit_update_asset_oracle(
        env,
        UpdateAssetOracleEvent {
            asset,
            oracle: EventOracleProvider::from_market(env, &market),
        },
    );
}

pub fn edit_oracle_tolerance(
    env: &Env,
    asset: Address,
    first_tolerance: i128,
    last_tolerance: i128,
) {
    let tolerance = validate_and_calculate_tolerances(env, first_tolerance, last_tolerance);

    let mut market = storage::get_market_config(env, &asset);
    market.oracle_config.tolerance = tolerance;
    storage::set_market_config(env, &asset, &market);

    emit_update_asset_oracle(
        env,
        UpdateAssetOracleEvent {
            asset,
            oracle: EventOracleProvider::from_market(env, &market),
        },
    );
}

pub fn disable_token_oracle(env: &Env, asset: Address) {
    let mut market = storage::get_market_config(env, &asset);
    market.status = MarketStatus::Disabled;
    storage::set_market_config(env, &asset, &market);
}

#[cfg(test)]
pub fn set_reflector_config(env: &Env, asset: Address, config: ReflectorConfig) {
    let (_, cex_decimals, dex_decimals) = resolve_oracle_decimals(
        env,
        &asset,
        &MarketOracleConfigInput {
            exchange_source: common::types::ExchangeSource::SpotVsTwap,
            max_price_stale_seconds: 900,
            first_tolerance_bps: 200,
            last_tolerance_bps: 500,
            cex_oracle: config.cex_oracle.clone(),
            cex_asset_kind: config.cex_asset_kind.clone(),
            cex_symbol: config.cex_symbol.clone(),
            dex_oracle: config.dex_oracle.clone(),
            dex_asset_kind: config.dex_asset_kind.clone(),
            dex_symbol: config.cex_symbol.clone(),
            twap_records: config.twap_records,
        },
    );

    let mut config = config;
    config.cex_decimals = cex_decimals;
    config.dex_decimals = dex_decimals;
    storage::set_reflector_config(env, &asset, &config);
}

#[cfg(test)]
pub fn set_token_oracle(
    env: &Env,
    asset: Address,
    config: OracleProviderConfig,
    first_tolerance: i128,
    last_tolerance: i128,
) {
    let market = storage::get_market_config(env, &asset);
    let cfg = MarketOracleConfigInput {
        exchange_source: config.exchange_source,
        max_price_stale_seconds: config.max_price_stale_seconds,
        first_tolerance_bps: first_tolerance,
        last_tolerance_bps: last_tolerance,
        cex_oracle: market
            .cex_oracle
            .unwrap_or_else(|| panic_with_error!(env, OracleError::ReflectorNotConfigured)),
        cex_asset_kind: market.cex_asset_kind,
        cex_symbol: market.cex_symbol.clone(),
        dex_oracle: market.dex_oracle,
        dex_asset_kind: market.dex_asset_kind,
        dex_symbol: market.dex_symbol,
        twap_records: market.twap_records,
    };

    configure_market_oracle(env, asset, cfg);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{
        AssetConfig, MarketConfig, MarketParams, MarketStatus, OraclePriceFluctuation,
        OracleProviderConfig, ReflectorAssetKind, ReflectorConfig,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Symbol};

    struct TestSetup {
        env: Env,
        controller: Address,
        asset: Address,
        cex_oracle: Address,
        dex_oracle: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin,));
            let asset = env
                .register_stellar_asset_contract_v2(Address::generate(&env))
                .address()
                .clone();

            // Register a dummy reflector config so interest updates don't panic.
            let reflector = env.register(crate::helpers::testutils::TestReflector, ());
            let r_client = crate::helpers::testutils::TestReflectorClient::new(&env, &reflector);
            r_client.set_spot(
                &crate::helpers::testutils::TestReflectorAsset::Stellar(asset.clone()),
                &10_0000000_0000000i128,
                &10_000,
            );

            let dex_oracle = env.register(crate::helpers::testutils::TestReflector, ());
            let dex_client = crate::helpers::testutils::TestReflectorClient::new(&env, &dex_oracle);
            dex_client.set_spot(
                &crate::helpers::testutils::TestReflectorAsset::Stellar(asset.clone()),
                &10_0000000_0000000i128,
                &10_000,
            );

            let setup = Self {
                env,
                controller,
                asset,
                cex_oracle: reflector,
                dex_oracle,
            };

            setup.as_controller(|| {
                crate::storage::set_market_config(
                    &setup.env,
                    &setup.asset,
                    &setup.market_config(common::types::OracleType::None),
                );
                crate::storage::set_reflector_config(
                    &setup.env,
                    &setup.asset,
                    &setup.reflector_config(0),
                );
            });

            setup
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn asset_config(&self) -> AssetConfig {
            AssetConfig {
                loan_to_value_bps: 7_500,
                liquidation_threshold_bps: 8_000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                is_collateralizable: true,
                is_borrowable: true,
                e_mode_enabled: false,
                is_isolated_asset: false,
                is_siloed_borrowing: false,
                is_flashloanable: true,
                isolation_borrow_enabled: true,
                isolation_debt_ceiling_usd_wad: 1_000_000,
                flashloan_fee_bps: 9,
                borrow_cap: i128::MAX,
                supply_cap: i128::MAX,
            }
        }

        fn market_config(&self, oracle_type: common::types::OracleType) -> MarketConfig {
            MarketConfig {
                status: if oracle_type == common::types::OracleType::None {
                    MarketStatus::PendingOracle
                } else {
                    MarketStatus::Active
                },
                asset_config: self.asset_config(),
                pool_address: Address::generate(&self.env),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset.clone(),
                    oracle_type,
                    exchange_source: common::types::ExchangeSource::SpotOnly,
                    asset_decimals: 7,
                    tolerance: OraclePriceFluctuation {
                        first_upper_ratio_bps: 10_200,
                        first_lower_ratio_bps: 9_800,
                        last_upper_ratio_bps: 10_500,
                        last_lower_ratio_bps: 9_500,
                    },
                    max_price_stale_seconds: 900,
                },
                cex_oracle: None,
                cex_asset_kind: ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, ""),
                cex_decimals: 0,
                dex_oracle: None,
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 0,
                twap_records: 0,
            }
        }

        fn reflector_config(&self, twap_records: u32) -> ReflectorConfig {
            ReflectorConfig {
                cex_oracle: self.cex_oracle.clone(),
                cex_asset_kind: ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, "USDC"),
                cex_decimals: 14,
                dex_oracle: None,
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_decimals: 0,
                twap_records,
            }
        }

        fn market_params(&self) -> MarketParams {
            MarketParams {
                max_borrow_rate_ray: 1_000_000,
                base_borrow_rate_ray: 5,
                slope1_ray: 100,
                slope2_ray: 1_000,
                slope3_ray: 100_000,
                mid_utilization_ray: 50,
                optimal_utilization_ray: 80,
                reserve_factor_bps: 100,
                asset_id: self.asset.clone(),
                asset_decimals: 7,
            }
        }
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #204)")]
    fn test_set_reflector_config_rejects_excessive_twap_records() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            set_reflector_config(&t.env, t.asset.clone(), t.reflector_config(13));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_edit_asset_config_rejects_excessive_liquidation_fees() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            let mut config = t.asset_config();
            config.liquidation_fees_bps = 10_001;
            edit_asset_config(&t.env, t.asset.clone(), config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_edit_e_mode_category_rejects_threshold_not_above_ltv() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            edit_e_mode_category(&t.env, id, 9_800, 9_800, 200);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #308)")]
    fn test_add_asset_to_e_mode_category_rejects_duplicate_asset() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);
        });
    }

    #[test]
    fn test_add_asset_to_e_mode_category_enables_market_flag() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);

            let market = storage::get_market_config(&t.env, &t.asset);
            assert!(market.asset_config.e_mode_enabled);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #307)")]
    fn test_edit_asset_in_e_mode_category_rejects_missing_asset() {
        let t = TestSetup::new();
        t.as_controller(|| {
            edit_asset_in_e_mode_category(&t.env, t.asset.clone(), 1, true, true);
        });
    }

    #[test]
    fn test_remove_asset_from_e_mode_disables_market_flag_when_last_category_is_removed() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);
            remove_asset_from_e_mode(&t.env, t.asset.clone(), id);

            let market = storage::get_market_config(&t.env, &t.asset);
            assert!(!market.asset_config.e_mode_enabled);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_add_asset_to_e_mode_category_requires_market_config() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, Address::generate(&t.env), id, true, true);
        });
    }

    #[test]
    fn test_add_asset_to_e_mode_category_preserves_pre_enabled_market_flag() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let mut market = t.market_config(common::types::OracleType::None);
            market.asset_config.e_mode_enabled = true;
            storage::set_market_config(&t.env, &t.asset, &market);

            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);

            assert!(
                storage::get_market_config(&t.env, &t.asset)
                    .asset_config
                    .e_mode_enabled
            );
        });
    }

    #[test]
    fn test_remove_asset_from_e_mode_preserves_pre_disabled_market_flag() {
        let t = TestSetup::new();

        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );

            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);

            let mut market = storage::get_market_config(&t.env, &t.asset);
            market.asset_config.e_mode_enabled = false;
            storage::set_market_config(&t.env, &t.asset, &market);

            remove_asset_from_e_mode(&t.env, t.asset.clone(), id);

            assert!(
                !storage::get_market_config(&t.env, &t.asset)
                    .asset_config
                    .e_mode_enabled
            );
        });
    }

    #[test]
    fn test_set_token_oracle_allows_reconfiguring_active_market() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::Normal),
            );
            let mut reflector = t.reflector_config(3);
            reflector.dex_oracle = Some(t.dex_oracle.clone());
            storage::set_reflector_config(&t.env, &t.asset, &reflector);

            let mut cfg = t
                .market_config(common::types::OracleType::Normal)
                .oracle_config;
            cfg.exchange_source = common::types::ExchangeSource::DualOracle;

            set_token_oracle(&t.env, t.asset.clone(), cfg.clone(), 200, 500);

            let market = storage::get_market_config(&t.env, &t.asset);
            assert_eq!(market.status as u32, MarketStatus::Active as u32);
            assert_eq!(
                market.oracle_config.exchange_source,
                common::types::ExchangeSource::DualOracle
            );
        });
    }

    #[test]
    fn test_set_token_oracle_reenables_disabled_market() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let mut market = t.market_config(common::types::OracleType::Normal);
            market.status = MarketStatus::Disabled;
            storage::set_market_config(&t.env, &t.asset, &market);
            let mut reflector = t.reflector_config(3);
            reflector.dex_oracle = Some(t.dex_oracle.clone());
            storage::set_reflector_config(&t.env, &t.asset, &reflector);

            set_token_oracle(&t.env, t.asset.clone(), market.oracle_config, 200, 500);

            let updated = storage::get_market_config(&t.env, &t.asset);
            assert_eq!(updated.status as u32, MarketStatus::Active as u32);
        });
    }

    #[test]
    fn test_set_aggregator_stores_wasm_contract_address() {
        let t = TestSetup::new();

        t.as_controller(|| {
            set_aggregator(&t.env, t.cex_oracle.clone());
            assert_eq!(storage::get_aggregator(&t.env), t.cex_oracle);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #201)")]
    fn test_set_aggregator_rejects_non_contract_address() {
        let t = TestSetup::new();

        t.as_controller(|| {
            set_aggregator(&t.env, Address::generate(&t.env));
        });
    }

    #[test]
    fn test_set_accumulator_stores_wasm_contract_address() {
        let t = TestSetup::new();

        t.as_controller(|| {
            set_accumulator(&t.env, t.cex_oracle.clone());
            assert_eq!(storage::get_accumulator(&t.env), t.cex_oracle);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #18)")]
    fn test_set_accumulator_rejects_non_contract_address() {
        let t = TestSetup::new();

        t.as_controller(|| {
            set_accumulator(&t.env, Address::generate(&t.env));
        });
    }

    #[test]
    fn test_set_liquidity_pool_template_stores_nonzero_hash() {
        let t = TestSetup::new();
        let hash = BytesN::from_array(&t.env, &[1; 32]);

        t.as_controller(|| {
            set_liquidity_pool_template(&t.env, hash.clone());
            assert_eq!(storage::get_pool_template(&t.env), hash);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn test_set_liquidity_pool_template_rejects_zero_hash() {
        let t = TestSetup::new();

        t.as_controller(|| {
            set_liquidity_pool_template(&t.env, BytesN::from_array(&t.env, &[0; 32]));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_create_liquidity_pool_already_supported() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            crate::router::create_liquidity_pool(
                &t.env,
                &t.asset,
                &t.market_params(),
                &t.asset_config(),
            );
        });
    }

    // The integration test suite (test-harness/tests/admin_config_tests.rs)
    // verifies full contract deployment and deterministic salt uniqueness,
    // since the actual pool WASM is available there for the host deployer.

    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_add_e_mode_category_bonus_limit() {
        let t = TestSetup::new();
        t.as_controller(|| {
            add_e_mode_category(&t.env, 9_000, 9_500, 2_001);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_edit_e_mode_category_bonus_limit() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let id = add_e_mode_category(&t.env, 9_000, 9_500, 200);
            edit_e_mode_category(&t.env, id, 9_000, 9_500, 2_001);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #409)")]
    fn test_edit_asset_config_flash_fee_limit() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(
                &t.env,
                &t.asset,
                &t.market_config(common::types::OracleType::None),
            );
            let mut config = t.asset_config();
            config.flashloan_fee_bps = 501;
            edit_asset_config(&t.env, t.asset.clone(), config);
        });
    }
}
