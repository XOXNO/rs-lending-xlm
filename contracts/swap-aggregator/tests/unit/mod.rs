//! Contract unit tests for the swap-aggregator router.
//!
//! Organized by concern so each file can be read without the full suite:
//!
//! | Module | What it covers |
//! |---|---|
//! | [`support`] | Shared helpers + mock pools/tokens |
//! | [`venues`] | Per-venue happy paths and adapter edge cases |
//! | [`execute_strategy`] | Decode, validation, balance-delta guards |
//! | [`splits`] | Multi-path PPM sum / rounding |
//! | [`admin`] | Ownership, whitelist, fee caps, upgrade |
//! | [`fees`] | Referral/static fee collection and claims |
//! | [`sweep`] | Stray-token recovery vs reserved fee backing |
//! | [`vault`] | Invocation-local balance map |

#![cfg(test)]

extern crate std;

mod support;

mod admin;
mod execute_strategy;
mod fees;
mod splits;
mod sweep;
mod vault;
mod venues;
