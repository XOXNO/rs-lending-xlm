use super::*;
use crate::constants::RAY;
use crate::Controller;
use common::types::{PositionManagerConfig, SpokeConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::String as SdkString;

/// Registers a native position-nft owned by `controller` and records it in the
/// controller's instance storage. Returns the NFT contract address.
pub(crate) fn setup_position_nft(env: &Env, controller: &Address) -> Address {
    let nft = env.register(
        position_nft::PositionNft,
        (
            controller.clone(),
            SdkString::from_str(env, "uri"),
            SdkString::from_str(env, "Position"),
            SdkString::from_str(env, "POS"),
        ),
    );
    env.as_contract(controller, || {
        crate::storage::set_position_nft(env, &nft);
    });
    nft
}

/// Mints an NFT to `owner` and writes matching account metadata; returns the
/// account id (== token id).
pub(crate) fn seed_account(env: &Env, controller: &Address, nft: &Address, owner: &Address) -> u64 {
    let account_id = u64::from(position_nft::PositionNftClient::new(env, nft).mint(owner));
    env.as_contract(controller, || {
        crate::storage::set_account_meta(
            env,
            account_id,
            &AccountMeta {
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
    });
    account_id
}

#[test]
fn owner_passes() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        require_owner_or_delegate(&env, account_id, &owner, &owner);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn stranger_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        require_owner_or_delegate(&env, account_id, &Address::generate(&env), &owner);
    });
}

#[test]
fn active_registered_opted_in_delegate_passes() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    let manager = Address::generate(&env);

    env.as_contract(&contract_id, || {
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        storage::add_delegate(&env, account_id, &owner, &manager);

        require_owner_or_delegate(&env, account_id, &manager, &owner);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn registered_but_not_opted_in_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    let manager = Address::generate(&env);

    env.as_contract(&contract_id, || {
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });

        require_owner_or_delegate(&env, account_id, &manager, &owner);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn opted_in_but_manager_inactive_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    let manager = Address::generate(&env);

    env.as_contract(&contract_id, || {
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        storage::add_delegate(&env, account_id, &owner, &manager);

        require_owner_or_delegate(&env, account_id, &manager, &owner);
    });
}

#[test]
fn require_account_owner_owner_passes() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        let meta = require_account_owner(&env, account_id, &owner);
        assert_eq!(storage::account_owner(&env, account_id), owner);
        assert_eq!(meta.spoke_id, 0);
        assert_eq!(meta.mode, PositionMode::Normal);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn require_account_owner_stranger_rejected_as_account_not_in_market() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        let _ = require_account_owner(&env, account_id, &Address::generate(&env));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn require_account_owner_missing_account_rejected_as_account_not_in_market() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let caller = Address::generate(&env);
        let _ = require_account_owner(&env, 99u64, &caller);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn require_account_owner_rejects_active_delegate() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    let manager = Address::generate(&env);

    env.as_contract(&contract_id, || {
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        storage::add_delegate(&env, account_id, &owner, &manager);

        // Owner-only: delegates must not pass even when fully opted-in/active.
        let _ = require_account_owner(&env, account_id, &manager);
    });
}

#[test]
fn load_existing_account_requires_matching_spoke() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        let mut cache = crate::context::Cache::new_view(&env);
        let (id, account) = load_or_create_account(
            &env,
            &owner,
            account_id,
            0,
            PositionMode::Normal,
            AccountGuard::Supply,
            &mut cache,
        );
        assert_eq!(id, account_id);
        assert_eq!(account.spoke_id, 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #310)")]
fn load_existing_account_rejects_spoke_mismatch() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        let mut cache = crate::context::Cache::new_view(&env);
        let _ = load_or_create_account(
            &env,
            &owner,
            account_id,
            1,
            PositionMode::Normal,
            AccountGuard::Supply,
            &mut cache,
        );
    });
}

#[test]
fn update_or_remove_supply_zero_removes_nonzero_keeps() {
    use common::math::fp::{Bps, Ray};
    use common::types::{Account, AccountPosition, HubAssetKey};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let owner = Address::generate(&env);
        let mut account = Account {
            owner,
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let live = AccountPosition {
            scaled_amount: Ray::from(RAY),
            liquidation_threshold: Bps::from(8_000),
            liquidation_bonus: Bps::from(500),
            loan_to_value: Bps::from(7_500),
            liquidation_fees: Bps::from(100),
        };
        update_or_remove_supply_position(&mut account, &hub, &live);
        assert_eq!(
            account
                .supply_positions
                .get(hub.clone())
                .unwrap()
                .scaled_amount,
            RAY
        );
        let zero = AccountPosition {
            scaled_amount: Ray::ZERO,
            ..live
        };
        update_or_remove_supply_position(&mut account, &hub, &zero);
        assert!(!account.supply_positions.contains_key(hub));
    });
}

#[test]
fn update_or_remove_debt_zero_removes_nonzero_keeps() {
    use common::math::fp::Ray;
    use common::types::{Account, DebtPosition, HubAssetKey};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let owner = Address::generate(&env);
        let mut account = Account {
            owner,
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        update_or_remove_debt_position(
            &mut account,
            &hub,
            &DebtPosition {
                scaled_amount: Ray::from(RAY),
            },
        );
        assert_eq!(
            account
                .borrow_positions
                .get(hub.clone())
                .unwrap()
                .scaled_amount,
            RAY
        );
        update_or_remove_debt_position(
            &mut account,
            &hub,
            &DebtPosition {
                scaled_amount: Ray::ZERO,
            },
        );
        assert!(!account.borrow_positions.contains_key(hub));
    });
}

#[test]
fn cleanup_account_if_empty_removes_only_empty() {
    use common::types::{Account, AccountPositionRaw, HubAssetKey};
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);

    let empty_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        let empty = Account {
            owner: owner.clone(),
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        cleanup_account_if_empty(&env, &empty, empty_id);
        assert!(storage::try_get_account_meta(&env, empty_id).is_none());
    });

    let live_id = seed_account(&env, &contract_id, &nft, &owner);
    env.as_contract(&contract_id, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut supply = Map::new(&env);
        supply.set(
            hub,
            AccountPositionRaw {
                scaled_amount: RAY,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        let live = Account {
            owner,
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: supply,
            borrow_positions: Map::new(&env),
        };
        cleanup_account_if_empty(&env, &live, live_id);
        assert!(storage::try_get_account_meta(&env, live_id).is_some());
    });
}

#[test]
fn transfer_revokes_prior_owner_and_delegates() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let manager = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &alice);

    env.as_contract(&contract_id, || {
        crate::storage::set_position_manager(
            &env,
            &manager,
            &common::types::PositionManagerConfig { is_active: true },
        );
        assert!(crate::storage::add_delegate(
            &env, account_id, &alice, &manager
        ));
        // Pre-transfer: owner and delegate both authorized.
        assert!(is_owner_or_delegate(&env, account_id, &alice, &alice));
        assert!(is_owner_or_delegate(&env, account_id, &manager, &alice));
    });

    position_nft::PositionNftClient::new(&env, &nft).transfer(
        &alice,
        &bob,
        &u32::try_from(account_id).unwrap(),
    );

    env.as_contract(&contract_id, || {
        let owner = crate::storage::account_owner(&env, account_id);
        assert_eq!(owner, bob);
        // Old owner and the grant they made are both dead.
        assert!(!is_owner_or_delegate(&env, account_id, &alice, &owner));
        assert!(!is_owner_or_delegate(&env, account_id, &manager, &owner));
        // New owner works, and a fresh grant by the new owner works.
        assert!(is_owner_or_delegate(&env, account_id, &bob, &owner));
        assert!(crate::storage::add_delegate(
            &env, account_id, &bob, &manager
        ));
        assert!(is_owner_or_delegate(&env, account_id, &manager, &owner));
    });
}

#[test]
fn account_owner_fails_closed() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let _nft = setup_position_nft(&env, &contract_id);

    env.as_contract(&contract_id, || {
        // Never minted.
        assert!(crate::storage::try_account_owner(&env, 7u64).is_none());
        // Outside the mintable domain — the narrowing rejects, no panic.
        assert!(crate::storage::try_account_owner(&env, u64::from(u32::MAX) + 1).is_none());
        assert!(crate::storage::try_account_owner(&env, u64::MAX).is_none());
    });
}

#[test]
fn cleanup_empty_account_burns_nft() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);

    let alice = Address::generate(&env);
    let account_id = seed_account(&env, &contract_id, &nft, &alice);

    env.as_contract(&contract_id, || {
        let account = crate::storage::get_account(&env, account_id);
        assert!(account.is_empty());
        cleanup_account_if_empty(&env, &account, account_id);
        assert!(crate::storage::try_get_account_meta(&env, account_id).is_none());
        assert!(crate::storage::try_account_owner(&env, account_id).is_none());
    });
    assert!(position_nft::PositionNftClient::new(&env, &nft)
        .try_owner_of(&u32::try_from(account_id).unwrap())
        .is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #53)")]
fn account_creation_before_nft_deploy_fails_closed() {
    // The PositionNft key is deliberately left unset (no `setup_position_nft`
    // call): governance has activated a spoke but never deployed the NFT.
    // `create_account` must fail closed with `PositionNftNotSet` (#53) rather
    // than skip minting or fall back to some default authority.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let owner = Address::generate(&env);

    env.as_contract(&contract_id, || {
        crate::storage::set_spoke(
            &env,
            1,
            &SpokeConfig {
                is_deprecated: false,
                liquidation_target_hf_wad: 0,
                hf_for_max_bonus_wad: 0,
                liquidation_bonus_factor_bps: 0,
            },
        );
        let mut cache = crate::context::Cache::new(&env);
        create_account(&env, &owner, 1, PositionMode::Normal, &mut cache);
    });
}
