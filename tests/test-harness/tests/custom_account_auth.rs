//! A contract account authorizes the controller directly through the host.

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractimpl,
    crypto::Hash,
    testutils::MockAuthInvoke,
    token, vec, xdr, Env, IntoVal, Vec,
};
use test_harness::{hub_asset, LendingTest, HARNESS_SPOKE};

#[contract]
struct CustomAccount;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
enum AccountError {
    InvalidSignature = 1,
}

#[contractimpl]
impl CustomAccountInterface for CustomAccount {
    type Signature = bool;
    type Error = AccountError;

    fn __check_auth(
        _env: Env,
        _signature_payload: Hash<32>,
        signature: bool,
        _auth_contexts: Vec<Context>,
    ) -> Result<(), AccountError> {
        // This test exercises host dispatch, not a production signature policy.
        if signature {
            Ok(())
        } else {
            Err(AccountError::InvalidSignature)
        }
    }
}

#[test]
fn contract_account_authenticates_supply_and_rejects_invalid_authorizations() {
    let t = LendingTest::new().standard_two_asset_dust_disabled();
    let env = &t.env;
    let account = env.register(CustomAccount, ());
    let market = t.resolve_market("USDC");
    let amount = 1_000 * 10_000_000;
    market.token_admin.mint(&account, &(2 * amount));
    let token = token::Client::new(env, &market.asset);
    let pool_before = token.balance(&market.pool);
    let assets = vec![env, (hub_asset(market.asset.clone()), amount)];
    let ctrl = t.ctrl_client();
    assert!(!t.account_exists(1));

    let authorization = |signature, include_transfer| {
        let transfer = [MockAuthInvoke {
            contract: &market.asset,
            fn_name: "transfer",
            args: (account.clone(), market.pool.clone(), amount).into_val(env),
            sub_invokes: &[],
        }];
        xdr::SorobanAuthorizationEntry {
            credentials: xdr::SorobanCredentials::Address(xdr::SorobanAddressCredentials {
                address: (&account).into(),
                nonce: 1,
                signature_expiration_ledger: env.ledger().sequence() + 100,
                signature: xdr::ScVal::Bool(signature),
            }),
            root_invocation: (&MockAuthInvoke {
                contract: &t.controller,
                fn_name: "supply",
                args: (account.clone(), 0_u64, HARNESS_SPOKE, assets.clone()).into_val(env),
                sub_invokes: if include_transfer { &transfer } else { &[] },
            })
                .into(),
        }
    };

    // set_auths disables the harness mock: only the custom-account entry exists.
    for (signature, include_transfer) in [(false, true), (true, false)] {
        env.set_auths(&[authorization(signature, include_transfer)]);
        assert!(ctrl
            .try_supply(&account, &0, &HARNESS_SPOKE, &assets)
            .is_err());
        assert_eq!(token.balance(&account), 2 * amount);
        assert_eq!(token.balance(&market.pool), pool_before);
        assert!(!t.account_exists(1));
        assert!(!t.try_nft_owner_of(1));
    }

    env.set_auths(&[authorization(true, true)]);
    let id = ctrl.supply(&account, &0, &HARNESS_SPOKE, &assets);
    assert_eq!(id, 1, "rejected calls did not allocate a position");
    assert_eq!(t.nft_owner_of(id), account);
    assert_eq!(t.supply_balance_raw_for(id, "USDC"), amount);
    assert_eq!(token.balance(&account), amount);
    assert_eq!(token.balance(&market.pool), pool_before + amount);
}
