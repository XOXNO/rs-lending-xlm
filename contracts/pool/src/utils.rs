use common::constants::{
    MS_PER_SECOND, TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw, PoolKey};
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

/// Current ledger time in milliseconds, panicking on overflow.
pub(crate) fn now_ms(env: &Env) -> u64 {
    env.ledger()
        .timestamp()
        .checked_mul(MS_PER_SECOND)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Renews TTLs for market params/state entries. Both keys must exist.
pub(crate) fn renew_market_keys(env: &Env, hub_asset: &HubAssetKey) {
    let storage = env.storage().persistent();
    storage.extend_ttl(
        &PoolKey::Params(hub_asset.clone()),
        TTL_THRESHOLD_SHARED,
        TTL_BUMP_SHARED,
    );
    storage.extend_ttl(
        &PoolKey::State(hub_asset.clone()),
        TTL_THRESHOLD_SHARED,
        TTL_BUMP_SHARED,
    );
}

/// Rejects a supply that would put current underlying supply above the hub cap.
pub(crate) fn enforce_supply_cap(env: &Env, cache: &Cache, scaled_delta: Ray) {
    let supply_cap = cache.params.supply_cap;
    if !cap_is_enabled(supply_cap) {
        return;
    }

    let cap = Ray::from_asset(supply_cap, cache.params.asset_decimals);
    let next_total = (cache.supplied + scaled_delta).mul(env, cache.supply_index);
    assert_with_error!(
        env,
        next_total <= cap,
        CollateralError::SupplyCapReached
    );
}

/// Rejects a borrow that would put current underlying debt above the hub cap.
pub(crate) fn enforce_borrow_cap(env: &Env, cache: &Cache, scaled_delta: Ray) {
    let borrow_cap = cache.params.borrow_cap;
    if !cap_is_enabled(borrow_cap) {
        return;
    }

    let cap = Ray::from_asset(borrow_cap, cache.params.asset_decimals);
    let next_total = (cache.borrowed + scaled_delta).mul(env, cache.borrow_index);
    assert_with_error!(
        env,
        next_total <= cap,
        CollateralError::BorrowCapReached
    );
}

pub(crate) fn apply_rate_model(env: &Env, hub_asset: &HubAssetKey, m: &InterestRateModel) {
    let key = PoolKey::Params(hub_asset.clone());
    let mut params: MarketParamsRaw = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

    params.max_borrow_rate = m.max_borrow_rate;
    params.base_borrow_rate = m.base_borrow_rate;
    params.slope1 = m.slope1;
    params.slope2 = m.slope2;
    params.slope3 = m.slope3;
    params.mid_utilization = m.mid_utilization;
    params.optimal_utilization = m.optimal_utilization;
    params.max_utilization = m.max_utilization;
    params.reserve_factor = m.reserve_factor;

    env.storage().persistent().set(&key, &params);
}

pub(crate) fn apply_hub_caps(env: &Env, hub_asset: &HubAssetKey, supply_cap: i128, borrow_cap: i128) {
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let key = PoolKey::Params(hub_asset.clone());
    let mut params: MarketParamsRaw = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));
    params.supply_cap = supply_cap;
    params.borrow_cap = borrow_cap;
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
    let fee = Ray::from_asset(protocol_fee, cache.params.asset_decimals);
    interest::add_protocol_revenue(cache, fee);
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
#[path = "../tests/utils.rs"]
mod tests;
