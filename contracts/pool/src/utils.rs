use common::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::types::{InterestRateModel, MarketParamsRaw, PoolKey};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{panic_with_error, Address, Env, Executable, IntoVal, Symbol, Vec};

use crate::cache::Cache;
use crate::interest;

// Rejects negatives at every mutating ABI.
pub(crate) fn require_nonneg_amount(env: &Env, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

pub(crate) fn require_positive_amount(env: &Env, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

pub(crate) fn require_wasm_receiver(env: &Env, receiver: &Address) {
    if !matches!(receiver.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, FlashLoanError::InvalidFlashloanReceiver);
    }
}

pub(crate) fn renew_pool_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

// Returns true if cap is enabled.
pub(crate) fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
}

// Panics if adding scaled_delta would breach supply_cap.
pub(crate) fn enforce_supply_cap(env: &Env, cache: &Cache, scaled_delta: Ray, supply_cap: i128) {
    if !cap_is_enabled(supply_cap) {
        return;
    }

    let cap_ray = Ray::from_asset(supply_cap, cache.params.asset_decimals);
    let next_total = (cache.supplied + scaled_delta).mul(env, cache.supply_index);
    if next_total > cap_ray {
        panic_with_error!(env, CollateralError::SupplyCapReached);
    }
}

// Panics if adding scaled_delta would breach borrow_cap.
pub(crate) fn enforce_borrow_cap(env: &Env, cache: &Cache, scaled_delta: Ray, borrow_cap: i128) {
    if !cap_is_enabled(borrow_cap) {
        return;
    }

    let cap_ray = Ray::from_asset(borrow_cap, cache.params.asset_decimals);
    let next_total = (cache.borrowed + scaled_delta).mul(env, cache.borrow_index);
    if next_total > cap_ray {
        panic_with_error!(env, CollateralError::BorrowCapReached);
    }
}

// Updates rate-model fields in stored MarketParams.
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

// Hard utilization ceiling.
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.supplied == Ray::ZERO {
        return;
    }
    // Cap at RAY (100 %) is the "effectively disabled" sentinel —
    // utilization can exceed RAY only when the pool is insolvent
    // (`borrowed > supplied`), which is a separate failure mode the
    // ceiling can't fix. Production deployments must keep this strictly
    // below RAY (validated at admin time by `InterestRateModel::verify`).
    if cache.params.max_utilization >= Ray::ONE {
        return;
    }
    // Use the index-aware utilization
    // (`borrowed * borrow_index / (supplied * supply_index)`).
    // Comparing scaled values alone misses index drift: after interest
    // accrues, real utilization can exceed the cap while the scaled
    // ratio still looks compliant.
    let utilization = cache.calculate_utilization();
    if utilization > cache.params.max_utilization {
        panic_with_error!(env, CollateralError::UtilizationAboveMax);
    }
}

// Rejects withdrawals leaving supplied == 0 and borrowed > 0.
pub(crate) fn require_solvent_withdraw_state(env: &Env, cache: &Cache) {
    if cache.supplied == Ray::ZERO && cache.borrowed != Ray::ZERO {
        panic_with_error!(env, CollateralError::PoolInsolvent);
    }
}

// Deducts liquidation protocol_fee from gross_amount.
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
    if gross_amount < protocol_fee {
        panic_with_error!(env, CollateralError::WithdrawLessThanFee);
    }
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
