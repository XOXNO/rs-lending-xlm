//! Registrar, hub, spoke, and cap-grant storage.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;
use soroban_sdk::{panic_with_error, Address, BytesN, Env, IntoVal, TryFromVal, Val};

use crate::events::{
    CapGrantEvent, HubRegistrarEvent, RegistrarAdminEvent, RolesEvent, SpokeRegistrarEvent,
    SuspendedEvent,
};
use crate::{CapGrant, DataKey};

pub(crate) fn init(env: &Env, governance: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::Governance, governance);
}

pub(crate) fn governance(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Governance).unwrap()
}

pub(crate) fn next_registrar(env: &Env) -> u32 {
    let current: u32 = env
        .storage()
        .instance()
        .get(&DataKey::NextRegistrar)
        .unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&DataKey::NextRegistrar, &next);
    next
}

fn read<V>(env: &Env, key: &DataKey) -> Option<V>
where
    V: TryFromVal<Env, Val>,
{
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    value
}

fn write<V>(env: &Env, key: &DataKey, value: &V)
where
    V: IntoVal<Env, Val>,
{
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

fn registrar_row(env: &Env, registrar: u32) -> Option<(Address, u32)> {
    read(env, &DataKey::RegistrarAdmin(registrar))
}

pub(crate) fn registrar_admin(env: &Env, registrar: u32) -> Option<Address> {
    registrar_row(env, registrar).map(|(admin, _)| admin)
}

fn admin_epoch(env: &Env, registrar: u32) -> u32 {
    registrar_row(env, registrar).map_or(0, |(_, epoch)| epoch)
}

/// Stores `admin` under the next admin epoch. Roles and cap grants are bound
/// to the epoch they were written in, so every admin change voids them.
pub(crate) fn set_registrar_admin(env: &Env, registrar: u32, admin: &Address) {
    let epoch = match registrar_row(env, registrar) {
        Some((_, epoch)) => epoch
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
        None => 0,
    };
    write(
        env,
        &DataKey::RegistrarAdmin(registrar),
        &(admin.clone(), epoch),
    );
    RegistrarAdminEvent {
        registrar,
        admin: admin.clone(),
    }
    .publish(env);
}

fn roles_key(env: &Env, registrar: u32, account: &Address) -> DataKey {
    DataKey::Roles(registrar, admin_epoch(env, registrar), account.clone())
}

pub(crate) fn role_bits(env: &Env, registrar: u32, account: &Address) -> u32 {
    read(env, &roles_key(env, registrar, account)).unwrap_or(0)
}

pub(crate) fn set_role_bits(env: &Env, registrar: u32, account: &Address, bits: u32) {
    let key = roles_key(env, registrar, account);
    if bits == 0 {
        env.storage().persistent().remove(&key);
    } else {
        write(env, &key, &bits);
    }
    RolesEvent {
        registrar,
        account: account.clone(),
        bits,
    }
    .publish(env);
}

pub(crate) fn is_suspended(env: &Env, registrar: u32) -> bool {
    read(env, &DataKey::Suspended(registrar)).unwrap_or(false)
}

pub(crate) fn set_suspended(env: &Env, registrar: u32, suspended: bool) {
    let key = DataKey::Suspended(registrar);
    if suspended {
        write(env, &key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
    SuspendedEvent {
        registrar,
        suspended,
    }
    .publish(env);
}

pub(crate) fn hub_registrar(env: &Env, hub_id: u32) -> Option<u32> {
    read(env, &DataKey::HubRegistrar(hub_id))
}

pub(crate) fn set_hub_registrar(env: &Env, hub_id: u32, registrar: Option<u32>) {
    set_or_remove(env, &DataKey::HubRegistrar(hub_id), registrar);
    HubRegistrarEvent { hub_id, registrar }.publish(env);
}

pub(crate) fn spoke_registrar(env: &Env, spoke_id: u32) -> Option<u32> {
    read(env, &DataKey::SpokeRegistrar(spoke_id))
}

pub(crate) fn set_spoke_registrar(env: &Env, spoke_id: u32, registrar: Option<u32>) {
    set_or_remove(env, &DataKey::SpokeRegistrar(spoke_id), registrar);
    SpokeRegistrarEvent {
        spoke_id,
        registrar,
    }
    .publish(env);
}

fn set_or_remove(env: &Env, key: &DataKey, registrar: Option<u32>) {
    match registrar {
        Some(registrar) => write(env, key, &registrar),
        None => env.storage().persistent().remove(key),
    }
}

/// Returns the stored grant and the registrar that granted it. A grant written
/// under an earlier admin of its grantor reads as absent.
pub(crate) fn cap_grant(
    env: &Env,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
) -> Option<(u32, CapGrant)> {
    let (grantor, epoch, grant): (u32, u32, CapGrant) = read(
        env,
        &DataKey::CapGrant(grantee, hub_id, spoke_id, asset.clone()),
    )?;
    (epoch == admin_epoch(env, grantor)).then_some((grantor, grant))
}

pub(crate) fn set_cap_grant(
    env: &Env,
    grantor: u32,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
    grant: &CapGrant,
) {
    write(
        env,
        &DataKey::CapGrant(grantee, hub_id, spoke_id, asset.clone()),
        &(grantor, admin_epoch(env, grantor), grant.clone()),
    );
    CapGrantEvent {
        grantor,
        grantee,
        hub_id,
        spoke_id,
        asset: asset.clone(),
        grant: Some(grant.clone()),
    }
    .publish(env);
}

pub(crate) fn remove_cap_grant(
    env: &Env,
    grantor: u32,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
) {
    env.storage()
        .persistent()
        .remove(&DataKey::CapGrant(grantee, hub_id, spoke_id, asset.clone()));
    CapGrantEvent {
        grantor,
        grantee,
        hub_id,
        spoke_id,
        asset: asset.clone(),
        grant: None,
    }
    .publish(env);
}

/// Hub and spoke the queued operation `op_id` names, if this contract queued it.
pub(crate) fn proposal(env: &Env, op_id: &BytesN<32>) -> Option<(Option<u32>, Option<u32>)> {
    read(env, &DataKey::Proposal(op_id.clone()))
}

/// Records the hub and spoke a queued operation names, so their owners can cancel it.
pub(crate) fn set_proposal(env: &Env, op_id: &BytesN<32>, hub: Option<u32>, spoke: Option<u32>) {
    write(env, &DataKey::Proposal(op_id.clone()), &(hub, spoke));
}

pub(crate) fn remove_proposal(env: &Env, op_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&DataKey::Proposal(op_id.clone()));
}
