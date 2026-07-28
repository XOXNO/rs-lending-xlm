#![no_std]
//! DeFindex vault adapter over the lending controller.
//!
//! Each vault address maps to exactly one controller account. Deposit and
//! withdraw supply and redeem collateral on that account; balance reads live
//! collateral. Harvest does not move funds — it publishes a 12-decimal
//! price-per-share derived from the market supply index (`RAY / 1e12`).
//!
//! Constructor binds one listed `HubAssetKey` and spoke. Persistent
//! `VaultAccount` entries extend TTL when read while live (~30d threshold,
//! ~180d extend-to). Full withdraw clears the mapping so the next deposit
//! opens a new account.

use common::constants::RAY;
use common::types::pool::HubAssetKey;

use controller_interface::ControllerClient;

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    vec, Address, Bytes, Env, IntoVal, Symbol, TryFromVal, Val, Vec,
};

/// Emitted by `harvest`. `amount` is always `0`; `price_per_share` is the
/// market supply index scaled to 12 decimals.
#[contractevent(topics = ["strategy", "harvest"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarvestEvent {
    pub from: Address,
    pub amount: i128,
    pub price_per_share: i128,
}

pub(crate) fn emit_harvest(e: &Env, from: Address, amount: i128, price_per_share: i128) {
    HarvestEvent {
        from,
        amount,
        price_per_share,
    }
    .publish(e);
}

/// Price-per-share scale: 12 decimals (`1e12`).
const PPS_SCALAR: i128 = 1_000_000_000_000;
// dimensional: D27{1} / D12{1} = D15{1} Ray-to-price-per-share divisor.
const RAY_PER_PPS: i128 = RAY / PPS_SCALAR;

/// Persistent `VaultAccount` TTL: extend when remaining ledgers fall below
/// ~30 days, up to ~180 days (`17_280` ledgers ≈ 1 day).
const VAULT_ACCOUNT_TTL_THRESHOLD: u32 = 17_280 * 30;
const VAULT_ACCOUNT_TTL_EXTEND_TO: u32 = 17_280 * 180;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeFindexStrategyError {
    /// Instance `Config` missing or constructor `init_args` malformed.
    NotInitialized = 401,
    /// Deposit or withdraw `amount` is not strictly positive.
    AmountNotPositive = 460,
    /// Withdraw against a missing account or for more than live collateral.
    InsufficientBalance = 461,
    /// Supply-index to price-per-share division failed.
    ArithmeticError = 462,
}

/// Instance configuration written by the constructor.
#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub hub_id: u32,
    pub spoke_id: u32,
    pub asset: Address,
    pub controller: Address,
    pub pool: Address,
}

#[contracttype]
pub enum DataKey {
    /// Instance `Config`.
    Config,
    /// Persistent vault address → controller account id (`0` means none).
    VaultAccount(Address),
}

/// DeFindex strategy surface implemented by [`Strategy`].
pub trait DeFindexStrategyTrait {
    /// Configured underlying asset.
    fn asset(env: Env) -> Result<Address, DeFindexStrategyError>;

    /// Pulls `amount` from `from` and supplies it into the vault account.
    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError>;

    /// Publishes supply-index price-per-share; does not move funds.
    fn harvest(env: Env, from: Address, data: Option<Bytes>) -> Result<(), DeFindexStrategyError>;

    /// Live underlying collateral for `from`'s vault account.
    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError>;

    /// Withdraws `amount` from the vault account to `to`.
    fn withdraw(
        env: Env,
        amount: i128,
        from: Address,
        to: Address,
    ) -> Result<i128, DeFindexStrategyError>;
}

#[contract]
pub struct Strategy;

struct Ctx<'a> {
    env: &'a Env,
    cfg: Config,
    controller: ControllerClient<'a>,
    strategy: Address,
}

impl<'a> Ctx<'a> {
    fn try_load(env: &'a Env) -> Result<Self, DeFindexStrategyError> {
        let cfg = config(env)?;
        Ok(Self {
            strategy: env.current_contract_address(),
            controller: ControllerClient::new(env, &cfg.controller),
            cfg,
            env,
        })
    }

    fn hub_asset(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: self.cfg.hub_id,
            asset: self.cfg.asset.clone(),
        }
    }

    fn collateral(&self, account_id: u64) -> i128 {
        // dimensional: controller reports live Token(asset), not scaled shares.
        self.controller
            .get_collateral_amount(&account_id, &self.hub_asset())
    }

    fn reconcile(&self, vault: &Address) -> u64 {
        reconcile_vault_account(self.env, &self.controller, vault)
    }

    fn vault_balance(&self, vault: &Address) -> i128 {
        let account_id = self.reconcile(vault);
        if account_id == 0 {
            return 0;
        }
        // dimensional: zero and collateral are D{AssetDecimals(asset)}{Token(asset)}.
        self.collateral(account_id)
    }

    fn harvest_price_per_share(&self) -> Result<i128, DeFindexStrategyError> {
        // dimensional: supply index is D27{Token(asset)/Share(asset, supply)}.
        let supply_index = self
            .controller
            .get_market_index(&self.hub_asset())
            .supply_index;
        // dimensional: D27{Token/Share} / D15{1} = D12{Token/Share}.
        supply_index
            .checked_div(RAY_PER_PPS)
            .ok_or(DeFindexStrategyError::ArithmeticError)
    }

    fn to_payment(&self, amount: i128) -> Vec<(HubAssetKey, i128)> {
        // dimensional: payment preserves D{AssetDecimals(asset)}{Token(asset)}.
        vec![self.env, (self.hub_asset(), amount)]
    }

    fn authorize_supply_to_pool(&self, amount: i128) {
        // dimensional: pool transfer amount is D{AssetDecimals(asset)}{Token(asset)}.
        self.env.authorize_as_current_contract(vec![
            self.env,
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: self.cfg.asset.clone(),
                    fn_name: Symbol::new(self.env, "transfer"),
                    args: (self.strategy.clone(), self.cfg.pool.clone(), amount).into_val(self.env),
                },
                sub_invocations: Vec::new(self.env),
            }),
        ]);
    }
}

#[contractimpl]
impl Strategy {
    /// Caches instance `Config` for one listed market: `asset`, `controller`,
    /// `hub_id`, `spoke_id`, and the controller's pool address.
    ///
    /// # Arguments
    /// * `asset` - underlying token for the strategy market.
    /// * `init_args` - `[controller, hub_id, spoke_id]` as `Val`s.
    ///
    /// # Errors
    /// * `NotInitialized` - missing `init_args` element or wrong element type.
    /// * Controller `get_market_index` reverts when `asset` is not listed for
    ///   `hub_id`.
    pub fn __constructor(env: Env, asset: Address, init_args: Vec<Val>) {
        let controller_val = init_args
            .get(0)
            .unwrap_or_else(|| panic_with_error!(&env, DeFindexStrategyError::NotInitialized));
        let controller = Address::try_from_val(&env, &controller_val)
            .unwrap_or_else(|_| panic_with_error!(&env, DeFindexStrategyError::NotInitialized));
        let hub_id_val = init_args
            .get(1)
            .unwrap_or_else(|| panic_with_error!(&env, DeFindexStrategyError::NotInitialized));
        let hub_id = u32::try_from_val(&env, &hub_id_val)
            .unwrap_or_else(|_| panic_with_error!(&env, DeFindexStrategyError::NotInitialized));
        let spoke_id_val = init_args
            .get(2)
            .unwrap_or_else(|| panic_with_error!(&env, DeFindexStrategyError::NotInitialized));
        let spoke_id = u32::try_from_val(&env, &spoke_id_val)
            .unwrap_or_else(|_| panic_with_error!(&env, DeFindexStrategyError::NotInitialized));

        let controller_client = ControllerClient::new(&env, &controller);
        let hub_asset = HubAssetKey {
            hub_id,
            asset: asset.clone(),
        };
        // Reverts if HubAssetKey is unlisted for hub_id.
        controller_client.get_market_index(&hub_asset);
        env.storage().instance().set(
            &DataKey::Config,
            &Config {
                hub_id,
                spoke_id,
                asset,
                controller,
                pool: controller_client.get_pool_address(),
            },
        );
    }
}

#[contractimpl]
impl DeFindexStrategyTrait for Strategy {
    /// Returns the configured underlying asset address.
    ///
    /// # Errors
    /// * `NotInitialized` - instance `Config` missing.
    fn asset(env: Env) -> Result<Address, DeFindexStrategyError> {
        Ok(config(&env)?.asset)
    }

    /// Transfers `amount` of the strategy asset from `from` into this contract,
    /// supplies it on the vault's controller account (opening one when none is
    /// live), stores the account id, and returns post-deposit collateral.
    ///
    /// # Arguments
    /// * `amount` - token amount to supply; must be `> 0`.
    /// * `from` - vault; must authorize.
    ///
    /// # Errors
    /// * `AmountNotPositive` - `amount <= 0`.
    /// * `NotInitialized` - instance `Config` missing.
    /// * Controller `supply` enforces its own market, spoke, cap, and pause
    ///   gates.
    ///
    /// # Notes
    /// * Stale vault mappings (stored id whose account no longer exists) are
    ///   cleared before supply so a fresh account can open.
    /// * Pool-transfer auth is installed immediately before `supply`; no
    ///   controller call may sit between auth and that invocation.
    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError> {
        if amount <= 0 {
            return Err(DeFindexStrategyError::AmountNotPositive);
        }
        from.require_auth();

        let ctx = Ctx::try_load(&env)?;

        // D{AssetDecimals(asset)}{Token(asset)}
        token::Client::new(&env, &ctx.cfg.asset).transfer(&from, &ctx.strategy, &amount);

        // Clear stale mapping before pool-transfer auth; auth covers the next
        // sub-invocation only, so no controller call may sit between auth and
        // supply.
        let stored_id = prepare_vault_account_for_supply(ctx.env, &ctx.controller, &from);
        ctx.authorize_supply_to_pool(amount);
        // dimensional: Token(asset) enters controller; supply shares are internal.
        let new_or_existing_id = ctx.controller.supply(
            &ctx.strategy,
            &stored_id,
            &ctx.cfg.spoke_id,
            &ctx.to_payment(amount),
        );
        set_vault_account(ctx.env, &from, new_or_existing_id);

        // D{AssetDecimals(asset)}{Token(asset)} post-deposit strategy balance.
        Ok(ctx.collateral(new_or_existing_id))
    }

    /// Requires `from` auth and emits `HarvestEvent` with `amount = 0` and the
    /// current supply-index price-per-share (12 decimals). Does not transfer
    /// tokens. `data` is ignored.
    ///
    /// # Arguments
    /// * `from` - caller; must authorize.
    /// * `data` - unused.
    ///
    /// # Errors
    /// * `NotInitialized` - instance `Config` missing.
    /// * `ArithmeticError` - supply-index / `RAY_PER_PPS` division fails.
    ///
    /// # Events
    /// * `HarvestEvent` - `from`, `amount = 0`, 12-decimal `price_per_share`.
    fn harvest(env: Env, from: Address, _data: Option<Bytes>) -> Result<(), DeFindexStrategyError> {
        from.require_auth();
        let ctx = Ctx::try_load(&env)?;
        emit_harvest(&env, from, 0, ctx.harvest_price_per_share()?);
        Ok(())
    }

    /// Returns live underlying collateral for `from`'s vault account, or `0`
    /// when none is mapped or the stored account no longer exists.
    ///
    /// # Errors
    /// * `NotInitialized` - instance `Config` missing.
    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError> {
        // dimensional: strategy balance is live D{AssetDecimals(asset)}{Token(asset)}.
        Ok(Ctx::try_load(&env)?.vault_balance(&from))
    }

    /// Withdraws `amount` of the strategy asset from `from`'s controller
    /// account to `to`. A full-balance withdraw uses the controller full-
    /// withdraw sentinel `0` and clears the vault mapping. Returns remaining
    /// collateral (or controller-reported collateral after a full exit).
    ///
    /// # Arguments
    /// * `amount` - token amount to withdraw; must be `> 0` and `<=` balance.
    /// * `from` - vault; must authorize.
    /// * `to` - recipient of withdrawn tokens.
    ///
    /// # Errors
    /// * `AmountNotPositive` - `amount <= 0`.
    /// * `NotInitialized` - instance `Config` missing.
    /// * `InsufficientBalance` - no live vault account or `amount > balance`.
    /// * Controller `withdraw` enforces its own gates.
    ///
    /// # Notes
    /// * Full exit clears `VaultAccount` immediately so the next deposit opens
    ///   a new controller account rather than reusing a closed id.
    fn withdraw(
        env: Env,
        amount: i128,
        from: Address,
        to: Address,
    ) -> Result<i128, DeFindexStrategyError> {
        if amount <= 0 {
            return Err(DeFindexStrategyError::AmountNotPositive);
        }
        from.require_auth();

        let ctx = Ctx::try_load(&env)?;
        let account_id = ctx.reconcile(&from);
        if account_id == 0 {
            return Err(DeFindexStrategyError::InsufficientBalance);
        }

        let balance = ctx.collateral(account_id);
        // dimensional: amount and balance are both Token(asset).
        if amount > balance {
            return Err(DeFindexStrategyError::InsufficientBalance);
        }

        // Controller full-withdraw sentinel is 0; public ABI already rejected amount <= 0.
        let is_full_withdraw = amount == balance;
        let withdraw_amount = if is_full_withdraw { 0 } else { amount };
        ctx.controller.withdraw(
            &ctx.strategy,
            &account_id,
            &ctx.to_payment(withdraw_amount),
            &Some(to),
        );

        if is_full_withdraw {
            clear_vault_account(ctx.env, &from);
        }

        // D{AssetDecimals(asset)}{Token(asset)} post-withdraw strategy balance.
        Ok(ctx.collateral(account_id))
    }
}

fn config(env: &Env) -> Result<Config, DeFindexStrategyError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(DeFindexStrategyError::NotInitialized)
}

fn set_vault_account(env: &Env, vault: &Address, account_id: u64) {
    let key = DataKey::VaultAccount(vault.clone());
    let storage = env.storage().persistent();
    storage.set(&key, &account_id);
    storage.extend_ttl(
        &key,
        VAULT_ACCOUNT_TTL_THRESHOLD,
        VAULT_ACCOUNT_TTL_EXTEND_TO,
    );
}

fn clear_vault_account(env: &Env, vault: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::VaultAccount(vault.clone()));
}

fn extend_vault_account_ttl(env: &Env, vault: &Address) {
    let key = DataKey::VaultAccount(vault.clone());
    let storage = env.storage().persistent();
    if storage.has(&key) {
        storage.extend_ttl(
            &key,
            VAULT_ACCOUNT_TTL_THRESHOLD,
            VAULT_ACCOUNT_TTL_EXTEND_TO,
        );
    }
}

/// Resolves the stored controller account for `vault`.
///
/// Returns `0` when none is stored or the account no longer exists. When the
/// account is live, extends the mapping TTL. When gone and `clear_if_gone`,
/// removes the stale mapping.
fn resolve_vault_account(
    env: &Env,
    controller: &ControllerClient,
    vault: &Address,
    clear_if_gone: bool,
) -> u64 {
    let stored: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::VaultAccount(vault.clone()))
        .unwrap_or(0);
    if stored == 0 {
        return 0;
    }
    if controller.account_exists(&stored) {
        extend_vault_account_ttl(env, vault);
        return stored;
    }
    if clear_if_gone {
        clear_vault_account(env, vault);
    }
    0
}

/// Supply path: resolve and clear a stale mapping so `account_id = 0` opens a
/// new controller account.
fn prepare_vault_account_for_supply(
    env: &Env,
    controller: &ControllerClient,
    vault: &Address,
) -> u64 {
    resolve_vault_account(env, controller, vault, true)
}

/// Read/withdraw path: resolve without clearing a stale mapping.
fn reconcile_vault_account(env: &Env, controller: &ControllerClient, vault: &Address) -> u64 {
    resolve_vault_account(env, controller, vault, false)
}
