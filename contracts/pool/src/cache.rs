//! In-memory view of one market: load once, mutate through named transitions,
//! commit once. Every field is private so the only way to move money is a
//! method whose name says what moved. Rounding always favours the pool over the
//! counterparty — floor what we pay out, ceil what we are owed.
//! See `docs/reference/invariants.md`.

use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::rates::{
    calculate_scaled_borrow, calculate_scaled_supply, resolve_repay, resolve_withdrawal,
    scaled_to_original, unscale_borrow, unscale_borrow_ceil, unscale_borrow_ceil_ray,
    unscale_supply, unscale_supply_floor, utilization,
};
use common::types::{
    HubAssetKey, MarketIndexRaw, MarketParams, MarketStateSnapshot, PoolPositionMutation,
    PoolState, PoolStateRaw, PoolStrategyMutation, ScaledPositionRaw,
};

use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

use crate::{storage, time};

/// One market's params and accounting state, held for the length of a single
/// leg. Construct with [`Cache::load`], mutate with the transitions below,
/// persist with [`Cache::commit`].
pub(crate) struct Cache {
    env: Env,
    hub_asset: HubAssetKey,
    params: MarketParams,
    last_timestamp: u64,
    current_timestamp: u64,
    supplied: Ray,
    borrowed: Ray,
    revenue: Ray,
    borrow_index: Ray,
    supply_index: Ray,
    /// Tracked cash (`Token(asset)`); direct donations never increase this.
    ///
    /// Invariant: `cash >= sum(claimable supplier + revenue value)`. The surplus
    /// is protocol-owned dead reserve, unreachable by any user path because every
    /// payout is cash-gated by [`Cache::require_reserves`]. See
    /// `docs/reference/invariants.md` for how it accrues.
    cash: i128,
}

impl Cache {
    /// Loads params and state for `hub_asset` and renews both market keys.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — params or state missing for the market.
    /// * `MathOverflow` — ledger timestamp to milliseconds overflow.
    pub(crate) fn load(env: &Env, hub_asset: &HubAssetKey) -> Self {
        let raw_params = storage::read_params(env, hub_asset);
        let raw_state = storage::read_state(env, hub_asset);
        storage::renew_market(env, hub_asset);

        let state = PoolState::from(&raw_state);
        let params = MarketParams::from(&raw_params);
        let time = time::now_ms(env);

        Self {
            env: env.clone(),
            hub_asset: hub_asset.clone(),
            params,
            last_timestamp: state.last_timestamp,
            current_timestamp: time,
            supplied: state.supplied,
            borrowed: state.borrowed,
            revenue: state.revenue,
            borrow_index: state.borrow_index,
            supply_index: state.supply_index,
            cash: state.cash,
        }
    }

    /// Persists the mutated state and returns the snapshot describing it, so a
    /// snapshot can never be published for a transition that was not written.
    pub(crate) fn commit(&self) -> MarketStateSnapshot {
        let state = PoolStateRaw {
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
            cash: self.cash,
        };
        storage::write_state(&self.env, &self.hub_asset, &state);
        self.snapshot()
    }

    // --- reads ---

    /// Borrows the environment this cache was loaded with.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Identifies the market this cache holds.
    pub(crate) fn hub_asset(&self) -> &HubAssetKey {
        &self.hub_asset
    }

    /// Borrows the market's risk and rate-model parameters.
    pub(crate) fn params(&self) -> &MarketParams {
        &self.params
    }

    /// Total supply shares outstanding — scaled, not an asset amount.
    pub(crate) fn supplied(&self) -> Ray {
        self.supplied
    }

    /// Total debt shares outstanding — scaled, not an asset amount.
    pub(crate) fn borrowed(&self) -> Ray {
        self.borrowed
    }

    /// The protocol-owned slice of the outstanding supply shares.
    pub(crate) fn revenue(&self) -> Ray {
        self.revenue
    }

    /// Current supply index.
    pub(crate) fn supply_index(&self) -> Ray {
        self.supply_index
    }

    /// Current borrow index.
    pub(crate) fn borrow_index(&self) -> Ray {
        self.borrow_index
    }

    /// Tracked cash in asset decimals.
    pub(crate) fn cash(&self) -> i128 {
        self.cash
    }

    // --- clock ---

    /// Returns milliseconds the market has not yet accrued for.
    pub(crate) fn elapsed_ms(&self) -> u64 {
        self.current_timestamp.saturating_sub(self.last_timestamp)
    }

    /// True while interest is owed for elapsed time.
    pub(crate) fn needs_accrual(&self) -> bool {
        self.elapsed_ms() > 0
    }

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
            let ratio = Ray::from_fraction(&self.env, amount, treasury_actual);
            self.revenue.mul(&self.env, ratio)
        };
        // A positive payout must always retire a positive part of the protocol
        // claim. At extreme cash-to-claim ratios both half-up operations above
        // can round to zero; paying in that state would leave revenue unchanged
        // and let the same claim absorb future cash repeatedly.
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

    // --- indexes ---

    /// Replaces the supply index.
    pub(crate) fn set_supply_index(&mut self, index: Ray) {
        self.supply_index = index;
    }

    /// Replaces the borrow index.
    pub(crate) fn set_borrow_index(&mut self, index: Ray) {
        self.borrow_index = index;
    }

    /// Marks the market accrued up to the current ledger time.
    pub(crate) fn mark_accrued(&mut self) {
        self.last_timestamp = self.current_timestamp;
    }

    // --- cash ---

    /// Rejects a payout that tracked cash cannot fund.
    ///
    /// # Errors
    /// * `InsufficientLiquidity` — tracked cash cannot cover `amount`.
    pub(crate) fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.cash >= amount,
            CollateralError::InsufficientLiquidity
        );
    }

    /// Adds `amount` to tracked cash.
    ///
    /// # Errors
    /// * `MathOverflow` — cash accounting overflows.
    pub(crate) fn credit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Removes `amount` from tracked cash.
    ///
    /// # Errors
    /// * `MathOverflow` — cash accounting overflows.
    pub(crate) fn debit_cash(&mut self, amount: i128) {
        self.cash = self
            .cash
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::MathOverflow));
    }

    /// Sends the market's asset out of the pool. Non-positive amounts are a
    /// no-op so callers need no zero guard.
    pub(crate) fn transfer_out(&self, recipient: &Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    // --- share scaling ---

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

    // --- reporting ---

    /// Reports both indexes in their raw ABI form.
    pub(crate) fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index: self.borrow_index.raw(),
            supply_index: self.supply_index.raw(),
        }
    }

    /// Returns the current in-memory state, committed or not. Prefer
    /// [`Cache::commit`], which persists and reports in one step.
    pub(crate) fn snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            hub_asset: self.hub_asset.clone(),
            timestamp: self.current_timestamp,
            supply_index: self.supply_index.raw(),
            borrow_index: self.borrow_index.raw(),
            // Asset-native cash, not a scaled RAY share.
            cash: self.cash,
            supplied: self.supplied.raw(),
            borrowed: self.borrowed.raw(),
            revenue: self.revenue.raw(),
        }
    }

    /// Builds the controller-facing result of a supply, borrow, withdraw, or
    /// repay leg. `actual_amount` is caller-defined: gross for withdraw and
    /// borrow, net of the refund for repay.
    pub(crate) fn position_mutation(
        &self,
        scaled: Ray,
        actual_amount: i128,
    ) -> PoolPositionMutation {
        PoolPositionMutation {
            position: ScaledPositionRaw {
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
            asset_decimals: self.params.asset_decimals,
        }
    }

    /// Builds the controller-facing result of a strategy borrow leg.
    pub(crate) fn strategy_mutation(
        &self,
        scaled: Ray,
        actual_amount: i128,
        amount_received: i128,
    ) -> PoolStrategyMutation {
        PoolStrategyMutation {
            position: ScaledPositionRaw {
                scaled_amount: scaled.raw(),
            },
            market_index: self.market_index(),
            actual_amount,
            amount_received,
            asset_decimals: self.params.asset_decimals,
        }
    }
}

/// Raw clock read. Production code works in deltas through
/// [`Cache::elapsed_ms`]; formal specs pin the absolute checkpoint.
#[cfg(any(test, feature = "certora"))]
impl Cache {
    /// Absolute accrual checkpoint in milliseconds.
    pub(crate) fn last_timestamp(&self) -> u64 {
        self.last_timestamp
    }
}

#[cfg(test)]
impl Cache {
    /// Builds a cache from explicit parts, bypassing storage, so unit tests can
    /// exercise one transition in isolation.
    pub(crate) fn from_parts(
        env: &Env,
        hub_asset: HubAssetKey,
        params: &common::types::MarketParamsRaw,
        state: &PoolStateRaw,
        current_timestamp: u64,
    ) -> Self {
        let parts = PoolState::from(state);
        Self {
            env: env.clone(),
            hub_asset,
            params: MarketParams::from(params),
            last_timestamp: parts.last_timestamp,
            current_timestamp,
            supplied: parts.supplied,
            borrowed: parts.borrowed,
            revenue: parts.revenue,
            borrow_index: parts.borrow_index,
            supply_index: parts.supply_index,
            cash: parts.cash,
        }
    }

    /// Ledger time this cache was loaded at, in milliseconds.
    pub(crate) fn current_timestamp(&self) -> u64 {
        self.current_timestamp
    }

    /// Moves the cache clock so a test can drive an elapsed interval.
    pub(crate) fn set_current_timestamp(&mut self, timestamp: u64) {
        self.current_timestamp = timestamp;
    }

    /// Forces tracked cash for a test fixture.
    pub(crate) fn set_cash(&mut self, cash: i128) {
        self.cash = cash;
    }

    /// Forces protocol revenue shares for a test fixture.
    pub(crate) fn set_revenue(&mut self, revenue: Ray) {
        self.revenue = revenue;
    }
}

#[cfg(test)]
#[path = "../tests/cache.rs"]
mod tests;
