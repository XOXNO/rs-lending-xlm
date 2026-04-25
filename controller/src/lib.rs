#![no_std]
#![allow(clippy::too_many_arguments)]

// Conditional summary helper: under `certora`, expands a function definition
// through `cvlr_soroban_macros::apply_summary!` (redirecting public callers
// to a nondet summary in `controller/certora/spec/summaries/`). Under any
// other build, it just re-emits the function body unchanged. Lets each
// production site declare its summary indirection once without duplicating
// the real body for each cfg arm.
#[cfg(feature = "certora")]
#[doc(hidden)]
#[macro_export]
macro_rules! summarized {
    ($summary:path, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!($summary, $($body)*);
    };
}

#[cfg(not(feature = "certora"))]
#[doc(hidden)]
#[macro_export]
macro_rules! summarized {
    ($summary:path, $($body:tt)*) => {
        $($body)*
    };
}

pub(crate) mod cache;
mod config;
mod flash_loan;
pub(crate) mod helpers;
pub(crate) mod oracle;
pub(crate) mod positions;
mod router;
mod storage;
mod strategy;
mod utils;
mod validation;
mod views;

#[cfg(feature = "certora")]
#[path = "../certora/spec/mod.rs"]
pub mod spec;

use common::errors::GenericError;
use common::events::{emit_approve_token_wasm, ApproveTokenWasmEvent};
use common::types::{
    AccountAttributes, AccountPosition, AssetConfig, AssetExtendedConfigView, EModeCategory,
    LiquidationEstimate, MarketConfig, MarketIndexView, MarketOracleConfigInput, MarketParams,
    PositionLimits, PositionMode, SwapSteps,
};
use soroban_sdk::{
    contract, contractimpl, panic_with_error, Address, Bytes, BytesN, Env, Symbol, Vec,
};
use stellar_access::{access_control, ownable};
use stellar_macros::{only_owner, only_role};

use crate::positions::supply;
#[cfg(test)]
use crate::positions::update;

// ---------------------------------------------------------------------------
// Role definitions
// ---------------------------------------------------------------------------

const KEEPER_ROLE: &str = "KEEPER"; // update_indexes, clean_bad_debt, update_account_threshold
const REVENUE_ROLE: &str = "REVENUE"; // claim_revenue, add_rewards
const ORACLE_ROLE: &str = "ORACLE"; // configure_market_oracle, edit_oracle_tolerance, disable_token_oracle

fn default_operational_roles(env: &Env) -> [Symbol; 3] {
    [
        Symbol::new(env, KEEPER_ROLE),
        Symbol::new(env, REVENUE_ROLE),
        Symbol::new(env, ORACLE_ROLE),
    ]
}

fn sync_pending_admin_transfer(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    let pending_admin_key = access_control::AccessControlStorageKey::PendingAdmin;

    if live_until_ledger == 0 {
        env.storage().temporary().remove(&pending_admin_key);
    } else {
        stellar_access::role_transfer::transfer_role(
            env,
            new_owner,
            &pending_admin_key,
            live_until_ledger,
        );
    }

    let current_admin = access_control::get_admin(env)
        .or_else(|| ownable::get_owner(env))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet));
    access_control::emit_admin_transfer_initiated(
        env,
        &current_admin,
        new_owner,
        live_until_ledger,
    );
}

fn sync_owner_access_control(env: &Env, previous_owner: &Address, new_owner: &Address) {
    let previous_admin = access_control::get_admin(env).unwrap_or_else(|| previous_owner.clone());

    env.storage()
        .instance()
        .set(&access_control::AccessControlStorageKey::Admin, new_owner);
    env.storage()
        .temporary()
        .remove(&access_control::AccessControlStorageKey::PendingAdmin);
    access_control::emit_admin_transfer_completed(env, &previous_admin, new_owner);

    for role in default_operational_roles(env) {
        access_control::grant_role_no_auth(env, new_owner, &role, new_owner);

        if previous_owner != new_owner
            && access_control::has_role(env, previous_owner, &role).is_some()
        {
            access_control::revoke_role_no_auth(env, previous_owner, &role, new_owner);
        }
    }
}

#[contract]
pub struct Controller;

#[contractimpl]
impl Controller {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);

        // Grant only KEEPER at construct. REVENUE and ORACLE require an
        // explicit `grant_role` after deploy so a compromised owner key in
        // the bootstrap window cannot immediately exercise those roles.
        access_control::set_admin(&env, &admin);
        let keeper_role = soroban_sdk::Symbol::new(&env, KEEPER_ROLE);
        access_control::grant_role_no_auth(&env, &admin, &keeper_role, &admin);

        storage::set_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 10,
                max_borrow_positions: 10,
            },
        );

        // Pause at construct; operator must `unpause` after wiring
        // aggregator, accumulator, pool template, oracles, and markets.
        // `upgrade` applies the same auto-pause.
        stellar_contract_utils::pausable::pause(&env);
    }

    // -----------------------------------------------------------------------
    // Admin-only upgrade
    // -----------------------------------------------------------------------

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        stellar_contract_utils::pausable::pause(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    // -----------------------------------------------------------------------
    // Pause / unpause
    // -----------------------------------------------------------------------

    #[only_owner]
    pub fn pause(env: Env) {
        stellar_contract_utils::pausable::pause(&env);
    }

    #[only_owner]
    pub fn unpause(env: Env) {
        stellar_contract_utils::pausable::unpause(&env);
    }

    // -----------------------------------------------------------------------
    // Supply
    // -----------------------------------------------------------------------

    pub fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        assets: Vec<(Address, i128)>,
    ) -> u64 {
        positions::supply::process_supply(&env, &caller, account_id, e_mode_category, &assets)
    }

    // -----------------------------------------------------------------------
    // Borrow
    // -----------------------------------------------------------------------

    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>) {
        positions::borrow::borrow_batch(&env, &caller, account_id, &borrows);
    }

    // -----------------------------------------------------------------------
    // Withdraw
    // -----------------------------------------------------------------------

    pub fn withdraw(env: Env, caller: Address, account_id: u64, withdrawals: Vec<(Address, i128)>) {
        positions::withdraw::process_withdraw(&env, &caller, account_id, &withdrawals);
    }

    // -----------------------------------------------------------------------
    // Repay
    // -----------------------------------------------------------------------

    pub fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(Address, i128)>) {
        positions::repay::process_repay(&env, &caller, account_id, &payments);
    }

    // -----------------------------------------------------------------------
    // Liquidation
    // -----------------------------------------------------------------------

    pub fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) {
        positions::liquidation::process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    // -----------------------------------------------------------------------
    // Flash Loans
    // -----------------------------------------------------------------------

    pub fn flash_loan(
        env: Env,
        caller: Address,
        asset: Address,
        amount: i128,
        receiver: Address,
        data: Bytes,
    ) {
        flash_loan::process_flash_loan(&env, &caller, &asset, amount, &receiver, &data);
    }

    // -----------------------------------------------------------------------
    // Strategies (Multiply, Long, Short)
    // -----------------------------------------------------------------------

    pub fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        collateral_token: Address,
        debt_to_flash_loan: i128,
        debt_token: Address,
        mode: PositionMode,
        steps: SwapSteps,
        initial_payment: Option<(Address, i128)>,
        convert_steps: Option<SwapSteps>,
    ) -> u64 {
        strategy::process_multiply(
            &env,
            &caller,
            account_id,
            e_mode_category,
            &collateral_token,
            debt_to_flash_loan,
            &debt_token,
            mode,
            &steps,
            initial_payment,
            convert_steps,
        )
    }

    pub fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt_token: Address,
        amount: i128,
        new_debt_token: Address,
        steps: SwapSteps,
    ) {
        strategy::process_swap_debt(
            &env,
            &caller,
            account_id,
            &existing_debt_token,
            amount,
            &new_debt_token,
            &steps,
        );
    }

    pub fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current_collateral: Address,
        amount: i128,
        new_collateral: Address,
        steps: SwapSteps,
    ) {
        strategy::process_swap_collateral(
            &env,
            &caller,
            account_id,
            &current_collateral,
            amount,
            &new_collateral,
            &steps,
        );
    }

    pub fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral_token: Address,
        collateral_amount: i128,
        debt_token: Address,
        steps: SwapSteps,
        close_position: bool,
    ) {
        strategy::process_repay_debt_with_collateral(
            &env,
            &caller,
            account_id,
            &collateral_token,
            collateral_amount,
            &debt_token,
            &steps,
            close_position,
        );
    }

    // -----------------------------------------------------------------------
    // Index management (KEEPER role)
    // -----------------------------------------------------------------------

    #[only_role(caller, "KEEPER")]
    pub fn update_indexes(env: Env, caller: Address, assets: Vec<Address>) {
        validation::require_not_paused(&env);
        validation::require_not_flash_loaning(&env);

        let mut cache = cache::ControllerCache::new(&env, true);
        utils::sync_market_indexes(&env, &mut cache, &assets);

        // Keep pool list entries alive: write-once data needs periodic TTL bumps.
        storage::bump_pools_list(&env);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_shared_state(env: Env, caller: Address, assets: Vec<Address>) {
        let _ = caller;
        router::keepalive_shared_state(&env, &assets);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_accounts(env: Env, caller: Address, account_ids: Vec<u64>) {
        let _ = caller;
        router::keepalive_accounts(&env, &account_ids);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_pools(env: Env, caller: Address, assets: Vec<Address>) {
        let _ = caller;
        router::keepalive_pools(&env, &assets);
    }

    // -----------------------------------------------------------------------
    // Bad debt cleanup (KEEPER role)
    // -----------------------------------------------------------------------

    #[only_role(caller, "KEEPER")]
    pub fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        validation::require_not_paused(&env);
        validation::require_not_flash_loaning(&env);

        positions::liquidation::clean_bad_debt_standalone(&env, account_id);
    }

    // -----------------------------------------------------------------------
    // Position threshold propagation (KEEPER role)
    // -----------------------------------------------------------------------

    #[only_role(caller, "KEEPER")]
    pub fn update_account_threshold(
        env: Env,
        caller: Address,
        asset: Address,
        has_risks: bool,
        account_ids: Vec<u64>,
    ) {
        validation::require_not_paused(&env);
        validation::require_not_flash_loaning(&env);
        validation::require_asset_supported(&env, &asset);

        // Risk-adjusting path: a threshold tightening can tip a position into
        // liquidation, so oracle prices must stay within tight tolerance.
        let mut cache = cache::ControllerCache::new(&env, false);

        let base_config = cache.cached_asset_config(&asset);
        let price_feed = cache.cached_price(&asset);
        let controller_addr = env.current_contract_address();

        for account_id in account_ids {
            // Clone per account so e-mode overrides stay account-specific.
            let mut account_asset_config = base_config.clone();

            supply::update_position_threshold(
                &env,
                account_id,
                &asset,
                has_risks,
                &mut account_asset_config,
                &controller_addr,
                &price_feed,
                &mut cache,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Role management (owner-only)
    // -----------------------------------------------------------------------

    #[only_owner]
    pub fn grant_role(env: Env, account: Address, role: Symbol) {
        let owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        access_control::grant_role_no_auth(&env, &account, &role, &owner);
    }

    #[only_owner]
    pub fn revoke_role(env: Env, account: Address, role: Symbol) {
        let owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        access_control::revoke_role_no_auth(&env, &account, &role, &owner);
    }

    pub fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access_control::has_role(&env, &account, &role).is_some()
    }

    // -----------------------------------------------------------------------
    // Two-step ownership transfer
    // -----------------------------------------------------------------------

    #[only_owner]
    pub fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        let current_owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));

        stellar_access::role_transfer::transfer_role(
            &env,
            &new_owner,
            &ownable::OwnableStorageKey::PendingOwner,
            live_until_ledger,
        );
        ownable::emit_ownership_transfer(&env, &current_owner, &new_owner, live_until_ledger);
        sync_pending_admin_transfer(&env, &new_owner, live_until_ledger);
    }

    pub fn accept_ownership(env: Env) {
        let previous_owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        ownable::accept_ownership(&env);
        let new_owner = ownable::get_owner(&env)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }

    // -----------------------------------------------------------------------
    // Admin & Oracle Config
    // -----------------------------------------------------------------------

    #[stellar_macros::only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        config::set_aggregator(&env, addr);
    }

    #[stellar_macros::only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        config::set_accumulator(&env, addr);
    }

    #[stellar_macros::only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        config::set_liquidity_pool_template(&env, hash);
    }

    #[stellar_macros::only_owner]
    pub fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfig) {
        config::edit_asset_config(&env, asset, cfg);
    }

    #[stellar_macros::only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        config::set_position_limits(&env, limits);
    }

    #[stellar_macros::only_owner]
    pub fn add_e_mode_category(env: Env, ltv: i128, threshold: i128, bonus: i128) -> u32 {
        config::add_e_mode_category(&env, ltv, threshold, bonus)
    }

    #[stellar_macros::only_owner]
    pub fn edit_e_mode_category(env: Env, id: u32, ltv: i128, threshold: i128, bonus: i128) {
        config::edit_e_mode_category(&env, id, ltv, threshold, bonus);
    }

    #[stellar_macros::only_owner]
    pub fn remove_e_mode_category(env: Env, id: u32) {
        config::remove_e_mode_category(&env, id);
    }

    #[stellar_macros::only_owner]
    pub fn add_asset_to_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        config::add_asset_to_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[stellar_macros::only_owner]
    pub fn edit_asset_in_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        config::edit_asset_in_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[stellar_macros::only_owner]
    pub fn remove_asset_from_e_mode(env: Env, asset: Address, category_id: u32) {
        config::remove_asset_from_e_mode(&env, asset, category_id);
    }

    /// Admin: approve a token address to back a new liquidity pool.
    ///
    /// The Soroban SDK exposes no runtime lookup of a deployed contract's
    /// Wasm hash, so the allow-list keys by token contract address rather
    /// than by Wasm hash. The emitted event still carries a `BytesN<32>` for
    /// schema compatibility, derived deterministically from the address.
    #[stellar_macros::only_owner]
    pub fn approve_token_wasm(env: Env, token: Address) {
        crate::storage::set_token_approved(&env, &token, true);
        let wasm_hash = env
            .crypto()
            .keccak256(&soroban_sdk::xdr::ToXdr::to_xdr(&token, &env))
            .into();
        emit_approve_token_wasm(
            &env,
            ApproveTokenWasmEvent {
                wasm_hash,
                approved: true,
            },
        );
    }

    #[stellar_macros::only_owner]
    pub fn revoke_token_wasm(env: Env, token: Address) {
        crate::storage::set_token_approved(&env, &token, false);
        let wasm_hash = env
            .crypto()
            .keccak256(&soroban_sdk::xdr::ToXdr::to_xdr(&token, &env))
            .into();
        emit_approve_token_wasm(
            &env,
            ApproveTokenWasmEvent {
                wasm_hash,
                approved: false,
            },
        );
    }

    #[stellar_macros::only_role(caller, "ORACLE")]
    pub fn configure_market_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) {
        let _ = caller;
        config::configure_market_oracle(&env, asset, cfg);
    }

    #[stellar_macros::only_role(caller, "ORACLE")]
    pub fn edit_oracle_tolerance(
        env: Env,
        caller: Address,
        asset: Address,
        first_tolerance: i128,
        last_tolerance: i128,
    ) {
        let _ = caller;
        config::edit_oracle_tolerance(&env, asset, first_tolerance, last_tolerance);
    }

    #[stellar_macros::only_role(caller, "ORACLE")]
    pub fn disable_token_oracle(env: Env, caller: Address, asset: Address) {
        let _ = caller;
        config::disable_token_oracle(&env, asset);
    }

    // -----------------------------------------------------------------------
    // Market admin and revenue operations
    // -----------------------------------------------------------------------

    #[stellar_macros::only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParams,
        config: AssetConfig,
    ) -> Address {
        router::create_liquidity_pool(&env, &asset, &params, &config)
    }

    #[stellar_macros::only_owner]
    pub fn upgrade_pool_params(
        env: Env,
        asset: Address,
        max_borrow_rate: i128,
        base_borrow_rate: i128,
        slope1: i128,
        slope2: i128,
        slope3: i128,
        mid_utilization: i128,
        optimal_utilization: i128,
        reserve_factor: i128,
    ) {
        router::upgrade_liquidity_pool_params(
            &env,
            &asset,
            max_borrow_rate,
            base_borrow_rate,
            slope1,
            slope2,
            slope3,
            mid_utilization,
            optimal_utilization,
            reserve_factor,
        );
    }

    #[stellar_macros::only_owner]
    pub fn upgrade_pool(env: Env, asset: Address, new_wasm_hash: BytesN<32>) {
        router::upgrade_liquidity_pool(&env, &asset, new_wasm_hash);
    }

    #[stellar_macros::only_role(caller, "REVENUE")]
    pub fn claim_revenue(env: Env, caller: Address, assets: Vec<Address>) -> Vec<i128> {
        let _ = caller;
        validation::require_not_flash_loaning(&env);
        router::claim_revenue(&env, assets)
    }

    #[stellar_macros::only_role(caller, "REVENUE")]
    pub fn add_rewards(env: Env, caller: Address, rewards: Vec<(Address, i128)>) {
        validation::require_not_flash_loaning(&env);
        router::add_rewards_batch(&env, &caller, rewards);
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    pub fn can_be_liquidated(env: Env, account_id: u64) -> bool {
        views::can_be_liquidated(&env, account_id)
    }

    pub fn health_factor(env: Env, account_id: u64) -> i128 {
        views::health_factor(&env, account_id)
    }

    pub fn total_collateral_in_usd(env: Env, account_id: u64) -> i128 {
        views::total_collateral_in_usd(&env, account_id)
    }

    pub fn total_borrow_in_usd(env: Env, account_id: u64) -> i128 {
        views::total_borrow_in_usd(&env, account_id)
    }

    pub fn collateral_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128 {
        views::collateral_amount_for_token(&env, account_id, &asset)
    }

    pub fn borrow_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128 {
        views::borrow_amount_for_token(&env, account_id, &asset)
    }

    pub fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (Vec<AccountPosition>, Vec<AccountPosition>) {
        views::get_account_positions(&env, account_id)
    }

    pub fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        views::get_account_attributes(&env, account_id)
    }

    pub fn get_market_config(env: Env, asset: Address) -> MarketConfig {
        views::get_market_config_view(&env, &asset)
    }

    pub fn get_e_mode_category(env: Env, category_id: u32) -> EModeCategory {
        views::get_emode_category_view(&env, category_id)
    }

    pub fn get_isolated_debt(env: Env, asset: Address) -> i128 {
        views::get_isolated_debt_view(&env, &asset)
    }

    pub fn get_all_markets_detailed(
        env: Env,
        assets: Vec<Address>,
    ) -> Vec<AssetExtendedConfigView> {
        views::get_all_markets_detailed(&env, &assets)
    }

    pub fn get_all_market_indexes_detailed(env: Env, assets: Vec<Address>) -> Vec<MarketIndexView> {
        views::get_all_market_indexes_detailed(&env, &assets)
    }

    pub fn liquidation_estimations_detailed(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) -> LiquidationEstimate {
        views::liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    pub fn liquidation_collateral_available(env: Env, account_id: u64) -> i128 {
        views::liquidation_collateral_available(&env, account_id)
    }

    pub fn ltv_collateral_in_usd(env: Env, account_id: u64) -> i128 {
        views::ltv_collateral_in_usd(&env, account_id)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{
        AccountPosition, AccountPositionType, AssetConfig, ExchangeSource, MarketConfig,
        MarketStatus, OraclePriceFluctuation, OracleProviderConfig, OracleType, PositionLimits,
        ReflectorAssetKind,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    struct TestSetup {
        env: Env,
        admin: Address,
        contract: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let contract = env.register(crate::Controller, (admin.clone(),));

            TestSetup {
                env,
                admin,
                contract,
            }
        }

        fn client(&self) -> crate::ControllerClient<'_> {
            crate::ControllerClient::new(&self.env, &self.contract)
        }

        fn setup_reflector(&self, asset: &Address) -> Address {
            let reflector = self
                .env
                .register(crate::helpers::testutils::TestReflector, ());
            let r_client =
                crate::helpers::testutils::TestReflectorClient::new(&self.env, &reflector);
            r_client.set_spot(
                &crate::helpers::testutils::TestReflectorAsset::Stellar(asset.clone()),
                &10_0000000_0000000i128,
                &10_000,
            );
            reflector
        }

        fn sample_asset_config(&self) -> AssetConfig {
            AssetConfig {
                loan_to_value_bps: 7500,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                is_collateralizable: true,
                is_borrowable: true,
                e_mode_enabled: false,
                is_isolated_asset: false,
                is_siloed_borrowing: false,
                is_flashloanable: true,
                isolation_borrow_enabled: false,
                isolation_debt_ceiling_usd_wad: 0,
                flashloan_fee_bps: 9,
                borrow_cap: i128::MAX,
                supply_cap: i128::MAX,
            }
        }

        fn seed_market_config(&self, asset: &Address) {
            self.env.as_contract(&self.contract, || {
                let default_oracle = OracleProviderConfig {
                    base_asset: asset.clone(),
                    oracle_type: OracleType::None,
                    exchange_source: ExchangeSource::SpotOnly,
                    asset_decimals: 7,
                    tolerance: OraclePriceFluctuation {
                        first_upper_ratio_bps: 0,
                        first_lower_ratio_bps: 0,
                        last_upper_ratio_bps: 0,
                        last_lower_ratio_bps: 0,
                    },
                    max_price_stale_seconds: 900,
                };
                let market = MarketConfig {
                    status: MarketStatus::PendingOracle,
                    asset_config: self.sample_asset_config(),
                    pool_address: Address::generate(&self.env),
                    oracle_config: default_oracle,
                    cex_oracle: None,
                    cex_asset_kind: ReflectorAssetKind::Stellar,
                    cex_symbol: Symbol::new(&self.env, ""),
                    cex_decimals: 0,
                    dex_oracle: None,
                    dex_asset_kind: ReflectorAssetKind::Stellar,
                    dex_symbol: Symbol::new(&self.env, ""),
                    dex_decimals: 0,
                    twap_records: 0,
                };
                storage::set_market_config(&self.env, asset, &market);
            });
        }
    }

    // -----------------------------------------------------------------------
    // Test: constructor sets admin and position limits
    // -----------------------------------------------------------------------
    #[test]
    fn test_constructor_sets_admin_and_limits() {
        let t = TestSetup::new();

        t.env.as_contract(&t.contract, || {
            // Verify owner storage.
            let stored_owner = ownable::get_owner(&t.env);
            assert_eq!(stored_owner, Some(t.admin.clone()));

            // Verify AccessControl admin.
            let stored_ac_admin = access_control::get_admin(&t.env);
            assert_eq!(stored_ac_admin, Some(t.admin.clone()));

            // M-02: only KEEPER is granted at construct. REVENUE and ORACLE
            // require explicit `grant_role` after deploy.
            assert!(
                access_control::has_role(&t.env, &t.admin, &Symbol::new(&t.env, KEEPER_ROLE))
                    .is_some()
            );
            assert!(
                access_control::has_role(&t.env, &t.admin, &Symbol::new(&t.env, REVENUE_ROLE))
                    .is_none(),
                "M-02: REVENUE must NOT be granted at construct"
            );
            assert!(
                access_control::has_role(&t.env, &t.admin, &Symbol::new(&t.env, ORACLE_ROLE))
                    .is_none(),
                "M-02: ORACLE must NOT be granted at construct"
            );

            // M-03: contract is paused after construct.
            assert!(
                stellar_contract_utils::pausable::paused(&t.env),
                "M-03: contract must be paused at construct"
            );

            // Verify default position limits.
            let limits = storage::get_position_limits(&t.env);
            assert_eq!(limits.max_supply_positions, 10);
            assert_eq!(limits.max_borrow_positions, 10);
        });
    }

    // -----------------------------------------------------------------------
    // Test: create_account increments nonce, stores owner and attrs
    // -----------------------------------------------------------------------
    #[test]
    fn test_create_account() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);

        t.env.as_contract(&t.contract, || {
            let id1 = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
            assert_eq!(id1, 1);

            let id2 = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
            assert_eq!(id2, 2);

            let account = storage::get_account(&t.env, id1);

            // Verify owner.
            assert_eq!(account.owner, owner);

            // Verify attrs.
            assert!(!account.is_isolated);
            assert_eq!(account.e_mode_category_id, 0);
            assert_eq!(account.mode, PositionMode::Normal);

            // Verify empty position maps.
            assert_eq!(account.supply_positions.len(), 0);
            assert_eq!(account.borrow_positions.len(), 0);
        });
    }

    // -----------------------------------------------------------------------
    // Test: remove_account cleans up storage
    // -----------------------------------------------------------------------
    #[test]
    fn test_remove_account() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);

        // Verify storage is cleaned.
        t.env.as_contract(&t.contract, || {
            let id = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
            assert_eq!(id, 1);

            utils::remove_account(&t.env, id);

            let exists = storage::try_get_account(&t.env, id).is_some();
            assert!(!exists, "account should be removed");
        });
    }

    // -----------------------------------------------------------------------
    // Test: store_position adds to position list
    // -----------------------------------------------------------------------
    #[test]
    fn test_store_position() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);
        let asset = Address::generate(&t.env);

        t.env.as_contract(&t.contract, || {
            let id = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
            let position = AccountPosition {
                position_type: AccountPositionType::Deposit,
                asset: asset.clone(),
                scaled_amount_ray: 1_000_000,
                account_id: id,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            };
            let mut account = storage::get_account(&t.env, id);
            update::store_position(&mut account, &position);
            storage::set_account(&t.env, id, &account);

            // Check that the position map has the asset.
            let account = storage::get_account(&t.env, id);
            assert_eq!(account.supply_positions.len(), 1);
            let stored = account.supply_positions.get(asset.clone());
            assert!(stored.is_some());
            assert_eq!(stored.unwrap().scaled_amount_ray, 1_000_000);

            // Store the same asset again; the position must not duplicate.
            let mut account = storage::get_account(&t.env, id);
            update::store_position(&mut account, &position);
            storage::set_account(&t.env, id, &account);
            let account = storage::get_account(&t.env, id);
            assert_eq!(account.supply_positions.len(), 1);
        });
    }

    // -----------------------------------------------------------------------
    // Test: update_or_remove_position removes when zero
    // -----------------------------------------------------------------------
    #[test]
    fn test_update_or_remove_position_zero() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);
        let asset = Address::generate(&t.env);

        t.env.as_contract(&t.contract, || {
            let id = utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None);
            // Store a position first.
            let position = AccountPosition {
                position_type: AccountPositionType::Deposit,
                asset: asset.clone(),
                scaled_amount_ray: 1_000_000,
                account_id: id,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            };
            let mut account = storage::get_account(&t.env, id);
            update::store_position(&mut account, &position);
            storage::set_account(&t.env, id, &account);

            // Now update with zero amount.
            let zero_position = AccountPosition {
                position_type: AccountPositionType::Deposit,
                asset: asset.clone(),
                scaled_amount_ray: 0,
                account_id: id,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            };
            let mut account = storage::get_account(&t.env, id);
            update::update_or_remove_position(&mut account, &zero_position);
            storage::set_account(&t.env, id, &account);

            // Position should be removed.
            let account = storage::get_account(&t.env, id);
            let stored = account.supply_positions.get(asset.clone());
            assert!(stored.is_none());

            // Position map should be empty.
            assert_eq!(account.supply_positions.len(), 0);
        });
    }

    // -----------------------------------------------------------------------
    // Test: config endpoints require admin auth
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic]
    fn test_config_requires_admin() {
        let env = Env::default();
        // Do NOT mock all auths.
        let admin = Address::generate(&env);
        let contract = env.register(Controller, (admin.clone(),));
        let client = ControllerClient::new(&env, &contract);

        let _non_admin = Address::generate(&env);
        let limits = PositionLimits {
            max_supply_positions: 10,
            max_borrow_positions: 10,
        };
        // Must panic: non_admin is not admin.
        client.set_position_limits(&limits);
    }

    // -----------------------------------------------------------------------
    // Test: edit_asset_config validates threshold > LTV
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_edit_asset_config_threshold_validation() {
        let t = TestSetup::new();
        let client = t.client();
        let asset = Address::generate(&t.env);

        let mut bad_config = t.sample_asset_config();
        // Set threshold <= LTV to trigger validation.
        bad_config.loan_to_value_bps = 8000;
        bad_config.liquidation_threshold_bps = 8000; // equal, must fail

        client.edit_asset_config(&asset, &bad_config);
    }

    // -----------------------------------------------------------------------
    // Test: edit_asset_config succeeds with valid params
    // -----------------------------------------------------------------------
    #[test]
    fn test_edit_asset_config_valid() {
        let t = TestSetup::new();
        let client = t.client();
        let asset = Address::generate(&t.env);

        // Seed a default market so edit_asset_config can read-modify-write.
        t.seed_market_config(&asset);

        let config = t.sample_asset_config();
        client.edit_asset_config(&asset, &config);

        t.env.as_contract(&t.contract, || {
            let market = storage::get_market_config(&t.env, &asset);
            assert_eq!(market.asset_config.loan_to_value_bps, 7500);
            assert_eq!(market.asset_config.liquidation_threshold_bps, 8000);
        });
    }

    #[test]
    fn test_edit_asset_config_preserves_existing_emode_flag() {
        let t = TestSetup::new();
        let client = t.client();
        let asset = Address::generate(&t.env);

        t.seed_market_config(&asset);

        t.env.as_contract(&t.contract, || {
            let mut market = storage::get_market_config(&t.env, &asset);
            market.asset_config.e_mode_enabled = true;
            storage::set_market_config(&t.env, &asset, &market);
        });

        let mut config = t.sample_asset_config();
        config.e_mode_enabled = false;
        client.edit_asset_config(&asset, &config);

        t.env.as_contract(&t.contract, || {
            let market = storage::get_market_config(&t.env, &asset);
            assert!(market.asset_config.e_mode_enabled);
        });
    }

    // -----------------------------------------------------------------------
    // Test: add_e_mode_category auto-increments ID
    // -----------------------------------------------------------------------
    #[test]
    fn test_add_e_mode_category_auto_increment() {
        let t = TestSetup::new();
        let client = t.client();

        let id1 = client.add_e_mode_category(&9700i128, &9800i128, &200i128);
        assert_eq!(id1, 1);

        let id2 = client.add_e_mode_category(&9500i128, &9600i128, &300i128);
        assert_eq!(id2, 2);

        t.env.as_contract(&t.contract, || {
            // Verify stored category.
            let cat = storage::get_emode_category(&t.env, id1);
            assert_eq!(cat.category_id, 1);
            assert_eq!(cat.loan_to_value_bps, 9700);
            assert_eq!(cat.liquidation_threshold_bps, 9800);
            assert!(!cat.is_deprecated);
        });
    }

    // -----------------------------------------------------------------------
    // Test: add_e_mode_category rejects threshold <= LTV
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic(expected = "Error(Contract, #113)")]
    fn test_add_e_mode_category_bad_params() {
        let t = TestSetup::new();
        let client = t.client();

        // threshold == ltv must fail.
        client.add_e_mode_category(&9800i128, &9800i128, &200i128);
    }

    // -----------------------------------------------------------------------
    // Test: remove_e_mode_category deprecates
    // -----------------------------------------------------------------------
    #[test]
    fn test_remove_e_mode_category() {
        let t = TestSetup::new();
        let client = t.client();

        let id = client.add_e_mode_category(&9700i128, &9800i128, &200i128);
        client.remove_e_mode_category(&id);

        t.env.as_contract(&t.contract, || {
            let cat = storage::get_emode_category(&t.env, id);
            assert!(cat.is_deprecated);
        });
    }

    // -----------------------------------------------------------------------
    // Test: add_asset_to_emode rejects deprecated category
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic(expected = "Error(Contract, #301)")]
    fn test_add_asset_to_deprecated_e_mode() {
        let t = TestSetup::new();
        let client = t.client();
        let asset = Address::generate(&t.env);

        let id = client.add_e_mode_category(&9700i128, &9800i128, &200i128);
        client.remove_e_mode_category(&id);

        // Must fail: the category is deprecated.
        client.add_asset_to_e_mode_category(&asset, &id, &true, &true);
    }

    // -----------------------------------------------------------------------
    // Test: create isolated account stores isolated asset
    // -----------------------------------------------------------------------
    #[test]
    fn test_create_isolated_account() {
        let t = TestSetup::new();
        let owner = Address::generate(&t.env);
        let iso_asset = Address::generate(&t.env);

        t.env.as_contract(&t.contract, || {
            let id = utils::create_account(
                &t.env,
                &owner,
                0,
                PositionMode::Normal,
                true,
                Some(iso_asset.clone()),
            );
            assert_eq!(id, 1);
            let account = storage::get_account(&t.env, id);
            assert!(account.is_isolated);

            // Check that the isolated asset is stored on the account.
            assert_eq!(account.isolated_asset, Some(iso_asset));
        });
    }

    // -----------------------------------------------------------------------
    // Test: position limit enforcement
    // -----------------------------------------------------------------------
    #[test]
    fn test_position_limit_enforcement() {
        let t = TestSetup::new();
        let client = t.client();
        let owner = Address::generate(&t.env);
        let id = t.env.as_contract(&t.contract, || {
            utils::create_account(&t.env, &owner, 0, PositionMode::Normal, false, None)
        });

        // Set tight limits.
        client.set_position_limits(&PositionLimits {
            max_supply_positions: 2,
            max_borrow_positions: 2,
        });

        // Store two supply positions inside contract context.
        let asset1 = Address::generate(&t.env);
        let asset2 = Address::generate(&t.env);

        t.env.as_contract(&t.contract, || {
            let mut account = storage::get_account(&t.env, id);
            for asset in [&asset1, &asset2] {
                let pos = AccountPosition {
                    position_type: AccountPositionType::Deposit,
                    asset: asset.clone(),
                    scaled_amount_ray: 1000,
                    account_id: id,
                    liquidation_threshold_bps: 8000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    loan_to_value_bps: 7500,
                };
                update::store_position(&mut account, &pos);
            }
            storage::set_account(&t.env, id, &account);

            // The limit check must now fail for a third position.
            let account = storage::get_account(&t.env, id);
            assert_eq!(account.supply_positions.len(), 2);
            let limits = storage::get_position_limits(&t.env);
            assert!(
                account.supply_positions.len() >= limits.max_supply_positions,
                "should be at limit"
            );
        });
    }

    // -----------------------------------------------------------------------
    // Test: oracle tolerance validation
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic(expected = "Error(Contract, #207)")]
    fn test_edit_oracle_tolerance_bad_first() {
        let t = TestSetup::new();
        let client = t.client();
        // M-02 + M-03 hardening: grant ORACLE role and unpause.
        client.grant_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE));
        client.unpause();
        let asset = t
            .env
            .register_stellar_asset_contract_v2(Address::generate(&t.env))
            .address()
            .clone();

        // Seed a default market so configure_market_oracle can read-modify-write.
        t.seed_market_config(&asset);
        let oracle_config = MarketOracleConfigInput {
            exchange_source: ExchangeSource::SpotVsTwap,
            max_price_stale_seconds: 900,
            first_tolerance_bps: 200,
            last_tolerance_bps: 500,
            cex_oracle: t.setup_reflector(&asset),
            cex_asset_kind: ReflectorAssetKind::Stellar,
            cex_symbol: Symbol::new(&t.env, "USDC"),
            dex_oracle: None,
            dex_asset_kind: ReflectorAssetKind::Stellar,
            dex_symbol: Symbol::new(&t.env, ""),
            twap_records: 3,
        };
        client.configure_market_oracle(&t.admin, &asset, &oracle_config);

        // first=10 falls below MIN_FIRST_TOLERANCE.
        client.edit_oracle_tolerance(&t.admin, &asset, &10, &500);
    }

    // -----------------------------------------------------------------------
    // Test: unsupported oracle modes are rejected at configuration time
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic(expected = "Error(Contract, #11)")]
    fn test_configure_market_oracle_rejects_missing_dual_oracle_dex() {
        let t = TestSetup::new();
        let client = t.client();
        // M-02 + M-03 hardening: grant ORACLE role and unpause.
        client.grant_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE));
        client.unpause();
        let asset = t
            .env
            .register_stellar_asset_contract_v2(Address::generate(&t.env))
            .address()
            .clone();

        t.seed_market_config(&asset);

        let oracle_config = MarketOracleConfigInput {
            exchange_source: ExchangeSource::DualOracle,
            max_price_stale_seconds: 900,
            first_tolerance_bps: 200,
            last_tolerance_bps: 500,
            cex_oracle: t.setup_reflector(&asset),
            cex_asset_kind: ReflectorAssetKind::Stellar,
            cex_symbol: Symbol::new(&t.env, "USDC"),
            dex_oracle: None,
            dex_asset_kind: ReflectorAssetKind::Stellar,
            dex_symbol: Symbol::new(&t.env, ""),
            twap_records: 3,
        };

        client.configure_market_oracle(&t.admin, &asset, &oracle_config);
    }

    // -----------------------------------------------------------------------
    // Test: pause blocks user endpoints, unpause re-enables them
    // -----------------------------------------------------------------------
    #[test]
    fn test_pause_and_unpause() {
        let t = TestSetup::new();
        let client = t.client();

        // M-03: paused at construct. Operator must unpause before any
        // user-facing flow runs.
        t.env.as_contract(&t.contract, || {
            assert!(stellar_contract_utils::pausable::paused(&t.env));
        });
        client.unpause();
        t.env.as_contract(&t.contract, || {
            assert!(!stellar_contract_utils::pausable::paused(&t.env));
        });

        // Pause.
        client.pause();
        t.env.as_contract(&t.contract, || {
            assert!(stellar_contract_utils::pausable::paused(&t.env));
        });

        // Unpause.
        client.unpause();
        t.env.as_contract(&t.contract, || {
            assert!(!stellar_contract_utils::pausable::paused(&t.env));
        });
    }

    // -----------------------------------------------------------------------
    // Test: require_not_paused panics when paused (error #1000)
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic(expected = "Error(Contract, #1000)")]
    fn test_require_not_paused_blocks_when_paused() {
        let t = TestSetup::new();

        t.env.as_contract(&t.contract, || {
            stellar_contract_utils::pausable::pause(&t.env);
            validation::require_not_paused(&t.env);
        });
    }

    // -----------------------------------------------------------------------
    // Test: require_not_paused passes when not paused
    // -----------------------------------------------------------------------
    #[test]
    fn test_require_not_paused_passes_when_unpaused() {
        let t = TestSetup::new();

        // M-03: constructor pauses. Unpause first, then verify the guard
        // passes.
        t.client().unpause();
        t.env.as_contract(&t.contract, || {
            validation::require_not_paused(&t.env);
        });
    }

    // -----------------------------------------------------------------------
    // Test: pause/unpause require admin auth
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic]
    fn test_pause_requires_admin() {
        let env = Env::default();
        // Do NOT mock all auths -- auth must fail.
        let admin = Address::generate(&env);
        let contract = env.register(Controller, (admin.clone(),));
        let client = ControllerClient::new(&env, &contract);

        // Call pause without auth -- must panic.
        client.pause();
    }

    // -----------------------------------------------------------------------
    // Test: role-based access control
    // -----------------------------------------------------------------------
    #[test]
    fn test_role_based_access() {
        let t = TestSetup::new();
        let client = t.client();

        // M-02: only KEEPER granted at construct. Operator must explicitly
        // grant REVENUE and ORACLE post-deploy.
        assert!(client.has_role(&t.admin, &Symbol::new(&t.env, KEEPER_ROLE)));
        assert!(!client.has_role(&t.admin, &Symbol::new(&t.env, REVENUE_ROLE)));
        assert!(!client.has_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE)));

        // Operator post-deploy hardening grants the other two roles.
        client.grant_role(&t.admin, &Symbol::new(&t.env, REVENUE_ROLE));
        client.grant_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE));
        assert!(client.has_role(&t.admin, &Symbol::new(&t.env, REVENUE_ROLE)));
        assert!(client.has_role(&t.admin, &Symbol::new(&t.env, ORACLE_ROLE)));

        // Grant KEEPER role to a bot address.
        let keeper_bot = Address::generate(&t.env);
        assert!(!client.has_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE)));

        client.grant_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE));
        assert!(client.has_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE)));

        // Bot must NOT hold other roles.
        assert!(!client.has_role(&keeper_bot, &Symbol::new(&t.env, REVENUE_ROLE)));

        // Revoke KEEPER role from the bot.
        client.revoke_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE));
        assert!(!client.has_role(&keeper_bot, &Symbol::new(&t.env, KEEPER_ROLE)));
    }

    // -----------------------------------------------------------------------
    // Test: ownership transfer keeps access-control admin and bootstrap roles aligned
    // -----------------------------------------------------------------------
    #[test]
    fn test_transfer_ownership_syncs_admin_and_roles() {
        let t = TestSetup::new();
        let client = t.client();
        let new_owner = Address::generate(&t.env);
        let live_until = t.env.ledger().sequence() + 100;

        client.transfer_ownership(&new_owner, &live_until);
        t.env.as_contract(&t.contract, || {
            assert!(stellar_access::role_transfer::has_active_pending_transfer(
                &t.env,
                &access_control::AccessControlStorageKey::PendingAdmin,
            ));
        });

        client.accept_ownership();

        t.env.as_contract(&t.contract, || {
            assert_eq!(ownable::get_owner(&t.env), Some(new_owner.clone()));
            assert_eq!(access_control::get_admin(&t.env), Some(new_owner.clone()));
        });

        for role_name in [KEEPER_ROLE, REVENUE_ROLE, ORACLE_ROLE] {
            let role = Symbol::new(&t.env, role_name);
            assert!(client.has_role(&new_owner, &role));
            assert!(!client.has_role(&t.admin, &role));
        }
    }

    // -----------------------------------------------------------------------
    // Test: canceling ownership transfer also clears the mirrored admin transfer
    // -----------------------------------------------------------------------
    #[test]
    fn test_transfer_ownership_cancel_clears_pending_admin() {
        let t = TestSetup::new();
        let client = t.client();
        let new_owner = Address::generate(&t.env);
        let live_until = t.env.ledger().sequence() + 100;

        client.transfer_ownership(&new_owner, &live_until);
        client.transfer_ownership(&new_owner, &0);

        t.env.as_contract(&t.contract, || {
            assert!(!stellar_access::role_transfer::has_active_pending_transfer(
                &t.env,
                &ownable::OwnableStorageKey::PendingOwner,
            ));
            assert!(!stellar_access::role_transfer::has_active_pending_transfer(
                &t.env,
                &access_control::AccessControlStorageKey::PendingAdmin,
            ));
            assert_eq!(access_control::get_admin(&t.env), Some(t.admin.clone()));
        });

        t.env.as_contract(&t.contract, || {
            assert_eq!(ownable::get_owner(&t.env), Some(t.admin.clone()));
        });
    }

    // -----------------------------------------------------------------------
    // Test: ownership view
    // -----------------------------------------------------------------------
    #[test]
    fn test_get_contract_owner() {
        let t = TestSetup::new();
        t.env.as_contract(&t.contract, || {
            assert_eq!(ownable::get_owner(&t.env), Some(t.admin.clone()));
        });
    }
}
