#![no_std]
mod cache;
mod events;
mod interest;
mod utils;
mod views;

#[cfg(test)]
mod test_support;

#[cfg(feature = "certora")]
#[path = "../../../certora/pool/spec/mod.rs"]
pub mod spec;

use cache::Cache;
use common::constants::{MS_PER_SECOND, RAY};
use common::errors::{FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::rates::{simulate_update_indexes, update_supply_index};
use common::types::{
    AccountPositionType, InterestRateModel, MarketIndexRaw, MarketParamsRaw, MarketStateSnapshot,
    PoolAction, PoolAmountMutation, PoolBorrowEntry, PoolKey, PoolPositionMutation, PoolStateRaw,
    PoolStrategyMutation, PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;
use soroban_sdk::{
    assert_with_error, contract, contractimpl, contractmeta, panic_with_error, token, Address,
    Bytes, BytesN, Env, IntoVal, Symbol, Vec,
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
    apply_liquidation_fee, apply_rate_model, authorize_token_transfer_from, enforce_borrow_cap,
    enforce_supply_cap, renew_market_keys, renew_pool_instance, require_nonneg_amount,
    require_positive_amount, require_wasm_receiver,
};

fn load_synced_cache(env: &Env, asset: &Address) -> Cache {
    renew_pool_instance(env);
    synced_market_cache(env, asset)
}

/// Market cache accrued to now without instance-TTL renewal.
/// Bulk endpoints renew instance TTL once per call.
fn synced_market_cache(env: &Env, asset: &Address) -> Cache {
    let mut cache = Cache::load(env, asset);
    interest::global_sync(env, &mut cache);
    cache
}

fn supply_one(env: &Env, entry: &PoolSupplyEntry) -> (PoolPositionMutation, MarketStateSnapshot) {
    let PoolAction {
        position,
        amount,
        asset,
    } = entry.action.clone();
    require_nonneg_amount(env, amount);
    let mut cache = synced_market_cache(env, &asset);

    let mut scaled = Ray::from(position.scaled_amount_ray);
    let scaled_amount = cache.calculate_scaled_supply(amount);

    enforce_supply_cap(env, &cache, scaled_amount, entry.supply_cap);

    scaled.checked_add_assign(env, scaled_amount);
    cache.supplied.checked_add_assign(env, scaled_amount);
    // Controller transferred `amount` into the pool before this call.
    cache.cash = cache
        .cash
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

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
    let PoolAction {
        position,
        amount,
        asset,
    } = entry.action.clone();
    require_nonneg_amount(env, amount);
    let mut cache = synced_market_cache(env, &asset);

    cache.require_reserves(amount);

    let mut scaled = Ray::from(position.scaled_amount_ray);
    let scaled_debt = cache.calculate_scaled_borrow(amount);

    enforce_borrow_cap(env, &cache, scaled_debt, entry.borrow_cap);

    scaled.checked_add_assign(env, scaled_debt);
    cache.borrowed.checked_add_assign(env, scaled_debt);
    // Borrow cannot leave the pool above its max-utilization cap.
    utils::require_utilization_below_max(env, &cache);
    cache.cash = cache
        .cash
        .checked_sub(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

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
    let PoolAction {
        position,
        amount,
        asset,
    } = entry.action.clone();
    // Controller maps user amount `0` to this full-withdraw sentinel.
    require_nonneg_amount(env, amount);
    require_nonneg_amount(env, entry.protocol_fee);
    let mut cache = synced_market_cache(env, &asset);

    let mut scaled = Ray::from(position.scaled_amount_ray);
    let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(amount, scaled);

    let net_transfer = apply_liquidation_fee(
        env,
        &mut cache,
        gross_amount,
        is_liquidation,
        entry.protocol_fee,
    );

    cache.require_reserves(net_transfer);

    cache.supplied.checked_sub_assign(env, scaled_withdrawal);
    scaled = scaled.checked_sub(env, scaled_withdrawal);

    // User withdrawals cannot leave the pool above max utilization.
    if !is_liquidation {
        utils::require_utilization_below_max(env, &cache);
    }
    utils::require_solvent_withdraw_state(env, &cache);
    cache.cash = cache
        .cash
        .checked_sub(net_transfer)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

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
    let PoolAction {
        position,
        amount,
        asset,
    } = action.clone();
    require_nonneg_amount(env, amount);
    let mut cache = synced_market_cache(env, &asset);

    let mut scaled = Ray::from(position.scaled_amount_ray);
    let (scaled_repay, overpayment) = cache.resolve_repay(amount, scaled);

    scaled = scaled.checked_sub(env, scaled_repay);
    cache.borrowed.checked_sub_assign(env, scaled_repay);
    // Controller moved `amount` in; the `overpayment` is refunded below.
    let net_repay = amount
        .checked_sub(overpayment)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    cache.cash = cache
        .cash
        .checked_add(net_repay)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    // CEI: snapshot + commit before external call.
    cache.save();
    cache.transfer_out(payer, overpayment);
    (
        cache.position_mutation(scaled, net_repay),
        cache.market_snapshot(),
    )
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
    fn create_market(env: Env, params: MarketParamsRaw) {
        renew_pool_instance(&env);
        params.verify(&env);

        let asset = params.asset_id.clone();
        assert_with_error!(
            &env,
            !env.storage()
                .persistent()
                .has(&PoolKey::Params(asset.clone())),
            GenericError::AssetAlreadySupported
        );

        env.storage()
            .persistent()
            .set(&PoolKey::Params(asset.clone()), &params);

        let state = PoolStateRaw {
            supplied_ray: 0,
            borrowed_ray: 0,
            revenue_ray: 0,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: env
                .ledger()
                .timestamp()
                .checked_mul(MS_PER_SECOND)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow)),
            cash: 0,
        };
        env.storage()
            .persistent()
            .set(&PoolKey::State(asset.clone()), &state);

        renew_market_keys(&env, &asset);
        let mut updates = Vec::new(&env);
        updates.push_back(events::PoolMarketParamsEvent { asset, params });
        events::publish_market_params_batch(&env, updates);
    }

    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        // Bulk entries sync independently; controller pre-transfers tokens for each entry.
        // First failure (cap, etc.) reverts the entire call.
        renew_pool_instance(&env);
        let mut out: Vec<PoolPositionMutation> = Vec::new(&env);
        let mut snapshots = Vec::new(&env);
        for entry in entries.iter() {
            let (mutation, snapshot) = supply_one(&env, &entry);
            out.push_back(mutation);
            snapshots.push_back(snapshot);
        }
        events::publish_market_state_batch(&env, snapshots);
        out
    }

    #[only_owner]
    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation> {
        renew_pool_instance(&env);
        let mut out: Vec<PoolPositionMutation> = Vec::new(&env);
        let mut snapshots = Vec::new(&env);
        for entry in entries.iter() {
            let (mutation, snapshot) = borrow_one(&env, &receiver, &entry);
            out.push_back(mutation);
            snapshots.push_back(snapshot);
        }
        events::publish_market_state_batch(&env, snapshots);
        out
    }

    #[only_owner]
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation> {
        renew_pool_instance(&env);
        let mut out: Vec<PoolPositionMutation> = Vec::new(&env);
        let mut snapshots = Vec::new(&env);
        for entry in entries.iter() {
            let (mutation, snapshot) = withdraw_one(&env, &receiver, is_liquidation, &entry);
            out.push_back(mutation);
            snapshots.push_back(snapshot);
        }
        events::publish_market_state_batch(&env, snapshots);
        out
    }

    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        renew_pool_instance(&env);
        let mut out: Vec<PoolPositionMutation> = Vec::new(&env);
        let mut snapshots = Vec::new(&env);
        for action in actions.iter() {
            let (mutation, snapshot) = repay_one(&env, &payer, &action);
            out.push_back(mutation);
            snapshots.push_back(snapshot);
        }
        events::publish_market_state_batch(&env, snapshots);
        out
    }

    #[only_owner]
    fn update_indexes(env: Env, asset: Address) {
        let cache = load_synced_cache(&env, &asset);
        cache.save();
        let mut snapshots = Vec::new(&env);
        snapshots.push_back(cache.market_snapshot());
        events::publish_market_state_batch(&env, snapshots);
    }

    #[only_owner]
    fn add_rewards(env: Env, asset: Address, amount: i128) {
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &asset);

        assert_with_error!(
            &env,
            cache.supplied != Ray::ZERO,
            GenericError::NoSuppliersToReward
        );

        let amount_ray = Ray::from_asset(amount, cache.params.asset_decimals);
        cache.supply_index =
            update_supply_index(&env, cache.supplied, cache.supply_index, amount_ray);
        // Controller transferred `amount` of reward tokens into the pool.
        cache.cash = cache
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        cache.save();
        let mut snapshots = Vec::new(&env);
        snapshots.push_back(cache.market_snapshot());
        events::publish_market_state_batch(&env, snapshots);
    }

    #[only_owner]
    // Flash loan safety: balance and allowance checks bracket callback and transfer_from.
    // CEI: state, including revenue, is committed before the external invoke.
    // Repayment is checked against the loaned token balance.
    fn flash_loan(
        env: Env,
        asset: Address,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) {
        require_positive_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        let mut cache = load_synced_cache(&env, &asset);

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
        let total = amount
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let expected_after_repay = pre_balance
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        tok.transfer(&pool_addr, &receiver, &amount);

        assert_with_error!(
            &env,
            tok.balance(&pool_addr) == expected_after_payout,
            FlashLoanError::InvalidFlashloanRepay
        );

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
        assert_with_error!(
            &env,
            tok.balance(&pool_addr) == expected_after_payout,
            FlashLoanError::InvalidFlashloanRepay
        );

        // Receiver approves `amount + fee` during callback. Check allowance before
        // transfer_from so SAC failures map to InvalidFlashloanRepay (#402).
        assert_with_error!(
            &env,
            tok.allowance(&receiver, &pool_addr) >= total,
            FlashLoanError::InvalidFlashloanRepay
        );
        authorize_token_transfer_from(&env, &cache.params.asset_id, &receiver, &pool_addr, total);
        tok.transfer_from(&pool_addr, &receiver, &pool_addr, &total);

        assert_with_error!(
            &env,
            tok.balance(&pool_addr) == expected_after_repay,
            FlashLoanError::InvalidFlashloanRepay
        );

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);
        // Net token effect: pool sends `amount` and receives `amount + fee`, so
        // `cash` increases by `fee`. Direct transfers are balance-checked above.
        cache.cash = cache
            .cash
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        cache.save();
        let mut snapshots = Vec::new(&env);
        snapshots.push_back(cache.market_snapshot());
        events::publish_market_state_batch(&env, snapshots);
    }

    #[only_owner]
    // Strategy borrow records fee as protocol revenue before transfer; net amount
    // is sent. Utilization and borrow cap use the full amount.
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation {
        let PoolAction {
            position,
            amount,
            asset,
        } = action;
        let caller = receiver;
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        assert_with_error!(&env, fee <= amount, FlashLoanError::StrategyFeeExceeds);

        let mut cache = load_synced_cache(&env, &asset);
        cache.require_reserves(amount);

        let mut scaled = Ray::from(position.scaled_amount_ray);
        let scaled_debt = cache.calculate_scaled_borrow(amount);

        enforce_borrow_cap(&env, &cache, scaled_debt, borrow_cap);

        scaled.checked_add_assign(&env, scaled_debt);
        cache.borrowed.checked_add_assign(&env, scaled_debt);
        // Strategy debt cannot leave the pool above max utilization.
        utils::require_utilization_below_max(&env, &cache);

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);

        let amount_to_send = amount
            .checked_sub(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        cache.cash = cache
            .cash
            .checked_sub(amount_to_send)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, amount_to_send);
        let mutation = cache.strategy_mutation(scaled, amount, amount_to_send);
        let mut snapshots = Vec::new(&env);
        snapshots.push_back(cache.market_snapshot());
        events::publish_market_state_batch(&env, snapshots);
        mutation
    }

    #[only_owner]
    // Seize: bad borrow debt reduces the supply index, subject to floor.
    // Deposit dust is moved into revenue.
    fn seize_position(
        env: Env,
        asset: Address,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation {
        let mut cache = load_synced_cache(&env, &asset);

        let scaled = Ray::from(position.scaled_amount_ray);
        match side {
            AccountPositionType::Borrow => {
                let current_debt_ray = cache.unscale_borrow_ray(scaled);
                interest::apply_bad_debt_to_supply_index(&mut cache, current_debt_ray);
                cache.borrowed.checked_sub_assign(&env, scaled);
            }
            AccountPositionType::Deposit => {
                cache.revenue.checked_add_assign(&env, scaled);
            }
        }

        // The seized position is removed from the controller-owned account map.
        cache.save();
        let mutation = cache.position_mutation(Ray::ZERO, 0);
        let mut snapshots = Vec::new(&env);
        snapshots.push_back(cache.market_snapshot());
        events::publish_market_state_batch(&env, snapshots);
        mutation
    }

    #[only_owner]
    // Claim burns scaled revenue from revenue and supplied totals, capped by reserves.
    // Solvency is checked before transfer.
    fn claim_revenue(env: Env, asset: Address) -> PoolAmountMutation {
        let mut cache = load_synced_cache(&env, &asset);

        assert_with_error!(&env, cache.revenue >= Ray::ZERO, GenericError::MathOverflow);

        let amount_to_transfer = cache.burn_claimable_revenue();

        utils::require_solvent_withdraw_state(&env, &cache);
        cache.cash = cache
            .cash
            .checked_sub(amount_to_transfer)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        // CEI: commit state before external call.
        cache.save();

        if amount_to_transfer > 0 {
            let owner = ownable::get_owner(&env)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
            cache.transfer_out(&owner, amount_to_transfer);
        }

        let mutation = cache.amount_mutation(amount_to_transfer);
        let mut snapshots = Vec::new(&env);
        snapshots.push_back(cache.market_snapshot());
        events::publish_market_state_batch(&env, snapshots);
        mutation
    }

    #[only_owner]
    fn update_params(env: Env, asset: Address, model: InterestRateModel) {
        // Accrue at the old rate model before replacing it.
        let cache = load_synced_cache(&env, &asset);
        cache.save();

        model.verify(&env);
        apply_rate_model(&env, &asset, &model);
        let params = views::load_params(&env, &asset);
        let mut updates = Vec::new(&env);
        updates.push_back(events::PoolMarketParamsEvent { asset, params });
        events::publish_market_params_batch(&env, updates);
    }

    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_pool_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    fn capital_utilisation(env: Env, asset: Address) -> i128 {
        views::capital_utilisation(&env, &asset)
    }

    fn reserves(env: Env, asset: Address) -> i128 {
        views::reserves(&env, &asset)
    }

    fn deposit_rate(env: Env, asset: Address) -> i128 {
        views::deposit_rate(&env, &asset)
    }

    fn borrow_rate(env: Env, asset: Address) -> i128 {
        views::borrow_rate(&env, &asset)
    }

    fn protocol_revenue(env: Env, asset: Address) -> i128 {
        views::protocol_revenue(&env, &asset)
    }

    fn supplied_amount(env: Env, asset: Address) -> i128 {
        views::supplied_amount(&env, &asset)
    }

    fn borrowed_amount(env: Env, asset: Address) -> i128 {
        views::borrowed_amount(&env, &asset)
    }

    fn delta_time(env: Env, asset: Address) -> u64 {
        views::delta_time(&env, &asset)
    }

    fn get_sync_data(env: Env, asset: Address) -> PoolSyncData {
        PoolSyncData {
            params: views::load_params(&env, &asset),
            state: views::load_state(&env, &asset),
        }
    }

    fn bulk_get_indexes(env: Env, assets: Vec<Address>) -> Vec<MarketIndexRaw> {
        let now_ms = env
            .ledger()
            .timestamp()
            .checked_mul(MS_PER_SECOND)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let mut indexes: Vec<MarketIndexRaw> = Vec::new(&env);
        for asset in assets.iter() {
            let sync = PoolSyncData {
                params: views::load_params(&env, &asset),
                state: views::load_state(&env, &asset),
            };
            indexes.push_back(MarketIndexRaw::from(&simulate_update_indexes(
                &env, now_ms, &sync,
            )));
        }
        indexes
    }
}

#[cfg(test)]
mod lib_orchestration_tests {
    extern crate std;

    use crate::test_support::init_ledger;
    use crate::{LiquidityPool, LiquidityPoolClient};
    use common::constants::RAY;
    use common::types::{MarketParamsRaw, PoolAction, PoolSupplyEntry, ScaledPositionRaw};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{token, vec, Address, Env};

    struct TestSetup {
        env: Env,
        contract: Address,
        asset: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            init_ledger(&env);

            let admin = Address::generate(&env);
            let asset = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address();
            let params = MarketParamsRaw {
                max_borrow_rate_ray: 2 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                max_utilization_ray: RAY * 95 / 100,
                reserve_factor_bps: 1_000,
                asset_id: asset.clone(),
                asset_decimals: 7,
            };
            let contract = env.register(LiquidityPool, (admin.clone(),));
            LiquidityPoolClient::new(&env, &contract).create_market(&params);

            // Seed liquidity for repay/overpay scenarios.
            let tok_admin = token::StellarAssetClient::new(&env, &asset);
            tok_admin.mint(&contract, &1_000_000_000);

            Self {
                env,
                contract,
                asset,
            }
        }

        fn client(&self) -> LiquidityPoolClient<'_> {
            LiquidityPoolClient::new(&self.env, &self.contract)
        }
    }

    fn make_action(position_scaled: i128, amount: i128, asset: &Address) -> PoolAction {
        PoolAction {
            position: ScaledPositionRaw {
                scaled_amount_ray: position_scaled,
            },
            amount,
            asset: asset.clone(),
        }
    }

    #[test]
    fn test_bulk_supply_returns_input_ordered_mutations() {
        let t = TestSetup::new();
        let client = t.client();
        // Call through the client; output order follows the *_one path.
        let entry1 = PoolSupplyEntry {
            action: make_action(0, 100_000_000, &t.asset),
            supply_cap: 0,
        };
        let entry2 = PoolSupplyEntry {
            action: make_action(0, 50_000_000, &t.asset),
            supply_cap: 0,
        };
        let results = client.supply(&vec![&t.env, entry1, entry2]);
        assert_eq!(results.len(), 2);
        assert_eq!(results.get(0).unwrap().actual_amount, 100_000_000);
        assert_eq!(results.get(1).unwrap().actual_amount, 50_000_000);
    }

    #[test]
    fn test_add_rewards_emits_snapshot_and_increases_supply_index() {
        let t = TestSetup::new();
        let client = t.client();
        // Supply first so there are suppliers to reward.
        let sup = PoolSupplyEntry {
            action: make_action(0, 100_000_000, &t.asset),
            supply_cap: 0,
        };
        let _ = client.supply(&vec![&t.env, sup]);

        client.add_rewards(&t.asset, &10_000_000);
        let snap = client.get_sync_data(&t.asset).state;
        assert!(snap.supply_index_ray > RAY);
    }
}

#[cfg(test)]
mod tests;
