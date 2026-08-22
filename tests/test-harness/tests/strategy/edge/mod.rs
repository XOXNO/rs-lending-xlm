mod multiply;
mod pause_bypass;
mod rejections;
mod swap;

use controller::types::{ControllerKey, SpokeAssetConfig};
use soroban_sdk::token;
use soroban_sdk::Bytes;
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, eth_preset, hub_asset,
    map_try_ok_unit, map_try_ok_value, usd, usdc_preset, usdt_stable_preset, HubAssetKey,
    LendingTest, MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
    STABLECOIN_SPOKE,
};

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
