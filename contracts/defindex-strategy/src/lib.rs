#![no_std]
//! DeFindex adapter: one vault ↔ one controller account; harvest emits D12 supply-index PPS.

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
    // D{AssetDecimals(asset)}{Token(asset)}; this adapter emits zero.
    pub amount: i128,
    // D12{Token(asset)/Share(asset, supply)}
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
// dimensional: D27{1} / D12{1} = D15{1} Ray-to-price-per-share divisor.
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
    pub hub_id: u32,
    pub spoke_id: u32,
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
    /// Deploys the adapter for one controller market, caching the resolved
    /// `Config` (hub, spoke, asset, controller, pool).
    ///
    /// # Arguments
    /// * `init_args` - `[controller, hub_id, spoke_id]`; the asset must be listed
    ///   for `hub_id`, and positions are opened on `spoke_id`.
    ///
    /// # Errors
    /// * `NotInitialized` - `init_args` is missing an element or an element has the wrong type.
    /// * The controller's `get_market_index` reverts when the asset is not listed for `hub_id`.
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
        // Validate configured HubAssetKey; get_market_index reverts if unlisted.
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

    /// Pulls `amount` of the strategy asset from `from` and supplies it into the
    /// vault's controller account (opening one on first deposit); returns the
    /// post-deposit underlying balance.
    ///
    /// # Arguments
    /// * `amount` - strategy-asset amount to supply; must be positive.
    /// * `from` - the vault; must authorize.
    ///
    /// # Errors
    /// * `AmountNotPositive` - `amount` is not greater than zero.
    /// * `NotInitialized` - the adapter has no cached `Config`.
    /// * The controller `supply` call enforces its own market, spoke, cap, and
    ///   pause gates; refer to controller errors.
    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError> {
        if amount <= 0 {
            return Err(DeFindexStrategyError::AmountNotPositive);
        }
        from.require_auth();

        let ctx = Ctx::try_load(&env)?;

        // D{AssetDecimals(asset)}{Token(asset)}
        token::Client::new(&env, &ctx.cfg.asset).transfer(&from, &ctx.strategy, &amount);

        // Resolve stale vault-account state before authorizing the pool transfer.
        // Authorization applies to the next sub-invocation, so no controller calls
        // can sit between authorization and `supply`.
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

    /// Publishes a `HarvestEvent` carrying the market's current supply-index
    /// price-per-share (12 decimals); moves no funds and reports zero amount.
    ///
    /// # Arguments
    /// * `from` - the caller; must authorize. `data` is ignored.
    ///
    /// # Errors
    /// * `NotInitialized` - the adapter has no cached `Config`.
    /// * `ArithmeticError` - the supply-index-to-price-per-share division overflows.
    ///
    /// # Events
    /// * `HarvestEvent` - the current 12-decimal price-per-share.
    fn harvest(env: Env, from: Address, _data: Option<Bytes>) -> Result<(), DeFindexStrategyError> {
        from.require_auth();
        let ctx = Ctx::try_load(&env)?;
        emit_harvest(&env, from, 0, ctx.harvest_price_per_share()?);
        Ok(())
    }

    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError> {
        // dimensional: strategy balance is live D{AssetDecimals(asset)}{Token(asset)}.
        Ok(Ctx::try_load(&env)?.vault_balance(&from))
    }

    /// account to `to`, closing the account on a full exit; returns the
    /// remaining underlying balance.
    ///
    /// # Arguments
    /// * `amount` - strategy-asset amount to withdraw; must be positive and at
    ///   most the current balance (a full-balance amount closes the position).
    /// * `from` - the vault; must authorize.
    /// * `to` - recipient of the withdrawn tokens.
    ///
    /// # Errors
    /// * `AmountNotPositive` - `amount` is not greater than zero.
    /// * `NotInitialized` - the adapter has no cached `Config`.
    /// * `InsufficientBalance` - no vault account exists or `amount` exceeds the balance.
    /// * The controller `withdraw` call enforces its own gates; refer to controller errors.
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

        // Full exit maps to controller full-withdraw sentinel 0; public ABI rejects amount ≤ 0.
        let is_full_withdraw = amount == balance;
        let withdraw_amount = if is_full_withdraw { 0 } else { amount };
        ctx.controller.withdraw(
            &ctx.strategy,
            &account_id,
            &ctx.to_payment(withdraw_amount),
            &Some(to),
        );

        // Full exit (zero collateral): clear the mapping immediately so the
        // next deposit gets a fresh controller account. Prevents dust pinning
        // that could hit PositionLimitExceeded on redeposit.
        if is_full_withdraw {
            clear_vault_account(ctx.env, &from);
        }

        // Removed accounts report 0 collateral.
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
