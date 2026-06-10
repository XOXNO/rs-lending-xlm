#![no_std]
//! Reference DeFindex strategy over the XOXNO lending controller.
//!
//! Implements the `DeFindexStrategyTrait` ABI (vendored below) so a DeFindex
//! vault can allocate one asset into the central lending pool:
//!
//! - the strategy owns one lending account (`account_id`) and keeps a
//!   per-depositor share ledger, so several vaults can share one instance —
//!   the same layout DeFindex's Blend strategy uses;
//! - `balance`/`deposit`/`withdraw` all report in underlying units from the
//!   controller's accrued-to-now views, never in shares;
//! - `withdraw` pays the recipient directly through the controller's `to`
//!   parameter (no token hop through the strategy);
//! - `harvest` is a no-op: lending interest auto-compounds via the supply
//!   index, there is no emissions token to claim or swap.
//!
//! Integration rules this contract demonstrates:
//! - the only auth a calling contract needs for `supply` is one
//!   `authorize_as_current_contract` entry for the nested SAC
//!   `transfer(strategy -> pool)`;
//! - a full close deletes the lending account, so the stored `account_id`
//!   is reset to `0` and the next deposit opens a fresh one;
//! - deposits below the market's USD dust floor and partial withdrawals
//!   leaving a sub-floor residue revert in the controller; callers can clamp
//!   with `max_withdrawable` / the controller's `max_withdraw` view.

use controller_interface::ControllerClient;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, vec, Address, Bytes,
    Env, IntoVal, Symbol, TryFromVal, Val, Vec,
};

/// Sampled by DeFindex's off-chain APY tooling: price per share in 12
/// decimals, the convention their `HarvestEvent` established.
#[contractevent(topics = ["strategy", "harvest"])]
#[derive(Clone, Debug)]
pub struct HarvestEvent {
    pub from: Address,
    pub amount: i128,
    pub price_per_share: i128,
}

/// First-deposit share forfeit guarding against share-price inflation; the
/// burned shares are paid out with the terminal exit. Same constant the
/// DeFindex Blend strategy uses.
const MINIMUM_SHARES: i128 = 1_000;

/// Price-per-share scale for the harvest event, matching the 12-decimal
/// convention DeFindex's APY tooling expects.
const PPS_SCALAR: i128 = 1_000_000_000_000;

/// Persistent share entries survive at least this long without touches;
/// every touch re-extends. (~30 days threshold, ~180 days extension.)
const SHARE_TTL_THRESHOLD: u32 = 17_280 * 30;
const SHARE_TTL_EXTEND_TO: u32 = 17_280 * 180;

/// Error codes 401/418 mirror `defindex-strategy-core`'s `StrategyError`;
/// codes from 460 are local to this reference implementation. The type is
/// not named `StrategyError` because `common::errors::StrategyError` is
/// linked into the WASM spec and the names would collide.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeFindexStrategyError {
    NotInitialized = 401,
    NotAuthorized = 418,
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
    /// Instance: immutable wiring.
    Config,
    /// Instance: the strategy's lending account; `0` means none open.
    AccountId,
    /// Instance: total shares including the inflation-guard forfeit.
    TotalShares,
    /// Persistent: shares per depositor (usually a DeFindex vault).
    Shares(Address),
}

/// Local mirror of `defindex-strategy-core` 0.2.0's `DeFindexStrategyTrait`
/// (github.com/paltalabs/defindex, `apps/contracts/strategies/core`).
/// Signatures match verbatim so the compiled WASM satisfies the DeFindex
/// vault ABI; depend on the published crate instead when its pinned
/// soroban-sdk matches this workspace.
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

#[contractimpl]
impl Strategy {
    /// `init_args = [controller: Address]`, mirroring the DeFindex
    /// constructor shape. The market for `asset` must already be listed:
    /// the pool address is resolved from the controller at deploy time.
    pub fn __constructor(env: Env, asset: Address, init_args: Vec<Val>) {
        let controller_val = init_args.get(0).unwrap_or_else(|| {
            soroban_sdk::panic_with_error!(&env, DeFindexStrategyError::NotInitialized)
        });
        let controller = Address::try_from_val(&env, &controller_val).unwrap_or_else(|_| {
            soroban_sdk::panic_with_error!(&env, DeFindexStrategyError::NotInitialized)
        });

        let pool = ControllerClient::new(&env, &controller)
            .get_all_markets_detailed(&vec![&env, asset.clone()])
            .get(0)
            .unwrap_or_else(|| {
                soroban_sdk::panic_with_error!(&env, DeFindexStrategyError::NotInitialized)
            })
            .pool_address;

        let storage = env.storage().instance();
        storage.set(
            &DataKey::Config,
            &Config {
                asset,
                controller,
                pool,
            },
        );
        storage.set(&DataKey::AccountId, &0u64);
        storage.set(&DataKey::TotalShares, &0i128);
    }
}

#[contractimpl]
impl DeFindexStrategyTrait for Strategy {
    fn asset(env: Env) -> Result<Address, DeFindexStrategyError> {
        Ok(config(&env)?.asset)
    }

    /// Pulls `amount` from `from`, supplies it into the lending account, and
    /// returns `from`'s post-deposit underlying balance — the value the
    /// DeFindex vault books as the strategy's state.
    fn deposit(env: Env, amount: i128, from: Address) -> Result<i128, DeFindexStrategyError> {
        if amount <= 0 {
            return Err(DeFindexStrategyError::AmountNotPositive);
        }
        from.require_auth();
        let cfg = config(&env)?;
        let strategy = env.current_contract_address();

        let account_id = stored_account_id(&env);
        let total_before = protocol_balance(&env, &cfg, account_id);

        // The vault pre-authorizes this pull, exactly as it does for its
        // other strategies.
        token::Client::new(&env, &cfg.asset).transfer(&from, &strategy, &amount);

        // The controller's supply runs one nested SAC transfer
        // (strategy -> pool); authorize precisely that sub-invocation.
        authorize_pool_transfer(&env, &cfg, amount);
        let new_account_id = ControllerClient::new(&env, &cfg.controller).supply(
            &strategy,
            &account_id,
            &0u32,
            &vec![&env, (cfg.asset.clone(), amount)],
        );
        set_account_id(&env, new_account_id);

        // Measure the credited value instead of trusting `amount`: exact at
        // the index the pool just used, no rounding replication needed.
        let total_after = protocol_balance(&env, &cfg, new_account_id);
        let credited = total_after
            .checked_sub(total_before)
            .filter(|v| *v > 0)
            .ok_or(DeFindexStrategyError::ArithmeticError)?;

        let total_shares = total_shares(&env);
        if total_shares == 0 {
            if credited <= MINIMUM_SHARES {
                return Err(DeFindexStrategyError::InsufficientBalance);
            }
            // Forfeit shares are parked on the strategy address; the
            // terminal exit pays out their backing.
            set_shares(&env, &strategy, MINIMUM_SHARES);
            set_shares(&env, &from, credited - MINIMUM_SHARES);
            set_total_shares(&env, credited);
        } else {
            let minted = muldiv_floor(credited, total_shares, total_before)?;
            if minted == 0 {
                return Err(DeFindexStrategyError::InsufficientBalance);
            }
            set_shares(&env, &from, shares_of(&env, &from) + minted);
            set_total_shares(&env, total_shares + minted);
        }

        balance_of(&env, &cfg, &from)
    }

    /// No-op: interest accrues into the supply index, so there is nothing to
    /// claim or compound. Emits the price-per-share the DeFindex APY
    /// tooling samples.
    fn harvest(env: Env, from: Address, _data: Option<Bytes>) -> Result<(), DeFindexStrategyError> {
        let cfg = config(&env)?;
        let total_shares = total_shares(&env);
        let pps = if total_shares == 0 {
            PPS_SCALAR
        } else {
            let total = protocol_balance(&env, &cfg, stored_account_id(&env));
            muldiv_floor(total, PPS_SCALAR, total_shares)?
        };
        HarvestEvent {
            from,
            amount: 0,
            price_per_share: pps,
        }
        .publish(&env);
        Ok(())
    }

    /// Current underlying value attributable to `from`, accrued to now.
    fn balance(env: Env, from: Address) -> Result<i128, DeFindexStrategyError> {
        let cfg = config(&env)?;
        balance_of(&env, &cfg, &from)
    }

    /// Withdraws `amount` of underlying for `from`, paying `to` directly via
    /// the controller. Returns `from`'s post-withdraw balance. The terminal
    /// exit (last holder leaving) closes the lending account with the `0`
    /// sentinel and resets `account_id`, so the next deposit reopens one.
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
        let cfg = config(&env)?;
        let strategy = env.current_contract_address();
        let account_id = stored_account_id(&env);
        if account_id == 0 {
            return Err(DeFindexStrategyError::InsufficientBalance);
        }

        let total_underlying = protocol_balance(&env, &cfg, account_id);
        let total_shares = total_shares(&env);
        let from_shares = shares_of(&env, &from);
        let from_balance = muldiv_floor(from_shares, total_underlying, total_shares)?;
        if amount > from_balance {
            return Err(DeFindexStrategyError::InsufficientBalance);
        }

        let forfeit = shares_of(&env, &strategy);
        // Sole holder asking for their entire balance: close the lending
        // account outright instead of leaving a sub-dust-floor residue
        // backing only the forfeit shares.
        let terminal = amount == from_balance && total_shares == from_shares + forfeit;
        let controller = ControllerClient::new(&env, &cfg.controller);

        if terminal {
            // Full close: the pool pays the floor-rounded position value —
            // `from`'s share plus the forfeit backing — straight to `to`,
            // and the controller deletes the account.
            controller.withdraw(
                &strategy,
                &account_id,
                &vec![&env, (cfg.asset.clone(), 0i128)],
                &Some(to),
            );
            set_account_id(&env, 0);
            set_shares(&env, &from, 0);
            set_shares(&env, &strategy, 0);
            set_total_shares(&env, 0);
            return Ok(0);
        }

        let mut burned = muldiv_ceil(amount, total_shares, total_underlying)?;
        if burned > from_shares {
            burned = from_shares;
        }

        // Partial: reverts in the controller if the pool lacks liquidity or
        // the residue would fall below the USD dust floor; vault keepers can
        // clamp with `max_withdrawable` first.
        controller.withdraw(
            &strategy,
            &account_id,
            &vec![&env, (cfg.asset.clone(), amount)],
            &Some(to),
        );
        set_shares(&env, &from, from_shares - burned);
        set_total_shares(&env, total_shares - burned);

        balance_of(&env, &cfg, &from)
    }
}

/// Integrator conveniences beyond the DeFindex trait. Plain returns keep
/// `DeFindexStrategyError` out of this block's spec (single export from the trait
/// impl); they panic `NotInitialized` before the constructor ran.
#[contractimpl]
impl Strategy {
    /// Underlying value of the whole strategy position, accrued to now.
    pub fn total_underlying(env: Env) -> i128 {
        let cfg = expect_config(&env);
        protocol_balance(&env, &cfg, stored_account_id(&env))
    }

    /// Largest withdrawal the lending market currently allows for this
    /// strategy's account (controller `max_withdraw` passthrough).
    pub fn max_withdrawable(env: Env) -> i128 {
        let cfg = expect_config(&env);
        let account_id = stored_account_id(&env);
        if account_id == 0 {
            return 0;
        }
        ControllerClient::new(&env, &cfg.controller).max_withdraw(&account_id, &cfg.asset)
    }

    pub fn shares(env: Env, of: Address) -> i128 {
        shares_of(&env, &of)
    }

    pub fn total_shares(env: Env) -> i128 {
        total_shares(&env)
    }

    pub fn lending_account_id(env: Env) -> u64 {
        stored_account_id(&env)
    }
}

fn config(env: &Env) -> Result<Config, DeFindexStrategyError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(DeFindexStrategyError::NotInitialized)
}

fn expect_config(env: &Env) -> Config {
    config(env).unwrap_or_else(|_| {
        soroban_sdk::panic_with_error!(env, DeFindexStrategyError::NotInitialized)
    })
}

fn stored_account_id(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::AccountId)
        .unwrap_or(0)
}

fn set_account_id(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::AccountId, &id);
}

fn total_shares(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalShares)
        .unwrap_or(0)
}

fn set_total_shares(env: &Env, value: i128) {
    env.storage().instance().set(&DataKey::TotalShares, &value);
}

fn shares_of(env: &Env, of: &Address) -> i128 {
    let key = DataKey::Shares(of.clone());
    let storage = env.storage().persistent();
    let value = storage.get(&key).unwrap_or(0);
    if value > 0 {
        storage.extend_ttl(&key, SHARE_TTL_THRESHOLD, SHARE_TTL_EXTEND_TO);
    }
    value
}

fn set_shares(env: &Env, of: &Address, value: i128) {
    let key = DataKey::Shares(of.clone());
    let storage = env.storage().persistent();
    if value == 0 {
        storage.remove(&key);
    } else {
        storage.set(&key, &value);
        storage.extend_ttl(&key, SHARE_TTL_THRESHOLD, SHARE_TTL_EXTEND_TO);
    }
}

/// Underlying value of the strategy's lending position; zero with no account.
fn protocol_balance(env: &Env, cfg: &Config, account_id: u64) -> i128 {
    if account_id == 0 {
        return 0;
    }
    ControllerClient::new(env, &cfg.controller).collateral_amount_for_token(&account_id, &cfg.asset)
}

fn balance_of(env: &Env, cfg: &Config, of: &Address) -> Result<i128, DeFindexStrategyError> {
    let total_shares = total_shares(env);
    if total_shares == 0 {
        return Ok(0);
    }
    let total = protocol_balance(env, cfg, stored_account_id(env));
    muldiv_floor(shares_of(env, of), total, total_shares)
}

/// Pre-authorizes the controller's nested `transfer(strategy -> pool)`.
fn authorize_pool_transfer(env: &Env, cfg: &Config, amount: i128) {
    env.authorize_as_current_contract(vec![
        env,
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: cfg.asset.clone(),
                fn_name: Symbol::new(env, "transfer"),
                args: (env.current_contract_address(), cfg.pool.clone(), amount).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }),
    ]);
}

fn muldiv_floor(a: i128, b: i128, denominator: i128) -> Result<i128, DeFindexStrategyError> {
    if denominator == 0 {
        return Err(DeFindexStrategyError::ArithmeticError);
    }
    a.checked_mul(b)
        .map(|product| product / denominator)
        .ok_or(DeFindexStrategyError::ArithmeticError)
}

fn muldiv_ceil(a: i128, b: i128, denominator: i128) -> Result<i128, DeFindexStrategyError> {
    if denominator == 0 {
        return Err(DeFindexStrategyError::ArithmeticError);
    }
    a.checked_mul(b)
        .and_then(|product| product.checked_add(denominator - 1))
        .map(|padded| padded / denominator)
        .ok_or(DeFindexStrategyError::ArithmeticError)
}
