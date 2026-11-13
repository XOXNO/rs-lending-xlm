//! The production fixed-point math, verbatim. As in `common/src/math/mod.rs`
//! since #224, `fp_core` is private: callers go through the typed `fp`
//! wrappers.

#[path = "../../../../common/src/math/fp.rs"]
pub mod fp;
#[path = "../../../../common/src/math/fp_core.rs"]
mod fp_core;
