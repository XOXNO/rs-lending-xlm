//! Fixed-point arithmetic used throughout the protocol. `fp` exposes the
//! `Ray`, `Wad`, and `Bps` newtypes for scale-safe arithmetic; `fp_core`
//! implements the underlying overflow-safe raw-integer operations they build
//! on.

pub mod fp;
pub mod fp_core;
