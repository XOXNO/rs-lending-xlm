//! Owner-gated protocol configuration: registries, hub/spoke listings, limits,
//! and allowlists. Auth is `#[only_owner]` (governance after execute; GUARDIAN
//! immediate for hub/spoke create and tighten-only flags).

pub(crate) mod approvals;
pub(crate) mod asset;
pub(crate) mod hub;
pub(crate) mod limits;
pub(crate) mod registry;
pub(crate) mod spoke;

#[cfg(feature = "certora")]
pub(crate) use asset::{add_asset_to_spoke, edit_asset_in_spoke};
pub(crate) use hub::require_hub_active;
#[cfg(feature = "certora")]
pub(crate) use spoke::remove_spoke;

#[cfg(test)]
use common::types::HubConfig;
#[cfg(test)]
use soroban_sdk::{Address, Env};

#[cfg(test)]
use crate::{storage, Controller};

#[cfg(test)]
#[path = "../../tests/governance/config.rs"]
mod tests;
