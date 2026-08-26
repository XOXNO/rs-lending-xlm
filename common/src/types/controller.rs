//! Types used by the controller contract: hub/spoke and asset risk configuration, account
//! and position storage shapes, liquidation inputs/outputs, and the `ControllerKey` storage
//! key enum.

use crate::math::fp::{Bps, Ray};
use crate::types::oracle::PriceFeedRaw;
use crate::types::pool::{
    AccountPosition, AccountPositionRaw, DebtPosition, DebtPositionRaw, HubAssetKey,
};
use crate::types::shared::PositionMode;
use soroban_sdk::{contracttype, Address, Map, Vec};

/// Risk parameters used to size and evaluate a position for one asset: the fixed-point
/// loan-to-value, liquidation threshold, liquidation bonus, and liquidation fee rates, plus
/// whether the asset currently accepts new supply or new borrows.
#[derive(Clone, Debug)]
pub struct AssetConfig {
    pub loan_to_value: Bps,

    pub liquidation_threshold: Bps,

    pub liquidation_bonus: Bps,

    pub liquidation_fees: Bps,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
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

/// Lightweight projection of an account's spoke and position mode, omitting the owner
/// address and position maps carried by `Account`. Structurally identical to the stored
/// `AccountMeta` it is built from; kept as a separate type so the view surface is decoupled
/// from storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountAttributes {
    pub spoke_id: u32,
    pub mode: PositionMode,
}

/// Per-account metadata stored independently of the account's supply and borrow position
/// maps: spoke membership and position mode. Ownership lives in the position-NFT contract
/// (`owner_of(account_id)`), not here.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountMeta {
    pub spoke_id: u32,
    pub mode: PositionMode,
}

/// A delegate list stamped with the owner who granted it. The grant is live only while
/// `granted_by` still owns the account's NFT: transferring the NFT deactivates the prior
/// owner's grant immediately (`get_delegates` reads it as empty for anyone else). The stale
/// entry is purged from storage the next time the new owner writes a delegate (`add_delegate`
/// or `remove_delegate`) — no explicit cleanup call is required. A grant re-arms with its
/// original delegate list only if the NFT returns to `granted_by` before any such write.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegateGrant {
    pub granted_by: Address,
    pub delegates: Vec<Address>,
}

/// Stored configuration for a hub: whether it currently accepts activity.
#[contracttype]
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub is_active: bool,
}

/// Stored configuration for a registered position manager: whether it is currently active.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionManagerConfig {
    pub is_active: bool,
}

/// Stored configuration for a spoke: deprecation flag and the health-factor and bonus
/// parameters that drive its liquidation curve.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeConfig {
    pub is_deprecated: bool,
    pub liquidation_target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}

/// Stored per-spoke risk and cap configuration for one asset, with basis-point rates as raw
/// `u32` values.
///
/// The three halt flags are independent and gate different legs: `paused` blocks every user
/// verb, `frozen` blocks entry but allows exit, and `no_seize` blocks only the liquidation
/// seizure leg. Seizure is deliberately *not* gated by `paused`, because seizure is pro-rata
/// across an account's whole collateral set: pausing one collateral would otherwise halt
/// liquidation of every account holding it. See ADR-0008.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub paused: bool,
    pub frozen: bool,

    pub no_seize: bool,
    pub loan_to_value: u32,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub liquidation_fees: u32,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

/// Input arguments for adding or editing an asset's listing in a spoke.
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

    pub no_seize: bool,

    pub ltv: u32,

    pub threshold: u32,

    pub bonus: u32,

    pub liquidation_fees: u32,

    pub supply_cap: i128,

    pub borrow_cap: i128,
}

/// Stored ray-scaled supply and borrow usage for one asset within a spoke, used to enforce
/// per-spoke supply and borrow caps.
#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct SpokeUsageRaw {
    pub supplied_scaled_ray: i128,

    pub borrowed_scaled_ray: i128,
}

/// View of a market's current interest indices and resolved price, including the individual
/// primary/anchor legs and staleness/deviation/validity flags reported by the oracle.
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

/// Per-account caps on the number of distinct supply and borrow positions held at once.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionLimits {
    pub max_borrow_positions: u32,
    pub max_supply_positions: u32,
}

/// An asset/amount pair used across payment, refund, and liquidation views.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PaymentTuple {
    pub asset: Address,

    pub amount: i128,
}

/// View-only projection of a simulated liquidation outcome: seized collateral and protocol
/// fees per asset, any refunded payments, the maximum USD-equivalent debt repayable, and the
/// applied bonus rate.
///
/// `seized_collaterals` is gross of `protocol_fees`: the liquidator ends up with
/// `seized_collaterals - protocol_fees`. Both are reported in the units the requested
/// `SeizeMode` moves — asset units for `Transfer`, RAY-scaled supply shares for `Credit`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidationEstimate {
    pub seized_collaterals: Vec<PaymentTuple>,

    pub protocol_fees: Vec<PaymentTuple>,

    pub refunds: Vec<PaymentTuple>,

    pub max_payment_wad: i128,

    pub bonus_rate_bps: i128,
}

/// How a liquidator takes delivery of the collateral seized from the liquidated account.
///
/// One mode governs the whole call rather than one mode per asset: seizure is pro-rata across
/// every collateral the account holds, so a per-asset choice would have no meaning.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeizeMode {
    /// The pool transfers the underlying tokens to the liquidator, withholding the protocol
    /// fee from the outbound amount.
    Transfer,

    /// The seized supply shares are credited to a controller account instead of being
    /// withdrawn. `0` creates a fresh account owned by the liquidator and bound to the
    /// liquidated account's spoke; any other id must already exist, not be the liquidated
    /// account itself, be owned by (or delegated to) the liquidator, sit in the liquidated
    /// account's spoke, and be in `PositionMode::Normal`.
    Credit(u64),
}

/// One collateral asset seized during a liquidation.
///
/// Two parallel representations of the same seizure are carried, because the two seize modes
/// consume different ones:
///
/// - `amount` / `protocol_fee` are asset units and drive `SeizeMode::Transfer`, which routes
///   through the pool's withdraw leg. `amount` is **gross**: the whole seizure the liquidated
///   account gives up, with `protocol_fee` still inside it. The liquidator is paid
///   `amount - protocol_fee`, the pool withholding the fee from the outbound transfer and
///   booking it as revenue. Reading `amount` as the liquidator's proceeds over-counts by the
///   fee.
/// - `scaled_amount` / `bonus_scaled` / `liquidation_fees` are the RAY-scaled supply shares and
///   the fee rate that drive `SeizeMode::Credit`, which moves shares between accounts without
///   touching pool cash. `scaled_amount` is gross on the same footing — the receiving account is
///   credited `scaled_amount` minus the fee share `split_seized_shares` derives.
///   `bonus_scaled` is the share-denominated bonus portion the protocol fee
///   is charged on; `liquidation_fees` is the seized position's own stamped fee rate in basis
///   points. Carrying the rate on the entry keeps the credit-mode split derivable from the
///   entry alone, so every site that recomputes it gets an identical answer.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub hub_asset: HubAssetKey,

    pub amount: i128,

    pub protocol_fee: i128,

    pub scaled_amount: i128,

    pub bonus_scaled: i128,

    pub liquidation_fees: u32,
    pub feed: PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

/// One debt asset repaid during a liquidation: amount repaid, its USD-equivalent value, and
/// the price feed and market index used to value it.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub hub_asset: HubAssetKey,

    pub amount: i128,

    pub usd_wad: i128,
    pub feed: PriceFeedRaw,
    pub market_index: crate::types::pool::MarketIndexRaw,
}

/// Full outcome of an executed liquidation: seized collateral entries, repaid debt entries,
/// any refunded payments, the maximum USD-equivalent debt repaid, and the bonus rate applied.
#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,

    pub max_debt_usd: i128,

    pub bonus_bps: i128,
}

/// An account's full position state: owner, spoke membership, position mode, and its supply
/// and borrow position maps keyed by hub asset.
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
    /// Returns the account's existing supply position for `hub_asset`, or a freshly seeded
    /// zero-amount position carrying `config`'s risk parameters if none exists yet.
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

    /// Returns the account's existing debt position for `hub_asset`, or a freshly seeded
    /// zero-amount position if none exists yet.
    pub fn get_or_create_debt_position(&self, hub_asset: &HubAssetKey) -> DebtPosition {
        self.borrow_positions
            .get(hub_asset.clone())
            .map(|raw| DebtPosition::from(&raw))
            .unwrap_or(DebtPosition {
                scaled_amount: Ray::ZERO,
            })
    }

    /// Returns true if the account holds neither supply nor borrow positions.
    pub fn is_empty(&self) -> bool {
        self.supply_positions.is_empty() && self.borrow_positions.is_empty()
    }

    /// Returns true if the account holds no borrow positions.
    pub fn debt_free(&self) -> bool {
        self.borrow_positions.is_empty()
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
            no_seize: false,
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

    fn account_meta(_env: &Env, spoke_id: u32) -> AccountMeta {
        AccountMeta {
            spoke_id,
            mode: PositionMode::Normal,
        }
    }

    fn empty_account(env: &Env, meta: AccountMeta) -> Account {
        Account {
            owner: Address::generate(env),
            spoke_id: meta.spoke_id,
            mode: meta.mode,
            supply_positions: Map::new(env),
            borrow_positions: Map::new(env),
        }
    }

    #[test]
    fn debt_free_is_true_iff_borrow_map_empty() {
        let env = Env::default();
        let meta = account_meta(&env, 1);
        let mut account = empty_account(&env, meta);
        assert!(account.debt_free());
        assert!(account.is_empty());

        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        account
            .borrow_positions
            .set(hub, DebtPositionRaw { scaled_amount: 1 });
        assert!(!account.debt_free());
        assert!(!account.is_empty());
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
}

/// Storage keys for all controller contract state: singleton protocol settings, per-hub and
/// per-spoke configuration, per-spoke-asset configuration and usage, the address-keyed
/// position-manager and Blend-pool registries, and per-account metadata, positions, and
/// delegates, all keyed by the `u64` account id.
#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    Pool,

    SwapAggregator,

    PriceAggregator,
    PositionNft,
    Accumulator,
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
