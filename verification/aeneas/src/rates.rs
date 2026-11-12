//! The production rate formulas, verbatim. `compound.rs` (array iteration) and
//! `simulate.rs` (storage-backed pool types) are not part of this MVP.

#[path = "../../../common/src/rates/curve.rs"]
pub mod curve;
#[path = "../../../common/src/rates/index.rs"]
pub mod index;
#[path = "../../../common/src/rates/scaling.rs"]
pub mod scaling;
#[path = "../../../common/src/rates/value.rs"]
pub mod value;
