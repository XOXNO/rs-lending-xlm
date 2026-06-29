#![no_std]
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
    PoolPositionMutation, PoolStateRaw, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry, ScaledPositionRaw,
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
    apply_hub_caps, apply_liquidation_fee, apply_rate_model, authorize_token_transfer_from,
    enforce_borrow_cap, enforce_supply_cap, now_ms, renew_market_keys, renew_pool_instance,
    require_nonneg_amount, require_positive_amount, require_wasm_receiver,
};

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

/// Runs position-mutating batch entries in order.
///
/// Renews instance TTL once, emits one market-state batch event, and
/// returns per-entry mutations. Any entry panic reverts the whole call.
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

/// Validates `action.amount`, loads accrued market cache, and reads scaled amount.
/// Instance TTL is renewed once per batch by `run_batch`.
fn load_position(env: &Env, action: &PoolAction) -> (Cache, Ray, i128) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market_cache(env, &action.hub_asset);
    // dimensional: action position is Ray<Share(asset, side)>; amount is Token(asset).
    let scaled = Ray::from(action.position.scaled_amount);
    (cache, scaled, action.amount)
}

/// Accrues a borrow of `amount` into `cache` and the caller's `scaled` position:
/// requires sufficient reserves, enforces the borrow cap, adds the scaled debt,
/// then rejects post-borrow utilization above the market's max.
fn accrue_borrow(env: &Env, cache: &mut Cache, scaled: &mut Ray, amount: i128) {
    require_positive_amount(env, amount);
    cache.require_reserves(amount);
    // dimensional: Token(asset) borrow amount -> Ray<Share(asset, debt)>.
    let scaled_debt = cache.calculate_scaled_borrow(amount);
    enforce_borrow_cap(env, cache, scaled_debt);
    // dimensional: account debt and market debt totals share the debt-share unit.
    scaled.checked_add_assign(env, scaled_debt);
    cache.borrowed.checked_add_assign(env, scaled_debt);
    utils::require_utilization_below_max(env, cache);
}

fn supply_one(env: &Env, entry: &PoolSupplyEntry) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);

    // dimensional: Token(asset) supply amount -> Ray<Share(asset, supply)>.
    let scaled_amount = cache.calculate_scaled_supply(amount);
    enforce_supply_cap(env, &cache, scaled_amount);

    // dimensional: account supply and market supply totals share the supply-share unit.
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

fn borrow_one(
    env: &Env,
    receiver: &Address,
    entry: &PoolBorrowEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);

    accrue_borrow(env, &mut cache, &mut scaled, amount);
    // dimensional: borrowed Token(asset) leaves tracked cash.
    cache.debit_cash(amount);

    // CEI: snapshot + commit before external call.
    cache.save();
    cache.transfer_out(receiver, amount);
    (
        cache.position_mutation(scaled, amount),
        cache.market_snapshot(),
    )
}

fn withdraw_one(
    env: &Env,
    receiver: &Address,
    is_liquidation: bool,
    entry: &PoolWithdrawEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    require_nonneg_amount(env, entry.protocol_fee);
    // Controller maps user amount `0` to this full-withdraw sentinel.
    let (mut cache, scaled, amount) = load_position(env, &entry.action);

    // dimensional: returns supply shares to burn and Token(asset) gross withdrawal.
    let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(amount, scaled);

    // Build the projected post-withdraw state: accrue the liquidation fee and
    // remove the withdrawn shares from supplied before any check runs.
    // dimensional: gross, protocol fee, and net transfer are Token(asset).
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
    // dimensional: net Token(asset) transfer leaves tracked cash.
    cache.debit_cash(net_transfer);

    // CEI: snapshot + commit before external call.
    cache.save();
    cache.transfer_out(receiver, net_transfer);
    (
        cache.position_mutation(scaled, gross_amount),
        cache.market_snapshot(),
    )
}

fn repay_one(
    env: &Env,
    payer: &Address,
    action: &PoolAction,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, scaled, amount) = load_position(env, action);

    // dimensional: Token(asset) repay amount -> debt shares burned plus Token(asset) refund.
    let (scaled_repay, overpayment) = cache.resolve_repay(amount, scaled);
    let scaled = scaled.checked_sub(env, scaled_repay);
    cache.borrowed.checked_sub_assign(env, scaled_repay);
    // Controller moved Token(asset) `amount` in; `overpayment` is refunded below.
    // dimensional: Token(asset) paid into tracked cash excludes Token(asset) refund.
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

/// Asserts the pool's loaned-token balance equals `expected`, mapping any
/// mismatch to InvalidFlashloanRepay. Brackets the payout and the callback so a
/// receiver cannot retain funds or alter the pool balance.
fn verify_flash_repay(env: &Env, tok: &token::Client, pool_addr: &Address, expected: i128) {
    assert_with_error!(
        env,
        tok.balance(pool_addr) == expected,
        FlashLoanError::InvalidFlashloanRepay
    );
}

/// Settles flash repayment: checks the receiver's allowance, pulls
/// `amount + fee` via `transfer_from`, and asserts the final pool balance.
/// Allowance is checked first so SAC failures map to InvalidFlashloanRepay.
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
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

// This impl is the pool ABI; signatures must match `LiquidityPoolInterface`.
#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
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
            // dimensional: zero Ray<Share> totals, unit Ray<Index> indexes, Token(asset) cash.
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

    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        // Controller pre-transfers tokens per entry; the first failure reverts all.
        run_batch(&env, entries, supply_one)
    }

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

    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        run_batch(&env, actions, |env, action| repay_one(env, &payer, action))
    }

    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        let cache = load_synced_cache(&env, &hub_asset);
        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
    }

    #[only_owner]
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128) {
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &hub_asset);

        assert_with_error!(
            &env,
            cache.supplied != Ray::ZERO,
            GenericError::NoSuppliersToReward
        );

        // dimensional: Token(asset) rewards -> Ray<Token(asset)> for supply-index growth.
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

        // Balance checks prevent repayment with any asset other than the loaned
        // token; balances are per-(token, holder) so other vault assets are inert.
        let pool_addr = env.current_contract_address();
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&pool_addr);
        let expected_after_payout = pre_balance
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        // dimensional: all balances and repayments here are Token(asset).
        let total = amount
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let expected_after_repay = pre_balance
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        // Payout, then verify the receiver did not retain funds.
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

        // The callback must not retain funds or change the pool balance again.
        verify_flash_repay(&env, &tok, &pool_addr, expected_after_payout);

        // Receiver approves `amount + fee` during callback; pull and verify repay.
        pull_flash_repayment(
            &env,
            &tok,
            &cache.params.asset_id,
            &receiver,
            &pool_addr,
            total,
            expected_after_repay,
        );

        // dimensional: Token(asset) fee -> Ray<Token(asset)> -> revenue supply shares.
        let protocol_fee = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue(&mut cache, protocol_fee);
        // Net token effect: pool sends `amount` and receives `amount + fee`, so
        // `cash` increases by `fee`. Direct transfers are balance-checked above.
        cache.credit_cash(fee);

        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
    }

    #[only_owner]
    // Strategy borrow records fee as protocol revenue before transfer; net amount
    // is sent. Utilization and borrow cap use the full amount.
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
        // dimensional: strategy position is Ray<Share(asset, debt)>.
        let mut scaled = Ray::from(position.scaled_amount);
        accrue_borrow(&env, &mut cache, &mut scaled, amount);

        // dimensional: Token(asset) fee -> Ray<Token(asset)> -> revenue supply shares.
        let protocol_fee = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue(&mut cache, protocol_fee);

        // dimensional: Token(asset) sent is borrow amount minus Token(asset) fee.
        let amount_to_send = amount
            .checked_sub(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        cache.debit_cash(amount_to_send);

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, amount_to_send);
        events::publish_strategy_fee(&env, asset.clone(), amount, fee, amount_to_send);
        events::publish_market_state(&env, cache.market_snapshot());
        cache.strategy_mutation(scaled, amount, amount_to_send)
    }

    #[only_owner]
    // Seize: bad borrow debt reduces the supply index, subject to floor.
    // Deposit dust is moved into revenue.
    fn seize_position(
        env: Env,
        hub_asset: HubAssetKey,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation {
        let mut cache = load_synced_cache(&env, &hub_asset);

        let scaled = Ray::from(position.scaled_amount);
        match side {
            AccountPositionType::Borrow => {
                // dimensional: seized debt becomes Ray<Token(asset)> bad debt, not scaled shares.
                let current_debt = cache.unscale_borrow_exact(scaled);
                interest::apply_bad_debt_to_supply_index(&mut cache, current_debt);
                cache.borrowed.checked_sub_assign(&env, scaled);
            }
            AccountPositionType::Deposit => {
                // dimensional: seized deposit dust is Ray<Share(asset, supply)> revenue.
                cache.revenue.checked_add_assign(&env, scaled);
            }
        }

        // The seized position is removed from the controller-owned account map.
        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
        cache.position_mutation(Ray::ZERO, 0)
    }

    #[only_owner]
    // Claim burns scaled revenue from revenue and supplied totals, capped by reserves.
    // Solvency is checked before transfer.
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        let mut cache = load_synced_cache(&env, &hub_asset);

        // dimensional: claim burns Ray<Share(asset, supply)> and returns Token(asset).
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

    #[only_owner]
    fn update_caps(env: Env, hub_asset: HubAssetKey, supply_cap: i128, borrow_cap: i128) {
        let asset = hub_asset.asset.clone();
        let cache = load_synced_cache(&env, &hub_asset);
        cache.save();
        apply_hub_caps(&env, &hub_asset, supply_cap, borrow_cap);
        let params = views::load_params(&env, &hub_asset);
        events::publish_market_params(&env, hub_asset.hub_id, asset, params);
    }

    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_pool_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::capital_utilisation(&env, &hub_asset)
    }

    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        views::load_sync_data(&env, &hub_asset)
    }

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
