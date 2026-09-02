//! Composition suite: one contract, as the top-level caller, chains
//! controller verbs in a single invocation.

extern crate std;

mod atomic_revert_all_legs;
mod contract_caller_runs_every_verb;
mod delegate_revocation_between_legs;
mod helpers;
mod nft_transfer_between_legs;
mod repeated_loops_never_extract_value;
mod supplier_exit_before_socialization_is_bounded_by_utilization;
