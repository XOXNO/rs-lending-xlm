use common::types::HubAssetKey;
use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, Address, Env, Map, Vec,
    I256,
};

use crate::helpers::{HARNESS_HUB, HARNESS_SPOKE};

#[contractclient(name = "BlendHookControllerClient")]
pub trait BlendHookController {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;
}

const REQ_WITHDRAW: u32 = 1;
const REQ_WITHDRAW_COLLATERAL: u32 = 3;
const REQ_REPAY: u32 = 5;

/// Blend's b_rate scale (`SCALAR_12`); a reserve with no rate set reads as 1.0.
const B_RATE_SCALAR: i128 = 1_000_000_000_000;

pub const KIND_COLLATERAL: u32 = 0;
pub const KIND_SUPPLY: u32 = 1;
pub const KIND_LIABILITY: u32 = 2;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MockBlendError {
    HealthCheckFailed = 1,
}

#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct BlendPositions {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

#[contracttype]
#[derive(Clone)]
enum Key {
    Collateral(Address, Address),
    Supply(Address, Address),
    Liability(Address, Address),

    LiabAssets(Address),
    BRate(Address),
    Hook,
}

#[contract]
pub struct MockBlend;

#[contractimpl]
impl MockBlend {
    pub fn __constructor(_env: Env) {}

    /// Makes `submit` call the controller's `supply` with 1 unit before it processes requests.
    /// Tests use it to prove that `guarded_submit` holds the flash-loan guard.
    pub fn set_hook(env: Env, controller: Address) {
        env.storage().instance().set(&Key::Hook, &controller);
    }

    pub fn seed(env: Env, user: Address, asset: Address, kind: u32, amount: i128) {
        env.storage()
            .persistent()
            .set(&key(kind, &user, &asset), &amount);
        if kind == KIND_LIABILITY {
            track_liability_asset(&env, &user, &asset);
        }
    }

    pub fn position(env: Env, user: Address, asset: Address, kind: u32) -> i128 {
        env.storage()
            .persistent()
            .get(&key(kind, &user, &asset))
            .unwrap_or(0)
    }

    /// Sets `asset`'s b_rate in `SCALAR_12`. Blend lowers a reserve's b_rate
    /// below 1.0 when it socializes bad debt.
    pub fn set_b_rate(env: Env, asset: Address, b_rate: i128) {
        env.storage().persistent().set(&Key::BRate(asset), &b_rate);
    }

    pub fn submit(
        env: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<BlendRequest>,
    ) -> Result<BlendPositions, MockBlendError> {
        spender.require_auth();
        if from != spender {
            from.require_auth();
        }

        if let Some(controller) = env.storage().instance().get::<Key, Address>(&Key::Hook) {
            let hook_asset = if requests.is_empty() {
                from.clone()
            } else {
                requests.get(0).unwrap().address
            };
            let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&env);
            assets.push_back((
                HubAssetKey {
                    hub_id: HARNESS_HUB,
                    asset: hook_asset,
                },
                1i128,
            ));
            BlendHookControllerClient::new(&env, &controller).supply(
                &env.current_contract_address(),
                &0u64,
                &HARNESS_SPOKE,
                &assets,
            );
        }

        let pool = env.current_contract_address();
        let mut withdrew_collateral = false;

        for req in requests.iter() {
            let token = soroban_sdk::token::Client::new(&env, &req.address);
            match req.request_type {
                REQ_REPAY => {
                    token.transfer(&spender, &pool, &req.amount);
                    let k = Key::Liability(from.clone(), req.address.clone());
                    let debt: i128 = env.storage().persistent().get(&k).unwrap_or(0);
                    let pay = req.amount.min(debt);
                    env.storage().persistent().set(&k, &(debt - pay));
                    let refund = req.amount - pay;
                    if refund > 0 {
                        token.transfer(&pool, &to, &refund);
                    }
                }
                REQ_WITHDRAW_COLLATERAL => {
                    withdrew_collateral = true;
                    pay_out(
                        &env,
                        &token,
                        &pool,
                        &to,
                        &from,
                        &req.address,
                        KIND_COLLATERAL,
                        req.amount,
                    );
                }
                REQ_WITHDRAW => {
                    pay_out(
                        &env,
                        &token,
                        &pool,
                        &to,
                        &from,
                        &req.address,
                        KIND_SUPPLY,
                        req.amount,
                    );
                }
                _ => {}
            }
        }

        if withdrew_collateral {
            let liab_assets: Vec<Address> = env
                .storage()
                .persistent()
                .get(&Key::LiabAssets(from.clone()))
                .unwrap_or_else(|| Vec::new(&env));
            for asset in liab_assets.iter() {
                let k = Key::Liability(from.clone(), asset.clone());
                let debt: i128 = env.storage().persistent().get(&k).unwrap_or(0);
                if debt > 0 {
                    return Err(MockBlendError::HealthCheckFailed);
                }
            }
        }

        Ok(BlendPositions {
            liabilities: Map::new(&env),
            collateral: Map::new(&env),
            supply: Map::new(&env),
        })
    }
}

fn key(kind: u32, user: &Address, asset: &Address) -> Key {
    match kind {
        KIND_COLLATERAL => Key::Collateral(user.clone(), asset.clone()),
        KIND_SUPPLY => Key::Supply(user.clone(), asset.clone()),
        _ => Key::Liability(user.clone(), asset.clone()),
    }
}

fn track_liability_asset(env: &Env, user: &Address, asset: &Address) {
    let k = Key::LiabAssets(user.clone());
    let mut list: Vec<Address> = env
        .storage()
        .persistent()
        .get(&k)
        .unwrap_or_else(|| Vec::new(env));
    if !list.contains(asset) {
        list.push_back(asset.clone());
        env.storage().persistent().set(&k, &list);
    }
}

#[allow(clippy::too_many_arguments)]
fn pay_out(
    env: &Env,
    token: &soroban_sdk::token::Client,
    pool: &Address,
    to: &Address,
    from: &Address,
    asset: &Address,
    kind: u32,
    amount: i128,
) {
    // Blend converts the request to b-tokens before clamping it to the
    // position (`apply_withdraw*` in blend-contracts-v2), so an amount the
    // conversion cannot represent traps there and must trap here.
    let b_rate: i128 = env
        .storage()
        .persistent()
        .get(&Key::BRate(asset.clone()))
        .unwrap_or(B_RATE_SCALAR);
    assert_b_token_conversion_fits(env, amount, b_rate);
    let k = key(kind, from, asset);
    let bal: i128 = env.storage().persistent().get(&k).unwrap_or(0);
    let out = amount.min(bal);
    if out > 0 {
        env.storage().persistent().set(&k, &(bal - out));
        token.transfer(pool, to, &out);
    }
}

/// Traps like Blend's `to_b_token_up` when `ceil(amount * SCALAR_12 / b_rate)`
/// does not fit in i128. The product is taken in I256, as Blend does.
fn assert_b_token_conversion_fits(env: &Env, amount: i128, b_rate: i128) {
    let rate = I256::from_i128(env, b_rate);
    let b_tokens = I256::from_i128(env, amount)
        .mul(&I256::from_i128(env, B_RATE_SCALAR))
        .add(&rate.sub(&I256::from_i128(env, 1)))
        .div(&rate);
    assert!(
        b_tokens.to_i128().is_some(),
        "Blend b-token conversion overflowed i128"
    );
}
