//! The production constants, verbatim: `common/src/constants` and the
//! controller's own `contracts/controller/src/constants.rs` (which re-exports
//! the common ones and adds `BAD_DEBT_USD_THRESHOLD` and friends).

#[path = "../../../../common/src/constants/mod.rs"]
mod production;

#[path = "../../../../contracts/controller/src/constants.rs"]
mod controller;

pub use controller::*;
pub use production::*;
