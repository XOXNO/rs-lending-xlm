//! Protocol numeric constants: pool-specific ([`pool`]) and protocol-wide
//! ([`shared`]) values, re-exported flat for `crate::constants::*` access.

pub mod pool;
pub mod shared;

pub use pool::*;
pub use shared::*;

#[cfg(test)]
#[path = "../../tests/constants.rs"]
mod tests;
