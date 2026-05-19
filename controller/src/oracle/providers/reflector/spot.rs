//! Spot-price read via Reflector's `lastprice` entry point. Returns
//! `None` when the feed has no value (e.g. brand-new asset) and the
//! caller didn't mark the read as required; panics with
//! `OracleError::NoLastPrice` when required.

use common::errors::OracleError;
use common::types::{OracleReadMode, ReflectorSourceConfig};
use soroban_sdk::{panic_with_error, Env};

use crate::oracle::observation::OracleObservation;
use crate::oracle::reflector::reflector_lastprice_call;

use super::{observation_from_price_data, to_reflector_asset};

pub(crate) fn read_spot(
    env: &Env,
    config: &ReflectorSourceConfig,
    required: bool,
) -> Option<OracleObservation> {
    let asset = to_reflector_asset(env, &config.asset);
    let Some(pd) = reflector_lastprice_call(env, &config.contract, &asset) else {
        if required {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
        return None;
    };
    Some(observation_from_price_data(
        env,
        &pd,
        config.decimals,
        OracleReadMode::Spot,
    ))
}
