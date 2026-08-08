//! Mint and burn of scaled supply, debt, and protocol revenue shares.
//!
//! Revenue shares are a subset of total supply: minting revenue increases both
//! `revenue` and `supplied`; burning claimable revenue decreases both.

use common::errors::GenericError;
use common::math::fp::Ray;

use soroban_sdk::assert_with_error;

use super::Cache;

impl Cache {
    /// Mint scaled supply shares into the market total.
    pub(crate) fn mint_supply(&mut self, scaled: Ray) {
        self.supplied = self.supplied.checked_add(&self.env, scaled);
    }

    /// Burn scaled supply shares; then assert revenue ≤ total supply.
    pub(crate) fn burn_supply(&mut self, scaled: Ray) {
        self.supplied = self.supplied.checked_sub(&self.env, scaled);
        self.require_revenue_backed();
    }

    /// Mint scaled debt shares into the market total.
    pub(crate) fn mint_debt(&mut self, scaled: Ray) {
        self.borrowed = self.borrowed.checked_add(&self.env, scaled);
    }

    /// Burn scaled debt shares from the market total.
    pub(crate) fn burn_debt(&mut self, scaled: Ray) {
        self.borrowed = self.borrowed.checked_sub(&self.env, scaled);
    }

    /// Mint protocol revenue shares (also increases total supply).
    pub(crate) fn accrue_revenue(&mut self, scaled: Ray) {
        self.revenue = self.revenue.checked_add(&self.env, scaled);
        self.supplied = self.supplied.checked_add(&self.env, scaled);
    }

    /// Reclassify existing supply shares as protocol revenue (seize deposit side).
    ///
    /// Increases `revenue` only; supply total is unchanged. Asserts revenue still
    /// cannot exceed total supply.
    pub(crate) fn absorb_supply_as_revenue(&mut self, scaled: Ray) {
        self.revenue = self.revenue.checked_add(&self.env, scaled);
        self.require_revenue_backed();
    }

    /// Burn claimable revenue up to available cash; return asset amount withdrawn.
    ///
    /// Partial claims burn a pro-rata ceiling of revenue shares so the treasury
    /// cannot leave dust that never claims. Returns `0` when nothing is available.
    pub(crate) fn burn_claimable_revenue(&mut self) -> i128 {
        let treasury_actual = self.unscale_supply_floor(self.revenue);
        let amount = self.cash.min(treasury_actual);
        if amount <= 0 {
            return 0;
        }
        let scaled_to_burn = if amount >= treasury_actual {
            self.revenue
        } else {
            self.revenue
                .mul_ratio_ceil(&self.env, amount, treasury_actual)
        };

        assert_with_error!(
            self.env,
            scaled_to_burn != Ray::ZERO,
            GenericError::InternalError
        );
        self.revenue = self.revenue.checked_sub(&self.env, scaled_to_burn);
        self.supplied = self.supplied.checked_sub(&self.env, scaled_to_burn);
        amount
    }

    /// Invariant: protocol revenue shares never exceed total supply shares.
    fn require_revenue_backed(&self) {
        assert_with_error!(
            self.env,
            self.revenue <= self.supplied,
            GenericError::InternalError
        );
    }
}
