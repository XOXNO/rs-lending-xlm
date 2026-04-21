use common::constants::{BAD_DEBT_USD_THRESHOLD, WAD};
use common::errors::CollateralError;
use common::errors::GenericError;
use common::events::{emit_clean_bad_debt, CleanBadDebtEvent};
use common::fp::{Bps, Ray, Wad};
use common::types::{Account, MarketIndex, PriceFeed};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Map, Vec};

use super::{repay, withdraw};
use crate::cache::ControllerCache;
use crate::positions;
use crate::{helpers, storage, validation};

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

pub fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<(Address, i128)>,
) {
    liquidator.require_auth();
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);

    if debt_payments.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
    for i in 0..debt_payments.len() {
        let (asset, amount) = debt_payments.get(i).unwrap();
        validation::require_asset_supported(env, &asset);
        validation::require_amount_positive(env, amount);
    }

    storage::bump_account(env, account_id);
    let mut account = storage::get_account(env, account_id);
    let mut cache = ControllerCache::new(env, false);

    // Math phase: decide seizure and repayment amounts.
    let (seized_collaterals, repaid_tokens, _refunds, _max_debt_usd, _bonus) =
        execute_liquidation(env, &account, debt_payments, &mut cache);

    if repaid_tokens.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }

    for i in 0..repaid_tokens.len() {
        let (asset, amount, _repaid_usd, feed, _market_index) = repaid_tokens.get(i).unwrap();

        let pool_addr = cache.cached_pool_address(&asset);
        let token = soroban_sdk::token::Client::new(env, &asset);
        token.transfer(liquidator, &pool_addr, &amount);

        let position = account.borrow_positions.get(asset.clone()).unwrap();

        // execute_repayment emits `liq_repay` UpdatePositionEvent internally
        // with the post-mutation account attributes.
        let _result = repay::execute_repayment(
            env,
            &mut account,
            liquidator,
            liquidator,
            symbol_short!("liq_repay"),
            &position,
            feed.price_wad,
            amount,
            &mut cache,
        );
    }

    for i in 0..seized_collaterals.len() {
        let (asset, amount, protocol_fee, feed, _market_index) = seized_collaterals.get(i).unwrap();

        let position = account.supply_positions.get(asset.clone()).unwrap();

        // execute_withdrawal emits `liq_seize` UpdatePositionEvent internally.
        let _result = withdraw::execute_withdrawal(
            env,
            account_id,
            &mut account,
            liquidator,
            liquidator,
            symbol_short!("liq_seize"),
            amount,
            &position,
            true, // is_liquidation
            protocol_fee,
            feed.price_wad,
            &mut cache,
        );
    }

    storage::set_account(env, account_id, &account);

    check_bad_debt_after_liquidation(env, &mut cache, account_id);

    cache.flush_isolated_debts();
}

// ---------------------------------------------------------------------------
// Execution Engine (Math)
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub(crate) fn execute_liquidation(
    env: &Env,
    account: &Account,
    debt_payments: &Vec<(Address, i128)>,
    cache: &mut ControllerCache,
) -> (
    Vec<(Address, i128, i128, PriceFeed, MarketIndex)>, // Seized: (asset, amount, fee, price, index)
    Vec<(Address, i128, i128, PriceFeed, MarketIndex)>, // Repaid: (asset, amount, usd_wad, price, index)
    Vec<(Address, i128)>,                               // Refunds: (asset, amount)
    i128,                                               // final_repayment_usd_wad
    i128,                                               // bonus_bps
) {
    let mut refunds = Vec::new(env);

    let hf = helpers::calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if hf >= WAD {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }

    let (total_collateral, total_debt, weighted_coll) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let (proportion_seized, bonus_params) =
        calculate_seizure_proportions(env, account, total_collateral, weighted_coll, cache);

    let (total_debt_payment_usd, repaid_tokens) =
        calculate_repayment_amounts(env, debt_payments, account, &mut refunds, cache);

    let (max_debt_to_repay_usd, _seizure_usd, bonus) = calculate_liquidation_amounts(
        env,
        total_debt,
        total_collateral,
        weighted_coll,
        proportion_seized,
        bonus_params,
        Wad::from_raw(hf),
        total_debt_payment_usd,
    );

    let seized_collaterals = calculate_seized_collateral(
        env,
        account,
        total_collateral,
        max_debt_to_repay_usd,
        bonus,
        cache,
    );

    let mut final_repayment_tokens = repaid_tokens;
    if total_debt_payment_usd > max_debt_to_repay_usd {
        let excess_usd = total_debt_payment_usd - max_debt_to_repay_usd;
        process_excess_payment(env, &mut final_repayment_tokens, &mut refunds, excess_usd);
    }

    (
        seized_collaterals,
        final_repayment_tokens,
        refunds,
        max_debt_to_repay_usd.raw(),
        bonus.raw(),
    )
}

fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut ControllerCache,
) -> (Wad, (Bps, Bps)) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bonus_params = helpers::get_account_bonus_params(env, cache, &account.supply_positions);

    (proportion_seized, bonus_params)
}

#[allow(clippy::type_complexity)]
fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<(Address, i128)>,
    account: &Account,
    refunds: &mut Vec<(Address, i128)>,
    cache: &mut ControllerCache,
) -> (Wad, Vec<(Address, i128, i128, PriceFeed, MarketIndex)>) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens = Vec::new(env);

    // Merge duplicates.
    let merged = merge_debt_payments(env, raw_payments);

    for i in 0..merged.len() {
        let (asset, amount) = merged.get(i).unwrap();
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index_readonly(&asset);

        let position = account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

        let actual_debt = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.borrow_index_ray))
            .to_asset(feed.asset_decimals);

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back((asset.clone(), excess));
            payment_amount = actual_debt;
        }

        let payment_wad = Wad::from_token(payment_amount, feed.asset_decimals);
        let payment_usd = payment_wad.mul(env, Wad::from_raw(feed.price_wad));

        total_repaid_usd = total_repaid_usd + payment_usd;
        repaid_tokens.push_back((asset, payment_amount, payment_usd.raw(), feed, market_index));
    }

    (total_repaid_usd, repaid_tokens)
}

fn calculate_liquidation_amounts(
    env: &Env,
    total_debt: Wad,
    total_collateral: Wad,
    weighted_coll: Wad,
    proportion_seized: Wad,
    bonus_params: (Bps, Bps),
    hf: Wad,
    total_payment_usd: Wad,
) -> (Wad, Wad, Bps) {
    let (base_bonus, max_bonus) = bonus_params;
    let (ideal_repayment_usd, bonus) = helpers::estimate_liquidation_amount(
        env,
        total_debt,
        weighted_coll,
        hf,
        base_bonus,
        max_bonus,
        proportion_seized,
        total_collateral,
    );

    let final_repayment_usd = total_payment_usd.min(ideal_repayment_usd);
    let seizure_multiplier = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = final_repayment_usd.mul(env, seizure_multiplier);

    (final_repayment_usd, total_seizure_usd, bonus)
}

#[allow(clippy::type_complexity)]
fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment_usd: Wad,
    bonus: Bps,
    cache: &mut ControllerCache,
) -> Vec<(Address, i128, i128, PriceFeed, MarketIndex)> {
    let mut seized = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = repayment_usd.mul(env, one_plus_bonus);

    for (asset, position) in account.supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        if feed.price_wad == 0 {
            continue;
        }

        let asset_config = cache.cached_asset_config(&asset);
        let market_index = cache.cached_market_index_readonly(&asset);

        let actual_ray = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.supply_index_ray));
        let actual_amount = actual_ray.to_asset(feed.asset_decimals);
        let actual_amount_wad = actual_ray.to_wad();
        let asset_value = actual_amount_wad.mul(env, Wad::from_raw(feed.price_wad));

        // share = (asset_value / total_collateral) * total_seizure
        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

        let seizure_amount_wad = seizure_for_asset_usd.div(env, Wad::from_raw(feed.price_wad));
        let seizure_amount = seizure_amount_wad.to_token(feed.asset_decimals);

        if seizure_amount <= 0 {
            continue;
        }

        // Split the seized amount into base and bonus before computing the
        // protocol fee. Floor-divide so `base_amount` is the mathematical
        // lower bound and `bonus_portion` captures any rounding remainder;
        // this keeps `protocol_fee >= bonus * fees_bps / BPS`.
        let capped_amount = seizure_amount.min(actual_amount);
        let base_amount = Wad::from_raw(capped_amount)
            .div_floor(env, one_plus_bonus)
            .raw();
        let bonus_portion = capped_amount - base_amount;
        let protocol_fee =
            Bps::from_raw(asset_config.liquidation_fees_bps).apply_to(env, bonus_portion);

        seized.push_back((asset, capped_amount, protocol_fee, feed, market_index));
    }

    seized
}

#[allow(clippy::type_complexity)]
fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<(Address, i128, i128, PriceFeed, MarketIndex)>,
    refunds: &mut Vec<(Address, i128)>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;

    while remaining_excess_usd > Wad::ZERO && !repaid_tokens.is_empty() {
        let current_index = repaid_tokens.len() - 1;
        let (asset, amount, usd_wad_raw, feed, market_index) =
            repaid_tokens.get(current_index).unwrap();

        let usd = Wad::from_raw(usd_wad_raw);

        if usd > remaining_excess_usd {
            let ratio = remaining_excess_usd.div(env, usd);
            let refund_amount = Wad::from_raw(amount).mul(env, ratio).raw();

            let new_amount = amount - refund_amount;
            // Recompute `new_usd` from `new_amount * price`. Subtracting the
            // excess directly lets the two precision paths drift and leaves
            // the (amount, usd_wad) pair inconsistent for downstream consumers.
            let new_amount_wad = Wad::from_token(new_amount, feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from_raw(feed.price_wad));

            refunds.push_back((asset.clone(), refund_amount));
            repaid_tokens.set(
                current_index,
                (asset, new_amount, new_usd.raw(), feed, market_index),
            );
            remaining_excess_usd = Wad::ZERO;
        } else {
            refunds.push_back((asset, amount));
            repaid_tokens.remove(current_index);
            remaining_excess_usd = remaining_excess_usd - usd;
        }
    }
}

// ---------------------------------------------------------------------------
// Bad debt check and cleanup
// ---------------------------------------------------------------------------

fn check_bad_debt_after_liquidation(env: &Env, cache: &mut ControllerCache, account_id: u64) {
    // Re-load the account to pick up mutated snapshots from storage.
    let account = storage::get_account(env, account_id);

    if account.borrow_positions.is_empty() {
        positions::account::cleanup_account_if_empty(env, &account, account_id);
        return;
    }

    let (total_collateral_usd, total_debt_usd, _) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let bad_debt_threshold = Wad::from_raw(BAD_DEBT_USD_THRESHOLD);
    if total_debt_usd > total_collateral_usd && total_collateral_usd <= bad_debt_threshold {
        execute_bad_debt_cleanup(
            env,
            cache,
            account_id,
            &account,
            total_debt_usd.raw(),
            total_collateral_usd.raw(),
        );
    }
}

pub fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    storage::bump_account(env, account_id);
    let mut cache = ControllerCache::new(env, false);
    let account = storage::get_account(env, account_id);

    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::PositionNotFound);
    }

    let (total_collateral_usd, total_debt_usd, _) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let bad_debt_threshold = Wad::from_raw(BAD_DEBT_USD_THRESHOLD);
    if !(total_debt_usd > total_collateral_usd && total_collateral_usd <= bad_debt_threshold) {
        panic_with_error!(env, CollateralError::CannotCleanBadDebt);
    }

    execute_bad_debt_cleanup(
        env,
        &mut cache,
        account_id,
        &account,
        total_debt_usd.raw(),
        total_collateral_usd.raw(),
    );
}

fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    for asset in account.supply_positions.keys() {
        let position = account.supply_positions.get(asset.clone()).unwrap();
        let pool_addr = cache.cached_pool_address(&position.asset);
        let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
        let feed = cache.cached_price(&position.asset);
        pool_client.seize_position(&position, &feed.price_wad);
    }

    for asset in account.borrow_positions.keys() {
        let position = account.borrow_positions.get(asset.clone()).unwrap();
        repay::clear_position_isolated_debt(env, &position, account, cache);
        let pool_addr = cache.cached_pool_address(&position.asset);
        let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
        let feed = cache.cached_price(&position.asset);
        pool_client.seize_position(&position, &feed.price_wad);
    }

    emit_clean_bad_debt(
        env,
        CleanBadDebtEvent {
            account_id,
            total_borrow_usd_wad: total_debt_usd,
            total_collateral_usd_wad: total_collateral_usd,
        },
    );

    positions::account::remove_account(env, account_id);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn merge_debt_payments(env: &Env, payments: &Vec<(Address, i128)>) -> Vec<(Address, i128)> {
    let mut map: Map<Address, i128> = Map::new(env);
    let mut order: Vec<Address> = Vec::new(env);

    for i in 0..payments.len() {
        let (asset, amount) = payments.get(i).unwrap();
        let prev = map.get(asset.clone()).unwrap_or(0);
        if prev == 0 {
            order.push_back(asset.clone());
        }
        map.set(asset.clone(), prev + amount);
    }

    let mut result: Vec<(Address, i128)> = Vec::new(env);
    for i in 0..order.len() {
        let asset = order.get(i).unwrap();
        let amount = map.get(asset.clone()).unwrap();
        result.push_back((asset, amount));
    }
    result
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::PositionMode;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Map};

    struct TestSetup {
        env: Env,
        controller: Address,
        owner: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin,));
            let owner = Address::generate(&env);

            Self {
                env,
                controller,
                owner,
            }
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn empty_account(&self) -> Account {
            Account {
                owner: self.owner.clone(),
                is_isolated: false,
                e_mode_category_id: 0,
                mode: PositionMode::Normal,
                isolated_asset: None,
                supply_positions: Map::new(&self.env),
                borrow_positions: Map::new(&self.env),
            }
        }
    }

    #[test]
    fn test_check_and_clean_bad_debt_removes_empty_accounts() {
        let t = TestSetup::new();
        let account_id = 1;

        t.as_controller(|| {
            storage::set_account(&t.env, account_id, &t.empty_account());
            let mut cache = ControllerCache::new(&t.env, false);
            check_bad_debt_after_liquidation(&t.env, &mut cache, account_id);
            assert!(storage::try_get_account(&t.env, account_id).is_none());
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #110)")]
    fn test_clean_bad_debt_standalone_rejects_accounts_without_borrows() {
        let t = TestSetup::new();
        t.as_controller(|| {
            storage::set_account(&t.env, 1, &t.empty_account());
            clean_bad_debt_standalone(&t.env, 1);
        });
    }
}
