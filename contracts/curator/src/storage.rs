//! Registrar, hub, spoke, and cap-grant storage.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;
use soroban_sdk::{panic_with_error, Address, BytesN, Env, IntoVal, TryFromVal, Val};

use crate::{CapGrant, DataKey};

pub(crate) fn init(env: &Env, owner: &Address, governance: &Address) {
    env.storage().instance().set(&DataKey::Owner, owner);
    env.storage()
        .instance()
        .set(&DataKey::Governance, governance);
}

pub(crate) fn owner(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Owner).unwrap()
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
    env.storage().persistent().get(key)
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

pub(crate) fn registrar_admin(env: &Env, registrar: u32) -> Option<Address> {
    read(env, &DataKey::RegistrarAdmin(registrar))
}

pub(crate) fn set_registrar_admin(env: &Env, registrar: u32, admin: &Address) {
    write(env, &DataKey::RegistrarAdmin(registrar), admin);
}

pub(crate) fn role_bits(env: &Env, registrar: u32, account: &Address) -> u32 {
    read(env, &DataKey::Roles(registrar, account.clone())).unwrap_or(0)
}

pub(crate) fn set_role_bits(env: &Env, registrar: u32, account: &Address, bits: u32) {
    let key = DataKey::Roles(registrar, account.clone());
    if bits == 0 {
        env.storage().persistent().remove(&key);
        return;
    }
    write(env, &key, &bits);
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
}

pub(crate) fn hub_registrar(env: &Env, hub_id: u32) -> Option<u32> {
    read(env, &DataKey::HubRegistrar(hub_id))
}

pub(crate) fn set_hub_registrar(env: &Env, hub_id: u32, registrar: Option<u32>) {
    set_or_remove(env, &DataKey::HubRegistrar(hub_id), registrar);
}

pub(crate) fn spoke_registrar(env: &Env, spoke_id: u32) -> Option<u32> {
    read(env, &DataKey::SpokeRegistrar(spoke_id))
}

pub(crate) fn set_spoke_registrar(env: &Env, spoke_id: u32, registrar: Option<u32>) {
    set_or_remove(env, &DataKey::SpokeRegistrar(spoke_id), registrar);
}

fn set_or_remove(env: &Env, key: &DataKey, registrar: Option<u32>) {
    match registrar {
        Some(registrar) => write(env, key, &registrar),
        None => env.storage().persistent().remove(key),
    }
}

/// Returns the stored grant and the registrar that granted it.
pub(crate) fn cap_grant(
    env: &Env,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
) -> Option<(u32, CapGrant)> {
    read(
        env,
        &DataKey::CapGrant(grantee, hub_id, spoke_id, asset.clone()),
    )
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
        &(grantor, grant.clone()),
    );
}

pub(crate) fn remove_cap_grant(
    env: &Env,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
) {
    env.storage()
        .persistent()
        .remove(&DataKey::CapGrant(grantee, hub_id, spoke_id, asset.clone()));
}

/// Returns the proposing registrar and the hub of a queued curator proposal.
pub(crate) fn pending(env: &Env, op_id: &BytesN<32>) -> Option<(u32, u32)> {
    read(env, &DataKey::Pending(op_id.clone()))
}

pub(crate) fn set_pending(env: &Env, op_id: &BytesN<32>, registrar: u32, hub_id: u32) {
    write(env, &DataKey::Pending(op_id.clone()), &(registrar, hub_id));
}

pub(crate) fn remove_pending(env: &Env, op_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&DataKey::Pending(op_id.clone()));
}
