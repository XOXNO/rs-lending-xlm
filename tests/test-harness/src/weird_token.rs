//! Test-only token with fee-on-transfer, extra credit, and optional transfer
//! hooks into the controller. Not a production SAC.

use common::types::HubAssetKey;
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, Address, Bytes, Env, String, Vec,
};

use crate::helpers::{HARNESS_HUB, HARNESS_SPOKE};

#[contractclient(name = "WeirdTokenControllerClient")]
pub trait WeirdTokenController {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );
}

#[contracttype]
enum Key {
    Balance(Address),
    Allowance(Address, Address),
    Decimals,
    ShortfallBps,
    ExtraBps,
    Hook,
}

#[contract]
pub struct WeirdToken;

#[contractimpl]
impl WeirdToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let balance = read_balance(&env, &to);
        write_balance(&env, &to, balance + amount);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    pub fn set_decimals(env: Env, decimals: u32) {
        env.storage().instance().set(&Key::Decimals, &decimals);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&Key::Decimals).unwrap_or(7)
    }

    pub fn name(env: Env) -> String {
        String::from_str(&env, "Weird")
    }

    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "WEIRD")
    }

    /// Recipient receives `amount * (1 - bps/10000)`.
    pub fn set_shortfall_bps(env: Env, bps: i128) {
        env.storage().instance().set(&Key::ShortfallBps, &bps);
    }

    /// Recipient receives `amount * (1 + bps/10000)`.
    pub fn set_extra_bps(env: Env, bps: i128) {
        env.storage().instance().set(&Key::ExtraBps, &bps);
    }

    /// After each transfer, the token tries to `supply` 1 unit onto the
    /// controller as `from`. Used to prove the flash guard covers the
    /// token-forward window.
    pub fn set_hook(env: Env, controller: Address) {
        env.storage().instance().set(&Key::Hook, &controller);
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        do_transfer(&env, &from, &to, amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        spend_allowance(&env, &from, &spender, amount);
        do_transfer(&env, &from, &to, amount);
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, _expiration: u32) {
        from.require_auth();
        env.storage()
            .instance()
            .set(&Key::Allowance(from, spender), &amount);
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .instance()
            .get(&Key::Allowance(from, spender))
            .unwrap_or(0)
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        let balance = read_balance(&env, &from);
        write_balance(&env, &from, balance - amount);
    }
}

fn read_balance(env: &Env, id: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&Key::Balance(id.clone()))
        .unwrap_or(0)
}

fn write_balance(env: &Env, id: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&Key::Balance(id.clone()), &amount);
}

fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
    let current = env
        .storage()
        .instance()
        .get(&Key::Allowance(from.clone(), spender.clone()))
        .unwrap_or(0);
    assert!(current >= amount, "weird token: insufficient allowance");
    env.storage().instance().set(
        &Key::Allowance(from.clone(), spender.clone()),
        &(current - amount),
    );
}

fn do_transfer(env: &Env, from: &Address, to: &Address, amount: i128) {
    assert!(amount >= 0, "weird token: negative amount");
    let sender = read_balance(env, from);
    assert!(sender >= amount, "weird token: insufficient balance");
    let shortfall: i128 = env
        .storage()
        .instance()
        .get(&Key::ShortfallBps)
        .unwrap_or(0);
    let extra: i128 = env.storage().instance().get(&Key::ExtraBps).unwrap_or(0);
    let delivered = amount - amount * shortfall / 10_000 + amount * extra / 10_000;
    write_balance(env, from, sender - amount);
    let recipient = read_balance(env, to);
    write_balance(env, to, recipient + delivered);

    if let Some(controller) = env.storage().instance().get::<Key, Address>(&Key::Hook) {
        let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
        assets.push_back((
            HubAssetKey {
                hub_id: HARNESS_HUB,
                asset: env.current_contract_address(),
            },
            1i128,
        ));
        WeirdTokenControllerClient::new(env, &controller).supply(
            from,
            &0u64,
            &HARNESS_SPOKE,
            &assets,
        );
    }
}
