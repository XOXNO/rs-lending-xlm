//! Soroban RPC: connection wrapper + read-only view simulation.

pub mod client;
pub mod view;

pub use client::RpcClient;
pub use view::{simulate_view, ViewError};
