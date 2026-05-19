//! Outbound cross-contract calls. Each submodule wraps one external
//! contract ABI so the rest of the controller never reaches a Soroban
//! `Client::new(...)` directly. Under `--features certora` each leaf
//! is path-swapped to a harness file that returns bounded nondet
//! values, isolating the prover from cross-contract havoc.

#[cfg(not(feature = "certora"))]
pub(crate) mod pool;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/cross_contract/pool.rs"]
pub(crate) mod pool;

#[cfg(not(feature = "certora"))]
pub(crate) mod sac;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/cross_contract/sac.rs"]
pub(crate) mod sac;
