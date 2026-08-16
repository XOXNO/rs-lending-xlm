use super::*;
use crate::constants::MAX_DELEGATES;
use crate::Controller;
use common::types::{AccountPositionRaw, DebtPositionRaw, HubAssetKey, PositionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Map};

/// Registers a native position-nft owned by `controller` and records it in the controller's
/// instance storage. Returns the NFT contract address. Duplicated from
/// `tests/helpers/account.rs`: pinned test modules cannot import each other.
fn setup_position_nft(env: &Env, controller: &Address) -> Address {
    let nft = env.register(
        position_nft::PositionNft,
        (
            controller.clone(),
            soroban_sdk::String::from_str(env, "uri"),
            soroban_sdk::String::from_str(env, "Position"),
            soroban_sdk::String::from_str(env, "POS"),
        ),
    );
    env.as_contract(controller, || {
        crate::storage::set_position_nft(env, &nft);
    });
    nft
}

#[test]
fn add_delegate_is_idempotent() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 7u64;
        let owner = Address::generate(&env);
        let delegate = Address::generate(&env);

        add_delegate(&env, account_id, &owner, &delegate);
        add_delegate(&env, account_id, &owner, &delegate);

        let delegates = get_delegates(&env, account_id, &owner);
        assert_eq!(delegates.len(), 1);
        assert!(delegates.contains(delegate.clone()));
    });
}

#[test]
fn remove_delegate_revokes_access() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 9u64;
        let owner = Address::generate(&env);
        let keep = Address::generate(&env);
        let drop = Address::generate(&env);

        add_delegate(&env, account_id, &owner, &keep);
        add_delegate(&env, account_id, &owner, &drop);
        remove_delegate(&env, account_id, &owner, &drop);

        let delegates = get_delegates(&env, account_id, &owner);
        assert_eq!(delegates.len(), 1);
        assert!(delegates.contains(keep));
        assert!(!delegates.contains(drop));
    });
}

#[test]
fn remove_absent_delegate_is_noop() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 11u64;
        let owner = Address::generate(&env);
        remove_delegate(&env, account_id, &owner, &Address::generate(&env));
        assert_eq!(get_delegates(&env, account_id, &owner).len(), 0);
    });
}

#[test]
fn add_delegate_accepts_exactly_max_delegates() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 13u64;
        let owner = Address::generate(&env);
        for _ in 0..MAX_DELEGATES {
            add_delegate(&env, account_id, &owner, &Address::generate(&env));
        }
        assert_eq!(get_delegates(&env, account_id, &owner).len(), MAX_DELEGATES);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #45)")]
fn add_delegate_rejects_delegate_past_the_cap() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 13u64;
        let owner = Address::generate(&env);
        for _ in 0..MAX_DELEGATES {
            add_delegate(&env, account_id, &owner, &Address::generate(&env));
        }
        add_delegate(&env, account_id, &owner, &Address::generate(&env));
    });
}

#[test]
fn renew_user_account_renews_delegates_ttl() {
    use common::constants::TTL_BUMP_USER;
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 7u64;
        let owner = Address::generate(&env);
        add_delegate(&env, account_id, &owner, &Address::generate(&env));

        env.ledger()
            .with_mut(|l| l.sequence_number += TTL_BUMP_USER - 1_000);

        let key = ControllerKey::Delegates(account_id);
        let aged = env.storage().persistent().get_ttl(&key);
        renew_user_account(&env, account_id);
        let renewed = env.storage().persistent().get_ttl(&key);

        assert!(
            renewed > aged,
            "Delegates TTL must be renewed: renewed={renewed}, aged={aged}"
        );
    });
}

/// A stale grant from a previous owner reads as empty once the NFT transfers, and gets
/// overwritten wholesale — not merged with — the new owner's next write.
#[test]
fn delegates_of_previous_owner_read_as_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let delegate = Address::generate(&env);
    let account_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&alice));

    env.as_contract(&contract_id, || {
        assert!(add_delegate(&env, account_id, &alice, &delegate));
        assert_eq!(get_delegates(&env, account_id, &alice).len(), 1);
    });

    position_nft::PositionNftClient::new(&env, &nft).transfer(
        &alice,
        &bob,
        &u32::try_from(account_id).unwrap(),
    );

    env.as_contract(&contract_id, || {
        assert_eq!(
            get_delegates(&env, account_id, &bob).len(),
            0,
            "a grant stamped by the previous owner must read as empty for the new owner"
        );
        assert!(add_delegate(&env, account_id, &bob, &delegate));
        assert_eq!(get_delegates(&env, account_id, &bob).len(), 1);
    });
}

/// A grant stamped by a previous owner must not resurrect if the NFT ever returns to
/// them: the new owner's `remove_delegate` purges the stale entry outright (even though
/// the requested delegate was never live for them and the call returns `false`), so a
/// later transfer back to the original owner finds no grant to re-arm.
#[test]
fn remove_delegate_purges_stale_grant_preventing_resurrection() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let delegate = Address::generate(&env);
    let account_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&alice));

    env.as_contract(&contract_id, || {
        assert!(add_delegate(&env, account_id, &alice, &delegate));
        assert_eq!(get_delegates(&env, account_id, &alice).len(), 1);
    });

    position_nft::PositionNftClient::new(&env, &nft).transfer(
        &alice,
        &bob,
        &u32::try_from(account_id).unwrap(),
    );

    env.as_contract(&contract_id, || {
        // Bob never granted `delegate` (or anyone), so removal reports nothing found —
        // but the stale entry stamped by alice must still be purged as a side effect.
        assert!(!remove_delegate(&env, account_id, &bob, &delegate));
        assert!(
            !env.storage()
                .persistent()
                .has(&ControllerKey::Delegates(account_id)),
            "stale grant must be purged from storage, not merely read as empty"
        );
    });

    position_nft::PositionNftClient::new(&env, &nft).transfer(
        &bob,
        &alice,
        &u32::try_from(account_id).unwrap(),
    );

    env.as_contract(&contract_id, || {
        assert_eq!(
            get_delegates(&env, account_id, &alice).len(),
            0,
            "the purged grant must not resurrect when the NFT returns to its original owner"
        );
    });
}

fn sample_supply_map(env: &Env) -> Map<HubAssetKey, AccountPositionRaw> {
    let mut map = Map::new(env);
    map.set(
        HubAssetKey {
            hub_id: 0,
            asset: Address::generate(env),
        },
        AccountPositionRaw {
            scaled_amount: 1_000,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            loan_to_value: 7_500,
            liquidation_fees: 100,
        },
    );
    map
}

fn sample_debt_map(env: &Env) -> Map<HubAssetKey, DebtPositionRaw> {
    let mut map = Map::new(env);
    map.set(
        HubAssetKey {
            hub_id: 0,
            asset: Address::generate(env),
        },
        DebtPositionRaw { scaled_amount: 500 },
    );
    map
}

#[test]
fn set_supply_positions_empty_map_removes_key() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 21u64;
        let key = ControllerKey::SupplyPositions(account_id);

        set_supply_positions(&env, account_id, &sample_supply_map(&env));
        assert!(env.storage().persistent().has(&key));

        set_supply_positions(&env, account_id, &Map::new(&env));
        assert!(
            !env.storage().persistent().has(&key),
            "empty supply map must remove SupplyPositions key"
        );
    });
}

#[test]
fn set_debt_positions_empty_map_removes_key() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 22u64;
        let key = ControllerKey::BorrowPositions(account_id);

        set_debt_positions(&env, account_id, &sample_debt_map(&env));
        assert!(env.storage().persistent().has(&key));

        set_debt_positions(&env, account_id, &Map::new(&env));
        assert!(
            !env.storage().persistent().has(&key),
            "empty debt map must remove BorrowPositions key"
        );
    });
}

/// Side writers must not co-renew siblings; renewal is owned by the caller
/// (e.g. persist_account_positions) so BOTH-side writes renew once.
#[test]
fn set_supply_positions_does_not_renew_sibling_ttls() {
    use common::constants::TTL_BUMP_USER;
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 23u64;
        let owner = Address::generate(&env);
        set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        set_debt_positions(&env, account_id, &sample_debt_map(&env));
        add_delegate(&env, account_id, &owner, &Address::generate(&env));
        // Establish live supply so subsequent set is an update.
        set_supply_positions(&env, account_id, &sample_supply_map(&env));
        renew_user_account(&env, account_id);

        env.ledger()
            .with_mut(|l| l.sequence_number += TTL_BUMP_USER - 1_000);

        let debt_key = ControllerKey::BorrowPositions(account_id);
        let delegates_key = ControllerKey::Delegates(account_id);
        let meta_key = ControllerKey::AccountMeta(account_id);
        let aged_debt = env.storage().persistent().get_ttl(&debt_key);
        let aged_delegates = env.storage().persistent().get_ttl(&delegates_key);
        let aged_meta = env.storage().persistent().get_ttl(&meta_key);

        set_supply_positions(&env, account_id, &sample_supply_map(&env));

        assert_eq!(
            env.storage().persistent().get_ttl(&debt_key),
            aged_debt,
            "set_supply_positions must not co-renew debt"
        );
        assert_eq!(
            env.storage().persistent().get_ttl(&delegates_key),
            aged_delegates,
            "set_supply_positions must not co-renew delegates"
        );
        assert_eq!(
            env.storage().persistent().get_ttl(&meta_key),
            aged_meta,
            "set_supply_positions must not co-renew meta"
        );
    });
}

#[test]
fn renew_user_account_co_renews_all_live_siblings() {
    use common::constants::TTL_BUMP_USER;
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 24u64;
        let owner = Address::generate(&env);
        set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        set_supply_positions(&env, account_id, &sample_supply_map(&env));
        set_debt_positions(&env, account_id, &sample_debt_map(&env));
        add_delegate(&env, account_id, &owner, &Address::generate(&env));
        renew_user_account(&env, account_id);

        env.ledger()
            .with_mut(|l| l.sequence_number += TTL_BUMP_USER - 1_000);

        let keys = [
            ControllerKey::AccountMeta(account_id),
            ControllerKey::SupplyPositions(account_id),
            ControllerKey::BorrowPositions(account_id),
            ControllerKey::Delegates(account_id),
        ];
        let aged = [
            env.storage().persistent().get_ttl(&keys[0]),
            env.storage().persistent().get_ttl(&keys[1]),
            env.storage().persistent().get_ttl(&keys[2]),
            env.storage().persistent().get_ttl(&keys[3]),
        ];

        renew_user_account(&env, account_id);

        for (key, aged_ttl) in keys.iter().zip(aged.iter()) {
            let renewed = env.storage().persistent().get_ttl(key);
            assert!(
                renewed > *aged_ttl,
                "sibling {key:?} must be co-renewed: renewed={renewed}, aged={aged_ttl}"
            );
        }
    });
}
