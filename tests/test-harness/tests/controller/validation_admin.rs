use controller::constants::MAX_FLASHLOAN_FEE_BPS;
use soroban_sdk::vec;
use test_harness::{hub_asset, HubAssetKey, usdc_preset, usdt_stable_preset, SpokePreset, LendingTest, ALICE};
// edit_asset_config is a thin setter: it persists the config as given
// (input validation lives in governance).

#[test]
fn test_edit_asset_config_persists_flashloan_fee_at_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    // Flash-loan fee lives on the pool `MarketParamsRaw`; the harness
    // `edit_asset_config` helper writes it through to pool storage.
    t.edit_asset_config("USDC", |c| {
        c.flashloan_fee = MAX_FLASHLOAN_FEE_BPS as u32;
    });
    let updated = t.get_asset_config("USDC");
    assert_eq!(updated.flashloan_fee, MAX_FLASHLOAN_FEE_BPS as u32);
}
// spoke.rs:95 -- SpokeDeprecated rejection on user supply path
//
// `remove_spoke_category` flips `is_deprecated = true` and walks asset
// reverse-indexes. A user attempting to supply with the deprecated category
// triggers `ensure_spoke_not_deprecated` via `active_spoke_category` which
// is called both from `create_account` and from `process_deposit`.

#[test]
#[should_panic(expected = "Error(Contract, #301)")]
fn test_spoke_user_supply_rejects_deprecated_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(
            2,
            SpokePreset {
                ltv: 9_700,
                threshold: 9_800,
                bonus: 200,
            },
        )
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    // Deprecate the category via admin.
    t.remove_spoke_category(2);

    // User attempts a fresh supply with the deprecated spoke category. The
    // controller resolves `active_spoke_category(env, 2)` and panics with
    // SpokeDeprecated (#301).
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_market("USDC");
    let usdc_addr = usdc.asset.clone();
    // 1_000 USDC at 7 decimals.
    usdc.token_admin.mint(&alice, &10_000_000_000_i128);
    let assets: soroban_sdk::Vec<(HubAssetKey, i128)> = vec![&t.env, (hub_asset(usdc_addr), 10_000_000_000_i128)];
    t.ctrl_client().supply(&alice, &0u64, &2u32, &assets);
}
