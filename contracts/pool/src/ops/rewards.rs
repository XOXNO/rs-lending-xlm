//! Reward distribution: hands a donated amount to suppliers through the supply
//! index, against tokens the controller already transferred in.

use common::types::HubAssetKey;
use common::validation::require_nonneg_amount;

use soroban_sdk::Env;

use crate::{events, interest, ops};

/// Distributes `amount` to the market's suppliers.
pub(crate) fn apply(env: &Env, hub_asset: HubAssetKey, amount: i128) {
    require_nonneg_amount(env, amount);
    let mut cache = ops::renewed_market(env, &hub_asset);

    interest::distribute_reward(env, &mut cache, amount);
    // The controller transferred Token(asset) `amount` into the pool before this call.
    cache.credit_cash(amount);

    events::emit_market_state(env, cache.commit());
}
