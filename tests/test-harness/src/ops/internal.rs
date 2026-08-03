use common::types::HubAssetKey;
use soroban_sdk::{vec, Address, Env, Vec};

use crate::helpers::hub_asset;
use crate::helpers::units::f64_to_i128;

pub fn amount_raw(amount: f64, decimals: u32) -> i128 {
    f64_to_i128(amount, decimals)
}

pub fn asset_payment_vec(env: &Env, asset: Address, raw_amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![env, (hub_asset(asset), raw_amount)]
}

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

pub fn burn_prefund(env: &Env, asset: &Address, addr: &Address, raw_amount: i128) {
    if raw_amount > 0 {
        soroban_sdk::token::TokenClient::new(env, asset).burn(addr, &raw_amount);
    }
}
