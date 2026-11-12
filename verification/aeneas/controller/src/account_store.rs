//! Re-exported as `crate::account` (see `lib.rs`). Stand-ins for `cleanup_account_if_empty` and
//! `update_or_remove_supply_position` (`contracts/controller/src/account.rs`).
//! Opaque (Charon `--opaque`); modelled in `FunsExternal.lean`.

use crate::types::{Account, AccountPosition, HubAssetKey};
use soroban_sdk::Env;

/// Removes the account (and burns its NFT) when it has no positions left.
pub(crate) fn cleanup_account_if_empty(_env: &Env, _account: &Account, _account_id: u64) {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}

/// Writes the in-memory supply position, removing it when its scaled amount is zero.
pub(crate) fn update_or_remove_supply_position(
    _account: &mut Account,
    _hub_asset: &HubAssetKey,
    _position: &AccountPosition,
) {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}
