//! Seize positions during liquidation or bad-debt cleanup.
//!
//! - **Borrow side:** socialize unpaid debt onto the supply index, burn debt.
//! - **Deposit side:** reclassify supply shares as protocol revenue (no cash out).

use common::math::fp::Ray;
use common::types::{AccountPositionType, MarketStateSnapshot, PoolSeizeEntry};
use common::validation::require_nonneg_amount;

use soroban_sdk::Env;

use crate::{interest, ops};

/// Apply one seize entry and return the committed market snapshot.
///
/// Does not transfer tokens; the hub adjusts user position books separately.
/// Rejects negative hub-supplied `scaled_amount` (fail-closed against a
/// compromised or buggy hub).
pub(crate) fn apply(env: &Env, entry: &PoolSeizeEntry) -> MarketStateSnapshot {
    require_nonneg_amount(env, entry.position.scaled_amount);
    let mut cache = ops::synced_market(env, &entry.hub_asset);
    let position = Ray::from(entry.position.scaled_amount);

    match entry.side {
        AccountPositionType::Borrow => {
            let bad_debt = cache.unscale_borrow_ceil_ray(position);
            interest::apply_bad_debt_to_supply_index(&mut cache, bad_debt);
            cache.burn_debt(position);
        }
        AccountPositionType::Deposit => {
            cache.absorb_supply_as_revenue(position);
        }
    }

    cache.commit()
}
