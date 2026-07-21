use controller::constants::RAY;

use crate::helpers::usd;

pub const ALICE: &str = "alice";
pub const BOB: &str = "bob";
pub const CAROL: &str = "carol";
pub const DAVE: &str = "dave";
pub const EVE: &str = "eve";
pub const LIQUIDATOR: &str = "liquidator";
pub const KEEPER_USER: &str = "keeper";

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
    pub loan_to_value: u32,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub liquidation_fees: u32,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_flashloanable: bool,
    pub flashloan_fee: u32,
}

#[derive(Clone)]
pub struct MarketParamsPreset {
    pub max_borrow_rate: i128,
    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,
    pub mid_utilization: i128,
    pub optimal_utilization: i128,
    pub max_utilization: i128,
    pub reserve_factor: u32,
}

#[derive(Clone)]
pub struct SpokePreset {
    pub ltv: u32,
    pub threshold: u32,
    pub bonus: u32,
}

#[derive(Clone)]
pub struct TolerancePreset {
    /// Primary/anchor deviation tolerance in BPS.
    pub tolerance_bps: u32,
}

pub const DEFAULT_ASSET_CONFIG: AssetConfigPreset = AssetConfigPreset {
    loan_to_value: 7500,
    liquidation_threshold: 8000,
    liquidation_bonus: 500,
    liquidation_fees: 100,
    is_collateralizable: true,
    is_borrowable: true,
    is_flashloanable: true,
    flashloan_fee: 9,
};

pub const DEFAULT_MARKET_PARAMS: MarketParamsPreset = MarketParamsPreset {
    // `max_borrow_rate` capped at `MAX_BORROW_RATE_RAY = 2 * RAY` (the
    // compound-interest Taylor envelope). `slope3` must stay <= max.
    max_borrow_rate: 2 * RAY,
    base_borrow_rate: RAY / 100,
    slope1: RAY * 4 / 100,
    slope2: RAY * 10 / 100,
    slope3: RAY * 150 / 100,
    mid_utilization: RAY * 50 / 100,
    optimal_utilization: RAY * 80 / 100,
    // 95 % utilization ceiling — sits at or above `optimal` and below
    // `RAY`. Markets may tighten per asset class.
    max_utilization: RAY * 95 / 100,
    reserve_factor: 1000,
};

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
            loan_to_value: 9000,
            liquidation_threshold: 9500,
            liquidation_bonus: 200,
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

pub const STABLECOIN_SPOKE: SpokePreset = SpokePreset {
    ltv: 9700,
    threshold: 9800,
    bonus: 200,
};

pub const TIGHT_TOLERANCE: TolerancePreset = TolerancePreset { tolerance_bps: 300 };

pub const DEFAULT_TOLERANCE: TolerancePreset = TolerancePreset { tolerance_bps: 500 };

pub const LOOSE_TOLERANCE: TolerancePreset = TolerancePreset {
    tolerance_bps: 1000,
};

impl AssetConfigPreset {
    /// Build the per-spoke risk-listing arguments for `add_asset_to_spoke` on
    /// `spoke_id`, listing `asset` (already a created market on `hub_id`). The
    /// risk ratios, collateral/borrow flags, and protocol `liquidation_fees`
    /// come from the preset; spoke caps are disabled (hub caps live on
    /// `MarketParamsRaw`) and the asset keeps its token-rooted oracle.
    pub fn to_spoke_args(
        &self,
        hub_id: u32,
        asset: soroban_sdk::Address,
        spoke_id: u32,
    ) -> controller::types::SpokeAssetArgs {
        controller::types::SpokeAssetArgs {
            hub_id,
            asset,
            spoke_id,
            can_collateral: self.is_collateralizable,
            can_borrow: self.is_borrowable,
            paused: false,
            frozen: false,
            ltv: self.loan_to_value,
            threshold: self.liquidation_threshold,
            bonus: self.liquidation_bonus,
            liquidation_fees: self.liquidation_fees,
            supply_cap: 0,
            borrow_cap: 0,
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
            max_borrow_rate: self.max_borrow_rate,
            base_borrow_rate: self.base_borrow_rate,
            slope1: self.slope1,
            slope2: self.slope2,
            slope3: self.slope3,
            mid_utilization: self.mid_utilization,
            optimal_utilization: self.optimal_utilization,
            max_utilization: self.max_utilization,
            reserve_factor: self.reserve_factor,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals: decimals,
        }
    }
}
