//! Fixed-point scale conversions between asset amounts and share positions.
//!
//! Wraps `common::rates` helpers with this market's decimals and live indexes
//! so ops code does not pass index/decimals on every call.

use common::math::fp::Ray;
use common::rates::{
    calculate_scaled_borrow, calculate_scaled_supply, resolve_net_settle, resolve_repay,
    resolve_withdrawal, scaled_to_original, unscale_borrow, unscale_borrow_ceil,
    unscale_borrow_ceil_ray, unscale_supply, unscale_supply_floor, utilization,
};

use super::Cache;

impl Cache {
    /// Utilization = total borrowed value / total supplied value (RAY).
    ///
    /// Returns zero when there is no supply.
    pub(crate) fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        let total_borrowed = scaled_to_original(&self.env, self.borrowed, self.borrow_index);
        let total_supplied = scaled_to_original(&self.env, self.supplied, self.supply_index);

        utilization(&self.env, total_borrowed, total_supplied)
    }

    /// Converts an asset deposit into scaled supply shares (floor at the supply index).
    pub(crate) fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        calculate_scaled_supply(
            &self.env,
            amount,
            self.params.asset_decimals,
            self.supply_index,
        )
    }

    /// Converts an asset borrow into scaled debt shares (ceil at the borrow index).
    pub(crate) fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        calculate_scaled_borrow(
            &self.env,
            amount,
            self.params.asset_decimals,
            self.borrow_index,
        )
    }

    /// Unscales supply shares to asset units with half-up rounding.
    pub(crate) fn unscale_supply(&self, scaled: Ray) -> i128 {
        unscale_supply(
            &self.env,
            scaled,
            self.supply_index,
            self.params.asset_decimals,
        )
    }

    /// Unscales supply shares rounding **down** (conservative claim value).
    pub(crate) fn unscale_supply_floor(&self, scaled: Ray) -> i128 {
        unscale_supply_floor(
            &self.env,
            scaled,
            self.supply_index,
            self.params.asset_decimals,
        )
    }

    /// Unscales debt shares to asset units with half-up rounding.
    pub(crate) fn unscale_borrow(&self, scaled: Ray) -> i128 {
        unscale_borrow(
            &self.env,
            scaled,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }

    /// Unscales debt shares rounding **up** (conservative liability).
    pub(crate) fn unscale_borrow_ceil(&self, scaled: Ray) -> i128 {
        unscale_borrow_ceil(
            &self.env,
            scaled,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }

    /// Unscales debt shares to a RAY asset amount, rounding up.
    pub(crate) fn unscale_borrow_ceil_ray(&self, scaled: Ray) -> Ray {
        unscale_borrow_ceil_ray(&self.env, scaled, self.borrow_index)
    }

    /// Resolves a withdrawal request into (shares burned, gross asset amount).
    ///
    /// Caps against `pos_scaled` so the user cannot withdraw more than held.
    pub(crate) fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        resolve_withdrawal(
            &self.env,
            amount,
            pos_scaled,
            self.supply_index,
            self.params.asset_decimals,
        )
    }

    /// Resolves a repay request into (shares burned, overpayment in asset units).
    ///
    /// Overpayment is returned to the payer by the repay op.
    pub(crate) fn resolve_repay(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        resolve_repay(
            &self.env,
            amount,
            pos_scaled,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }

    /// Resolves a same-asset net-settle into (supply burned, debt burned, tokens).
    ///
    /// Uses the conservative overlap of floored supply and ceiled debt. Does
    /// not inherit withdraw's half-up full-close switch.
    pub(crate) fn resolve_net_settle(
        &self,
        amount: i128,
        supply_scaled: Ray,
        debt_scaled: Ray,
    ) -> (Ray, Ray, i128) {
        resolve_net_settle(
            &self.env,
            amount,
            supply_scaled,
            debt_scaled,
            self.supply_index,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }
}
