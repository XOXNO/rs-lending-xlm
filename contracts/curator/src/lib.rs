//! Registrar registry owned by governance, which opens registrars and moves
//! hubs and spokes through its timelock. This contract is the address that
//! holds the governance `CURATOR`, `PROPOSER` and `CANCELLER` roles.

#![no_std]
#![allow(clippy::too_many_arguments)]

mod api;
mod auth;
mod ops;
mod storage;

use soroban_sdk::{contract, contracterror, contractmeta, contracttype, Address, BytesN};

/// Registers markets, lists assets, lowers caps, and queues listing edits.
pub const ROLE_MARKET: u32 = 1;
/// Creates hubs and spokes.
pub const ROLE_SPOKE: u32 = 2;
/// Tightens listing flags and queues a flag relaxation.
pub const ROLE_FLAGS: u32 = 4;
pub(crate) const ROLE_MASK: u32 = ROLE_MARKET | ROLE_SPOKE | ROLE_FLAGS;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CuratorError {
    UnknownRegistrar = 801,
    RoleMissing = 802,
    RoleBits = 803,
    HubOwned = 804,
    HubUnowned = 805,
    SpokeOwned = 806,
    SpokeUnowned = 807,
    NotHubRegistrar = 808,
    NotSpokeRegistrar = 809,
    GrantMissing = 812,
    GrantExceeded = 813,
    Unsupported = 814,
    RegistrarSuspended = 815,
    NotListingParty = 816,
    NotCancellable = 817,
}

/// Cap one registrar grants another for a single hub asset in one grantee spoke.
#[contracttype]
#[derive(Clone, Debug)]
pub struct CapGrant {
    pub supply_cap: i128,
    pub borrow_cap: i128,
    pub can_collateral: bool,
    pub can_borrow: bool,
    /// Maximum loan-to-value, in BPS, a grantee may set for this hub asset.
    pub ltv: u32,
    /// Maximum liquidation threshold, in BPS, a grantee may set for this hub asset.
    pub threshold: u32,
    /// Maximum liquidation bonus, in BPS, a grantee may set for this hub asset.
    pub bonus: u32,
    /// Maximum liquidation fee, in BPS, a grantee may set for this hub asset.
    pub liquidation_fees: u32,
}

#[contracttype]
#[derive(Clone)]
pub(crate) enum DataKey {
    Governance,
    NextRegistrar,
    RegistrarAdmin(u32),
    Roles(u32, Address),
    HubRegistrar(u32),
    SpokeRegistrar(u32),
    CapGrant(u32, u32, u32, Address),
    Suspended(u32),
    Pending(BytesN<32>),
}

contractmeta!(key = "name", val = "Lending Curator");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Curator;
