//! Stand-in for `execute_bad_debt_cleanup`
//! (`contracts/controller/src/positions/liquidation/bad_debt.rs`): seizes all
//! remaining collateral as revenue and socializes all remaining debt through
//! the pool. Opaque (Charon `--opaque`); modelled in `FunsExternal.lean` as
//! an uninterpreted effect on the context.

use crate::context::Context;
use crate::risk::AccountRiskTotals;
use crate::types::Account;
use soroban_sdk::Env;

pub(crate) fn execute_bad_debt_cleanup(
    _env: &Env,
    _cache: &mut Context,
    _account_id: u64,
    _account: &Account,
    _totals: &AccountRiskTotals,
) {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}
