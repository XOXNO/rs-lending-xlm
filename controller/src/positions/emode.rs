use common::errors::{CollateralError, EModeError};
use common::types::{Account, AssetConfig, EModeAssetConfig, EModeCategory};
use soroban_sdk::{panic_with_error, Address, Env};

use crate::storage;

// ---------------------------------------------------------------------------
// Core e-mode functions
// ---------------------------------------------------------------------------

pub fn apply_e_mode_to_asset_config(
    _env: &Env,
    asset_config: &mut AssetConfig,
    category: &Option<EModeCategory>,
    asset_emode_config: Option<EModeAssetConfig>,
) {
    if let (Some(cat), Some(aec)) = (category, asset_emode_config) {
        if cat.is_deprecated {
            return;
        }
        asset_config.is_collateralizable = aec.is_collateralizable;
        asset_config.is_borrowable = aec.is_borrowable;
        asset_config.loan_to_value_bps = cat.loan_to_value_bps;
        asset_config.liquidation_threshold_bps = cat.liquidation_threshold_bps;
        asset_config.liquidation_bonus_bps = cat.liquidation_bonus_bps;
    }
}

pub fn ensure_e_mode_compatible_with_asset(env: &Env, asset_config: &AssetConfig, e_mode_id: u32) {
    if asset_config.is_isolated_asset && e_mode_id > 0 {
        panic_with_error!(env, EModeError::EModeWithIsolated);
    }
}

pub fn token_e_mode_config(env: &Env, e_mode_id: u32, asset: &Address) -> Option<EModeAssetConfig> {
    if e_mode_id == 0 {
        return None;
    }

    let asset_cats = storage::get_asset_emodes(env, asset);
    if !asset_cats.contains(e_mode_id) {
        panic_with_error!(env, EModeError::EModeCategoryNotFound);
    }

    let config = storage::get_emode_asset(env, e_mode_id, asset);
    if config.is_none() {
        panic_with_error!(env, EModeError::EModeCategoryNotFound);
    }
    config
}

pub fn e_mode_category(env: &Env, e_mode_id: u32) -> Option<EModeCategory> {
    if e_mode_id == 0 {
        return None;
    }
    Some(storage::get_emode_category(env, e_mode_id))
}

// ---------------------------------------------------------------------------
// Deprecation check
// ---------------------------------------------------------------------------

pub fn ensure_e_mode_not_deprecated(env: &Env, category: &Option<EModeCategory>) {
    if let Some(cat) = category {
        if cat.is_deprecated {
            panic_with_error!(env, EModeError::EModeCategoryDeprecated);
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience helpers (used by strategy.rs and other callers)
// ---------------------------------------------------------------------------

pub fn validate_e_mode_asset(env: &Env, e_mode_category_id: u32, asset: &Address, is_supply: bool) {
    if e_mode_category_id == 0 {
        return;
    }

    let config = token_e_mode_config(env, e_mode_category_id, asset);
    match config {
        None => {},
        Some(cfg) => {
            if is_supply && !cfg.is_collateralizable {
                panic_with_error!(env, CollateralError::NotCollateral);
            }
            if !is_supply && !cfg.is_borrowable {
                panic_with_error!(env, CollateralError::AssetNotBorrowable);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Isolation mode enforcement (accepts pre-loaded data from caller)
// ---------------------------------------------------------------------------

pub fn validate_isolated_collateral(
    env: &Env,
    account: &Account,
    asset: &Address,
    asset_config: &AssetConfig,
) {
    // Neither account nor asset is isolated — nothing to check.
    if !account.is_isolated && !asset_config.is_isolated_asset {
        return;
    }

    // Non-isolated account trying to supply an isolated asset — reject.
    if !account.is_isolated && asset_config.is_isolated_asset {
        panic_with_error!(env, EModeError::MixIsolatedCollateral);
    }

    // Isolated account: first deposit is always OK.
    if account.supply_positions.is_empty() {
        return;
    }

    // If deposits exist, the new asset must match the existing one.
    for existing_asset in account.supply_positions.keys() {
        if existing_asset != *asset {
            panic_with_error!(env, EModeError::MixIsolatedCollateral);
        }
    }
}

pub fn validate_e_mode_isolation_exclusion(env: &Env, e_mode_category: u32, is_isolated: bool) {
    if e_mode_category > 0 && is_isolated {
        panic_with_error!(env, EModeError::EModeWithIsolated);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{AccountPosition, ControllerKey};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Map, Vec};

    struct TestSetup {
        env: Env,
        controller: Address,
        asset_a: Address,
        asset_b: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin,));
            let asset_a = Address::generate(&env);
            let asset_b = Address::generate(&env);

            Self {
                env,
                controller,
                asset_a,
                asset_b,
            }
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn account(&self, isolated: bool) -> Account {
            Account {
                owner: Address::generate(&self.env),
                is_isolated: isolated,
                e_mode_category_id: 0,
                mode: common::types::PositionMode::Normal,
                isolated_asset: isolated.then(|| self.asset_a.clone()),
                supply_positions: Map::new(&self.env),
                borrow_positions: Map::new(&self.env),
            }
        }

        fn asset_config(&self, is_isolated_asset: bool) -> AssetConfig {
            AssetConfig {
                loan_to_value_bps: 7_500,
                liquidation_threshold_bps: 8_000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                is_collateralizable: true,
                is_borrowable: true,
                e_mode_enabled: false,
                is_isolated_asset,
                is_siloed_borrowing: false,
                is_flashloanable: true,
                isolation_borrow_enabled: true,
                isolation_debt_ceiling_usd_wad: 0,
                flashloan_fee_bps: 9,
                borrow_cap: 0,
                supply_cap: 0,
            }
        }
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_ensure_e_mode_compatible_with_asset_rejects_isolated_assets() {
        let t = TestSetup::new();
        t.as_controller(|| {
            ensure_e_mode_compatible_with_asset(&t.env, &t.asset_config(true), 1);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #300)")]
    fn test_token_e_mode_config_rejects_missing_asset_config() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let categories = Vec::from_array(&t.env, [1u32]);
            t.env
                .storage()
                .persistent()
                .set(&ControllerKey::AssetEModes(t.asset_a.clone()), &categories);

            let _ = token_e_mode_config(&t.env, 1, &t.asset_a);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #104)")]
    fn test_validate_e_mode_asset_rejects_non_collateralizable_membership() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let categories = Vec::from_array(&t.env, [1u32]);
            t.env
                .storage()
                .persistent()
                .set(&ControllerKey::AssetEModes(t.asset_a.clone()), &categories);
            storage::set_emode_asset(
                &t.env,
                1,
                &t.asset_a,
                &EModeAssetConfig {
                    is_collateralizable: false,
                    is_borrowable: true,
                },
            );

            validate_e_mode_asset(&t.env, 1, &t.asset_a, true);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #107)")]
    fn test_validate_e_mode_asset_rejects_non_borrowable_membership() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let categories = Vec::from_array(&t.env, [1u32]);
            t.env
                .storage()
                .persistent()
                .set(&ControllerKey::AssetEModes(t.asset_a.clone()), &categories);
            storage::set_emode_asset(
                &t.env,
                1,
                &t.asset_a,
                &EModeAssetConfig {
                    is_collateralizable: true,
                    is_borrowable: false,
                },
            );

            validate_e_mode_asset(&t.env, 1, &t.asset_a, false);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #303)")]
    fn test_validate_isolated_collateral_rejects_mixing_assets() {
        let t = TestSetup::new();
        t.as_controller(|| {
            let mut account = t.account(true);
            account.supply_positions.set(
                t.asset_a.clone(),
                AccountPosition {
                    position_type: common::types::AccountPositionType::Deposit,
                    asset: t.asset_a.clone(),
                    scaled_amount_ray: 100,
                    account_id: 1,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    loan_to_value_bps: 7_500,
                },
            );

            validate_isolated_collateral(&t.env, &account, &t.asset_b, &t.asset_config(true));
        });
    }
}
