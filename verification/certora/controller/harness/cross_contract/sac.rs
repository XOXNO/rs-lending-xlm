//! Certora harness substitute for `controller::sac_calls`.
//!
//! Re-exports the SAC `transfer` summary under the production wrapper
//! name. Production performs a cross-contract call to the SAC; the
//! summary models the `amount >= 0` precondition without executing the
//! cross-contract path.

pub(crate) use crate::spec::summaries::sac::transfer_summary as sac_transfer_call;
