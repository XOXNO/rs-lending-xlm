//! Protocol numeric constants (pool + shared), re-exported flat.

pub mod pool;
pub mod shared;

pub use pool::*;
pub use shared::*;

#[cfg(test)]
#[path = "../../tests/constants.rs"]
mod tests;
