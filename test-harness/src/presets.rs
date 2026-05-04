use common::constants::RAY;
use common::types::OraclePriceFluctuation;

use crate::helpers::usd;

// ---------------------------------------------------------------------------
// Wallet name constants
// ---------------------------------------------------------------------------

pub const ALICE: &str = "alice";
pub const BOB: &str = "bob";
pub const CAROL: &str = "carol";
pub const DAVE: &str = "dave";
pub const EVE: &str = "eve";
pub const LIQUIDATOR: &str = "liquidator";
pub const KEEPER_USER: &str = "keeper";

// ---------------------------------------------------------------------------
// Market preset structs
// ---------------------------------------------------------------------------

pub struct MarketPreset {
    pub name: &'static str,
    pub decimals: u32,
    pub price_wad: i128,
    pub initial_liquidity: f64,
    pub config: AssetConfigPreset,
    pub params: MarketParamsPreset,
}

#[derive(Clone)]
pub struct AssetConfigPreset {
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_flashloanable: bool,
    pub is_isolated_asset: bool,
    pub is_siloed_borrowing: bool,
    pub isolation_borrow_enabled: bool,
    pub isolation_debt_ceiling_usd_wad: i128,
    pub flashloan_fee_bps: u32,
    pub borrow_cap: i128,
    pub supply_cap: i128,
}

#[derive(Clone)]
pub struct MarketParamsPreset {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub reserve_factor_bps: u32,
}

#[derive(Clone)]
pub struct EModeCategoryPreset {
    pub ltv: u32,
    pub threshold: u32,
    pub bonus: u32,
}

#[derive(Clone)]
pub struct TolerancePreset {
    pub first_upper_bps: u32,
    pub first_lower_bps: u32,
    pub last_upper_bps: u32,
    pub last_lower_bps: u32,
}

// ---------------------------------------------------------------------------
// Default configs
// ---------------------------------------------------------------------------

pub const DEFAULT_ASSET_CONFIG: AssetConfigPreset = AssetConfigPreset {
    loan_to_value_bps: 7500,
    liquidation_threshold_bps: 8000,
    liquidation_bonus_bps: 500,
    liquidation_fees_bps: 100,
    is_collateralizable: true,
    is_borrowable: true,
    is_flashloanable: true,
    is_isolated_asset: false,
    is_siloed_borrowing: false,
    isolation_borrow_enabled: false,
    isolation_debt_ceiling_usd_wad: 0,
    flashloan_fee_bps: 9,
    borrow_cap: 0, // 0 = no cap (tests that need caps override per-market)
    supply_cap: 0, // 0 = no cap
};

pub const DEFAULT_MARKET_PARAMS: MarketParamsPreset = MarketParamsPreset {
    // `max_borrow_rate_ray` capped at `MAX_BORROW_RATE_RAY = 2 * RAY` (the
    // compound-interest Taylor envelope). `slope3_ray` must stay <= max.
    max_borrow_rate_ray: 2 * RAY,
    base_borrow_rate_ray: RAY / 100,
    slope1_ray: RAY * 4 / 100,
    slope2_ray: RAY * 10 / 100,
    slope3_ray: RAY * 150 / 100,
    mid_utilization_ray: RAY * 50 / 100,
    optimal_utilization_ray: RAY * 80 / 100,
    reserve_factor_bps: 1000,
};

// ---------------------------------------------------------------------------
// Market presets (functions instead of const due to f64 field)
// ---------------------------------------------------------------------------

pub fn usdc_preset() -> MarketPreset {
    MarketPreset {
        name: "USDC",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

pub fn usdt_stable_preset() -> MarketPreset {
    MarketPreset {
        name: "USDT",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: AssetConfigPreset {
            loan_to_value_bps: 9000,
            liquidation_threshold_bps: 9500,
            liquidation_bonus_bps: 200,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

pub fn eth_preset() -> MarketPreset {
    MarketPreset {
        name: "ETH",
        decimals: 7,
        price_wad: usd(2000),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

pub fn wbtc_preset() -> MarketPreset {
    MarketPreset {
        name: "WBTC",
        decimals: 7,
        price_wad: usd(60_000),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

pub fn xlm_preset() -> MarketPreset {
    MarketPreset {
        name: "XLM",
        decimals: 7,
        price_wad: usd(1) / 10, // $0.10
        initial_liquidity: 10_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

// ---------------------------------------------------------------------------
// E-Mode presets
// ---------------------------------------------------------------------------

pub const STABLECOIN_EMODE: EModeCategoryPreset = EModeCategoryPreset {
    ltv: 9700,
    threshold: 9800,
    bonus: 200,
};

// ---------------------------------------------------------------------------
// Tolerance presets
// ---------------------------------------------------------------------------

pub const TIGHT_TOLERANCE: TolerancePreset = TolerancePreset {
    first_upper_bps: 100,
    first_lower_bps: 100,
    last_upper_bps: 300,
    last_lower_bps: 300,
};

pub const DEFAULT_TOLERANCE: TolerancePreset = TolerancePreset {
    first_upper_bps: 200,
    first_lower_bps: 200,
    last_upper_bps: 500,
    last_lower_bps: 500,
};

pub const LOOSE_TOLERANCE: TolerancePreset = TolerancePreset {
    first_upper_bps: 500,
    first_lower_bps: 500,
    last_upper_bps: 1000,
    last_lower_bps: 1000,
};

// ---------------------------------------------------------------------------
// Conversion helpers (preset -> contract types)
// ---------------------------------------------------------------------------

impl AssetConfigPreset {
    pub fn to_asset_config(&self, env: &soroban_sdk::Env) -> common::types::AssetConfig {
        common::types::AssetConfig {
            loan_to_value_bps: self.loan_to_value_bps,
            liquidation_threshold_bps: self.liquidation_threshold_bps,
            liquidation_bonus_bps: self.liquidation_bonus_bps,
            liquidation_fees_bps: self.liquidation_fees_bps,
            is_collateralizable: self.is_collateralizable,
            is_borrowable: self.is_borrowable,
            is_flashloanable: self.is_flashloanable,
            is_isolated_asset: self.is_isolated_asset,
            is_siloed_borrowing: self.is_siloed_borrowing,
            isolation_borrow_enabled: self.isolation_borrow_enabled,
            isolation_debt_ceiling_usd_wad: self.isolation_debt_ceiling_usd_wad,
            flashloan_fee_bps: self.flashloan_fee_bps,
            borrow_cap: self.borrow_cap,
            supply_cap: self.supply_cap,
            // Memberships are populated via `add_asset_to_e_mode_category`,
            // never at preset → market construction time.
            e_mode_categories: soroban_sdk::Vec::new(env),
        }
    }
}

impl MarketParamsPreset {
    pub fn to_market_params(
        &self,
        asset: &soroban_sdk::Address,
        decimals: u32,
    ) -> common::types::MarketParams {
        common::types::MarketParams {
            max_borrow_rate_ray: self.max_borrow_rate_ray,
            base_borrow_rate_ray: self.base_borrow_rate_ray,
            slope1_ray: self.slope1_ray,
            slope2_ray: self.slope2_ray,
            slope3_ray: self.slope3_ray,
            mid_utilization_ray: self.mid_utilization_ray,
            optimal_utilization_ray: self.optimal_utilization_ray,
            reserve_factor_bps: self.reserve_factor_bps,
            asset_id: asset.clone(),
            asset_decimals: decimals,
        }
    }
}

impl TolerancePreset {
    pub fn to_oracle_tolerance(&self) -> OraclePriceFluctuation {
        OraclePriceFluctuation {
            first_upper_ratio_bps: self.first_upper_bps,
            first_lower_ratio_bps: self.first_lower_bps,
            last_upper_ratio_bps: self.last_upper_bps,
            last_lower_ratio_bps: self.last_lower_bps,
        }
    }
}
