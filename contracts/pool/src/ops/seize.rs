use common::math::fp::Ray;
use common::types::{AccountPositionType, MarketStateSnapshot, PoolSeizeEntry};

use soroban_sdk::Env;

use crate::{interest, ops};

pub(crate) fn apply(env: &Env, entry: &PoolSeizeEntry) -> MarketStateSnapshot {
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
