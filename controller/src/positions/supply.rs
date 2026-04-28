use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, UpdatePositionEvent};
use common::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, MarketIndex, PriceFeed,
    POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Vec};

use super::{emode, update};
use crate::cache::ControllerCache;
use crate::{helpers, storage, utils, validation};

const THRESHOLD_UPDATE_MIN_HF: i128 = 1_050_000_000_000_000_000;

// ---------------------------------------------------------------------------
// Endpoint entry
// ---------------------------------------------------------------------------

/// Entry point for supply: validates, creates the account when `account_id == 0`,
/// and processes the deposit batch. Returns the resolved `account_id`.
pub fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<(Address, i128)>,
) -> u64 {
    caller.require_auth();
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);

    // Reject empty batch up-front: prevents a free TTL-bump no-op write on the
    // existing-account path and avoids reaching `assets.get(0).unwrap()` panics
    // on the create path.
    if assets.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }

    // Resolve or create the account.
    let acct_id = if account_id == 0 {
        utils::create_account_for_first_asset(env, caller, e_mode_category, assets)
    } else {
        account_id
    };

    // Load the account once: single storage read.
    let mut account = storage::get_account(env, acct_id);

    // Note: third-party deposits are intentionally permitted. Supplying to
    // someone else's account can only
    // increase their collateral / health factor — never decrease either. The
    // isolation/e-mode invariants are still enforced per asset via
    // `validate_isolated_collateral` and `validate_e_mode_asset` below.

    let mut cache = ControllerCache::new(env, true); // Supply is risk-decreasing.

    // Process deposits for every asset in the batch.
    process_deposit(env, caller, acct_id, &mut account, assets, &mut cache);

    // Single write at the end of the batch.
    storage::set_account(env, acct_id, &account);

    acct_id
}

// ---------------------------------------------------------------------------
// process_deposit -- batch loop + per-asset validation
// ---------------------------------------------------------------------------

/// Processes a deposit batch on `account`: validates e-mode, isolation, supply caps,
/// and calls the pool for each asset using balance-delta accounting.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    assets: &Vec<(Address, i128)>,
    cache: &mut ControllerCache,
) {
    // Fetch the e-mode category once and reuse across every iteration.
    let e_mode = emode::e_mode_category(env, account.e_mode_category_id);
    emode::ensure_e_mode_not_deprecated(env, &e_mode);

    // Pre-flight position limit check rejects the full batch atomically.
    validation::validate_bulk_position_limits(env, account, POSITION_TYPE_DEPOSIT, assets);

    // Pre-flight bulk-isolation guard.
    validation::validate_bulk_isolation(env, account, assets, cache);

    for (asset, amount) in assets {
        // validate_payment equivalent: asset must be supported, amount must be positive.
        validation::require_asset_supported(env, &asset);
        validation::require_amount_positive(env, amount);

        let mut asset_config = cache.cached_asset_config(&asset);
        let asset_emode_config = cache.cached_emode_asset(account.e_mode_category_id, &asset);

        emode::validate_e_mode_asset(env, account.e_mode_category_id, &asset, true);
        emode::ensure_e_mode_compatible_with_asset(env, &asset_config, account.e_mode_category_id);
        emode::apply_e_mode_to_asset_config(env, &mut asset_config, &e_mode, asset_emode_config);

        if !asset_config.can_supply() {
            panic_with_error!(env, CollateralError::NotCollateral);
        }

        emode::validate_isolated_collateral(env, account, &asset, &asset_config);

        let feed = cache.cached_price(&asset);
        validate_supply_cap(env, cache, &asset_config, &asset, amount, &feed);

        update_deposit_position(
            env,
            account_id,
            account,
            &asset,
            amount,
            &asset_config,
            caller,
            &feed,
            cache,
        );
    }
}

// ---------------------------------------------------------------------------
// get_or_create_deposit_position
// ---------------------------------------------------------------------------

fn get_or_create_deposit_position(
    account: &Account,
    account_id: u64,
    asset_config: &AssetConfig,
    asset: &Address,
) -> AccountPosition {
    account
        .supply_positions
        .get(asset.clone())
        .unwrap_or_else(|| AccountPosition {
            position_type: AccountPositionType::Deposit,
            asset: asset.clone(),
            scaled_amount_ray: 0,
            account_id,
            liquidation_threshold_bps: asset_config.liquidation_threshold_bps,
            liquidation_bonus_bps: asset_config.liquidation_bonus_bps,
            liquidation_fees_bps: asset_config.liquidation_fees_bps,
            loan_to_value_bps: asset_config.loan_to_value_bps,
        })
}

// ---------------------------------------------------------------------------
// update_deposit_position
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Updates the deposit position with the latest risk parameters and calls the pool to record the supply.
/// Refreshes LTV, bonus, and fees from `asset_config`; the liquidation threshold is keeper-only.
pub fn update_deposit_position(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    asset_config: &AssetConfig,
    caller: &Address,
    feed: &PriceFeed,
    cache: &mut ControllerCache,
) -> AccountPosition {
    let mut position = get_or_create_deposit_position(account, account_id, asset_config, asset);

    // Refresh LTV, liquidation bonus, and liquidation fees from the latest
    // asset config. Do NOT refresh `liquidation_threshold_bps` here: only
    // the keeper path (`update_position_threshold`) propagates threshold
    // changes, and it enforces the 5% HF buffer required to prevent an
    // immediate liquidation from a threshold reduction.
    if position.loan_to_value_bps != asset_config.loan_to_value_bps {
        position.loan_to_value_bps = asset_config.loan_to_value_bps;
    }
    if position.liquidation_bonus_bps != asset_config.liquidation_bonus_bps {
        position.liquidation_bonus_bps = asset_config.liquidation_bonus_bps;
    }
    if position.liquidation_fees_bps != asset_config.liquidation_fees_bps {
        position.liquidation_fees_bps = asset_config.liquidation_fees_bps;
    }

    let market_index =
        update_market_position(env, cache, &mut position, asset, amount, caller, feed);

    // Event (supply uses supply_index_ray). The pool synced indexes and
    // returned the exact market index used for this mutation.
    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("supply"),
            index: market_index.supply_index_ray,
            amount,
            position: position.clone().into(),
            asset_price: Some(feed.price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );

    // Update the in-memory account. `process_supply` writes storage once at
    // the end of the batch.
    update::update_or_remove_position(account, &position);

    position
}

// ---------------------------------------------------------------------------
// update_market_position
// ---------------------------------------------------------------------------

fn update_market_position(
    env: &Env,
    cache: &mut ControllerCache,
    position: &mut AccountPosition,
    asset: &Address,
    amount: i128,
    caller: &Address,
    feed: &PriceFeed,
) -> MarketIndex {
    let pool_addr = cache.cached_pool_address(asset);

    // Transfer caller -> pool before calling supply, using balance-delta
    // accounting, so fee-on-transfer or rebasing tokens cannot inflate the
    // booked amount relative to what the pool actually received.
    let token = soroban_sdk::token::Client::new(env, asset);
    let pool_balance_before = token.balance(&pool_addr);
    token.transfer(caller, &pool_addr, &amount);
    let pool_balance_after = token.balance(&pool_addr);
    let actual_received = pool_balance_after
        .checked_sub(pool_balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AmountMustBePositive));
    if actual_received <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    // Call pool.supply and replace the in-memory position with the updated
    // version. scaled_amount_ray reflects the pool's new supply index.
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let result = pool_client.supply(position, &feed.price_wad, &actual_received);
    *position = result.position;
    result.market_index
}

// ---------------------------------------------------------------------------
// validate_supply_cap
// ---------------------------------------------------------------------------

fn validate_supply_cap(
    env: &Env,
    cache: &mut ControllerCache,
    asset_config: &AssetConfig,
    asset: &Address,
    amount: i128,
    _feed: &PriceFeed,
) {
    if asset_config.supply_cap <= 0 {
        return;
    }
    // Use the synced supply index from the cache rather than the pool's stored
    // (potentially stale) value. `cached_market_index` simulates global_sync
    // forward to the current timestamp so cap enforcement is exact across
    // accrual gaps and same-tx multi-payment loops.
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let sync_data = pool_client.get_sync_data();
    let market_index = cache.cached_market_index(asset);
    let supplied_actual_ray = Ray::from_raw(sync_data.state.supplied_ray)
        .mul(env, Ray::from_raw(market_index.supply_index_ray));
    let current_total = supplied_actual_ray.to_asset(sync_data.params.asset_decimals);
    let total = current_total.saturating_add(amount); // Both in asset decimals.
    if total > asset_config.supply_cap {
        panic_with_error!(env, CollateralError::SupplyCapReached);
    }
}

// ---------------------------------------------------------------------------
// update_position_threshold
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Keeper-driven propagation of updated risk parameters to a specific account's supply position.
/// When `has_risks` is true (threshold tightening), enforces a 5% HF buffer to prevent
/// immediate liquidation after the update.
pub fn update_position_threshold(
    env: &Env,
    account_id: u64,
    asset: &Address,
    has_risks: bool,
    asset_config: &mut AssetConfig,
    controller_addr: &Address,
    feed: &PriceFeed,
    cache: &mut ControllerCache,
) {
    // Load account; no-op when the account is gone (bad-debt cleanup, full exit).
    let mut account = match storage::try_get_account(env, account_id) {
        Some(acct) => acct,
        None => return,
    };

    // No-op when the account has no supply position for this asset.
    let Some(position) = account.supply_positions.get(asset.clone()) else {
        return;
    };

    storage::bump_account(env, account_id);

    // Apply the per-account e-mode override. `ensure_e_mode_not_deprecated`
    // is deliberately NOT called: the keeper must propagate updated
    // thresholds to accounts in deprecated categories so they wind down to
    // base asset params. For deprecated categories with no asset entry,
    // `apply_e_mode_to_asset_config` becomes a no-op.
    let e_mode_category = emode::e_mode_category(env, account.e_mode_category_id);
    let asset_emode_config = cache.cached_emode_asset(account.e_mode_category_id, asset);
    emode::apply_e_mode_to_asset_config(env, asset_config, &e_mode_category, asset_emode_config);

    let mut updated_pos = position;

    if has_risks {
        if updated_pos.liquidation_threshold_bps != asset_config.liquidation_threshold_bps {
            updated_pos.liquidation_threshold_bps = asset_config.liquidation_threshold_bps;
        }
    } else {
        if updated_pos.loan_to_value_bps != asset_config.loan_to_value_bps {
            updated_pos.loan_to_value_bps = asset_config.loan_to_value_bps;
        }
        if updated_pos.liquidation_bonus_bps != asset_config.liquidation_bonus_bps {
            updated_pos.liquidation_bonus_bps = asset_config.liquidation_bonus_bps;
        }
        if updated_pos.liquidation_fees_bps != asset_config.liquidation_fees_bps {
            updated_pos.liquidation_fees_bps = asset_config.liquidation_fees_bps;
        }
    }

    update::store_position(&mut account, &updated_pos);
    storage::set_account(env, account_id, &account);

    // Risky updates can tip a position into liquidation; enforce a 5%
    // safety buffer after the store so the position is not immediately
    // liquidatable.
    if has_risks {
        let hf = helpers::calculate_health_factor(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        if hf < THRESHOLD_UPDATE_MIN_HF {
            panic_with_error!(env, CollateralError::HealthFactorTooLow);
        }
    }

    // Emit a position update event with amount = 0; no deposit or withdraw
    // occurred, only a parameter change.
    let market_index = cache.cached_market_index(asset);
    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("param_upd"),
            index: market_index.supply_index_ray,
            amount: 0,
            position: updated_pos.into(),
            asset_price: Some(feed.price_wad),
            caller: Some(controller_addr.clone()),
            account_attributes: Some((&account).into()),
        },
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::constants::{RAY, WAD};
    use common::types::{
        ExchangeSource, MarketConfig, MarketParams, MarketStatus, OraclePriceFluctuation,
        OracleProviderConfig, OracleType, PositionMode, ReflectorAssetKind,
    };
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{token, Address, Map, Symbol, Vec};

    struct TestSetup {
        env: Env,
        controller: Address,
        asset: Address,
        pool: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths_allowing_non_root_auth();
            env.ledger().set(LedgerInfo {
                timestamp: 1_000,
                protocol_version: 25,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin.clone(),));
            let asset = env
                .register_stellar_asset_contract_v2(admin)
                .address()
                .clone();
            let params = MarketParams {
                max_borrow_rate_ray: 5 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                reserve_factor_bps: 1_000,
                asset_id: asset.clone(),
                asset_decimals: 7,
            };
            let pool = env.register(
                pool::LiquidityPool,
                (controller.clone(), params, controller.clone()),
            );

            Self {
                env,
                controller,
                asset,
                pool,
            }
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
                isolation_debt_ceiling_usd_wad: 0,
                flashloan_fee_bps: 9,
                borrow_cap: i128::MAX,
                supply_cap: i128::MAX,
            }
        }

        fn market_config(&self, asset_config: AssetConfig) -> MarketConfig {
            MarketConfig {
                status: MarketStatus::Active,
                asset_config,
                pool_address: self.pool.clone(),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset.clone(),
                    oracle_type: OracleType::Normal,
                    exchange_source: ExchangeSource::SpotOnly,
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

        fn price_feed(&self) -> PriceFeed {
            PriceFeed {
                price_wad: WAD,
                asset_decimals: 7,
                timestamp: 1_000,
            }
        }

        fn account_with_supply(
            &self,
            owner: Address,
            ltv: i128,
            bonus: i128,
            fees: i128,
        ) -> Account {
            let mut supply_positions = Map::new(&self.env);
            supply_positions.set(
                self.asset.clone(),
                AccountPosition {
                    position_type: AccountPositionType::Deposit,
                    asset: self.asset.clone(),
                    scaled_amount_ray: 1_0000000,
                    account_id: 1,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: bonus,
                    liquidation_fees_bps: fees,
                    loan_to_value_bps: ltv,
                },
            );

            Account {
                owner,
                is_isolated: false,
                e_mode_category_id: 0,
                mode: PositionMode::Normal,
                isolated_asset: None,
                supply_positions,
                borrow_positions: Map::new(&self.env),
            }
        }
    }

    #[test]
    #[should_panic]
    fn test_process_supply_rejects_wrong_owner_for_existing_account() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);
        let caller = Address::generate(&t.env);

        t.as_controller(|| {
            storage::set_account(&t.env, 1, &t.account_with_supply(owner, 7_500, 500, 100));

            let mut assets = Vec::new(&t.env);
            assets.push_back((t.asset.clone(), 1));

            let _ = process_supply(&t.env, &caller, 1, 0, &assets);
        });
    }

    #[test]
    fn test_update_deposit_position_refreshes_risk_parameters_on_existing_position() {
        let t = TestSetup::new();
        let caller = Address::generate(&t.env);
        let mut updated_config = t.asset_config();
        let feed = t.price_feed();

        updated_config.loan_to_value_bps = 7_900;
        updated_config.liquidation_bonus_bps = 650;
        updated_config.liquidation_fees_bps = 175;

        token::StellarAssetClient::new(&t.env, &t.asset).mint(&caller, &2_0000000);

        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(updated_config.clone()));

            let mut account = t.account_with_supply(caller.clone(), 7_500, 500, 100);
            let mut cache = ControllerCache::new(&t.env, true);
            cache.set_price(&t.asset, &feed);

            let position = update_deposit_position(
                &t.env,
                1,
                &mut account,
                &t.asset,
                1_0000000,
                &updated_config,
                &caller,
                &feed,
                &mut cache,
            );

            assert_eq!(position.loan_to_value_bps, 7_900);
            assert_eq!(position.liquidation_bonus_bps, 650);
            assert_eq!(position.liquidation_fees_bps, 175);
        });
    }

    #[test]
    fn test_update_position_threshold_noops_for_missing_account_and_position() {
        let t = TestSetup::new();
        let feed = t.price_feed();
        let mut asset_config = t.asset_config();

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            cache.set_price(&t.asset, &feed);
            storage::set_market_config(&t.env, &t.asset, &t.market_config(asset_config.clone()));

            update_position_threshold(
                &t.env,
                404,
                &t.asset,
                false,
                &mut asset_config,
                &t.controller,
                &feed,
                &mut cache,
            );

            let empty_account = Account {
                owner: Address::generate(&t.env),
                is_isolated: false,
                e_mode_category_id: 0,
                mode: PositionMode::Normal,
                isolated_asset: None,
                supply_positions: Map::new(&t.env),
                borrow_positions: Map::new(&t.env),
            };
            storage::set_account(&t.env, 1, &empty_account);

            update_position_threshold(
                &t.env,
                1,
                &t.asset,
                false,
                &mut asset_config,
                &t.controller,
                &feed,
                &mut cache,
            );

            assert!(storage::get_account(&t.env, 1).supply_positions.is_empty());
        });
    }

    #[test]
    fn test_update_position_threshold_updates_safe_fields() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);
        let feed = t.price_feed();
        let mut updated_config = t.asset_config();

        updated_config.loan_to_value_bps = 7_900;
        updated_config.liquidation_bonus_bps = 650;
        updated_config.liquidation_fees_bps = 175;

        t.as_controller(|| {
            storage::set_market_config(&t.env, &t.asset, &t.market_config(updated_config.clone()));
            storage::set_account(&t.env, 1, &t.account_with_supply(owner, 7_500, 500, 100));

            let mut cache = ControllerCache::new(&t.env, true);
            cache.set_price(&t.asset, &feed);

            update_position_threshold(
                &t.env,
                1,
                &t.asset,
                false,
                &mut updated_config,
                &t.controller,
                &feed,
                &mut cache,
            );

            let updated = storage::get_account(&t.env, 1)
                .supply_positions
                .get(t.asset.clone())
                .unwrap();
            assert_eq!(updated.loan_to_value_bps, 7_900);
            assert_eq!(updated.liquidation_bonus_bps, 650);
            assert_eq!(updated.liquidation_fees_bps, 175);
        });
    }
}
