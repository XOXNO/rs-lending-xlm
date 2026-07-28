//! Proposal-shape validation before schedule or forward.
//!
//! Shape and address checks only. Oracle-source rules live on the
//! price-aggregator.

pub(crate) mod asset;
pub(crate) mod spoke;
pub(crate) mod tolerance;

use common::errors::GenericError;

use soroban_sdk::{
    assert_with_error, panic_with_error, Address, BytesN, Env, Error, Executable, SpecShakingMarker,
};

/// Requires a deployed Wasm contract address.
///
/// # Errors
/// * Caller-supplied `error` when the address is missing or not Wasm.
pub(crate) fn require_contract_address(
    env: &Env,
    addr: &Address,
    error: impl Into<Error> + SpecShakingMarker,
) {
    if !addr.exists() || !matches!(addr.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, error);
    }
}

/// Rejects an all-zero wasm hash.
///
/// # Errors
/// * [`GenericError::InvalidWasmHash`] — zero hash.
pub(crate) fn require_nonzero_wasm_hash(env: &Env, hash: &BytesN<32>) {
    assert_with_error!(
        env,
        hash.to_array() != [0; 32],
        GenericError::InvalidWasmHash
    );
}
