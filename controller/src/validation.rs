use common::constants::{MAX_FLASHLOAN_FEE_BPS, MAX_LIQUIDATION_BONUS, RAY};
use common::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use common::types::{
    Account, AssetConfig, MarketParams, MarketStatus, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::storage;

pub fn require_asset_supported(env: &Env, asset: &Address) {
    if !storage::has_market_config(env, asset) {
        panic_with_error!(env, GenericError::AssetNotSupported);
    }
}

pub fn require_market_active(env: &Env, asset: &Address) {
    let market = storage::get_market_config(env, asset);
    if market.status != MarketStatus::Active {
        panic_with_error!(env, GenericError::PairNotActive);
    }
}

pub fn require_account_owner(env: &Env, account: &Account, caller: &Address) {
    if account.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }
    caller.require_auth();
}

pub fn require_not_paused(env: &Env) {
    stellar_contract_utils::pausable::when_not_paused(env);
}

pub fn require_not_flash_loaning(env: &Env) {
    if storage::is_flash_loan_ongoing(env) {
        panic_with_error!(env, FlashLoanError::FlashLoanOngoing);
    }
}

pub fn require_amount_positive(env: &Env, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

pub fn validate_bulk_position_limits(
    env: &Env,
    account: &Account,
    position_type: u32,
    assets: &Vec<(Address, i128)>,
) {
    let limits = storage::get_position_limits(env);

    let (current_count, max_allowed) = if position_type == POSITION_TYPE_DEPOSIT {
        (account.supply_positions.len(), limits.max_supply_positions)
    } else if position_type == POSITION_TYPE_BORROW {
        (account.borrow_positions.len(), limits.max_borrow_positions)
    } else {
        // M-08: panic on unknown position_type. Silent return would skip
        // the limit check entirely if a future caller passes a wrong value.
        panic_with_error!(env, GenericError::InvalidPositionType);
    };

    // Count how many new positions the batch would create.
    //
    // Repeated assets in one batch resolve to the same position; do not
    // count them twice.
    let mut seen: Map<Address, bool> = Map::new(env);
    let mut new_positions_count: u32 = 0;
    for i in 0..assets.len() {
        let (asset, _) = assets.get(i).unwrap();
        if seen.contains_key(asset.clone()) {
            continue;
        }
        seen.set(asset.clone(), true);

        let already_present = if position_type == POSITION_TYPE_DEPOSIT {
            account.supply_positions.contains_key(asset)
        } else {
            account.borrow_positions.contains_key(asset)
        };
        if !already_present {
            new_positions_count += 1;
        }
    }

    if current_count + new_positions_count > max_allowed {
        panic_with_error!(env, CollateralError::PositionLimitExceeded);
    }
}

pub fn validate_interest_rate_model(env: &Env, params: &MarketParams) {
    if params.base_borrow_rate_ray < 0
        || params.slope1_ray < params.base_borrow_rate_ray
        || params.slope2_ray < params.slope1_ray
        || params.slope3_ray < params.slope2_ray
        || params.max_borrow_rate_ray < params.slope3_ray
    {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    if params.mid_utilization_ray <= 0 {
        panic_with_error!(env, CollateralError::InvalidUtilRange);
    }
    if params.optimal_utilization_ray <= params.mid_utilization_ray {
        panic_with_error!(env, CollateralError::InvalidUtilRange);
    }
    if params.optimal_utilization_ray >= RAY {
        panic_with_error!(env, CollateralError::OptUtilTooHigh);
    }
    if params.reserve_factor_bps < 0 || params.reserve_factor_bps >= 10_000 {
        panic_with_error!(env, CollateralError::InvalidReserveFactor);
    }
}

pub fn validate_asset_config(env: &Env, config: &AssetConfig) {
    // Guard: LTV must be non-negative.
    if config.loan_to_value_bps < 0 {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    // Guard: liquidation threshold must stay above LTV and at or below 100%
    // so new debt cannot start in liquidatable territory and HF math stays
    // bounded.
    if config.liquidation_threshold_bps <= config.loan_to_value_bps
        || config.liquidation_threshold_bps > 10_000
    {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    // Guard: liquidation_bonus must be non-negative and stay within the
    // protocol-wide parity bound.
    if config.liquidation_bonus_bps < 0 || config.liquidation_bonus_bps > MAX_LIQUIDATION_BONUS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    // Guard: liquidation_fees must be non-negative and not exceed 100% (sanity bound).
    if config.liquidation_fees_bps < 0 || config.liquidation_fees_bps > 10_000 {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    // Guard: caps must be non-negative (0 = no cap, >0 = enforced limit).
    if config.supply_cap < 0 || config.borrow_cap < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    // Guard: isolation debt ceiling must be non-negative. A negative ceiling
    // makes the `isolated_debt > ceiling` check vacuously true, permitting
    // unlimited isolated borrowing.
    if config.isolation_debt_ceiling_usd_wad < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    // H-04: flashloan_fee_bps bounds. A negative fee would have the pool pay
    // receivers; a fee above MAX_FLASHLOAN_FEE_BPS exceeds the protocol cap.
    // Enforced here so `create_liquidity_pool` and `edit_asset_config` share
    // one validation site.
    if config.flashloan_fee_bps < 0 {
        panic_with_error!(env, FlashLoanError::NegativeFlashLoanFee);
    }
    if config.flashloan_fee_bps > MAX_FLASHLOAN_FEE_BPS {
        panic_with_error!(env, FlashLoanError::StrategyFeeExceeds);
    }
}

pub fn validate_oracle_bounds(env: &Env, first: i128, last: i128) {
    if last <= first {
        panic_with_error!(env, OracleError::BadAnchorTolerances);
    }
    // The caller's range check (`validate_and_calculate_tolerances`) enforces
    // the upper bound via `MAX_LAST_TOLERANCE`. No redundant cap here.
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{
        AccountPosition, AssetConfig, MarketConfig, MarketStatus, OraclePriceFluctuation,
        OracleProviderConfig,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Symbol};

    struct TestSetup {
        env: Env,
        contract: Address,
        asset_a: Address,
        asset_b: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let contract = env.register(crate::Controller, (admin.clone(),));
            let asset_a = Address::generate(&env);
            let asset_b = Address::generate(&env);

            Self {
                env,
                contract,
                asset_a,
                asset_b,
            }
        }

        fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.contract, f)
        }

        fn account_with_supply(&self) -> Account {
            let mut supply_positions = Map::new(&self.env);
            supply_positions.set(
                self.asset_a.clone(),
                AccountPosition {
                    position_type: common::types::AccountPositionType::Deposit,
                    asset: self.asset_a.clone(),
                    scaled_amount_ray: 100,
                    account_id: 1,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    loan_to_value_bps: 7_500,
                },
            );

            Account {
                owner: Address::generate(&self.env),
                is_isolated: false,
                e_mode_category_id: 0,
                mode: common::types::PositionMode::Normal,
                isolated_asset: None,
                supply_positions,
                borrow_positions: Map::new(&self.env),
            }
        }

        fn market_config(&self) -> MarketConfig {
            MarketConfig {
                status: MarketStatus::PendingOracle,
                asset_config: AssetConfig {
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
                },
                pool_address: Address::generate(&self.env),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset_a.clone(),
                    oracle_type: common::types::OracleType::Normal,
                    exchange_source: common::types::ExchangeSource::SpotOnly,
                    asset_decimals: 7,
                    tolerance: OraclePriceFluctuation {
                        first_upper_ratio_bps: 10_200,
                        first_lower_ratio_bps: 9_800,
                        last_upper_ratio_bps: 11_000,
                        last_lower_ratio_bps: 9_000,
                    },
                    max_price_stale_seconds: 900,
                },
                cex_oracle: None,
                cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, ""),
                cex_decimals: 0,
                dex_oracle: None,
                dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 0,
                twap_records: 0,
            }
        }
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_require_asset_supported_panics_for_missing_market() {
        let t = TestSetup::new();
        t.as_contract(|| {
            require_asset_supported(&t.env, &t.asset_a);
        });
    }

    // M-08: previously this fn silently no-op'd for unknown position_type.
    // Audit fix made it panic with InvalidPositionType (= 23) so a buggy
    // caller cannot bypass the position-limit check.
    #[test]
    #[should_panic(expected = "Error(Contract, #23)")]
    fn test_validate_bulk_position_limits_panics_for_unknown_position_type() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let account = t.account_with_supply();
            let mut assets = Vec::new(&t.env);
            assets.push_back((t.asset_b.clone(), 1));

            validate_bulk_position_limits(&t.env, &account, 99, &assets);
        });
    }

    #[test]
    fn test_validate_bulk_position_limits_does_not_double_count_duplicate_assets() {
        let t = TestSetup::new();
        t.as_contract(|| {
            crate::storage::set_market_config(&t.env, &t.asset_a, &t.market_config());
            let mut limits = crate::storage::get_position_limits(&t.env);
            limits.max_supply_positions = 2;
            crate::storage::set_position_limits(&t.env, &limits);

            let account = t.account_with_supply();
            let mut assets = Vec::new(&t.env);
            assets.push_back((t.asset_b.clone(), 1));
            assets.push_back((t.asset_b.clone(), 2));

            validate_bulk_position_limits(&t.env, &account, POSITION_TYPE_DEPOSIT, &assets);
        });
    }

    // H-04: validate_asset_config now also enforces flashloan_fee_bps bounds.
    // Previously these checks lived only in `edit_asset_config`, leaving
    // `create_liquidity_pool` (which calls validate_asset_config) wide open.
    #[test]
    #[should_panic(expected = "Error(Contract, #411)")]
    fn test_validate_asset_config_rejects_negative_flashloan_fee() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cfg = AssetConfig {
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
                flashloan_fee_bps: -1, // invalid: negative
                borrow_cap: i128::MAX,
                supply_cap: i128::MAX,
            };
            validate_asset_config(&t.env, &cfg);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #409)")]
    fn test_validate_asset_config_rejects_excessive_flashloan_fee() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cfg = AssetConfig {
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
                flashloan_fee_bps: 501, // > MAX_FLASHLOAN_FEE_BPS (500)
                borrow_cap: i128::MAX,
                supply_cap: i128::MAX,
            };
            validate_asset_config(&t.env, &cfg);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #109)")]
    fn test_validate_bulk_position_limits_rejects_when_new_positions_exceed_cap() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut limits = crate::storage::get_position_limits(&t.env);
            limits.max_supply_positions = 1;
            crate::storage::set_position_limits(&t.env, &limits);

            let account = t.account_with_supply();
            let mut assets = Vec::new(&t.env);
            assets.push_back((t.asset_b.clone(), 1));

            validate_bulk_position_limits(&t.env, &account, POSITION_TYPE_DEPOSIT, &assets);
        });
    }
}
