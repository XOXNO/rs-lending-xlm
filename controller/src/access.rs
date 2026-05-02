use common::errors::GenericError;
use common::types::PositionLimits;
use soroban_sdk::{contractimpl, panic_with_error, Address, BytesN, Env, Symbol};
use stellar_access::{access_control, ownable};
use stellar_macros::only_owner;

use crate::{storage, Controller, ControllerArgs, ControllerClient};

pub(crate) const KEEPER_ROLE: &str = "KEEPER"; // update_indexes, clean_bad_debt, update_account_threshold
pub(crate) const REVENUE_ROLE: &str = "REVENUE"; // claim_revenue, add_rewards
pub(crate) const ORACLE_ROLE: &str = "ORACLE"; // configure_market_oracle, edit_oracle_tolerance, disable_token_oracle

fn default_operational_roles(env: &Env) -> [Symbol; 3] {
    [
        Symbol::new(env, KEEPER_ROLE),
        Symbol::new(env, REVENUE_ROLE),
        Symbol::new(env, ORACLE_ROLE),
    ]
}

fn sync_pending_admin_transfer(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    let pending_admin_key = access_control::AccessControlStorageKey::PendingAdmin;

    if live_until_ledger == 0 {
        env.storage().temporary().remove(&pending_admin_key);
    } else {
        stellar_access::role_transfer::transfer_role(
            env,
            new_owner,
            &pending_admin_key,
            live_until_ledger,
        );
    }

    let current_admin = access_control::get_admin(env)
        .or_else(|| ownable::get_owner(env))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet));
    access_control::emit_admin_transfer_initiated(
        env,
        &current_admin,
        new_owner,
        live_until_ledger,
    );
}

fn sync_owner_access_control(env: &Env, previous_owner: &Address, new_owner: &Address) {
    let previous_admin = access_control::get_admin(env).unwrap_or_else(|| previous_owner.clone());

    env.storage()
        .instance()
        .set(&access_control::AccessControlStorageKey::Admin, new_owner);
    env.storage()
        .temporary()
        .remove(&access_control::AccessControlStorageKey::PendingAdmin);
    access_control::emit_admin_transfer_completed(env, &previous_admin, new_owner);

    for role in default_operational_roles(env) {
        access_control::grant_role_no_auth(env, new_owner, &role, new_owner);

        if previous_owner != new_owner
            && access_control::has_role(env, previous_owner, &role).is_some()
        {
            access_control::revoke_role_no_auth(env, previous_owner, &role, new_owner);
        }
    }
}

#[contractimpl]
impl Controller {
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);

        // Grant only KEEPER at construct. REVENUE and ORACLE require an
        // explicit `grant_role` after deploy so a compromised owner key in
        // the bootstrap window cannot immediately exercise those roles.
        access_control::set_admin(&env, &admin);
        let keeper_role = Symbol::new(&env, KEEPER_ROLE);
        access_control::grant_role_no_auth(&env, &admin, &keeper_role, &admin);

        storage::set_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 10,
                max_borrow_positions: 10,
            },
        );

        // Pause at construct; operator must `unpause` after wiring
        // aggregator, accumulator, pool template, oracles, and markets.
        // `upgrade` applies the same auto-pause.
        stellar_contract_utils::pausable::pause(&env);
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        stellar_contract_utils::pausable::pause(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    #[only_owner]
    pub fn pause(env: Env) {
        stellar_contract_utils::pausable::pause(&env);
    }

    #[only_owner]
    pub fn unpause(env: Env) {
        stellar_contract_utils::pausable::unpause(&env);
    }

    #[only_owner]
    pub fn grant_role(env: Env, account: Address, role: Symbol) {
        let owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        access_control::grant_role_no_auth(&env, &account, &role, &owner);
    }

    #[only_owner]
    pub fn revoke_role(env: Env, account: Address, role: Symbol) {
        let owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        access_control::revoke_role_no_auth(&env, &account, &role, &owner);
    }

    pub fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access_control::has_role(&env, &account, &role).is_some()
    }

    #[only_owner]
    pub fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        let current_owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));

        stellar_access::role_transfer::transfer_role(
            &env,
            &new_owner,
            &ownable::OwnableStorageKey::PendingOwner,
            live_until_ledger,
        );
        ownable::emit_ownership_transfer(&env, &current_owner, &new_owner, live_until_ledger);
        sync_pending_admin_transfer(&env, &new_owner, live_until_ledger);
    }

    pub fn accept_ownership(env: Env) {
        let previous_owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        ownable::accept_ownership(&env);
        let new_owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }
}
