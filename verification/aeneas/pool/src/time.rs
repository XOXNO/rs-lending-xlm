//! Stand-in for `contracts/pool/src/time.rs` (ledger time in ms). Marked
//! `opaque` for Charon; in Lean `now_ms` is an uninterpreted constant
//! (`FunsExternal.lean`), so theorems hold for every ledger time.

use soroban_sdk::Env;

pub(crate) fn now_ms(_env: &Env) -> u64 {
    unimplemented!("ledger time")
}
