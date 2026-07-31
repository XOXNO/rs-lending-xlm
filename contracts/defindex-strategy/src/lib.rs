#![no_std]

use common::constants::RAY;
use common::types::pool::HubAssetKey;

use controller_interface::ControllerClient;

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    vec, Address, Bytes, Env, IntoVal, Symbol, TryFromVal, Val, Vec,
};

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

const PPS_SCALAR: i128 = 1_000_000_000_000;

const RAY_PER_PPS: i128 = RAY / PPS_SCALAR;

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
    pub hub_id: u32,
    pub spoke_id: u32,
    pub asset: Address,
    pub controller: Address,
    pub pool: Address,
}

#[contracttype]
pub enum DataKey {
    Config,

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

    fn hub_asset(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: self.cfg.hub_id,
            asset: self.cfg.asset.clone(),
        }
    }

    fn collateral(&self, account_id: u64) -> i128 {
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

        self.collateral(account_id)
    }

    fn harvest_price_per_share(&self) -> Result<i128, DeFindexStrategyError> {
        let supply_index = self
            .controller
            .get_market_index(&self.hub_asset())
            .supply_index;

        supply_index
            .checked_div(RAY_PER_PPS)
            .ok_or(DeFindexStrategyError::ArithmeticError)
    }

    fn to_payment(&self, amount: i128) -> Vec<(HubAssetKey, i128)> {
        vec![self.env, (self.hub_asset(), amount)]
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

        let stored_id = prepare_vault_account_for_supply(ctx.env, &ctx.controller, &from);
        ctx.authorize_supply_to_pool(amount);

        let new_or_existing_id = ctx.controller.supply(
            &ctx.strategy,
            &stored_id,
            &ctx.cfg.spoke_id,
            &ctx.to_payment(amount),
        );
        set_vault_account(ctx.env, &from, new_or_existing_id);

        Ok(ctx.collateral(new_or_existing_id))
    }

    fn harvest(env: Env, from: Address, _data: Option<Bytes>) -> Result<(), DeFindexStrategyError> {
        from.require_auth();
        let ctx = Ctx::try_load(&env)?;
        emit_harvest(&env, from, 0, ctx.harvest_price_per_share()?);
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
