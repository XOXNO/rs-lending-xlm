use controller::constants::{MAX_FLASHLOAN_FEE_BPS, WAD};
use soroban_sdk::{vec, Address};
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, EModeCategoryPreset, LendingTest, ALICE,
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
// edit_asset_config is a thin setter: it persists the config as given
// (input validation lives in governance).

#[test]
fn test_edit_asset_config_persists_flashloan_fee_at_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();
    let mut cfg = ctrl.get_market_config(&asset).asset_config;
    cfg.flashloan_fee_bps = MAX_FLASHLOAN_FEE_BPS as u32;
    ctrl.edit_asset_config(&asset, &cfg);
    let updated = ctrl.get_market_config(&asset).asset_config;
    assert_eq!(updated.flashloan_fee_bps, MAX_FLASHLOAN_FEE_BPS as u32);
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
