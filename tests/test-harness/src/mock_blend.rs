//! Faithful Blend V2 pool stand-in for migration tests.
//!
//! Mimics the Blend `submit` surface the migration strategy uses, with REAL
//! per-user position accounting (underlying amounts, no shares/index — the
//! migration never reads Blend rates). Field names in `BlendRequest` /
//! `BlendPositions` match Blend V2 production exactly because `#[contracttype]`
//! encodes as field-name maps and the controller's `BlendPoolClient` decodes by
//! name.
//!
//! Semantics (Blend `pool/src/pool/{actions,submit}.rs`):
//!   - `spender.require_auth()` always; `from.require_auth()` iff `from != spender`.
//!   - REQ_REPAY (5): pull the FULL `amount` from `spender`, reduce `from`'s
//!     liability by `min(amount, debt)`, refund the excess to `to`.
//!   - REQ_WITHDRAW_COLLATERAL (3) / REQ_WITHDRAW (1): pay `min(amount, balance)`
//!     of `from`'s collateral / supply to `to` (so `i128::MAX` == withdraw-all).
//!   - After the batch, if any collateral was withdrawn while `from` still owes a
//!     repaid asset, revert (Blend's post-action health check).
//!
//! Seed a user's Blend balances with `seed(user, asset, kind, amount)` and mint
//! the underlying tokens to the mock's address so it can pay withdrawals/refunds.

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
    /// Collateral withdrawn while the user still owes a repaid asset.
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
}

#[contract]
pub struct MockBlend;

#[contractimpl]
impl MockBlend {
    pub fn __constructor(_env: Env) {}

    /// Sets `user`'s underlying balance for a position `kind` (0=collateral,
    /// 1=supply, 2=liability). Test-only.
    pub fn seed(env: Env, user: Address, asset: Address, kind: u32, amount: i128) {
        env.storage()
            .persistent()
            .set(&key(kind, &user, &asset), &amount);
    }

    /// Reads `user`'s underlying balance for a position `kind`.
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

        // Blend reverts when collateral is withdrawn while the user is still
        // indebted in any repaid asset (the post-action health check).
        if withdrew_collateral {
            for req in requests.iter() {
                if req.request_type == REQ_REPAY {
                    let k = Key::Liability(from.clone(), req.address.clone());
                    let debt: i128 = env.storage().persistent().get(&k).unwrap_or(0);
                    if debt > 0 {
                        return Err(MockBlendError::HealthCheckFailed);
                    }
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
