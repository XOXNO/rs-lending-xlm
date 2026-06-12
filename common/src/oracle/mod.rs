//! Shared oracle client plumbing for Reflector and RedStone providers.
//!
//! Hosts the provider call wrappers and the observation normalization and
//! freshness guards used by both production price resolution and the
//! oracle config validators.

pub mod observation;
pub mod providers;
