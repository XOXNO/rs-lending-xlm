//! Read-only Prometheus exporter for XOXNO Lending (Soroban).
//!
//! Scrapes pool/controller/oracle views on a timer; serves `/metrics`. No signer.

pub mod collector;
pub mod config;
pub mod contract;
pub mod keys;
pub mod metrics;
pub mod model;
pub mod scval;
pub mod stellar;
