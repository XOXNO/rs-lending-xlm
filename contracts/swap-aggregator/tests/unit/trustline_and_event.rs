//! Receiver trustline opening (CAP-73 `trust`) and the `SwapEvent` payload.

use alloc::rc::Rc;

use crate::events::SwapEvent;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::{
    Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, Ledger,
};
use soroban_sdk::token::{self, StellarAssetClient};
use soroban_sdk::xdr::{
    AccountEntry, AccountEntryExt, AccountId, AlphaNum4, AssetCode4, LedgerEntry, LedgerEntryData,
    LedgerEntryExt, LedgerKey, LedgerKeyAccount, LedgerKeyTrustLine, PublicKey, ScAddress,
    SequenceNumber, Thresholds, TrustLineAsset, Uint256,
};
use soroban_sdk::Event as _;
use soroban_sdk::{Address, Env, Symbol, TryIntoVal};

use super::support::{aquarius_mock, one_hop_path, strategy_xdr_with_referral};

/// Base reserve in stroops (0.5 XLM), as on mainnet.
const BASE_RESERVE: u32 = 5_000_000;
/// 100 XLM, enough for any number of test trustlines.
const FUNDED: i64 = 1_000_000_000;

/// A test Stellar Asset Contract with its issuer account id.
struct Asset<'a> {
    address: Address,
    admin: StellarAssetClient<'a>,
    issuer: AccountId,
}

fn new_sac<'a>(env: &'a Env) -> Asset<'a> {
    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let ScAddress::Account(issuer) = ScAddress::from(&sac.issuer().address()) else {
        panic!("SAC issuer must be an account");
    };
    let address = sac.address();
    Asset {
        admin: StellarAssetClient::new(env, &address),
        address,
        issuer,
    }
}

fn account_id(seed: u8) -> AccountId {
    AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([seed; 32])))
}

/// Writes a classic account entry with `balance` stroops and returns its `G...` address.
fn classic_account(env: &Env, seed: u8, balance: i64) -> Address {
    let id = account_id(seed);
    set_account(env, &id, balance, 0);
    ScAddress::Account(id).try_into_val(env).unwrap()
}

fn set_account(env: &Env, id: &AccountId, balance: i64, num_sub_entries: u32) {
    let key = LedgerKey::Account(LedgerKeyAccount {
        account_id: id.clone(),
    });
    let entry = LedgerEntry {
        last_modified_ledger_seq: 0,
        ext: LedgerEntryExt::V0,
        data: LedgerEntryData::Account(AccountEntry {
            account_id: id.clone(),
            balance,
            flags: 0,
            home_domain: Default::default(),
            inflation_dest: None,
            num_sub_entries,
            seq_num: SequenceNumber(0),
            thresholds: Thresholds([1; 4]),
            signers: Default::default(),
            ext: AccountEntryExt::V0,
        }),
    };
    env.host()
        .add_ledger_entry(&Rc::new(key), &Rc::new(entry), None)
        .unwrap();
}

fn has_trustline(env: &Env, holder: &AccountId, asset: &Asset) -> bool {
    let key = LedgerKey::Trustline(LedgerKeyTrustLine {
        account_id: holder.clone(),
        asset: TrustLineAsset::CreditAlphanum4(AlphaNum4 {
            // `register_stellar_asset_contract_v2` always uses code "aaa".
            asset_code: AssetCode4(*b"aaa\0"),
            issuer: asset.issuer.clone(),
        }),
    });
    env.host()
        .get_ledger_entry(&Rc::new(key))
        .unwrap()
        .is_some()
}

/// Returns whether any recorded authorization tree contains a call to `function`.
fn authorized_call(env: &Env, function: &str) -> bool {
    fn walk(node: &AuthorizedInvocation, target: &Symbol) -> bool {
        let hit = matches!(&node.function, AuthorizedFunction::Contract((_, f, _)) if f == target);
        hit || node.sub_invocations.iter().any(|sub| walk(sub, target))
    }
    let target = Symbol::new(env, function);
    env.auths().iter().any(|(_, root)| walk(root, &target))
}

/// Router, A→B Aquarius pool holding 500 B, and a sender holding 500 A.
struct Swap<'a> {
    env: &'a Env,
    router: RouterClient<'a>,
    a: Asset<'a>,
    b: Asset<'a>,
    pool: Address,
}

fn setup<'a>(env: &'a Env) -> Swap<'a> {
    env.mock_all_auths();
    env.ledger().set_base_reserve(BASE_RESERVE);
    let router_addr = env.register(Router, (Address::generate(env),));
    let a = new_sac(env);
    let b = new_sac(env);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(env, &pool).init(&a.address, &b.address);
    b.admin.mint(&pool, &500);
    Swap {
        env,
        router: RouterClient::new(env, &router_addr),
        a,
        b,
        pool,
    }
}

impl Swap<'_> {
    /// Opens `sender`'s input trustline and funds it with 500 A.
    fn fund_classic_sender(&self, sender: &Address) {
        self.a.admin.trust(sender);
        self.a.admin.mint(sender, &500);
    }

    /// One-hop A→B payload with a 400 minimum; `referral_id` 0 means none.
    fn payload(&self, referral_id: u64) -> soroban_sdk::Bytes {
        strategy_xdr_with_referral(
            self.env,
            self.a.address.clone(),
            self.b.address.clone(),
            400,
            alloc::vec![one_hop_path(
                self.env,
                SwapVenue::Aquarius,
                self.pool.clone(),
                self.a.address.clone(),
                self.b.address.clone(),
                1_000_000,
            )],
            referral_id,
        )
    }

    /// Asserts the router published the `SwapEvent` for a 500-unit A→B swap.
    fn assert_swap_event(
        &self,
        sender: &Address,
        amount_out: i128,
        referral_id: u64,
        fee_token: &Address,
        (static_fee, referral_fee): (i128, i128),
    ) {
        let expected = SwapEvent {
            sender: sender.clone(),
            token_in: self.a.address.clone(),
            token_out: self.b.address.clone(),
            amount_in: 500,
            amount_out,
            referral_id,
            fee_token: fee_token.clone(),
            static_fee,
            referral_fee,
        }
        .to_xdr(self.env, &self.router.address);
        assert!(
            self.env.events().all().events().contains(&expected),
            "missing {expected:?}"
        );
    }
}

#[test]
fn swap_opens_missing_output_trustline_for_classic_sender() {
    let env = Env::default();
    let s = setup(&env);
    let sender = classic_account(&env, 7, FUNDED);
    s.fund_classic_sender(&sender);
    assert!(!has_trustline(&env, &account_id(7), &s.b));

    let out = s.router.execute_strategy(&sender, &500, &s.payload(0));

    assert_eq!(out, 500);
    assert!(authorized_call(&env, "trust"), "sender must sign trust");
    assert!(has_trustline(&env, &account_id(7), &s.b));
    assert_eq!(token::Client::new(&env, &s.b.address).balance(&sender), 500);
}

#[test]
fn swap_with_existing_output_trustline_needs_no_trust_auth() {
    let env = Env::default();
    let s = setup(&env);
    let sender = classic_account(&env, 8, FUNDED);
    s.fund_classic_sender(&sender);
    s.b.admin.trust(&sender);

    let out = s.router.execute_strategy(&sender, &500, &s.payload(0));

    assert_eq!(out, 500);
    // `env.auths()` holds only the last invocation: the swap itself.
    assert!(!authorized_call(&env, "trust"));
    assert_eq!(token::Client::new(&env, &s.b.address).balance(&sender), 500);
}

#[test]
fn swap_reverts_when_sender_cannot_fund_trustline_reserve() {
    let env = Env::default();
    let s = setup(&env);
    let sender = classic_account(&env, 9, FUNDED);
    s.fund_classic_sender(&sender);
    // Reserve for one trustline only: (2 + 1) * base reserve.
    set_account(&env, &account_id(9), 3 * BASE_RESERVE as i64, 1);

    let result = s.router.try_execute_strategy(&sender, &500, &s.payload(0));

    assert!(result.is_err(), "payout without a trustline must revert");
    assert!(!has_trustline(&env, &account_id(9), &s.b));
    assert_eq!(token::Client::new(&env, &s.a.address).balance(&sender), 500);
}

#[test]
fn swap_to_contract_sender_needs_no_trust_auth() {
    let env = Env::default();
    let s = setup(&env);
    let sender = Address::generate(&env);
    s.a.admin.mint(&sender, &500);

    // The SAC `trust` call is a no-op for a contract address.
    let out = s.router.execute_strategy(&sender, &500, &s.payload(0));

    assert_eq!(out, 500);
    assert!(!authorized_call(&env, "trust"));
}

#[test]
fn swap_to_output_issuer_ignores_failed_trust() {
    let env = Env::default();
    let s = setup(&env);
    set_account(&env, &s.b.issuer, FUNDED, 0);
    let issuer: Address = ScAddress::Account(s.b.issuer.clone())
        .try_into_val(&env)
        .unwrap();
    s.a.admin.trust(&issuer);
    s.a.admin.mint(&issuer, &500);

    // `trust` errors for the issuer; the payout to the issuer still succeeds.
    let out = s.router.execute_strategy(&issuer, &500, &s.payload(0));

    assert_eq!(out, 500);
}

#[test]
fn swap_publishes_event_with_amounts_and_fees() {
    let env = Env::default();
    let s = setup(&env);
    let sender = Address::generate(&env);
    s.a.admin.mint(&sender, &500);
    s.router.set_static_fee(&100);
    let referral = s.router.add_referral(&Address::generate(&env), &50);

    // Fee on input: static 1% = 5, referral 0.5% = 2.
    let out = s
        .router
        .execute_strategy(&sender, &500, &s.payload(referral));

    assert_eq!(out, 493);
    s.assert_swap_event(&sender, 493, referral, &s.a.address, (5, 2));
}

#[test]
fn swap_event_reports_zero_fees_without_referral() {
    let env = Env::default();
    let s = setup(&env);
    let sender = Address::generate(&env);
    s.a.admin.mint(&sender, &500);

    s.router.execute_strategy(&sender, &500, &s.payload(0));

    s.assert_swap_event(&sender, 500, 0, &s.b.address, (0, 0));
}

#[test]
fn swap_to_custom_token_never_calls_its_trust() {
    use super::support::trust_spy_token_mock::{TrustSpyToken, TrustSpyTokenClient};
    let env = Env::default();
    let s = setup(&env);
    // Output is a custom token whose `trust` records every call.
    let custom = env.register(TrustSpyToken, ());
    let custom_client = TrustSpyTokenClient::new(&env, &custom);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&s.a.address, &custom);
    custom_client.mint(&pool, &500);
    let sender = classic_account(&env, 11, FUNDED);
    s.fund_classic_sender(&sender);
    let payload = strategy_xdr_with_referral(
        &env,
        s.a.address.clone(),
        custom.clone(),
        400,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            s.a.address.clone(),
            custom.clone(),
            1_000_000,
        )],
        0,
    );

    let out = s.router.execute_strategy(&sender, &500, &payload);

    assert_eq!(out, 500);
    assert_eq!(custom_client.balance(&sender), 500);
    assert_eq!(custom_client.trust_calls(), 0, "custom token code ran");
    assert!(!authorized_call(&env, "trust"));
}
