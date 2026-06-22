//! Core position lifecycle operations.
//!
//! Each submodule owns one public position flow and its `process_*` pipeline.
//! Shared stages are auth, cache setup, account resolution, validation, pool
//! calls, post-checks, then `finalize_position_flow` (or `persist_account_positions`
//! + `emit_account_updates` when a hook is needed, e.g. liquidation bad-debt).

use common::errors::CollateralError;
use controller_interface::types::{
    Account, AccountPosition, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    ScaledPositionRaw,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::Cache;
use crate::emode;
use crate::helpers;
use crate::storage;
use crate::validation;

pub mod borrow;
pub mod liquidation;
pub mod liquidation_math;
pub mod repay;
pub mod supply;
pub mod withdraw;

/// Deduped payment rows: one entry per asset with the summed amount for the call.
pub(crate) type AggregatedPayments = Vec<Payment>;

/// Which position maps to persist at the end of a flow.
#[derive(Copy, Clone)]
pub(crate) struct PositionSides {
    pub supply: bool,
    pub debt: bool,
}

impl PositionSides {
    pub const SUPPLY: Self = Self {
        supply: true,
        debt: false,
    };
    pub const DEBT: Self = Self {
        supply: false,
        debt: true,
    };
    pub const BOTH: Self = Self {
        supply: true,
        debt: true,
    };
}

/// Writes supply and/or debt maps, or removes the account when `remove_if_empty`
/// and the snapshot has no positions.
pub(crate) fn persist_account_positions(
    env: &Env,
    account_id: u64,
    account: &Account,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    if remove_if_empty && account.is_empty() {
        helpers::remove_account(env, account_id);
        return;
    }
    if sides.supply {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    if sides.debt {
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }
}

/// Standard tail for user position flows: persist then emit.
pub(crate) fn finalize_position_flow(
    env: &Env,
    account_id: u64,
    account: &Account,
    cache: &mut Cache,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    cache.persist_emode_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
    cache.emit_position_batch(account_id, account);
}

/// E-mode-adjusted configs resolved once per aggregated asset, shared by
/// validation and pool execution. Stores the raw form (`Map` values must be
/// contract types); `get` decodes per read.
pub(crate) struct AggregatedConfigs(Map<Address, AssetConfigRaw>);

impl AggregatedConfigs {
    /// Resolves the active e-mode category once and adjusts each aggregated asset.
    pub fn resolve(
        env: &Env,
        account: &Account,
        aggregated: &AggregatedPayments,
        cache: &mut Cache,
    ) -> Self {
        let e_mode = cache.active_e_mode_category(env, account.e_mode_category_id);
        let mut configs: Map<Address, AssetConfigRaw> = Map::new(env);
        for (asset, _) in aggregated.iter() {
            let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
            configs.set(asset, (&cfg).into());
        }
        Self(configs)
    }

    /// Config for an aggregated asset; `resolve` populated each key.
    pub fn get(&self, env: &Env, asset: &Address) -> AssetConfig {
        (&validation::expect_invariant(env, self.0.get(asset.clone()))).into()
    }
}

/// Pure construction helper for the repeated `PoolAction` literal used in each
/// bulk pool entry path. Preserves exact semantics and Into behavior.
pub(crate) fn make_pool_action(
    position: impl Into<ScaledPositionRaw>,
    amount: i128,
    asset: Address,
) -> PoolAction {
    PoolAction {
        position: position.into(),
        amount,
        asset,
    }
}

/// Exact lookup for user-facing repay/withdraw paths. Kept separate from
/// `expect_invariant` liquidation apply paths to preserve missing-position
/// error codes.
pub(crate) fn get_supply_position_or_panic(
    env: &Env,
    account: &Account,
    asset: &Address,
) -> AccountPosition {
    (&account
        .supply_positions
        .get(asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
        .into()
}

pub(crate) fn get_debt_position_or_panic(
    env: &Env,
    account: &Account,
    asset: &Address,
) -> DebtPosition {
    (&account
        .borrow_positions
        .get(asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::PositionNotFound)))
        .into()
}
