//! Controller domain types: spoke risk projections, accounts, positions.

use crate::math::fp::{Bps, Ray};
use crate::types::oracle::PriceFeedRaw;
use crate::types::pool::{
    AccountPosition, AccountPositionRaw, DebtPosition, DebtPositionRaw, HubAssetKey,
};
use crate::types::shared::PositionMode;
use soroban_sdk::{contracttype, Address, Map, Vec};

/// Spoke-projected risk flags. Caps on [`SpokeAssetConfig`]; flash fee on pool params.
/// Risk ratios are BPS.
#[derive(Clone, Debug)]
pub struct AssetConfig {
    /// Loan-to-value, BPS.
    pub loan_to_value: Bps,
    /// Liquidation threshold, BPS.
    pub liquidation_threshold: Bps,
    /// Liquidation bonus, BPS.
    pub liquidation_bonus: Bps,
    /// Protocol fee on seized collateral, BPS.
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

/// Hub registry entry (ids from 1). Gates `(hub, asset)` markets.
#[contracttype]
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub is_active: bool,
}

/// Position manager: active + listed on the account's delegates.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionManagerConfig {
    pub is_active: bool,
}

/// Spoke record: deprecation flag + liquidation-curve params for bound accounts.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeConfig {
    pub is_deprecated: bool,
    /// Target health factor after liquidation, WAD.
    pub liquidation_target_hf_wad: i128,
    /// HF at which max bonus applies, WAD.
    pub hf_for_max_bonus_wad: i128,
    /// Bonus factor along the curve, BPS.
    pub liquidation_bonus_factor_bps: u32,
}

/// Spoke listing: risk BPS, caps (asset-native), pause/freeze, optional oracle override.
/// `paused` blocks all user verbs; `frozen` blocks only new supply/borrow.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub paused: bool,
    pub frozen: bool,
    /// Loan-to-value, BPS.
    pub loan_to_value: u32,
    /// Liquidation threshold, BPS.
    pub liquidation_threshold: u32,
    /// Liquidation bonus, BPS.
    pub liquidation_bonus: u32,
    /// Protocol fee on seized collateral, BPS.
    pub liquidation_fees: u32,
    /// Supply cap in asset-native units (`0` / `i128::MAX` disable).
    pub supply_cap: i128,
    /// Borrow cap in asset-native units (`0` / `i128::MAX` disable).
    pub borrow_cap: i128,
}

/// Args for add/edit spoke asset. Edits set pause/freeze explicitly (no silent clear).
/// Risk fields BPS; caps asset-native.
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
    /// Loan-to-value, BPS.
    pub ltv: u32,
    /// Liquidation threshold, BPS.
    pub threshold: u32,
    /// Liquidation bonus, BPS.
    pub bonus: u32,
    /// Protocol fee on seized collateral, BPS.
    pub liquidation_fees: u32,
    /// Supply cap in asset-native units.
    pub supply_cap: i128,
    /// Borrow cap in asset-native units.
    pub borrow_cap: i128,
}

/// Running scaled-share totals for one asset within a spoke (RAY).
#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct SpokeUsageRaw {
    /// Aggregate scaled supply shares on this spoke-asset, RAY.
    pub supplied_scaled_ray: i128,
    /// Aggregate scaled borrow shares on this spoke-asset, RAY.
    pub borrowed_scaled_ray: i128,
}

/// Pool indexes + soft oracle status for one hub-asset market.
///
/// Price fields use historical ABI names: `safe_price_wad` = primary leg,
/// `aggregator_price_wad` = secondary/anchor leg. Status flags describe whether
/// the price is usable (`valid`) or blocked by staleness / dual-source deviation.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketIndexView {
    pub asset: Address,
    /// Supply index, RAY.
    pub supply_index: i128,
    /// Borrow index, RAY.
    pub borrow_index: i128,
    /// Final composed USD WAD (0 when unusable / unreadable).
    pub price_wad: i128,
    /// Primary oracle leg (historical ABI name; not a safety flag).
    pub safe_price_wad: i128,
    /// Secondary/anchor leg (historical ABI name; not the swap aggregator).
    pub aggregator_price_wad: i128,
    /// Freshness timestamp of the final blend (seconds).
    pub price_timestamp: u64,
    pub stale: bool,
    pub deviation: bool,
    /// True when price is fresh, in-band, positive, and within sanity.
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
    /// Amount in asset-native units.
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
    pub hub_asset: HubAssetKey,
    /// Seized collateral in asset-native units.
    pub amount: i128,
    /// Protocol fee in asset-native units.
    pub protocol_fee: i128,
    pub feed: PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub hub_asset: HubAssetKey,
    /// Debt repaid in asset-native units.
    pub amount: i128,
    /// USD value of the repayment, WAD.
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
    /// Maximum debt payment accepted, USD WAD.
    pub max_debt_usd: i128,
    /// Liquidation bonus applied, BPS.
    pub bonus_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Account {
    /// Account owner authorized for owner-gated account mutations.
    pub owner: Address,
    /// Active spoke; always `>= 1` and references an existing spoke.
    pub spoke_id: u32,
    pub mode: PositionMode,
    /// Collateral positions keyed by hub asset.
    pub supply_positions: Map<HubAssetKey, AccountPositionRaw>,
    /// Debt positions keyed by hub asset.
    pub borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
}

impl Account {
    pub fn attributes(&self) -> AccountAttributes {
        AccountAttributes::from(self)
    }

    /// Existing collateral position for `asset` (decoded to typed form) or a
    /// fresh one seeded from `config`'s risk parameters. Collateral positions
    /// carry the risk params that HF/LTV/liquidation math reads.
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

    /// Existing debt position for `asset` or a fresh zero one. Debt positions
    /// carry only the scaled share — risk params live on collateral.
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

        // Debt positions carry only the scaled share — no risk params.
        let fresh = account.get_or_create_debt_position(&hub_asset);
        assert_eq!(fresh.scaled_amount, Ray::ZERO);
    }
}

// Instance and persistent storage tiers for these keys live in
// `controller::storage` accessors. Per-account state is split
// (`AccountMeta`/`SupplyPositions`/`BorrowPositions`) so callers load only
// the side they need.
#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    PoolTemplate,
    /// Address of the single central liquidity pool deployed by the controller.
    Pool,
    /// Swap venue (the swap-aggregator contract), used by strategy flows.
    SwapAggregator,
    /// Oracle authority (the price-aggregator contract) for bulk price reads.
    PriceAggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    AppVersion,
    /// Instance-level minimum LTV-weighted collateral USD WAD while debt exists.
    MinBorrowCollateralUsd,
    LastSpokeId,
    LastHubId,
    Hub(u32),
    Spoke(u32),
    SpokeAsset(u32, HubAssetKey),
    SpokeUsage(u32, HubAssetKey),
    PositionManager(Address),
    /// Governance allowlist entry for a Blend migration pool (persistent).
    BlendPoolAllowed(Address),
    AccountMeta(u64),
    Delegates(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
}
