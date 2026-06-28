//! Multi-hub test helpers.
//!
//! Existing single-hub helpers operate on the base harness hub; these add the
//! `hub_id`-parameterized variants used by the isolation suite: creating a hub,
//! listing an already-registered asset on a second hub, supplying/borrowing on a
//! specific `(hub_id, asset)`, and reading a hub-scoped pool `State`.

use common::types::HubAssetKey;
use controller::types::{MarketParamsRaw, PoolKey, PoolStateRaw, PositionMode, SpokeAssetConfig};
use governance::op::{AdminOperation, CreatePoolArgs};
use soroban_sdk::{token, vec, TryFromVal, Vec};

use crate::core::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset};

impl LendingTest {
    /// Creates a new hub through governance and returns its id. Hub ids start at
    /// 1; the base setup already owns the harness hub, so the first extra hub
    /// created here is 2.
    pub fn create_hub(&self) -> u32 {
        let id_val = self
            .gov_client()
            .execute_immediate(&self.admin, &AdminOperation::CreateHub);
        u32::try_from_val(&self.env, &id_val).expect("create_hub returns a hub id")
    }

    /// Lists an already-registered market's asset on `hub_id` (distinct from the
    /// base harness hub), reusing the base hub's params/config and seeding
    /// `initial_liquidity` of cash. The asset oracle is token-rooted, so the base
    /// hub listing already configured it.
    pub fn list_market_on_hub(&mut self, hub_id: u32, asset_name: &str, initial_liquidity: f64) {
        let market = self.resolve_market(asset_name);
        let asset = market.asset.clone();
        let pool = market.pool.clone();
        let decimals = market.decimals;

        // Reuse the base hub's params/config so the new hub market is valid.
        let params: MarketParamsRaw = self.env.as_contract(&pool, || {
            self.env
                .storage()
                .persistent()
                .get(&PoolKey::Params(hub_asset(asset.clone())))
                .expect("base hub params must exist")
        });
        let config: SpokeAssetConfig = self
            .ctrl_client()
            .get_spoke_asset(&0u32, &hub_asset(asset.clone()));

        let gov = self.gov_client();
        gov.execute_immediate(&self.admin, &AdminOperation::ApproveToken(asset.clone()));
        gov.execute_immediate(
            &self.admin,
            &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                hub_id,
                asset: asset.clone(),
                params,
                config,
            }),
        );

        // Seed cash directly, mirroring the builder's base hub liquidity seed.
        let liquidity = f64_to_i128(initial_liquidity, decimals);
        token::StellarAssetClient::new(&self.env, &asset).mint(&pool, &liquidity);
        self.env.as_contract(&pool, || {
            let key = PoolKey::State(HubAssetKey {
                hub_id,
                asset: asset.clone(),
            });
            let mut state: PoolStateRaw = self
                .env
                .storage()
                .persistent()
                .get(&key)
                .expect("hub market state exists after create_market");
            state.cash += liquidity;
            self.env.storage().persistent().set(&key, &state);
        });
    }

    /// Lists an already-registered market on `hub_id` (distinct from the base
    /// harness hub) with an explicit `liquidation_fees_bps`, overriding the base
    /// hub config. Used to prove the liquidation seizure resolves the protocol
    /// fee from the position's own hub.
    pub fn list_market_on_hub_with_fees(
        &mut self,
        hub_id: u32,
        asset_name: &str,
        initial_liquidity: f64,
        liquidation_fees_bps: u32,
    ) {
        let market = self.resolve_market(asset_name);
        let asset = market.asset.clone();
        let pool = market.pool.clone();
        let decimals = market.decimals;

        let params: MarketParamsRaw = self.env.as_contract(&pool, || {
            self.env
                .storage()
                .persistent()
                .get(&PoolKey::Params(hub_asset(asset.clone())))
                .expect("base hub params must exist")
        });
        let mut config: SpokeAssetConfig = self
            .ctrl_client()
            .get_spoke_asset(&0u32, &hub_asset(asset.clone()));
        config.liquidation_fees_bps = liquidation_fees_bps;

        let gov = self.gov_client();
        gov.execute_immediate(&self.admin, &AdminOperation::ApproveToken(asset.clone()));
        gov.execute_immediate(
            &self.admin,
            &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                hub_id,
                asset: asset.clone(),
                params,
                config,
            }),
        );

        let liquidity = f64_to_i128(initial_liquidity, decimals);
        token::StellarAssetClient::new(&self.env, &asset).mint(&pool, &liquidity);
        self.env.as_contract(&pool, || {
            let key = PoolKey::State(HubAssetKey {
                hub_id,
                asset: asset.clone(),
            });
            let mut state: PoolStateRaw = self
                .env
                .storage()
                .persistent()
                .get(&key)
                .expect("hub market state exists after create_market");
            state.cash += liquidity;
            self.env.storage().persistent().set(&key, &state);
        });
    }

    /// Supplies `amount` of `asset_name` on `hub_id`. Mints to the user, creates
    /// the account on first call, registers it, and returns the account id.
    pub fn supply_on_hub(
        &mut self,
        hub_id: u32,
        user: &str,
        asset_name: &str,
        amount: f64,
    ) -> u64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.get_or_create_user(user);
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();
        market.token_admin.mint(&addr, &raw_amount);

        let account_id = self.default_account_id_or_zero(user);

        let ctrl = self.ctrl_client();
        let assets: Vec<(HubAssetKey, i128)> = vec![
            &self.env,
            (
                HubAssetKey {
                    hub_id,
                    asset: asset_addr,
                },
                raw_amount,
            ),
        ];
        let returned_id = ctrl.supply(&addr, &account_id, &0u32, &assets);

        if account_id == 0 {
            self.register_account(user, returned_id, 0, PositionMode::Normal);
        }
        returned_id
    }

    /// Borrows `amount` of `asset_name` on `hub_id` for `account_id`.
    pub fn borrow_on_hub(
        &mut self,
        hub_id: u32,
        user: &str,
        account_id: u64,
        asset_name: &str,
        amount: f64,
    ) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.users.get(user).expect("user must exist").address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let borrows: Vec<(HubAssetKey, i128)> = vec![
            &self.env,
            (
                HubAssetKey {
                    hub_id,
                    asset: asset_addr,
                },
                raw_amount,
            ),
        ];
        ctrl.borrow(&addr, &account_id, &borrows, &None);
    }

    /// Try-borrow on `hub_id`; returns the contract error instead of panicking.
    pub fn try_borrow_on_hub(
        &mut self,
        hub_id: u32,
        user: &str,
        account_id: u64,
        asset_name: &str,
        amount: f64,
    ) -> Result<(), soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.users.get(user).expect("user must exist").address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let borrows: Vec<(HubAssetKey, i128)> = vec![
            &self.env,
            (
                HubAssetKey {
                    hub_id,
                    asset: asset_addr,
                },
                raw_amount,
            ),
        ];
        match ctrl.try_borrow(&addr, &account_id, &borrows, &None) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Accrues a single hub market's indexes by calling the pool's hub-aware
    /// `update_indexes` directly, bypassing the controller keeper verb.
    pub fn accrue_on_hub(&self, hub_id: u32, asset_name: &str) {
        let market = self.resolve_market(asset_name);
        let pool = market.pool.clone();
        let hub_asset = HubAssetKey {
            hub_id,
            asset: market.asset.clone(),
        };
        pool::LiquidityPoolClient::new(&self.env, &pool).update_indexes(&hub_asset);
    }

    /// Accrues each `(hub_id, asset)` market's indexes through the controller's
    /// hub-aware `update_indexes` keeper verb (the production keeper path).
    pub fn update_indexes_on_hub(&self, hub_id: u32, asset_names: &[&str]) {
        let mut hub_assets = Vec::new(&self.env);
        for name in asset_names {
            hub_assets.push_back(HubAssetKey {
                hub_id,
                asset: self.resolve_asset(name),
            });
        }
        self.ctrl_client().update_indexes(&self.keeper, &hub_assets);
    }

    /// Claims protocol revenue for a single `(hub_id, asset)` market through the
    /// controller's hub-aware `claim_revenue` verb; returns the claimed amount.
    pub fn claim_revenue_on_hub(&self, hub_id: u32, asset_name: &str) -> i128 {
        let hub_asset = HubAssetKey {
            hub_id,
            asset: self.resolve_asset(asset_name),
        };
        let assets = vec![&self.env, hub_asset];
        self.ctrl_client()
            .claim_revenue(&self.admin, &assets)
            .get(0)
            .unwrap()
    }

    /// Reads the raw pool `State` for `(hub_id, asset_name)`.
    pub fn pool_state_on_hub(&self, hub_id: u32, asset_name: &str) -> PoolStateRaw {
        let market = self.resolve_market(asset_name);
        let asset = market.asset.clone();
        let pool = market.pool.clone();
        self.env.as_contract(&pool, || {
            self.env
                .storage()
                .persistent()
                .get(&PoolKey::State(HubAssetKey { hub_id, asset }))
                .expect("pool state must exist for the hub market")
        })
    }
}
