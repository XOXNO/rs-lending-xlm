use common::math::fp::Ray;
use common::types::{Account, AccountPosition, AccountPositionType};
use soroban_sdk::Address;

// Upserts or removes position.
pub fn update_or_remove_position(
    account: &mut Account,
    side: AccountPositionType,
    asset: &Address,
    position: &AccountPosition,
) -> bool {
    let map = match side {
        AccountPositionType::Deposit => &mut account.supply_positions,
        AccountPositionType::Borrow => &mut account.borrow_positions,
    };

    if position.scaled_amount == Ray::ZERO {
        map.remove(asset.clone());
        true
    } else {
        map.set(asset.clone(), position.into());
        false
    }
}
