#![no_std]
//! DeFindex strategy adapter for the XOXNO lending controller.
//!
//! One WASM is deployed per underlying asset. Each vault (`from`) maps to one
//! controller `account_id`; vaults do not share positions.
//!
//! - Balances come from `get_collateral_amount`.
//! - Full withdraw maps `amount == balance()` to controller amount `0`.
//! - Supply clears stale vault-account mappings.
//! - `harvest` publishes Blend-compatible `price_per_share` from the supply index.

use common::constants::RAY;
use controller_interface::ControllerClient;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, vec, Address, Bytes,
    Env, IntoVal, Symbol, TryFromVal, Val, Vec,
};

/// Harvest event with 12-decimal `price_per_share`.
#[contractevent(topics = ["strategy", "harvest"])]
#[derive(Clone, Debug)]
pub struct HarvestEvent {
    pub from: Address,
    pub amount: i128,
    pub price_per_share: i128,
}

const PPS_SCALAR: i128 = 1_000_000_000_000;
const RAY_PER_PPS: i128 = RAY / PPS_SCALAR;

/// Vault-account TTL: extend when below ~30 days, up to ~180 days.
const VAULT_ACCOUNT_TTL_THRESHOLD: u32 = 17_280 * 30;
const VAULT_ACCOUNT_TTL_EXTEND_TO: u32 = 17_280 * 180;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeFindexStrategyError {
    NotInitialized = 401,
    AmountNotPositive = 460,
    InsufficientBalance = 461,
    ArithmeticError = 462,
}

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub asset: Address,
    pub controller: Address,
    pub pool: Address,
}

#[contracttype]
pub enum DataKey {
    Config,
    /// Per-vault controller account id (`0` = none stored).
    VaultAccount(Address),
}

pub trait DeFindexStrategyTrait {
    fn asset(env: Env) -> Result<Address, DeFindexStrategyError>;
    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError>;
    fn harvest(env: Env, from: Address, data: Option<Bytes>) -> Result<(), DeFindexStrategyError>;
    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError>;
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

    fn load(env: &'a Env) -> Self {
        Self::try_load(env).unwrap_or_else(|_| {
            soroban_sdk::panic_with_error!(env, DeFindexStrategyError::NotInitialized)
        })
    }

    fn collateral(&self, account_id: u64) -> i128 {
        self.controller
            .get_collateral_amount(&account_id, &self.cfg.asset)
    }

    fn reconcile(&self, vault: &Address) -> u64 {
        reconcile_vault_account(self.env, &self.controller, vault)
    }

    fn vault_balance(&self, vault: &Address) -> i128 {
        let account_id = self.reconcile(vault);
        if account_id == 0 {
            return 0;
        }
        self.collateral(account_id)
    }

    fn harvest_price_per_share(&self) -> Result<i128, DeFindexStrategyError> {
        let supply_index_ray = self
            .controller
            .get_market_index(&self.cfg.asset)
            .supply_index_ray;
        supply_index_ray
            .checked_div(RAY_PER_PPS)
            .ok_or(DeFindexStrategyError::ArithmeticError)
    }

    fn to_payment(&self, amount: i128) -> Vec<(Address, i128)> {
        vec![self.env, (self.cfg.asset.clone(), amount)]
    }

    fn authorize_supply_to_pool(&self, amount: i128) {
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
    /// `init_args = [controller]`. `asset` must be a listed market.
    pub fn __constructor(env: Env, asset: Address, init_args: Vec<Val>) {
        let controller_val = init_args.get(0).unwrap_or_else(|| {
            soroban_sdk::panic_with_error!(&env, DeFindexStrategyError::NotInitialized)
        });
        let controller = Address::try_from_val(&env, &controller_val).unwrap_or_else(|_| {
            soroban_sdk::panic_with_error!(&env, DeFindexStrategyError::NotInitialized)
        });

        let controller_client = ControllerClient::new(&env, &controller);
        controller_client.get_market_config(&asset);
        env.storage().instance().set(
            &DataKey::Config,
            &Config {
                asset,
                controller,
                pool: controller_client.get_pool_address(),
            },
        );
    }

    /// Live controller account id for `vault` (`0` if missing or removed).
    pub fn lending_account_id(env: Env, vault: Address) -> u64 {
        Ctx::load(&env).reconcile(&vault)
    }

    /// Whether `vault` has a live controller account.
    pub fn has_lending_account(env: Env, vault: Address) -> bool {
        Ctx::load(&env).reconcile(&vault) != 0
    }
}

#[contractimpl]
impl DeFindexStrategyTrait for Strategy {
    fn asset(env: Env) -> Result<Address, DeFindexStrategyError> {
        Ok(config(&env)?.asset)
    }

    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError> {
        if amount <= 0 {
            return Err(DeFindexStrategyError::AmountNotPositive);
        }
        from.require_auth();

        let ctx = Ctx::try_load(&env)?;

        token::Client::new(&env, &ctx.cfg.asset).transfer(&from, &ctx.strategy, &amount);

        // Resolve stale vault-account state before authorizing the pool transfer.
        // Authorization applies to the next sub-invocation, so no controller calls
        // can sit between authorization and `supply`.
        let stored_id = prepare_vault_account_for_supply(ctx.env, &ctx.controller, &from);
        ctx.authorize_supply_to_pool(amount);
        let new_or_existing_id =
            ctx.controller
                .supply(&ctx.strategy, &stored_id, &0u32, &ctx.to_payment(amount));
        set_vault_account(ctx.env, &from, new_or_existing_id);

        Ok(ctx.collateral(new_or_existing_id))
    }

    fn harvest(env: Env, from: Address, _data: Option<Bytes>) -> Result<(), DeFindexStrategyError> {
        from.require_auth();
        let ctx = Ctx::try_load(&env)?;
        HarvestEvent {
            from,
            amount: 0,
            price_per_share: ctx.harvest_price_per_share()?,
        }
        .publish(&env);
        Ok(())
    }

    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError> {
        Ok(Ctx::try_load(&env)?.vault_balance(&from))
    }

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
        if amount > balance {
            return Err(DeFindexStrategyError::InsufficientBalance);
        }

        // Full exit uses controller withdraw-all sentinel `0`.
        let withdraw_amount = if amount == balance { 0 } else { amount };
        ctx.controller.withdraw(
            &ctx.strategy,
            &account_id,
            &ctx.to_payment(withdraw_amount),
            &Some(to),
        );

        // Removed accounts report 0 collateral.
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

/// Returns the live stored account id; optionally clears stale storage.
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

fn prepare_vault_account_for_supply(
    env: &Env,
    controller: &ControllerClient,
    vault: &Address,
) -> u64 {
    resolve_vault_account(env, controller, vault, true)
}

fn reconcile_vault_account(env: &Env, controller: &ControllerClient, vault: &Address) -> u64 {
    resolve_vault_account(env, controller, vault, false)
}
