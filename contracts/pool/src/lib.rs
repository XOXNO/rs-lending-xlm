#![no_std]

//! Liquidity pool contract. Owns per-market interest accrual, scaled
//! supply/debt accounting, and reserve (`cash`) tracking. Every mutating
//! entrypoint is owner-gated: the controller is the sole caller and supplies
//! all account solvency and risk checks the pool itself does not perform.

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

use cache::Cache;
use common::constants::RAY;
use common::errors::{FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::rates::{simulate_update_indexes, update_supply_index};
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

contractmeta!(key = "name", val = "Liquidity Pool");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

use stellar_access::ownable;
use stellar_macros::only_owner;

use utils::{
    apply_liquidation_fee, apply_rate_model, authorize_token_transfer_from, now_ms,
    renew_market_keys, renew_pool_instance, require_nonneg_amount, require_positive_amount,
    require_wasm_receiver,
};

/// Renews the instance TTL, then loads and interest-syncs the market cache.
fn load_synced_cache(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    renew_pool_instance(env);
    synced_market_cache(env, hub_asset)
}

/// Accrued market cache without instance-TTL renewal.
/// Bulk endpoints renew instance TTL once per call.
fn synced_market_cache(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    let mut cache = Cache::load(env, hub_asset);
    interest::global_sync(env, &mut cache);
    cache
}

/// Runs ordered position mutations and emits one market-state batch.
fn run_batch<E>(
    env: &Env,
    entries: Vec<E>,
    mut apply: impl FnMut(&Env, &E) -> (PoolPositionMutation, MarketStateSnapshot),
) -> Vec<PoolPositionMutation>
where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    renew_pool_instance(env);
    let mut mutations = Vec::new(env);
    let mut snapshots = Vec::new(env);
    for entry in entries.iter() {
        let (mutation, snapshot) = apply(env, &entry);
        mutations.push_back(mutation);
        snapshots.push_back(snapshot);
    }
    events::publish_market_state_batch(env, snapshots);
    mutations
}

/// Loads accrued market cache and action position.
fn load_position(env: &Env, action: &PoolAction) -> (Cache, Ray, i128) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market_cache(env, &action.hub_asset);
    let scaled = Ray::from(action.position.scaled_amount);
    (cache, scaled, action.amount)
}

/// Accrues borrow into caller and market debt.
fn accrue_borrow(env: &Env, cache: &mut Cache, scaled: &mut Ray, amount: i128) {
    require_positive_amount(env, amount);
    cache.require_reserves(amount);
    let scaled_debt = cache.calculate_scaled_borrow(amount);
    scaled.checked_add_assign(env, scaled_debt);
    cache.borrowed.checked_add_assign(env, scaled_debt);
    utils::require_utilization_below_max(env, cache);
}

/// Credits one supply leg as scaled shares and tracked cash.
fn supply_one(env: &Env, entry: &PoolSupplyEntry) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);

    let scaled_amount = cache.calculate_scaled_supply(amount);
    scaled.checked_add_assign(env, scaled_amount);
    cache.supplied.checked_add_assign(env, scaled_amount);
    // Controller transferred Token(asset) `amount` into the pool before this call.
    cache.credit_cash(amount);

    cache.save();
    (
        cache.position_mutation(scaled, amount),
        cache.market_snapshot(),
    )
}

/// Accrues one borrow leg into market debt and transfers the proceeds.
fn borrow_one(
    env: &Env,
    receiver: &Address,
    entry: &PoolBorrowEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);

    accrue_borrow(env, &mut cache, &mut scaled, amount);
    cache.debit_cash(amount);

    // CEI: snapshot + commit before external call.
    cache.save();
    cache.transfer_out(receiver, amount);
    (
        cache.position_mutation(scaled, amount),
        cache.market_snapshot(),
    )
}

/// Burns one withdraw leg's supply shares (applying any liquidation fee) and
/// transfers the net amount to `receiver`.
fn withdraw_one(
    env: &Env,
    receiver: &Address,
    is_liquidation: bool,
    entry: &PoolWithdrawEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    require_nonneg_amount(env, entry.protocol_fee);
    // Controller maps user amount `0` to this full-withdraw sentinel.
    let (mut cache, scaled, amount) = load_position(env, &entry.action);

    let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(amount, scaled);

    // Build the projected post-withdraw state: accrue the liquidation fee and
    // remove the withdrawn shares from supplied before any check runs.
    let net_transfer = apply_liquidation_fee(
        env,
        &mut cache,
        gross_amount,
        is_liquidation,
        entry.protocol_fee,
    );
    cache.supplied.checked_sub_assign(env, scaled_withdrawal);
    let scaled = scaled.checked_sub(env, scaled_withdrawal);

    // Validate the projected state before committing or transferring.
    cache.require_reserves(net_transfer);
    // User withdrawals cannot leave the pool above max utilization.
    if !is_liquidation {
        utils::require_utilization_below_max(env, &cache);
    }
    utils::require_solvent_withdraw_state(env, &cache);
    cache.debit_cash(net_transfer);

    // CEI: snapshot + commit before external call.
    cache.save();
    cache.transfer_out(receiver, net_transfer);
    (
        cache.position_mutation(scaled, gross_amount),
        cache.market_snapshot(),
    )
}

/// Burns one repay leg's debt shares and refunds any overpayment to `payer`.
fn repay_one(
    env: &Env,
    payer: &Address,
    action: &PoolAction,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, scaled, amount) = load_position(env, action);

    let (scaled_repay, overpayment) = cache.resolve_repay(amount, scaled);
    let scaled = scaled.checked_sub(env, scaled_repay);
    cache.borrowed.checked_sub_assign(env, scaled_repay);
    // Controller moved Token(asset) `amount` in; `overpayment` is refunded below.
    let net_repay = amount
        .checked_sub(overpayment)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    cache.credit_cash(net_repay);

    // CEI: snapshot + commit before external call.
    cache.save();
    cache.transfer_out(payer, overpayment);
    (
        cache.position_mutation(scaled, net_repay),
        cache.market_snapshot(),
    )
}

/// Seize: bad borrow debt reduces the supply index, subject to floor.
/// Deposit dust is moved into revenue. Each entry reloads persisted state, so
/// a batch with duplicate hub-assets applies sequentially.
fn seize_one(env: &Env, entry: &PoolSeizeEntry) -> MarketStateSnapshot {
    let mut cache = synced_market_cache(env, &entry.hub_asset);

    let scaled = Ray::from(entry.position.scaled_amount);
    match entry.side {
        AccountPositionType::Borrow => {
            // dimensional: seized debt becomes Ray<Token(asset)> bad debt, not scaled shares.
            let current_debt = cache.unscale_borrow_exact(scaled);
            interest::apply_bad_debt_to_supply_index(&mut cache, current_debt);
            cache.borrowed.checked_sub_assign(env, scaled);
        }
        AccountPositionType::Deposit => {
            cache.revenue.checked_add_assign(env, scaled);
        }
    }

    // The seized position is removed from the controller-owned account map.
    cache.save();
    cache.market_snapshot()
}

/// Nets one supply leg against one debt leg on the same hub-asset. Caps the
/// settled amount to the debt owed *before* resolving the withdrawal, then
/// feeds the withdrawal's actual gross amount (not the request) into the
/// repay resolution. That ordering guarantees `resolve_repay`'s own
/// full-close branch never triggers here — overpayment is always exactly
/// zero — so no transfer is ever needed for a leftover: `supplied - borrowed`
/// (== cash) never moves, because the same real amount that left supply is
/// exactly what settled the debt.
fn net_settle_one(
    env: &Env,
    entry: &PoolNetSettleEntry,
) -> (PoolNetSettleResult, MarketStateSnapshot) {
    require_nonneg_amount(env, entry.amount);
    let mut cache = load_synced_cache(env, &entry.hub_asset);

    let supply_scaled = Ray::from(entry.supply_position.scaled_amount);
    let debt_scaled = Ray::from(entry.debt_position.scaled_amount);

    let max_debt = cache.unscale_borrow_ceil(debt_scaled);
    let capped_amount = entry.amount.min(max_debt);

    let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(capped_amount, supply_scaled);
    let (scaled_repay, overpayment) = cache.resolve_repay(gross_amount, debt_scaled);
    assert_with_error!(env, overpayment == 0, GenericError::InternalError);

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

/// Checks loaned-token balance; mismatches map to InvalidFlashloanRepay.
fn verify_flash_repay(env: &Env, tok: &token::Client, pool_addr: &Address, expected: i128) {
    assert_with_error!(
        env,
        tok.balance(pool_addr) == expected,
        FlashLoanError::InvalidFlashloanRepay
    );
}

/// Pulls flash-loan repayment from receiver.
fn pull_flash_repayment(
    env: &Env,
    tok: &token::Client,
    asset_id: &Address,
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
    authorize_token_transfer_from(env, asset_id, receiver, pool_addr, total);
    tok.transfer_from(pool_addr, receiver, pool_addr, &total);
    verify_flash_repay(env, tok, pool_addr, expected_after_repay);
}

#[contract]
pub struct LiquidityPool;

// Soroban constructors cannot be declared in contractclient traits.
#[contractimpl]
impl LiquidityPool {
    /// Sets `admin` as the one-time pool owner at deploy.
    ///
    /// # Arguments
    /// * `admin` - the address granted owner rights; must be the lending controller.
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
    /// Creates the `(hub_id, asset)` market with fresh RAY indexes and zeroed
    /// accounting.
    ///
    /// # Arguments
    /// * `params` - validated market params; `asset_id` becomes the market asset.
    ///
    /// # Errors
    /// * `AssetAlreadySupported` - a market already exists for this hub-asset.
    /// * Param validation: `AssetDecimalsTooHigh`, `InvalidBorrowParams`,
    ///   `BaseRateNegative`, `SlopeNonMonotonic`, `MaxRateBelowBase`,
    ///   `MaxBorrowRateTooHigh`, `InvalidUtilRange`, `OptUtilTooHigh`, or
    ///   `InvalidReserveFactor`.
    /// * `MathOverflow` - ledger timestamp scaling overflows.
    ///
    /// # Events
    /// * A market-params update carrying the new market configuration.
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        renew_pool_instance(&env);
        params.verify(&env);

        let asset = params.asset_id.clone();
        let hub_asset = HubAssetKey {
            hub_id,
            asset: asset.clone(),
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
        events::publish_market_params(&env, hub_asset.hub_id, asset, params);
    }

    /// Credits each supply entry as scaled shares and returns the input-ordered
    /// position mutations. The controller pre-transfers the tokens.
    ///
    /// # Arguments
    /// * `entries` - one supply leg per entry; amounts must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - an entry targets a market with no stored state.
    /// * `AmountMustBePositive` - an entry amount is negative.
    /// * `MathOverflow` - scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * A market-state batch summarizing each mutated market.
    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        // Controller pre-transfers tokens per entry; the first failure reverts all.
        run_batch(&env, entries, supply_one)
    }

    /// Accrues each borrow leg into market debt and transfers the proceeds to
    /// `receiver`, returning the input-ordered position mutations.
    ///
    /// # Arguments
    /// * `receiver` - proceeds recipient for every leg.
    /// * `entries` - one borrow leg per entry; amounts must be positive.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - an entry targets a market with no stored state.
    /// * `AmountMustBePositive` - an entry amount is not strictly positive.
    /// * `InsufficientLiquidity` - tracked reserves cannot cover the borrow.
    /// * `UtilizationAboveMax` - the borrow pushes utilization past the market cap.
    /// * `MathOverflow` - scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * A market-state batch summarizing each mutated market.
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

    /// Burns supply shares for each leg and transfers the net amount to
    /// `receiver`, returning the input-ordered position mutations.
    ///
    /// # Arguments
    /// * `receiver` - recipient of the net withdrawal for every leg.
    /// * `is_liquidation` - applies to the whole call; enables the protocol fee
    ///   and skips the max-utilization check for liquidation seizures.
    /// * `entries` - one withdraw leg per entry; a full-position sentinel amount
    ///   closes the position, `protocol_fee` must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - an entry targets a market with no stored state.
    /// * `AmountMustBePositive` - an entry amount or `protocol_fee` is negative.
    /// * `WithdrawLessThanFee` - the liquidation fee exceeds the gross seized amount.
    /// * `InsufficientLiquidity` - tracked reserves cannot cover the net transfer.
    /// * `UtilizationAboveMax` - a non-liquidation withdrawal breaches the utilization cap.
    /// * `PoolInsolvent` - the projected state leaves debt with zero supply.
    /// * `MathOverflow` - scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * A market-state batch summarizing each mutated market.
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

    /// Burns debt shares for each action and refunds any overpayment to `payer`,
    /// returning the input-ordered position mutations. The controller pre-transfers
    /// the repayment tokens.
    ///
    /// # Arguments
    /// * `payer` - recipient of any overpayment refund.
    /// * `actions` - one repay leg per action; amounts must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - an action targets a market with no stored state.
    /// * `AmountMustBePositive` - an action amount is negative.
    /// * `MathOverflow` - debt-share or cash accounting overflows.
    ///
    /// # Events
    /// * A market-state batch summarizing each mutated market.
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        run_batch(&env, actions, |env, action| repay_one(env, &payer, action))
    }

    /// Accrues and persists the market's borrow/supply indexes to the current
    /// ledger time.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - interest accrual or timestamp scaling overflows.
    ///
    /// # Events
    /// * A market-state update carrying the accrued indexes.
    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        let cache = load_synced_cache(&env, &hub_asset);
        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
    }

    /// Distributes `amount` to suppliers by growing the supply index. The
    /// controller pre-transfers the reward tokens.
    ///
    /// # Arguments
    /// * `amount` - reward tokens to distribute; must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `AmountMustBePositive` - `amount` is negative.
    /// * `NoSuppliersToReward` - the market has no scaled supply to receive rewards.
    /// * `MathOverflow` - index or cash accounting overflows.
    ///
    /// # Events
    /// * A market-state update carrying the grown supply index.
    #[only_owner]
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128) {
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &hub_asset);

        assert_with_error!(
            &env,
            cache.supplied != Ray::ZERO,
            GenericError::NoSuppliersToReward
        );

        cache.supply_index = update_supply_index(
            &env,
            cache.supplied,
            cache.supply_index,
            Ray::from_asset(amount, cache.params.asset_decimals),
        );
        // Controller transferred Token(asset) reward `amount` into the pool.
        cache.credit_cash(amount);

        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
    }

    /// Lends `amount` to `receiver`, invokes its `execute_flash_loan` callback,
    /// and pulls back `amount + fee`; the fee becomes protocol revenue.
    ///
    /// # Arguments
    /// * `initiator` - forwarded to the receiver callback as the loan originator.
    /// * `receiver` - deployed Wasm contract that receives the loan and repays it.
    /// * `amount` - loaned amount; must be positive.
    /// * `fee` - repayment premium; must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `AmountMustBePositive` - `amount` is not positive or `fee` is negative.
    /// * `InsufficientLiquidity` - tracked reserves cannot fund the loan.
    /// * `InvalidFlashloanReceiver` - `receiver` is not a deployed Wasm contract.
    /// * `InvalidFlashloanRepay` - the payout, callback, allowance, or repayment
    ///   leaves the pool's loaned-token balance off its expected value.
    /// * `MathOverflow` - loan/fee/balance accounting overflows.
    ///
    /// # Events
    /// * A market-state update carrying the fee added to revenue.
    ///
    /// # Security Warning
    /// * Bridges an external callback: repayment is enforced solely by loaned-token
    ///   balance and allowance checks that bracket the callback and `transfer_from`,
    ///   so the asset must be a well-behaved SAC.
    #[only_owner]
    // Flash loan safety: balance and allowance checks bracket callback and transfer_from.
    // State and revenue are saved after callback repayment passes balance checks.
    // Repayment is checked against the loaned token balance.
    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) {
        require_positive_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        let mut cache = load_synced_cache(&env, &hub_asset);

        cache.require_reserves(amount);
        require_wasm_receiver(&env, &receiver);

        // Balance checks bind repayment to the loaned token.
        let pool_addr = env.current_contract_address();
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&pool_addr);
        let expected_after_payout = pre_balance
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let total = amount
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let expected_after_repay = pre_balance
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

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
            &cache.params.asset_id,
            &receiver,
            &pool_addr,
            total,
            expected_after_repay,
        );

        let protocol_fee = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue(&mut cache, protocol_fee);
        // Net token effect: pool sends `amount` and receives `amount + fee`, so
        // `cash` increases by `fee`. Direct transfers are balance-checked above.
        cache.credit_cash(fee);

        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
    }

    /// Opens strategy debt for the full `amount`, records `fee` as protocol
    /// revenue, and transfers `amount - fee` to `receiver`.
    ///
    /// # Arguments
    /// * `receiver` - recipient of the net (post-fee) borrowed amount.
    /// * `action` - the strategy borrow leg; amount must be non-negative.
    /// * `fee` - protocol fee withheld from the transfer; must be non-negative
    ///   and at most `amount`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for the action's market.
    /// * `AmountMustBePositive` - `amount` or `fee` is negative.
    /// * `StrategyFeeExceeds` - `fee` exceeds the borrowed `amount`.
    /// * `InsufficientLiquidity` - tracked reserves cannot fund the borrow.
    /// * `UtilizationAboveMax` - the borrow pushes utilization past the market cap.
    /// * `MathOverflow` - scaled-debt, fee, or cash accounting overflows.
    ///
    /// # Events
    /// * A strategy-fee event (for a non-zero fee) and a market-state update.
    ///
    /// # Security Warning
    /// * Performs no borrower solvency check and enforces no spoke borrow cap; the
    ///   owning controller must gate the strategy against account health and caps.
    #[only_owner]
    // Strategy borrow records fee as protocol revenue before transfer; net amount
    // is sent. The pool's own utilization check runs against the full pre-fee
    // amount; the returned scaled delta (also full-amount-based) later feeds
    // the controller's spoke borrow-cap check.
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        fee: i128,
    ) -> PoolStrategyMutation {
        let PoolAction {
            position,
            amount,
            hub_asset,
        } = action;
        let asset = hub_asset.asset.clone();
        let caller = receiver.clone();
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        assert_with_error!(&env, fee <= amount, FlashLoanError::StrategyFeeExceeds);

        let mut cache = load_synced_cache(&env, &hub_asset);
        let mut scaled = Ray::from(position.scaled_amount);
        accrue_borrow(&env, &mut cache, &mut scaled, amount);

        let protocol_fee = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue(&mut cache, protocol_fee);

        let amount_to_send = amount
            .checked_sub(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        cache.debit_cash(amount_to_send);

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, amount_to_send);
        events::publish_strategy_fee(
            &env,
            hub_asset.hub_id,
            asset.clone(),
            amount,
            fee,
            amount_to_send,
        );
        events::publish_market_state(&env, cache.market_snapshot());
        cache.strategy_mutation(scaled, amount, amount_to_send)
    }

    /// Removes seized positions: borrow legs socialize as bad debt into the
    /// supply index, deposit legs move dust into revenue. Duplicate hub-assets
    /// in one batch apply sequentially.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - an entry targets a market with no stored state.
    /// * `MathOverflow` - bad-debt, revenue, or scaled-total accounting overflows.
    ///
    /// # Events
    /// * A market-state batch summarizing each mutated market.
    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        renew_pool_instance(&env);
        let mut snapshots = Vec::new(&env);
        for entry in entries.iter() {
            snapshots.push_back(seize_one(&env, &entry));
        }
        events::publish_market_state_batch(&env, snapshots);
    }

    /// Nets a supply leg against a debt leg on the same hub-asset with zero
    /// token transfer. Settles the lesser of `entry.amount`, the supply
    /// balance, and the debt owed; any leftover collateral beyond outstanding
    /// debt is left untouched as supply.
    ///
    /// # Arguments
    /// * `entry` - the hub-asset market plus both legs' current scaled amounts.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - the entry targets a market with no stored state.
    /// * `AmountMustBePositive` - `entry.amount` is negative.
    /// * `InternalError` - the repay leg overpaid, which should be structurally
    ///   impossible given the debt-first capping — surfaces a math bug rather
    ///   than silently mismatching the two legs.
    /// * `MathOverflow` - scaled-share accounting overflows.
    ///
    /// # Events
    /// * A market-state update carrying the settled indexes.
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        renew_pool_instance(&env);
        let (result, snapshot) = net_settle_one(&env, &entry);
        events::publish_market_state(&env, snapshot);
        result
    }

    /// Burns accrued protocol revenue (capped by live reserves) and transfers the
    /// asset amount to the pool owner.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `UtilizationAboveMax` - the claim would breach the utilization cap.
    /// * `PoolInsolvent` - the projected state leaves debt with zero supply.
    /// * `OwnerNotSet` - the pool has no owner to receive the revenue.
    /// * `MathOverflow` - revenue-burn or cash accounting overflows.
    ///
    /// # Events
    /// * A market-state update reflecting the burned revenue and reduced cash.
    #[only_owner]
    // Claim burns scaled revenue from revenue and supplied totals, capped by reserves.
    // Solvency is checked before transfer.
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        let mut cache = load_synced_cache(&env, &hub_asset);

        let amount_to_transfer = cache.burn_claimable_revenue();

        utils::require_utilization_below_max(&env, &cache);
        utils::require_solvent_withdraw_state(&env, &cache);
        cache.debit_cash(amount_to_transfer);

        // CEI: commit state before external call.
        cache.save();

        if amount_to_transfer > 0 {
            let owner = ownable::get_owner(&env)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
            cache.transfer_out(&owner, amount_to_transfer);
        }

        events::publish_market_state(&env, cache.market_snapshot());
        cache.amount_mutation(amount_to_transfer)
    }

    /// Accrues interest at the current model, then replaces the market's
    /// interest-rate model with a validated `model`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * Rate-model validation: `BaseRateNegative`, `SlopeNonMonotonic`,
    ///   `MaxRateBelowBase`, `MaxBorrowRateTooHigh`, `InvalidUtilRange`,
    ///   `OptUtilTooHigh`, or `InvalidReserveFactor`.
    /// * `MathOverflow` - interest accrual overflows.
    ///
    /// # Events
    /// * A market-params update carrying the new rate model.
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        let asset = hub_asset.asset.clone();
        // Accrue at the existing rate model before replacing it.
        let cache = load_synced_cache(&env, &hub_asset);
        cache.save();

        model.verify(&env);
        apply_rate_model(&env, &hub_asset, &model);
        let params = views::load_params(&env, &hub_asset);
        events::publish_market_params(&env, hub_asset.hub_id, asset, params);
    }

    /// Replaces the pool contract Wasm with the code at `new_wasm_hash`.
    ///
    /// # Arguments
    /// * `new_wasm_hash` - hash of already-installed Wasm to run on next invocation.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_pool_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    /// Reads the market's capital-utilization ratio (RAY) from the last
    /// checkpoint, without accruing interest.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - utilization math overflows.
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::capital_utilisation(&env, &hub_asset)
    }

    /// Reads available reserves (accounted `cash`, in asset decimals); direct
    /// token donations are excluded.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    /// Reads the current per-millisecond deposit rate (RAY) without accruing
    /// interest.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - rate math overflows.
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    /// Reads the current per-millisecond borrow rate (RAY) without accruing
    /// interest.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - rate math overflows.
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    /// Reads accrued protocol revenue in asset decimals without accruing interest.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - unscale math overflows.
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    /// Reads total supplied in asset decimals without accruing interest.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - unscale math overflows.
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    /// Reads total borrowed in asset decimals without accruing interest.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - unscale math overflows.
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    /// Reads milliseconds elapsed since the market's last accrual checkpoint.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    /// * `MathOverflow` - timestamp scaling overflows.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    /// Reads raw params and accounting state for one market, without accruing.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - no stored state for `hub_asset`.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        views::load_sync_data(&env, &hub_asset)
    }

    /// Reads borrow/supply indexes accrued to the current ledger time for each
    /// requested market, index-aligned with `hub_assets`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - a requested market has no stored state.
    /// * `MathOverflow` - accrual or timestamp scaling overflows.
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
