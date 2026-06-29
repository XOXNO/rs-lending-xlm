use std::collections::HashMap;

use controller::types::PositionMode;
use soroban_sdk::{token, Address};

use crate::presets::{AssetConfigPreset, SpokePreset, MarketParamsPreset, MarketPreset};

pub struct UserState {
    pub address: Address,
    pub default_account_id: Option<u64>,
    pub accounts: Vec<AccountEntry>,
}

#[allow(dead_code)]
pub struct AccountEntry {
    pub account_id: u64,
    pub spoke_id: u32,
    pub mode: PositionMode,
}

pub struct MarketState {
    pub asset: Address,
    pub pool: Address,
    pub token_admin: token::StellarAssetClient<'static>,
    pub decimals: u32,
    pub price_wad: i128,
}

pub(crate) struct PendingMarket {
    pub name: &'static str,
    pub decimals: u32,
    pub price_wad: i128,
    pub initial_liquidity: f64,
    pub config: AssetConfigPreset,
    pub params: MarketParamsPreset,
    pub configure_oracle: bool,
}

impl PendingMarket {
    pub fn from_preset(preset: MarketPreset) -> Self {
        PendingMarket {
            name: preset.name,
            decimals: preset.decimals,
            price_wad: preset.price_wad,
            initial_liquidity: preset.initial_liquidity,
            config: preset.config,
            params: preset.params,
            configure_oracle: true,
        }
    }
}

pub(crate) struct PendingSpoke {
    pub category_id: u32,
    pub preset: SpokePreset,
    pub assets: Vec<(String, bool, bool)>,
}

pub struct LendingTest {
    pub env: soroban_sdk::Env,
    pub admin: Address,
    pub governance: Address,
    pub controller: Address,
    pub mock_reflector: Address,
    #[allow(dead_code)]
    pub aggregator: Address,
    pub keeper: Address,
    pub users: HashMap<String, UserState>,
    pub markets: HashMap<String, MarketState>,
}
