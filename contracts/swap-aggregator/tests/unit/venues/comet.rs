use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, Address, Env, IntoVal};

use super::super::support::{
    comet_mock, comet_zero_mock, new_asset, one_hop_path, sticky_allowance_token_mock, strategy_xdr,
};

#[test]
fn comet_single_hop_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(comet_mock::CometPool, ());

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        250,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &250, &swap_xdr);
    assert_eq!(out, 250);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 750);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 250);
    assert_eq!(
        token::Client::new(&env, &token_a).allowance(&router_addr, &pool),
        0
    );
}

#[test]
fn comet_rejects_output_without_input_spend() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(comet_mock::NoPullCometPool, ());

    sac_a.mint(&sender, &250);
    sac_b.mint(&pool, &250);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        250,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &250, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::InvalidAmount.into());
    assert_eq!(
        token::Client::new(&env, &token_a).allowance(&router_addr, &pool),
        0
    );
    assert_eq!(token::Client::new(&env, &token_a).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
}

#[test]
fn comet_zero_report_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(comet_zero_mock::ZeroOutComet, ());
    sac_a.mint(&sender, &250);
    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &250, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );
}

#[test]
fn comet_clears_unconsumed_allowance() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let token_a = env.register(sticky_allowance_token_mock::StickyAllowanceToken, ());
    let token_a_client =
        sticky_allowance_token_mock::StickyAllowanceTokenClient::new(&env, &token_a);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(comet_mock::CometPool, ());
    token_a_client.mint(&sender, &250);
    sac_b.mint(&pool, &250);

    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        250,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &250, &xdr);
    assert_eq!(out, 250);
    assert_eq!(
        token_a_client.allowance(&router_addr, &pool),
        0,
        "unconsumed comet approval must be cleared"
    );
}

#[test]
fn comet_approval_ledger_covers_current_sequence() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.sequence_number = 3_000_000_001);

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(comet_mock::CometPool, ());
    sac_a.mint(&sender, &250);
    sac_b.mint(&pool, &250);

    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        250,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &250, &xdr);
    assert_eq!(out, 250);
    assert_eq!(
        token::Client::new(&env, &token_a).allowance(&router_addr, &pool),
        0
    );
}

/// A Comet swap asks the user to authorize one tree: the `execute_strategy` root with the
/// inbound `transfer` as its only sub-invocation.
///
/// Only the sender's entry is mocked, so the recorded tree shows what the user must sign. It
/// fails if the router adds a venue approval or a second token movement to that tree.
///
/// It cannot observe `authorize_token_approve` or `authorize_comet_swap`: against a SAC token
/// the router's own `approve` needs no extra auth, and `transfer_from` requires the spender
/// (the pool). `.cargo/mutants.toml` lists their mutants as equivalent.
#[test]
fn comet_swap_relies_on_router_invoker_auth_not_mocked_auth() {
    let env = Env::default();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(comet_mock::CometPool, ());

    env.mock_all_auths();
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        250,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );

    // From here on only the sender's own tree is mocked.
    env.set_auths(&[]);
    env.mock_auths(&[MockAuth {
        address: &sender,
        invoke: &MockAuthInvoke {
            contract: &router_addr,
            fn_name: "execute_strategy",
            args: (sender.clone(), 250_i128, xdr.clone()).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &token_a,
                fn_name: "transfer",
                args: (sender.clone(), router_addr.clone(), 250_i128).into_val(&env),
                sub_invokes: &[],
            }],
        },
    }]);

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &250, &xdr);
    assert_eq!(out, 250);

    // Exactly one address is asked to authorize anything: the sender.
    let auths = env.auths();
    assert_eq!(auths.len(), 1, "only the sender may be asked to authorize");
    let (who, root) = &auths[0];
    assert_eq!(who, &sender);
    assert_eq!(
        root.sub_invocations.len(),
        1,
        "the signed tree must cover the inbound transfer and nothing further"
    );
    assert!(
        root.sub_invocations[0].sub_invocations.is_empty(),
        "the inbound transfer must be a leaf -- no venue call rides on the user's signature"
    );
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 750);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 250);
    assert_eq!(
        token::Client::new(&env, &token_a).allowance(&router_addr, &pool),
        0,
        "the swap approval must be cleared once the hop settles"
    );
}
