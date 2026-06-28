//! Shared operation helpers.

use common::types::HubAssetKey;
use soroban_sdk::{vec, Address, Env, Vec};

use crate::helpers::hub_asset;
use crate::helpers::units::f64_to_i128;

/// Converts a token amount to raw units.
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
