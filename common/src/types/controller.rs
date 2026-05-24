use crate::math::fp::{Bps, Ray, Wad};
use crate::types::oracle::MarketOracleConfig;
use crate::types::pool::{AccountPosition, AccountPositionRaw};
use crate::types::shared::{AccountPositionType, PositionMode};
use soroban_sdk::{contracttype, Address, Map, Vec};

// Wire/storage form. Embedded in MarketConfig (persistent storage value).
#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetConfigRaw {
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_isolated_asset: bool,
    pub is_siloed_borrowing: bool,
    pub is_flashloanable: bool,
    pub isolation_borrow_enabled: bool,
    pub isolation_debt_ceiling_usd_wad: i128,
    pub flashloan_fee_bps: u32,
    pub borrow_cap: i128,
    pub supply_cap: i128,
    pub min_collat_floor_usd_wad: i128,
    pub min_debt_floor_usd_wad: i128,
    pub e_mode_categories: Vec<u32>,
}

// In-memory typed form. Used by every compute path.
#[derive(Clone, Debug)]
pub struct AssetConfig {
    pub loan_to_value: Bps,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub liquidation_fees: Bps,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_isolated_asset: bool,
    pub is_siloed_borrowing: bool,
    pub is_flashloanable: bool,
    pub isolation_borrow_enabled: bool,
    pub isolation_debt_ceiling_usd: Wad,
    pub flashloan_fee: Bps,
    pub borrow_cap: i128,
    pub supply_cap: i128,
    pub min_collat_floor_usd: Wad,
    pub min_debt_floor_usd: Wad,
    pub e_mode_categories: Vec<u32>,
}

impl AssetConfig {
    pub fn can_supply(&self) -> bool {
        self.is_collateralizable
    }

    pub fn can_borrow(&self) -> bool {
        self.is_borrowable
    }

    pub fn is_isolated(&self) -> bool {
        self.is_isolated_asset
    }

    pub fn is_siloed_borrowing(&self) -> bool {
        self.is_siloed_borrowing
    }

    pub fn can_borrow_in_isolation(&self) -> bool {
        self.isolation_borrow_enabled
    }

    pub fn has_emode(&self) -> bool {
        !self.e_mode_categories.is_empty()
    }
}

impl From<&AssetConfigRaw> for AssetConfig {
    fn from(r: &AssetConfigRaw) -> Self {
        Self {
            loan_to_value: Bps::from_raw(i128::from(r.loan_to_value_bps)),
            liquidation_threshold: Bps::from_raw(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from_raw(i128::from(r.liquidation_bonus_bps)),
            liquidation_fees: Bps::from_raw(i128::from(r.liquidation_fees_bps)),
            is_collateralizable: r.is_collateralizable,
            is_borrowable: r.is_borrowable,
            is_isolated_asset: r.is_isolated_asset,
            is_siloed_borrowing: r.is_siloed_borrowing,
            is_flashloanable: r.is_flashloanable,
            isolation_borrow_enabled: r.isolation_borrow_enabled,
            isolation_debt_ceiling_usd: Wad::from_raw(r.isolation_debt_ceiling_usd_wad),
            flashloan_fee: Bps::from_raw(i128::from(r.flashloan_fee_bps)),
            borrow_cap: r.borrow_cap,
            supply_cap: r.supply_cap,
            min_collat_floor_usd: Wad::from_raw(r.min_collat_floor_usd_wad),
            min_debt_floor_usd: Wad::from_raw(r.min_debt_floor_usd_wad),
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
            is_isolated_asset: t.is_isolated_asset,
            is_siloed_borrowing: t.is_siloed_borrowing,
            is_flashloanable: t.is_flashloanable,
            isolation_borrow_enabled: t.isolation_borrow_enabled,
            isolation_debt_ceiling_usd_wad: t.isolation_debt_ceiling_usd.raw(),
            flashloan_fee_bps: t.flashloan_fee.raw() as u32,
            borrow_cap: t.borrow_cap,
            supply_cap: t.supply_cap,
            min_collat_floor_usd_wad: t.min_collat_floor_usd.raw(),
            min_debt_floor_usd_wad: t.min_debt_floor_usd.raw(),
            e_mode_categories: t.e_mode_categories.clone(),
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountAttributes {
    pub is_isolated: bool,
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
    pub owner: Address,
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    pub isolated_asset: Option<Address>,
}

// Wire/storage form. Stored under ControllerKey::EModeCategory(id).
#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeCategoryRaw {
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub is_deprecated: bool,
    pub assets: Map<Address, EModeAssetConfig>,
}

// In-memory typed form. Used by the e-mode compute path.
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
            loan_to_value: Bps::from_raw(i128::from(r.loan_to_value_bps)),
            liquidation_threshold: Bps::from_raw(i128::from(r.liquidation_threshold_bps)),
            liquidation_bonus: Bps::from_raw(i128::from(r.liquidation_bonus_bps)),
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
    pub seized_collaterals: Vec<PaymentTuple>,
    pub protocol_fees: Vec<PaymentTuple>,
    pub refunds: Vec<PaymentTuple>,
    pub max_payment_wad: i128,
    pub bonus_rate_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub asset: Address,
    pub amount: i128,
    pub protocol_fee: i128,
    pub feed: crate::types::oracle::PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub asset: Address,
    pub amount: i128,
    pub usd_wad: i128,
    pub feed: crate::types::oracle::PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<crate::types::shared::Payment>,
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
    pub status: MarketStatus,
    pub asset_config: AssetConfigRaw,
    pub pool_address: Address,
    pub oracle_config: MarketOracleConfig,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Account {
    pub owner: Address,
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    pub isolated_asset: Option<Address>,
    pub supply_positions: Map<Address, AccountPositionRaw>,
    pub borrow_positions: Map<Address, AccountPositionRaw>,
}

impl Account {
    pub fn attributes(&self) -> AccountAttributes {
        AccountAttributes::from(self)
    }

    pub fn has_emode(&self) -> bool {
        self.e_mode_category_id > 0
    }

    pub fn try_isolated_token(&self) -> Option<Address> {
        self.isolated_asset.clone()
    }

    /// Returns the existing supply/borrow position for `asset` (loaded from the
    /// raw map and decoded to the typed form) or a fresh one seeded from
    /// `config`'s risk parameters. Used at the start of every position
    /// mutation so callers operate on a typed value regardless of whether the
    /// asset has been touched before.
    pub fn get_or_create_position(
        &self,
        kind: AccountPositionType,
        asset: &Address,
        config: &AssetConfig,
    ) -> AccountPosition {
        let positions = match kind {
            AccountPositionType::Deposit => &self.supply_positions,
            AccountPositionType::Borrow => &self.borrow_positions,
        };
        positions
            .get(asset.clone())
            .map(|raw| AccountPosition::from(&raw))
            .unwrap_or(AccountPosition {
                scaled_amount: Ray::ZERO,
                liquidation_threshold: config.liquidation_threshold,
                liquidation_bonus: config.liquidation_bonus,
                loan_to_value: config.loan_to_value,
            })
    }

    pub fn is_empty(&self) -> bool {
        self.supply_positions.is_empty() && self.borrow_positions.is_empty()
    }
}

impl From<&Account> for AccountAttributes {
    fn from(account: &Account) -> Self {
        AccountAttributes {
            is_isolated: account.is_isolated,
            e_mode_category_id: account.e_mode_category_id,
            mode: account.mode,
        }
    }
}

impl From<&AccountMeta> for AccountAttributes {
    fn from(account: &AccountMeta) -> Self {
        AccountAttributes {
            is_isolated: account.is_isolated,
            e_mode_category_id: account.e_mode_category_id,
            mode: account.mode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WAD;
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
            is_isolated_asset: false,
            is_siloed_borrowing: false,
            is_flashloanable: true,
            isolation_borrow_enabled: true,
            isolation_debt_ceiling_usd_wad: 1_000 * WAD,
            flashloan_fee_bps: 9,
            borrow_cap: 1_000_000,
            supply_cap: 5_000_000,
            min_collat_floor_usd_wad: 10 * WAD,
            min_debt_floor_usd_wad: 10 * WAD,
            e_mode_categories: categories,
        }
    }

    fn isolated_asset_config_raw(env: &Env) -> AssetConfigRaw {
        let mut raw = sample_asset_config_raw(env);
        raw.is_collateralizable = false;
        raw.is_borrowable = false;
        raw.is_isolated_asset = true;
        raw.is_siloed_borrowing = true;
        raw.isolation_borrow_enabled = false;
        raw.e_mode_categories = Vec::new(env);
        raw
    }

    #[test]
    fn test_asset_config_raw_typed_roundtrip() {
        let env = Env::default();
        let raw = sample_asset_config_raw(&env);
        let typed = AssetConfig::from(&raw);
        let back = AssetConfigRaw::from(&typed);
        assert_eq!(back.loan_to_value_bps, raw.loan_to_value_bps);
        assert_eq!(back.liquidation_threshold_bps, raw.liquidation_threshold_bps);
        assert_eq!(back.liquidation_bonus_bps, raw.liquidation_bonus_bps);
        assert_eq!(back.liquidation_fees_bps, raw.liquidation_fees_bps);
        assert_eq!(back.is_collateralizable, raw.is_collateralizable);
        assert_eq!(back.is_borrowable, raw.is_borrowable);
        assert_eq!(back.is_isolated_asset, raw.is_isolated_asset);
        assert_eq!(back.is_siloed_borrowing, raw.is_siloed_borrowing);
        assert_eq!(back.is_flashloanable, raw.is_flashloanable);
        assert_eq!(back.isolation_borrow_enabled, raw.isolation_borrow_enabled);
        assert_eq!(
            back.isolation_debt_ceiling_usd_wad,
            raw.isolation_debt_ceiling_usd_wad
        );
        assert_eq!(back.flashloan_fee_bps, raw.flashloan_fee_bps);
        assert_eq!(back.borrow_cap, raw.borrow_cap);
        assert_eq!(back.supply_cap, raw.supply_cap);
        assert_eq!(back.min_collat_floor_usd_wad, raw.min_collat_floor_usd_wad);
        assert_eq!(back.min_debt_floor_usd_wad, raw.min_debt_floor_usd_wad);
        assert_eq!(back.e_mode_categories, raw.e_mode_categories);
    }

    #[test]
    fn test_asset_config_accessors_collateralizable_borrowable() {
        let env = Env::default();
        let cfg = AssetConfig::from(&sample_asset_config_raw(&env));
        assert!(cfg.can_supply());
        assert!(cfg.can_borrow());
        assert!(!cfg.is_isolated());
        assert!(!cfg.is_siloed_borrowing());
        assert!(cfg.can_borrow_in_isolation());
        assert!(cfg.has_emode());
    }

    #[test]
    fn test_asset_config_accessors_isolated_silent() {
        let env = Env::default();
        let cfg = AssetConfig::from(&isolated_asset_config_raw(&env));
        assert!(!cfg.can_supply());
        assert!(!cfg.can_borrow());
        assert!(cfg.is_isolated());
        assert!(cfg.is_siloed_borrowing());
        assert!(!cfg.can_borrow_in_isolation());
        assert!(!cfg.has_emode());
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
        assert_eq!(back.liquidation_threshold_bps, raw.liquidation_threshold_bps);
        assert_eq!(back.liquidation_bonus_bps, raw.liquidation_bonus_bps);
        assert_eq!(back.is_deprecated, raw.is_deprecated);
        assert_eq!(back.assets.len(), raw.assets.len());
    }

    fn account_meta(env: &Env, category: u32, isolated: bool) -> AccountMeta {
        AccountMeta {
            owner: Address::generate(env),
            is_isolated: isolated,
            e_mode_category_id: category,
            mode: PositionMode::Normal,
            isolated_asset: if isolated {
                Some(Address::generate(env))
            } else {
                None
            },
        }
    }

    fn empty_account(env: &Env, meta: AccountMeta) -> Account {
        Account {
            owner: meta.owner,
            is_isolated: meta.is_isolated,
            e_mode_category_id: meta.e_mode_category_id,
            mode: meta.mode,
            isolated_asset: meta.isolated_asset,
            supply_positions: Map::new(env),
            borrow_positions: Map::new(env),
        }
    }

    #[test]
    fn test_account_attributes_from_account_and_meta_match() {
        let env = Env::default();
        let meta = account_meta(&env, 4, true);
        let from_meta = AccountAttributes::from(&meta);
        let account = empty_account(&env, meta);
        let from_account = AccountAttributes::from(&account);
        assert_eq!(from_meta, from_account);
        assert!(from_account.has_emode());
        assert!(from_account.is_isolated);
        assert_eq!(from_account.e_mode_category_id, 4);
    }

    #[test]
    fn test_account_attributes_no_emode_without_category() {
        let env = Env::default();
        let attrs = AccountAttributes::from(&account_meta(&env, 0, false));
        assert!(!attrs.has_emode());
    }

    #[test]
    fn test_account_has_emode_and_try_isolated_token_and_attributes() {
        let env = Env::default();
        let normal = empty_account(&env, account_meta(&env, 0, false));
        assert!(!normal.has_emode());
        assert!(normal.try_isolated_token().is_none());
        assert_eq!(normal.attributes().e_mode_category_id, 0);

        let isolated = empty_account(&env, account_meta(&env, 1, true));
        assert!(isolated.has_emode());
        assert!(isolated.try_isolated_token().is_some());
    }

    #[test]
    fn test_account_is_empty_only_when_both_sides_empty() {
        let env = Env::default();
        let mut account = empty_account(&env, account_meta(&env, 0, false));
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
        let mut account = empty_account(&env, account_meta(&env, 0, false));
        let asset = Address::generate(&env);
        let stored = AccountPositionRaw {
            scaled_amount_ray: 42 * crate::constants::RAY,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            loan_to_value_bps: 7_500,
        };
        account.supply_positions.set(asset.clone(), stored.clone());

        let cfg = AssetConfig::from(&sample_asset_config_raw(&env));
        let got = account.get_or_create_position(AccountPositionType::Deposit, &asset, &cfg);
        assert_eq!(got.scaled_amount.raw(), stored.scaled_amount_ray);
    }

    #[test]
    fn test_get_or_create_position_seeds_from_config_on_borrow_side() {
        let env = Env::default();
        let account = empty_account(&env, account_meta(&env, 0, false));
        let cfg = AssetConfig::from(&sample_asset_config_raw(&env));
        let asset = Address::generate(&env);

        let fresh = account.get_or_create_position(AccountPositionType::Borrow, &asset, &cfg);
        assert_eq!(fresh.scaled_amount, Ray::ZERO);
        assert_eq!(fresh.loan_to_value, cfg.loan_to_value);
        assert_eq!(fresh.liquidation_threshold, cfg.liquidation_threshold);
        assert_eq!(fresh.liquidation_bonus, cfg.liquidation_bonus);
    }
}

// Storage tiers per variant are defined by the accessor functions in
// `controller::storage` (instance vs persistent vs temporary). Per-account
// state (`AccountMeta`, `SupplyPositions`, `BorrowPositions`) is split per
// INVARIANTS §5.2 so callers load only the side they need.
#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    PoolTemplate,
    Aggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    LastEModeCategoryId,
    FlashLoanOngoing,
    Market(Address),
    AccountMeta(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
    EModeCategory(u32),
    IsolatedDebt(Address),
    PoolsList,
    AppVersion,
}
