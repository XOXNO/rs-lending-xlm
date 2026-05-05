extern crate std;

use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, MAX_LIQUIDATION_BONUS, RAY, WAD};
use common::types::{InterestRateModel, MarketOracleConfigInput, ReflectorAssetKind};
use soroban_sdk::{vec, Address, Symbol};
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, EModeCategoryPreset, LendingTest, ALICE,
    DEFAULT_TOLERANCE,
};

// ---------------------------------------------------------------------------
// validate_bulk_isolation -- BulkSupplyNoIso (validation.rs:109)
// ---------------------------------------------------------------------------
//
// `validate_bulk_isolation` panics with #405 when a batch of distinct assets
// has length > 1 and the first asset is isolated, or when the account is
// isolated. Duplicate-asset batches are deduped before validation, so the
// scenario uses two distinct asset addresses.

#[test]
#[should_panic(expected = "Error(Contract, #405)")]
fn test_validate_bulk_isolation_rejects_isolated_first_asset_bulk() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000i128 * WAD;
        })
        .build();

    // Mint both tokens to ALICE then call supply with a bulk batch where the
    // first entry is the isolated asset.
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_market("USDC");
    let usdc_addr = usdc.asset.clone();
    // 10_000 USDC at 7 decimals, 1 ETH at 7 decimals (Stellar-native).
    usdc.token_admin.mint(&alice, &100_000_000_000_i128);
    let eth = t.resolve_market("ETH");
    let eth_addr = eth.asset.clone();
    eth.token_admin.mint(&alice, &10_000_000_i128);

    let assets = vec![
        &t.env,
        (usdc_addr, 100_000_000_000_i128),
        (eth_addr, 10_000_000_i128),
    ];
    t.ctrl_client().supply(&alice, &0u64, &0u32, &assets);
}

// ---------------------------------------------------------------------------
// validate_interest_rate_model invariants
// ---------------------------------------------------------------------------
//
// Driven via `upgrade_pool_params`, which calls `validate_interest_rate_model`
// directly with no other invariants in the way.

fn baseline_irm() -> InterestRateModel {
    InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY * 4 / 100,
        slope2_ray: RAY * 10 / 100,
        slope3_ray: RAY * 150 / 100,
        mid_utilization_ray: RAY * 50 / 100,
        optimal_utilization_ray: RAY * 80 / 100,
        reserve_factor_bps: 1000,
    }
}

// validation.rs:173 -- monotone-slope / cap chain rejects InvalidBorrowParams (#116).
#[test]
#[should_panic(expected = "Error(Contract, #116)")]
fn test_validate_irm_rejects_negative_base_rate() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.base_borrow_rate_ray = -1;
    t.ctrl_client().upgrade_pool_params(&asset, &irm);
}

// validation.rs:185 -- mid_utilization_ray <= 0 rejects InvalidUtilRange (#117).
#[test]
#[should_panic(expected = "Error(Contract, #117)")]
fn test_validate_irm_rejects_zero_mid_utilization() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.mid_utilization_ray = 0;
    t.ctrl_client().upgrade_pool_params(&asset, &irm);
}

// validation.rs:188 -- optimal_utilization_ray <= mid_utilization_ray rejects #117.
#[test]
#[should_panic(expected = "Error(Contract, #117)")]
fn test_validate_irm_rejects_optimal_not_above_mid() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.optimal_utilization_ray = irm.mid_utilization_ray;
    t.ctrl_client().upgrade_pool_params(&asset, &irm);
}

// validation.rs:191 -- optimal_utilization_ray >= RAY rejects OptUtilTooHigh (#118).
#[test]
#[should_panic(expected = "Error(Contract, #118)")]
fn test_validate_irm_rejects_optimal_at_or_above_ray() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.optimal_utilization_ray = RAY;
    t.ctrl_client().upgrade_pool_params(&asset, &irm);
}

// validation.rs (reserve_factor) -- reserve_factor_bps >= BPS rejects
// InvalidReserveFactor (#119).
#[test]
#[should_panic(expected = "Error(Contract, #119)")]
fn test_validate_irm_rejects_reserve_factor_at_bps() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.reserve_factor_bps = BPS as u32;
    t.ctrl_client().upgrade_pool_params(&asset, &irm);
}

// ---------------------------------------------------------------------------
// validate_asset_config invariants
// ---------------------------------------------------------------------------
//
// Driven via `edit_asset_config`. The validator is also called from
// `create_liquidity_pool`, but the edit path is shorter to set up.

// `loan_to_value_bps` is `u32`, so negative values are unrepresentable
// and the corresponding runtime branch is unreachable.

// validation.rs -- liquidation_bonus_bps > MAX_LIQUIDATION_BONUS rejects #113.
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_validate_asset_config_rejects_excessive_liq_bonus() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();
    let mut cfg = ctrl.get_market_config(&asset).asset_config;
    cfg.liquidation_bonus_bps = (MAX_LIQUIDATION_BONUS + 1) as u32;
    ctrl.edit_asset_config(&asset, &cfg);
}

// validation.rs -- isolation_debt_ceiling_usd_wad < 0 rejects InvalidBorrowParams (#116).
#[test]
#[should_panic(expected = "Error(Contract, #116)")]
fn test_validate_asset_config_rejects_negative_isolation_ceiling() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();
    let mut cfg = ctrl.get_market_config(&asset).asset_config;
    cfg.isolation_debt_ceiling_usd_wad = -1;
    ctrl.edit_asset_config(&asset, &cfg);
}

// Sanity: validator caps flashloan_fee_bps at MAX_FLASHLOAN_FEE_BPS.
#[test]
fn test_validate_asset_config_accepts_flashloan_fee_at_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();
    let mut cfg = ctrl.get_market_config(&asset).asset_config;
    cfg.flashloan_fee_bps = MAX_FLASHLOAN_FEE_BPS as u32;
    ctrl.edit_asset_config(&asset, &cfg);
    let updated = ctrl.get_market_config(&asset).asset_config;
    assert_eq!(updated.flashloan_fee_bps, MAX_FLASHLOAN_FEE_BPS as u32);
}

// ---------------------------------------------------------------------------
// configure_market_oracle error paths (config.rs:453, 479, 524)
// ---------------------------------------------------------------------------

fn base_oracle_config(t: &LendingTest) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        exchange_source: common::types::ExchangeSource::SpotVsTwap,
        max_price_stale_seconds: 900,
        first_tolerance_bps: DEFAULT_TOLERANCE.first_upper_bps,
        last_tolerance_bps: DEFAULT_TOLERANCE.last_upper_bps,
        cex_oracle: t.mock_reflector.clone(),
        cex_asset_kind: ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(&t.env, ""),
        dex_oracle: None,
        dex_asset_kind: ReflectorAssetKind::Stellar,
        dex_symbol: Symbol::new(&t.env, ""),
        twap_records: 3,
    }
}

// config.rs:524 -- max_price_stale_seconds < 60 rejects InvalidStalenessConfig (#218).
#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_configure_market_oracle_rejects_low_staleness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.max_price_stale_seconds = 30; // Below the 60-second floor.
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// config.rs:524 -- max_price_stale_seconds > 86_400 rejects #218 (upper bound).
#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_configure_market_oracle_rejects_high_staleness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.max_price_stale_seconds = 86_401; // Above the 24-hour ceiling.
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// config.rs:453 -- twap_records > 12 rejects InvalidOracleTokenType (#204).
#[test]
#[should_panic(expected = "Error(Contract, #204)")]
fn test_configure_market_oracle_rejects_excessive_twap_records() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.twap_records = 13;
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// config.rs:479 -- DualOracle without dex_oracle rejects InvalidExchangeSrc (#11).
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_configure_market_oracle_rejects_dual_without_dex() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.exchange_source = common::types::ExchangeSource::DualOracle;
    cfg.dex_oracle = None;
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// ---------------------------------------------------------------------------
// emode.rs:95 -- EModeCategoryDeprecated rejection on user supply path
// ---------------------------------------------------------------------------
//
// `remove_e_mode_category` flips `is_deprecated = true` and walks asset
// reverse-indexes. A user attempting to supply with the deprecated category
// triggers `ensure_e_mode_not_deprecated` via `active_e_mode_category` which
// is called both from `create_account` and from `process_deposit`.

#[test]
#[should_panic(expected = "Error(Contract, #301)")]
fn test_emode_user_supply_rejects_deprecated_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(
            1,
            EModeCategoryPreset {
                ltv: 9_700,
                threshold: 9_800,
                bonus: 200,
            },
        )
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // Deprecate the category via admin.
    t.remove_e_mode_category(1);

    // User attempts a fresh supply with the deprecated e-mode category. The
    // controller resolves `active_e_mode_category(env, 1)` and panics with
    // EModeCategoryDeprecated (#301).
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_market("USDC");
    let usdc_addr = usdc.asset.clone();
    // 1_000 USDC at 7 decimals.
    usdc.token_admin.mint(&alice, &10_000_000_000_i128);
    let assets: soroban_sdk::Vec<(Address, i128)> = vec![&t.env, (usdc_addr, 10_000_000_000_i128)];
    t.ctrl_client().supply(&alice, &0u64, &1u32, &assets);
}
