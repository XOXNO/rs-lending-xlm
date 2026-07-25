//! Reconcile leg: realigns tracked cash after an out-of-band token loss
//! (issuer clawback) and socializes the shortfall through the supply index.

use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketStateSnapshot};

use soroban_sdk::{token, Env};

use crate::{events, interest, ops, storage};

/// Reads the live SAC balance, reconciles against it, and publishes the result.
pub(crate) fn apply(env: &Env, hub_asset: HubAssetKey) {
    storage::renew_instance(env);

    // Only the asset id is needed to read the balance, and accrual cannot move
    // tokens, so the balance is read before the market is loaded and synced.
    let asset_id = storage::read_params(env, &hub_asset).asset_id;
    let live_balance = token::Client::new(env, &asset_id).balance(&env.current_contract_address());

    let snapshot = accounting(env, &hub_asset, live_balance);
    events::emit_market_state(env, snapshot);
}

/// Reconcile accounting against an already-read `live_balance`, split from the
/// SAC read so formal rules can drive the shortfall symbolically.
///
/// No-op unless tracked cash exceeds the balance: donations only raise the
/// balance, so `cash > live_balance` isolates an out-of-band loss and
/// reconciling never socializes a donation.
pub(crate) fn accounting(
    env: &Env,
    hub_asset: &HubAssetKey,
    live_balance: i128,
) -> MarketStateSnapshot {
    let mut cache = ops::synced_market(env, hub_asset);

    if cache.cash() <= live_balance {
        return cache.snapshot();
    }

    let deficit = cache.cash() - live_balance;
    let deficit_ray = Ray::from_asset(deficit, cache.params().asset_decimals);
    // On an empty market the write-down is a no-op while the debit still runs,
    // so the loss lands wholly on dead reserve. Intended — there is no supplier
    // to charge.
    interest::apply_bad_debt_to_supply_index(&mut cache, deficit_ray);
    cache.debit_cash(deficit);
    cache.commit()
}
