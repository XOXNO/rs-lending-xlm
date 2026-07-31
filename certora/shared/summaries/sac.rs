use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

pub fn transfer_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _to: &Address,
    amount: &i128,
) {
    cvlr_assume!(*amount >= 0);
}

pub fn balance_summary(_env: &Env, _token: &Address, _account: &Address) -> i128 {
    let bal: i128 = nondet();
    cvlr_assume!(bal >= 0);
    bal
}

pub fn approve_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _spender: &Address,
    amount: &i128,
    _live_until_ledger: &u32,
) {
    cvlr_assume!(*amount >= 0);
}

pub fn allowance_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _spender: &Address,
) -> i128 {
    let allowance: i128 = nondet();
    cvlr_assume!(allowance >= 0);
    allowance
}
