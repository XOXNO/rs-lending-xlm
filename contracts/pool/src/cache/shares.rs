//! Supply, debt, and protocol-revenue share transitions.

use common::errors::GenericError;
use common::math::fp::Ray;

use soroban_sdk::assert_with_error;

use super::Cache;

impl Cache {
    // --- supply shares ---

    /// Adds `scaled` shares to the market's supply total.
    pub(crate) fn mint_supply(&mut self, scaled: Ray) {
        self.supplied = self.supplied.checked_add(&self.env, scaled);
    }

    /// Removes `scaled` shares from the supply total. Shrinking supply can drop
    /// it under the revenue slice, so this is the one total that re-checks backing.
    pub(crate) fn burn_supply(&mut self, scaled: Ray) {
        self.supplied = self.supplied.checked_sub(&self.env, scaled);
        self.require_revenue_backed();
    }

    // --- debt shares ---

    /// Adds `scaled` shares to the market's debt total.
    pub(crate) fn mint_debt(&mut self, scaled: Ray) {
        self.borrowed = self.borrowed.checked_add(&self.env, scaled);
    }

    /// Removes `scaled` shares from the market's debt total.
    pub(crate) fn burn_debt(&mut self, scaled: Ray) {
        self.borrowed = self.borrowed.checked_sub(&self.env, scaled);
    }

    // --- protocol revenue ---

    /// Mints fresh protocol-owned shares. Revenue is a slice of total supply, so
    /// both totals grow together.
    pub(crate) fn accrue_revenue(&mut self, scaled: Ray) {
        self.revenue = self.revenue.checked_add(&self.env, scaled);
        self.supplied = self.supplied.checked_add(&self.env, scaled);
    }

    /// Reassigns already-minted supply shares to the protocol (seized deposit
    /// dust). Total supply is unchanged — only ownership moves.
    pub(crate) fn absorb_supply_as_revenue(&mut self, scaled: Ray) {
        self.revenue = self.revenue.checked_add(&self.env, scaled);
        self.require_revenue_backed();
    }

    /// Burns the protocol's claimable shares and returns the cash payout.
    ///
    /// Floor conversion: a claim never transfers more than the shares it burns
    /// are worth, so rounding dust stays as supplier backing. A cash-short market
    /// settles partially — shares burn pro-rata to the cash actually paid.
    pub(crate) fn burn_claimable_revenue(&mut self) -> i128 {
        let treasury_actual = self.unscale_supply_floor(self.revenue);
        let amount = self.cash.min(treasury_actual);
        if amount <= 0 {
            return 0;
        }
        let scaled_to_burn = if amount >= treasury_actual {
            self.revenue
        } else {
            // Burn `revenue * amount / treasury_actual` in one full-precision
            // ceil. A single ceil rounds the burn against the claimant, so the
            // shares retired always cover the cash paid: the residual claim can
            // never exceed `previous_claim - payout`. A prior two-step half-up
            // (`from_fraction` then `mul`) could under-burn by a few ulps at
            // extreme index/decimal states and leave paid-for claim behind.
            self.revenue
                .mul_ratio_ceil(&self.env, amount, treasury_actual)
        };
        // A positive payout must always retire a positive part of the protocol
        // claim. The ceil above cannot round a positive ratio to zero; the guard
        // stays as defense against a future rounding-direction change.
        assert_with_error!(
            self.env,
            scaled_to_burn != Ray::ZERO,
            GenericError::InternalError
        );
        self.revenue = self.revenue.checked_sub(&self.env, scaled_to_burn);
        self.supplied = self.supplied.checked_sub(&self.env, scaled_to_burn);
        amount
    }

    /// Protocol revenue is a slice of the supply shares, so it can never exceed
    /// them. Failing here names the cause — an oversized position from the
    /// caller — instead of trapping later in [`Cache::burn_claimable_revenue`].
    ///
    /// # Errors
    /// * `InternalError` — revenue shares exceed total supply shares.
    fn require_revenue_backed(&self) {
        assert_with_error!(
            self.env,
            self.revenue <= self.supplied,
            GenericError::InternalError
        );
    }
}
