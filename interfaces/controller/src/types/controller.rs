use crate::types::oracle::MarketOracleConfig;
use common::math::fp::{Bps, Ray};
use common::types::pool::{AccountPosition, AccountPositionRaw, DebtPosition, DebtPositionRaw};
use common::types::shared::PositionMode;
use soroban_sdk::{contracttype, Address, Map, Vec};

/// Persistent asset risk and limit configuration.
///
/// `*_bps` fields use basis points. `*_usd_wad` floors and ceilings are
/// denominated in USD WAD. `borrow_cap` and `supply_cap` use asset units.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetConfigRaw {
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_siloed_borrowing: bool,
    pub is_flashloanable: bool,
    pub flashloan_fee_bps: u32,
    pub borrow_cap: i128,
    pub supply_cap: i128,
    pub e_mode_categories: Vec<u32>,
}

/// Typed asset risk and limit configuration.
#[derive(Clone, Debug)]
pub struct AssetConfig {
    pub loan_to_value: Bps,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub liquidation_fees: Bps,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_siloed_borrowing: bool,
    pub is_flashloanable: bool,
    pub flashloan_fee: Bps,
    pub borrow_cap: i128,
    pub supply_cap: i128,
    pub e_mode_categories: Vec<u32>,
}

impl AssetConfig {
    pub fn can_supply(&self) -> bool {
        self.is_collateralizable
    }

    pub fn can_borrow(&self) -> bool {
        self.is_borrowable
    }

    pub fn is_siloed_borrowing(&self) -> bool {
        self.is_siloed_borrowing
    }

    pub fn has_emode(&self) -> bool {
        !self.e_mode_categories.is_empty()
    }
}

impl From<&AssetConfigRaw> for AssetConfig {
    fn from(r: &AssetConfigRaw) -> Self {
        Self {
            loan_to_value: Bps::from(i128::from(r.loan_to_value_bps)),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus_bps)),
            liquidation_fees: Bps::from(i128::from(r.liquidation_fees_bps)),
            is_collateralizable: r.is_collateralizable,
            is_borrowable: r.is_borrowable,
            is_siloed_borrowing: r.is_siloed_borrowing,
            is_flashloanable: r.is_flashloanable,
            flashloan_fee: Bps::from(i128::from(r.flashloan_fee_bps)),
            borrow_cap: r.borrow_cap,
            supply_cap: r.supply_cap,
            e_mode_categories: r.e_mode_categories.clone(),
        }
    }
}

impl From<&AssetConfig> for AssetConfigRaw {
    fn from(t: &AssetConfig) -> Self {
        Self {
            loan_to_value_bps: t.loan_to_value.raw() as u32,
            liquidation_threshold_bps: t.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: t.liquidation_bonus.raw() as u32,
            liquidation_fees_bps: t.liquidation_fees.raw() as u32,
            is_collateralizable: t.is_collateralizable,
            is_borrowable: t.is_borrowable,
            is_siloed_borrowing: t.is_siloed_borrowing,
            is_flashloanable: t.is_flashloanable,
            flashloan_fee_bps: t.flashloan_fee.raw() as u32,
            borrow_cap: t.borrow_cap,
            supply_cap: t.supply_cap,
            e_mode_categories: t.e_mode_categories.clone(),
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountAttributes {
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
}

impl AccountAttributes {
    pub fn has_emode(&self) -> bool {
        self.e_mode_category_id > 0
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountMeta {
    /// Account owner authorized for supply, borrow, withdraw, and strategies.
    pub owner: Address,
    /// Active e-mode category; zero means no e-mode.
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
}

/// Persistent e-mode category definition.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeCategoryRaw {
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub is_deprecated: bool,
    pub assets: Map<Address, EModeAssetConfig>,
}

/// Typed e-mode category used when applying category overrides.
#[derive(Clone, Debug)]
pub struct EModeCategory {
    pub loan_to_value: Bps,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub is_deprecated: bool,
    pub assets: Map<Address, EModeAssetConfig>,
}

impl From<&EModeCategoryRaw> for EModeCategory {
    fn from(r: &EModeCategoryRaw) -> Self {
        Self {
            loan_to_value: Bps::from(i128::from(r.loan_to_value_bps)),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus_bps)),
            is_deprecated: r.is_deprecated,
            assets: r.assets.clone(),
        }
    }
}

impl From<&EModeCategory> for EModeCategoryRaw {
    fn from(t: &EModeCategory) -> Self {
        Self {
            loan_to_value_bps: t.loan_to_value.raw() as u32,
            liquidation_threshold_bps: t.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: t.liquidation_bonus.raw() as u32,
            is_deprecated: t.is_deprecated,
            assets: t.assets.clone(),
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketIndexView {
    pub asset: Address,
    pub supply_index_ray: i128,
    pub borrow_index_ray: i128,
    pub price_wad: i128,
    pub safe_price_wad: i128,
    pub aggregator_price_wad: i128,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetExtendedConfigView {
    pub asset: Address,
    pub pool_address: Address,
    pub price_wad: i128,
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
    /// Collateral amounts expected to be seized, in asset-native units.
    pub seized_collaterals: Vec<PaymentTuple>,
    /// Liquidation protocol fees deducted from seized collateral.
    pub protocol_fees: Vec<PaymentTuple>,
    /// Debt-payment amounts expected to be refunded to the liquidator.
    pub refunds: Vec<PaymentTuple>,
    /// Maximum debt payment accepted by the liquidation math, in USD WAD.
    pub max_payment_wad: i128,
    /// Liquidation bonus used for the estimate, in BPS.
    pub bonus_rate_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub asset: Address,
    pub amount: i128,
    pub protocol_fee: i128,
    pub feed: crate::types::oracle::PriceFeedRaw,
    pub market_index: common::types::pool::MarketIndexRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub asset: Address,
    pub amount: i128,
    pub usd_wad: i128,
    pub feed: crate::types::oracle::PriceFeedRaw,
    pub market_index: common::types::pool::MarketIndexRaw,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MarketStatus {
    PendingOracle = 0,
    Active = 1,
    Disabled = 2,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketConfig {
    /// Pending markets cannot be used until oracle config is active.
    pub status: MarketStatus,
    pub asset_config: AssetConfigRaw,
    pub oracle_config: MarketOracleConfig,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Account {
    /// Account owner authorized for owner-gated account mutations.
    pub owner: Address,
    /// Active e-mode category; zero means no e-mode.
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    /// Collateral positions keyed by asset.
    pub supply_positions: Map<Address, AccountPositionRaw>,
    /// Debt positions keyed by asset.
    pub borrow_positions: Map<Address, DebtPositionRaw>,
}

impl Account {
    pub fn attributes(&self) -> AccountAttributes {
        AccountAttributes::from(self)
    }

    pub fn has_emode(&self) -> bool {
        self.e_mode_category_id > 0
    }

    /// Existing collateral position for `asset` (decoded to typed form) or a
    /// fresh one seeded from `config`'s risk parameters. Collateral positions
    /// carry the risk params that HF/LTV/liquidation math reads.
    pub fn get_or_create_supply_position(
        &self,
        asset: &Address,
        config: &AssetConfig,
    ) -> AccountPosition {
        self.supply_positions
            .get(asset.clone())
            .map(|raw| AccountPosition::from(&raw))
            .unwrap_or(AccountPosition {
                scaled_amount: Ray::ZERO,
                liquidation_threshold: config.liquidation_threshold,
                liquidation_bonus: config.liquidation_bonus,
                loan_to_value: config.loan_to_value,
            })
    }

    /// Existing debt position for `asset` or a fresh zero one. Debt positions
    /// carry only the scaled share — risk params live on collateral.
    pub fn get_or_create_debt_position(&self, asset: &Address) -> DebtPosition {
        self.borrow_positions
            .get(asset.clone())
            .map(|raw| DebtPosition::from(&raw))
            .unwrap_or(DebtPosition {
                scaled_amount: Ray::ZERO,
            })
    }

    pub fn is_empty(&self) -> bool {
        self.supply_positions.is_empty() && self.borrow_positions.is_empty()
    }
}

impl From<&Account> for AccountAttributes {
    fn from(account: &Account) -> Self {
        AccountAttributes {
            e_mode_category_id: account.e_mode_category_id,
            mode: account.mode,
        }
    }
}

impl From<&AccountMeta> for AccountAttributes {
    fn from(account: &AccountMeta) -> Self {
        AccountAttributes {
            e_mode_category_id: account.e_mode_category_id,
            mode: account.mode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::WAD;
    use soroban_sdk::{testutils::Address as _, Env};

    fn sample_asset_config_raw(env: &Env) -> AssetConfigRaw {
        let mut categories: Vec<u32> = Vec::new(env);
        categories.push_back(1);
        categories.push_back(2);
        AssetConfigRaw {
            loan_to_value_bps: 7_500,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            is_siloed_borrowing: false,
            is_flashloanable: true,
            flashloan_fee_bps: 9,
            borrow_cap: 1_000_000,
            supply_cap: 5_000_000,
            e_mode_categories: categories,
        }
    }

    #[test]
    fn test_asset_config_raw_typed_roundtrip() {
        let env = Env::default();
        let raw = sample_asset_config_raw(&env);
        let typed = AssetConfig::from(&raw);
        let back = AssetConfigRaw::from(&typed);
        assert_eq!(back.loan_to_value_bps, raw.loan_to_value_bps);
        assert_eq!(
            back.liquidation_threshold_bps,
            raw.liquidation_threshold_bps
        );
        assert_eq!(back.liquidation_bonus_bps, raw.liquidation_bonus_bps);
        assert_eq!(back.liquidation_fees_bps, raw.liquidation_fees_bps);
        assert_eq!(back.is_collateralizable, raw.is_collateralizable);
        assert_eq!(back.is_borrowable, raw.is_borrowable);
        assert_eq!(back.is_siloed_borrowing, raw.is_siloed_borrowing);
        assert_eq!(back.is_flashloanable, raw.is_flashloanable);
        assert_eq!(back.flashloan_fee_bps, raw.flashloan_fee_bps);
        assert_eq!(back.borrow_cap, raw.borrow_cap);
        assert_eq!(back.supply_cap, raw.supply_cap);
        assert_eq!(back.e_mode_categories, raw.e_mode_categories);
    }

    #[test]
    fn test_asset_config_accessors_collateralizable_borrowable() {
        let env = Env::default();
        let cfg = AssetConfig::from(&sample_asset_config_raw(&env));
        assert!(cfg.can_supply());
        assert!(cfg.can_borrow());
        assert!(!cfg.is_siloed_borrowing());
        assert!(cfg.has_emode());
    }

    fn emode_category_raw(env: &Env) -> EModeCategoryRaw {
        let mut assets: Map<Address, EModeAssetConfig> = Map::new(env);
        assets.set(
            Address::generate(env),
            EModeAssetConfig {
                is_collateralizable: true,
                is_borrowable: true,
            },
        );
        EModeCategoryRaw {
            loan_to_value_bps: 9_000,
            liquidation_threshold_bps: 9_300,
            liquidation_bonus_bps: 300,
            is_deprecated: false,
            assets,
        }
    }

    #[test]
    fn test_emode_category_raw_typed_roundtrip() {
        let env = Env::default();
        let raw = emode_category_raw(&env);
        let typed = EModeCategory::from(&raw);
        let back = EModeCategoryRaw::from(&typed);
        assert_eq!(back.loan_to_value_bps, raw.loan_to_value_bps);
        assert_eq!(
            back.liquidation_threshold_bps,
            raw.liquidation_threshold_bps
        );
        assert_eq!(back.liquidation_bonus_bps, raw.liquidation_bonus_bps);
        assert_eq!(back.is_deprecated, raw.is_deprecated);
        assert_eq!(back.assets.len(), raw.assets.len());
    }

    fn account_meta(env: &Env, category: u32) -> AccountMeta {
        AccountMeta {
            owner: Address::generate(env),
            e_mode_category_id: category,
            mode: PositionMode::Normal,
        }
    }

    fn empty_account(env: &Env, meta: AccountMeta) -> Account {
        Account {
            owner: meta.owner,
            e_mode_category_id: meta.e_mode_category_id,
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
        assert!(from_account.has_emode());
        assert_eq!(from_account.e_mode_category_id, 4);
    }

    #[test]
    fn test_account_attributes_no_emode_without_category() {
        let env = Env::default();
        let attrs = AccountAttributes::from(&account_meta(&env, 0));
        assert!(!attrs.has_emode());
    }

    #[test]
    fn test_account_has_emode_and_attributes() {
        let env = Env::default();
        let normal = empty_account(&env, account_meta(&env, 0));
        assert!(!normal.has_emode());
        assert_eq!(normal.attributes().e_mode_category_id, 0);

        let emode = empty_account(&env, account_meta(&env, 1));
        assert!(emode.has_emode());
    }

    #[test]
    fn test_account_is_empty_only_when_both_sides_empty() {
        let env = Env::default();
        let mut account = empty_account(&env, account_meta(&env, 0));
        assert!(account.is_empty());

        let position = AccountPositionRaw {
            scaled_amount_ray: 1,
            liquidation_threshold_bps: 0,
            liquidation_bonus_bps: 0,
            loan_to_value_bps: 0,
        };
        account
            .supply_positions
            .set(Address::generate(&env), position.clone());
        assert!(!account.is_empty());
    }

    #[test]
    fn test_get_or_create_position_returns_existing() {
        let env = Env::default();
        let mut account = empty_account(&env, account_meta(&env, 0));
        let asset = Address::generate(&env);
        let stored = AccountPositionRaw {
            scaled_amount_ray: 42 * common::constants::RAY,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            loan_to_value_bps: 7_500,
        };
        account.supply_positions.set(asset.clone(), stored.clone());

        let cfg = AssetConfig::from(&sample_asset_config_raw(&env));
        let got = account.get_or_create_supply_position(&asset, &cfg);
        assert_eq!(got.scaled_amount.raw(), stored.scaled_amount_ray);
    }

    #[test]
    fn test_get_or_create_supply_position_seeds_risk_from_config() {
        let env = Env::default();
        let account = empty_account(&env, account_meta(&env, 0));
        let cfg = AssetConfig::from(&sample_asset_config_raw(&env));
        let asset = Address::generate(&env);

        let fresh = account.get_or_create_supply_position(&asset, &cfg);
        assert_eq!(fresh.scaled_amount, Ray::ZERO);
        assert_eq!(fresh.loan_to_value, cfg.loan_to_value);
        assert_eq!(fresh.liquidation_threshold, cfg.liquidation_threshold);
        assert_eq!(fresh.liquidation_bonus, cfg.liquidation_bonus);
    }

    #[test]
    fn test_get_or_create_debt_position_is_scaled_only() {
        let env = Env::default();
        let account = empty_account(&env, account_meta(&env, 0));
        let asset = Address::generate(&env);

        // Debt positions carry only the scaled share — no risk params.
        let fresh = account.get_or_create_debt_position(&asset);
        assert_eq!(fresh.scaled_amount, Ray::ZERO);
    }
}

// Storage tiers (instance/persistent/temporary) live in `controller::storage`
// accessors. Per-account state is split (`AccountMeta`/`SupplyPositions`/
// `BorrowPositions`) so callers load only the side they need.
#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    PoolTemplate,
    /// Address of the single central liquidity pool deployed by the controller.
    Pool,
    Aggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    LastEModeCategoryId,
    Market(Address),
    AccountMeta(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
    EModeCategory(u32),
    PoolsList,
    AppVersion,
    /// Instance-level minimum LTV-weighted collateral USD WAD while debt exists.
    MinBorrowCollateralUsd,
}
