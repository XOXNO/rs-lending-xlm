//! Strategy edge-case and rejection tests.

mod multiply;
mod rejections;
mod swap;

use controller::types::{ControllerKey, SpokeAssetConfig};
use soroban_sdk::token;
use soroban_sdk::Bytes;
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, eth_preset, hub_asset,
    usd, usdc_preset, usdt_stable_preset, wbtc_preset, HubAssetKey, LendingTest, MarketPreset,
    ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, STABLECOIN_SPOKE,
};

use super::helpers::build_swap_steps;

fn dai_preset() -> MarketPreset {
    MarketPreset {
        name: "DAI",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// Flatten the nested result returned by the raw `ctrl_client().try_*` calls
/// into `Result<T, soroban_sdk::Error>` so it can feed `assert_contract_error`.
/// A host-level InvokeError (pre-contract host check) is escalated via
/// `.expect()` so host-level failures surface clearly.
fn flatten<T>(
    r: Result<Result<T, soroban_sdk::Error>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error> {
    match r {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

fn expect_host_auth_rejection<T, E>(
    label: &str,
    r: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) where
    T: core::fmt::Debug,
    E: core::fmt::Debug,
{
    match r {
        Err(_) => {}
        Ok(Ok(v)) => panic!("{label} executed without auth: {v:?}"),
        Ok(Err(e)) => panic!("{label} reached contract body without auth: {e:?}"),
    }
}

fn supply_position_params(t: &LendingTest, account_id: u64, asset_name: &str) -> (u32, u32) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        let position = map
            .get(hub_asset(asset))
            .expect("supply position should exist for asset");
        (position.loan_to_value, position.liquidation_threshold)
    })
}
