//! Role definitions and access-control state transitions for the governance
//! contract: the default operational roles, owner/admin transfer
//! synchronization, executor/canceller separation enforcement, role
//! grant/revoke application, and the contract constructor.

use common::errors::GenericError;

use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Symbol,
};

use stellar_access::{access_control, ownable, role_transfer};

use crate::{storage, timelock, Governance, GovernanceArgs, GovernanceClient};

/// Identifier for the oracle operational role.
pub(crate) const ORACLE_ROLE: &str = "ORACLE";
/// Identifier for the proposer operational role.
pub(crate) const PROPOSER_ROLE: &str = "PROPOSER";
/// Identifier for the executor operational role.
pub(crate) const EXECUTOR_ROLE: &str = "EXECUTOR";
/// Identifier for the canceller operational role.
pub(crate) const CANCELLER_ROLE: &str = "CANCELLER";

/// Identifier for the guardian operational role.
pub(crate) const GUARDIAN_ROLE: &str = "GUARDIAN";

/// Returns the five default operational role symbols (oracle, proposer,
/// executor, canceller, guardian).
pub(crate) fn default_operational_roles(env: &Env) -> [Symbol; 5] {
    [
        Symbol::new(env, ORACLE_ROLE),
        Symbol::new(env, PROPOSER_ROLE),
        Symbol::new(env, EXECUTOR_ROLE),
        Symbol::new(env, CANCELLER_ROLE),
        Symbol::new(env, GUARDIAN_ROLE),
    ]
}

/// Panics with `GenericError::InvalidRole` unless `role` is one of the
/// default operational roles.
pub(crate) fn require_known_governance_role(env: &Env, role: &Symbol) {
    assert_with_error!(
        env,
        default_operational_roles(env).contains(role),
        GenericError::InvalidRole
    );
}

/// Mirrors a pending owner transfer onto the access-control admin role.
/// Clears the pending-admin temporary-storage entry when `live_until_ledger`
/// is zero, otherwise records `new_owner` as pending admin until that
/// ledger, then emits an admin-transfer-initiated event referencing the
/// current admin (falling back to the current owner). Panics with
/// `GenericError::OwnerNotSet` if neither is set.
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

/// Finalizes an owner handover on the access-control side: sets `new_owner`
/// as the admin, clears the pending-admin entry, emits an
/// admin-transfer-completed event, and re-grants each default operational
/// role to `new_owner`, revoking it from `previous_owner` when the two
/// addresses differ and `previous_owner` still holds it.
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

/// Returns the current owner. Panics with `GenericError::OwnerNotSet` if no
/// owner is set.
pub(crate) fn owner_or_panic(env: &Env) -> Address {
    ownable::get_owner(env).unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet))
}

/// Renews the governance instance's storage TTL and upgrades the contract
/// to `new_wasm_hash`.
pub(crate) fn apply_upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    storage::renew_governance_instance(env);
    stellar_contract_utils::upgradeable::upgrade(env, new_wasm_hash);
}

/// Renews the governance instance's storage TTL, updates the pending-owner
/// entry for `new_owner` (recording it as pending until `live_until_ledger`,
/// or clearing an existing pending transfer to it when `live_until_ledger`
/// is zero), emits an ownership-transfer event, and mirrors the pending
/// transfer onto the access-control admin role.
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

/// Panics with `GenericError::InvalidRole` if granting `role` to `account`
/// would give it both the executor and canceller roles. No-op when
/// `account` is `owner`, and when `role` is neither executor nor canceller.
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

/// Renews the governance instance's storage TTL and grants `role` to
/// `account`, after checking that the grant does not give `account` both
/// the executor and canceller roles.
pub(crate) fn apply_grant_role(env: &Env, account: &Address, role: &Symbol) {
    storage::renew_governance_instance(env);
    let owner = owner_or_panic(env);
    require_executor_canceller_separation(env, &owner, account, role);
    access_control::grant_role_no_auth(env, account, role, &owner);
}

/// Renews the governance instance's storage TTL, revokes the canceller role
/// from every current holder other than `owner`, then grants it to each
/// address in `new_cancellers` that does not already hold it, skipping
/// `owner` and enforcing the executor/canceller separation on each new
/// grant.
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

/// Renews the governance instance's storage TTL and revokes `role` from
/// `account`. Panics with `GenericError::InvalidRole` if `account` does not
/// hold `role`, with `GenericError::NotAuthorized` if `account` is the
/// owner, and with `GenericError::CannotRemoveLastProposer` if this would
/// remove the last remaining holder of the proposer role.
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

/// Renews the governance instance's storage TTL, completes a pending
/// ownership transfer to the caller, and synchronizes the access-control
/// admin and operational-role holders from the previous owner to the new
/// owner.
pub(crate) fn accept_ownership(env: &Env) {
    storage::renew_governance_instance(env);
    let previous_owner = owner_or_panic(env);
    ownable::accept_ownership(env);
    let new_owner = owner_or_panic(env);
    sync_owner_access_control(env, &previous_owner, &new_owner);
}

/// Returns whether `account` currently holds `role`.
pub(crate) fn has_role(env: &Env, account: &Address, role: &Symbol) -> bool {
    access_control::has_role(env, account, role).is_some()
}

#[contractimpl]
impl Governance {
    /// Initializes the governance contract: sets `admin` as both owner and
    /// access-control admin, grants it every default operational role, and
    /// sets the timelock minimum delay to `min_delay`. Panics with
    /// `GenericError::InvalidTimelockDelay` if `min_delay` is zero.
    pub fn __constructor(env: Env, admin: Address, min_delay: u32) {
        ownable::set_owner(&env, &admin);
        // Both `set_owner` and `set_admin` are bare storage writes. Without
        // these emissions the initial owner and admin are unreachable from the
        // event stream — `ownership_transfer*` and `admin_transfer*` only fire
        // on a later handover, so a replay from genesis still learns nothing.
        ownable::emit_ownership_transfer_completed(&env, &admin);
        access_control::set_admin(&env, &admin);
        // Previous and new admin are the same address at construction: there is
        // no prior admin to hand over from, and the event's meaning here is
        // "admin is now this address".
        access_control::emit_admin_transfer_completed(&env, &admin, &admin);

        for role in default_operational_roles(&env) {
            access_control::grant_role_no_auth(&env, &admin, &role, &admin);
        }

        timelock::require_nonzero_delay(&env, min_delay);
        stellar_governance::timelock::set_min_delay(&env, min_delay);
    }
}

#[cfg(test)]
#[path = "../tests/access.rs"]
mod tests;
