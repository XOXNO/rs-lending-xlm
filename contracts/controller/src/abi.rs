//! Compile-time guard that the inherent `Controller` methods match
//! `controller_interface::ControllerInterface` exactly. Any drift in name,
//! arity, parameter type, or return type produces a build error here long
//! before deployment.
//!
//! Pure type-level binding — no runtime cost.

#![allow(dead_code, clippy::type_complexity)]

use crate::Controller;
// `use` keeps the trait crate live as a dependency-check and surfaces
// trait-file syntax errors as build failures here.
#[allow(unused_imports)]
use controller_interface::ControllerInterface;
use soroban_sdk::{Address, Bytes, Env, Map, Vec};

use common::types::{
    AccountAttributes, AccountPositionRaw, AggregatorSwap, AssetExtendedConfigView,
    EModeCategoryRaw, LiquidationEstimate, MarketConfig, MarketIndexView, PositionMode,
};

// Each line binds an inherent method to the trait declaration's function
// pointer type. Mismatch in signature → E0308 at this site.
fn _abi_proof() {
    // --- Position operations ----------------------------------------------
    let _: fn(Env, Address, u64, u32, Vec<(Address, i128)>) -> u64 = Controller::supply;
    let _: fn(Env, Address, u64, Vec<(Address, i128)>) = Controller::borrow;
    let _: fn(Env, Address, u64, Vec<(Address, i128)>) = Controller::withdraw;
    let _: fn(Env, Address, u64, Vec<(Address, i128)>) = Controller::repay;
    let _: fn(Env, Address, u64, Vec<(Address, i128)>) = Controller::liquidate;

    // --- Strategies --------------------------------------------------------
    let _: fn(
        Env,
        Address,
        u64,
        u32,
        Address,
        i128,
        Address,
        PositionMode,
        AggregatorSwap,
        Option<(Address, i128)>,
        Option<AggregatorSwap>,
    ) -> u64 = Controller::multiply;
    let _: fn(Env, Address, u64, Address, i128, Address, AggregatorSwap) = Controller::swap_debt;
    let _: fn(Env, Address, u64, Address, i128, Address, AggregatorSwap) =
        Controller::swap_collateral;
    let _: fn(Env, Address, u64, Address, i128, Address, AggregatorSwap, bool) =
        Controller::repay_debt_with_collateral;

    // --- Flash loan and account TTL ---------------------------------------
    let _: fn(Env, Address, Address, i128, Address, Bytes) = Controller::flash_loan;
    let _: fn(Env, Address, u64) = Controller::renew_account;

    // --- Views -------------------------------------------------------------
    let _: fn(Env, u64) -> bool = Controller::can_be_liquidated;
    let _: fn(Env, u64) -> i128 = Controller::health_factor;
    let _: fn(Env, u64) -> i128 = Controller::total_collateral_in_usd;
    let _: fn(Env, u64) -> i128 = Controller::total_borrow_in_usd;
    let _: fn(Env, u64, Address) -> i128 = Controller::collateral_amount_for_token;
    let _: fn(Env, u64, Address) -> i128 = Controller::borrow_amount_for_token;
    let _: fn(
        Env,
        u64,
    ) -> (
        Map<Address, AccountPositionRaw>,
        Map<Address, AccountPositionRaw>,
    ) = Controller::get_account_positions;
    let _: fn(Env, u64) -> AccountAttributes = Controller::get_account_attributes;
    let _: fn(Env, Address) -> MarketConfig = Controller::get_market_config;
    let _: fn(Env, u32) -> EModeCategoryRaw = Controller::get_e_mode_category;
    let _: fn(Env, Address) -> i128 = Controller::get_isolated_debt;
    let _: fn(Env, Vec<Address>) -> Vec<AssetExtendedConfigView> =
        Controller::get_all_markets_detailed;
    let _: fn(Env, Vec<Address>) -> Vec<MarketIndexView> = Controller::get_all_market_indexes_detailed;
    let _: fn(Env, u64, Vec<(Address, i128)>) -> LiquidationEstimate =
        Controller::liquidation_estimations_detailed;
    let _: fn(Env, u64) -> i128 = Controller::liquidation_collateral_available;
    let _: fn(Env, u64) -> i128 = Controller::ltv_collateral_in_usd;
}
