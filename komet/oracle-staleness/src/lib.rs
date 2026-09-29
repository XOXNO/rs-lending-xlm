#![no_std]

use soroban_sdk::{contract, contractimpl};

#[contract]
pub struct OracleStalenessProof;

// Keep the production call separate so the compiler cannot replace the entire
// property with a constant before Komet symbolically executes it.
#[inline(never)]
fn production_is_stale(now: u64, feed: u64, max_age: u64) -> bool {
    common::oracle::observation::is_stale(now, feed, max_age)
}

#[contractimpl]
impl OracleStalenessProof {
    /// All u64 timestamps and age limits: a newly timestamped feed is not stale.
    pub fn test_fresh_never_stale(now: u64, max_age: u64) -> bool {
        !production_is_stale(now, now, max_age)
    }

    /// Deliberately false control: a newly timestamped feed would be stale.
    pub fn test_fresh_wrong(now: u64, max_age: u64) -> bool {
        production_is_stale(now, now, max_age)
    }
}
