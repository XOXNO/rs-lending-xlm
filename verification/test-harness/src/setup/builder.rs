extern crate std;

use std::collections::HashMap;

use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{token, Address, Env, Symbol};

use crate::core::types::{LendingTest, MarketState, PendingEMode, PendingMarket};
use crate::helpers::f64_to_i128;
use crate::presets::{
    AssetConfigPreset, EModeCategoryPreset, MarketParamsPreset, MarketPreset, DEFAULT_TOLERANCE,
};

pub struct LendingTestBuilder {
    pending_markets: Vec<PendingMarket>,
    pending_emodes: Vec<PendingEMode>,
    position_limits: Option<(u32, u32)>,
    budget_enabled: bool,
    skip_mock_auths: bool,
}

impl LendingTest {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> LendingTestBuilder {
        LendingTestBuilder {
            pending_markets: Vec::new(),
            pending_emodes: Vec::new(),
            position_limits: None,
            budget_enabled: false,
            skip_mock_auths: false,
        }
    }
}

impl LendingTestBuilder {
    pub fn with_market(mut self, preset: MarketPreset) -> Self {
        self.pending_markets
            .push(PendingMarket::from_preset(preset));
        self
    }

    pub fn with_market_config(
        mut self,
        name: &str,
        f: impl FnOnce(&mut AssetConfigPreset),
    ) -> Self {
        for pm in &mut self.pending_markets {
            if pm.name == name {
                f(&mut pm.config);
                return self;
            }
        }
        panic!("market '{}' not found -- call .with_market() first", name);
    }

    pub fn with_market_params(
        mut self,
        name: &str,
        f: impl FnOnce(&mut MarketParamsPreset),
    ) -> Self {
        for pm in &mut self.pending_markets {
            if pm.name == name {
                f(&mut pm.params);
                return self;
            }
        }
        panic!("market '{}' not found -- call .with_market() first", name);
    }

    pub fn with_position_limits(mut self, max_supply: u32, max_borrow: u32) -> Self {
        self.position_limits = Some((max_supply, max_borrow));
        self
    }

    pub fn with_dust_disabled_all_markets(mut self) -> Self {
        for pm in &mut self.pending_markets {
            pm.config.min_collat_floor_usd_wad = 0;
            pm.config.min_debt_floor_usd_wad = 0;
        }
        self
    }

    pub fn with_max_utilization_disabled_all_markets(mut self) -> Self {
        for pm in &mut self.pending_markets {
            pm.params.max_utilization_ray = controller::constants::RAY;
        }
        self
    }

    pub fn with_budget_enabled(mut self) -> Self {
        self.budget_enabled = true;
        self
    }

    pub fn without_auto_auth(mut self) -> Self {
        self.skip_mock_auths = true;
        self
    }

    pub fn with_emode(mut self, category_id: u32, preset: EModeCategoryPreset) -> Self {
        self.pending_emodes.push(PendingEMode {
            category_id,
            preset,
            assets: Vec::new(),
        });
        self
    }

    pub fn with_emode_asset(
        mut self,
        category_id: u32,
        asset_name: &str,
        can_collateral: bool,
        can_borrow: bool,
    ) -> Self {
        for emode in &mut self.pending_emodes {
            if emode.category_id == category_id {
                emode
                    .assets
                    .push((asset_name.to_string(), can_collateral, can_borrow));
                return self;
            }
        }
        panic!(
            "e-mode category {} not found -- call .with_emode() first",
            category_id
        );
    }

    pub fn build(self) -> LendingTest {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        if !self.budget_enabled {
            env.cost_estimate().budget().reset_unlimited();
            env.cost_estimate().disable_resource_limits();
        }

        env.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 26,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3_110_400,
        });

        let admin = Address::generate(&env);

        let mock_reflector_address = env.register(crate::mock_reflector::MockReflector, ());

        let aggregator_address =
            env.register(crate::mock_aggregator::MockAggregator, (admin.clone(),));

        let controller_address = env.register(controller::Controller, (admin.clone(),));
        let ctrl = controller::ControllerClient::new(&env, &controller_address);

        ctrl.unpause();
        ctrl.grant_role(&admin, &Symbol::new(&env, "REVENUE"));
        ctrl.grant_role(&admin, &Symbol::new(&env, "ORACLE"));

        let pool_wasm_path = "target/wasm32v1-none/release/pool.wasm".to_string();
        let mut bytes = std::fs::read(&pool_wasm_path);
        if bytes.is_err() {
            bytes = std::fs::read(format!("../{}", pool_wasm_path));
        }
        if bytes.is_err() {
            bytes = std::fs::read(format!("../../{}", pool_wasm_path));
        }

        let pool_hash = match bytes {
            Ok(b) => env
                .deployer()
                .upload_contract_wasm(soroban_sdk::Bytes::from_slice(&env, &b)),
            Err(_) => panic!("Liquidity pool WASM not found. Run 'make build' first."),
        };
        ctrl.set_liquidity_pool_template(&pool_hash);

        let global_pool = ctrl.deploy_pool();

        ctrl.set_aggregator(&aggregator_address);

        let accumulator = aggregator_address.clone();
        ctrl.set_accumulator(&accumulator);

        let keeper = Address::generate(&env);
        ctrl.grant_role(&keeper, &Symbol::new(&env, "KEEPER"));

        if let Some((max_supply, max_borrow)) = self.position_limits {
            let limits = controller::types::PositionLimits {
                max_supply_positions: max_supply,
                max_borrow_positions: max_borrow,
            };
            ctrl.set_position_limits(&limits);
        }

        let mock_reflector_client =
            crate::mock_reflector::MockReflectorClient::new(&env, &mock_reflector_address);

        let mut markets = HashMap::new();

        for pm in &self.pending_markets {
            let asset_address = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
            let token_admin = token::StellarAssetClient::new(&env, &asset_address);

            let market_params = pm.params.to_market_params(&asset_address, pm.decimals);
            let asset_config = pm.config.to_asset_config(&env);
            ctrl.approve_token(&asset_address);
            let pool_address =
                ctrl.create_liquidity_pool(&asset_address, &market_params, &asset_config);
            assert_eq!(
                pool_address, global_pool,
                "create_liquidity_pool must return the global pool address"
            );

            if pm.configure_oracle {
                mock_reflector_client.set_price(&asset_address, &pm.price_wad);

                let oracle_cfg = crate::oracle::config::reflector_primary_anchor_config(
                    &mock_reflector_address,
                    &asset_address,
                    DEFAULT_TOLERANCE.first_upper_bps,
                    DEFAULT_TOLERANCE.last_upper_bps,
                );
                ctrl.configure_market_oracle(&admin, &asset_address, &oracle_cfg);
            }

            let liquidity_amount = f64_to_i128(pm.initial_liquidity, pm.decimals);
            token_admin.mint(&pool_address, &liquidity_amount);

            env.as_contract(&pool_address, || {
                let key = controller::types::PoolKey::State(asset_address.clone());
                let mut state: controller::types::PoolStateRaw =
                    env.storage().persistent().get(&key).unwrap();
                state.cash += liquidity_amount;
                env.storage().persistent().set(&key, &state);
            });

            markets.insert(
                pm.name.to_string(),
                MarketState {
                    asset: asset_address,
                    pool: pool_address,
                    token_admin,
                    decimals: pm.decimals,
                    price_wad: pm.price_wad,
                },
            );
        }

        for emode in &self.pending_emodes {
            let _id = ctrl.add_e_mode_category(
                &emode.preset.ltv,
                &emode.preset.threshold,
                &emode.preset.bonus,
            );

            for (asset_name, can_collateral, can_borrow) in &emode.assets {
                let asset_addr = markets
                    .get(asset_name.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "e-mode asset '{}' not found -- add it with .with_market() first",
                            asset_name
                        )
                    })
                    .asset
                    .clone();
                ctrl.add_asset_to_e_mode_category(
                    &asset_addr,
                    &emode.category_id,
                    can_collateral,
                    can_borrow,
                );
            }
        }

        if self.skip_mock_auths {
            env.set_auths(&[]);
        }

        LendingTest {
            env,
            admin,
            controller: controller_address,
            mock_reflector: mock_reflector_address,
            aggregator: aggregator_address,
            keeper,
            users: HashMap::new(),
            markets,
        }
    }
}
