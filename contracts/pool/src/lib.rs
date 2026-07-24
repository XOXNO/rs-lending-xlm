#![no_std]

//! Owner-gated liquidity pool: interest, scaled shares, cash. Controller owns
//! solvency and risk; storage keys live in `common::types::PoolKey`.
//! See `docs/reference/invariants.md`.

mod cache;
mod events;
mod interest;
mod utils;
mod views;

#[cfg(test)]
#[path = "../tests/test_support.rs"]
mod test_support;

#[cfg(feature = "certora")]
#[path = "../../../certora/pool/spec/mod.rs"]
pub mod spec;

use common::constants::{RAY, SUPPLY_INDEX_REWARD_CEILING_RAY};
use common::errors::{FlashLoanError, GenericError};
use common::math::fp::{Bps, Ray};
use common::rates::{simulate_update_indexes, supply_index_reward_shortfall, update_supply_index};
use common::types::{
    AccountPositionType, HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw,
    MarketStateSnapshot, PoolAction, PoolAmountMutation, PoolBorrowEntry, PoolKey,
    PoolNetSettleEntry, PoolNetSettleResult, PoolPositionMutation, PoolSeizeEntry, PoolStateRaw,
    PoolStrategyMutation, PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};

use pool_interface::LiquidityPoolInterface;

use soroban_sdk::{
    assert_with_error, contract, contractimpl, contractmeta, panic_with_error, token, Address,
    Bytes, BytesN, Env, IntoVal, Symbol, TryFromVal, Val, Vec,
};

use stellar_access::ownable;
use stellar_macros::only_owner;

use crate::cache::Cache;
use crate::utils::{
    apply_liquidation_fee, apply_rate_model, now_ms, renew_market_keys, renew_pool_instance,
    require_backed_market, require_nonneg_amount, require_positive_amount, require_wasm_receiver,
};

contractmeta!(key = "name", val = "Liquidity Pool");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

fn load_synced_cache(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    renew_pool_instance(env);
    synced_market_cache(env, hub_asset)
}

// Bulk endpoints renew instance TTL once per call.
fn synced_market_cache(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    let mut cache = Cache::load(env, hub_asset);
    interest::global_sync(env, &mut cache);
    cache
}

fn run_batch<E>(
    env: &Env,
    entries: Vec<E>,
    mut apply: impl FnMut(&Env, &E) -> (PoolPositionMutation, MarketStateSnapshot),
) -> Vec<PoolPositionMutation>
where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    renew_pool_instance(env);
    // TODO(ttl): batch-local market cache across duplicate hub_assets to skip
    // repeated load+sync when consecutive legs hit the same market.
    let mut mutations = Vec::new(env);
    let mut snapshots = Vec::new(env);
    for entry in entries.iter() {
        let (mutation, snapshot) = apply(env, &entry);
        mutations.push_back(mutation);
        snapshots.push_back(snapshot);
    }
    events::emit_market_state_batch(env, snapshots);
    mutations
}

fn load_position(env: &Env, action: &PoolAction) -> (Cache, Ray, i128) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market_cache(env, &action.hub_asset);
    let scaled = Ray::from(action.position.scaled_amount);
    (cache, scaled, action.amount)
}

fn position_result(
    cache: &Cache,
    scaled: Ray,
    actual_amount: i128,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    (
        cache.position_mutation(scaled, actual_amount),
        cache.market_snapshot(),
    )
}

fn accrue_borrow(env: &Env, cache: &mut Cache, scaled: &mut Ray, amount: i128) {
    require_positive_amount(env, amount);
    cache.require_reserves(amount);
    let scaled_debt = cache.calculate_scaled_borrow(amount);
    // Defensive free-borrow guard: ceil scaling makes every valid positive
    // amount positive, and this keeps that invariant explicit at the mutation.
    assert_with_error!(
        env,
        scaled_debt.raw() > 0,
        GenericError::BorrowRoundsToZeroShares
    );
    scaled.checked_add_assign(env, scaled_debt);
    cache.borrowed.checked_add_assign(env, scaled_debt);
    utils::require_utilization_below_max(env, cache);
}

fn supply_one(env: &Env, entry: &PoolSupplyEntry) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);
    require_backed_market(env, &cache);

    let scaled_amount = cache.calculate_scaled_supply(amount);
    assert_with_error!(
        env,
        amount == 0 || scaled_amount.raw() > 0,
        GenericError::SupplyRoundsToZeroShares
    );
    scaled.checked_add_assign(env, scaled_amount);
    cache.supplied.checked_add_assign(env, scaled_amount);
    // Controller transferred Token(asset) `amount` into the pool before this call.
    cache.credit_cash(amount);

    cache.save();
    position_result(&cache, scaled, amount)
}

fn borrow_one(
    env: &Env,
    receiver: &Address,
    entry: &PoolBorrowEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (cache, mutation, snapshot) = borrow_accounting(env, entry);

    // CEI: accounting committed before external transfer.
    cache.transfer_out(receiver, mutation.actual_amount);
    (mutation, snapshot)
}

/// Production borrow accounting, split from the SAC transfer so formal rules
/// can verify the persisted transition without assuming external token code.
fn borrow_accounting(
    env: &Env,
    entry: &PoolBorrowEntry,
) -> (Cache, PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);

    accrue_borrow(env, &mut cache, &mut scaled, amount);
    cache.debit_cash(amount);

    cache.save();
    let (mutation, snapshot) = position_result(&cache, scaled, amount);
    (cache, mutation, snapshot)
}

fn withdraw_one(
    env: &Env,
    receiver: &Address,
    is_liquidation: bool,
    entry: &PoolWithdrawEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (cache, mutation, snapshot, net_transfer) = withdraw_accounting(env, is_liquidation, entry);

    // CEI: accounting committed before external transfer.
    cache.transfer_out(receiver, net_transfer);
    (mutation, snapshot)
}

/// Production withdrawal accounting, split from the SAC transfer for direct
/// transition proofs. `actual_amount` remains the gross withdrawal; the fourth
/// return value is the net token transfer after any liquidation fee.
fn withdraw_accounting(
    env: &Env,
    is_liquidation: bool,
    entry: &PoolWithdrawEntry,
) -> (Cache, PoolPositionMutation, MarketStateSnapshot, i128) {
    require_nonneg_amount(env, entry.protocol_fee);
    // Controller maps user amount `0` to this full-withdraw sentinel.
    let (mut cache, scaled, amount) = load_position(env, &entry.action);

    let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(amount, scaled);
    assert_with_error!(
        env,
        gross_amount == 0 || scaled_withdrawal.raw() > 0,
        GenericError::WithdrawRoundsToZeroShares
    );

    let net_transfer = apply_liquidation_fee(
        env,
        &mut cache,
        gross_amount,
        is_liquidation,
        entry.protocol_fee,
    );
    cache.supplied.checked_sub_assign(env, scaled_withdrawal);
    let scaled = scaled.checked_sub(env, scaled_withdrawal);

    cache.require_reserves(net_transfer);
    // User withdrawals cannot leave the pool above max utilization.
    if !is_liquidation {
        utils::require_utilization_below_max(env, &cache);
    }
    utils::require_solvent_withdraw_state(env, &cache);
    cache.debit_cash(net_transfer);

    cache.save();
    let (mutation, snapshot) = position_result(&cache, scaled, gross_amount);
    (cache, mutation, snapshot, net_transfer)
}

fn repay_one(
    env: &Env,
    payer: &Address,
    action: &PoolAction,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (cache, mutation, snapshot, overpayment) = repay_accounting(env, action);

    // CEI: accounting committed before external refund.
    cache.transfer_out(payer, overpayment);
    (mutation, snapshot)
}

/// Production repay accounting, split from the SAC refund for direct proofs.
fn repay_accounting(
    env: &Env,
    action: &PoolAction,
) -> (Cache, PoolPositionMutation, MarketStateSnapshot, i128) {
    let (mut cache, scaled, amount) = load_position(env, action);

    let (scaled_repay, overpayment) = cache.resolve_repay(amount, scaled);
    let net_repay = amount
        .checked_sub(overpayment)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    assert_with_error!(
        env,
        net_repay == 0 || scaled_repay.raw() > 0,
        GenericError::RepayRoundsToZeroShares
    );
    let scaled = scaled.checked_sub(env, scaled_repay);
    cache.borrowed.checked_sub_assign(env, scaled_repay);
    // Controller moved Token(asset) `amount` in; `overpayment` is refunded below.
    cache.credit_cash(net_repay);

    cache.save();
    let (mutation, snapshot) = position_result(&cache, scaled, net_repay);
    (cache, mutation, snapshot, overpayment)
}

// Seize: bad borrow → supply-index write-down (floor); deposit dust → revenue.
// Each entry reloads state so duplicate hub-assets apply sequentially.
fn seize_one(env: &Env, entry: &PoolSeizeEntry) -> MarketStateSnapshot {
    let mut cache = synced_market_cache(env, &entry.hub_asset);

    let scaled = Ray::from(entry.position.scaled_amount);
    match entry.side {
        AccountPositionType::Borrow => {
            // dimensional: seized debt becomes Ray<Token(asset)> bad debt, not scaled shares.
            let current_debt = cache.unscale_borrow_ceil_ray(scaled);
            interest::apply_bad_debt_to_supply_index(&mut cache, current_debt);
            cache.borrowed.checked_sub_assign(env, scaled);
        }
        AccountPositionType::Deposit => {
            cache.revenue.checked_add_assign(env, scaled);
        }
    }

    cache.save();
    cache.market_snapshot()
}

// Caps to debt before withdraw resolution; feeds actual gross (not request)
// into repay so full-close never overpays. Cash invariant: gross out of
// supply equals gross into debt — no transfer, cash unchanged.
fn net_settle_one(
    env: &Env,
    entry: &PoolNetSettleEntry,
) -> (PoolNetSettleResult, MarketStateSnapshot) {
    require_nonneg_amount(env, entry.amount);
    let mut cache = synced_market_cache(env, &entry.hub_asset);

    let supply_scaled = Ray::from(entry.supply_position.scaled_amount);
    let debt_scaled = Ray::from(entry.debt_position.scaled_amount);

    let max_debt = cache.unscale_borrow_ceil(debt_scaled);
    let capped_amount = entry.amount.min(max_debt);

    let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(capped_amount, supply_scaled);
    let (scaled_repay, overpayment) = cache.resolve_repay(gross_amount, debt_scaled);
    assert_with_error!(env, overpayment == 0, GenericError::InternalError);
    assert_with_error!(
        env,
        gross_amount == 0 || (scaled_withdrawal.raw() > 0 && scaled_repay.raw() > 0),
        GenericError::NetSettleRoundsToZeroShares
    );

    cache.supplied.checked_sub_assign(env, scaled_withdrawal);
    cache.borrowed.checked_sub_assign(env, scaled_repay);
    // No credit_cash/debit_cash: the withdrawn amount and the repaid amount
    // are identical (`gross_amount`), so cash is invariant by construction.

    cache.save();
    let supply_scaled_after = supply_scaled.checked_sub(env, scaled_withdrawal);
    let debt_scaled_after = debt_scaled.checked_sub(env, scaled_repay);
    (
        PoolNetSettleResult {
            supply_position: ScaledPositionRaw {
                scaled_amount: supply_scaled_after.raw(),
            },
            debt_position: ScaledPositionRaw {
                scaled_amount: debt_scaled_after.raw(),
            },
            market_index: cache.market_index(),
            settled_amount: gross_amount,
        },
        cache.market_snapshot(),
    )
}

// Loaned-token balance must match expected; else InvalidFlashloanRepay.
fn verify_flash_repay(env: &Env, tok: &token::Client, pool_addr: &Address, expected: i128) {
    assert_with_error!(
        env,
        tok.balance(pool_addr) == expected,
        FlashLoanError::InvalidFlashloanRepay
    );
}

fn pull_flash_repayment(
    env: &Env,
    tok: &token::Client,
    receiver: &Address,
    pool_addr: &Address,
    total: i128,
    expected_after_repay: i128,
) {
    assert_with_error!(
        env,
        tok.allowance(receiver, pool_addr) >= total,
        FlashLoanError::InvalidFlashloanRepay
    );
    tok.transfer_from(pool_addr, receiver, pool_addr, &total);
    verify_flash_repay(env, tok, pool_addr, expected_after_repay);
}

/// Exact balance targets used by the production flash-loan checks.
///
/// Returns `(fee, amount_plus_fee, after_payout, after_repayment)`.
fn flash_repayment_terms(
    env: &Env,
    amount: i128,
    fee_bps: u32,
    pre_balance: i128,
) -> (i128, i128, i128, i128) {
    let fee = Bps::from(i128::from(fee_bps)).flash_loan_fee_on(env, amount);
    let total = amount
        .checked_add(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let expected_after_payout = pre_balance
        .checked_sub(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let expected_after_repay = pre_balance
        .checked_add(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    (fee, total, expected_after_payout, expected_after_repay)
}

/// Books the successful flash-loan fee into the same production cache fields
/// whose token balance was checked by `flash_loan`.
fn book_flash_fee(cache: &mut Cache, fee: i128) {
    let protocol_fee = Ray::from_asset(fee, cache.params.asset_decimals);
    interest::add_protocol_revenue(cache, protocol_fee);
    cache.credit_cash(fee);
}

/// Production strategy accounting, split from the SAC transfer so the debt,
/// cash, fee, and revenue transition is directly verifiable.
fn create_strategy_accounting(
    env: &Env,
    action: PoolAction,
    charge_fee: bool,
) -> (Cache, PoolStrategyMutation, i128) {
    let PoolAction {
        position,
        amount,
        hub_asset,
    } = action;
    require_nonneg_amount(env, amount);

    let mut cache = load_synced_cache(env, &hub_asset);

    // Flash-loan fee is derived pool-side from the market's configured
    // `flashloan_fee` bps; `charge_fee = false` (migration) borrows fee-free.
    let fee = if charge_fee {
        Bps::from(i128::from(cache.params.flashloan_fee)).flash_loan_fee_on(env, amount)
    } else {
        0
    };
    assert_with_error!(env, fee <= amount, FlashLoanError::StrategyFeeExceeds);

    let mut scaled = Ray::from(position.scaled_amount);
    accrue_borrow(env, &mut cache, &mut scaled, amount);

    let protocol_fee = Ray::from_asset(fee, cache.params.asset_decimals);
    interest::add_protocol_revenue(&mut cache, protocol_fee);

    let amount_to_send = amount
        .checked_sub(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    cache.debit_cash(amount_to_send);

    cache.save();
    let mutation = cache.strategy_mutation(scaled, amount, amount_to_send);
    (cache, mutation, fee)
}

/// Production protocol-revenue burn and cash accounting, split from the owner
/// transfer for direct transition proofs.
fn claim_revenue_accounting(env: &Env, hub_asset: HubAssetKey) -> (Cache, PoolAmountMutation) {
    let mut cache = load_synced_cache(env, &hub_asset);

    let amount_to_transfer = cache.burn_claimable_revenue();

    utils::require_utilization_below_max(env, &cache);
    utils::require_solvent_withdraw_state(env, &cache);
    cache.debit_cash(amount_to_transfer);

    cache.save();
    (
        cache,
        PoolAmountMutation {
            actual_amount: amount_to_transfer,
        },
    )
}

#[contract]
pub struct LiquidityPool;

// Soroban constructors cannot be declared in contractclient traits.
#[contractimpl]
impl LiquidityPool {
    /// Sets the contract owner once. Called by the deploying factory.
    ///
    /// # Arguments
    /// * `admin` — address granted owner rights; must be the lending controller.
    ///
    /// # Security Warning
    /// * Runs without authorization and can set the owner only once; the
    ///   deploying factory must pass the trusted controller address.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

// This impl is the pool ABI; signatures must match `LiquidityPoolInterface`.
#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    /// Creates a market with `params` and zeroed state (indexes = `RAY`). Owner
    /// (controller) only.
    ///
    /// # Errors
    /// * `AssetAlreadySupported` — params already exist for `(hub_id, asset)`.
    /// * `AssetDecimalsTooHigh` — `asset_decimals` exceeds `RAY_DECIMALS`.
    /// * `InvalidBorrowParams` — `flashloan_fee` exceeds the protocol cap.
    /// * `BaseRateNegative` / `SlopeNonMonotonic` / `MaxRateBelowBase` /
    ///   `MaxBorrowRateTooHigh` / `InvalidUtilRange` / `OptUtilTooHigh` /
    ///   `InvalidReserveFactor` — rate-model bounds from `InterestRateModel::verify`.
    /// * `MathOverflow` — ledger timestamp to ms overflow.
    ///
    /// # Events
    /// * topics — `["market", "batch_params_update"]`
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        renew_pool_instance(&env);
        params.verify(&env);

        let hub_asset = HubAssetKey {
            hub_id,
            asset: params.asset_id.clone(),
        };
        assert_with_error!(
            &env,
            !env.storage()
                .persistent()
                .has(&PoolKey::Params(hub_asset.clone())),
            GenericError::AssetAlreadySupported
        );

        env.storage()
            .persistent()
            .set(&PoolKey::Params(hub_asset.clone()), &params);

        let state = PoolStateRaw {
            supplied: 0,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: now_ms(&env),
            cash: 0,
        };
        env.storage()
            .persistent()
            .set(&PoolKey::State(hub_asset.clone()), &state);

        renew_market_keys(&env, &hub_asset);
        events::emit_market_params(&env, hub_id, hub_asset.asset, params);
    }

    /// Supplies each entry and mints scaled shares, returning input-ordered
    /// position mutations. Owner (controller) only. The controller must
    /// pre-transfer the tokens.
    ///
    /// # Arguments
    /// * `entries` — one supply leg per entry; amounts must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `AmountMustBePositive` — an entry amount is negative.
    /// * `PoolInsolvent` — aggregate supply claims exceed cash plus debt.
    /// * `SupplyRoundsToZeroShares` — a positive supply mints zero shares at
    ///   the current index.
    /// * `MathOverflow` — scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no account health check; the controller must gate the supply.
    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        // Controller pre-transfers tokens per entry; the first failure reverts all.
        run_batch(&env, entries, supply_one)
    }

    /// Borrows each entry to `receiver`, returning input-ordered position
    /// mutations. Owner (controller) only.
    ///
    /// # Arguments
    /// * `receiver` — proceeds recipient for every leg.
    /// * `entries` — one borrow leg per entry; amounts must be positive.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `AmountMustBePositive` — an entry amount is not strictly positive.
    /// * `BorrowRoundsToZeroShares` — a positive amount mints zero scaled debt
    ///   despite ceil rounding.
    /// * `InsufficientLiquidity` — tracked cash cannot cover the borrow.
    /// * `UtilizationAboveMax` — the borrow pushes utilization past the market cap.
    /// * `MathOverflow` — scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no borrower solvency or collateral check; the owning
    ///   controller must gate the borrow against account health.
    #[only_owner]
    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation> {
        run_batch(&env, entries, |env, entry| {
            borrow_one(env, &receiver, entry)
        })
    }

    /// Withdraws each entry to `receiver`, returning input-ordered position
    /// mutations. Owner (controller) only.
    ///
    /// # Arguments
    /// * `receiver` — recipient of the net withdrawal for every leg.
    /// * `is_liquidation` — applies to the whole call; enables the protocol fee
    ///   and skips the max-utilization check for liquidation seizures.
    /// * `entries` — one withdraw leg per entry; a full-position sentinel amount
    ///   closes the position; `protocol_fee` must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `AmountMustBePositive` — an entry amount or `protocol_fee` is negative.
    /// * `WithdrawLessThanFee` — the liquidation fee exceeds the gross seized amount.
    /// * `WithdrawRoundsToZeroShares` — a positive withdrawal burns zero scaled
    ///   supply despite ceil rounding.
    /// * `InsufficientLiquidity` — tracked cash cannot cover the net transfer.
    /// * `UtilizationAboveMax` — a non-liquidation withdrawal breaches the utilization cap.
    /// * `PoolInsolvent` — the projected state leaves debt with zero supply.
    /// * `MathOverflow` — scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no borrower solvency check; the owning controller must confirm
    ///   the account stays healthy after the withdrawal.
    #[only_owner]
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation> {
        run_batch(&env, entries, |env, entry| {
            withdraw_one(env, &receiver, is_liquidation, entry)
        })
    }

    /// Repays each action and refunds overpayments to `payer`, returning
    /// input-ordered position mutations. Owner (controller) only. The
    /// controller must pre-transfer the repayment tokens.
    ///
    /// # Arguments
    /// * `payer` — recipient of any overpayment refund.
    /// * `actions` — one repay leg per action; amounts must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an action targets a market with no stored state.
    /// * `AmountMustBePositive` — an action amount is negative.
    /// * `RepayRoundsToZeroShares` — a positive applied repayment burns zero
    ///   scaled debt at the current index.
    /// * `MathOverflow` — debt-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        run_batch(&env, actions, |env, action| repay_one(env, &payer, action))
    }

    /// Accrues interest for `hub_asset` and persists indexes. Owner (controller)
    /// only.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `MathOverflow` — accrual or timestamp math overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        renew_pool_instance(&env);
        let mut cache = Cache::load(&env, &hub_asset);
        let dirty = cache.current_timestamp != cache.last_timestamp;
        interest::global_sync(&env, &mut cache);
        if dirty {
            cache.save();
        }
        events::emit_market_state(&env, cache.market_snapshot());
    }

    /// Distributes `amount` to suppliers by growing the supply index. Owner
    /// (controller) only. The controller must pre-transfer the reward tokens.
    ///
    /// # Arguments
    /// * `amount` — reward tokens to distribute; must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `AmountMustBePositive` — `amount` is negative.
    /// * `NoSuppliersToReward` — the market has no scaled supply to receive rewards.
    /// * `MathOverflow` — index or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128) {
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &hub_asset);

        assert_with_error!(
            &env,
            cache.supplied != Ray::ZERO,
            GenericError::NoSuppliersToReward
        );

        let reward = Ray::from_asset(amount, cache.params.asset_decimals);
        let old_supply_index = cache.supply_index;
        cache.supply_index = update_supply_index(&env, cache.supplied, old_supply_index, reward);
        // Cap reward growth so repeated legs cannot pin the index at MAX.
        assert_with_error!(
            &env,
            cache.supply_index.raw() <= SUPPLY_INDEX_REWARD_CEILING_RAY,
            GenericError::SupplyIndexRewardCeiling
        );
        // The virtual-offset shortfall (reward not distributed to suppliers) is
        // booked as protocol revenue instead of stranded as dead reserve, so the
        // full donated reward is accounted (suppliers via index + protocol via
        // revenue) and remains backed by the cash credited below.
        let offset_shortfall = supply_index_reward_shortfall(
            &env,
            cache.supplied,
            old_supply_index,
            cache.supply_index,
            reward,
        );
        interest::add_protocol_revenue(&mut cache, offset_shortfall);
        // Controller transferred Token(asset) reward `amount` into the pool.
        cache.credit_cash(amount);

        cache.save();
        events::emit_market_state(&env, cache.market_snapshot());
    }

    /// Lends `amount` to `receiver`, invokes its `execute_flash_loan` callback,
    /// and pulls back `amount + fee`; the fee (from market `flashloan_fee` bps)
    /// becomes protocol revenue. Owner (controller) only. Returns the fee.
    ///
    /// # Arguments
    /// * `initiator` — forwarded to the receiver callback as the loan originator.
    /// * `receiver` — deployed Wasm contract that receives the loan and repays it.
    /// * `amount` — loaned amount; must be positive.
    /// * `data` — opaque callback payload forwarded to the receiver.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `AmountMustBePositive` — `amount` is not strictly positive.
    /// * `FlashloanNotEnabled` — the market is not flashloanable.
    /// * `InsufficientLiquidity` — tracked cash cannot fund the loan.
    /// * `InvalidFlashloanReceiver` — `receiver` is not a deployed Wasm contract.
    /// * `InvalidFlashloanRepay` — payout, callback, allowance, or repayment leaves
    ///   the pool's loaned-token balance off its expected value.
    /// * `MathOverflow` — loan, fee, or balance accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Bridges an external callback: repayment is enforced solely by loaned-token
    ///   balance and allowance checks that bracket the callback and `transfer_from`,
    ///   so the asset must be a well-behaved SAC.
    #[only_owner]
    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        data: Bytes,
    ) -> i128 {
        require_positive_amount(&env, amount);

        let mut cache = load_synced_cache(&env, &hub_asset);

        // Flash-loan availability and fee are pool-owned: the market must be
        // flashloanable and the fee derives from its `flashloan_fee` bps.
        assert_with_error!(
            &env,
            cache.params.is_flashloanable,
            FlashLoanError::FlashloanNotEnabled
        );
        cache.require_reserves(amount);
        require_wasm_receiver(&env, &receiver);

        // Balance checks bind repayment to the loaned token.
        let pool_addr = env.current_contract_address();
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&pool_addr);
        let (fee, total, expected_after_payout, expected_after_repay) =
            flash_repayment_terms(&env, amount, cache.params.flashloan_fee, pre_balance);

        // Sends the loan, then confirms the pool balance dropped by exactly `amount`.
        tok.transfer(&pool_addr, &receiver, &amount);
        verify_flash_repay(&env, &tok, &pool_addr, expected_after_payout);

        env.invoke_contract::<()>(
            &receiver,
            &Symbol::new(&env, "execute_flash_loan"),
            (
                initiator,
                cache.params.asset_id.clone(),
                amount,
                fee,
                pool_addr.clone(),
                data,
            )
                .into_val(&env),
        );

        // Callback must not change pool loaned-token balance.
        verify_flash_repay(&env, &tok, &pool_addr, expected_after_payout);

        // Receiver approves repayment during callback; pool pulls it.
        pull_flash_repayment(
            &env,
            &tok,
            &receiver,
            &pool_addr,
            total,
            expected_after_repay,
        );

        // Net token effect: pool sends `amount` and receives `amount + fee`, so
        // `cash` increases by `fee`. Direct transfers are balance-checked above.
        book_flash_fee(&mut cache, fee);

        cache.save();
        events::emit_market_state(&env, cache.market_snapshot());
        fee
    }

    /// Opens a strategy borrow: mints scaled debt, books the market flash-loan
    /// fee as protocol revenue when `charge_fee`, and transfers `amount - fee`
    /// to `receiver`. Owner (controller) only.
    ///
    /// # Arguments
    /// * `receiver` — recipient of the net (post-fee) borrowed amount.
    /// * `action` — the strategy borrow leg; amount must be positive.
    /// * `charge_fee` — when true, withhold the market `flashloan_fee` bps;
    ///   when false (migration), borrow fee-free.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for the action's market.
    /// * `AmountMustBePositive` — `amount` is not strictly positive.
    /// * `StrategyFeeExceeds` — computed fee exceeds the borrowed `amount`.
    /// * `BorrowRoundsToZeroShares` — a positive amount mints zero scaled debt.
    /// * `InsufficientLiquidity` — tracked cash cannot fund the borrow.
    /// * `UtilizationAboveMax` — the borrow pushes utilization past the market cap.
    /// * `MathOverflow` — scaled-debt, fee, or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["strategy", "fee"]` (suppressed when fee is zero)
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no borrower solvency check and enforces no spoke borrow cap; the
    ///   owning controller must gate the strategy against account health and caps.
    #[only_owner]
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation {
        let (cache, mutation, fee) = create_strategy_accounting(&env, action, charge_fee);

        // CEI: accounting committed before external transfer.
        cache.transfer_out(&receiver, mutation.amount_received);
        events::emit_strategy_fee(
            &env,
            cache.hub_asset.hub_id,
            cache.hub_asset.asset.clone(),
            mutation.actual_amount,
            fee,
            mutation.amount_received,
        );
        events::emit_market_state(&env, cache.market_snapshot());
        mutation
    }

    /// Seizes positions: borrow legs write down the supply index for bad debt;
    /// deposit legs move dust into revenue. Owner (controller) only. Duplicate
    /// hub-assets in one batch apply sequentially.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `MathOverflow` — bad-debt, revenue, or scaled-total accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        renew_pool_instance(&env);
        // TODO(ttl): batch-local market cache across duplicate hub_assets (same
        // as `run_batch`) so sequential seize legs on one market skip reload.
        let mut snapshots = Vec::new(&env);
        for entry in entries.iter() {
            snapshots.push_back(seize_one(&env, &entry));
        }
        events::emit_market_state_batch(&env, snapshots);
    }

    /// Nets a supply leg against a debt leg on the same hub-asset with zero
    /// token transfer. Settles the lesser of `entry.amount`, supply balance,
    /// and debt owed; leftover collateral stays as supply. Owner (controller)
    /// only.
    ///
    /// # Arguments
    /// * `entry` — hub-asset market plus both legs' current scaled amounts.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — the entry targets a market with no stored state.
    /// * `AmountMustBePositive` — `entry.amount` is negative.
    /// * `InternalError` — the repay leg overpaid (structurally unexpected).
    /// * `NetSettleRoundsToZeroShares` — a positive settlement burns zero scaled
    ///   units on either leg.
    /// * `MathOverflow` — scaled-share accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        renew_pool_instance(&env);
        let (result, snapshot) = net_settle_one(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    /// Burns claimable protocol revenue shares and transfers the floored cash
    /// payout to the owner. Owner (controller) only.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `UtilizationAboveMax` — the claim would leave utilization above the cap.
    /// * `PoolInsolvent` — the projected state leaves debt with zero supply.
    /// * `OwnerNotSet` — claimable amount is positive but no owner is configured.
    /// * `MathOverflow` — revenue or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        let (cache, mutation) = claim_revenue_accounting(&env, hub_asset);

        if mutation.actual_amount > 0 {
            let owner = ownable::get_owner(&env)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
            cache.transfer_out(&owner, mutation.actual_amount);
        }

        events::emit_market_state(&env, cache.market_snapshot());
        mutation
    }

    /// Accrues at the current rate model, then replaces the interest-rate
    /// parameters for `hub_asset`. Owner (controller) only.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `BaseRateNegative` / `SlopeNonMonotonic` / `MaxRateBelowBase` /
    ///   `MaxBorrowRateTooHigh` / `InvalidUtilRange` / `OptUtilTooHigh` /
    ///   `InvalidReserveFactor` — rate-model bounds from `InterestRateModel::verify`.
    /// * `MathOverflow` — accrual or timestamp math overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_params_update"]`
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        // Accrue at the existing rate model before replacing it.
        let cache = load_synced_cache(&env, &hub_asset);
        cache.save();

        model.verify(&env);
        let params = apply_rate_model(&env, &hub_asset, &model);
        events::emit_market_params(&env, hub_asset.hub_id, hub_asset.asset, params);
    }

    /// Replaces the pool contract Wasm with the code at `new_wasm_hash`. Owner
    /// (controller) only.
    ///
    /// # Arguments
    /// * `new_wasm_hash` — hash of already-installed Wasm to run on next invocation.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_pool_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    /// Returns checkpoint utilization in RAY for `hub_asset` (no accrual).
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::capital_utilisation(&env, &hub_asset)
    }

    /// Returns tracked `cash` in asset decimals (not live SAC balance).
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    /// Returns the checkpoint deposit rate in RAY (no accrual).
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    /// Returns the checkpoint borrow rate in RAY (no accrual).
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    /// Returns floored claimable protocol revenue in asset decimals.
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    /// Returns total supplied amount in asset decimals (checkpoint, no accrual).
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    /// Returns total borrowed amount in asset decimals (checkpoint, no accrual).
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    /// Returns seconds since the market last accrued interest.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    /// Returns raw params and accounting state (checkpoint). Prefer
    /// `get_bulk_indexes` for live indexes.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        views::load_sync_data(&env, &hub_asset)
    }

    /// Returns borrow/supply indexes accrued to now for each hub-asset (simulate,
    /// no write).
    fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw> {
        let now = now_ms(&env);
        let mut indexes: Vec<MarketIndexRaw> = Vec::new(&env);
        for hub_asset in hub_assets.iter() {
            let sync = views::load_sync_data(&env, &hub_asset);
            indexes.push_back(MarketIndexRaw::from(&simulate_update_indexes(
                &env, now, &sync,
            )));
        }
        indexes
    }
}

#[cfg(test)]
#[path = "../tests/lib_orchestration.rs"]
mod lib_orchestration_tests;

#[cfg(test)]
#[path = "../tests/flows.rs"]
mod tests;
