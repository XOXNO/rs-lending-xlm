use common::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{InterestRateModel, MarketParamsRaw, PoolKey};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, IntoVal, Symbol, Vec};

use crate::cache::Cache;
use crate::interest;

pub(crate) use common::validation::{
    require_nonneg_amount, require_positive_amount, require_wasm_receiver,
};

pub(crate) fn renew_pool_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

pub(crate) fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
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

pub(crate) fn apply_rate_model(env: &Env, m: &InterestRateModel) {
    let mut params: MarketParamsRaw = env
        .storage()
        .instance()
        .get(&PoolKey::Params)
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

    env.storage().instance().set(&PoolKey::Params, &params);
}

/// Rejects post-mutation utilization above the market's max-utilization cap.
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.supplied == Ray::ZERO {
        return;
    }
    // Cap at RAY (100%) is the "disabled" sentinel; utilization only exceeds RAY
    // when the pool is insolvent (`borrowed > supplied`), a failure mode this
    // ceiling can't fix. Prod must keep this < RAY (admin-validated by `InterestRateModel::verify`).
    if cache.params.max_utilization >= Ray::ONE {
        return;
    }
    // Index-aware utilization (`borrowed * borrow_index / (supplied * supply_index)`):
    // comparing scaled values alone misses index drift, so accrued interest could
    // push real utilization past the cap while the scaled ratio still looks compliant.
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
    gross_amount - protocol_fee
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
