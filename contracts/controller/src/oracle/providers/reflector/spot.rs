// Spot-price read via Reflector lastprice.

use common::errors::OracleError;
use common::types::ReflectorSourceConfig;
use soroban_sdk::{assert_with_error, Env};

use super::reflector_lastprice_call;
use crate::oracle::observation::OracleObservation;

use super::{observation_from_price_data, to_reflector_asset};

pub(crate) fn read_spot(
    env: &Env,
    config: &ReflectorSourceConfig,
    required: bool,
) -> Option<OracleObservation> {
    let asset = to_reflector_asset(env, &config.asset);
    let Some(pd) = reflector_lastprice_call(env, &config.contract, &asset) else {
        assert_with_error!(env, !required, OracleError::NoLastPrice);
        return None;
    };
    Some(observation_from_price_data(env, &pd, config.decimals))
}
