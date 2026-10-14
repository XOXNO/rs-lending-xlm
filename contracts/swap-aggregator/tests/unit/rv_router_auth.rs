//! RV break pass: the router authorization tree under enforcing auth.
//!
//! Everything below runs with `env.set_auths(&[])`: no account signature is
//! mocked, so the only authorization in play is invoker-contract auth, which is
//! exactly what a controller-driven swap relies on in production.

use common::token::authorize_transfer_as_current;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, contracttype, token, vec, Address, Bytes, Env, Vec};

use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};

use super::support::{new_asset, one_hop_path, strategy_xdr, Builder, PreSwap};

/// Controller stand-in: grants exactly one `token_in.transfer(self, router, amount_in)`
/// invoker entry and calls the real router, like `strategies::swap::swap_tokens`.
#[contract]
pub struct CtrlStandIn;

#[contractimpl]
impl CtrlStandIn {
    pub fn swap(env: Env, router: Address, token_in: Address, amount_in: i128, xdr: Bytes) -> i128 {
        let me = env.current_contract_address();
        authorize_transfer_as_current(&env, &token_in, &me, &router, amount_in);
        RouterClient::new(&env, &router).execute_strategy(&me, &amount_in, &xdr)
    }
}

#[contracttype]
enum GreedyKey {
    TokenA,
    TokenB,
    Victim,
    Mode,
}

/// Aquarius-shaped hop pool. `mode` 0 is honest; 1 re-pulls the sender's entry
/// into the router; 2 re-pulls it into the pool.
#[contract]
pub struct GreedyPool;

#[contractimpl]
impl GreedyPool {
    pub fn init(env: Env, token_a: Address, token_b: Address, victim: Address, mode: u32) {
        env.storage().instance().set(&GreedyKey::TokenA, &token_a);
        env.storage().instance().set(&GreedyKey::TokenB, &token_b);
        env.storage().instance().set(&GreedyKey::Victim, &victim);
        env.storage().instance().set(&GreedyKey::Mode, &mode);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let a: Address = env.storage().instance().get(&GreedyKey::TokenA).unwrap();
        let b: Address = env.storage().instance().get(&GreedyKey::TokenB).unwrap();
        vec![&env, a, b]
    }

    pub fn swap(
        env: Env,
        user: Address,
        in_idx: u32,
        _out_idx: u32,
        in_amount: u128,
        _out_min: u128,
    ) -> u128 {
        let tokens = Self::get_tokens(env.clone());
        let (token_in, token_out) = if in_idx == 0 {
            (tokens.get_unchecked(0), tokens.get_unchecked(1))
        } else {
            (tokens.get_unchecked(1), tokens.get_unchecked(0))
        };
        let pool = env.current_contract_address();
        let amount = in_amount as i128;
        token::Client::new(&env, &token_in).transfer(&user, &pool, &amount);
        let victim: Address = env.storage().instance().get(&GreedyKey::Victim).unwrap();
        let mode: u32 = env.storage().instance().get(&GreedyKey::Mode).unwrap();
        match mode {
            1 => token::Client::new(&env, &token_in).transfer(&victim, &user, &amount),
            2 => token::Client::new(&env, &token_in).transfer(&victim, &pool, &amount),
            _ => {}
        }
        token::Client::new(&env, &token_out).transfer(&pool, &user, &amount);
        in_amount
    }
}

#[contracttype]
enum LpKey {
    TokenA,
    TokenB,
    Share,
    Exact,
}

/// Aquarius-shaped LP pool whose `deposit` pulls the *computed* amounts (as the
/// real constant-product pool does), not "pull desired then refund".
#[contract]
pub struct ComputedPullLpPool;

#[contractimpl]
impl ComputedPullLpPool {
    pub fn init(env: Env, token_a: Address, token_b: Address, share: Address, exact: bool) {
        env.storage().instance().set(&LpKey::TokenA, &token_a);
        env.storage().instance().set(&LpKey::TokenB, &token_b);
        env.storage().instance().set(&LpKey::Share, &share);
        env.storage().instance().set(&LpKey::Exact, &exact);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let a: Address = env.storage().instance().get(&LpKey::TokenA).unwrap();
        let b: Address = env.storage().instance().get(&LpKey::TokenB).unwrap();
        vec![&env, a, b]
    }

    pub fn share_id(env: Env) -> Address {
        env.storage().instance().get(&LpKey::Share).unwrap()
    }

    pub fn deposit(
        env: Env,
        user: Address,
        desired: Vec<u128>,
        _min_shares: u128,
    ) -> (Vec<u128>, u128) {
        user.require_auth();
        let tokens = Self::get_tokens(env.clone());
        let pool = env.current_contract_address();
        let exact: bool = env.storage().instance().get(&LpKey::Exact).unwrap();
        let d0 = desired.get_unchecked(0) as i128;
        let d1 = desired.get_unchecked(1) as i128;
        // A real pool keeps the deposit proportional: it takes less of the side
        // that is in excess. Here side B is always the excess side.
        let used1 = if exact { d1 } else { d1 - 1 };
        token::Client::new(&env, &tokens.get_unchecked(0)).transfer(&user, &pool, &d0);
        token::Client::new(&env, &tokens.get_unchecked(1)).transfer(&user, &pool, &used1);
        let share: Address = env.storage().instance().get(&LpKey::Share).unwrap();
        let shares = d0 + used1;
        token::StellarAssetClient::new(&env, &share).mint(&user, &shares);
        (vec![&env, d0 as u128, used1 as u128], shares as u128)
    }
}

struct Scene {
    env: Env,
    router: Address,
    ctrl: Address,
    token_a: Address,
    token_b: Address,
    pool: Address,
}

impl Scene {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let router = env.register(Router, (Address::generate(&env),));
        let ctrl = env.register(CtrlStandIn, ());
        let admin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &admin);
        let (token_b, sac_b) = new_asset(&env, &admin);
        let pool = env.register(GreedyPool, ());
        sac_a.mint(&ctrl, &10_000);
        sac_b.mint(&pool, &10_000);
        Self {
            env,
            router,
            ctrl,
            token_a,
            token_b,
            pool,
        }
    }

    fn one_hop(&self) -> Bytes {
        strategy_xdr(
            &self.env,
            self.token_a.clone(),
            self.token_b.clone(),
            1,
            alloc::vec![one_hop_path(
                &self.env,
                SwapVenue::Aquarius,
                self.pool.clone(),
                self.token_a.clone(),
                self.token_b.clone(),
                1_000_000,
            )],
        )
    }

    fn bal(&self, token: &Address, who: &Address) -> i128 {
        token::Client::new(&self.env, token).balance(who)
    }
}

/// R-7: the sender's single invoker entry is exhausted by the router's own pull.
/// A hop pool that replays `token_in.transfer(sender, router, amount_in)` — or
/// aims it at itself — is refused by the host and the whole call rolls back.
#[test]
fn hop_pool_cannot_replay_the_senders_input_transfer_entry() {
    let s = Scene::new();
    let xdr = s.one_hop();

    for mode in [1u32, 2u32] {
        GreedyPoolClient::new(&s.env, &s.pool).init(&s.token_a, &s.token_b, &s.ctrl, &mode);
        s.env.set_auths(&[]);
        let refused =
            CtrlStandInClient::new(&s.env, &s.ctrl).try_swap(&s.router, &s.token_a, &1_000, &xdr);
        assert!(
            refused.is_err(),
            "mode {mode}: a second pull on the sender's entry must be unauthorized"
        );
        assert_eq!(
            s.bal(&s.token_a, &s.ctrl),
            10_000,
            "mode {mode}: nothing left the sender"
        );
        assert_eq!(
            s.bal(&s.token_a, &s.router),
            0,
            "mode {mode}: router holds no stray input"
        );
        assert_eq!(
            s.bal(&s.token_a, &s.pool),
            0,
            "mode {mode}: pool got nothing"
        );
    }

    // Control: the honest pool settles with only invoker auth.
    GreedyPoolClient::new(&s.env, &s.pool).init(&s.token_a, &s.token_b, &s.ctrl, &0);
    s.env.set_auths(&[]);
    let out = CtrlStandInClient::new(&s.env, &s.ctrl).swap(&s.router, &s.token_a, &1_000, &xdr);
    assert_eq!(out, 1_000);
    assert_eq!(s.bal(&s.token_a, &s.ctrl), 9_000);
    assert_eq!(s.bal(&s.token_b, &s.ctrl), 1_000);
    assert_eq!(s.bal(&s.token_a, &s.router), 0);
    assert_eq!(s.bal(&s.token_b, &s.router), 0);
}

/// R-8 (Aquarius mint): the router hands the pool an invoker entry for exactly
/// the vault balance of each constituent. A pool that pulls the *computed*
/// proportional amount (one unit less on the excess side) does not match that
/// entry, so the deposit is refused under enforcing auth even though the mocks
/// (pull-desired-then-refund) make it pass. Pulling exactly what was offered
/// settles.
#[test]
fn mint_against_a_pool_that_pulls_less_than_offered_is_refused_under_enforcing_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let router = env.register(Router, (Address::generate(&env),));
    let ctrl = env.register(CtrlStandIn, ());
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let hop = env.register(GreedyPool, ());
    GreedyPoolClient::new(&env, &hop).init(&token_a, &token_b, &ctrl, &0);
    let lp = env.register(ComputedPullLpPool, ());
    let share = env.register_stellar_asset_contract_v2(lp.clone()).address();
    sac_a.mint(&ctrl, &10_000);
    sac_b.mint(&hop, &10_000);

    // Half of A is swapped into B through the hop, then everything is minted.
    let xdr = Builder::new(&env, token_a.clone(), share.clone(), 1, 0)
        .paths(alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            hop.clone(),
            token_a.clone(),
            token_b.clone(),
            500_000,
        )])
        .mint(lp.clone(), share.clone(), 1, PreSwap::None, true)
        .build();

    for exact in [false, true] {
        ComputedPullLpPoolClient::new(&env, &lp).init(&token_a, &token_b, &share, &exact);
        env.set_auths(&[]);
        let result = CtrlStandInClient::new(&env, &ctrl).try_swap(&router, &token_a, &1_000, &xdr);
        if exact {
            assert_eq!(
                result.unwrap().unwrap(),
                1_000,
                "exact pull mints 500 + 500 shares"
            );
            assert_eq!(token::Client::new(&env, &share).balance(&ctrl), 1_000);
        } else {
            assert!(
                result.is_err(),
                "a pull of one unit less than the offered constituent is unauthorized"
            );
            assert_eq!(token::Client::new(&env, &token_a).balance(&ctrl), 10_000);
            assert_eq!(token::Client::new(&env, &share).balance(&ctrl), 0);
        }
    }
}

/// R-B fix (router_adapter): the same-token check compares the registry
/// *addresses*, not the indices. A registry that lists the input token twice can
/// no longer declare the input token as the strategy output: `Program::decode`
/// refuses the header with `SameToken` before the input is pulled.
///
/// Before the fix this route settled: the router paid the unrouted input (990)
/// back to the sender as the "output" and swept the routed hop's output (10 B)
/// into the admin residual bucket. Now nothing moves and nothing is booked.
#[test]
fn duplicate_registry_entries_cannot_declare_the_input_token_as_the_output() {
    use crate::program::encode::{self, RawOp};
    use crate::types::StrategyPayload;
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    let router = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let hop = env.register(GreedyPool, ());
    GreedyPoolClient::new(&env, &hop).init(&token_a, &token_b, &sender, &0);
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&hop, &1_000);
    sac_a.mint(&router, &5_000); // unrelated router balance (stands in for fee reserve)

    // assets: [A, B, hop, A]; amounts: [min_out = 1, fixed = 10]
    let assets = vec![
        &env,
        token_a.clone(),
        token_b.clone(),
        hop.clone(),
        token_a.clone(),
    ];
    let amounts = vec![&env, 1_i128, 10_i128];
    let ops = encode::program(
        &env,
        0,
        3,
        0,
        0,
        &[RawOp {
            opcode: 1,
            mode: encode::fixed(1),
            idx_a: 2,
            idx_b: 0,
            idx_c: 1,
        }],
        &[],
    );
    let xdr = StrategyPayload {
        amounts,
        assets,
        ops,
    }
    .to_xdr(&env);

    let refused = RouterClient::new(&env, &router).try_execute_strategy(&sender, &1_000, &xdr);
    assert_eq!(
        refused.unwrap_err().unwrap(),
        Error::SameToken.into(),
        "an input token declared as the output through a duplicate entry is refused"
    );
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 1_000);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
    assert_eq!(
        token::Client::new(&env, &token_a).balance(&router),
        5_000,
        "the router's pre-existing input-token balance is untouched"
    );
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&hop),
        1_000,
        "the hop pool was never reached"
    );
    assert_eq!(
        RouterClient::new(&env, &router).admin_fee_balance(&token_b),
        0,
        "no residual is booked"
    );
}
