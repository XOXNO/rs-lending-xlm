//! Read-only Prometheus exporter for the XOXNO Lending Soroban protocol.
//!
//! Reads pool/controller/oracle view functions over Soroban RPC on a timer and
//! serves them as Prometheus metrics for a public Grafana dashboard. No signer,
//! no writes.

pub mod collector;
pub mod config;
pub mod contract;
pub mod keys;
pub mod metrics;
pub mod model;
pub mod scval;
pub mod stellar;
