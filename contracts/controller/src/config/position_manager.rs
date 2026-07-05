//! Position-manager registry setter (capped active-manager allowlist).

use common::types::PositionManagerConfig;
use soroban_sdk::{Address, Env};

use crate::storage;

pub(crate) fn set_position_manager(env: &Env, manager: Address, is_active: bool) {
    storage::set_position_manager(env, &manager, &PositionManagerConfig { is_active });
}
