//! Applies a built liquidation plan: debt repayments, collateral seizures, and
//! the post-liquidation residual bad-debt check.
//!
//! Reuses `repay::settle_repay_actions` and `withdraw::settle_withdraw_entries`
//! with `LiqRepay` / `LiqSeize` so usage and position maps stay aligned with
//! user flows. LiqSeize freezes supply risk-param refresh (see withdraw).

use crate::account;
use common::errors::SpokeError;
use common::math::fp::Wad;
use common::types::{
    Account, AccountPosition, DebtPosition, PoolAction, PoolWithdrawEntry, RepayEntry, SeizeEntry,
};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::external::sac::sac_transfer_call;
use crate::positions::liquidation::bad_debt;
use crate::positions::liquidation::math::is_socializable_bad_debt;
use crate::positions::{make_pool_action, repay, withdraw};
use crate::risk::validation;

/// Transfer each planned repay leg from the liquidator, then one bulk pool repay.
pub(crate) fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut Cache,
) {
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for entry in repaid.iter() {
        // Paused debt listing accepts no liquidator tokens (post-normalization legs).
        let debt_paused = cache
            .cached_spoke_asset(account.spoke_id, &entry.hub_asset)
            .is_some_and(|c| c.paused);
        assert_with_error!(env, !debt_paused, SpokeError::SpokeAssetPaused);

        sac_transfer_call(
            env,
            &entry.hub_asset.asset,
            liquidator,
            &pool_addr,
            &entry.amount,
        );

        let position: DebtPosition = (&validation::expect_invariant(
            env,
            account.borrow_positions.get(entry.hub_asset.clone()),
        ))
            .into();
        actions.push_back(make_pool_action(&position, entry.amount, entry.hub_asset));
    }
    repay::settle_repay_actions(
        env,
        account,
        liquidator,
        events::PositionAction::LiqRepay,
        &actions,
        cache,
    );
}

/// One bulk pool withdraw of planned seizures to the liquidator (with protocol fees).
///
/// Does not enforce spoke pause: paused collateral remains seizable. Risk params
/// stay frozen via `LiqSeize` in withdraw settle.
pub(crate) fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Cache,
) {
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for entry in seized.iter() {
        let position: AccountPosition = (&validation::expect_invariant(
            env,
            account.supply_positions.get(entry.hub_asset.clone()),
        ))
            .into();
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(&position, entry.amount, entry.hub_asset),
            protocol_fee: entry.protocol_fee,
        });
    }
    withdraw::settle_withdraw_entries(
        env,
        account,
        liquidator,
        events::PositionAction::LiqSeize,
        &entries,
        cache,
    );
}

/// After liquidation: remove an emptied account, or socialize residual bad debt.
pub(crate) fn check_bad_debt_after_liquidation(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_collateral_usd: Wad,
    total_debt_usd: Wad,
) {
    if account.borrow_positions.is_empty() {
        account::cleanup_account_if_empty(env, account, account_id);
        return;
    }

    if is_socializable_bad_debt(total_debt_usd, total_collateral_usd) {
        bad_debt::execute_bad_debt_cleanup(
            env,
            cache,
            account_id,
            account,
            total_debt_usd.raw(),
            total_collateral_usd.raw(),
        );
    }
}
