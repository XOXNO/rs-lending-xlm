#![no_std]

//! Adapter contract that exposes a lending-controller position through the
//! DeFindex strategy interface. Deposits and withdrawals proxy to the
//! `controller` contract's `supply`/`withdraw` entry points for a single
//! configured hub asset, and each vault address is mapped to the controller
//! account id that holds its collateral.

use common::constants::{TTL_BUMP_USER, TTL_THRESHOLD_USER};
use common::math::fp::Ray;
use common::token::authorize_transfer_as_current;
use common::types::pool::HubAssetKey;

use controller_interface::ControllerClient;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, vec,
    Address, Bytes, Env, TryFromVal, Val, Vec,
};

/// Event published on each `harvest` call, reporting the caller and the
/// current price per share for the configured hub asset.
#[contractevent(topics = ["strategy", "harvest"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarvestEvent {
    pub from: Address,
    pub amount: i128,
    pub price_per_share: i128,
}

/// DeFindex price-per-share is reported with 12 decimals (RAY → 12-dec rescale).
const PPS_DECIMALS: u32 = 12;

/// Error codes returned by the strategy contract's external entry points.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeFindexStrategyError {
    NotInitialized = 401,

    AmountNotPositive = 460,

    InsufficientBalance = 461,

    ArithmeticError = 462,

    AccountLookupFailed = 463,
}

/// Configuration stored once at construction: the hub/spoke ids and asset
/// this strategy supplies to, plus the controller and pool contract
/// addresses resolved at that time.
#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub hub_id: u32,
    pub spoke_id: u32,
    pub asset: Address,
    pub controller: Address,
    pub pool: Address,
}

/// Instance/persistent storage keys. `Config` holds the strategy's
/// `Config` value; `VaultAccount(Address)` maps a vault address to the
/// controller account id that holds its collateral.
#[contracttype]
pub enum DataKey {
    Config,

    VaultAccount(Address),
}

/// DeFindex vault strategy interface implemented by [`Strategy`].
pub trait DeFindexStrategyTrait {
    /// Returns the configured underlying asset address, or `NotInitialized`
    /// if the contract has not been constructed.
    fn asset(env: Env) -> Result<Address, DeFindexStrategyError>;

    /// Transfers `amount` of the underlying asset from `from` into the
    /// strategy and supplies it to the controller, creating or reusing
    /// `from`'s vault account. Requires `from`'s authorization and returns
    /// `AmountNotPositive` if `amount` is not positive or `NotInitialized`
    /// if the contract has not been constructed. Returns the account's
    /// resulting collateral balance.
    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError>;

    /// Emits a `HarvestEvent` carrying the current price per share for the
    /// configured hub asset. Requires `from`'s authorization and returns
    /// `NotInitialized` if the contract has not been constructed. Moves no
    /// funds.
    fn harvest(env: Env, from: Address, data: Option<Bytes>) -> Result<(), DeFindexStrategyError>;

    /// Returns the collateral balance of `from`'s vault account, or 0 if it
    /// has none. Returns `NotInitialized` if the contract has not been
    /// constructed.
    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError>;

    /// Withdraws `amount` of collateral from `from`'s vault account to `to`
    /// via the controller. Requires `from`'s authorization and returns
    /// `AmountNotPositive`, `InsufficientBalance`, or `NotInitialized` if
    /// `amount` is not positive, `from` has no vault account or an amount
    /// exceeding its balance, or the contract has not been constructed. A
    /// withdrawal equal to the full balance clears the vault account
    /// mapping. Returns the account's remaining collateral balance.
    fn withdraw(
        env: Env,
        amount: i128,
        from: Address,
        to: Address,
    ) -> Result<i128, DeFindexStrategyError>;
}

/// The DeFindex strategy contract, implementing [`DeFindexStrategyTrait`].
#[contract]
pub struct Strategy;

/// Bundles the loaded [`Config`], a client for the configured controller,
/// and the strategy's own contract address, for use across a single entry
/// point invocation.
struct Ctx<'a> {
    env: &'a Env,
    cfg: Config,
    controller: ControllerClient<'a>,
    strategy: Address,
}

impl<'a> Ctx<'a> {
    /// Loads the stored [`Config`] and builds a `Ctx` from it. Returns
    /// `NotInitialized` if the contract has no stored configuration.
    fn try_load(env: &'a Env) -> Result<Self, DeFindexStrategyError> {
        let cfg = config(env)?;
        Ok(Self {
            strategy: env.current_contract_address(),
            controller: ControllerClient::new(env, &cfg.controller),
            cfg,
            env,
        })
    }

    /// Builds the `HubAssetKey` for the configured hub id and asset.
    fn hub_asset(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: self.cfg.hub_id,
            asset: self.cfg.asset.clone(),
        }
    }

    /// Returns the controller-reported collateral amount for `account_id`
    /// in the configured hub asset.
    fn collateral(&self, account_id: u64) -> i128 {
        self.controller
            .get_collateral_amount(&account_id, &self.hub_asset())
    }

    /// Returns `vault`'s collateral balance, or 0 if it has no resolvable
    /// vault account.
    fn vault_balance(&self, vault: &Address) -> i128 {
        let account_id = resolve_vault_account(self.env, &self.controller, vault, false);
        if account_id == 0 {
            return 0;
        }

        self.collateral(account_id)
    }

    /// Computes the current price per share from the controller's supply
    /// index for the configured hub asset, rescaled from RAY (27 decimals)
    /// down to `PPS_DECIMALS` with floor rounding.
    fn harvest_price_per_share(&self) -> Result<i128, DeFindexStrategyError> {
        let supply_index = self
            .controller
            .get_market_index(&self.hub_asset())
            .supply_index;

        // Floor rescale RAY (27 dec) → PPS (12 dec); matches prior `index / (RAY/1e12)`.
        Ok(Ray::from(supply_index).to_asset_floor(PPS_DECIMALS))
    }

    /// Wraps `amount` in a single-entry payment vector keyed by the
    /// configured hub asset, as expected by the controller's
    /// `supply`/`withdraw` entry points.
    fn to_payment(&self, amount: i128) -> Vec<(HubAssetKey, i128)> {
        vec![self.env, (self.hub_asset(), amount)]
    }

    /// Authorizes the pool contract to pull `amount` of the configured
    /// asset from the strategy's own balance.
    fn authorize_supply_to_pool(&self, amount: i128) {
        authorize_transfer_as_current(
            self.env,
            &self.cfg.asset,
            &self.strategy,
            &self.cfg.pool,
            amount,
        );
    }
}

#[contractimpl]
impl Strategy {
    /// Decodes `init_args` as `(controller: Address, hub_id: u32, spoke_id: u32)`,
    /// panics with `NotInitialized` if any argument is missing or of the
    /// wrong type, verifies the hub market index exists for `asset`, and
    /// stores the resulting `Config` (resolving the pool address from the
    /// controller) in instance storage.
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

        let received = common::token::transfer_amount_measured(
            &env,
            &ctx.cfg.asset,
            &from,
            &ctx.strategy,
            amount,
            common::errors::GenericError::AmountMustBePositive,
        );

        let stored_id = resolve_vault_account(ctx.env, &ctx.controller, &from, true);
        ctx.authorize_supply_to_pool(received);

        let new_or_existing_id = ctx.controller.supply(
            &ctx.strategy,
            &stored_id,
            &ctx.cfg.spoke_id,
            &ctx.to_payment(received),
        );
        set_vault_account(ctx.env, &from, new_or_existing_id);

        Ok(ctx.collateral(new_or_existing_id))
    }

    fn harvest(env: Env, from: Address, _data: Option<Bytes>) -> Result<(), DeFindexStrategyError> {
        from.require_auth();
        let ctx = Ctx::try_load(&env)?;
        HarvestEvent {
            from,
            // `harvest` moves no funds.
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
        let account_id = resolve_vault_account(&env, &ctx.controller, &from, false);
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

/// Loads the stored `Config` from instance storage. Returns
/// `NotInitialized` if the contract has not been constructed.
fn config(env: &Env) -> Result<Config, DeFindexStrategyError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(DeFindexStrategyError::NotInitialized)
}

/// Stores `vault`'s controller account id in persistent storage and
/// extends its TTL.
fn set_vault_account(env: &Env, vault: &Address, account_id: u64) {
    let key = DataKey::VaultAccount(vault.clone());
    let storage = env.storage().persistent();
    storage.set(&key, &account_id);
    storage.extend_ttl(&key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

/// Removes `vault`'s stored account id mapping, if any.
fn clear_vault_account(env: &Env, vault: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::VaultAccount(vault.clone()));
}

/// Extends the TTL of `vault`'s stored account id mapping if it exists.
fn extend_vault_account_ttl(env: &Env, vault: &Address) {
    let key = DataKey::VaultAccount(vault.clone());
    let storage = env.storage().persistent();
    if storage.has(&key) {
        // Same tier as the controller account this points at, so the pointer can
        // never outlive its target.
        storage.extend_ttl(&key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
    }
}

/// Reads `vault`'s stored account id, returning 0 if none is stored. If the
/// controller confirms the account still exists, extends the mapping's TTL
/// and returns the id; otherwise clears the mapping when `clear_if_gone` is
/// set and returns 0. Panics with `AccountLookupFailed` if the controller
/// lookup itself fails.
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
    match controller.try_account_exists(&stored) {
        Ok(Ok(true)) => {
            extend_vault_account_ttl(env, vault);
            stored
        }
        // Only an explicit "gone" clears. The mapping is the sole route back to
        // the collateral it points at and there is no way to re-point it, so a
        // lookup that merely failed to answer must not be read as gone.
        Ok(Ok(false)) => {
            if clear_if_gone {
                clear_vault_account(env, vault);
            }
            0
        }
        _ => panic_with_error!(env, DeFindexStrategyError::AccountLookupFailed),
    }
}
