use crate::math::fp::{Bps, Ray};
use crate::types::oracle::PriceFeedRaw;
use crate::types::pool::{
    AccountPosition, AccountPositionRaw, DebtPosition, DebtPositionRaw, HubAssetKey,
};
use crate::types::shared::PositionMode;
use soroban_sdk::{contracttype, Address, Map, Vec};

#[derive(Clone, Debug)]
pub struct AssetConfig {
    pub loan_to_value: Bps,

    pub liquidation_threshold: Bps,

    pub liquidation_bonus: Bps,

    pub liquidation_fees: Bps,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
}

impl AssetConfig {
    pub fn can_supply(&self) -> bool {
        self.is_collateralizable
    }

    pub fn can_borrow(&self) -> bool {
        self.is_borrowable
    }
}

impl From<&SpokeAssetConfig> for AssetConfig {
    fn from(c: &SpokeAssetConfig) -> Self {
        Self {
            loan_to_value: Bps::from(i128::from(c.loan_to_value)),
            liquidation_threshold: Bps::from(i128::from(c.liquidation_threshold)),
            liquidation_bonus: Bps::from(i128::from(c.liquidation_bonus)),
            liquidation_fees: Bps::from(i128::from(c.liquidation_fees)),
            is_collateralizable: c.is_collateralizable,
            is_borrowable: c.is_borrowable,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountAttributes {
    pub spoke_id: u32,
    pub mode: PositionMode,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountMeta {
    pub owner: Address,
    pub spoke_id: u32,
    pub mode: PositionMode,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionManagerConfig {
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeConfig {
    pub is_deprecated: bool,

    pub liquidation_target_hf_wad: i128,

    pub hf_for_max_bonus_wad: i128,

    pub liquidation_bonus_factor_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub paused: bool,
    pub frozen: bool,

    pub loan_to_value: u32,

    pub liquidation_threshold: u32,

    pub liquidation_bonus: u32,

    pub liquidation_fees: u32,

    pub supply_cap: i128,

    pub borrow_cap: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeAssetArgs {
    pub hub_id: u32,
    pub asset: Address,
    pub spoke_id: u32,
    pub can_collateral: bool,
    pub can_borrow: bool,
    pub paused: bool,
    pub frozen: bool,

    pub ltv: u32,

    pub threshold: u32,

    pub bonus: u32,

    pub liquidation_fees: u32,

    pub supply_cap: i128,

    pub borrow_cap: i128,
}

#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct SpokeUsageRaw {
    pub supplied_scaled_ray: i128,

    pub borrowed_scaled_ray: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketIndexView {
    pub asset: Address,

    pub supply_index: i128,

    pub borrow_index: i128,

    pub price_wad: i128,

    pub primary_price_wad: i128,

    pub anchor_price_wad: i128,

    pub price_timestamp: u64,
    pub stale: bool,
    pub deviation: bool,

    pub valid: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionLimits {
    pub max_borrow_positions: u32,
    pub max_supply_positions: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PaymentTuple {
    pub asset: Address,

    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidationEstimate {
    pub seized_collaterals: Vec<PaymentTuple>,

    pub protocol_fees: Vec<PaymentTuple>,

    pub refunds: Vec<PaymentTuple>,

    pub max_payment_wad: i128,

    pub bonus_rate_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub hub_asset: HubAssetKey,

    pub amount: i128,

    pub protocol_fee: i128,
    pub feed: PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub hub_asset: HubAssetKey,

    pub amount: i128,

    pub usd_wad: i128,
    pub feed: PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,

    pub max_debt_usd: i128,

    pub bonus_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Account {
    pub owner: Address,

    pub spoke_id: u32,
    pub mode: PositionMode,

    pub supply_positions: Map<HubAssetKey, AccountPositionRaw>,

    pub borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
}

impl Account {
    pub fn attributes(&self) -> AccountAttributes {
        AccountAttributes::from(self)
    }

    pub fn get_or_create_supply_position(
        &self,
        hub_asset: &HubAssetKey,
        config: &AssetConfig,
    ) -> AccountPosition {
        self.supply_positions
            .get(hub_asset.clone())
            .map(|raw| AccountPosition::from(&raw))
            .unwrap_or(AccountPosition {
                scaled_amount: Ray::ZERO,
                liquidation_threshold: config.liquidation_threshold,
                liquidation_bonus: config.liquidation_bonus,
                loan_to_value: config.loan_to_value,
                liquidation_fees: config.liquidation_fees,
            })
    }

    pub fn get_or_create_debt_position(&self, hub_asset: &HubAssetKey) -> DebtPosition {
        self.borrow_positions
            .get(hub_asset.clone())
            .map(|raw| DebtPosition::from(&raw))
            .unwrap_or(DebtPosition {
                scaled_amount: Ray::ZERO,
            })
    }

    pub fn is_empty(&self) -> bool {
        self.supply_positions.is_empty() && self.borrow_positions.is_empty()
    }

    pub fn debt_free(&self) -> bool {
        self.borrow_positions.is_empty()
    }
}

impl From<&Account> for AccountAttributes {
    fn from(account: &Account) -> Self {
        AccountAttributes {
            spoke_id: account.spoke_id,
            mode: account.mode,
        }
    }
}

impl From<&AccountMeta> for AccountAttributes {
    fn from(account: &AccountMeta) -> Self {
        AccountAttributes {
            spoke_id: account.spoke_id,
            mode: account.mode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn sample_spoke_asset_config() -> SpokeAssetConfig {
        SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: true,
            paused: false,
            frozen: false,
            loan_to_value: 7_500,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            liquidation_fees: 100,
            supply_cap: 0,
            borrow_cap: 0,
        }
    }

    #[test]
    fn test_asset_config_projects_spoke_asset_risk() {
        let spoke = sample_spoke_asset_config();
        let cfg = AssetConfig::from(&spoke);
        assert_eq!(cfg.loan_to_value.raw() as u32, spoke.loan_to_value);
        assert_eq!(
            cfg.liquidation_threshold.raw() as u32,
            spoke.liquidation_threshold
        );
        assert_eq!(cfg.liquidation_bonus.raw() as u32, spoke.liquidation_bonus);
        assert_eq!(cfg.liquidation_fees.raw() as u32, spoke.liquidation_fees);
        assert_eq!(cfg.is_collateralizable, spoke.is_collateralizable);
        assert_eq!(cfg.is_borrowable, spoke.is_borrowable);
    }

    #[test]
    fn test_asset_config_accessors_collateralizable_borrowable() {
        let cfg = AssetConfig::from(&sample_spoke_asset_config());
        assert!(cfg.can_supply());
        assert!(cfg.can_borrow());
    }

    fn spoke_config() -> SpokeConfig {
        SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: 0,
            hf_for_max_bonus_wad: 0,
            liquidation_bonus_factor_bps: 0,
        }
    }

    fn spoke_asset_config() -> SpokeAssetConfig {
        SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: true,
            paused: false,
            frozen: false,
            loan_to_value: 9_000,
            liquidation_threshold: 9_300,
            liquidation_bonus: 300,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
        }
    }

    #[test]
    fn test_spoke_config_and_asset_config_build() {
        let spoke = spoke_config();
        assert!(!spoke.is_deprecated);

        let asset = spoke_asset_config();
        assert!(asset.is_collateralizable);
        assert!(asset.is_borrowable);
        assert_eq!(asset.loan_to_value, 9_000);
    }

    fn account_meta(env: &Env, spoke_id: u32) -> AccountMeta {
        AccountMeta {
            owner: Address::generate(env),
            spoke_id,
            mode: PositionMode::Normal,
        }
    }

    fn empty_account(env: &Env, meta: AccountMeta) -> Account {
        Account {
            owner: meta.owner,
            spoke_id: meta.spoke_id,
            mode: meta.mode,
            supply_positions: Map::new(env),
            borrow_positions: Map::new(env),
        }
    }

    #[test]
    fn test_account_attributes_from_account_and_meta_match() {
        let env = Env::default();
        let meta = account_meta(&env, 4);
        let from_meta = AccountAttributes::from(&meta);
        let account = empty_account(&env, meta);
        let from_account = AccountAttributes::from(&account);
        assert_eq!(from_meta, from_account);
        assert_eq!(from_account.spoke_id, 4);
    }

    #[test]
    fn test_account_attributes_carry_spoke_id() {
        let env = Env::default();
        let attrs = AccountAttributes::from(&account_meta(&env, 1));
        assert_eq!(attrs.spoke_id, 1);
    }

    #[test]
    fn test_account_is_empty_only_when_both_sides_empty() {
        let env = Env::default();
        let mut account = empty_account(&env, account_meta(&env, 1));
        assert!(account.is_empty());

        let position = AccountPositionRaw {
            scaled_amount: 1,
            liquidation_threshold: 0,
            liquidation_bonus: 0,
            loan_to_value: 0,
            liquidation_fees: 0,
        };
        account.supply_positions.set(
            HubAssetKey {
                hub_id: 0,
                asset: Address::generate(&env),
            },
            position.clone(),
        );
        assert!(!account.is_empty());
    }

    #[test]
    fn test_get_or_create_position_returns_existing() {
        let env = Env::default();
        let mut account = empty_account(&env, account_meta(&env, 0));
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let stored = AccountPositionRaw {
            scaled_amount: 42 * crate::constants::RAY,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            loan_to_value: 7_500,
            liquidation_fees: 1_000,
        };
        account
            .supply_positions
            .set(hub_asset.clone(), stored.clone());

        let cfg = AssetConfig::from(&sample_spoke_asset_config());
        let got = account.get_or_create_supply_position(&hub_asset, &cfg);
        assert_eq!(got.scaled_amount.raw(), stored.scaled_amount);
    }

    #[test]
    fn test_get_or_create_supply_position_seeds_risk_from_config() {
        let env = Env::default();
        let account = empty_account(&env, account_meta(&env, 0));
        let cfg = AssetConfig::from(&sample_spoke_asset_config());
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };

        let fresh = account.get_or_create_supply_position(&hub_asset, &cfg);
        assert_eq!(fresh.scaled_amount, Ray::ZERO);
        assert_eq!(fresh.loan_to_value, cfg.loan_to_value);
        assert_eq!(fresh.liquidation_threshold, cfg.liquidation_threshold);
        assert_eq!(fresh.liquidation_bonus, cfg.liquidation_bonus);
    }

    #[test]
    fn test_get_or_create_debt_position_is_scaled_only() {
        let env = Env::default();
        let account = empty_account(&env, account_meta(&env, 0));
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };

        let fresh = account.get_or_create_debt_position(&hub_asset);
        assert_eq!(fresh.scaled_amount, Ray::ZERO);
    }

    // `can_supply` / `can_borrow` are one-line field reads, so the only mutants
    // available are the constant returns. Both survived every native pass and
    // were killed only by the integration harness, because nothing in `common`
    // read them with the flag `false` — and a config that always answers `true`
    // is exactly the shape that silently re-enables a delisted asset.
    fn asset_config(is_collateralizable: bool, is_borrowable: bool) -> AssetConfig {
        AssetConfig {
            loan_to_value: Bps::from(8_000i128),
            liquidation_threshold: Bps::from(8_500i128),
            liquidation_bonus: Bps::from(500i128),
            liquidation_fees: Bps::from(100i128),
            is_collateralizable,
            is_borrowable,
        }
    }

    #[test]
    fn can_supply_tracks_is_collateralizable() {
        assert!(asset_config(true, false).can_supply());
        assert!(!asset_config(false, false).can_supply());
        assert!(!asset_config(false, true).can_supply());
    }

    #[test]
    fn can_borrow_tracks_is_borrowable() {
        assert!(asset_config(false, true).can_borrow());
        assert!(!asset_config(false, false).can_borrow());
        assert!(!asset_config(true, false).can_borrow());
    }

    #[test]
    fn supply_and_borrow_permissions_are_independent() {
        let supply_only = asset_config(true, false);
        assert!(supply_only.can_supply() && !supply_only.can_borrow());

        let borrow_only = asset_config(false, true);
        assert!(!borrow_only.can_supply() && borrow_only.can_borrow());
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    Pool,

    SwapAggregator,

    PriceAggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    AppVersion,

    MinBorrowCollateralUsd,
    LastSpokeId,
    LastHubId,
    Hub(u32),
    Spoke(u32),
    SpokeAsset(u32, HubAssetKey),
    SpokeUsage(u32, HubAssetKey),
    PositionManager(Address),

    BlendPoolAllowed(Address),
    AccountMeta(u64),
    Delegates(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
}
