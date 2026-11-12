//! The production fixed-point math, verbatim: `fp.rs` (the `Ray`/`Wad`/`Bps`
//! newtypes, through which the oracle code reaches the raw arithmetic since
//! #224) and the private core `fp_core.rs` it builds on.

#[path = "../../../../common/src/math/fp.rs"]
pub mod fp;
#[path = "../../../../common/src/math/fp_core.rs"]
mod fp_core;
