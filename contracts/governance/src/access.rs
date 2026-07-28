//! Ownership, operational roles, and self-admin apply helpers.
//!
//! Owner is root recovery authority. Non-owners may not hold both `EXECUTOR`
//! and `CANCELLER`. Owner roles are not revocable; the last `PROPOSER` cannot
//! be removed.

use common::errors::GenericError;

use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Symbol,
};

use stellar_access::{access_control, ownable, role_transfer};

use crate::{storage, timelock, Governance, GovernanceArgs, GovernanceClient};

pub(crate) const ORACLE_ROLE: &str = "ORACLE";
pub(crate) const PROPOSER_ROLE: &str = "PROPOSER";
pub(crate) const EXECUTOR_ROLE: &str = "EXECUTOR";
pub(crate) const CANCELLER_ROLE: &str = "CANCELLER";
/// Incident role: per-listing pause/freeze and hub/spoke creation without delay.
pub(crate) const GUARDIAN_ROLE: &str = "GUARDIAN";

/// The five operational roles. Ownership is separate and is not a role.
pub(crate) fn default_operational_roles(env: &Env) -> [Symbol; 5] {
    [
        Symbol::new(env, ORACLE_ROLE),
        Symbol::new(env, PROPOSER_ROLE),
        Symbol::new(env, EXECUTOR_ROLE),
        Symbol::new(env, CANCELLER_ROLE),
        Symbol::new(env, GUARDIAN_ROLE),
    ]
}

/// Rejects role symbols outside [`default_operational_roles`].
///
/// # Errors
/// * [`GenericError::InvalidRole`] — unknown role symbol.
pub(crate) fn require_known_governance_role(env: &Env, role: &Symbol) {
    assert_with_error!(
        env,
        default_operational_roles(env).contains(role),
        GenericError::InvalidRole
    );
}

fn sync_pending_admin_transfer(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    let pending_admin_key = access_control::AccessControlStorageKey::PendingAdmin;

    if live_until_ledger == 0 {
        env.storage().temporary().remove(&pending_admin_key);
    } else {
        role_transfer::transfer_role(env, new_owner, &pending_admin_key, live_until_ledger);
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

/// Current owner.
///
/// # Errors
/// * [`GenericError::OwnerNotSet`] — no owner configured.
pub(crate) fn owner_or_panic(env: &Env) -> Address {
    ownable::get_owner(env).unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet))
}

/// Replaces this contract's Wasm after its Sensitive-tier timelock matures.
pub(crate) fn apply_upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    storage::renew_governance_instance(env);
    stellar_contract_utils::upgradeable::upgrade(env, new_wasm_hash);
}

/// Starts two-step ownership transfer: pending owner with expiry; current owner
/// remains until `accept_ownership`.
pub(crate) fn apply_transfer_ownership(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    storage::renew_governance_instance(env);
    let current_owner = owner_or_panic(env);

    role_transfer::transfer_role(
        env,
        new_owner,
        &ownable::OwnableStorageKey::PendingOwner,
        live_until_ledger,
    );
    ownable::emit_ownership_transfer(env, &current_owner, new_owner, live_until_ledger);
    sync_pending_admin_transfer(env, new_owner, live_until_ledger);
}

/// Rejects non-owner accounts that would hold both `EXECUTOR` and `CANCELLER`.
/// Owner is exempt so it can retain cancel authority while holding execute.
fn require_executor_canceller_separation(
    env: &Env,
    owner: &Address,
    account: &Address,
    role: &Symbol,
) {
    if account == owner {
        return;
    }
    let executor = Symbol::new(env, EXECUTOR_ROLE);
    let canceller = Symbol::new(env, CANCELLER_ROLE);
    let conflicting = if role == &executor {
        canceller
    } else if role == &canceller {
        executor
    } else {
        return;
    };
    assert_with_error!(
        env,
        access_control::has_role(env, account, &conflicting).is_none(),
        GenericError::InvalidRole
    );
}

/// Grants an operational role. Enforces executor/canceller separation for
/// non-owners.
///
/// # Errors
/// * [`GenericError::InvalidRole`] — would create executor/canceller overlap.
/// * [`GenericError::OwnerNotSet`] — no owner.
pub(crate) fn apply_grant_role(env: &Env, account: &Address, role: &Symbol) {
    storage::renew_governance_instance(env);
    let owner = owner_or_panic(env);
    require_executor_canceller_separation(env, &owner, account, role);
    access_control::grant_role_no_auth(env, account, role, &owner);
}

/// Replaces the non-owner `CANCELLER` set with `new_cancellers`. Owner's
/// `CANCELLER` is always preserved. Enforces executor/canceller separation on
/// each non-owner grant.
///
/// # Errors
/// * [`GenericError::InvalidRole`] — non-owner would hold both roles.
/// * [`GenericError::OwnerNotSet`] — no owner.
pub(crate) fn apply_canceller_reset(env: &Env, new_cancellers: &soroban_sdk::Vec<Address>) {
    storage::renew_governance_instance(env);
    let owner = owner_or_panic(env);
    let role = Symbol::new(env, CANCELLER_ROLE);
    let mut count = access_control::get_role_member_count(env, &role);
    while count > 0 {
        count -= 1;
        let holder = access_control::get_role_member(env, &role, count);
        if holder != owner {
            access_control::revoke_role_no_auth(env, &holder, &role, &owner);
        }
    }
    for account in new_cancellers.iter() {
        if account != owner && access_control::has_role(env, &account, &role).is_none() {
            require_executor_canceller_separation(env, &owner, &account, &role);
            access_control::grant_role_no_auth(env, &account, &role, &owner);
        }
    }
}

/// Revokes an operational role.
///
/// # Errors
/// * [`GenericError::InvalidRole`] — account does not hold the role.
/// * [`GenericError::NotAuthorized`] — account is the owner.
/// * [`GenericError::CannotRemoveLastProposer`] — would leave zero proposers.
/// * [`GenericError::OwnerNotSet`] — no owner.
pub(crate) fn apply_revoke_role(env: &Env, account: &Address, role: &Symbol) {
    storage::renew_governance_instance(env);
    assert_with_error!(
        env,
        access_control::has_role(env, account, role).is_some(),
        GenericError::InvalidRole
    );
    let owner = owner_or_panic(env);
    assert_with_error!(env, account != &owner, GenericError::NotAuthorized);
    if *role == Symbol::new(env, PROPOSER_ROLE) {
        assert_with_error!(
            env,
            access_control::get_role_member_count(env, role) > 1,
            GenericError::CannotRemoveLastProposer
        );
    }
    access_control::revoke_role_no_auth(env, account, role, &owner);
}

#[contractimpl]
impl Governance {
    /// Sets owner, access-control admin, five operational roles, and min delay.
    ///
    /// # Arguments
    /// * `admin` — becomes owner and holds all operational roles.
    /// * `min_delay` — initial timelock minimum (nonzero; mainnet should be
    ///   ≥ [`crate::TIMELOCK_MIN_DELAY_LEDGERS`]).
    ///
    /// # Errors
    /// * [`GenericError::InvalidTimelockDelay`] — `min_delay` is zero.
    ///
    /// # Security Warning
    /// * Runs once at deploy with no authorization.
    pub fn __constructor(env: Env, admin: Address, min_delay: u32) {
        ownable::set_owner(&env, &admin);
        access_control::set_admin(&env, &admin);

        for role in default_operational_roles(&env) {
            access_control::grant_role_no_auth(&env, &admin, &role, &admin);
        }

        timelock::require_nonzero_delay(&env, min_delay);
        stellar_governance::timelock::set_min_delay(&env, min_delay);
    }

    /// Completes pending ownership transfer. Pending owner must authorize.
    /// Migrates access-control admin and grants all operational roles to the
    /// new owner; revokes them from the previous owner when distinct.
    ///
    /// # Errors
    /// * [`GenericError::OwnerNotSet`] — no current owner.
    /// * Ownable rejects missing or unauthorized pending transfer.
    ///
    /// # Events
    /// * Ownership and admin transfer completed; role grant/revoke events.
    pub fn accept_ownership(env: Env) {
        storage::renew_governance_instance(&env);
        let previous_owner = owner_or_panic(&env);
        ownable::accept_ownership(&env);
        let new_owner = owner_or_panic(&env);
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }

    /// Whether `account` holds `role`.
    pub fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access_control::has_role(&env, &account, &role).is_some()
    }
}

#[cfg(test)]
#[path = "../tests/access.rs"]
mod tests;
