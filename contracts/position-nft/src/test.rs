use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

use crate::{PositionNft, PositionNftClient};

fn setup(env: &Env) -> (Address, PositionNftClient<'_>) {
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
    (controller, PositionNftClient::new(env, &id))
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
