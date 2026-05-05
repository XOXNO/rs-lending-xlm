use common::types::{Account, AccountPosition, AccountPositionType};
use soroban_sdk::Address;

/// Upserts or removes the position from the appropriate side map on
/// `account`. Removes the entry when `scaled_amount_ray == 0`; returns
/// `true` when removed.
///
/// `side` and `asset` are taken from the caller because
/// [`AccountPosition`] no longer carries them in its stored form — the
/// side is implied by which map the value lives in and the asset is the
/// map key.
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

    if position.scaled_amount_ray == 0 {
        map.remove(asset.clone());
        true
    } else {
        map.set(asset.clone(), position.clone());
        false
    }
}
