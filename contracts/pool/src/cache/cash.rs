//! Cash reserve bookkeeping and outbound token transfers.
//!
//! `credit_cash` and `debit_cash` change accounting cash. `transfer_out`
//! moves tokens through the market's `asset_id` token client.

use common::constants::{BPS, LIQUIDATION_BUFFER_BPS, MAX_CLAIM_UTILIZATION_RAY, RAY};
use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::math::fp_core::{mul_div_ceil, mul_div_floor_saturating};
use common::validation::require_nonneg_amount;

use soroban_sdk::{assert_with_error, panic_with_error, token, Address};

use super::Cache;

/// Native units the claim utilization room holds back for the rounding of
/// the pro-rata ceiling burn (see [`Cache::claim_utilization_room`]).
const CLAIM_BURN_MARGIN_UNITS: i128 = 2;

impl Cache {
    /// Cash the liquidation buffer holds back: `LIQUIDATION_BUFFER_BPS` of the
    /// floored supplied token value, rounded up (INV-ACCT-07).
    pub(crate) fn liquidation_reserve(&self) -> i128 {
        let supplied = self.unscale_supply_floor(self.supplied);
        mul_div_ceil(&self.env, supplied, LIQUIDATION_BUFFER_BPS, BPS)
    }

    /// Cash a revenue claim may draw: all of it while the market carries no
    /// debt, otherwise only the cash above the liquidation buffer, so a claim
    /// never takes the cash that liquidating the outstanding debt needs.
    pub(crate) fn claimable_cash(&self) -> i128 {
        if self.borrowed == Ray::ZERO {
            return self.cash;
        }
        self.cash.saturating_sub(self.liquidation_reserve()).max(0)
    }

    /// Largest revenue payout that keeps utilization at or below the claim
    /// bound, `min(max_utilization, MAX_CLAIM_UTILIZATION_RAY)`, in native
    /// units; `i128::MAX` while the market carries no debt. It is the floored
    /// supplied value above `debt_value / bound` (rounded up, saturating),
    /// floored to native units, less `CLAIM_BURN_MARGIN_UNITS`.
    ///
    /// The pro-rata ceiling burn can lower the supplied value by up to one
    /// native unit plus `supply_index / RAY` raw units more than the payout;
    /// the two-unit margin covers that, so after a payout within the room the
    /// floored supplied value still covers `debt_value / bound`, the
    /// utilization guard never refuses it, and the last supply share under
    /// debt never burns. Every rounding step errs toward a smaller room, and
    /// both products saturate, so the room never traps the claim.
    pub(crate) fn claim_utilization_room(&self) -> i128 {
        if self.borrowed == Ray::ZERO {
            return i128::MAX;
        }
        let bound = self
            .params
            .max_utilization
            .min(Ray::from(MAX_CLAIM_UTILIZATION_RAY));
        let supply_value =
            mul_div_floor_saturating(&self.env, self.supplied.raw(), self.supply_index.raw(), RAY);
        // `debt_value / bound` rounded up (one raw unit over when exact), and
        // saturating: a large debt over a low ceiling reads as no room
        // instead of trapping the claim.
        let min_supply_value = mul_div_floor_saturating(
            &self.env,
            self.unscale_borrow_ceil_ray(self.borrowed).raw(),
            RAY,
            bound.raw(),
        )
        .saturating_add(1);
        Ray::from(supply_value.saturating_sub(min_supply_value).max(0))
            .to_asset_floor(&self.env, self.params.asset_decimals)
            .saturating_sub(CLAIM_BURN_MARGIN_UNITS)
    }

    /// Panics if cash reserves are below `amount`.
    pub(crate) fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.cash >= amount,
            CollateralError::InsufficientLiquidity
        );
    }

    /// Increases accounting cash by `amount`. Rejects negative amounts and overflow.
    pub(crate) fn credit_cash(&mut self, amount: i128) {
        require_nonneg_amount(&self.env, amount);
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Decreases accounting cash by `amount`. Rejects negative amounts or
    /// insufficient reserves.
    pub(crate) fn debit_cash(&mut self, amount: i128) {
        require_nonneg_amount(&self.env, amount);
        self.require_reserves(amount);
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Transfers `amount` of the market asset from the pool to `recipient`.
    ///
    /// Rejects negative amounts; zero is a no-op. Does not adjust accounting cash.
    pub(crate) fn transfer_out(&self, recipient: &Address, amount: i128) {
        require_nonneg_amount(&self.env, amount);
        if amount == 0 {
            return;
        }
        let tok = token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }
}
