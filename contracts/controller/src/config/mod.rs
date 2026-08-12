
pub(crate) mod asset;
pub(crate) mod limits;
pub(crate) mod registry;
pub(crate) mod spoke;

#[cfg(feature = "certora")]
pub(crate) use asset::{add_asset_to_spoke, edit_asset_in_spoke};
pub(crate) use spoke::require_hub_active;
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
