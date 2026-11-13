//! The production liquidation modules (see `liquidation/mod.rs`) and
//! stand-ins for the helpers `liquidation/apply.rs` imports from
//! `contracts/controller/src/positions/{mod,debt,supply}.rs`:
//!
//! * `make_pool_action`, `get_supply_position_or_panic`, `FreezePolicy` and
//!   `WithdrawKind`: copied verbatim.
//! * `enforce_spoke_asset_flags`, `apply_repay_batch` and
//!   `apply_withdraw_batch`: opaque (Charon `--opaque`); they read spoke
//!   configuration and call the pool. Modelled in `FunsExternal.lean`.

pub mod liquidation;

use crate::context::Context;
use crate::errors::CollateralError;
use crate::events;
use crate::types::{
    Account, AccountPosition, HubAssetKey, PoolAction, PoolPositionMutation, PoolWithdrawEntry,
    ScaledPositionRaw,
};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

/// Spoke-asset flag policy (`positions/mod.rs`).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FreezePolicy {
    /// New exposure: rejects `paused` and `frozen`.
    BlockOnEntry,
    /// User-initiated exit: rejects `paused`, tolerates `frozen`.
    AllowOnExit,
    /// Liquidation seizure: rejects `no_seize` only, tolerates `paused` and `frozen`.
    SeizureLeg,
}

/// `positions/supply.rs`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WithdrawKind {
    Normal,
    Liquidation,
}

/// Rejects the leg when the spoke asset's flags forbid it under `freeze`. Opaque.
pub(crate) fn enforce_spoke_asset_flags(
    _env: &Env,
    _cache: &mut Context,
    _spoke_id: u32,
    _hub_asset: &HubAssetKey,
    _freeze: FreezePolicy,
) {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}

/// Repays the actions through the pool and applies the results to `account`. Opaque.
pub(crate) fn apply_repay_batch(
    _env: &Env,
    _account: &mut Account,
    _payer: &Address,
    _action: events::PositionAction,
    _actions: &Vec<PoolAction>,
    _cache: &mut Context,
) -> Vec<PoolPositionMutation> {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}

/// Withdraws the entries through the pool and applies the results to `account`. Opaque.
pub(crate) fn apply_withdraw_batch(
    _env: &Env,
    _account: &mut Account,
    _recipient: &Address,
    _kind: WithdrawKind,
    _action: events::PositionAction,
    _entries: &Vec<PoolWithdrawEntry>,
    _cache: &mut Context,
) -> Vec<PoolPositionMutation> {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}

/// Copied verbatim from `positions/mod.rs`.
pub(crate) fn make_pool_action(
    position: impl Into<ScaledPositionRaw>,
    amount: i128,
    hub_asset: HubAssetKey,
) -> PoolAction {
    PoolAction {
        position: position.into(),
        amount,
        hub_asset,
    }
}

/// Copied verbatim from `positions/mod.rs`.
pub(crate) fn get_supply_position_or_panic(
    env: &Env,
    account: &Account,
    hub_asset: &HubAssetKey,
) -> AccountPosition {
    (&account
        .supply_positions
        .get(hub_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound)))
        .into()
}
