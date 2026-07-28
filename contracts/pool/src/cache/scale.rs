//! Utilization and share ↔ asset conversions. Rounding always favours the pool.

use common::math::fp::Ray;
use common::rates::{
    calculate_scaled_borrow, calculate_scaled_supply, resolve_repay, resolve_withdrawal,
    scaled_to_original, unscale_borrow, unscale_borrow_ceil, unscale_borrow_ceil_ray,
    unscale_supply, unscale_supply_floor, utilization,
};

use super::Cache;

impl Cache {
    /// Utilization in RAY at the cache's current indexes; zero on an empty market.
    pub(crate) fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        let total_borrowed = scaled_to_original(&self.env, self.borrowed, self.borrow_index);
        let total_supplied = scaled_to_original(&self.env, self.supplied, self.supply_index);

        utilization(&self.env, total_borrowed, total_supplied)
    }

    /// Floor-scaled: a deposit never mints more shares than it paid for.
    pub(crate) fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        calculate_scaled_supply(
            &self.env,
            amount,
            self.params.asset_decimals,
            self.supply_index,
        )
    }

    /// Ceil-scaled: a positive borrow always mints positive debt shares.
    pub(crate) fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        calculate_scaled_borrow(
            &self.env,
            amount,
            self.params.asset_decimals,
            self.borrow_index,
        )
    }

    /// Half-up: a reporting figure. Payouts use [`Cache::unscale_supply_floor`].
    pub(crate) fn unscale_supply(&self, scaled: Ray) -> i128 {
        unscale_supply(
            &self.env,
            scaled,
            self.supply_index,
            self.params.asset_decimals,
        )
    }

    /// Floor: never pay a supplier more than their shares are worth.
    pub(crate) fn unscale_supply_floor(&self, scaled: Ray) -> i128 {
        unscale_supply_floor(
            &self.env,
            scaled,
            self.supply_index,
            self.params.asset_decimals,
        )
    }

    /// Half-up: a reporting figure. Debt owed uses [`Cache::unscale_borrow_ceil`].
    pub(crate) fn unscale_borrow(&self, scaled: Ray) -> i128 {
        unscale_borrow(
            &self.env,
            scaled,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }

    /// Ceil: never understate what a borrower owes.
    pub(crate) fn unscale_borrow_ceil(&self, scaled: Ray) -> i128 {
        unscale_borrow_ceil(
            &self.env,
            scaled,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }

    /// Ceil, kept in RAY for the bad-debt write-down.
    pub(crate) fn unscale_borrow_ceil_ray(&self, scaled: Ray) -> Ray {
        unscale_borrow_ceil_ray(&self.env, scaled, self.borrow_index)
    }

    /// Returns the shares to burn and the gross payout. A request that reaches
    /// the half-up balance closes the position and pays the floor value, so a
    /// full close never transfers more than the burned shares are worth.
    ///
    /// The controller's dust gate must mirror that half-up threshold exactly, or
    /// its position map and the pool diverge.
    pub(crate) fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        resolve_withdrawal(
            &self.env,
            amount,
            pos_scaled,
            self.supply_index,
            self.params.asset_decimals,
        )
    }

    /// Returns the debt shares to burn and any overpayment, closing the position
    /// when the payment reaches the ceil debt owed.
    pub(crate) fn resolve_repay(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        resolve_repay(
            &self.env,
            amount,
            pos_scaled,
            self.borrow_index,
            self.params.asset_decimals,
        )
    }
}
