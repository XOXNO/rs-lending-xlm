//! Account renewal and delegate-list mutation helpers.

use common::errors::GenericError;
use soroban_sdk::{assert_with_error, Address, Env};

use crate::storage;

pub(crate) fn renew_account(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    let meta = storage::get_account_meta(env, account_id);
    assert_with_error!(env, meta.owner == *caller, GenericError::AccountNotInMarket);

    storage::renew_user_account(env, account_id);
}

pub(crate) fn set_account_delegate(
    env: &Env,
    caller: &Address,
    account_id: u64,
    delegate: &Address,
    add: bool,
) {
    caller.require_auth();
    let meta = storage::get_account_meta(env, account_id);
    assert_with_error!(env, meta.owner == *caller, GenericError::AccountNotInMarket);

    if add {
        storage::add_delegate(env, account_id, delegate);
    } else {
        storage::remove_delegate(env, account_id, delegate);
    }
}
