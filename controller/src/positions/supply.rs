use common::errors::{CollateralError, GenericError};
use common::events::{emit_update_position, EventAccountPosition, UpdatePositionEvent};
use common::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, EModeCategory, MarketIndex,
    Payment, PriceFeed, POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Vec};
use stellar_macros::{only_role, when_not_paused};

use super::{emode, update};
use crate::cache::ControllerCache;
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

const THRESHOLD_UPDATE_MIN_HF: i128 = 1_050_000_000_000_000_000;

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        assets: Vec<Payment>,
    ) -> u64 {
        process_supply(&env, &caller, account_id, e_mode_category, &assets)
    }

    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_account_threshold(
        env: Env,
        caller: Address,
        asset: Address,
        has_risks: bool,
        account_ids: Vec<u64>,
    ) {
        validation::require_not_flash_loaning(&env);
        validation::require_asset_supported(&env, &asset);

        // Risk-adjusting path: a threshold tightening can tip a position into
        // liquidation, so oracle prices must stay within tight tolerance.
        let mut cache = ControllerCache::new(&env, false);

        let base_config = cache.cached_asset_config(&asset);
        let price_feed = cache.cached_price(&asset);
        let controller_addr = env.current_contract_address();

        for account_id in account_ids {
            let mut account_asset_config = base_config.clone();

            update_position_threshold(
                &env,
                account_id,
                &asset,
                has_risks,
                &mut account_asset_config,
                &controller_addr,
                &price_feed,
                &mut cache,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Endpoint entry
// ---------------------------------------------------------------------------

/// Entry point for supply: validates, creates the account when `account_id == 0`,
/// and processes the deposit batch. Returns the resolved `account_id`.
///
/// Storage I/O: 1 meta read + 1 supply-side read + 1 supply-side write.
/// The borrow side is never touched.
pub fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let (acct_id, mut account) =
        resolve_supply_account(env, caller, account_id, e_mode_category, assets);

    // Note: third-party deposits are intentionally permitted. Supplying to
    // someone else's account can only increase their collateral / health
    // factor — never decrease either. The isolation/e-mode invariants are
    // still enforced per asset via `validate_isolated_collateral` and
    // `validate_e_mode_asset` below.

    let mut cache = ControllerCache::new(env, true); // Supply is risk-decreasing.

    process_deposit(env, caller, acct_id, &mut account, assets, &mut cache);

    // Supply mutates only the supply side; meta and borrow side stay as-is
    // on disk.
    storage::set_supply_positions(env, acct_id, &account.supply_positions);

    acct_id
}

/// Returns the resolved account id and a ready-to-mutate `Account` snapshot.
/// On the new-account path the snapshot is the freshly-created account
/// (skipping the meta + supply-map re-read of what we just wrote). On the
/// existing-account path we read meta + supply-map and leave borrow empty
/// because supply never consumes borrow positions.
fn resolve_supply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> (u64, Account) {
    validation::require_non_empty_payments(env, assets);

    if account_id == 0 {
        utils::create_account_for_first_asset(env, caller, e_mode_category, assets)
    } else {
        let meta = storage::get_account_meta(env, account_id);
        let supply_positions = storage::get_supply_positions(env, account_id);
        let account = storage::account_from_parts(
            meta,
            supply_positions,
            soroban_sdk::Map::new(env),
        );
        (account_id, account)
    }
}

// ---------------------------------------------------------------------------
// process_deposit -- reusable supply flow
// ---------------------------------------------------------------------------

/// Processes a deposit batch on `account`: aggregates duplicate assets,
/// preflights the batch before token movement, then calls the pool once per
/// unique asset using balance-delta accounting.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
) {
    // Fetch the e-mode category once and reuse across every iteration.
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);

    let deposit_plan = utils::aggregate_positive_payments(env, assets);

    prepare_deposit_plan(env, account, &deposit_plan, cache, &e_mode);
    execute_deposit_plan(
        env,
        caller,
        account_id,
        account,
        &deposit_plan,
        cache,
        &e_mode,
    );
}

fn prepare_deposit_plan(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    e_mode: &Option<EModeCategory>,
) {
    validation::validate_bulk_position_limits(env, account, POSITION_TYPE_DEPOSIT, assets);
    validation::validate_bulk_isolation(env, account, assets, cache);

    // Cap is verified post-transfer in `update_market_position` against the
    // balance-delta-credited amount; running a pre-flight cap pass on the
    // input would still mis-account fee-on-transfer assets and adds one
    // `current_supplied_amount` cross-contract read per asset.
    for (asset, _) in assets {
        validation::require_asset_supported(env, &asset);
        validation::require_market_active(env, &asset);

        let asset_config = emode::effective_asset_config(env, account, &asset, cache, e_mode);

        emode::validate_e_mode_asset(env, account.e_mode_category_id, &asset, true);
        emode::ensure_e_mode_compatible_with_asset(env, &asset_config, account.e_mode_category_id);

        if !asset_config.can_supply() {
            panic_with_error!(env, CollateralError::NotCollateral);
        }

        emode::validate_isolated_collateral(env, account, &asset, &asset_config);
    }
}

fn execute_deposit_plan(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    e_mode: &Option<EModeCategory>,
) {
    for (asset, amount_in) in assets {
        let asset_config = emode::effective_asset_config(env, account, &asset, cache, e_mode);
        let feed = cache.cached_price(&asset);

        update_deposit_position(
            env,
            account_id,
            account,
            &asset,
            amount_in,
            &asset_config,
            caller,
            &feed,
            cache,
        );
    }
}

fn get_or_create_deposit_position(
    account: &Account,
    asset_config: &AssetConfig,
    asset: &Address,
) -> AccountPosition {
    account
        .supply_positions
        .get(asset.clone())
        .unwrap_or(AccountPosition {
            scaled_amount_ray: 0,
            liquidation_threshold_bps: asset_config.liquidation_threshold_bps,
            liquidation_bonus_bps: asset_config.liquidation_bonus_bps,
            liquidation_fees_bps: asset_config.liquidation_fees_bps,
            loan_to_value_bps: asset_config.loan_to_value_bps,
        })
}

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
    let mut position = get_or_create_deposit_position(account, asset_config, asset);

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

    let market_update = update_market_position(
        env,
        cache,
        &mut position,
        asset,
        amount,
        asset_config,
        caller,
        feed,
    );

    // Event (supply uses supply_index_ray). The pool synced indexes and
    // returned the exact market index used for this mutation.
    emit_update_position(
        env,
        UpdatePositionEvent {
            action: symbol_short!("supply"),
            index: market_update.market_index.supply_index_ray,
            amount: market_update.credited_amount,
            position: EventAccountPosition::new(
                AccountPositionType::Deposit,
                asset.clone(),
                account_id,
                &position,
            ),
            asset_price: Some(feed.price_wad),
            caller: Some(caller.clone()),
            account_attributes: Some((&*account).into()),
        },
    );

    // Update the in-memory account. `process_supply` writes storage once at
    // the end of the batch.
    update::update_or_remove_position(account, AccountPositionType::Deposit, asset, &position);

    position
}

struct SupplyMarketUpdate {
    market_index: MarketIndex,
    credited_amount: i128,
}

fn update_market_position(
    env: &Env,
    cache: &mut ControllerCache,
    position: &mut AccountPosition,
    asset: &Address,
    amount: i128,
    asset_config: &AssetConfig,
    caller: &Address,
    feed: &PriceFeed,
) -> SupplyMarketUpdate {
    let pool_addr = cache.cached_pool_address(asset);

    let credited_amount = pull_supply_tokens(env, caller, asset, &pool_addr, amount);

    validate_supply_cap(env, cache, asset_config, asset, credited_amount, feed);
    apply_pool_supply(env, asset, position, feed, credited_amount)
}

fn pull_supply_tokens(
    env: &Env,
    caller: &Address,
    asset: &Address,
    pool_addr: &Address,
    amount: i128,
) -> i128 {
    let received = utils::transfer_and_measure_received(
        env,
        asset,
        caller,
        pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    validate_supply_credit(env, amount, received);

    received
}

fn validate_supply_credit(env: &Env, sent: i128, received: i128) {
    if received <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
    // Fee-on-transfer tokens may credit less than sent. A larger balance delta
    // means the token inflated pool reserves during transfer, which this flow
    // does not attribute to the supplier.
    validation::require_credit_not_above_sent(env, sent, received);
}

fn apply_pool_supply(
    env: &Env,
    asset: &Address,
    position: &mut AccountPosition,
    feed: &PriceFeed,
    amount: i128,
) -> SupplyMarketUpdate {
    let result = pool_supply_call(env, asset, position.clone(), feed.price_wad, amount);

    *position = result.position;

    SupplyMarketUpdate {
        market_index: result.market_index,
        credited_amount: amount,
    }
}

crate::summarized!(
    crate::spec::summaries::pool::supply_summary,
    fn pool_supply_call(
        env: &Env,
        asset: &Address,
        position: AccountPosition,
        price_wad: i128,
        amount: i128,
    ) -> common::types::PoolPositionMutation {
        let pool_addr = storage::get_market_config(env, asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr).supply(
            &position,
            &price_wad,
            &amount,
        )
    }
);

fn validate_supply_cap(
    env: &Env,
    cache: &mut ControllerCache,
    asset_config: &AssetConfig,
    asset: &Address,
    amount: i128,
    feed: &PriceFeed,
) {
    if asset_config.supply_cap <= 0 {
        return;
    }
    let current_total = current_supplied_amount(env, cache, asset, feed.asset_decimals);
    let total = current_total
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)); // Both in asset decimals.
    if total > asset_config.supply_cap {
        panic_with_error!(env, CollateralError::SupplyCapReached);
    }
}

fn current_supplied_amount(
    env: &Env,
    cache: &mut ControllerCache,
    asset: &Address,
    asset_decimals: u32,
) -> i128 {
    // Use the synced supply index from the cache rather than the pool's stored
    // (potentially stale) value. `cached_market_index` simulates global_sync
    // forward to the current timestamp so cap enforcement is exact across
    // accrual gaps and same-tx multi-payment loops.
    let sync_data = cache.cached_pool_sync_data(asset);
    let market_index = cache.cached_market_index(asset);
    let supplied_actual_ray = Ray::from_raw(sync_data.state.supplied_ray)
        .mul(env, Ray::from_raw(market_index.supply_index_ray));
    supplied_actual_ray.to_asset(asset_decimals)
}

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
    // No-op when the account is gone (bad-debt cleanup, full exit).
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_supply_positions(env, account_id);

    // No-op when the account has no supply position for this asset.
    let Some(position) = supply_positions.get(asset.clone()) else {
        return;
    };

    // Borrow side is only loaded when we actually need it for the HF gate.
    let borrow_positions = if has_risks {
        storage::get_borrow_positions(env, account_id)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::bump_account(env, account_id);

    // Apply the per-account e-mode override. `ensure_e_mode_not_deprecated`
    // is deliberately NOT called: the keeper must propagate updated
    // thresholds to accounts in deprecated categories so they wind down to
    // base asset params. For deprecated categories with no asset entry,
    // `apply_e_mode_to_asset_config` becomes a no-op.
    let e_mode_category = emode::e_mode_category(env, meta.e_mode_category_id);
    let asset_emode_config = cache.cached_emode_asset(meta.e_mode_category_id, asset);
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

    let mut account = Account {
        owner: meta.owner.clone(),
        is_isolated: meta.is_isolated,
        e_mode_category_id: meta.e_mode_category_id,
        mode: meta.mode,
        isolated_asset: meta.isolated_asset.clone(),
        supply_positions,
        borrow_positions,
    };
    update::update_or_remove_position(
        &mut account,
        AccountPositionType::Deposit,
        asset,
        &updated_pos,
    );

    // Persist only the supply side; borrow stays as-is.
    storage::set_supply_positions(env, account_id, &account.supply_positions);

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
            position: EventAccountPosition::new(
                AccountPositionType::Deposit,
                asset.clone(),
                account_id,
                &updated_pos,
            ),
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
    use crate::helpers::testutils::test_market_config;
    use common::constants::{RAY, WAD};
    use common::types::{MarketParams, PositionMode};
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{token, Address, Map, Vec};

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
                protocol_version: 26,
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
            let pool = env.register(pool::LiquidityPool, (controller.clone(), params));

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
            ltv: u32,
            bonus: u32,
            fees: u32,
        ) -> Account {
            let mut supply_positions = Map::new(&self.env);
            supply_positions.set(
                self.asset.clone(),
                AccountPosition {
                    scaled_amount_ray: 1_0000000,
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
    fn test_validate_supply_credit_allows_exact_and_fee_on_transfer() {
        let env = Env::default();

        validate_supply_credit(&env, 100, 100);
        validate_supply_credit(&env, 100, 99);
    }

    #[test]
    #[should_panic]
    fn test_validate_supply_credit_rejects_zero_credit() {
        let env = Env::default();

        validate_supply_credit(&env, 100, 0);
    }

    #[test]
    #[should_panic]
    fn test_validate_supply_credit_rejects_inflated_credit() {
        let env = Env::default();

        validate_supply_credit(&env, 100, 101);
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
            storage::set_market_config(
                &t.env,
                &t.asset,
                &test_market_config(&t.env, &t.asset, &t.pool, updated_config.clone()),
            );

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
            storage::set_market_config(
                &t.env,
                &t.asset,
                &test_market_config(&t.env, &t.asset, &t.pool, asset_config.clone()),
            );

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
            storage::set_market_config(
                &t.env,
                &t.asset,
                &test_market_config(&t.env, &t.asset, &t.pool, updated_config.clone()),
            );
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
