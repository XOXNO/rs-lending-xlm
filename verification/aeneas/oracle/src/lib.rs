//! The production LP-token oracle pricing in `common/src/oracle/lp.rs` and
//! `common/src/oracle/lp_stable.rs`, with what they use
//! (`common/src/oracle/observation.rs`, `common/src/math/fp.rs` and the
//! private `common/src/math/fp_core.rs` behind it, `common/src/constants`),
//! compiled verbatim through `#[path]` includes for extraction to Lean with
//! Charon and Aeneas. Nothing in `common/src` is
//! copied or edited. See ../docs/oracle.md.
#![no_std]

pub mod constants;
pub mod errors;
pub mod math;
pub mod oracle;
