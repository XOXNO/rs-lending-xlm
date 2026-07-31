use common::errors::GenericError;
use common::math::fp::Ray;

use soroban_sdk::assert_with_error;

use super::Cache;

impl Cache {
    pub(crate) fn mint_supply(&mut self, scaled: Ray) {
        self.supplied = self.supplied.checked_add(&self.env, scaled);
    }

    pub(crate) fn burn_supply(&mut self, scaled: Ray) {
        self.supplied = self.supplied.checked_sub(&self.env, scaled);
        self.require_revenue_backed();
    }

    pub(crate) fn mint_debt(&mut self, scaled: Ray) {
        self.borrowed = self.borrowed.checked_add(&self.env, scaled);
    }

    pub(crate) fn burn_debt(&mut self, scaled: Ray) {
        self.borrowed = self.borrowed.checked_sub(&self.env, scaled);
    }

    pub(crate) fn accrue_revenue(&mut self, scaled: Ray) {
        self.revenue = self.revenue.checked_add(&self.env, scaled);
        self.supplied = self.supplied.checked_add(&self.env, scaled);
    }

    pub(crate) fn absorb_supply_as_revenue(&mut self, scaled: Ray) {
        self.revenue = self.revenue.checked_add(&self.env, scaled);
        self.require_revenue_backed();
    }

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

    fn require_revenue_backed(&self) {
        assert_with_error!(
            self.env,
            self.revenue <= self.supplied,
            GenericError::InternalError
        );
    }
}
