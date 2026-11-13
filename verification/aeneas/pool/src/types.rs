//! Stand-in for `common::types`: the two production files the pool code needs,
//! `types/pool.rs` and `types/shared.rs`, included **verbatim**, plus a copy
//! of the `OracleTolerance` record from `types/oracle.rs`, which only
//! `validation.rs` names (in a function the proofs never reach). The other
//! production type files (controller, oracle, composable oracle) are left out.

#[path = "../../../../common/src/types/pool.rs"]
mod pool;
#[path = "../../../../common/src/types/shared.rs"]
mod shared;

pub use pool::*;
pub use shared::*;

/// Copy of `common::types::OracleTolerance` (fields only).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleTolerance {
    pub upper_ratio_bps: u32,
    pub lower_ratio_bps: u32,
}
