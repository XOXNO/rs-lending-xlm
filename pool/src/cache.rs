use common::errors::GenericError;
use common::fp::Ray;
use common::types::{
    AccountPosition, MarketIndex, MarketParams, MarketStateSnapshot, PoolAmountMutation, PoolKey,
    PoolPositionMutation, PoolState, PoolStrategyMutation,
};
use soroban_sdk::{panic_with_error, Env};

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
    /// Loads params + state from instance storage. Panics `PoolNotInitialized`
    /// if either is missing — both are set together in `__constructor`.
    pub fn load(env: &Env) -> Self {
        let params: MarketParams = env
            .storage()
            .instance()
            .get(&PoolKey::Params)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

        let s: PoolState = env
            .storage()
            .instance()
            .get(&PoolKey::State)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

        Cache {
            env: env.clone(),
            supplied: Ray::from_raw(s.supplied_ray),
            borrowed: Ray::from_raw(s.borrowed_ray),
            revenue: Ray::from_raw(s.revenue_ray),
            borrow_index: Ray::from_raw(s.borrow_index_ray),
            supply_index: Ray::from_raw(s.supply_index_ray),
            last_timestamp: s.last_timestamp,
            current_timestamp: env.ledger().timestamp() * 1000,
            params,
        }
    }

    /// Writes the current cache fields back to instance storage.
    pub fn save(&self) {
        let state = PoolState {
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
        };

        self.env.storage().instance().set(&PoolKey::State, &state);
    }

    /// Utilization in RAY: `(borrowed × borrow_index) / (supplied × supply_index)`.
    /// Returns `Ray::ZERO` when supply is zero or the product underflows.
    pub fn calculate_utilization(&self) -> Ray {
        if self.supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        let total_borrowed = self.borrowed.mul(&self.env, self.borrow_index);
        let total_supplied = self.supplied.mul(&self.env, self.supply_index);
        if total_supplied == Ray::ZERO {
            return Ray::ZERO;
        }
        total_borrowed.div(&self.env, total_supplied)
    }

    /// `true` if the pool's on-chain balance covers `amount`.
    pub fn has_reserves(&self, amount: i128) -> bool {
        let reserves = self.get_reserves_for(&self.params.asset_id);
        reserves >= amount
    }

    /// Panics `InsufficientLiquidity` when on-chain balance can't cover `amount`.
    pub fn require_reserves(&self, amount: i128) {
        if !self.has_reserves(amount) {
            panic_with_error!(
                self.env,
                common::errors::CollateralError::InsufficientLiquidity
            );
        }
    }

    /// Pool's live on-chain balance of `asset`.
    pub fn get_reserves_for(&self, asset: &soroban_sdk::Address) -> i128 {
        let token = soroban_sdk::token::Client::new(&self.env, asset);
        token.balance(&self.env.current_contract_address())
    }

    /// Transfers `amount` of the pool's asset to `recipient`. No-op on
    /// `amount <= 0` (covers dust burns, full fee absorption, zero overpayment).
    pub fn transfer_out(&self, recipient: &soroban_sdk::Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let tok = soroban_sdk::token::Client::new(&self.env, &self.params.asset_id);
        tok.transfer(&self.env.current_contract_address(), recipient, &amount);
    }

    /// Asset-decimal amount → RAY scaled: `from_asset(amount) / supply_index`.
    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.supply_index)
    }

    /// Asset-decimal amount → RAY scaled: `from_asset(amount) / borrow_index`.
    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.borrow_index)
    }

    /// Scaled → actual in RAY (stays in 27-decimal precision): `scaled * borrow_index`.
    pub fn calculate_original_borrow_ray(&self, scaled: Ray) -> Ray {
        scaled.mul(&self.env, self.borrow_index)
    }

    /// Scaled → actual in asset decimals (for token transfers).
    pub fn calculate_original_supply(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.supply_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Scaled → actual in asset decimals (for token transfers).
    pub fn calculate_original_borrow(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.borrow_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Picks `(scaled_to_burn, gross_asset_amount)` for a withdraw of `amount`:
    /// full when `amount >= current`, full when partial residual rounds to 0
    /// (dust-lock avoids stuck dust), else the requested partial.
    ///
    /// The dust-lock branch is mathematically unreachable in a clean single
    /// call (half-up rounding makes the windows non-overlapping by 1 ulp); it
    /// only fires when `pos_scaled` carries drift from many prior cycles.
    pub fn resolve_withdrawal(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_supply_actual = self.calculate_original_supply(pos_scaled);
        if amount >= current_supply_actual {
            return (pos_scaled, current_supply_actual);
        }
        let scaled = self.calculate_scaled_supply(amount);
        let remaining_actual = self.calculate_original_supply(pos_scaled - scaled);
        if remaining_actual == 0 {
            (pos_scaled, current_supply_actual)
        } else {
            (scaled, amount)
        }
    }

    /// Burns the reserve-covered share of protocol revenue and returns the
    /// payable asset-decimal amount. Treasury above reserves stays as future
    /// revenue; returns `0` when nothing is payable.
    pub fn burn_claimable_revenue(&mut self) -> i128 {
        let reserves = self.get_reserves_for(&self.params.asset_id);
        let treasury_actual = self.calculate_original_supply(self.revenue);
        let amount = reserves.min(treasury_actual);
        if amount <= 0 {
            return amount.max(0);
        }
        let scaled_to_burn = if amount >= treasury_actual {
            self.revenue
        } else {
            let ratio = Ray::from_raw(amount).div(&self.env, Ray::from_raw(treasury_actual));
            self.revenue.mul(&self.env, ratio)
        };
        self.revenue.checked_sub_assign(&self.env, scaled_to_burn);
        self.supplied.checked_sub_assign(&self.env, scaled_to_burn);
        amount
    }

    /// Picks `(scaled_to_burn, overpayment)` for a repay of `amount`. Full
    /// burn + positive overpayment when `amount >= current_debt`, else partial
    /// burn with zero overpayment.
    pub fn resolve_repay(&self, amount: i128, pos_scaled: Ray) -> (Ray, i128) {
        let current_debt = self.calculate_original_borrow(pos_scaled);
        if amount >= current_debt {
            (pos_scaled, amount - current_debt)
        } else {
            (self.calculate_scaled_borrow(amount), 0)
        }
    }

    /// Current borrow/supply indexes for the controller event stream.
    pub fn market_index(&self) -> MarketIndex {
        MarketIndex {
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
        }
    }

    /// Full market snapshot at current cache state; reads live reserves from
    /// the token contract.
    pub fn market_snapshot(&self) -> MarketStateSnapshot {
        MarketStateSnapshot {
            asset: self.params.asset_id.clone(),
            timestamp: self.current_timestamp,
            supply_index_ray: self.supply_index.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            reserves_ray: self.get_reserves_for(&self.params.asset_id),
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
            asset_price_wad: None,
        }
    }

    /// Mutation returned by supply / borrow / withdraw / repay / seize_position.
    pub fn position_mutation(
        &self,
        position: AccountPosition,
        actual_amount: i128,
    ) -> PoolPositionMutation {
        PoolPositionMutation {
            position,
            market_index: self.market_index(),
            market_state: self.market_snapshot(),
            actual_amount,
        }
    }

    /// Mutation returned by claim_revenue.
    pub fn amount_mutation(&self, actual_amount: i128) -> PoolAmountMutation {
        PoolAmountMutation {
            market_state: self.market_snapshot(),
            actual_amount,
        }
    }

    /// Mutation returned by create_strategy.
    pub fn strategy_mutation(
        &self,
        position: AccountPosition,
        actual_amount: i128,
        amount_received: i128,
    ) -> PoolStrategyMutation {
        PoolStrategyMutation {
            position,
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
        params: MarketParams,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            init_ledger(&env);

            let admin = Address::generate(&env);
            let params = MarketParams {
                max_borrow_rate_ray: 2 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
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
                supplied: Ray::from_raw(10 * RAY),
                borrowed: Ray::from_raw(5 * RAY),
                revenue: Ray::ZERO,
                borrow_index: Ray::from_raw(2 * RAY),
                supply_index: Ray::ZERO,
                last_timestamp: 0,
                current_timestamp: 1_000_000,
                params: t.params.clone(),
            };

            assert_eq!(cache.calculate_utilization(), Ray::ZERO);
        });
    }

    // Helper to build a fully-controlled cache for unit-level tests.
    fn cache_with(
        env: &Env,
        params: MarketParams,
        supplied: i128,
        borrowed: i128,
        revenue: i128,
        supply_index: i128,
        borrow_index: i128,
    ) -> Cache {
        Cache {
            env: env.clone(),
            supplied: Ray::from_raw(supplied),
            borrowed: Ray::from_raw(borrowed),
            revenue: Ray::from_raw(revenue),
            borrow_index: Ray::from_raw(borrow_index),
            supply_index: Ray::from_raw(supply_index),
            last_timestamp: 0,
            current_timestamp: 1_000_000,
            params,
        }
    }

    #[test]
    fn test_calculate_utilization_returns_zero_when_supplied_is_zero() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, t.params.clone(), 0, 5 * RAY, 0, RAY, RAY);
            assert_eq!(cache.calculate_utilization(), Ray::ZERO);
        });
    }

    #[test]
    fn test_calculate_utilization_returns_ratio_at_normal_state() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // 5 borrowed against 10 supplied at index 1 -> 50% utilization.
            let cache = cache_with(&t.env, t.params.clone(), 10 * RAY, 5 * RAY, 0, RAY, RAY);
            assert_eq!(cache.calculate_utilization(), Ray::from_raw(RAY / 2));
        });
    }

    #[test]
    fn test_resolve_repay_partial_returns_zero_overpayment() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // current_debt = 1 asset unit; partial repay of 0 (no debt cleared).
            let cache = cache_with(&t.env, t.params.clone(), 0, 10i128.pow(20), 0, RAY, RAY);
            let pos_scaled = Ray::from_raw(10i128.pow(20));
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
            let cache = cache_with(&t.env, t.params.clone(), 0, 10i128.pow(20), 0, RAY, RAY);
            let pos_scaled = Ray::from_raw(10i128.pow(20));
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
            let cache = cache_with(&t.env, t.params.clone(), 10i128.pow(20), 0, 0, RAY, RAY);
            let pos_scaled = Ray::from_raw(10i128.pow(20));
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
            let cache = cache_with(&t.env, t.params.clone(), supplied, 0, 0, RAY, RAY);
            let pos_scaled = Ray::from_raw(supplied);
            let (scaled, gross) = cache.resolve_withdrawal(2, pos_scaled);
            assert_eq!(scaled.raw(), 2 * 10i128.pow(20));
            assert_eq!(gross, 2);
        });
    }

    #[test]
    fn test_market_index_reflects_current_indexes() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let cache = cache_with(&t.env, t.params.clone(), 0, 0, 0, 2 * RAY, 3 * RAY);
            let idx = cache.market_index();
            assert_eq!(idx.supply_index_ray, 2 * RAY);
            assert_eq!(idx.borrow_index_ray, 3 * RAY);
        });
    }

    // Note: `amount_mutation` / `burn_claimable_revenue` aren't unit-tested
    // here because both call `get_reserves_for` (live token balance read),
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
            let mut a = Ray::from_raw(RAY);
            a.checked_sub_assign(&t.env, Ray::from_raw(2 * RAY));
        });
    }

    #[test]
    fn test_ray_checked_sub_assign_normal_case() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut a = Ray::from_raw(5 * RAY);
            a.checked_sub_assign(&t.env, Ray::from_raw(2 * RAY));
            assert_eq!(a, Ray::from_raw(3 * RAY));
        });
    }
}
