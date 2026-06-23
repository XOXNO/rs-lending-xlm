use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY, RAY_DECIMALS};
use crate::errors::CollateralError;
use crate::math::fp::{Bps, Ray};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, Env};

/// Persistent pool parameter encoding.
///
/// `*_ray` fields use 27-decimal RAY scale, `*_bps` fields use basis points,
/// and `asset_decimals` is the SAC token decimal count used for conversions.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketParamsRaw {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
    /// Hub supply cap in asset-native units; zero or `i128::MAX` disables.
    pub supply_cap: i128,
    /// Hub borrow cap in asset-native units; zero or `i128::MAX` disables.
    pub borrow_cap: i128,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl MarketParamsRaw {
    pub fn rate_model_view(&self) -> InterestRateModel {
        InterestRateModel {
            max_borrow_rate_ray: self.max_borrow_rate_ray,
            base_borrow_rate_ray: self.base_borrow_rate_ray,
            slope1_ray: self.slope1_ray,
            slope2_ray: self.slope2_ray,
            slope3_ray: self.slope3_ray,
            mid_utilization_ray: self.mid_utilization_ray,
            optimal_utilization_ray: self.optimal_utilization_ray,
            max_utilization_ray: self.max_utilization_ray,
            reserve_factor_bps: self.reserve_factor_bps,
        }
    }

    pub fn verify_rate_model(&self, env: &Env) {
        self.rate_model_view().verify(env);
    }

    // Boundary validation: rate model plus `asset_decimals <= RAY_DECIMALS`
    // to keep `Ray::from_asset` inside the supported decimal domain.
    pub fn verify(&self, env: &Env) {
        assert_with_error!(
            env,
            self.asset_decimals <= RAY_DECIMALS,
            CollateralError::AssetDecimalsTooHigh
        );
        assert_with_error!(
            env,
            self.supply_cap >= 0 && self.borrow_cap >= 0,
            CollateralError::InvalidBorrowParams
        );
        // Hub caps share the e-mode spoke-cap domain guard: reject any cap that
        // would overflow `Ray::from_asset` during cap previews so a misconfig
        // fails here at the boundary, not as a runtime MathOverflow in a view.
        crate::validation::require_cap_within_asset_domain(env, self.supply_cap, self.asset_decimals);
        crate::validation::require_cap_within_asset_domain(env, self.borrow_cap, self.asset_decimals);
        self.verify_rate_model(env);
    }
}

/// Typed pool parameters used by interest and cap math.
#[derive(Clone, Debug)]
pub struct MarketParams {
    pub max_borrow_rate: Ray,
    pub base_borrow_rate: Ray,
    pub slope1: Ray,
    pub slope2: Ray,
    pub slope3: Ray,
    pub mid_utilization: Ray,
    pub optimal_utilization: Ray,
    pub max_utilization: Ray,
    pub reserve_factor: Bps,
    pub supply_cap: i128,
    pub borrow_cap: i128,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl From<&MarketParamsRaw> for MarketParams {
    fn from(r: &MarketParamsRaw) -> Self {
        Self {
            max_borrow_rate: Ray::from(r.max_borrow_rate_ray),
            base_borrow_rate: Ray::from(r.base_borrow_rate_ray),
            slope1: Ray::from(r.slope1_ray),
            slope2: Ray::from(r.slope2_ray),
            slope3: Ray::from(r.slope3_ray),
            mid_utilization: Ray::from(r.mid_utilization_ray),
            optimal_utilization: Ray::from(r.optimal_utilization_ray),
            max_utilization: Ray::from(r.max_utilization_ray),
            reserve_factor: Bps::from(i128::from(r.reserve_factor_bps)),
            supply_cap: r.supply_cap,
            borrow_cap: r.borrow_cap,
            asset_id: r.asset_id.clone(),
            asset_decimals: r.asset_decimals,
        }
    }
}

impl From<&MarketParams> for MarketParamsRaw {
    fn from(t: &MarketParams) -> Self {
        Self {
            max_borrow_rate_ray: t.max_borrow_rate.raw(),
            base_borrow_rate_ray: t.base_borrow_rate.raw(),
            slope1_ray: t.slope1.raw(),
            slope2_ray: t.slope2.raw(),
            slope3_ray: t.slope3.raw(),
            mid_utilization_ray: t.mid_utilization.raw(),
            optimal_utilization_ray: t.optimal_utilization.raw(),
            max_utilization_ray: t.max_utilization.raw(),
            reserve_factor_bps: t.reserve_factor.raw() as u32,
            supply_cap: t.supply_cap,
            borrow_cap: t.borrow_cap,
            asset_id: t.asset_id.clone(),
            asset_decimals: t.asset_decimals,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct InterestRateModel {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
}

impl InterestRateModel {
    pub fn verify(&self, env: &Env) {
        assert_with_error!(
            env,
            self.base_borrow_rate_ray >= 0,
            CollateralError::BaseRateNegative
        );
        if self.slope1_ray < self.base_borrow_rate_ray
            || self.slope2_ray < self.slope1_ray
            || self.slope3_ray < self.slope2_ray
            || self.max_borrow_rate_ray < self.slope3_ray
        {
            panic_with_error!(env, CollateralError::SlopeNonMonotonic);
        }
        assert_with_error!(
            env,
            self.max_borrow_rate_ray > self.base_borrow_rate_ray,
            CollateralError::MaxRateBelowBase
        );
        assert_with_error!(
            env,
            self.max_borrow_rate_ray <= MAX_BORROW_RATE_RAY,
            CollateralError::MaxBorrowRateTooHigh
        );
        assert_with_error!(
            env,
            self.mid_utilization_ray > 0,
            CollateralError::InvalidUtilRange
        );
        assert_with_error!(
            env,
            self.optimal_utilization_ray > self.mid_utilization_ray,
            CollateralError::InvalidUtilRange
        );
        assert_with_error!(
            env,
            self.optimal_utilization_ray < RAY,
            CollateralError::OptUtilTooHigh
        );
        if self.max_utilization_ray < self.optimal_utilization_ray || self.max_utilization_ray > RAY
        {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        assert_with_error!(
            env,
            i128::from(self.reserve_factor_bps) < BPS,
            CollateralError::InvalidReserveFactor
        );
    }
}

/// Persistent collateral position encoding.
///
/// `scaled_amount_ray` is a supply share, not current underlying balance.
/// Risk fields are snapshotted by the controller for HF/LTV/liquidation math.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPositionRaw {
    pub scaled_amount_ray: i128,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub loan_to_value_bps: u32,
}

/// Typed collateral position used by controller risk math.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount: Ray,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub loan_to_value: Bps,
}

impl From<&AccountPositionRaw> for AccountPosition {
    fn from(r: &AccountPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount_ray),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus_bps)),
            loan_to_value: Bps::from(i128::from(r.loan_to_value_bps)),
        }
    }
}

impl From<&AccountPosition> for AccountPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
            liquidation_threshold_bps: t.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: t.liquidation_bonus.raw() as u32,
            loan_to_value_bps: t.loan_to_value.raw() as u32,
        }
    }
}

/// Pool ABI position shape containing only scaled shares.
///
/// Collateral risk parameters stay on the controller and do not cross the pool
/// boundary.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaledPositionRaw {
    pub scaled_amount_ray: i128,
}

impl From<&AccountPosition> for ScaledPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
        }
    }
}

impl From<&DebtPosition> for ScaledPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
        }
    }
}

/// Persistent debt position encoding.
///
/// `scaled_amount_ray` is a borrow share, not current underlying debt.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtPositionRaw {
    pub scaled_amount_ray: i128,
}

/// Typed debt position used by borrow-index accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebtPosition {
    pub scaled_amount: Ray,
}

impl From<&DebtPositionRaw> for DebtPosition {
    fn from(r: &DebtPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount_ray),
        }
    }
}

// Pool returns the post-mutation scaled share, which is the full debt position.
impl From<&ScaledPositionRaw> for DebtPosition {
    fn from(r: &ScaledPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount_ray),
        }
    }
}

impl From<&DebtPosition> for DebtPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount_ray: t.scaled_amount.raw(),
        }
    }
}

/// Borrow and supply indexes in RAY scale.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndexRaw {
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
}

/// Typed borrow and supply indexes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index: Ray,
    pub supply_index: Ray,
}

impl From<&MarketIndexRaw> for MarketIndex {
    fn from(r: &MarketIndexRaw) -> Self {
        Self {
            borrow_index: Ray::from(r.borrow_index_ray),
            supply_index: Ray::from(r.supply_index_ray),
        }
    }
}

impl From<&MarketIndex> for MarketIndexRaw {
    fn from(t: &MarketIndex) -> Self {
        Self {
            borrow_index_ray: t.borrow_index.raw(),
            supply_index_ray: t.supply_index.raw(),
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketStateSnapshot {
    /// Pool asset whose state was updated.
    pub asset: Address,
    /// Millisecond timestamp used for the accrual checkpoint.
    pub timestamp: u64,
    /// Supply index after accrual, in RAY.
    pub supply_index_ray: i128,
    /// Borrow index after accrual, in RAY.
    pub borrow_index_ray: i128,
    /// Live pool token balance in asset-native units despite the legacy suffix.
    pub reserves_ray: i128,
    /// Total scaled supply shares.
    pub supplied_ray: i128,
    /// Total scaled borrow shares.
    pub borrowed_ray: i128,
    /// Scaled protocol revenue shares.
    pub revenue_ray: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolPositionMutation {
    pub position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: ScaledPositionRaw,
    pub market_index: MarketIndexRaw,
    pub actual_amount: i128,
    pub amount_received: i128,
}

impl From<&PoolStrategyMutation> for PoolPositionMutation {
    fn from(m: &PoolStrategyMutation) -> Self {
        Self {
            position: m.position.clone(),
            market_index: m.market_index.clone(),
            actual_amount: m.actual_amount,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAmountMutation {
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSyncData {
    pub params: MarketParamsRaw,
    pub state: PoolStateRaw,
}

/// Persistent storage keys of the central pool, keyed by market asset.
#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params(Address),
    State(Address),
}

/// Asset-scoped mutation payload for the central pool ABI.
///
/// The funds counterparty (receiver/payer) is carried by endpoint arguments,
/// shared by each entry in a bulk call.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAction {
    pub position: ScaledPositionRaw,
    pub amount: i128,
    pub asset: Address,
}

/// One asset of a bulk pool `supply`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSupplyEntry {
    pub action: PoolAction,
}

/// One asset of a bulk pool `borrow`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolBorrowEntry {
    pub action: PoolAction,
}

/// One asset of a bulk pool `withdraw`. The liquidation flag is per call;
/// the protocol fee scales with each asset's value and stays per entry.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolWithdrawEntry {
    pub action: PoolAction,
    pub protocol_fee: i128,
}

/// Persistent pool accounting state.
///
/// Supply, borrow, and revenue totals are scaled shares; indexes convert them
/// to current underlying amounts.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStateRaw {
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
    pub last_timestamp: u64,
    /// Liquid token units the pool holds (available reserves), tracked internally
    /// on each in/out flow instead of reading the token balance. Direct donations
    /// cannot inflate borrowable liquidity.
    pub cash: i128,
}

/// Typed pool accounting state.
#[derive(Clone, Debug)]
pub struct PoolState {
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
    /// Liquid token units held by the pool (available reserves).
    pub cash: i128,
}

impl From<&PoolStateRaw> for PoolState {
    fn from(r: &PoolStateRaw) -> Self {
        Self {
            supplied: Ray::from(r.supplied_ray),
            borrowed: Ray::from(r.borrowed_ray),
            revenue: Ray::from(r.revenue_ray),
            borrow_index: Ray::from(r.borrow_index_ray),
            supply_index: Ray::from(r.supply_index_ray),
            last_timestamp: r.last_timestamp,
            cash: r.cash,
        }
    }
}

impl From<&PoolState> for PoolStateRaw {
    fn from(t: &PoolState) -> Self {
        Self {
            supplied_ray: t.supplied.raw(),
            borrowed_ray: t.borrowed.raw(),
            revenue_ray: t.revenue.raw(),
            borrow_index_ray: t.borrow_index.raw(),
            supply_index_ray: t.supply_index.raw(),
            last_timestamp: t.last_timestamp,
            cash: t.cash,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn asset(env: &Env) -> Address {
        Address::generate(env)
    }

    fn sample_raw_params(env: &Env) -> MarketParamsRaw {
        MarketParamsRaw {
            max_borrow_rate_ray: RAY,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY / 20,
            slope2_ray: RAY / 10,
            slope3_ray: RAY / 2,
            mid_utilization_ray: RAY / 2,
            optimal_utilization_ray: RAY * 8 / 10,
            max_utilization_ray: RAY * 95 / 100,
            reserve_factor_bps: 1_000,
            asset_id: asset(env),
            asset_decimals: 7,
            supply_cap: 0,
            borrow_cap: 0,
        }
    }

    #[test]
    fn test_market_params_raw_typed_roundtrip() {
        let env = Env::default();
        let raw = sample_raw_params(&env);
        let typed = MarketParams::from(&raw);
        let back = MarketParamsRaw::from(&typed);
        assert_eq!(back.max_borrow_rate_ray, raw.max_borrow_rate_ray);
        assert_eq!(back.base_borrow_rate_ray, raw.base_borrow_rate_ray);
        assert_eq!(back.slope1_ray, raw.slope1_ray);
        assert_eq!(back.slope2_ray, raw.slope2_ray);
        assert_eq!(back.slope3_ray, raw.slope3_ray);
        assert_eq!(back.mid_utilization_ray, raw.mid_utilization_ray);
        assert_eq!(back.optimal_utilization_ray, raw.optimal_utilization_ray);
        assert_eq!(back.max_utilization_ray, raw.max_utilization_ray);
        assert_eq!(back.reserve_factor_bps, raw.reserve_factor_bps);
        assert_eq!(back.asset_id, raw.asset_id);
        assert_eq!(back.asset_decimals, raw.asset_decimals);
    }

    #[test]
    fn test_market_params_rate_model_view_copies_fields() {
        let env = Env::default();
        let raw = sample_raw_params(&env);
        let model = raw.rate_model_view();
        assert_eq!(model.max_borrow_rate_ray, raw.max_borrow_rate_ray);
        assert_eq!(model.base_borrow_rate_ray, raw.base_borrow_rate_ray);
        assert_eq!(model.slope1_ray, raw.slope1_ray);
        assert_eq!(model.slope2_ray, raw.slope2_ray);
        assert_eq!(model.slope3_ray, raw.slope3_ray);
        assert_eq!(model.mid_utilization_ray, raw.mid_utilization_ray);
        assert_eq!(model.optimal_utilization_ray, raw.optimal_utilization_ray);
        assert_eq!(model.max_utilization_ray, raw.max_utilization_ray);
        assert_eq!(model.reserve_factor_bps, raw.reserve_factor_bps);
    }

    #[test]
    fn test_market_params_verify_accepts_valid_config() {
        let env = Env::default();
        sample_raw_params(&env).verify(&env);
    }

    #[test]
    #[should_panic(expected = "#132")]
    fn test_market_params_verify_rejects_decimals_above_ray() {
        let env = Env::default();
        let mut raw = sample_raw_params(&env);
        raw.asset_decimals = RAY_DECIMALS + 1;
        raw.verify(&env);
    }

    #[test]
    fn test_account_position_raw_typed_roundtrip() {
        let raw = AccountPositionRaw {
            scaled_amount_ray: 12_345 * RAY,
            liquidation_threshold_bps: 8_500,
            liquidation_bonus_bps: 500,
            loan_to_value_bps: 8_000,
        };
        let typed = AccountPosition::from(&raw);
        let back = AccountPositionRaw::from(&typed);
        assert_eq!(back, raw);
    }

    #[test]
    fn test_market_index_raw_typed_roundtrip() {
        let raw = MarketIndexRaw {
            borrow_index_ray: RAY + RAY / 10,
            supply_index_ray: RAY + RAY / 20,
        };
        let typed = MarketIndex::from(&raw);
        let back = MarketIndexRaw::from(&typed);
        assert_eq!(back, raw);
    }

    #[test]
    fn test_pool_state_raw_typed_roundtrip() {
        let raw = PoolStateRaw {
            supplied_ray: 100 * RAY,
            borrowed_ray: 60 * RAY,
            revenue_ray: 5 * RAY,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: 1_700_000_000_000,
            cash: 40_000_000,
        };
        let typed = PoolState::from(&raw);
        let back = PoolStateRaw::from(&typed);
        assert_eq!(back.cash, raw.cash);
        assert_eq!(back.supplied_ray, raw.supplied_ray);
        assert_eq!(back.borrowed_ray, raw.borrowed_ray);
        assert_eq!(back.revenue_ray, raw.revenue_ray);
        assert_eq!(back.borrow_index_ray, raw.borrow_index_ray);
        assert_eq!(back.supply_index_ray, raw.supply_index_ray);
        assert_eq!(back.last_timestamp, raw.last_timestamp);
    }
    // InterestRateModel::verify boundary coverage.
    //
    // Slope-monotonicity and max-utilization guards use plain `if { panic }`
    // blocks, so comparison and `||` mutations are observable here. The
    // `assert_with_error!` checks (base >= 0, max > base, <= MAX_BORROW_RATE_RAY,
    // mid > 0, optimal > mid, optimal < RAY, reserve < BPS) hide their conditions
    // in macro arguments and are not targeted here.

    fn valid_rate_model() -> InterestRateModel {
        InterestRateModel {
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY / 10,
            slope2_ray: RAY * 2 / 10,
            slope3_ray: RAY * 3 / 10,
            max_borrow_rate_ray: RAY,
            mid_utilization_ray: RAY / 2,
            optimal_utilization_ray: RAY * 8 / 10,
            max_utilization_ray: RAY * 9 / 10,
            reserve_factor_bps: 1_000,
        }
    }

    #[test]
    fn test_rate_model_verify_accepts_valid() {
        let env = Env::default();
        valid_rate_model().verify(&env);
    }

    // `replace verify with ()`: invalid input must panic, catching a stubbed body.
    #[test]
    #[should_panic(expected = "#129")]
    fn test_rate_model_verify_body_is_not_a_noop() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.slope2_ray = m.slope1_ray - 1; // slope2 < slope1 → non-monotonic.
        m.verify(&env);
    }

    // Monotonic chain: `||` short-circuit.
    // Each test makes one disjunct true and the rest false: `||` panics,
    // while `&&` does not.

    #[test]
    #[should_panic(expected = "#129")]
    fn test_rate_model_monotonic_only_slope1_below_base_panics() {
        let env = Env::default();
        let mut m = valid_rate_model();
        // slope1 < base, but keep slope2/slope3/max above their predecessors.
        m.base_borrow_rate_ray = RAY * 2 / 10;
        m.slope1_ray = RAY / 10;
        m.slope2_ray = RAY * 3 / 10;
        m.slope3_ray = RAY * 4 / 10;
        m.max_borrow_rate_ray = RAY * 5 / 10;
        m.verify(&env);
    }

    #[test]
    #[should_panic(expected = "#129")]
    fn test_rate_model_monotonic_only_slope2_below_slope1_panics() {
        let env = Env::default();
        let mut m = valid_rate_model();
        // slope2 < slope1 only.
        m.slope1_ray = RAY * 3 / 10;
        m.slope2_ray = RAY * 2 / 10;
        m.slope3_ray = RAY * 4 / 10;
        m.max_borrow_rate_ray = RAY * 5 / 10;
        m.verify(&env);
    }

    #[test]
    #[should_panic(expected = "#129")]
    fn test_rate_model_monotonic_only_slope3_below_slope2_panics() {
        let env = Env::default();
        let mut m = valid_rate_model();
        // slope3 < slope2 only.
        m.slope2_ray = RAY * 4 / 10;
        m.slope3_ray = RAY * 3 / 10;
        m.max_borrow_rate_ray = RAY * 5 / 10;
        m.verify(&env);
    }

    #[test]
    #[should_panic(expected = "#129")]
    fn test_rate_model_monotonic_only_max_below_slope3_panics() {
        let env = Env::default();
        let mut m = valid_rate_model();
        // max < slope3 only, while max still > base (avoids MaxRateBelowBase).
        m.slope3_ray = RAY * 5 / 10;
        m.max_borrow_rate_ray = RAY * 3 / 10;
        m.verify(&env);
    }

    // Monotonic chain: `<` vs `<=`/`==` at exact equality.
    // At `a == b`, `<` is false. `<=` or `==` would panic.

    #[test]
    fn test_rate_model_monotonic_slope1_eq_base_does_not_panic() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.slope1_ray = m.base_borrow_rate_ray; // slope1 == base.
        m.verify(&env);
    }

    #[test]
    fn test_rate_model_monotonic_slope2_eq_slope1_does_not_panic() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.slope2_ray = m.slope1_ray; // slope2 == slope1.
        m.verify(&env);
    }

    #[test]
    fn test_rate_model_monotonic_slope3_eq_slope2_does_not_panic() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.slope3_ray = m.slope2_ray; // slope3 == slope2.
        m.verify(&env);
    }

    #[test]
    fn test_rate_model_monotonic_max_eq_slope3_does_not_panic() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.max_borrow_rate_ray = m.slope3_ray; // max == slope3.
        m.verify(&env);
    }

    // Max-utilization guard: `max_util < optimal || max_util > RAY`.

    // `||` vs `&&`: only the left disjunct is true.
    #[test]
    #[should_panic(expected = "#117")]
    fn test_rate_model_max_util_below_optimal_panics() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.max_utilization_ray = m.optimal_utilization_ray - 1;
        m.verify(&env);
    }

    // `||` vs `&&`: only the right disjunct is true.
    #[test]
    #[should_panic(expected = "#117")]
    fn test_rate_model_max_util_above_ray_panics() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.max_utilization_ray = RAY + 1;
        m.verify(&env);
    }

    // `max_util < optimal`, `<` vs `<=`/`==` at equality: at max_util == optimal,
    // `<` is false. Right disjunct is also false (optimal < RAY).
    #[test]
    fn test_rate_model_max_util_eq_optimal_does_not_panic() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.max_utilization_ray = m.optimal_utilization_ray; // == optimal.
        m.verify(&env);
    }

    // `max_util > RAY`, `>` vs `>=`/`==` at equality: at max_util == RAY,
    // `>` is false and the left disjunct is false.
    #[test]
    fn test_rate_model_max_util_eq_ray_does_not_panic() {
        let env = Env::default();
        let mut m = valid_rate_model();
        m.max_utilization_ray = RAY; // == RAY (upper edge of valid range).
        m.verify(&env);
    }

    // `verify_rate_model with ()`: wrapper delegates to `rate_model_view().verify()`.
    // Non-monotonic slopes must panic.
    #[test]
    #[should_panic(expected = "#129")]
    fn test_market_params_verify_rate_model_delegates() {
        let env = Env::default();
        let mut raw = sample_raw_params(&env);
        raw.slope2_ray = raw.slope1_ray - 1; // slope2 < slope1.
        raw.verify_rate_model(&env);
    }
}
