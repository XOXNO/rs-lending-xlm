extern crate std;

use super::*;
use crate::access::{KEEPER_ROLE, ORACLE_ROLE, REVENUE_ROLE};
use crate::positions::update;
use common::types::{
    AccountPosition, AccountPositionType, AssetConfig, ExchangeSource, MarketConfig,
    MarketOracleConfigInput, MarketStatus, OraclePriceFluctuation, OracleProviderConfig,
    OracleType, PositionLimits, PositionMode, ReflectorAssetKind,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol};
use stellar_access::{access_control, ownable};

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

struct TestSetup {
    env: Env,
    admin: Address,
    contract: Address,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract = env.register(crate::Controller, (admin.clone(),));

        TestSetup {
            env,
            admin,
            contract,
        }
    }

    fn client(&self) -> crate::ControllerClient<'_> {
        crate::ControllerClient::new(&self.env, &self.contract)
    }

    fn setup_reflector(&self, asset: &Address) -> Address {
        let reflector = self
            .env
            .register(crate::helpers::testutils::TestReflector, ());
        let r_client = crate::helpers::testutils::TestReflectorClient::new(&self.env, &reflector);
        r_client.set_spot(
            &crate::helpers::testutils::TestReflectorAsset::Stellar(asset.clone()),
            &10_0000000_0000000i128,
            &10_000,
        );
        reflector
    }

    fn sample_asset_config(&self) -> AssetConfig {
        AssetConfig {
            loan_to_value_bps: 7500,
            liquidation_threshold_bps: 8000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            e_mode_enabled: false,
            is_isolated_asset: false,
            is_siloed_borrowing: false,
            is_flashloanable: true,
            isolation_borrow_enabled: false,
            isolation_debt_ceiling_usd_wad: 0,
            flashloan_fee_bps: 9,
            borrow_cap: i128::MAX,
            supply_cap: i128::MAX,
        }
    }

    fn seed_market_config(&self, asset: &Address) {
        self.env.as_contract(&self.contract, || {
            let default_oracle = OracleProviderConfig {
                base_asset: asset.clone(),
                oracle_type: OracleType::None,
                exchange_source: ExchangeSource::SpotOnly,
                asset_decimals: 7,
                tolerance: OraclePriceFluctuation {
                    first_upper_ratio_bps: 0,
                    first_lower_ratio_bps: 0,
                    last_upper_ratio_bps: 0,
                    last_lower_ratio_bps: 0,
                },
                max_price_stale_seconds: 900,
            };
            let market = MarketConfig {
                status: MarketStatus::PendingOracle,
                asset_config: self.sample_asset_config(),
                pool_address: Address::generate(&self.env),
                oracle_config: default_oracle,
                cex_oracle: None,
                cex_asset_kind: ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, ""),
                cex_decimals: 0,
                dex_oracle: None,
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 0,
                twap_records: 0,
            };
            storage::set_market_config(&self.env, asset, &market);
        });
    }
}

// -----------------------------------------------------------------------
// Test: constructor sets admin and position limits
// -----------------------------------------------------------------------
#[test]
fn test_constructor_sets_admin_and_limits() {
    let t = TestSetup::new();

    t.env.as_contract(&t.contract, || {
        // Verify owner storage.
        let stored_owner = ownable::get_owner(&t.env);
        assert_eq!(stored_owner, Some(t.admin.clone()));

        // Verify AccessControl admin.
        let stored_ac_admin = access_control::get_admin(&t.env);
        assert_eq!(stored_ac_admin, Some(t.admin.clone()));

        // Only KEEPER is granted at construct; REVENUE and ORACLE require explicit grant post-deploy.
        assert!(
            access_control::has_role(&t.env, &t.admin, &Symbol::new(&t.env, KEEPER_ROLE)).is_some()
        );
        assert!(
            access_control::has_role(&t.env, &t.admin, &Symbol::new(&t.env, REVENUE_ROLE))
                .is_none(),
            "REVENUE must NOT be granted at construct"
        );
        assert!(
            access_control::has_role(&t.env, &t.admin, &Symbol::new(&t.env, ORACLE_ROLE)).is_none(),
            "ORACLE must NOT be granted at construct"
        );

        // Contract is paused at construct.
        assert!(
            stellar_contract_utils::pausable::paused(&t.env),
            "contract must be paused at construct"
        );

        // Verify default position limits.
        let limits = storage::get_position_limits(&t.env);
        assert_eq!(limits.max_supply_positions, 10);
        assert_eq!(limits.max_borrow_positions, 10);
    });
}

// -----------------------------------------------------------------------
// Test: create_account increments nonce, stores owner and attrs
// -----------------------------------------------------------------------
#[test]
fn test_create_account() {
    let t = TestSetup::new();
    let owner = Address::generate(&t.env);

    t.env.as_contract(&t.contract, || {
        let (id1, account) =
            utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
        assert_eq!(id1, 1);

        let (id2, _) = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
        assert_eq!(id2, 2);

        // Verify owner.
        assert_eq!(account.owner, owner);

        // Verify attrs.
        assert!(!account.is_isolated);
        assert_eq!(account.e_mode_category_id, 0);
        assert_eq!(account.mode, PositionMode::Normal);

        // Verify empty position maps.
        assert_eq!(account.supply_positions.len(), 0);
        assert_eq!(account.borrow_positions.len(), 0);
    });
}

// -----------------------------------------------------------------------
// Test: remove_account cleans up storage
// -----------------------------------------------------------------------
#[test]
fn test_remove_account() {
    let t = TestSetup::new();
    let owner = Address::generate(&t.env);

    // Verify storage is cleaned.
    t.env.as_contract(&t.contract, || {
        let (id, _) = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
        assert_eq!(id, 1);

        utils::remove_account(&t.env, id);

        let exists = storage::try_get_account(&t.env, id).is_some();
        assert!(!exists, "account should be removed");
    });
}

// -----------------------------------------------------------------------
// Test: update_or_remove_position adds to position list
// -----------------------------------------------------------------------
#[test]
fn test_update_or_remove_position_stores_position() {
    let t = TestSetup::new();
    let owner = Address::generate(&t.env);
    let asset = Address::generate(&t.env);

    t.env.as_contract(&t.contract, || {
        let (id, _) = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
        let position = AccountPosition {
            position_type: AccountPositionType::Deposit,
            asset: asset.clone(),
            scaled_amount_ray: 1_000_000,
            account_id: id,
            liquidation_threshold_bps: 8000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            loan_to_value_bps: 7500,
        };
        let mut account = storage::get_account(&t.env, id);
        update::update_or_remove_position(&mut account, &position);
        storage::set_account(&t.env, id, &account);

        // Check that the position map has the asset.
        let account = storage::get_account(&t.env, id);
        assert_eq!(account.supply_positions.len(), 1);
        let stored = account.supply_positions.get(asset.clone());
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().scaled_amount_ray, 1_000_000);

        // Store the same asset again; the position must not duplicate.
        let mut account = storage::get_account(&t.env, id);
        update::update_or_remove_position(&mut account, &position);
        storage::set_account(&t.env, id, &account);
        let account = storage::get_account(&t.env, id);
        assert_eq!(account.supply_positions.len(), 1);
    });
}

// -----------------------------------------------------------------------
// Test: update_or_remove_position removes when zero
// -----------------------------------------------------------------------
#[test]
fn test_update_or_remove_position_zero() {
    let t = TestSetup::new();
    let owner = Address::generate(&t.env);
    let asset = Address::generate(&t.env);

    t.env.as_contract(&t.contract, || {
        let (id, _) = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
        // Store a position first.
        let position = AccountPosition {
            position_type: AccountPositionType::Deposit,
            asset: asset.clone(),
            scaled_amount_ray: 1_000_000,
            account_id: id,
            liquidation_threshold_bps: 8000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            loan_to_value_bps: 7500,
        };
        let mut account = storage::get_account(&t.env, id);
        update::update_or_remove_position(&mut account, &position);
        storage::set_account(&t.env, id, &account);

        // Now update with zero amount.
        let zero_position = AccountPosition {
            position_type: AccountPositionType::Deposit,
            asset: asset.clone(),
            scaled_amount_ray: 0,
            account_id: id,
            liquidation_threshold_bps: 8000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            loan_to_value_bps: 7500,
        };
        let mut account = storage::get_account(&t.env, id);
        update::update_or_remove_position(&mut account, &zero_position);
        storage::set_account(&t.env, id, &account);

        // Position should be removed.
        let account = storage::get_account(&t.env, id);
        let stored = account.supply_positions.get(asset.clone());
        assert!(stored.is_none());

        // Position map should be empty.
        assert_eq!(account.supply_positions.len(), 0);
    });
}

// -----------------------------------------------------------------------
// Test: config endpoints require admin auth
// -----------------------------------------------------------------------
#[test]
#[should_panic]
fn test_config_requires_admin() {
    let env = Env::default();
    // Do NOT mock all auths.
    let admin = Address::generate(&env);
    let contract = env.register(Controller, (admin.clone(),));
    let client = ControllerClient::new(&env, &contract);

    let _non_admin = Address::generate(&env);
    let limits = PositionLimits {
        max_supply_positions: 10,
        max_borrow_positions: 10,
    };
    // Must panic: non_admin is not admin.
    client.set_position_limits(&limits);
}

// -----------------------------------------------------------------------
// Test: edit_asset_config validates threshold > LTV
// -----------------------------------------------------------------------
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_edit_asset_config_threshold_validation() {
    let t = TestSetup::new();
    let client = t.client();
    let asset = Address::generate(&t.env);

    let mut bad_config = t.sample_asset_config();
    // Set threshold <= LTV to trigger validation.
    bad_config.loan_to_value_bps = 8000;
    bad_config.liquidation_threshold_bps = 8000; // equal, must fail

    client.edit_asset_config(&asset, &bad_config);
}

// -----------------------------------------------------------------------
// Test: edit_asset_config succeeds with valid params
// -----------------------------------------------------------------------
#[test]
fn test_edit_asset_config_valid() {
    let t = TestSetup::new();
    let client = t.client();
    let asset = Address::generate(&t.env);

    // Seed a default market so edit_asset_config can read-modify-write.
    t.seed_market_config(&asset);

    let config = t.sample_asset_config();
    client.edit_asset_config(&asset, &config);

    t.env.as_contract(&t.contract, || {
        let market = storage::get_market_config(&t.env, &asset);
        assert_eq!(market.asset_config.loan_to_value_bps, 7500);
        assert_eq!(market.asset_config.liquidation_threshold_bps, 8000);
    });
}

#[test]
fn test_edit_asset_config_preserves_existing_emode_flag() {
    let t = TestSetup::new();
    let client = t.client();
    let asset = Address::generate(&t.env);

    t.seed_market_config(&asset);

    t.env.as_contract(&t.contract, || {
        let mut market = storage::get_market_config(&t.env, &asset);
        market.asset_config.e_mode_enabled = true;
        storage::set_market_config(&t.env, &asset, &market);
    });

    let mut config = t.sample_asset_config();
    config.e_mode_enabled = false;
    client.edit_asset_config(&asset, &config);

    t.env.as_contract(&t.contract, || {
        let market = storage::get_market_config(&t.env, &asset);
        assert!(market.asset_config.e_mode_enabled);
    });
}

// -----------------------------------------------------------------------
// Test: add_e_mode_category auto-increments ID
// -----------------------------------------------------------------------
#[test]
fn test_add_e_mode_category_auto_increment() {
    let t = TestSetup::new();
    let client = t.client();

    let id1 = client.add_e_mode_category(&9700i128, &9800i128, &200i128);
    assert_eq!(id1, 1);

    let id2 = client.add_e_mode_category(&9500i128, &9600i128, &300i128);
    assert_eq!(id2, 2);

    t.env.as_contract(&t.contract, || {
        // Verify stored category.
        let cat = storage::get_emode_category(&t.env, id1);
        assert_eq!(cat.category_id, 1);
        assert_eq!(cat.loan_to_value_bps, 9700);
        assert_eq!(cat.liquidation_threshold_bps, 9800);
        assert!(!cat.is_deprecated);
    });
}

// -----------------------------------------------------------------------
// Test: add_e_mode_category rejects threshold <= LTV
// -----------------------------------------------------------------------
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_add_e_mode_category_bad_params() {
    let t = TestSetup::new();
    let client = t.client();

    // threshold == ltv must fail.
    client.add_e_mode_category(&9800i128, &9800i128, &200i128);
}

// -----------------------------------------------------------------------
// Test: remove_e_mode_category deprecates
// -----------------------------------------------------------------------
#[test]
fn test_remove_e_mode_category() {
    let t = TestSetup::new();
    let client = t.client();

    let id = client.add_e_mode_category(&9700i128, &9800i128, &200i128);
    client.remove_e_mode_category(&id);

    t.env.as_contract(&t.contract, || {
        let cat = storage::get_emode_category(&t.env, id);
        assert!(cat.is_deprecated);
    });
}

// -----------------------------------------------------------------------
// Test: add_asset_to_emode rejects deprecated category
// -----------------------------------------------------------------------
#[test]
#[should_panic(expected = "Error(Contract, #301)")]
fn test_add_asset_to_deprecated_e_mode() {
    let t = TestSetup::new();
    let client = t.client();
    let asset = Address::generate(&t.env);

    let id = client.add_e_mode_category(&9700i128, &9800i128, &200i128);
    client.remove_e_mode_category(&id);

    // Must fail: the category is deprecated.
    client.add_asset_to_e_mode_category(&asset, &id, &true, &true);
}

// -----------------------------------------------------------------------
// Test: create isolated account stores isolated asset
// -----------------------------------------------------------------------
#[test]
fn test_create_isolated_account() {
    let t = TestSetup::new();
    let owner = Address::generate(&t.env);
    let iso_asset = Address::generate(&t.env);

    t.env.as_contract(&t.contract, || {
        let (id, account) = utils::create_account(
            &t.env,
            &owner,
            0,
            PositionMode::Normal,
            true,
            Some(iso_asset.clone()),
        );
        assert_eq!(id, 1);
        assert!(account.is_isolated);

        // Check that the isolated asset is stored on the account.
        assert_eq!(account.isolated_asset, Some(iso_asset));
    });
}

// -----------------------------------------------------------------------
// Test: position storage reaches configured max (no enforcement assertion)
// End-to-end enforcement (rejecting a third asset) lives in
// test-harness::supply_tests::test_supply_position_limit_exceeded.
// -----------------------------------------------------------------------
#[test]
fn test_position_limit_reaches_configured_max() {
    let t = TestSetup::new();
    let client = t.client();
    let owner = Address::generate(&t.env);
    let id = t.env.as_contract(&t.contract, || {
        utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None).0
    });

    // Set tight limits.
    client.set_position_limits(&PositionLimits {
        max_supply_positions: 2,
        max_borrow_positions: 2,
    });

    // Store two supply positions inside contract context.
    let asset1 = Address::generate(&t.env);
    let asset2 = Address::generate(&t.env);

    t.env.as_contract(&t.contract, || {
        let mut account = storage::get_account(&t.env, id);
        for asset in [&asset1, &asset2] {
            let pos = AccountPosition {
                position_type: AccountPositionType::Deposit,
                asset: asset.clone(),
                scaled_amount_ray: 1000,
                account_id: id,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            };
            update::update_or_remove_position(&mut account, &pos);
        }
        storage::set_account(&t.env, id, &account);

        // After storing two positions the supply_positions map is at the
        // configured limit. End-to-end enforcement is covered separately
        // in the harness suite (see banner above).
        let account = storage::get_account(&t.env, id);
        assert_eq!(account.supply_positions.len(), 2);
        let limits = storage::get_position_limits(&t.env);
        assert!(
            account.supply_positions.len() >= limits.max_supply_positions,
            "should be at limit"
        );
    });
}

// -----------------------------------------------------------------------
// Test: oracle tolerance validation
// -----------------------------------------------------------------------
#[test]
#[should_panic(expected = "Error(Contract, #207)")]
fn test_edit_oracle_tolerance_bad_first() {
    let t = TestSetup::new();
    let client = t.client();
    // M-02 + M-03 hardening: grant ORACLE role and unpause.
    client.grant_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE));
    client.unpause();
    let asset = t
        .env
        .register_stellar_asset_contract_v2(Address::generate(&t.env))
        .address()
        .clone();

    // Seed a default market so configure_market_oracle can read-modify-write.
    t.seed_market_config(&asset);
    let oracle_config = MarketOracleConfigInput {
        exchange_source: ExchangeSource::SpotVsTwap,
        max_price_stale_seconds: 900,
        first_tolerance_bps: 200,
        last_tolerance_bps: 500,
        cex_oracle: t.setup_reflector(&asset),
        cex_asset_kind: ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(&t.env, "USDC"),
        dex_oracle: None,
        dex_asset_kind: ReflectorAssetKind::Stellar,
        dex_symbol: Symbol::new(&t.env, ""),
        twap_records: 3,
    };
    client.configure_market_oracle(&t.admin, &asset, &oracle_config);

    // first=10 falls below MIN_FIRST_TOLERANCE.
    client.edit_oracle_tolerance(&t.admin, &asset, &10, &500);
}

// -----------------------------------------------------------------------
// Test: unsupported oracle modes are rejected at configuration time
// -----------------------------------------------------------------------
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_configure_market_oracle_rejects_missing_dual_oracle_dex() {
    let t = TestSetup::new();
    let client = t.client();
    // M-02 + M-03 hardening: grant ORACLE role and unpause.
    client.grant_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE));
    client.unpause();
    let asset = t
        .env
        .register_stellar_asset_contract_v2(Address::generate(&t.env))
        .address()
        .clone();

    t.seed_market_config(&asset);

    let oracle_config = MarketOracleConfigInput {
        exchange_source: ExchangeSource::DualOracle,
        max_price_stale_seconds: 900,
        first_tolerance_bps: 200,
        last_tolerance_bps: 500,
        cex_oracle: t.setup_reflector(&asset),
        cex_asset_kind: ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(&t.env, "USDC"),
        dex_oracle: None,
        dex_asset_kind: ReflectorAssetKind::Stellar,
        dex_symbol: Symbol::new(&t.env, ""),
        twap_records: 3,
    };

    client.configure_market_oracle(&t.admin, &asset, &oracle_config);
}

// -----------------------------------------------------------------------
// Test: pause blocks user endpoints, unpause re-enables them
// -----------------------------------------------------------------------
#[test]
fn test_pause_and_unpause() {
    let t = TestSetup::new();
    let client = t.client();

    // Paused at construct; operator must unpause before user-facing flows run.
    t.env.as_contract(&t.contract, || {
        assert!(stellar_contract_utils::pausable::paused(&t.env));
    });
    client.unpause();
    t.env.as_contract(&t.contract, || {
        assert!(!stellar_contract_utils::pausable::paused(&t.env));
    });

    // Pause.
    client.pause();
    t.env.as_contract(&t.contract, || {
        assert!(stellar_contract_utils::pausable::paused(&t.env));
    });

    // Unpause.
    client.unpause();
    t.env.as_contract(&t.contract, || {
        assert!(!stellar_contract_utils::pausable::paused(&t.env));
    });
}

// -----------------------------------------------------------------------
// Test: require_not_paused panics when paused (error #1000)
// -----------------------------------------------------------------------
#[test]
#[should_panic(expected = "Error(Contract, #1000)")]
fn test_require_not_paused_blocks_when_paused() {
    let t = TestSetup::new();

    t.env.as_contract(&t.contract, || {
        stellar_contract_utils::pausable::pause(&t.env);
        stellar_contract_utils::pausable::when_not_paused(&t.env);
    });
}

// -----------------------------------------------------------------------
// Test: require_not_paused passes when not paused
// -----------------------------------------------------------------------
#[test]
fn test_require_not_paused_passes_when_unpaused() {
    let t = TestSetup::new();

    // Constructor pauses the contract; unpause first to verify the guard passes.
    t.client().unpause();
    t.env.as_contract(&t.contract, || {
        stellar_contract_utils::pausable::when_not_paused(&t.env);
    });
}

// -----------------------------------------------------------------------
// Test: pause/unpause require admin auth
// -----------------------------------------------------------------------
#[test]
#[should_panic]
fn test_pause_requires_admin() {
    let env = Env::default();
    // Do NOT mock all auths -- auth must fail.
    let admin = Address::generate(&env);
    let contract = env.register(Controller, (admin.clone(),));
    let client = ControllerClient::new(&env, &contract);

    // Call pause without auth -- must panic.
    client.pause();
}

// -----------------------------------------------------------------------
// Test: role-based access control
// -----------------------------------------------------------------------
#[test]
fn test_role_based_access() {
    let t = TestSetup::new();
    let client = t.client();

    // Only KEEPER is granted at construct; REVENUE and ORACLE require explicit grant post-deploy.
    assert!(client.has_role(&t.admin, &Symbol::new(&t.env, KEEPER_ROLE)));
    assert!(!client.has_role(&t.admin, &Symbol::new(&t.env, REVENUE_ROLE)));
    assert!(!client.has_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE)));

    // Operator post-deploy hardening grants the other two roles.
    client.grant_role(&t.admin, &Symbol::new(&t.env, REVENUE_ROLE));
    client.grant_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE));
    assert!(client.has_role(&t.admin, &Symbol::new(&t.env, REVENUE_ROLE)));
    assert!(client.has_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE)));

    // Grant KEEPER role to a bot address.
    let keeper_bot = Address::generate(&t.env);
    assert!(!client.has_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE)));

    client.grant_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE));
    assert!(client.has_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE)));

    // Bot must NOT hold other roles.
    assert!(!client.has_role(&keeper_bot, &Symbol::new(&t.env, REVENUE_ROLE)));

    // Revoke KEEPER role from the bot.
    client.revoke_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE));
    assert!(!client.has_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE)));
}

// -----------------------------------------------------------------------
// Test: ownership transfer keeps access-control admin and bootstrap roles aligned
// -----------------------------------------------------------------------
#[test]
fn test_transfer_ownership_syncs_admin_and_roles() {
    let t = TestSetup::new();
    let client = t.client();
    let new_owner = Address::generate(&t.env);
    let live_until = t.env.ledger().sequence() + 100;

    client.transfer_ownership(&new_owner, &live_until);
    t.env.as_contract(&t.contract, || {
        assert!(stellar_access::role_transfer::has_active_pending_transfer(
            &t.env,
            &access_control::AccessControlStorageKey::PendingAdmin,
        ));
    });

    client.accept_ownership();

    t.env.as_contract(&t.contract, || {
        assert_eq!(ownable::get_owner(&t.env), Some(new_owner.clone()));
        assert_eq!(access_control::get_admin(&t.env), Some(new_owner.clone()));
    });

    for role_name in [KEEPER_ROLE, REVENUE_ROLE, ORACLE_ROLE] {
        let role = Symbol::new(&t.env, role_name);
        assert!(client.has_role(&new_owner, &role));
        assert!(!client.has_role(&t.admin, &role));
    }
}

// -----------------------------------------------------------------------
// Test: canceling ownership transfer also clears the mirrored admin transfer
// -----------------------------------------------------------------------
#[test]
fn test_transfer_ownership_cancel_clears_pending_admin() {
    let t = TestSetup::new();
    let client = t.client();
    let new_owner = Address::generate(&t.env);
    let live_until = t.env.ledger().sequence() + 100;

    client.transfer_ownership(&new_owner, &live_until);
    client.transfer_ownership(&new_owner, &0);

    t.env.as_contract(&t.contract, || {
        assert!(!stellar_access::role_transfer::has_active_pending_transfer(
            &t.env,
            &ownable::OwnableStorageKey::PendingOwner,
        ));
        assert!(!stellar_access::role_transfer::has_active_pending_transfer(
            &t.env,
            &access_control::AccessControlStorageKey::PendingAdmin,
        ));
        assert_eq!(access_control::get_admin(&t.env), Some(t.admin.clone()));
    });

    t.env.as_contract(&t.contract, || {
        assert_eq!(ownable::get_owner(&t.env), Some(t.admin.clone()));
    });
}

// -----------------------------------------------------------------------
// Test: ownership view
// -----------------------------------------------------------------------
#[test]
fn test_get_contract_owner() {
    let t = TestSetup::new();
    t.env.as_contract(&t.contract, || {
        assert_eq!(ownable::get_owner(&t.env), Some(t.admin.clone()));
    });
}
