//! Shared validation helpers for governance: contract-address
//! existence/executability checks and wasm-hash zero checks, used by `deploy`
//! and `op`. The `asset` and `tolerance` submodules hold the asset-onboarding
//! and oracle-tolerance validators.

pub(crate) mod asset;
pub(crate) mod tolerance;

use common::errors::GenericError;

use soroban_sdk::{
    assert_with_error, panic_with_error, Address, BytesN, Env, Error, Executable, SpecShakingMarker,
};

/// Panics with `error` unless `addr` exists on-chain and is a deployed Wasm contract.
pub(crate) fn require_contract_address(
    env: &Env,
    addr: &Address,
    error: impl Into<Error> + SpecShakingMarker,
) {
    if !addr.exists() || !matches!(addr.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, error);
    }
}

/// Panics with `GenericError::InvalidWasmHash` if `hash` is all zero bytes.
pub(crate) fn require_nonzero_wasm_hash(env: &Env, hash: &BytesN<32>) {
    assert_with_error!(
        env,
        hash.to_array() != [0; 32],
        GenericError::InvalidWasmHash
    );
}
