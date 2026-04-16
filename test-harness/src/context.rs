extern crate std;

use std::collections::HashMap;

use common::errors::GenericError;
use common::types::{ControllerKey, PositionMode};
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{token, Address, Env, Symbol};

use crate::helpers::f64_to_i128;
use crate::presets::{
    AssetConfigPreset, EModeCategoryPreset, MarketParamsPreset, MarketPreset, DEFAULT_TOLERANCE,
};

// ---------------------------------------------------------------------------
// Internal state types
// ---------------------------------------------------------------------------

pub struct UserState {
    pub address: Address,
    pub default_account_id: Option<u64>,
    pub accounts: Vec<AccountEntry>,
}

#[allow(dead_code)]
pub struct AccountEntry {
    pub account_id: u64,
    pub e_mode_category: u32,
    pub mode: PositionMode,
    pub is_isolated: bool,
}

pub struct MarketState {
    pub asset: Address,
    pub pool: Address,
    pub token_admin: token::StellarAssetClient<'static>,
    pub decimals: u32,
    pub price_wad: i128,
}

// ---------------------------------------------------------------------------
// Builder types (pre-build configuration)
// ---------------------------------------------------------------------------

struct PendingMarket {
    name: &'static str,
    decimals: u32,
    price_wad: i128,
    initial_liquidity: f64,
    config: AssetConfigPreset,
    params: MarketParamsPreset,
    configure_oracle: bool,
}

impl PendingMarket {
    fn from_preset(preset: MarketPreset) -> Self {
        PendingMarket {
            name: preset.name,
            decimals: preset.decimals,
            price_wad: preset.price_wad,
            initial_liquidity: preset.initial_liquidity,
            config: preset.config,
            params: preset.params,
            configure_oracle: true,
        }
    }
}

struct PendingEMode {
    category_id: u32,
    preset: EModeCategoryPreset,
    assets: Vec<(String, bool, bool)>, // (asset_name, can_collateral, can_borrow)
}

// ---------------------------------------------------------------------------
// LendingTest
// ---------------------------------------------------------------------------

pub struct LendingTest {
    pub env: Env,
    pub admin: Address,
    pub controller: Address,
    pub mock_reflector: Address,
    #[allow(dead_code)]
    pub aggregator: Address,
    pub keeper: Address,
    pub users: HashMap<String, UserState>,
    pub markets: HashMap<String, MarketState>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

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

    // Helper: resolve user name -> address, creating if needed
    pub fn get_or_create_user(&mut self, name: &str) -> Address {
        if let Some(user) = self.users.get(name) {
            return user.address.clone();
        }
        let address = Address::generate(&self.env);
        self.users.insert(
            name.to_string(),
            UserState {
                address: address.clone(),
                default_account_id: None,
                accounts: Vec::new(),
            },
        );
        address
    }

    pub fn find_account_id(&self, name: &str) -> Option<u64> {
        let user = self.users.get(name).unwrap_or_else(|| {
            panic!(
                "user '{}' not found -- supply or create_account first",
                name
            )
        });
        user.default_account_id
            .filter(|id| self.account_exists(*id))
            .or_else(|| {
                user.accounts.iter().find_map(|account| {
                    self.account_exists(account.account_id)
                        .then_some(account.account_id)
                })
            })
    }

    // Helper: resolve user name -> default account id, panic if ambiguous
    pub fn resolve_account_id(&self, name: &str) -> u64 {
        match self.find_account_id(name) {
            Some(id) => id,
            None => panic!(
                "'{}' has no account -- call supply() or create_account() first",
                name
            ),
        }
    }

    pub fn try_resolve_account_id(&self, name: &str) -> Result<u64, soroban_sdk::Error> {
        self.find_account_id(name).ok_or_else(|| {
            soroban_sdk::Error::from_contract_error(GenericError::AccountNotInMarket as u32)
        })
    }

    // Helper: resolve asset name -> MarketState, panic if not found
    pub fn resolve_market(&self, asset_name: &str) -> &MarketState {
        self.markets.get(asset_name).unwrap_or_else(|| {
            panic!(
                "market '{}' not found -- add it with .with_market()",
                asset_name
            )
        })
    }

    pub fn resolve_market_by_asset(&self, asset: &Address) -> &MarketState {
        self.markets
            .values()
            .find(|market| market.asset == *asset)
            .unwrap_or_else(|| panic!("market for asset '{:?}' not found", asset))
    }

    // Helper: resolve asset name -> Address
    pub fn resolve_asset(&self, asset_name: &str) -> Address {
        self.resolve_market(asset_name).asset.clone()
    }

    // Helper: get controller client
    pub fn ctrl_client(&self) -> controller::ControllerClient<'_> {
        controller::ControllerClient::new(&self.env, &self.controller)
    }

    // Helper: get mock reflector client
    pub fn mock_reflector_client(&self) -> crate::mock_reflector::MockReflectorClient<'_> {
        crate::mock_reflector::MockReflectorClient::new(&self.env, &self.mock_reflector)
    }

    pub fn account_exists(&self, account_id: u64) -> bool {
        self.env.as_contract(&self.controller, || {
            self.env
                .storage()
                .persistent()
                .has(&ControllerKey::AccountMeta(account_id))
        })
    }

    pub fn default_account_id_or_zero(&self, user: &str) -> u64 {
        self.find_account_id(user).unwrap_or(0)
    }

    // Helper: get pool client for an asset
    #[allow(dead_code)]
    pub fn pool_client(&self, asset_name: &str) -> pool::LiquidityPoolClient<'_> {
        let market = self.resolve_market(asset_name);
        pool::LiquidityPoolClient::new(&self.env, &market.pool)
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

    /// Opt in to Soroban's default budget and resource limits.
    ///
    /// By default `build()` calls `reset_unlimited()` + `disable_resource_limits()`
    /// so correctness tests aren't bounded by cost-model ceilings. Fuzz harnesses
    /// that explicitly want to exercise the metering ceiling (e.g.
    /// `fuzz_budget_metering.rs`) flip this on.
    pub fn with_budget_enabled(mut self) -> Self {
        self.budget_enabled = true;
        self
    }

    /// Opt out of `env.mock_all_auths()`.
    ///
    /// Most tests rely on the blanket auth mock to avoid boilerplate. Tests
    /// that exercise nested contract-to-contract auth (flash-loan receivers
    /// that mint SAC tokens, multi-hop strategy swaps, etc.) should call this
    /// so they can attach explicit `MockAuth` trees per call via
    /// `ctrl.mock_auths(&[...])`. Default behavior is unchanged for every
    /// existing test.
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
        // Always mock auths during the setup phase (upload pool WASM, register
        // markets, grant roles, configure oracles, mint initial liquidity).
        // If the caller asked to opt out via `without_auto_auth()`, we revert
        // to strict auth-checking at the very end of build(), so the test
        // itself starts from a clean slate.
        env.mock_all_auths();
        // Remove test budget and resource limits so these tests focus on
        // business logic rather than execution ceilings. Opt-in flag
        // `with_budget_enabled()` keeps Soroban's defaults in place so
        // budget/metering fuzz harnesses can catch cost-model regressions.
        if !self.budget_enabled {
            env.cost_estimate().budget().reset_unlimited();
            env.cost_estimate().disable_resource_limits();
        }

        // Set ledger info
        env.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3_110_400,
        });

        let admin = Address::generate(&env);

        // Deploy mock reflector oracle
        let mock_reflector_address = env.register(crate::mock_reflector::MockReflector, ());

        // Deploy mock aggregator
        let aggregator_address =
            env.register(crate::mock_aggregator::MockAggregator, (admin.clone(),));

        // Deploy controller
        let controller_address = env.register(controller::Controller, (admin.clone(),));
        let ctrl = controller::ControllerClient::new(&env, &controller_address);

        // Upload and set pool template
        let pool_wasm_path = "target/wasm32v1-none/release/pool.wasm".to_string();
        // Since tests run from various workspace roots, we try a few relative paths
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

        // Set aggregator in controller
        ctrl.set_aggregator(&aggregator_address);

        // Create keeper and grant KEEPER role
        let keeper = Address::generate(&env);
        ctrl.grant_role(&keeper, &Symbol::new(&env, "KEEPER"));

        // Set position limits if configured
        if let Some((max_supply, max_borrow)) = self.position_limits {
            let limits = common::types::PositionLimits {
                max_supply_positions: max_supply,
                max_borrow_positions: max_borrow,
            };
            ctrl.set_position_limits(&limits);
        }

        let mock_reflector_client =
            crate::mock_reflector::MockReflectorClient::new(&env, &mock_reflector_address);

        // Deploy markets
        let mut markets = HashMap::new();

        for pm in &self.pending_markets {
            // Deploy token
            let asset_address = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
            let token_admin = token::StellarAssetClient::new(&env, &asset_address);

            // 1. First, deploy the pool and initialize the market (Step 1: creates the MarketConfig in PendingOracle state)
            let market_params = pm.params.to_market_params(&asset_address, pm.decimals);
            let asset_config = pm.config.to_asset_config();
            // Pre-approve the token contract — the controller's allow-list gate
            // (T1-7) now requires explicit admin approval before market creation.
            ctrl.approve_token_wasm(&asset_address);
            let pool_address =
                ctrl.create_liquidity_pool(&asset_address, &market_params, &asset_config);

            if pm.configure_oracle {
                // 2. Configure the market oracle in one shot.
                mock_reflector_client.set_price(&asset_address, &pm.price_wad);

                let oracle_cfg = common::types::MarketOracleConfigInput {
                    exchange_source: common::types::ExchangeSource::SpotVsTwap,
                    max_price_stale_seconds: 900,
                    first_tolerance_bps: DEFAULT_TOLERANCE.first_upper_bps,
                    last_tolerance_bps: DEFAULT_TOLERANCE.last_upper_bps,
                    cex_oracle: mock_reflector_address.clone(),
                    cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
                    cex_symbol: Symbol::new(&env, ""),
                    dex_oracle: None,
                    dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
                    twap_records: 3,
                };
                ctrl.configure_market_oracle(&admin, &asset_address, &oracle_cfg);
            }

            // Mint initial liquidity to pool
            let liquidity_amount = f64_to_i128(pm.initial_liquidity, pm.decimals);
            token_admin.mint(&pool_address, &liquidity_amount);

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

        // Deploy e-mode categories
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

        // If the caller opted out of auto-auth, clear the blanket auth mock
        // now that setup is done. Tests can re-enable it locally (via
        // `t.env.mock_all_auths()`) or attach per-call `MockAuth` trees.
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
