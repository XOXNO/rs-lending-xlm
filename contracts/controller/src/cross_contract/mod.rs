//! Outbound cross-contract call surface (the only place that talks to pools
//! and SAC tokens).
//!
//! Design rule (enforced by review and by the certora harness strategy):
//! **no other module may ever construct a Soroban contract client**.
//! All interaction with `pool_interface::LiquidityPoolClient` and token
//! `transfer` / `balance` calls is funneled through the thin wrappers in
//! `pool.rs` and `sac.rs`.
//!
//! Under the `certora` feature the submodules are completely replaced by
//! harnesses (see harness/cross_contract/{pool,sac}.rs — thin re-exports of
//! shared/summaries). This keeps the prover from having to model real token
//! balances or pool internal state while still exercising the controller's
//! call sites and authorization logic.
//!
//! See the sibling `pool.rs` header for the `ScaledPositionRaw` discipline.
//! Compare to the cleaner providers/*/client.rs + shared summaries pattern
//! used for oracles (avoids full module replacement where possible).

#[cfg(not(feature = "certora"))]
pub(crate) mod pool;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/cross_contract/pool.rs"]
pub(crate) mod pool;

#[cfg(not(feature = "certora"))]
pub(crate) mod sac;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/cross_contract/sac.rs"]
pub(crate) mod sac;
