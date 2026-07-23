//! Admin-input validation before scheduling or forwarding to the controller /
//! price-aggregator. Pure shape checks plus live oracle probes.

pub(crate) mod asset;
pub(crate) mod oracle_config;
pub(crate) mod oracle_probe;
pub(crate) mod spoke;
pub(crate) mod tolerance;

use common::errors::GenericError;

use soroban_sdk::{
    assert_with_error, panic_with_error, Address, BytesN, Env, Error, Executable, SpecShakingMarker,
};

pub(crate) fn require_contract_address(
    env: &Env,
    addr: &Address,
    error: impl Into<Error> + SpecShakingMarker,
) {
    if !addr.exists() || !matches!(addr.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, error);
    }
}

pub(crate) fn require_nonzero_wasm_hash(env: &Env, hash: &BytesN<32>) {
    assert_with_error!(
        env,
        hash.to_array() != [0; 32],
        GenericError::InvalidPoolTemplate
    );
}
