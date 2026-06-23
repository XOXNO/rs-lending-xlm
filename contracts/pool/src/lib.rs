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
use common::constants::RAY;
// Re-exported for the `tests` submodule's `use super::*`.
#[cfg(test)]
use common::constants::MS_PER_SECOND;
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
    enforce_borrow_cap,
    enforce_supply_cap, now_ms, renew_market_keys, renew_pool_instance, require_nonneg_amount,
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

/// Shared shell for the position-mutating batch endpoints.
///
/// Renews instance TTL once, applies `apply` to each entry in input order, then
/// emits a single market-state batch event and returns the per-entry mutations.
/// Any per-entry panic (cap, reserves, math) reverts the whole call. Per-entry
/// CEI and accounting live in each `*_one` callee.
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

/// Validates `action.amount` is non-negative, loads the market cache accrued to
/// now, and reads the position's current scaled amount. Instance TTL is renewed
/// once per batch by `run_batch`, so this does not renew it.
fn load_position(env: &Env, action: &PoolAction) -> (Cache, Ray, i128) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market_cache(env, &action.asset);
    let scaled = Ray::from(action.position.scaled_amount_ray);
    (cache, scaled, action.amount)
}

/// Accrues a borrow of `amount` into `cache` and the caller's `scaled` position:
/// requires sufficient reserves, enforces the borrow cap, adds the scaled debt,
/// then rejects post-borrow utilization above the market's max.
fn accrue_borrow(env: &Env, cache: &mut Cache, scaled: &mut Ray, amount: i128) {
    cache.require_reserves(amount);
    let scaled_debt = cache.calculate_scaled_borrow(amount);
    enforce_borrow_cap(env, cache, scaled_debt);
    scaled.checked_add_assign(env, scaled_debt);
    cache.borrowed.checked_add_assign(env, scaled_debt);
    utils::require_utilization_below_max(env, cache);
}

fn supply_one(env: &Env, entry: &PoolSupplyEntry) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut scaled, amount) = load_position(env, &entry.action);

    let scaled_amount = cache.calculate_scaled_supply(amount);
    enforce_supply_cap(env, &cache, scaled_amount);

    scaled.checked_add_assign(env, scaled_amount);
    cache.supplied.checked_add_assign(env, scaled_amount);
    // Controller transferred `amount` into the pool before this call.
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

fn repay_one(
    env: &Env,
    payer: &Address,
    action: &PoolAction,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, scaled, amount) = load_position(env, action);

    let (scaled_repay, overpayment) = cache.resolve_repay(amount, scaled);
    let scaled = scaled.checked_sub(env, scaled_repay);
    cache.borrowed.checked_sub_assign(env, scaled_repay);
    // Controller moved `amount` in; the `overpayment` is refunded below.
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
fn verify_flash_repay(
    env: &Env,
    tok: &token::Client,
    pool_addr: &Address,
    expected: i128,
) {
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
            last_timestamp: now_ms(&env),
            cash: 0,
        };
        env.storage()
            .persistent()
            .set(&PoolKey::State(asset.clone()), &state);

        renew_market_keys(&env, &asset);
        events::publish_market_params(&env, asset, params);
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
    fn update_indexes(env: Env, asset: Address) {
        let cache = load_synced_cache(&env, &asset);
        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
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
        cache.credit_cash(amount);

        cache.save();
        events::publish_market_state(&env, cache.market_snapshot());
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

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);
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
            asset,
        } = action;
        let caller = receiver.clone();
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        assert_with_error!(&env, fee <= amount, FlashLoanError::StrategyFeeExceeds);

        let mut cache = load_synced_cache(&env, &asset);
        let mut scaled = Ray::from(position.scaled_amount_ray);
        accrue_borrow(&env, &mut cache, &mut scaled, amount);

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);

        let amount_to_send = amount
            .checked_sub(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        cache.debit_cash(amount_to_send);

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, amount_to_send);
        events::publish_strategy_fee(
            &env,
            asset.clone(),
            amount,
            fee,
            amount_to_send,
        );
        events::publish_market_state(&env, cache.market_snapshot());
        cache.strategy_mutation(scaled, amount, amount_to_send)
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
        events::publish_market_state(&env, cache.market_snapshot());
        cache.position_mutation(Ray::ZERO, 0)
    }

    #[only_owner]
    // Claim burns scaled revenue from revenue and supplied totals, capped by reserves.
    // Solvency is checked before transfer.
    fn claim_revenue(env: Env, asset: Address) -> PoolAmountMutation {
        let mut cache = load_synced_cache(&env, &asset);

        let amount_to_transfer = cache.burn_claimable_revenue();

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
    fn update_params(env: Env, asset: Address, model: InterestRateModel) {
        // Accrue at the old rate model before replacing it.
        let cache = load_synced_cache(&env, &asset);
        cache.save();

        model.verify(&env);
        apply_rate_model(&env, &asset, &model);
        let params = views::load_params(&env, &asset);
        events::publish_market_params(&env, asset, params);
    }

    #[only_owner]
    fn update_caps(env: Env, asset: Address, supply_cap: i128, borrow_cap: i128) {
        let cache = load_synced_cache(&env, &asset);
        cache.save();
        apply_hub_caps(&env, &asset, supply_cap, borrow_cap);
        let params = views::load_params(&env, &asset);
        events::publish_market_params(&env, asset, params);
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
        views::load_sync_data(&env, &asset)
    }

    fn bulk_get_indexes(env: Env, assets: Vec<Address>) -> Vec<MarketIndexRaw> {
        let now = now_ms(&env);
        let mut indexes: Vec<MarketIndexRaw> = Vec::new(&env);
        for asset in assets.iter() {
            let sync = views::load_sync_data(&env, &asset);
            indexes.push_back(MarketIndexRaw::from(&simulate_update_indexes(
                &env, now, &sync,
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
                supply_cap: 0,
                borrow_cap: 0,
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
        };
        let entry2 = PoolSupplyEntry {
            action: make_action(0, 50_000_000, &t.asset),
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
        };
        let _ = client.supply(&vec![&t.env, sup]);

        client.add_rewards(&t.asset, &10_000_000);
        let snap = client.get_sync_data(&t.asset).state;
        assert!(snap.supply_index_ray > RAY);
    }
}

#[cfg(test)]
mod tests;
