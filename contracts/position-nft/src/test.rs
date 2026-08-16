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
