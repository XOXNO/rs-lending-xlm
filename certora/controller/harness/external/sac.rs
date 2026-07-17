//! Certora harness for `controller::sac_calls`.
//! SAC transfer summary (`amount >= 0`); no cross-contract call.

pub(crate) use crate::spec::summaries::sac::transfer_summary as sac_transfer_call;
