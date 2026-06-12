//! Core position lifecycle operations.
//!
//! Each submodule owns one public position flow and its `process_*` pipeline.
//! Shared stages are auth, cache setup, account resolution, validation, pool
//! calls, post-checks, storage writes, and event recording.

use common::errors::CollateralError;
use common::types::{
    Account, AccountPosition, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    ScaledPositionRaw,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::Cache;
use crate::emode;
use crate::validation;

pub mod borrow;
pub mod isolated_debt;
pub mod liquidation;
pub mod liquidation_math;
pub mod repay;
pub mod supply;
pub mod withdraw;

/// E-mode-adjusted configs resolved once per plan asset, shared by plan
/// validation and pool execution. Stores the raw form (`Map` values must be
/// contract types); `get` decodes per read.
pub(crate) struct PlanConfigs(Map<Address, AssetConfigRaw>);

impl PlanConfigs {
    /// Resolves the active e-mode category once and adjusts every plan asset.
    pub fn resolve(env: &Env, account: &Account, plan: &Vec<Payment>, cache: &mut Cache) -> Self {
        let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
        let mut configs: Map<Address, AssetConfigRaw> = Map::new(env);
        for (asset, _) in plan.iter() {
            let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
            configs.set(asset, (&cfg).into());
        }
        Self(configs)
    }

    /// Config for a plan asset; `resolve` populated every plan key.
    pub fn get(&self, env: &Env, asset: &Address) -> AssetConfig {
        (&validation::expect_invariant(env, self.0.get(asset.clone()))).into()
    }
}

/// Pure construction helper for the repeated `PoolAction` literal used in every
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

/// Exact lookup used by user-facing repay/withdraw paths (deliberately distinct
/// from the `expect_invariant` style used in liquidation apply paths to preserve
/// precise error codes on missing positions).
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
