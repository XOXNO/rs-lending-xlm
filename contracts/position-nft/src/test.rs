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
    // Mirrors the Makefile deploy defaults (POSITION_NFT_URI/NAME/SYMBOL).
    let id = env.register(
        PositionNft,
        (
            controller.clone(),
            String::from_str(env, "https://api.xoxno.com/user/lending/image/"),
            String::from_str(env, "XOXNO Lending Position"),
            String::from_str(env, "XLEND"),
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
    // All three come from constructor metadata (storage is the sole source).
    assert_eq!(
        client.name(),
        String::from_str(&env, "XOXNO Lending Position")
    );
    assert_eq!(client.symbol(), String::from_str(&env, "XLEND"));
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
fn operator_for_all_can_move_every_token_and_grant_is_per_owner_and_revocable() {
    // F-9: `approve_for_all` is the protocol's broadest authority grant
    // (account_id == token_id, so moving a token moves the position).
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let owner_a = Address::generate(&env);
    let owner_b = Address::generate(&env);
    let operator = Address::generate(&env);
    let recipient = Address::generate(&env);

    // owner_a holds two positions; owner_b holds one.
    let a1 = client.mint(&owner_a);
    let a2 = client.mint(&owner_a);
    let b1 = client.mint(&owner_b);

    let live_until = env.ledger().sequence() + 1_000;
    client.approve_for_all(&owner_a, &operator, &live_until);
    assert!(client.is_approved_for_all(&owner_a, &operator));

    // Blanket approval moves EVERY owner_a token, with no per-token approval.
    client.transfer_from(&operator, &owner_a, &recipient, &a1);
    client.transfer_from(&operator, &owner_a, &recipient, &a2);
    assert_eq!(client.owner_of(&a1), recipient);
    assert_eq!(client.owner_of(&a2), recipient);

    // The grant is per-owner: it gives no authority over owner_b's token.
    assert!(!client.is_approved_for_all(&owner_b, &operator));
    assert!(client
        .try_transfer_from(&operator, &owner_b, &recipient, &b1)
        .is_err());

    // Revocation is immediate.
    client.approve_for_all(&owner_a, &operator, &0u32);
    assert!(!client.is_approved_for_all(&owner_a, &operator));
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

    // mint now sets the Owner entry to the full user window (F-7), so age the
    // ledger until its remaining TTL drops below renew's threshold, where renew
    // must restore it.
    env.ledger()
        .with_mut(|l| l.sequence_number += TTL_BUMP_USER - TTL_THRESHOLD_USER / 2);
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

#[test]
fn token_uri_composes_nonce_with_query_suffix() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    let user = Address::generate(&env);
    let token_id = client.mint(&user);
    assert_eq!(token_id, 1);
    assert_eq!(
        client.token_uri(&token_id),
        String::from_str(
            &env,
            "https://api.xoxno.com/user/lending/image/1?isStatic=true&chain=STELLAR"
        )
    );
    // Multi-digit ids keep digit order.
    for _ in 0..11 {
        client.mint(&user);
    }
    assert_eq!(
        client.token_uri(&12u32),
        String::from_str(
            &env,
            "https://api.xoxno.com/user/lending/image/12?isStatic=true&chain=STELLAR"
        )
    );
}

#[test]
fn token_uri_of_missing_token_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (_controller, client) = setup(&env);
    assert!(client.try_token_uri(&7u32).is_err());
}

/// `token_uri` builds its result in a fixed 256-byte buffer: base + up to 10
/// digits of a u32 id + the 28-byte suffix. The buffer is only safe because
/// OpenZeppelin's `set_metadata` caps `base_uri` at MAX_BASE_URI_LEN (200),
/// leaving 238 worst-case bytes. These two tests pin that upstream bound so a
/// longer suffix or a raised OZ cap fails here rather than corrupting memory.
#[test]
#[should_panic(expected = "Error(Contract, #211)")]
fn constructor_rejects_a_base_uri_over_the_oz_maximum() {
    let env = Env::default();
    env.mock_all_auths();
    let controller = Address::generate(&env);

    let oversized = [b'a'; 201];
    let oversized = core::str::from_utf8(&oversized).unwrap();

    env.register(
        PositionNft,
        (
            controller,
            String::from_str(&env, oversized),
            String::from_str(&env, "XOXNO Lending Position"),
            String::from_str(&env, "XLEND"),
        ),
    );
}

#[test]
fn longest_accepted_base_uri_still_renders_token_uri() {
    let env = Env::default();
    env.mock_all_auths();
    let controller = Address::generate(&env);

    let max_base = [b'a'; 200];
    let max_base = core::str::from_utf8(&max_base).unwrap();

    let id = env.register(
        PositionNft,
        (
            controller.clone(),
            String::from_str(&env, max_base),
            String::from_str(&env, "XOXNO Lending Position"),
            String::from_str(&env, "XLEND"),
        ),
    );
    let client = PositionNftClient::new(&env, &id);
    let user = Address::generate(&env);
    let token_id = client.mint(&user);

    // 200 base + 1 digit + 28 suffix = 229 bytes, inside the 256-byte buffer.
    assert_eq!(client.token_uri(&token_id).len(), 200 + 1 + 28);
}
