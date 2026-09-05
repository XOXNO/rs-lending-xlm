use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

use crate::types::PriceFeedRaw;
use common::math::fp::Wad;
use common::types::MarketIndexRaw;

use crate::context::Context;

pub mod pool;

pub(crate) fn price_feed_summary(env: &Env, _asset: &Address) -> PriceFeedRaw {
    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0);
    cvlr_assume!(asset_decimals <= 27);
    cvlr_assume!(timestamp <= env.ledger().timestamp().saturating_add(60));
    PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    }
}

/// Indexes returned by `LiquidityPool::get_bulk_indexes`, feeding
/// `Context::cached_market_index`.
///
/// Draws from exactly the same generator as the index fields of
/// [`pool::get_sync_data_summary`] (`Context::cached_pool_sync_data`), so the two
/// paths can no longer disagree about the domain of one market's indexes. They
/// still draw *independently* on each call; the controller harness memoises
/// both per rule (`certora/controller/harness/ghost_prices.rs`) so a rule that
/// reads a market's sync data and its bulk index sees one draw, not two.
pub fn bulk_index_summary(_env: &Env, _asset: &Address) -> MarketIndexRaw {
    pool::nondet_market_index_raw()
}

/// Havoc summary for `risk::totals::calculate_account_risk_totals`.
///
/// Sunbeam havocs storage at rule start, so the position maps a rule does not
/// seed hold arbitrary entries: arbitrary length, arbitrary `scaled_amount`
/// (including zero and negative), and an unclamped `u32 loan_to_value` /
/// `liquidation_threshold`. The constraints below are therefore split into two
/// groups, and the split is what a caller has to read before trusting a verdict.
///
/// Unconditional (true for any book the production body can be handed):
///
/// - every total is non-negative — the body sums `checked_add` of `Wad` values
///   and `Wad::checked_add` panics on a negative operand;
/// - an empty map yields zero on its side — the body starts at `Wad::ZERO` and
///   never enters the loop.
///
/// A **non-empty** map yields `>= 0`, not `> 0`: a havoced book may hold a
/// `scaled_amount` of zero, or a price and index that floor the position's
/// value to zero, so a live map does not imply a positive total. The earlier
/// `> 0` form silently assumed a well-formed book in every dependent proof.
///
/// Well-formed-book premise (kept, because dropping it makes every health rule
/// vacuous, but it *is* an assumption):
///
/// - `weighted_collateral <= total_collateral` and
///   `ltv_collateral <= total_collateral`. The body computes the weighted and
///   LTV sums as `Bps::apply_to_wad_floor` of a floored position value, so the
///   inequalities hold exactly when every position's `liquidation_threshold`
///   and `loan_to_value` are at most `BPS` — which market creation enforces and
///   a havoced book does not. The controller fixture states the same premise
///   explicitly through its `assume_wellformed_book` helper.
///
/// The health factor is computed the way the production body computes it:
/// `i128::MAX` with no debt, otherwise `div_floor_saturating` (not
/// `div_floor`), so the summary cannot panic where production saturates.
pub(crate) fn calculate_account_risk_totals_summary(
    env: &Env,
    _cache: &mut Context,
    supply_positions: &soroban_sdk::Map<
        common::types::HubAssetKey,
        common::types::AccountPositionRaw,
    >,
    borrow_positions: &soroban_sdk::Map<common::types::HubAssetKey, common::types::DebtPositionRaw>,
) -> crate::risk::AccountRiskTotals {
    let total_collateral_raw: i128 = nondet();
    let ltv_collateral_raw: i128 = nondet();
    let weighted_coll_raw: i128 = nondet();
    let total_debt_raw: i128 = nondet();
    cvlr_assume!(total_collateral_raw >= 0);
    cvlr_assume!(ltv_collateral_raw >= 0);
    cvlr_assume!(weighted_coll_raw >= 0);
    cvlr_assume!(total_debt_raw >= 0);
    // Well-formed-book premise; see the doc comment above.
    cvlr_assume!(weighted_coll_raw <= total_collateral_raw);
    cvlr_assume!(ltv_collateral_raw <= total_collateral_raw);
    if supply_positions.is_empty() {
        cvlr_assume!(total_collateral_raw == 0);
        cvlr_assume!(ltv_collateral_raw == 0);
        cvlr_assume!(weighted_coll_raw == 0);
    }
    if borrow_positions.is_empty() {
        cvlr_assume!(total_debt_raw == 0);
    }

    let total_debt = Wad::from(total_debt_raw);
    let weighted_collateral = Wad::from(weighted_coll_raw);
    let health_factor = if total_debt == Wad::ZERO {
        Wad::from(i128::MAX)
    } else {
        weighted_collateral.div_floor_saturating(env, total_debt)
    };

    crate::risk::AccountRiskTotals {
        total_collateral: Wad::from(total_collateral_raw),
        ltv_collateral: Wad::from(ltv_collateral_raw),
        weighted_collateral,
        total_debt,
        health_factor,
    }
}
