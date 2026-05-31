use common::constants::MS_PER_SECOND;
use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::scaled_to_original;
use common::types::{
    MarketIndexRaw, MarketParams, MarketParamsRaw, MarketStateSnapshot, PoolAmountMutation,
    PoolKey, PoolPositionMutation, PoolState, PoolStateRaw, PoolStrategyMutation,
    ScaledPositionRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

pub struct Cache {
    pub env: Env,
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
    pub current_timestamp: u64,
    pub params: MarketParams,
}

impl Cache {
    /// Loads the pool's immutable params and mutable interest state from
    /// instance storage. Panics with PoolNotInitialized if either record is absent.
    pub fn load(env: &Env) -> Self {
        let params: MarketParamsRaw = env
            .storage()
            .instance()
            .get(&PoolKey::Params)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

        let raw_state: PoolStateRaw = env
            .storage()
            .instance()
            .get(&PoolKey::State)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));
        let state = PoolState::from(&raw_state);
        let market_params = MarketParams::from(&params);
        let timestamp = env.ledger().timestamp() * MS_PER_SECOND;

        Cache {
            env: env.clone(),
            supplied: state.supplied,
            borrowed: state.borrowed,
            revenue: state.revenue,
            borrow_index: state.borrow_index,
            supply_index: state.supply_index,
            last_timestamp: state.last_timestamp,
            current_timestamp: timestamp,
            params: market_params,
        }
    }

    /// Persists the current interest state (indexes, supplied/borrowed totals,
    /// revenue, last accrual timestamp) back to instance storage.
    pub fn save(&self) {
        let state = PoolStateRaw {
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
        };

        self.env.storage().instance().set(&PoolKey::State, &state);
    }

    /// Current utilization = total_borrowed_value / total_supplied_value (RAY).
    /// Returns zero when supplied is zero (avoids div-by-zero).
    pub fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        let total_borrowed = scaled_to_original(&self.env, self.borrowed, self.borrow_index);
        let total_supplied = scaled_to_original(&self.env, self.supplied, self.supply_index);

        common::rates::utilization(&self.env, total_borrowed, total_supplied)
    }

    /// Returns true when the pool's current on-chain token balance is at least `amount`.
    pub fn has_reserves(&self, amount: i128) -> bool {
        let reserves = self.live_reserves();
        reserves >= amount
    }

    /// Panics with InsufficientLiquidity if live on-chain balance < amount.
    pub fn require_reserves(&self, amount: i128) {
        assert_with_error!(
            self.env,
            self.has_reserves(amount),
            common::errors::CollateralError::InsufficientLiquidity
        )
    }

    /// Live on-chain balance of the pool's asset held by this contract.
    pub fn live_reserves(&self) -> i128 {
        let token = soroban_sdk::token::Client::new(&self.env, &self.params.asset_id);
        token.balance(&self.env.current_contract_address())
    }

    /// Transfers pool asset to `recipient`; zero and negative amounts are no-ops.
    pub fn transfer_out(&self, recipient: &soroban_sdk::Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = soroban_sdk::token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    /// Converts an asset amount into scaled supply shares at the current index.
    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.supply_index)
    }

    /// Converts an asset amount into scaled debt shares at the current index.
    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.borrow_index)
    }

    /// Converts scaled supply shares to asset units using half-up rounding.
    pub fn unscale_supply(&self, scaled: Ray) -> i128 {
        scaled_to_original(&self.env, scaled, self.supply_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Converts supply shares to asset units rounded down for user credits.
    pub fn unscale_supply_floor(&self, scaled: Ray) -> i128 {
        scaled
            .mul_floor(&self.env, self.supply_index)
            .to_asset_floor(self.params.asset_decimals)
    }

    /// Converts scaled debt shares to asset units using half-up rounding.
    pub fn unscale_borrow(&self, scaled: Ray) -> i128 {
        scaled_to_original(&self.env, scaled, self.borrow_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Converts debt shares to asset units rounded up for user debits.
    pub fn unscale_borrow_ceil(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.borrow_index)
            .to_asset_ceil(self.params.asset_decimals)
    }

    /// Converts scaled debt shares to underlying debt in RAY.
    pub fn unscale_borrow_ray(&self, scaled: Ray) -> Ray {
        scaled_to_original(&self.env, scaled, self.borrow_index)
    }

    /// Resolves a withdrawal into scaled shares and gross asset amount.
    ///
    /// Full-close uses floor rounding so the pool never over-credits the user
    /// when burning the final scaled supply share.
    pub fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_supply_actual = self.unscale_supply(pos_scaled);
        let current_supply_floor = self.unscale_supply_floor(pos_scaled);
        if amount >= current_supply_actual {
            return (pos_scaled, current_supply_floor);
        }
        let scaled = self.calculate_scaled_supply(amount);
        let remaining_actual = self.unscale_supply(pos_scaled - scaled);
        if remaining_actual == 0 {
            (pos_scaled, current_supply_floor)
        } else {
            (scaled, amount)
        }
    }

    /// Burns claimable revenue shares, capped by live reserves and scaled revenue.
    pub fn burn_claimable_revenue(&mut self) -> i128 {
        let reserves = self.live_reserves();
        let treasury_actual = self.unscale_supply(self.revenue);
        let amount = reserves.min(treasury_actual);
        if amount <= 0 {
            return amount.max(0);
        }
        let scaled_to_burn = if amount >= treasury_actual {
            self.revenue
        } else {
            let ratio = Ray::from_fraction(&self.env, amount, treasury_actual);
            self.revenue.mul(&self.env, ratio)
        };
        self.revenue.checked_sub_assign(&self.env, scaled_to_burn);
        self.supplied.checked_sub_assign(&self.env, scaled_to_burn);
        amount
    }

    /// Resolves repayment into debt shares and overpayment refund.
    ///
    /// Full-close uses ceiling rounding so repayment cannot leave indexed dust.
    pub fn resolve_repay(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_debt_ceil = self.unscale_borrow_ceil(pos_scaled);
        if amount >= current_debt_ceil {
            (pos_scaled, amount - current_debt_ceil)
        } else {
            (self.calculate_scaled_borrow(amount), 0)
        }
    }

    /// Current borrow and supply indexes in event/wire form.
    pub fn market_index(&self) -> MarketIndexRaw {
        MarketIndexRaw {
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
        }
    }

    /// Snapshot emitted to indexers after each pool state mutation.
    pub fn market_snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            asset: self.params.asset_id.clone(),
            timestamp: self.current_timestamp,
            supply_index_ray: self.supply_index.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            reserves_ray: self.live_reserves(),
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
            asset_price_wad: None,
        }
    }

    /// Position mutation snapshot containing only the pool-owned scaled share.
    pub fn position_mutation(&self, scaled: Ray, actual_amount: i128) -> PoolPositionMutation {
        PoolPositionMutation {
            position: ScaledPositionRaw {
                scaled_amount_ray: scaled.raw(),
            },
            market_index: self.market_index(),
            market_state: self.market_snapshot(),
            actual_amount,
        }
    }

    /// Revenue claim mutation snapshot.
    pub fn amount_mutation(&self, actual_amount: i128) -> PoolAmountMutation {
        PoolAmountMutation {
            market_state: self.market_snapshot(),
            actual_amount,
        }
    }

    /// Strategy borrow mutation snapshot, including net amount sent to caller.
    pub fn strategy_mutation(
        &self,
        scaled: Ray,
        actual_amount: i128,
        amount_received: i128,
    ) -> PoolStrategyMutation {
        PoolStrategyMutation {
            position: ScaledPositionRaw {
                scaled_amount_ray: scaled.raw(),
            },
            market_index: self.market_index(),
            market_state: self.market_snapshot(),
            actual_amount,
            amount_received,
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::test_support::init_ledger;
    use common::constants::RAY;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    struct TestSetup {
        env: Env,
        contract: Address,
        params: MarketParamsRaw,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            init_ledger(&env);

            let admin = Address::generate(&env);
            let params = MarketParamsRaw {
                max_borrow_rate_ray: 2 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                max_utilization_ray: RAY * 95 / 100,
                reserve_factor_bps: 1_000,
                asset_id: Address::generate(&env),
                asset_decimals: 7,
            };
            let contract = env.register(crate::LiquidityPool, (admin.clone(), params.clone()));

            Self {
                env,
                contract,
                params,
            }
        }

        fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.contract, f)
        }
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #")]
    fn test_load_panics_when_state_is_missing() {
        let t = TestSetup::new();

        t.as_contract(|| {
            t.env.storage().instance().remove(&PoolKey::State);
            let _ = Cache::load(&t.env);
        });
    }

    #[test]
    fn test_calculate_utilization_returns_zero_when_supply_index_zeroes_total_supply() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let cache = Cache {
                env: t.env.clone(),
                supplied: Ray::from(10 * RAY),
                borrowed: Ray::from(5 * RAY),
                revenue: Ray::ZERO,
                borrow_index: Ray::from(2 * RAY),
                supply_index: Ray::ZERO,
                last_timestamp: 0,
                current_timestamp: 1_000_000,
                params: (&t.params).into(),
            };

            assert_eq!(cache.calculate_utilization(), Ray::ZERO);
        });
    }

    fn cache_with(
        env: &Env,
        params: &MarketParamsRaw,
        supplied: i128,
        borrowed: i128,
        revenue: i128,
        supply_index: i128,
        borrow_index: i128,
    ) -> Cache {
        Cache {
            env: env.clone(),
            supplied: Ray::from(supplied),
            borrowed: Ray::from(borrowed),
            revenue: Ray::from(revenue),
            borrow_index: Ray::from(borrow_index),
            supply_index: Ray::from(supply_index),
            last_timestamp: 0,
            current_timestamp: 1_000_000,
            params: params.into(),
        }
    }

    #[test]
    fn test_calculate_utilization_returns_zero_when_supplied_is_zero() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 0, 5 * RAY, 0, RAY, RAY);
            assert_eq!(cache.calculate_utilization(), Ray::ZERO);
        });
    }

    #[test]
    fn test_calculate_utilization_returns_ratio_at_normal_state() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // 5 borrowed against 10 supplied at index 1 -> 50% utilization.
            let cache = cache_with(&t.env, &t.params, 10 * RAY, 5 * RAY, 0, RAY, RAY);
            assert_eq!(cache.calculate_utilization(), Ray::from(RAY / 2));
        });
    }

    #[test]
    fn test_resolve_repay_partial_returns_zero_overpayment() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // current_debt = 1 asset unit; partial repay of 0 (no debt cleared).
            let cache = cache_with(&t.env, &t.params, 0, 10i128.pow(20), 0, RAY, RAY);
            let pos_scaled = Ray::from(10i128.pow(20));
            let (scaled, overpayment) = cache.resolve_repay(0, pos_scaled);
            assert_eq!(scaled, Ray::ZERO);
            assert_eq!(overpayment, 0);
        });
    }

    #[test]
    fn test_resolve_repay_full_returns_positive_overpayment() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // current_debt = 1; pay 5 -> overpayment = 4, burn full position.
            let cache = cache_with(&t.env, &t.params, 0, 10i128.pow(20), 0, RAY, RAY);
            let pos_scaled = Ray::from(10i128.pow(20));
            let (scaled, overpayment) = cache.resolve_repay(5, pos_scaled);
            assert_eq!(scaled, pos_scaled);
            assert_eq!(overpayment, 4);
        });
    }

    #[test]
    fn test_resolve_withdrawal_full_when_amount_exceeds_position() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // Position = 1 asset unit; request 100 -> full withdraw.
            let cache = cache_with(&t.env, &t.params, 10i128.pow(20), 0, 0, RAY, RAY);
            let pos_scaled = Ray::from(10i128.pow(20));
            let (scaled, gross) = cache.resolve_withdrawal(100, pos_scaled);
            assert_eq!(scaled, pos_scaled);
            assert_eq!(gross, 1);
        });
    }

    #[test]
    fn test_resolve_withdrawal_partial_returns_requested_amount() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // Position = 5 asset units; request 2 -> partial.
            let supplied = 5 * 10i128.pow(20);
            let cache = cache_with(&t.env, &t.params, supplied, 0, 0, RAY, RAY);
            let pos_scaled = Ray::from(supplied);
            let (scaled, gross) = cache.resolve_withdrawal(2, pos_scaled);
            assert_eq!(scaled.raw(), 2 * 10i128.pow(20));
            assert_eq!(gross, 2);
        });
    }

    #[test]
    fn test_market_index_reflects_current_indexes() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, &t.params, 0, 0, 0, 2 * RAY, 3 * RAY);
            let idx = cache.market_index();
            assert_eq!(idx.supply_index_ray, 2 * RAY);
            assert_eq!(idx.borrow_index_ray, 3 * RAY);
        });
    }

    // Note: `amount_mutation` / `burn_claimable_revenue` aren't unit-tested
    // here because both call `live_reserves` (live token balance read),
    // and this module's `TestSetup` uses a generated address rather than a
    // registered Stellar Asset Contract. Both are exercised via lib.rs
    // ABI-level tests (`test_claim_revenue*`).

    // Sub-tests for `Ray::checked_sub_assign` are covered by withdraw/seize
    // panic tests at the ABI layer, but a direct unit test gives a faster
    // failure signal when the helper is touched.
    #[test]
    #[should_panic(expected = "Error(Contract, #33)")]
    fn test_ray_checked_sub_assign_panics_on_underflow() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut a = Ray::from(RAY);
            a.checked_sub_assign(&t.env, Ray::from(2 * RAY));
        });
    }

    #[test]
    fn test_ray_checked_sub_assign_normal_case() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut a = Ray::from(5 * RAY);
            a.checked_sub_assign(&t.env, Ray::from(2 * RAY));
            assert_eq!(a, Ray::from(3 * RAY));
        });
    }
}
