//! Registry addressing and partial-fill behaviour for the Aquarius LP legs.
//!
//! A burn reads its per-constituent floors out of the shared `amounts`
//! registry at a caller-declared offset, and a mint has to survive a pool that
//! takes less than it was offered. Both are places where an off-by-one in the
//! indexing, or an over-eager guard, silently changes which numbers the
//! protocol enforces.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    contract, contractimpl, contracttype, token, vec, Address, Bytes, Env, Map, Vec,
};

use crate::errors::Error;
use crate::program::encode::{self, RawOp};
use crate::types::StrategyPayload;
use crate::vault::Vault;
use crate::venues::aquarius::{add_liquidity, MintLiquidity};
use crate::{Router, RouterClient};

use super::super::support::{aquarius_lp_mock, aquarius_mock, new_asset};

/// Opcode bytes, mirroring `Opcode::from_u8`.
const OP_AQUARIUS_SWAP: u8 = 1;
const OP_BURN: u8 = 5;

/// Serialize a hand-built payload: these tests need registry shapes the
/// path-oriented builder cannot express.
fn payload(env: &Env, assets: &[Address], amounts: &[i128], ops: Bytes) -> Bytes {
    let mut asset_registry = Vec::new(env);
    for asset in assets {
        asset_registry.push_back(asset.clone());
    }
    let mut amount_registry = Vec::new(env);
    for amount in amounts {
        amount_registry.push_back(*amount);
    }
    StrategyPayload {
        amounts: amount_registry,
        assets: asset_registry,
        ops,
    }
    .to_xdr(env)
}

/// A two-constituent Aquarius pool seeded with `reserve` of each token.
fn lp_pool<'a>(
    env: &'a Env,
    admin: &Address,
    reserve: i128,
) -> (
    Address,
    Address,
    (Address, token::StellarAssetClient<'a>),
    (Address, token::StellarAssetClient<'a>),
) {
    let a = new_asset(env, admin);
    let b = new_asset(env, admin);
    let pool = env.register(aquarius_lp_mock::AqLpPool, ());
    let share = env
        .register_stellar_asset_contract_v2(pool.clone())
        .address();
    aquarius_lp_mock::AqLpPoolClient::new(env, &pool).init(&a.0, &b.0, &share);

    let seeder = Address::generate(env);
    a.1.mint(&seeder, &reserve);
    b.1.mint(&seeder, &reserve);
    aquarius_lp_mock::AqLpPoolClient::new(env, &pool).deposit(
        &seeder,
        &vec![env, reserve as u128, reserve as u128],
        &0u128,
    );

    (pool, share, a, b)
}

/// Burn the sender's whole LP position, then fold the second constituent back
/// into the first. `min_start` is the declared offset of the floor run.
fn burn_then_fold(env: &Env, min_start: u8) -> (Address, Address, Address, Bytes) {
    let admin = Address::generate(env);
    let sender = Address::generate(env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(env, &admin, 1_000_000);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);
    aquarius_lp_mock::AqLpPoolClient::new(env, &pool).deposit(
        &sender,
        &vec![env, 1_000u128, 1_000u128],
        &0u128,
    );
    assert_eq!(token::Client::new(env, &share).balance(&sender), 1_000);

    // A 1:1 book that folds the second constituent back into token_a.
    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(env, &swap_pool).init(&token_b, &token_a);
    sac_a.mint(&swap_pool, &1_000_000);

    let assets = alloc::vec![
        share.clone(),
        token_a.clone(),
        token_b.clone(),
        pool,
        swap_pool,
    ];
    // Slot 0 is the strategy-wide minimum and is *not* part of the floor run.
    // It is set past anything a constituent could deliver, so a floor read from
    // the wrong offset fails loudly instead of passing by accident.
    let amounts = alloc::vec![1_000_000_000i128, 0, 0, 1];
    let ops = encode::program(
        env,
        0,
        1,
        3,
        0,
        &alloc::vec![
            RawOp {
                opcode: OP_BURN,
                mode: encode::ALL,
                idx_a: 3,
                idx_b: 0,
                idx_c: min_start,
            },
            RawOp {
                opcode: OP_AQUARIUS_SWAP,
                mode: encode::ALL,
                idx_a: 4,
                idx_b: 2,
                idx_c: 1,
            },
        ],
        &[],
    );

    (sender, share, token_a, payload(env, &assets, &amounts, ops))
}

#[test]
fn burn_reads_its_floor_run_from_the_declared_offset() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));

    let (sender, _share, token_a, xdr) = burn_then_fold(&env, 1);

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    assert_eq!(out, 2_000, "both constituents must arrive as token_a");
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 2_000);
}

/// The floor run must fit inside the registry for the pool's whole arity: a run
/// that starts in range but ends past the end would otherwise enforce floors
/// read from nowhere.
#[test]
fn burn_rejects_a_floor_run_that_overruns_the_amount_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));

    let (sender, _share, _token_a, xdr) = burn_then_fold(&env, 3);

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1_000, &xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::MinAmountsNotMet.into());
}

// --- mint against a pool that does not take everything on offer -------------

/// Runs `add_liquidity` over a caller-supplied vault so the deposit can be
/// observed on its own, without a whole strategy around it.
#[contract]
pub struct MintProbe;

#[contractimpl]
impl MintProbe {
    /// `[shares, vault balance of tokens[0], vault balance of tokens[1], ...]`.
    pub fn mint(
        env: Env,
        pool: Address,
        lp_token: Address,
        tokens: Vec<Address>,
        held: Vec<i128>,
        min_shares: i128,
    ) -> Vec<i128> {
        let router = env.current_contract_address();
        let mut vault = Vault::new(&env);
        for i in 0..tokens.len() {
            vault.deposit(&tokens.get_unchecked(i), held.get_unchecked(i));
        }

        let mut cache: Map<Address, Vec<Address>> = Map::new(&env);
        let shares = add_liquidity(
            &env,
            &router,
            &mut vault,
            MintLiquidity {
                pool: &pool,
                lp_token: &lp_token,
                min_shares,
            },
            &mut cache,
        );

        let mut report = Vec::new(&env);
        report.push_back(shares);
        for i in 0..tokens.len() {
            report.push_back(vault.balance_of(&tokens.get_unchecked(i)));
        }
        report
    }
}

#[contracttype]
enum LpKey {
    TokenA,
    TokenB,
    Share,
}

fn lp_tokens(env: &Env) -> Vec<Address> {
    let token_a: Address = env.storage().instance().get(&LpKey::TokenA).unwrap();
    let token_b: Address = env.storage().instance().get(&LpKey::TokenB).unwrap();
    vec![env, token_a, token_b]
}

fn lp_share(env: &Env) -> Address {
    env.storage().instance().get(&LpKey::Share).unwrap()
}

fn lp_init(env: &Env, token_a: Address, token_b: Address, share: Address) {
    env.storage().instance().set(&LpKey::TokenA, &token_a);
    env.storage().instance().set(&LpKey::TokenB, &token_b);
    env.storage().instance().set(&LpKey::Share, &share);
}

/// A pool that pulls the first constituent and declines the second outright.
#[contract]
pub struct PartialLpPool;

#[contractimpl]
impl PartialLpPool {
    pub fn init(env: Env, token_a: Address, token_b: Address, share: Address) {
        lp_init(&env, token_a, token_b, share);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        lp_tokens(&env)
    }

    pub fn share_id(env: Env) -> Address {
        lp_share(&env)
    }

    pub fn deposit(
        env: Env,
        user: Address,
        desired_amounts: Vec<u128>,
        min_shares: u128,
    ) -> (Vec<u128>, u128) {
        let pool = env.current_contract_address();
        let tokens = lp_tokens(&env);
        let taken = desired_amounts.get_unchecked(0) as i128;
        if taken > 0 {
            token::Client::new(&env, &tokens.get_unchecked(0)).transfer(&user, &pool, &taken);
        }
        assert!(taken as u128 >= min_shares, "min_shares not met");
        token::StellarAssetClient::new(&env, &lp_share(&env)).mint(&user, &taken);
        (vec![&env, taken as u128, 0u128], taken as u128)
    }
}

/// A pool that asks for every constituent, including the ones it was offered
/// nothing of.
#[contract]
pub struct EagerLpPool;

#[contractimpl]
impl EagerLpPool {
    pub fn init(env: Env, token_a: Address, token_b: Address, share: Address) {
        lp_init(&env, token_a, token_b, share);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        lp_tokens(&env)
    }

    pub fn share_id(env: Env) -> Address {
        lp_share(&env)
    }

    pub fn deposit(
        env: Env,
        user: Address,
        desired_amounts: Vec<u128>,
        min_shares: u128,
    ) -> (Vec<u128>, u128) {
        let pool = env.current_contract_address();
        let tokens = lp_tokens(&env);
        let mut used = Vec::new(&env);
        for i in 0..tokens.len() {
            let amount = desired_amounts.get_unchecked(i) as i128;
            token::Client::new(&env, &tokens.get_unchecked(i)).transfer(&user, &pool, &amount);
            used.push_back(amount as u128);
        }
        let shares = desired_amounts.get_unchecked(0) as i128;
        assert!(shares as u128 >= min_shares, "min_shares not met");
        token::StellarAssetClient::new(&env, &lp_share(&env)).mint(&user, &shares);
        (used, shares as u128)
    }
}

/// A pool is free to take less than it was offered. Spending nothing of a
/// constituent is a partial fill, not a failure: the shares that did arrive
/// must be credited and the untouched balance must stay spendable.
#[test]
fn mint_tolerates_a_constituent_the_pool_declines_to_pull() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    let pool = env.register(PartialLpPool, ());
    let share = env
        .register_stellar_asset_contract_v2(pool.clone())
        .address();
    PartialLpPoolClient::new(&env, &pool).init(&token_a, &token_b, &share);

    let probe = env.register(MintProbe, ());
    sac_a.mint(&probe, &1_000);
    sac_b.mint(&probe, &400);

    let report = MintProbeClient::new(&env, &probe).mint(
        &pool,
        &share,
        &vec![&env, token_a, token_b],
        &vec![&env, 1_000i128, 400i128],
        &1_000i128,
    );

    assert_eq!(report.get_unchecked(0), 1_000, "shares must be credited");
    assert_eq!(
        report.get_unchecked(1),
        0,
        "the constituent the pool took must leave the vault"
    );
    assert_eq!(
        report.get_unchecked(2),
        400,
        "the constituent the pool declined must stay spendable"
    );
}

/// The router hands the pool invoker auth for exactly the legs it funds, so a
/// pool that reaches for a constituent the vault contributed nothing of has no
/// authorization to spend under.
#[test]
fn mint_authorizes_only_the_constituents_the_vault_funds() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _sac_b) = new_asset(&env, &admin);

    let pool = env.register(EagerLpPool, ());
    let share = env
        .register_stellar_asset_contract_v2(pool.clone())
        .address();
    EagerLpPoolClient::new(&env, &pool).init(&token_a, &token_b, &share);

    let probe = env.register(MintProbe, ());
    sac_a.mint(&probe, &1_000);

    // Nothing is mocked past this point: only the router's own invoker auth
    // stands behind the pool's transfers.
    env.set_auths(&[]);

    let refused = MintProbeClient::new(&env, &probe).try_mint(
        &pool,
        &share,
        &vec![&env, token_a, token_b],
        &vec![&env, 1_000i128, 0i128],
        &1_000i128,
    );
    assert!(
        refused.is_err(),
        "an unfunded constituent must carry no authorization"
    );
}

/// Counterpart to the test above: the same pool settles once every constituent
/// it reaches for is one the vault actually funded.
#[test]
fn mint_authorizes_every_constituent_the_vault_does_fund() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    let pool = env.register(EagerLpPool, ());
    let share = env
        .register_stellar_asset_contract_v2(pool.clone())
        .address();
    EagerLpPoolClient::new(&env, &pool).init(&token_a, &token_b, &share);

    let probe = env.register(MintProbe, ());
    sac_a.mint(&probe, &1_000);
    sac_b.mint(&probe, &250);

    env.set_auths(&[]);

    let report = MintProbeClient::new(&env, &probe).mint(
        &pool,
        &share,
        &vec![&env, token_a, token_b],
        &vec![&env, 1_000i128, 250i128],
        &1_000i128,
    );
    assert_eq!(report.get_unchecked(0), 1_000, "shares must be credited");
    assert_eq!(report.get_unchecked(1), 0);
    assert_eq!(report.get_unchecked(2), 0);
}
