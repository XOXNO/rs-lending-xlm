//! One event per registry write, published by the storage setter that makes
//! the write. Forwarded calls are covered by controller and governance
//! events; `ProposedEvent` names the registrar behind a queued operation.

use soroban_sdk::{contractevent, Address, BytesN};

use crate::CapGrant;

/// A registrar was opened or its admin replaced.
#[contractevent(topics = ["curator", "registrar_admin"])]
#[derive(Clone, Debug)]
pub struct RegistrarAdminEvent {
    pub registrar: u32,
    pub admin: Address,
}

/// An account's role bits in a registrar; `0` removes the account.
#[contractevent(topics = ["curator", "roles"])]
#[derive(Clone, Debug)]
pub struct RolesEvent {
    pub registrar: u32,
    pub account: Address,
    pub bits: u32,
}

#[contractevent(topics = ["curator", "suspended"])]
#[derive(Clone, Debug)]
pub struct SuspendedEvent {
    pub registrar: u32,
    pub suspended: bool,
}

/// Hub ownership; `None` unbinds the hub from every registrar.
#[contractevent(topics = ["curator", "hub_registrar"])]
#[derive(Clone, Debug)]
pub struct HubRegistrarEvent {
    pub hub_id: u32,
    pub registrar: Option<u32>,
}

/// Spoke ownership; `None` unbinds the spoke from every registrar.
#[contractevent(topics = ["curator", "spoke_registrar"])]
#[derive(Clone, Debug)]
pub struct SpokeRegistrarEvent {
    pub spoke_id: u32,
    pub registrar: Option<u32>,
}

/// A grant was set (`Some`) or revoked (`None`) by `grantor`.
#[contractevent(topics = ["curator", "cap_grant"])]
#[derive(Clone, Debug)]
pub struct CapGrantEvent {
    pub grantor: u32,
    pub grantee: u32,
    pub hub_id: u32,
    pub spoke_id: u32,
    pub asset: Address,
    pub grant: Option<CapGrant>,
}

/// A registrar queued governance operation `op_id`.
#[contractevent(topics = ["curator", "proposed"])]
#[derive(Clone, Debug)]
pub struct ProposedEvent {
    pub registrar: u32,
    pub op_id: BytesN<32>,
}
