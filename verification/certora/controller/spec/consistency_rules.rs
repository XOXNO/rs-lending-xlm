/// Controller/pool consistency rules.
///
/// Prove that the controller persists the position the pool returns: after a
/// supply or borrow, the account's scaled deposit/debt does not decrease. These
/// back `confs/controller-pool-consistency.conf` (heavy, sanity off) and its
/// paired `confs/controller-pool-consistency-light.conf` (basic sanity).
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use controller::constants::WAD;
use controller::types::AccountPositionType;

#[rule]
fn controller_supply_persists_pool_returned_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );
    cvlr_assert!(after >= before);
}

#[rule]
fn controller_borrow_persists_pool_returned_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    let after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );
    cvlr_assert!(after >= before);
}
