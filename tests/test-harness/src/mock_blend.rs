//! Blend V2 `submit` mock for migration: real per-user balances (no shares/index).
//! Field names match production `#[contracttype]` maps. Auth: spender always;
//! `from` iff `from != spender`. Repay full pull + refund; withdraw min(amt, bal)
//! (`i128::MAX` = all). Post-batch: any withdraw while liability remains reverts.
//! Seed via `seed` + mint underlyings to the mock for payouts.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Map, Vec};

const REQ_WITHDRAW: u32 = 1;
const REQ_WITHDRAW_COLLATERAL: u32 = 3;
const REQ_REPAY: u32 = 5;

/// `kind` values for [`MockBlend::seed`].
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
    /// Per-user list of assets the user has a liability in (append-only).
    LiabAssets(Address),
}

#[contract]
pub struct MockBlend;

#[contractimpl]
impl MockBlend {
    pub fn __constructor(_env: Env) {}

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

        // Blend reverts when collateral is withdrawn while the user still owes ANY
        // liability (the post-action health check). Checking every tracked
        // liability asset — not just assets repaid in this batch — keeps the check
        // valid when the withdrawal is in a different submit from the repay (the
        // two-phase migration path).
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

        // Discarded by the controller; an empty value is sufficient.
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

/// Records `asset` in `user`'s liability-asset list (append-only, deduplicated)
/// so the post-action health check can enumerate the user's debts.
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
    let k = key(kind, from, asset);
    let bal: i128 = env.storage().persistent().get(&k).unwrap_or(0);
    let out = amount.min(bal);
    if out > 0 {
        env.storage().persistent().set(&k, &(bal - out));
        token.transfer(pool, to, &out);
    }
}
