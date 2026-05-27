// No file-level allow needed here anymore — the contractclient allow lives in client.rs.

//! RedStone Price Feed provider (consumption logic).
//!
//! The contract client surface lives in `client.rs` (following the clean
//! pattern established by Reflector).
//!
//! This module now focuses on how the controller consumes RedStone prices.

use common::errors::GenericError;
use common::types::RedStoneSourceConfig;
use soroban_sdk::{panic_with_error, Env};

use super::super::observation::{
    build_observation, check_not_future_at, millis_to_seconds, u256_to_i128, OracleObservation,
};
use crate::cache::ControllerCache;

mod client; // Canonical home of RedStone contract client types + trait.

pub(crate) use client::{read_price_data, RedStonePriceData, REDSTONE_DECIMALS};

pub(crate) fn read_redstone_source(
    cache: &ControllerCache,
    config: &RedStoneSourceConfig,
    required: bool,
) -> Option<OracleObservation> {
    let env = cache.env();

    let price_data = match read_price_data(env, &config.contract, &config.feed_id) {
        Some(price_data) => price_data,
        _ if required => panic_with_error!(env, GenericError::InvalidTicker),
        _ => return None,
    };

    Some(observation_from_price_data(
        env,
        &price_data,
        config.decimals,
    ))
}

fn observation_from_price_data(
    env: &Env,
    price_data: &RedStonePriceData,
    decimals: u32,
) -> OracleObservation {
    let package_ts = millis_to_seconds(env, price_data.package_timestamp);
    let write_ts = millis_to_seconds(env, price_data.write_timestamp);
    let now_secs = env.ledger().timestamp();
    check_not_future_at(env, now_secs, package_ts);
    check_not_future_at(env, now_secs, write_ts);

    let raw_price = u256_to_i128(env, &price_data.price);
    build_observation(env, raw_price, decimals, write_ts, Some(package_ts))
}
