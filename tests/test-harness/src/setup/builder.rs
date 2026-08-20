extern crate std;

use std::collections::HashMap;

use governance::op::{AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, SpokeAssetArgs};
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{token, Address, Env, TryFromVal};

use crate::core::types::{LendingTest, MarketState, PendingMarket, PendingSpoke};
use crate::helpers::{f64_to_i128, hub_asset, HARNESS_HUB, HARNESS_SPOKE};
use crate::presets::{
    unconstrained_test_cap, AssetConfigPreset, MarketParamsPreset, MarketPreset, SpokePreset,
    DEFAULT_TOLERANCE,
};

pub struct LendingTestBuilder {
    pending_markets: Vec<PendingMarket>,
    pending_spokes: Vec<PendingSpoke>,
    position_limits: Option<(u32, u32)>,
    min_borrow_collateral_usd_wad: Option<i128>,
    budget_enabled: bool,
}

impl LendingTest {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> LendingTestBuilder {
        LendingTestBuilder {
            pending_markets: Vec::new(),
            pending_spokes: Vec::new(),
            position_limits: None,
            min_borrow_collateral_usd_wad: None,
            budget_enabled: false,
        }
    }
}

impl LendingTestBuilder {
    pub fn with_market(mut self, preset: MarketPreset) -> Self {
        self.pending_markets
            .push(PendingMarket::from_preset(preset));
        self
    }

    pub fn with_freezable_market(mut self, preset: MarketPreset) -> Self {
        let mut pending = PendingMarket::from_preset(preset);
        pending.freezable = true;
        self.pending_markets.push(pending);
        self
    }

    pub fn with_fee_on_transfer_market(mut self, preset: MarketPreset, bps: i128) -> Self {
        let mut pending = PendingMarket::from_preset(preset);
        pending.shortfall_bps = bps;
        self.pending_markets.push(pending);
        self
    }

    pub fn with_extra_credit_market(mut self, preset: MarketPreset, bps: i128) -> Self {
        let mut pending = PendingMarket::from_preset(preset);
        pending.extra_bps = bps;
        self.pending_markets.push(pending);
        self
    }

    pub fn with_transfer_hook_market(mut self, preset: MarketPreset) -> Self {
        let mut pending = PendingMarket::from_preset(preset);
        pending.transfer_hook = true;
        self.pending_markets.push(pending);
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

    pub fn with_min_borrow_collateral_disabled(mut self) -> Self {
        self.min_borrow_collateral_usd_wad = Some(0);
        self
    }

    pub fn with_dust_disabled_all_markets(self) -> Self {
        self.with_min_borrow_collateral_disabled()
    }

    pub fn with_max_utilization_disabled_all_markets(mut self) -> Self {
        for pm in &mut self.pending_markets {
            pm.params.max_utilization = controller::constants::RAY;
        }
        self
    }

    pub fn with_budget_enabled(mut self) -> Self {
        self.budget_enabled = true;
        self
    }

    pub fn with_spoke(mut self, category_id: u32, preset: SpokePreset) -> Self {
        self.pending_spokes.push(PendingSpoke {
            category_id,
            preset,
            assets: Vec::new(),
        });
        self
    }

    pub fn with_spoke_asset(
        mut self,
        category_id: u32,
        asset_name: &str,
        can_collateral: bool,
        can_borrow: bool,
    ) -> Self {
        for spoke in &mut self.pending_spokes {
            if spoke.category_id == category_id {
                spoke
                    .assets
                    .push((asset_name.to_string(), can_collateral, can_borrow));
                return self;
            }
        }
        panic!(
            "spoke category {} not found -- call .with_spoke() first",
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

        const HARNESS_TIMELOCK_MIN_DELAY_LEDGERS: u32 = 50;
        let governance_address = env.register(
            governance::Governance,
            (admin.clone(), HARNESS_TIMELOCK_MIN_DELAY_LEDGERS),
        );
        let gov = governance::GovernanceClient::new(&env, &governance_address);

        let price_aggregator_address = env.register(
            price_aggregator::PriceAggregator,
            (governance_address.clone(),),
        );
        gov.set_price_aggregator(&price_aggregator_address);

        let controller_address = env.register(controller::Controller, (admin.clone(),));
        gov.set_controller(&controller_address);
        gov.execute_immediate(
            &admin,
            &AdminOperation::SetPriceAggregator(price_aggregator_address.clone()),
        );

        controller::ControllerClient::new(&env, &controller_address).unpause();

        let pool_wasm_path = std::env::var("POOL_WASM_PATH")
            .unwrap_or_else(|_| "target/wasm32v1-none/release/pool.wasm".to_string());
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
        let global_pool_val = gov.execute_immediate(&admin, &AdminOperation::DeployPool(pool_hash));
        let global_pool: Address = Address::try_from_val(&env, &global_pool_val).unwrap();

        let nft_wasm_path = std::env::var("POSITION_NFT_WASM_PATH")
            .unwrap_or_else(|_| "target/wasm32v1-none/release/position_nft.wasm".to_string());
        let mut nft_bytes = std::fs::read(&nft_wasm_path);
        if nft_bytes.is_err() {
            nft_bytes = std::fs::read(format!("../{}", nft_wasm_path));
        }
        if nft_bytes.is_err() {
            nft_bytes = std::fs::read(format!("../../{}", nft_wasm_path));
        }
        let nft_hash = match nft_bytes {
            Ok(b) => env
                .deployer()
                .upload_contract_wasm(soroban_sdk::Bytes::from_slice(&env, &b)),
            Err(_) => panic!("position-nft WASM not found. Run 'make build' first."),
        };
        let ctrl = controller::ControllerClient::new(&env, &controller_address);
        let position_nft = ctrl.deploy_position_nft(
            &nft_hash,
            &soroban_sdk::String::from_str(&env, "https://test/nft/"),
            &soroban_sdk::String::from_str(&env, "Test Position"),
            &soroban_sdk::String::from_str(&env, "TPOS"),
        );

        gov.execute_immediate(
            &admin,
            &AdminOperation::SetSwapAggregator(aggregator_address.clone()),
        );

        let treasury = Address::generate(&env);
        gov.execute_immediate(&admin, &AdminOperation::SetAccumulator(treasury.clone()));

        let keeper = Address::generate(&env);

        if let Some((max_supply, max_borrow)) = self.position_limits {
            let limits = controller::types::PositionLimits {
                max_supply_positions: max_supply,
                max_borrow_positions: max_borrow,
            };
            gov.execute_immediate(&admin, &AdminOperation::SetPositionLimits(limits));
        }

        if let Some(floor_wad) = self.min_borrow_collateral_usd_wad {
            gov.execute_immediate(
                &admin,
                &AdminOperation::SetMinBorrowCollateralUsd(floor_wad),
            );
        }

        let mock_reflector_client =
            crate::mock_reflector::MockReflectorClient::new(&env, &mock_reflector_address);

        let base_hub_val = gov.execute_immediate(&admin, &AdminOperation::CreateHub);
        let base_hub: u32 = u32::try_from_val(&env, &base_hub_val).unwrap();
        assert_eq!(
            base_hub, HARNESS_HUB,
            "the base setup hub must be the harness hub id"
        );

        let base_spoke_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
        let base_spoke: u32 = u32::try_from_val(&env, &base_spoke_val).unwrap();
        assert_eq!(
            base_spoke, HARNESS_SPOKE,
            "the base setup spoke must be the harness spoke id"
        );

        let mut markets = HashMap::new();

        for pm in &self.pending_markets {
            let needs_mock_token = pm.freezable
                || pm.decimals != 7
                || pm.shortfall_bps != 0
                || pm.extra_bps != 0
                || pm.transfer_hook;
            let asset_address = if pm.shortfall_bps != 0 || pm.extra_bps != 0 || pm.transfer_hook {
                let addr = env.register(crate::weird_token::WeirdToken, ());
                let client = crate::weird_token::WeirdTokenClient::new(&env, &addr);
                client.set_decimals(&pm.decimals);
                if pm.shortfall_bps != 0 {
                    client.set_shortfall_bps(&pm.shortfall_bps);
                }
                if pm.extra_bps != 0 {
                    client.set_extra_bps(&pm.extra_bps);
                }
                if pm.transfer_hook {
                    client.set_hook(&controller_address);
                }
                addr
            } else if needs_mock_token {
                let addr = env.register(crate::freezable_token::FreezableToken, ());
                crate::freezable_token::FreezableTokenClient::new(&env, &addr)
                    .set_decimals(&pm.decimals);
                addr
            } else {
                env.register_stellar_asset_contract_v2(admin.clone())
                    .address()
                    .clone()
            };
            let token_admin = token::StellarAssetClient::new(&env, &asset_address);

            let token_decimals = token::Client::new(&env, &asset_address).decimals();
            assert_eq!(
                token_decimals, pm.decimals,
                "market '{}' declares {} decimals but its token reports {}",
                pm.name, pm.decimals, token_decimals
            );
            let market_decimals = token_decimals;

            let mut market_params = pm.params.to_market_params(&asset_address, market_decimals);

            market_params.is_flashloanable = pm.config.is_flashloanable;
            market_params.flashloan_fee = pm.config.flashloan_fee;
            let pool_address_val = gov.execute_immediate(
                &admin,
                &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                    hub_id: HARNESS_HUB,
                    asset: asset_address.clone(),
                    params: market_params,
                }),
            );
            let pool_address: Address = Address::try_from_val(&env, &pool_address_val).unwrap();
            assert_eq!(
                pool_address, global_pool,
                "create_liquidity_pool must return the global pool address"
            );

            gov.execute_immediate(
                &admin,
                &AdminOperation::AddAssetToSpoke(pm.config.to_spoke_args(
                    HARNESS_HUB,
                    asset_address.clone(),
                    HARNESS_SPOKE,
                    market_decimals,
                )),
            );

            if pm.configure_oracle {
                mock_reflector_client.set_price(&asset_address, &pm.price_wad);

                let oracle = crate::oracle::config::reflector_primary_anchor_config(
                    &env,
                    &mock_reflector_address,
                    &asset_address,
                    pm.price_wad,
                    DEFAULT_TOLERANCE.tolerance_bps,
                );
                gov.execute_immediate(
                    &admin,
                    &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                        key: controller::types::PriceKey::Token(asset_address.clone()),
                        oracle,
                    }),
                );
            }

            let liquidity_amount = f64_to_i128(pm.initial_liquidity, market_decimals);
            if pm.shortfall_bps != 0 || pm.extra_bps != 0 || pm.transfer_hook {
                crate::weird_token::WeirdTokenClient::new(&env, &asset_address)
                    .mint(&pool_address, &liquidity_amount);
            } else {
                token_admin.mint(&pool_address, &liquidity_amount);
            }

            env.as_contract(&pool_address, || {
                let key = controller::types::PoolKey::State(hub_asset(asset_address.clone()));
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
                    decimals: market_decimals,
                    price_wad: pm.price_wad,
                },
            );
        }

        for spoke in &self.pending_spokes {
            let id_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
            let id: u32 = u32::try_from_val(&env, &id_val).unwrap();

            assert_eq!(
                id, spoke.category_id,
                "spoke category id must match its created spoke id (base spoke is {HARNESS_SPOKE}, spoke ids start at {})",
                HARNESS_SPOKE + 1
            );

            for (asset_name, can_collateral, can_borrow) in &spoke.assets {
                let market = markets.get(asset_name.as_str()).unwrap_or_else(|| {
                    panic!(
                        "spoke asset '{}' not found -- add it with .with_market() first",
                        asset_name
                    )
                });
                let asset_addr = market.asset.clone();
                // Cap must track the market's own decimals: a fixed base-unit
                // value is only meaningful at one scale.
                let cap = unconstrained_test_cap(market.decimals);
                gov.execute_immediate(
                    &admin,
                    &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
                        hub_id: HARNESS_HUB,
                        asset: asset_addr.clone(),
                        spoke_id: spoke.category_id,
                        can_collateral: *can_collateral,
                        can_borrow: *can_borrow,
                        paused: false,
                        frozen: false,
                        no_seize: false,
                        ltv: spoke.preset.ltv,
                        threshold: spoke.preset.threshold,
                        bonus: spoke.preset.bonus,
                        liquidation_fees: 0,
                        supply_cap: cap,
                        borrow_cap: cap,
                    }),
                );
            }
        }

        LendingTest {
            env,
            admin,
            governance: governance_address,
            controller: controller_address,
            mock_reflector: mock_reflector_address,
            aggregator: aggregator_address,
            price_aggregator: price_aggregator_address,
            keeper,
            position_nft,
            position_nft_wasm_hash: nft_hash,
            users: HashMap::new(),
            markets,
            blend_pool: None,
        }
    }
}
