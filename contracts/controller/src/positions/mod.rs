//! Core position lifecycle operations.
//!
//! Each submodule owns one public position flow and its `process_*` pipeline.
//! Shared stages are auth, cache setup, account resolution, validation, pool
//! calls, post-checks, storage writes, and event recording.

use common::types::{Account, AssetConfig, AssetConfigRaw, Payment};
use soroban_sdk::{Address, Env, Map, Vec};

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

/// Resolves the e-mode-adjusted config for every plan asset once, shared by
/// plan validation and pool execution.
pub(crate) fn effective_configs_for_plan(
    env: &Env,
    account: &Account,
    plan: &Vec<Payment>,
    cache: &mut Cache,
) -> Map<Address, AssetConfigRaw> {
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let mut configs: Map<Address, AssetConfigRaw> = Map::new(env);
    for (asset, _) in plan.iter() {
        let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
        configs.set(asset, (&cfg).into());
    }
    configs
}

/// Decodes the prepared config for `asset`; `effective_configs_for_plan`
/// populated every plan key.
pub(crate) fn effective_config(
    env: &Env,
    configs: &Map<Address, AssetConfigRaw>,
    asset: &Address,
) -> AssetConfig {
    (&validation::expect_invariant(env, configs.get(asset.clone()))).into()
}
