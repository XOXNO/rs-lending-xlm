use controller::constants::RAY;

use crate::helpers::usd;
// Wallet name constants

pub const ALICE: &str = "alice";
pub const BOB: &str = "bob";
pub const CAROL: &str = "carol";
pub const DAVE: &str = "dave";
pub const EVE: &str = "eve";
pub const LIQUIDATOR: &str = "liquidator";
pub const KEEPER_USER: &str = "keeper";
// Market preset structs

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
    pub flashloan_fee_bps: u32,
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
    pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

#[derive(Clone)]
pub struct EModeCategoryPreset {
    pub ltv: u32,
    pub threshold: u32,
    pub bonus: u32,
}

#[derive(Clone)]
pub struct TolerancePreset {
    /// Primary/anchor deviation tolerance in BPS.
    pub tolerance_bps: u32,
}
// Default configs

pub const DEFAULT_ASSET_CONFIG: AssetConfigPreset = AssetConfigPreset {
    loan_to_value_bps: 7500,
    liquidation_threshold_bps: 8000,
    liquidation_bonus_bps: 500,
    liquidation_fees_bps: 100,
    is_collateralizable: true,
    is_borrowable: true,
    is_flashloanable: true,
    flashloan_fee_bps: 9,
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
    // 95 % utilization ceiling — sits at or above `optimal` and below
    // `RAY`. Markets may tighten per asset class.
    max_utilization_ray: RAY * 95 / 100,
    reserve_factor_bps: 1000,
    supply_cap: 0,
    borrow_cap: 0,
};
// Market presets (functions instead of const due to f64 field)

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
// E-Mode presets

pub const STABLECOIN_EMODE: EModeCategoryPreset = EModeCategoryPreset {
    ltv: 9700,
    threshold: 9800,
    bonus: 200,
};
// Tolerance presets

pub const TIGHT_TOLERANCE: TolerancePreset = TolerancePreset { tolerance_bps: 300 };

pub const DEFAULT_TOLERANCE: TolerancePreset = TolerancePreset { tolerance_bps: 500 };

pub const LOOSE_TOLERANCE: TolerancePreset = TolerancePreset {
    tolerance_bps: 1000,
};
// Conversion helpers (preset -> contract types)

impl AssetConfigPreset {
    pub fn to_asset_config(
        &self,
        env: &soroban_sdk::Env,
        decimals: u32,
    ) -> controller::types::AssetConfigRaw {
        controller::types::AssetConfigRaw {
            loan_to_value_bps: self.loan_to_value_bps,
            liquidation_threshold_bps: self.liquidation_threshold_bps,
            liquidation_bonus_bps: self.liquidation_bonus_bps,
            liquidation_fees_bps: self.liquidation_fees_bps,
            is_collateralizable: self.is_collateralizable,
            is_borrowable: self.is_borrowable,
            is_flashloanable: self.is_flashloanable,
            flashloan_fee_bps: self.flashloan_fee_bps,
            asset_decimals: decimals,
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
    ) -> controller::types::MarketParamsRaw {
        controller::types::MarketParamsRaw {
            max_borrow_rate_ray: self.max_borrow_rate_ray,
            base_borrow_rate_ray: self.base_borrow_rate_ray,
            slope1_ray: self.slope1_ray,
            slope2_ray: self.slope2_ray,
            slope3_ray: self.slope3_ray,
            mid_utilization_ray: self.mid_utilization_ray,
            optimal_utilization_ray: self.optimal_utilization_ray,
            max_utilization_ray: self.max_utilization_ray,
            reserve_factor_bps: self.reserve_factor_bps,
            supply_cap: self.supply_cap,
            borrow_cap: self.borrow_cap,
            asset_id: asset.clone(),
            asset_decimals: decimals,
        }
    }
}
