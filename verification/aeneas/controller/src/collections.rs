//! `push_unique_address`, copied verbatim from `common/src/collections.rs`
//! (imported by `risk/totals.rs`; not on any extracted path).

use soroban_sdk::{Address, Vec};

/// Appends `addr` to `out` unless it is already present.
pub fn push_unique_address(out: &mut Vec<Address>, addr: Address) {
    if !out.contains(&addr) {
        out.push_back(addr);
    }
}
