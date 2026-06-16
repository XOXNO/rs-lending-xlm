use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{InterestRateModel, MarketParamsRaw, PoolKey};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, IntoVal, Symbol, Vec};

use crate::cache::Cache;
use crate::interest;

pub(crate) use common::validation::{
    cap_is_enabled, require_nonneg_amount, require_positive_amount, require_wasm_receiver,
};

pub(crate) fn renew_pool_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Renews TTLs for market params/state entries. Both keys must exist because
/// `extend_ttl` panics on missing keys (soroban-sdk 26.x).
pub(crate) fn renew_market_keys(env: &Env, asset: &Address) {
    let storage = env.storage().persistent();
    storage.extend_ttl(
        &PoolKey::Params(asset.clone()),
        TTL_THRESHOLD_SHARED,
        TTL_BUMP_SHARED,
    );
    storage.extend_ttl(
        &PoolKey::State(asset.clone()),
        TTL_THRESHOLD_SHARED,
        TTL_BUMP_SHARED,
    );
}

/// Rejects a supply that would put current underlying supply above the cap.
pub(crate) fn enforce_supply_cap(env: &Env, cache: &Cache, scaled_delta: Ray, supply_cap: i128) {
    if !cap_is_enabled(supply_cap) {
        return;
    }

    let cap_ray = Ray::from_asset(supply_cap, cache.params.asset_decimals);
    let next_total = (cache.supplied + scaled_delta).mul(env, cache.supply_index);
    assert_with_error!(
        env,
        next_total <= cap_ray,
        CollateralError::SupplyCapReached
    );
}

/// Rejects a borrow that would put current underlying debt above the cap.
pub(crate) fn enforce_borrow_cap(env: &Env, cache: &Cache, scaled_delta: Ray, borrow_cap: i128) {
    if !cap_is_enabled(borrow_cap) {
        return;
    }

    let cap_ray = Ray::from_asset(borrow_cap, cache.params.asset_decimals);
    let next_total = (cache.borrowed + scaled_delta).mul(env, cache.borrow_index);
    assert_with_error!(
        env,
        next_total <= cap_ray,
        CollateralError::BorrowCapReached
    );
}

pub(crate) fn apply_rate_model(env: &Env, asset: &Address, m: &InterestRateModel) {
    let key = PoolKey::Params(asset.clone());
    let mut params: MarketParamsRaw = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

    params.max_borrow_rate_ray = m.max_borrow_rate_ray;
    params.base_borrow_rate_ray = m.base_borrow_rate_ray;
    params.slope1_ray = m.slope1_ray;
    params.slope2_ray = m.slope2_ray;
    params.slope3_ray = m.slope3_ray;
    params.mid_utilization_ray = m.mid_utilization_ray;
    params.optimal_utilization_ray = m.optimal_utilization_ray;
    params.max_utilization_ray = m.max_utilization_ray;
    params.reserve_factor_bps = m.reserve_factor_bps;

    env.storage().persistent().set(&key, &params);
}

/// Rejects post-mutation utilization above the market's max-utilization cap.
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.supplied == Ray::ZERO {
        return;
    }
    // RAY is the disabled sentinel. Utilization exceeds RAY when
    // `borrowed > supplied`; enabled params are validated below RAY.
    if cache.params.max_utilization >= Ray::ONE {
        return;
    }
    // Use index-aware utilization; index drift can push the real ratio above
    // the cap while scaled totals remain below it.
    let utilization = cache.calculate_utilization();
    assert_with_error!(
        env,
        utilization <= cache.params.max_utilization,
        CollateralError::UtilizationAboveMax
    );
}

pub(crate) fn require_solvent_withdraw_state(env: &Env, cache: &Cache) {
    if cache.supplied == Ray::ZERO && cache.borrowed != Ray::ZERO {
        panic_with_error!(env, CollateralError::PoolInsolvent);
    }
}

/// Adds liquidation protocol fee to revenue and returns net collateral transfer.
/// Liquidation fees are minted as scaled revenue, diluting suppliers.
pub(crate) fn apply_liquidation_fee(
    env: &Env,
    cache: &mut Cache,
    gross_amount: i128,
    is_liquidation: bool,
    protocol_fee: i128,
) -> i128 {
    if !is_liquidation || protocol_fee == 0 {
        return gross_amount;
    }
    assert_with_error!(
        env,
        gross_amount >= protocol_fee,
        CollateralError::WithdrawLessThanFee
    );
    let fee_ray = Ray::from_asset(protocol_fee, cache.params.asset_decimals);
    interest::add_protocol_revenue_ray(cache, fee_ray);
    gross_amount
        .checked_sub(protocol_fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

pub(crate) fn authorize_token_transfer_from(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) {
    let pool_addr = env.current_contract_address();
    let token_transfer_from = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: asset.clone(),
            fn_name: Symbol::new(env, "transfer_from"),
            args: (pool_addr, from.clone(), to.clone(), amount).into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    let mut auth_entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    auth_entries.push_back(token_transfer_from);
    env.authorize_as_current_contract(auth_entries);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::cache::Cache;
    use crate::test_support::init_ledger;
    use common::constants::RAY;
    use common::math::fp::Ray;
    use common::types::MarketParamsRaw;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    struct TestSetup {
        env: Env,
        contract: Address,
        params: MarketParamsRaw,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            init_ledger(&env);

            let admin = Address::generate(&env);
            let asset = Address::generate(&env);
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
            let contract = env.register(crate::LiquidityPool, (admin.clone(),));
            crate::LiquidityPoolClient::new(&env, &contract).create_market(&params);

            Self {
                env,
                contract,
                params,
            }
        }

        fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.contract, f)
        }
    }

    fn cache_with(
        env: &Env,
        params: &MarketParamsRaw,
        supplied: i128,
        borrowed: i128,
        cash: i128,
    ) -> Cache {
        Cache {
            env: env.clone(),
            supplied: Ray::from(supplied),
            borrowed: Ray::from(borrowed),
            revenue: Ray::ZERO,
            borrow_index: Ray::from(RAY),
            supply_index: Ray::from(RAY),
            last_timestamp: 0,
            current_timestamp: 1_000_000,
            params: params.into(),
            cash,
        }
    }

    #[test]
    fn test_enforce_supply_cap_disabled_is_noop() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 10 * 10i128.pow(20), 0, 0);
            let delta = Ray::from(10i128.pow(20));
            enforce_supply_cap(&t.env, &cache, delta, 0);
            enforce_supply_cap(&t.env, &cache, delta, i128::MAX);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #")]
    fn test_enforce_supply_cap_rejects_over_cap() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // Use the same scaled convention as other tests in this file (10 * RAY = 10 units @ idx 1)
            let cache = cache_with(&t.env, &t.params, 10 * RAY, 0, 0);
            let delta = Ray::from(3 * RAY);
            // cap 12 units in asset (7 dec) -> from_asset inside will be 12 * RAY in value terms
            enforce_supply_cap(&t.env, &cache, delta, 12 * 10_000_000);
        });
    }

    #[test]
    fn test_enforce_borrow_cap_disabled_is_noop() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 0, 5 * RAY, 0);
            enforce_borrow_cap(&t.env, &cache, Ray::from(RAY), 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #")]
    fn test_enforce_borrow_cap_rejects_over_cap() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 0, 5 * RAY, 0);
            let delta = Ray::from(3 * RAY);
            enforce_borrow_cap(&t.env, &cache, delta, 7 * 10_000_000);
        });
    }

    #[test]
    fn test_require_utilization_below_max_early_returns_when_supplied_zero() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 0, 100 * RAY, 0);
            require_utilization_below_max(&t.env, &cache);
        });
    }

    #[test]
    fn test_require_utilization_below_max_early_returns_when_max_util_ge_one() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut params = t.params.clone();
            params.max_utilization_ray = RAY;
            let cache = cache_with(&t.env, &params, 10 * RAY, 11 * RAY, 0);
            require_utilization_below_max(&t.env, &cache);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #")]
    fn test_require_utilization_below_max_panics_when_above() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 10 * RAY, 10 * RAY, 0);
            require_utilization_below_max(&t.env, &cache);
        });
    }

    #[test]
    fn test_require_solvent_withdraw_state_happy() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 10 * RAY, 5 * RAY, 0);
            require_solvent_withdraw_state(&t.env, &cache);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #")]
    fn test_require_solvent_withdraw_state_panics_when_insolvent() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 0, RAY, 0);
            require_solvent_withdraw_state(&t.env, &cache);
        });
    }

    #[test]
    fn test_apply_liquidation_fee_noop_when_not_liquidation_or_zero_fee() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 50_000_000);
            let out = apply_liquidation_fee(&t.env, &mut cache, 10_000_000, false, 1_000_000);
            assert_eq!(out, 10_000_000);
            let out2 = apply_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 0);
            assert_eq!(out2, 10_000_000);
        });
    }

    #[test]
    fn test_apply_liquidation_fee_accrues_to_revenue_and_reduces_net() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 50_000_000);
            let rev_before = cache.revenue;
            let net = apply_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 2_000_000);
            assert_eq!(net, 8_000_000);
            assert!(cache.revenue > rev_before);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #")]
    fn test_apply_liquidation_fee_rejects_fee_greater_than_gross() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 50_000_000);
            let _ = apply_liquidation_fee(&t.env, &mut cache, 1_000_000, true, 2_000_000);
        });
    }
}
