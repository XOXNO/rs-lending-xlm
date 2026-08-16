use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_USER,
};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, BytesN, Env, String};

use crate::{PositionNft, PositionNftClient};

fn setup(env: &Env) -> (Address, PositionNftClient<'_>) {
    let (_id, controller, client) = setup_with_id(env);
    (controller, client)
}

fn setup_with_id(env: &Env) -> (Address, Address, PositionNftClient<'_>) {
    let controller = Address::generate(env);
    let id = env.register(
        PositionNft,
        (
            controller.clone(),
            String::from_str(env, "https://xoxno.com/nft/"),
            String::from_str(env, "XOXNO Lending Position"),
            String::from_str(env, "XLP"),
        ),
    );
    (id.clone(), controller, PositionNftClient::new(env, &id))
}

#[test]
fn first_mint_is_token_id_one() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);

    let user = Address::generate(&env);
    assert_eq!(client.mint(&user), 1u32);
    assert_eq!(client.owner_of(&1u32), user);
    assert_eq!(client.balance(&user), 1u32);
    assert_eq!(client.total_supply(), 1u32);
}

#[test]
fn token_id_zero_is_never_minted() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);

    let user = Address::generate(&env);
    let first = client.mint(&user);
    let second = client.mint(&user);
    assert_eq!((first, second), (1u32, 2u32));
    // Id 0 was consumed by the constructor and never assigned.
    assert!(client.try_owner_of(&0u32).is_err());
}

#[test]
fn metadata_is_set() {
    let env = Env::default();
    let (_controller, client) = setup(&env);
    assert_eq!(client.symbol(), String::from_str(&env, "XLP"));
}

#[test]
fn mint_requires_controller_auth() {
    let env = Env::default();
    // No auth mocking: the controller's require_auth must fail.
    let (_controller, client) = setup(&env);
    let user = Address::generate(&env);
    assert!(client.try_mint(&user).is_err());
}

#[test]
fn burn_requires_controller_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let user = Address::generate(&env);
    let token_id = client.mint(&user);

    // Fresh env auth state: stop mocking, so the controller's require_auth fails.
    env.set_auths(&[]);
    assert!(client.try_burn(&token_id).is_err());
}

#[test]
fn burn_does_not_need_owner_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let user = Address::generate(&env);
    let token_id = client.mint(&user);

    // mock_all_auths satisfies the controller; the owner never signs anything —
    // there is no `from` parameter at all.
    client.burn(&token_id);
    assert!(client.try_owner_of(&token_id).is_err());
    assert_eq!(client.balance(&user), 0u32);
    assert_eq!(client.total_supply(), 0u32);
}

#[test]
fn burned_token_cannot_be_transferred() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let seller = Address::generate(&env);
    let buyer = Address::generate(&env);
    let token_id = client.mint(&seller);
    client.burn(&token_id);
    assert!(client.try_transfer(&seller, &buyer, &token_id).is_err());
}

#[test]
fn transfer_moves_ownership_and_enumeration() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let token_id = client.mint(&alice);

    client.transfer(&alice, &bob, &token_id);
    assert_eq!(client.owner_of(&token_id), bob);
    assert_eq!(client.balance(&alice), 0u32);
    assert_eq!(client.balance(&bob), 1u32);
    assert_eq!(client.get_owner_token_id(&bob, &0u32), token_id);
}

#[test]
fn approved_operator_can_transfer_and_approval_is_cleared() {
    // Pins stock OZ approval semantics (approve / transfer_from), since
    // `NonFungibleToken`/`NonFungibleEnumerable` are otherwise untested here.
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let owner = Address::generate(&env);
    let operator = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_id = client.mint(&owner);

    let live_until = env.ledger().sequence() + 1_000;
    client.approve(&owner, &operator, &token_id, &live_until);
    assert_eq!(client.get_approved(&token_id), Some(operator.clone()));

    // The approved operator, not the owner, authorizes the transfer.
    client.transfer_from(&operator, &owner, &recipient, &token_id);

    assert_eq!(client.owner_of(&token_id), recipient);
    assert_eq!(client.balance(&owner), 0u32);
    assert_eq!(client.balance(&recipient), 1u32);
    // Approval does not carry over to the new owner.
    assert_eq!(client.get_approved(&token_id), None);
}

#[test]
fn unapproved_caller_cannot_transfer_from() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let owner = Address::generate(&env);
    let stranger = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_id = client.mint(&owner);

    assert!(client
        .try_transfer_from(&stranger, &owner, &recipient, &token_id)
        .is_err());
}

#[test]
fn mint_extends_instance_ttl() {
    use soroban_sdk::testutils::storage::Instance as _;

    let env = Env::default();
    env.mock_all_auths();
    let (id, _controller, client) = setup_with_id(&env);
    let user = Address::generate(&env);

    // Age the instance below the renewal threshold.
    env.ledger()
        .with_mut(|l| l.sequence_number += TTL_BUMP_INSTANCE);
    env.as_contract(&id, || {
        assert!(env.storage().instance().get_ttl() < TTL_THRESHOLD_INSTANCE);
    });

    client.mint(&user);

    env.as_contract(&id, || {
        assert_eq!(env.storage().instance().get_ttl(), TTL_BUMP_INSTANCE);
    });
}

#[test]
fn burn_extends_instance_ttl() {
    use soroban_sdk::testutils::storage::Instance as _;

    let env = Env::default();
    env.mock_all_auths();
    let (id, _controller, client) = setup_with_id(&env);
    let user = Address::generate(&env);
    let token_id = client.mint(&user);

    // Age the instance below the renewal threshold.
    env.ledger()
        .with_mut(|l| l.sequence_number += TTL_BUMP_INSTANCE);
    env.as_contract(&id, || {
        assert!(env.storage().instance().get_ttl() < TTL_THRESHOLD_INSTANCE);
    });

    client.burn(&token_id);

    env.as_contract(&id, || {
        assert_eq!(env.storage().instance().get_ttl(), TTL_BUMP_INSTANCE);
    });
}

#[test]
fn renew_extends_owner_entry_ttl_to_user_window() {
    use soroban_sdk::testutils::storage::Persistent as _;
    use stellar_tokens::non_fungible::NFTStorageKey;

    let env = Env::default();
    env.mock_all_auths();
    let (id, _controller, client) = setup_with_id(&env);
    let user = Address::generate(&env);
    let token_id = client.mint(&user);

    // Age the ledger past the OZ 30-day owner-entry bump so the entry is
    // measurably older than the user window renew() must restore.
    env.ledger()
        .with_mut(|l| l.sequence_number += TTL_THRESHOLD_USER / 2);
    env.as_contract(&id, || {
        assert!(
            env.storage()
                .persistent()
                .get_ttl(&NFTStorageKey::Owner(token_id))
                < TTL_BUMP_USER
        );
    });

    // Permissionless: no auth mocked beyond the mint above, caller is anyone.
    client.renew(&token_id);

    env.as_contract(&id, || {
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&NFTStorageKey::Owner(token_id)),
            TTL_BUMP_USER
        );
    });
}

#[test]
fn renew_nonexistent_token_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    assert!(client.try_renew(&7u32).is_err());
}

#[test]
fn upgrade_requires_controller_auth() {
    let env = Env::default();
    // No auth mocking: the controller's require_auth must fail before any
    // wasm validation happens.
    let (_controller, client) = setup(&env);
    let fake_hash = BytesN::from_array(&env, &[7u8; 32]);
    assert!(client.try_upgrade(&fake_hash).is_err());
}
