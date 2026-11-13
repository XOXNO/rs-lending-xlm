//! Signature-only stand-ins for `validate_bulk_position_limits` and
//! `require_whole_unit_isolation` (`contracts/controller/src/risk/validation.rs`),
//! which the included `apply.rs` calls only from
//! `require_credit_position_limit` (not extracted).

use crate::context::Context;
use crate::types::{Account, AccountPositionType, AggregatedPayments};
use soroban_sdk::Env;

pub(crate) fn validate_bulk_position_limits(
    _env: &Env,
    _account: &Account,
    _position_type: AccountPositionType,
    _aggregated: &AggregatedPayments,
) {
    unimplemented!("not extracted")
}

pub(crate) fn require_whole_unit_isolation(
    _env: &Env,
    _cache: &mut Context,
    _account: &Account,
    _aggregated: &AggregatedPayments,
) {
    unimplemented!("not extracted")
}
