use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MAX_LIQUIDATION_BONUS, MIN_FIRST_TOLERANCE,
    MIN_LAST_TOLERANCE,
};
use common::errors::{CollateralError, EModeError, GenericError, OracleError};
use common::events::{
    emit_approve_token_wasm, emit_remove_emode_asset, emit_update_asset_config,
    emit_update_asset_oracle, emit_update_emode_asset, emit_update_emode_category,
    ApproveTokenWasmEvent, EventEModeCategory, EventOracleProvider, RemoveEModeAssetEvent,
    UpdateAssetConfigEvent, UpdateAssetOracleEvent, UpdateEModeAssetEvent,
    UpdateEModeCategoryEvent,
};
use common::fp_core;
#[cfg(test)]
use common::types::ReflectorConfig;
use common::types::{
    AssetConfig, EModeAssetConfig, EModeCategory, ExchangeSource, MarketOracleConfigInput,
    MarketStatus, OraclePriceFluctuation, OracleProviderConfig, OracleType, PositionLimits,
    ReflectorAssetKind,
};
use soroban_sdk::{
    contractimpl, panic_with_error, token, xdr::ToXdr, Address, BytesN, Env, Executable,
};
use stellar_macros::{only_owner, only_role};

use crate::oracle::reflector::{ReflectorAsset, ReflectorClient};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        set_aggregator(&env, addr);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        set_accumulator(&env, addr);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        set_liquidity_pool_template(&env, hash);
    }

    #[only_owner]
    pub fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfig) {
        edit_asset_config(&env, asset, cfg);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        set_position_limits(&env, limits);
    }

    #[only_owner]
    pub fn add_e_mode_category(env: Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
        add_e_mode_category(&env, ltv, threshold, bonus)
    }

    #[only_owner]
    pub fn edit_e_mode_category(env: Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
        edit_e_mode_category(&env, id, ltv, threshold, bonus);
    }

    #[only_owner]
    pub fn remove_e_mode_category(env: Env, id: u32) {
        remove_e_mode_category(&env, id);
    }

    #[only_owner]
    pub fn add_asset_to_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        add_asset_to_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[only_owner]
    pub fn edit_asset_in_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        edit_asset_in_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[only_owner]
    pub fn remove_asset_from_e_mode(env: Env, asset: Address, category_id: u32) {
        remove_asset_from_e_mode(&env, asset, category_id);
    }

    #[only_owner]
    pub fn remove_asset_e_mode_category(env: Env, asset: Address, category_id: u32) {
        remove_asset_from_e_mode(&env, asset, category_id);
    }

    #[only_owner]
    pub fn approve_token_wasm(env: Env, token: Address) {
        storage::set_token_approved(&env, &token, true);
        let wasm_hash = env.crypto().keccak256(&token.to_xdr(&env)).into();
        emit_approve_token_wasm(
            &env,
            ApproveTokenWasmEvent {
                wasm_hash,
                approved: true,
            },
        );
    }

    #[only_owner]
    pub fn revoke_token_wasm(env: Env, token: Address) {
        storage::set_token_approved(&env, &token, false);
        let wasm_hash = env.crypto().keccak256(&token.to_xdr(&env)).into();
        emit_approve_token_wasm(
            &env,
            ApproveTokenWasmEvent {
                wasm_hash,
                approved: false,
            },
        );
    }

    #[only_role(caller, "ORACLE")]
    pub fn configure_market_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) {
        let _ = caller;
        configure_market_oracle(&env, asset, cfg);
    }

    #[only_role(caller, "ORACLE")]
    pub fn edit_oracle_tolerance(
        env: Env,
        caller: Address,
        asset: Address,
        first_tolerance: u32,
        last_tolerance: u32,
    ) {
        let _ = caller;
        edit_oracle_tolerance(&env, asset, first_tolerance, last_tolerance);
    }

    #[only_role(caller, "ORACLE")]
    pub fn disable_token_oracle(env: Env, caller: Address, asset: Address) {
        let _ = caller;
        disable_token_oracle(&env, asset);
    }
}

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

pub fn edit_asset_config(env: &Env, asset: Address, mut next_config: AssetConfig) {
    validation::validate_asset_config(env, &next_config);

    let mut market = storage::get_market_config(env, &asset);
    // Preserve the controller-managed e-mode membership list — it's
    // updated via add/remove_asset_to_e_mode_category, never by the
    // admin asset-config edit path.
    next_config.e_mode_categories = market.asset_config.e_mode_categories.clone();
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

/// Validates an e-mode category's risk params against the global bps
/// invariants. Lifts the `u32` inputs into `i128` once so the existing
/// `BPS` / `MAX_LIQUIDATION_BONUS` i128 constants stay authoritative.
fn validate_emode_params(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    let ltv_i = i128::from(ltv);
    let threshold_i = i128::from(threshold);
    let bonus_i = i128::from(bonus);
    if threshold_i <= ltv_i || threshold_i > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
    if !(0..=MAX_LIQUIDATION_BONUS).contains(&bonus_i) {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }
}

pub fn add_e_mode_category(env: &Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
    validate_emode_params(env, ltv, threshold, bonus);

    let id = storage::increment_emode_category_id(env);
    let cat = EModeCategory {
        loan_to_value_bps: ltv,
        liquidation_threshold_bps: threshold,
        liquidation_bonus_bps: bonus,
        is_deprecated: false,
        assets: soroban_sdk::Map::new(env),
    };
    storage::set_emode_category(env, id, &cat);

    emit_update_emode_category(
        env,
        UpdateEModeCategoryEvent {
            category: EventEModeCategory::new(id, &cat),
        },
    );

    id
}

pub fn edit_e_mode_category(env: &Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
    validate_emode_params(env, ltv, threshold, bonus);
    let mut cat = storage::try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    cat.loan_to_value_bps = ltv;
    cat.liquidation_threshold_bps = threshold;
    cat.liquidation_bonus_bps = bonus;
    storage::set_emode_category(env, id, &cat);

    emit_update_emode_category(
        env,
        UpdateEModeCategoryEvent {
            category: EventEModeCategory::new(id, &cat),
        },
    );
}

pub fn remove_e_mode_category(env: &Env, id: u32) {
    let mut cat = storage::try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    cat.is_deprecated = true;

    // Snapshot the member set, clear it on the entry, then walk it to
    // drop the category id from each member's
    // `MarketConfig.asset_config.e_mode_categories` list. Accounts
    // already pointing at this category wind down through the keeper
    // path — `is_deprecated` stays set so `effective_asset_config`
    // falls back to base values.
    let members = cat.assets.clone();
    cat.assets = soroban_sdk::Map::new(env);
    storage::set_emode_category(env, id, &cat);

    for asset in members.keys() {
        if let Some(mut market) = storage::try_get_market_config(env, &asset) {
            if let Some(idx) = market
                .asset_config
                .e_mode_categories
                .iter()
                .position(|cid| cid == id)
            {
                market.asset_config.e_mode_categories.remove(idx as u32);
                storage::set_market_config(env, &asset, &market);
            }
        }
    }

    emit_update_emode_category(
        env,
        UpdateEModeCategoryEvent {
            category: EventEModeCategory::new(id, &cat),
        },
    );
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
    let cat = storage::try_get_emode_category(env, category_id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    if cat.is_deprecated {
        panic_with_error!(env, EModeError::EModeCategoryDeprecated);
    }

    if !storage::has_market_config(env, &asset) {
        panic_with_error!(env, GenericError::AssetNotSupported);
    }

    if storage::get_emode_asset(env, category_id, &asset).is_some() {
        panic_with_error!(env, EModeError::AssetAlreadyInEmode);
    }

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    // Append `category_id` to the asset's reverse index — stored on
    // `MarketConfig.asset_config.e_mode_categories`. `has_emode()`
    // becomes true on the first non-empty push, so no separate flag
    // toggle is needed.
    let mut market = storage::get_market_config(env, &asset);
    if !market.asset_config.e_mode_categories.contains(category_id) {
        market.asset_config.e_mode_categories.push_back(category_id);
        storage::set_market_config(env, &asset, &market);
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

    // Remove `category_id` from the asset's reverse index on
    // `MarketConfig.asset_config.e_mode_categories`. When the list
    // becomes empty `has_emode()` flips to false naturally.
    if let Some(mut market) = storage::try_get_market_config(env, &asset) {
        if let Some(idx) = market
            .asset_config
            .e_mode_categories
            .iter()
            .position(|id| id == category_id)
        {
            market.asset_config.e_mode_categories.remove(idx as u32);
            storage::set_market_config(env, &asset, &market);
        }
    }

    emit_remove_emode_asset(env, RemoveEModeAssetEvent { asset, category_id });
}

// ---------------------------------------------------------------------------
// Oracle configuration
// ---------------------------------------------------------------------------

/// Bps math is i128-backed; the inputs/outputs are bounded by `BPS +
/// MAX_*_TOLERANCE` so the return values always fit in `u32` after the
/// `i128` arithmetic settles. Convert at the boundary via this helper
/// so the bps domain invariant is enforced in one place.
fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

fn calculate_tolerance_range(env: &Env, tolerance: i128) -> (i128, i128) {
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

fn validate_and_calculate_tolerances(
    env: &Env,
    first_tolerance: u32,
    last_tolerance: u32,
) -> OraclePriceFluctuation {
    let first = i128::from(first_tolerance);
    let last = i128::from(last_tolerance);
    if !(MIN_FIRST_TOLERANCE..=MAX_FIRST_TOLERANCE).contains(&first) {
        panic_with_error!(env, OracleError::BadFirstTolerance);
    }
    if !(MIN_LAST_TOLERANCE..=MAX_LAST_TOLERANCE).contains(&last) {
        panic_with_error!(env, OracleError::BadLastTolerance);
    }

    validation::validate_oracle_bounds(env, first, last);

    let (first_upper, first_lower) = calculate_tolerance_range(env, first);
    let (last_upper, last_lower) = calculate_tolerance_range(env, last);

    OraclePriceFluctuation {
        first_upper_ratio_bps: bps_i128_to_u32(env, first_upper),
        first_lower_ratio_bps: bps_i128_to_u32(env, first_lower),
        last_upper_ratio_bps: bps_i128_to_u32(env, last_upper),
        last_lower_ratio_bps: bps_i128_to_u32(env, last_lower),
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
    if config.exchange_source == ExchangeSource::DualOracle && config.dex_oracle.is_none() {
        panic_with_error!(env, GenericError::InvalidExchangeSrc);
    }

    let asset_decimals = validate_oracle_asset(env, asset);
    let reflector_asset = match config.cex_asset_kind {
        ReflectorAssetKind::Stellar => ReflectorAsset::Stellar(asset.clone()),
        ReflectorAssetKind::Other => ReflectorAsset::Other(config.cex_symbol.clone()),
    };

    let cex_client = ReflectorClient::new(env, &config.cex_oracle);
    let cex_decimals = cex_client.decimals();
    if crate::oracle::reflector_lastprice_call(env, &config.cex_oracle, &reflector_asset).is_none()
    {
        panic_with_error!(env, GenericError::InvalidTicker);
    }

    // Probe the DEX feed with the operator-supplied dex_symbol and
    // dex_asset_kind and reject unresolvable symbols at config time.
    let dex_decimals = if let Some(dex_addr) = config.dex_oracle.clone() {
        let dex_client = ReflectorClient::new(env, &dex_addr);
        let dex_asset = match config.dex_asset_kind {
            ReflectorAssetKind::Stellar => ReflectorAsset::Stellar(asset.clone()),
            ReflectorAssetKind::Other => ReflectorAsset::Other(config.dex_symbol.clone()),
        };
        if crate::oracle::reflector_lastprice_call(env, &dex_addr, &dex_asset).is_none() {
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
    if matches!(config.exchange_source, ExchangeSource::SpotOnly) {
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
    first_tolerance: u32,
    last_tolerance: u32,
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
            exchange_source: ExchangeSource::SpotVsTwap,
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
    first_tolerance: u32,
    last_tolerance: u32,
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
        AssetConfig, ExchangeSource, MarketConfig, MarketParams, MarketStatus,
        OraclePriceFluctuation, OracleProviderConfig, ReflectorAssetKind, ReflectorConfig,
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
                    &setup.market_config(OracleType::None),
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
                e_mode_categories: soroban_sdk::Vec::new(&self.env),
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

        fn market_config(&self, oracle_type: OracleType) -> MarketConfig {
            MarketConfig {
                status: if oracle_type == OracleType::None {
                    MarketStatus::PendingOracle
                } else {
                    MarketStatus::Active
                },
                asset_config: self.asset_config(),
                pool_address: Address::generate(&self.env),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset.clone(),
                    oracle_type,
                    exchange_source: ExchangeSource::SpotOnly,
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
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
            set_reflector_config(&t.env, t.asset.clone(), t.reflector_config(13));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_edit_asset_config_rejects_excessive_liquidation_fees() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
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
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);
        });
    }

    #[test]
    fn test_add_asset_to_e_mode_category_records_membership() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);

            let market = storage::get_market_config(&t.env, &t.asset);
            assert!(market.asset_config.has_emode());
            assert!(market.asset_config.e_mode_categories.contains(id));
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
    fn test_remove_asset_from_e_mode_clears_membership_when_last_category_is_removed() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);
            remove_asset_from_e_mode(&t.env, t.asset.clone(), id);

            let market = storage::get_market_config(&t.env, &t.asset);
            assert!(!market.asset_config.has_emode());
            assert_eq!(market.asset_config.e_mode_categories.len(), 0);
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
    fn test_add_asset_to_e_mode_category_appends_without_duplicates() {
        let t = TestSetup::new();

        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
            let id = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id, true, true);

            let cats = storage::get_market_config(&t.env, &t.asset)
                .asset_config
                .e_mode_categories;
            assert_eq!(cats.len(), 1);
            assert!(cats.contains(id));
        });
    }

    #[test]
    fn test_remove_asset_from_e_mode_removes_only_named_category() {
        let t = TestSetup::new();

        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));

            let id_a = add_e_mode_category(&t.env, 9_700, 9_800, 200);
            let id_b = add_e_mode_category(&t.env, 9_500, 9_600, 300);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id_a, true, true);
            add_asset_to_e_mode_category(&t.env, t.asset.clone(), id_b, true, true);

            remove_asset_from_e_mode(&t.env, t.asset.clone(), id_a);

            let cats = storage::get_market_config(&t.env, &t.asset)
                .asset_config
                .e_mode_categories;
            assert_eq!(cats.len(), 1);
            assert!(cats.contains(id_b));
            assert!(!cats.contains(id_a));
        });
    }

    #[test]
    fn test_set_token_oracle_allows_reconfiguring_active_market() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::Normal));
            let mut reflector = t.reflector_config(3);
            reflector.dex_oracle = Some(t.dex_oracle.clone());
            storage::set_reflector_config(&t.env, &t.asset, &reflector);

            let mut cfg = t.market_config(OracleType::Normal).oracle_config;
            cfg.exchange_source = ExchangeSource::DualOracle;

            set_token_oracle(&t.env, t.asset.clone(), cfg.clone(), 200, 500);

            let market = storage::get_market_config(&t.env, &t.asset);
            assert_eq!(market.status as u32, MarketStatus::Active as u32);
            assert_eq!(
                market.oracle_config.exchange_source,
                ExchangeSource::DualOracle
            );
        });
    }

    #[test]
    fn test_set_token_oracle_reenables_disabled_market() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let mut market = t.market_config(OracleType::Normal);
            market.status = MarketStatus::Disabled;
            market.oracle_config.exchange_source = ExchangeSource::SpotVsTwap;
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
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
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
            storage::set_market_config(&t.env, &t.asset, &t.market_config(OracleType::None));
            let mut config = t.asset_config();
            config.flashloan_fee_bps = 501;
            edit_asset_config(&t.env, t.asset.clone(), config);
        });
    }
}
