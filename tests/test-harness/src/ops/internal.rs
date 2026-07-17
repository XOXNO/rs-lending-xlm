//! Shared operation helpers.

use common::types::HubAssetKey;
use soroban_sdk::{vec, Address, Env, Vec};

use crate::helpers::hub_asset;
use crate::helpers::units::f64_to_i128;

pub fn amount_raw(amount: f64, decimals: u32) -> i128 {
    f64_to_i128(amount, decimals)
}

/// Single-asset payment vector for controller calls.
pub fn asset_payment_vec(env: &Env, asset: Address, raw_amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![env, (hub_asset(asset), raw_amount)]
}

/// Map `try_*` client nested `Result` to `Result<(), Error>`.
pub fn map_try_ok_unit(
    result: Result<
        Result<(), soroban_sdk::ConversionError>,
        Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
    >,
) -> Result<(), soroban_sdk::Error> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

/// Burn a harness pre-fund mint back out of `addr`'s wallet.
///
/// `try_*` helpers mint the operation amount to the caller in a separate
/// top-level invoke before the contract call. A failed contract invoke rolls
/// back only its own subtree, so the mint would otherwise persist and a
/// failed op would not be wallet-neutral — the fuzz rollback invariant
/// (wallet balances unchanged across a failed op) then fires without any
/// contract bug. Compensating by exactly the minted amount keeps that
/// invariant sharp: any residual drift is real contract-side movement.
pub fn burn_prefund(env: &Env, asset: &Address, addr: &Address, raw_amount: i128) {
    if raw_amount > 0 {
        soroban_sdk::token::TokenClient::new(env, asset).burn(addr, &raw_amount);
    }
}
