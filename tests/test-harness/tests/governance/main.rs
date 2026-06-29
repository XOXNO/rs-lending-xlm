//! Governance admin-validation integration tests.
//!
//! Every admin input check lives in the governance contract; these suites
//! drive the validation through the governance client against real mock
//! oracles and tokens, asserting the exact contract error codes.

extern crate std;

mod admin;
mod admin_config;
mod dex_usd_repricing;
mod spoke;
mod redstone;
mod timelock;
mod tolerance;
mod validation_admin;
