//! Certora harness substitute for `controller::pool_calls`.
//!
//! Under `--features certora`, `controller/src/lib.rs` path-swaps the
//! `pool_calls` module to this file. Re-exports the existing pool ABI
//! summaries from `verification/certora/shared/summaries/pool.rs`
//! under the production wrapper names. The prover then sees bounded
//! nondet returns in place of cross-contract `LiquidityPoolClient`
//! invocations.

pub(crate) use crate::spec::summaries::pool::{
    add_rewards_summary as pool_add_rewards_call, borrow_summary as pool_borrow_call,
    claim_revenue_summary as pool_claim_revenue_call,
    create_strategy_summary as pool_create_strategy_call,
    flash_loan_summary as pool_flash_loan_call, get_sync_data_summary as fetch_pool_sync_data,
    repay_summary as pool_repay_call, seize_position_summary as pool_seize_position_call,
    supply_summary as pool_supply_call, update_indexes_summary as pool_update_indexes_call,
    withdraw_summary as pool_withdraw_call,
};

// Void privileged-config calls have no return value to summarize, so the
// prover treats them as no-ops. They exist only so the production import
// path in `router.rs` resolves under the certora feature.
use controller::types::InterestRateModel;
use soroban_sdk::{Address, BytesN, Env};

pub(crate) fn pool_keepalive_call(_env: &Env, _pool_addr: &Address) {}

pub(crate) fn pool_update_params_call(
    _env: &Env,
    _pool_addr: &Address,
    _params: &InterestRateModel,
) {
}

pub(crate) fn pool_upgrade_call(_env: &Env, _pool_addr: &Address, _new_wasm_hash: &BytesN<32>) {}
