use super::*;
use crate::constants::MAX_DELEGATES;
use crate::Controller;
use common::types::{AccountPositionRaw, DebtPositionRaw, HubAssetKey, PositionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Map};

#[test]
fn add_delegate_is_idempotent() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 7u64;
        let delegate = Address::generate(&env);

        add_delegate(&env, account_id, &delegate);
        add_delegate(&env, account_id, &delegate);

        let delegates = get_delegates(&env, account_id);
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
        let keep = Address::generate(&env);
        let drop = Address::generate(&env);

        add_delegate(&env, account_id, &keep);
        add_delegate(&env, account_id, &drop);
        remove_delegate(&env, account_id, &drop);

        let delegates = get_delegates(&env, account_id);
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
        remove_delegate(&env, account_id, &Address::generate(&env));
        assert_eq!(get_delegates(&env, account_id).len(), 0);
    });
}

#[test]
fn add_delegate_accepts_exactly_max_delegates() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 13u64;
        for _ in 0..MAX_DELEGATES {
            add_delegate(&env, account_id, &Address::generate(&env));
        }
        assert_eq!(get_delegates(&env, account_id).len(), MAX_DELEGATES);
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
        for _ in 0..MAX_DELEGATES {
            add_delegate(&env, account_id, &Address::generate(&env));
        }
        add_delegate(&env, account_id, &Address::generate(&env));
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
        add_delegate(&env, account_id, &Address::generate(&env));

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
        DebtPositionRaw {
            scaled_amount: 500,
        },
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
        set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                owner: Address::generate(&env),
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        set_debt_positions(&env, account_id, &sample_debt_map(&env));
        add_delegate(&env, account_id, &Address::generate(&env));
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
        set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                owner: Address::generate(&env),
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        set_supply_positions(&env, account_id, &sample_supply_map(&env));
        set_debt_positions(&env, account_id, &sample_debt_map(&env));
        add_delegate(&env, account_id, &Address::generate(&env));
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
