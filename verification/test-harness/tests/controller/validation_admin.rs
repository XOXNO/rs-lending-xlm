use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, MAX_REASONABLE_PRICE_WAD, RAY, WAD};
use common::types::{
    InterestRateModel, MarketOracleConfigInput, OracleReadMode, OracleSourceConfigInput,
    OracleSourceConfigInputOption, OracleStrategy,
};
use soroban_sdk::{vec, Address, String, Symbol};
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, EModeCategoryPreset, LendingTest, ALICE,
    DEFAULT_TOLERANCE,
};
// validate_bulk_isolation -- BulkSupplyNoIso (validation.rs:109)
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
// validate_interest_rate_model invariants
//
// Driven via `upgrade_liquidity_pool_params`, which calls `validate_interest_rate_model`
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
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    }
}

// base_borrow_rate_ray < 0 -> BaseRateNegative (#128).
#[test]
#[should_panic(expected = "Error(Contract, #128)")]
fn test_validate_irm_rejects_negative_base_rate() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.base_borrow_rate_ray = -1;
    t.ctrl_client().upgrade_liquidity_pool_params(&asset, &irm);
}

// validation.rs:185 -- mid_utilization_ray <= 0 rejects InvalidUtilRange (#117).
#[test]
#[should_panic(expected = "Error(Contract, #117)")]
fn test_validate_irm_rejects_zero_mid_utilization() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.mid_utilization_ray = 0;
    t.ctrl_client().upgrade_liquidity_pool_params(&asset, &irm);
}

// validation.rs:188 -- optimal_utilization_ray <= mid_utilization_ray rejects #117.
#[test]
#[should_panic(expected = "Error(Contract, #117)")]
fn test_validate_irm_rejects_optimal_not_above_mid() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.optimal_utilization_ray = irm.mid_utilization_ray;
    t.ctrl_client().upgrade_liquidity_pool_params(&asset, &irm);
}

// validation.rs:191 -- optimal_utilization_ray >= RAY rejects OptUtilTooHigh (#118).
#[test]
#[should_panic(expected = "Error(Contract, #118)")]
fn test_validate_irm_rejects_optimal_at_or_above_ray() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut irm = baseline_irm();
    irm.optimal_utilization_ray = RAY;
    t.ctrl_client().upgrade_liquidity_pool_params(&asset, &irm);
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
    t.ctrl_client().upgrade_liquidity_pool_params(&asset, &irm);
}
// validate_asset_config invariants
//
// Driven via `edit_asset_config`. The validator is also called from
// `create_liquidity_pool`, but the edit path is shorter to set up.

// `loan_to_value_bps` is `u32`, so negative values are unrepresentable
// and the corresponding runtime branch is unreachable.

// validation.rs -- threshold*(1+bonus) > 100% rejects #113: a bonus large enough
// that liquidation seizure would exceed collateral is invalid (mints bad debt).
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_validate_asset_config_rejects_excessive_liq_bonus() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();
    let mut cfg = ctrl.get_market_config(&asset).asset_config;
    // 95% threshold * (1 + 10% bonus) = 104.5% > 100%.
    cfg.loan_to_value_bps = 8000;
    cfg.liquidation_threshold_bps = 9500;
    cfg.liquidation_bonus_bps = 1000;
    ctrl.edit_asset_config(&asset, &cfg);
}

// validation.rs -- a large bonus is permitted when the threshold leaves room:
// 50% threshold * (1 + 50% bonus) = 75% <= 100%. The bonus ceiling is the
// invariant, not a flat cap.
#[test]
fn test_validate_asset_config_accepts_high_bonus_low_threshold() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();
    let mut cfg = ctrl.get_market_config(&asset).asset_config;
    cfg.loan_to_value_bps = 4000;
    cfg.liquidation_threshold_bps = 5000;
    cfg.liquidation_bonus_bps = 5000;
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
// configure_market_oracle error paths (config.rs:453, 479, 524)

fn base_oracle_config(t: &LendingTest) -> MarketOracleConfigInput {
    let asset = t.resolve_market("USDC").asset.clone();
    test_harness::reflector_primary_anchor_config(
        &t.mock_reflector,
        &asset,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    )
}

fn set_primary_reflector_read_mode(cfg: &mut MarketOracleConfigInput, read_mode: OracleReadMode) {
    if let OracleSourceConfigInput::Reflector(ref mut source) = cfg.primary {
        source.read_mode = read_mode;
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
    set_primary_reflector_read_mode(&mut cfg, OracleReadMode::Twap(13));
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// PrimaryWithAnchor without an anchor rejects InvalidExchangeSrc (#11).
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_configure_market_oracle_rejects_dual_without_dex() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.strategy = OracleStrategy::PrimaryWithAnchor;
    cfg.anchor = OracleSourceConfigInputOption::None;
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// Identical primary and anchor collapse the dual-source diversity guarantee
// (anchor compared against itself always passes the tolerance band) and are
// rejected with InvalidExchangeSrc (#11).
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_configure_market_oracle_rejects_identical_primary_anchor() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.strategy = OracleStrategy::PrimaryWithAnchor;
    cfg.anchor = OracleSourceConfigInputOption::Some(cfg.primary.clone());
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// Two RedStone sources on the same contract and feed differ only in the
// policy-only `max_stale_seconds`, so they read the same underlying feed and
// collapse the dual-source diversity guarantee. Rejected with InvalidExchangeSrc
// (#11) at shape validation even though the configs are not byte-equal.
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_configure_market_oracle_rejects_same_redstone_feed_distinct_max_stale() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    // The diversity check runs before any live feed read, so a placeholder
    // contract address suffices; both sources share it (and the feed id) on
    // purpose so they resolve to the same underlying feed.
    let redstone = t.mock_reflector.clone();
    let feed_id = String::from_str(&t.env, "BTC");

    let mut cfg = base_oracle_config(&t);
    cfg.strategy = OracleStrategy::PrimaryWithAnchor;
    cfg.primary = test_harness::redstone_source_with_max_stale(&redstone, &feed_id, 600);
    cfg.anchor = OracleSourceConfigInputOption::Some(
        test_harness::redstone_source_with_max_stale(&redstone, &feed_id, 900),
    );
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_configure_market_oracle_rejects_non_usd_base() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client()
        .set_base_other(&Symbol::new(&t.env, "EUR"));
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_configure_market_oracle_rejects_bad_reflector_decimals() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_decimals(&19);
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #222)")]
fn test_configure_market_oracle_rejects_bad_reflector_resolution() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_resolution(&0);
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #212)")]
fn test_configure_market_oracle_rejects_missing_twap_history() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = base_oracle_config(&t);
    t.mock_reflector_client().set_twap_history_mode(&asset, &1);
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

// validate_sanity_bounds at configure time (config.rs:103-111) — #224.
#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_configure_market_oracle_rejects_zero_min_sanity() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.min_sanity_price_wad = 0;
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_configure_market_oracle_rejects_min_ge_max_sanity() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.min_sanity_price_wad = 100;
    cfg.max_sanity_price_wad = 100;
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_configure_market_oracle_rejects_max_sanity_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let mut cfg = base_oracle_config(&t);
    cfg.max_sanity_price_wad = MAX_REASONABLE_PRICE_WAD + 1;
    t.ctrl_client()
        .configure_market_oracle(&admin, &asset, &cfg);
}
// emode.rs:95 -- EModeCategoryDeprecated rejection on user supply path
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
