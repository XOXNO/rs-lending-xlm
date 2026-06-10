//! Meta / regression / simulation tests (non-domain contract API surface).

extern crate std;

mod account_ttl_regression;
mod bench_liquidate_max_positions;
mod budget_breakdown;
mod chaos_simulation;
mod economic_attacks;
mod footprint_test;
mod invariant;
mod lifecycle_regression;
mod reentrancy_matrix;
mod mem_attribution;
mod repro_live_supply;
mod stress_simulation;
mod utils;