//! W7 adversarial tests: spoke caps, spoke usage, halt flags and position limits.
//!
//! H1 caps are checked at exact unit boundaries on every debt and deposit path (INV-HALT-03).
//! H2 saturation of the asset-to-scaled cap conversion at a bad-debt-depressed supply index
//! (INV-HALT-03, formulas.md "Caps, fees, and numeric limits").
//! H3 the halt-flag matrix and the ratchet/epoch rules (INV-HALT-02, INV-AUTH-04, ADR-0007/0008).
//! H4 position limits, delegates and whole-unit isolation (INV-RISK-04).
//! H5 the spoke-usage identity after every share writer (ADR-0015, INV-ACCT-10).
//! H6 deprecated spokes and the immutable spoke binding (INV-AUTH-06, ADR-0009).

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::calculate_scaled_cap;
use common::types::{
    HubAssetKey, MarketIndexRaw, PoolStateRaw, SeizeMode, SpokeAssetConfig, SpokeUsageRaw,
};
use common::validation::max_cap_for_decimals;
use controller::types::{PositionMode, SpokeAssetArgs};
use position_nft::PositionNftClient;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{vec, Address, Bytes, Env, Vec};
use test_harness::mock_blend::{KIND_COLLATERAL, KIND_LIABILITY};
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, f64_to_i128, hub_asset, map_try_ok_unit,
    map_try_ok_value, usd, usd_cents, AssetConfigPreset, FlashPositionMode, FlashPositionRequest,
    LendingTest, MarketPreset, ALICE, BOB, CAROL, DAVE, DEFAULT_ASSET_CONFIG,
    DEFAULT_MARKET_PARAMS, HARNESS_HUB, HARNESS_SPOKE, LIQUIDATOR, STABLECOIN_SPOKE,
};

type Outcome = Result<(), soroban_sdk::Error>;

/// One whole token at 7 decimals, in raw units.
const ETH: i128 = 10_000_000;
const USDC: i128 = 10_000_000;
/// Scaled units of one raw 7-decimal unit at index RAY: RAY / 10^7.
const SCALE_7: i128 = 10_i128.pow(20);
/// Second spoke of the stablecoin fixture (lists USDC and USDT, or USDC and ETH).
const SPOKE_B: u32 = 2;
const FLOOR_INDEX: i128 = SUPPLY_INDEX_FLOOR_RAW;
const EVE: &str = "eve";
const FRANK: &str = "frank";
const GINA: &str = "gina";
const HANK: &str = "hank";
const IVAN: &str = "ivan";
const JACK: &str = "jack";

// --- helpers ---------------------------------------------------------------

fn key(t: &LendingTest, asset: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(asset))
}

fn listing(t: &LendingTest, spoke: u32, asset: &str) -> SpokeAssetConfig {
    t.ctrl_client().get_spoke_asset(&spoke, &key(t, asset))
}

fn usage(t: &LendingTest, spoke: u32, asset: &str) -> SpokeUsageRaw {
    t.ctrl_client().get_spoke_usage(&spoke, &key(t, asset))
}

fn epoch(t: &LendingTest, spoke: u32, asset: &str) -> u64 {
    t.ctrl_client()
        .get_spoke_asset_flags_epoch(&spoke, &key(t, asset))
}

fn market_index(t: &LendingTest, asset: &str) -> MarketIndexRaw {
    t.ctrl_client().get_market_index(&key(t, asset))
}

fn pool_state(t: &LendingTest, asset: &str) -> PoolStateRaw {
    t.pool_client(asset).get_sync_data(&key(t, asset)).state
}

fn spoke_of(t: &LendingTest, account_id: u64) -> u32 {
    if account_id == 0 {
        HARNESS_SPOKE
    } else {
        t.ctrl_client().get_account_attributes(&account_id).spoke_id
    }
}

fn scaled_of(t: &LendingTest, account_id: u64, asset: &str) -> i128 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);
    supplies
        .get(key(t, asset))
        .map_or(0, |position| position.scaled_amount)
}

/// Every field of the listing, with the caller's changes applied afterwards. Reading the current
/// config first keeps the flags, so an edit only changes what the test names.
fn listing_args(t: &LendingTest, spoke: u32, asset: &str) -> SpokeAssetArgs {
    let cfg = listing(t, spoke, asset);
    SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset(asset),
        spoke_id: spoke,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: cfg.paused,
        frozen: cfg.frozen,
        no_seize: cfg.no_seize,
        ltv: cfg.loan_to_value,
        threshold: cfg.liquidation_threshold,
        bonus: cfg.liquidation_bonus,
        liquidation_fees: cfg.liquidation_fees,
        supply_cap: cfg.supply_cap,
        borrow_cap: cfg.borrow_cap,
    }
}

fn set_supply_cap(t: &LendingTest, asset: &str, cap: i128) {
    let mut args = listing_args(t, HARNESS_SPOKE, asset);
    args.supply_cap = cap;
    t.ctrl_client().edit_asset_in_spoke(&args);
}

fn set_borrow_cap(t: &LendingTest, asset: &str, cap: i128) {
    let mut args = listing_args(t, HARNESS_SPOKE, asset);
    args.borrow_cap = cap;
    t.ctrl_client().edit_asset_in_spoke(&args);
}

/// Sum of every live account's scaled supply in `asset`, read through the position NFT index.
fn scaled_supply_sum(t: &LendingTest, asset: &str) -> i128 {
    let hub = key(t, asset);
    let nft = PositionNftClient::new(&t.env, &t.position_nft);
    let ctrl = t.ctrl_client();
    let mut total = 0;
    for index in 0..nft.total_supply() {
        let account_id = u64::from(nft.get_token_id(&index));
        let (supplies, _) = ctrl.get_account_positions(&account_id);
        if let Some(position) = supplies.get(hub.clone()) {
            total += position.scaled_amount;
        }
    }
    total
}

/// The usage identity: every spoke usage row equals the positions it caps (harness check), and
/// the pool's `supplied - revenue` equals the sum of account scaled supply.
fn assert_identity(t: &LendingTest, assets: &[&str], step: &str) {
    t.assert_spoke_usage_matches_positions();
    for asset in assets {
        let state = pool_state(t, asset);
        assert_eq!(
            state.supplied - state.revenue,
            scaled_supply_sum(t, asset),
            "{step}: pool supplied - revenue != sum of account scaled supply for {asset}"
        );
    }
}

/// Supplies `raw` units through the controller so a rejection is returned, not panicked. The
/// account is the user's default one, or a new one in the harness spoke when the user has none.
fn supply_raw_try(
    t: &mut LendingTest,
    user: &str,
    asset: &str,
    raw: i128,
) -> Result<u64, soroban_sdk::Error> {
    let addr = t.get_or_create_user(user);
    let asset_addr = t.resolve_asset(asset);
    t.resolve_market(asset).token_admin.mint(&addr, &raw);
    let account_id = t.default_account_id_or_zero(user);
    let spoke = spoke_of(t, account_id);
    let assets = vec![&t.env, (hub_asset(asset_addr), raw)];
    map_try_ok_value(
        t.ctrl_client()
            .try_supply(&addr, &account_id, &spoke, &assets),
    )
}

fn borrow_raw_try(t: &LendingTest, user: &str, asset: &str, raw: i128) -> Outcome {
    let account_id = t.resolve_account_id(user);
    let addr = t.users.get(user).expect("user exists").address.clone();
    let borrows = vec![&t.env, (key(t, asset), raw)];
    map_try_ok_unit(
        t.ctrl_client()
            .try_borrow(&addr, &account_id, &borrows, &None),
    )
}

fn swap_debt_try(
    t: &LendingTest,
    user: &str,
    existing: &str,
    new: &str,
    new_raw: i128,
    steps: &Bytes,
) -> Outcome {
    let account_id = t.resolve_account_id(user);
    let addr = t.users.get(user).expect("user exists").address.clone();
    let existing_key = key(t, existing);
    let new_key = key(t, new);
    map_try_ok_unit(t.ctrl_client().try_swap_debt(
        &addr,
        &account_id,
        &existing_key,
        &new_raw,
        &new_key,
        steps,
    ))
}

/// Opens a multiply position on a new account in `spoke`: `debt_raw` of `debt` is flash-borrowed
/// and swapped into `collateral` through `steps`.
fn multiply_try(
    t: &mut LendingTest,
    user: &str,
    spoke: u32,
    collateral: &str,
    debt: &str,
    debt_raw: i128,
    steps: &Bytes,
) -> Result<u64, soroban_sdk::Error> {
    let caller = t.get_or_create_user(user);
    let coll = key(t, collateral);
    let debt_key = key(t, debt);
    map_try_ok_value(t.ctrl_client().try_multiply(
        &caller,
        &0u64,
        &spoke,
        &coll,
        &debt_raw,
        &debt_key,
        &PositionMode::Multiply,
        steps,
        &None,
        &None,
    ))
}

fn flash_payload(t: &LendingTest, collateral: &str, raw: i128) -> Bytes {
    FlashPositionRequest {
        mode: FlashPositionMode::Success,
        collateral: t.resolve_asset(collateral),
        collateral_amount: raw,
        extra_asset: t.resolve_asset(collateral),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env)
}

/// `flash_position` on a new account in `spoke`: the receiver mints and returns `collateral_raw`
/// of `collateral`, which must reach the deposit leg.
fn flash_try(
    t: &mut LendingTest,
    user: &str,
    spoke: u32,
    debt: &str,
    debt_raw: i128,
    collateral: &str,
    collateral_raw: i128,
) -> Result<u64, soroban_sdk::Error> {
    flash_on_account_try(
        t,
        user,
        0,
        spoke,
        debt,
        debt_raw,
        collateral,
        collateral_raw,
    )
}

/// `flash_position` on `account_id` (0 creates one) in `spoke`.
#[allow(clippy::too_many_arguments)]
fn flash_on_account_try(
    t: &mut LendingTest,
    user: &str,
    account_id: u64,
    spoke: u32,
    debt: &str,
    debt_raw: i128,
    collateral: &str,
    collateral_raw: i128,
) -> Result<u64, soroban_sdk::Error> {
    let caller = t.get_or_create_user(user);
    let receiver = t.deploy_flash_position_receiver();
    let data = flash_payload(t, collateral, collateral_raw);
    let mins = vec![&t.env, (key(t, collateral), collateral_raw)];
    let refunds: Vec<Address> = Vec::new(&t.env);
    let debt_key = key(t, debt);
    map_try_ok_value(t.ctrl_client().try_flash_position(
        &caller,
        &account_id,
        &spoke,
        &PositionMode::Multiply,
        &debt_key,
        &debt_raw,
        &receiver,
        &data,
        &mins,
        &refunds,
    ))
}

/// `migrate_from_blend` through the controller with an explicit spoke.
fn migrate_try(
    t: &mut LendingTest,
    user: &str,
    account_id: u64,
    spoke: u32,
    collateral: &[&str],
    supply: &[&str],
    debt: &[(&str, i128)],
) -> Result<u64, soroban_sdk::Error> {
    let blend = t.ensure_approved_blend();
    let caller = t.get_or_create_user(user);
    let mut coll: Vec<Address> = Vec::new(&t.env);
    for name in collateral {
        coll.push_back(t.resolve_asset(name));
    }
    let mut supp: Vec<Address> = Vec::new(&t.env);
    for name in supply {
        supp.push_back(t.resolve_asset(name));
    }
    let mut debts: Vec<(Address, i128)> = Vec::new(&t.env);
    for (name, raw) in debt {
        debts.push_back((t.resolve_asset(name), *raw));
    }
    map_try_ok_value(t.ctrl_client().try_migrate_from_blend(
        &caller,
        &account_id,
        &spoke,
        &HARNESS_HUB,
        &blend,
        &coll,
        &supp,
        &debts,
    ))
}

// --- H1: caps on every debt and deposit path -------------------------------

/// INV-HALT-03: a plain borrow admits exactly `cap` in total, rejects one more raw unit, and a
/// repayment returns headroom by exactly its scaled amount.
#[test]
fn rv_borrow_eth_cap_boundary_admits_exact_cap_then_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    set_borrow_cap(&t, "ETH", 2 * ETH);

    borrow_raw_try(&t, ALICE, "ETH", 2 * ETH - 1).expect("cap - 1 raw is admitted");
    borrow_raw_try(&t, ALICE, "ETH", 1).expect("the exact cap is admitted");
    assert_contract_error(
        borrow_raw_try(&t, ALICE, "ETH", 1),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        2 * ETH * SCALE_7,
        "usage sits exactly on the cap after the boundary borrow"
    );

    t.repay_raw(ALICE, "ETH", 1);
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        (2 * ETH - 1) * SCALE_7,
        "a one-unit repayment frees exactly one unit of usage"
    );
    borrow_raw_try(&t, ALICE, "ETH", 1).expect("the freed unit is admitted again");
    assert_identity(&t, &["USDC", "ETH"], "borrow boundary");
}

/// Shared fixture for the debt-leg tests: Alice already owes 1 ETH and the ETH borrow cap leaves
/// exactly one ETH of room. The router can pay out USDC for the swaps.
fn one_eth_room() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply("kim", "ETH", 20.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    set_borrow_cap(&t, "ETH", 2 * ETH);
    t.fund_router("USDC", 100_000.0);
    t
}

/// INV-HALT-03: `multiply` and `flash_position` each borrow the strategy debt through the same
/// entry gate, so a debt leg of exactly the room is admitted and one raw unit more is rejected.
#[test]
fn rv_multiply_and_flash_position_eth_debt_leg_stop_at_borrow_cap() {
    let mut t = one_eth_room();
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(3_000.0, 7));
    multiply_try(&mut t, BOB, HARNESS_SPOKE, "USDC", "ETH", ETH, &steps)
        .expect("multiply borrowing exactly the room is admitted");
    // A 1-raw strategy debt would round its 9 bps fee to the whole amount and fail earlier with
    // AmountMustBePositive, so the over-room probe borrows a real 0.001 ETH.
    assert_contract_error(
        multiply_try(
            &mut t,
            CAROL,
            HARNESS_SPOKE,
            "USDC",
            "ETH",
            ETH / 1_000,
            &steps,
        ),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        2 * ETH * SCALE_7
    );

    let mut t = one_eth_room();
    assert_contract_error(
        flash_try(
            &mut t,
            BOB,
            HARNESS_SPOKE,
            "ETH",
            ETH + 1,
            "USDC",
            3_000 * USDC,
        ),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    flash_try(&mut t, BOB, HARNESS_SPOKE, "ETH", ETH, "USDC", 3_000 * USDC)
        .expect("flash_position borrowing exactly the room is admitted");
    assert_contract_error(
        flash_try(&mut t, CAROL, HARNESS_SPOKE, "ETH", 1, "USDC", 3_000 * USDC),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        2 * ETH * SCALE_7
    );
}

/// INV-HALT-03: `swap_debt` caps its new debt leg, and `migrate_from_blend` caps each debt cap it
/// borrows. Both are checked at the exact boundary.
#[test]
fn rv_swap_debt_and_migrate_debt_legs_stop_at_eth_borrow_cap() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 2_000.0);
    set_borrow_cap(&t, "ETH", 2 * ETH);
    t.fund_router("USDC", 10_000.0);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(2_000.0, 7));
    assert_contract_error(
        swap_debt_try(&t, ALICE, "USDC", "ETH", 2 * ETH + 1, &steps),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    swap_debt_try(&t, ALICE, "USDC", "ETH", 2 * ETH, &steps)
        .expect("swap_debt borrowing exactly the cap is admitted");
    assert_eq!(
        t.borrow_balance_raw("alice", "USDC"),
        0,
        "the swapped proceeds repaid the USDC leg"
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        2 * ETH * SCALE_7
    );

    let mut t = LendingTest::new().standard_two_asset().build();
    set_borrow_cap(&t, "ETH", 2 * ETH);
    t.ensure_approved_blend();
    t.seed_blend("ivan", "ETH", KIND_LIABILITY, 1.0);
    t.seed_blend("ivan", "USDC", KIND_COLLATERAL, 10_000.0);
    assert_contract_error(
        migrate_try(
            &mut t,
            "ivan",
            0,
            HARNESS_SPOKE,
            &["USDC"],
            &[],
            &[("ETH", 25_000_000)],
        ),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    t.try_migrate_from_blend("ivan", 0, &["USDC"], &[], &[("ETH", 2.0)])
        .expect("a debt cap of exactly the room migrates");
    assert_eq!(
        t.borrow_balance_raw("ivan", "ETH"),
        ETH,
        "the 1 ETH Blend liability was repaid and the excess 1 ETH returned to the hub debt"
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        ETH * SCALE_7
    );
}

/// INV-HALT-03: a USDC supply cap binds `supply` and the deposit leg of `multiply` at the exact
/// boundary; one raw unit over is rejected and the rejected call leaves no usage behind.
#[test]
fn rv_supply_and_multiply_deposit_respect_usdc_supply_cap() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.fund_router("USDC", 10_000.0);
    set_supply_cap(&t, "USDC", 4_000 * USDC);
    t.supply(BOB, "USDC", 1_000.0);

    let over = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(3_001.0, 7));
    assert_contract_error(
        multiply_try(&mut t, CAROL, HARNESS_SPOKE, "USDC", "ETH", ETH, &over),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    let exact = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(3_000.0, 7));
    multiply_try(&mut t, CAROL, HARNESS_SPOKE, "USDC", "ETH", ETH, &exact)
        .expect("multiply depositing exactly the room is admitted");

    assert_contract_error(
        supply_raw_try(&mut t, BOB, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray,
        4_000 * USDC * SCALE_7,
        "the cap is reached exactly and no rejected deposit was recorded"
    );
    assert_identity(&t, &["USDC", "ETH"], "supply cap");
}

/// INV-HALT-03: the deposit leg of `swap_collateral` is cap-checked after the withdrawal, and the
/// whole call reverts when the swapped output exceeds the room.
#[test]
fn rv_swap_collateral_deposit_respects_usdc_supply_cap() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.fund_router("USDC", 10_000.0);
    t.supply(DAVE, "ETH", 1.0);
    set_supply_cap(&t, "USDC", 2_000 * USDC);

    let over = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(2_001.0, 7));
    assert_contract_error(
        t.try_swap_collateral(DAVE, "ETH", 1.0, "USDC", &over),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").supplied_scaled_ray,
        ETH * SCALE_7,
        "the rejected swap left the ETH collateral in place"
    );

    let exact = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(2_000.0, 7));
    t.try_swap_collateral(DAVE, "ETH", 1.0, "USDC", &exact)
        .expect("a deposit of exactly the room is admitted");
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray,
        2_000 * USDC * SCALE_7
    );
    assert_eq!(usage(&t, HARNESS_SPOKE, "ETH").supplied_scaled_ray, 0);
}

/// INV-HALT-03: the deposit legs of `flash_position` and `migrate_from_blend` are cap-checked.
#[test]
fn rv_flash_position_and_migrate_deposit_respect_usdc_supply_cap() {
    let mut t = LendingTest::new().standard_two_asset().build();
    set_supply_cap(&t, "USDC", 3_000 * USDC - 1);
    assert_contract_error(
        flash_try(
            &mut t,
            "eve",
            HARNESS_SPOKE,
            "ETH",
            ETH,
            "USDC",
            3_000 * USDC,
        ),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    set_supply_cap(&t, "USDC", 3_000 * USDC);
    flash_try(
        &mut t,
        "eve",
        HARNESS_SPOKE,
        "ETH",
        ETH,
        "USDC",
        3_000 * USDC,
    )
    .expect("a receiver deposit of exactly the room is admitted");
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray,
        3_000 * USDC * SCALE_7
    );

    let mut t = LendingTest::new().standard_two_asset().build();
    t.ensure_approved_blend();
    t.seed_blend("frank", "USDC", KIND_COLLATERAL, 5_000.0);
    set_supply_cap(&t, "USDC", 4_000 * USDC);
    assert_contract_error(
        migrate_try(&mut t, "frank", 0, HARNESS_SPOKE, &["USDC"], &[], &[]),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    set_supply_cap(&t, "USDC", 5_000 * USDC);
    t.try_migrate_from_blend("frank", 0, &["USDC"], &[], &[])
        .expect("migrating deposits that fit under the cap");
    assert!(
        t.supply_balance("frank", "USDC") > 4_999.0,
        "the Blend collateral was deposited into the hub"
    );
}

/// ADR-0015: a same-spoke Credit liquidation adds no supply usage and is not cap-checked. Its
/// only effect on usage is the protocol fee leaving it, and a fresh supply stays capped.
#[test]
fn rv_liquidation_credit_ignores_supply_cap_and_moves_usage_by_fee() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    let alice = t.account_id(ALICE);

    let usage_before = usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray;
    let revenue_before = pool_state(&t, "USDC").revenue;
    let alice_before = scaled_of(&t, alice, "USDC");
    set_supply_cap(&t, "USDC", 1);

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    let fee = pool_state(&t, "USDC").revenue - revenue_before;
    let seized = alice_before - scaled_of(&t, alice, "USDC");
    let credited = scaled_of(&t, receiver, "USDC");

    assert!(fee > 0, "the fixture carries a liquidation fee");
    assert_eq!(credited + fee, seized, "seized shares = credited + fee");
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray,
        usage_before - fee,
        "spoke usage falls by exactly the fee shares and by nothing else"
    );
    assert_contract_error(
        supply_raw_try(&mut t, BOB, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_identity(&t, &["USDC", "ETH"], "credit under a lowered cap");
}

/// INV-HALT-03: exits consume no cap. After a cap is lowered below current usage, a top-up of an
/// existing position and a new borrow are rejected, while withdrawals and repayments succeed.
#[test]
fn rv_exits_ignore_lowered_caps_topup_and_new_borrow_reject() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    set_supply_cap(&t, "USDC", 5_000 * USDC);
    set_borrow_cap(&t, "ETH", ETH / 2);

    assert_contract_error(
        supply_raw_try(&mut t, ALICE, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_contract_error(
        borrow_raw_try(&t, ALICE, "ETH", 1),
        errors::SPOKE_BORROW_CAP_REACHED,
    );

    t.withdraw(ALICE, "USDC", 2_000.0);
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray,
        8_000 * USDC * SCALE_7,
        "withdrawal succeeds while usage is still above the lowered cap"
    );
    t.repay(ALICE, "ETH", 1.0);
    assert_eq!(usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray, 0);
    t.withdraw_all(ALICE, "USDC");
    assert_eq!(usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray, 0);
    assert_identity(&t, &["USDC", "ETH"], "exits under lowered caps");
}

// --- H2: fail-open saturation at a depressed supply index ------------------

/// Bob supplies 10 ETH. Alice borrows `borrow_eth` ETH against 30,000 USDC. USDC then falls to
/// $0.0001 and a Transfer liquidation leaves the ETH debt with no collateral, so the pool socializes
/// it into ETH's supply index.
fn writedown_fixture(borrow_eth: f64) -> LendingTest {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_max_utilization_disabled_all_markets()
        .build();
    t.supply(BOB, "ETH", 10.0);
    t.supply(ALICE, "USDC", 30_000.0);
    t.borrow(ALICE, "ETH", borrow_eth);
    t.set_price("USDC", usd(1) / 10_000);
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    t
}

/// INV-IDX-03 and INV-HALT-03: socializing debt larger than the supply lowers ETH's supply index to
/// the floor RAY/1000. The borrow index is untouched by a write-down.
#[test]
fn rv_bad_debt_writedown_floors_eth_supply_index_borrow_index_stays_ray() {
    let t = writedown_fixture(10.5);
    let idx = market_index(&t, "ETH");
    std::println!(
        "H2 deep write-down: ETH supply_index = {} (floor {}), borrow_index = {}",
        idx.supply_index,
        FLOOR_INDEX,
        idx.borrow_index
    );
    assert_eq!(
        idx.supply_index, FLOOR_INDEX,
        "10.5 ETH of socialized debt exceeds the 10 ETH supplied, so the write-down clamps"
    );
    assert_eq!(idx.borrow_index, RAY, "the borrow index never moves down");
    assert!(
        t.supply_balance("bob", "ETH") < 0.02,
        "Bob's 10 ETH claim is now worth about 0.01 ETH"
    );
    assert_identity(&t, &["ETH", "USDC"], "after write-down");
}

/// INV-HALT-03 (fail open): at the supply-index floor a cap of 2e15 raw saturates. Deposits above
/// the configured cap are refused, and so are deposits between the saturation threshold and the cap.
/// The admitted ceiling is the scaled-share overflow point, not the configured cap.
#[test]
fn rv_saturated_eth_supply_cap_refuses_every_deposit_above_overflow_ceiling() {
    let mut t = writedown_fixture(10.5);
    let cap: i128 = 2_000_000_000_000_000;
    let c_sat = i128::MAX / 10_i128.pow(23);
    let floor = Ray::from(FLOOR_INDEX);
    assert_eq!(
        calculate_scaled_cap(&t.env, cap, 7, floor).raw(),
        i128::MAX,
        "2e15 saturates at the floor"
    );
    assert!(
        calculate_scaled_cap(&t.env, c_sat, 7, floor).raw() < i128::MAX,
        "the last non-saturating cap is i128::MAX / 10^23"
    );
    assert_eq!(
        calculate_scaled_cap(&t.env, c_sat + 1, 7, floor).raw(),
        i128::MAX
    );
    set_supply_cap(&t, "ETH", cap);

    assert_contract_error(
        supply_raw_try(&mut t, DAVE, "ETH", cap + 1),
        GenericError::MathOverflow as u32,
    );
    assert_contract_error(
        supply_raw_try(&mut t, DAVE, "ETH", c_sat + 1),
        GenericError::MathOverflow as u32,
    );

    // Bob is the only other supplier. The largest admissible deposit leaves the total on i128::MAX.
    let bob_scaled = scaled_of(&t, t.account_id(BOB), "ETH");
    let max_deposit = (i128::MAX - bob_scaled) / 10_i128.pow(23);
    assert_contract_error(
        supply_raw_try(&mut t, DAVE, "ETH", max_deposit + 1),
        GenericError::MathOverflow as u32,
    );
    t.supply_raw(CAROL, "ETH", max_deposit);
    assert_eq!(
        usage(&t, HARNESS_SPOKE, "ETH").supplied_scaled_ray,
        bob_scaled + max_deposit * 10_i128.pow(23),
        "the spoke row holds Bob's claim plus the admitted deposit"
    );
    std::println!(
        "H2 deep: configured cap {cap}, saturation threshold {c_sat}, \
         largest admitted deposit {max_deposit} raw ({:.4}% of the cap)",
        max_deposit as f64 * 100.0 / cap as f64
    );
    assert_identity(&t, &["ETH"], "saturated cap");
}

/// INV-HALT-03: at a partial write-down (index about 0.15) a 2e17 cap does not saturate. It is
/// enforced against the running total: a deposit that crosses it is refused, and one that stays
/// under it is admitted.
#[test]
fn rv_eth_supply_cap_enforced_at_partial_writedown_index() {
    let mut t = writedown_fixture(8.5);
    let idx = market_index(&t, "ETH").supply_index;
    let cap: i128 = 200_000_000_000_000_000;
    assert!(
        calculate_scaled_cap(&t.env, cap, 7, Ray::from(idx)).raw() < i128::MAX,
        "a 2e17 cap is not saturated at supply index {idx}"
    );
    std::println!(
        "H2 partial: ETH supply_index = {idx} (about {:.4} RAY)",
        idx as f64 / RAY as f64
    );
    set_supply_cap(&t, "ETH", cap);

    supply_raw_try(&mut t, CAROL, "ETH", 100_000_000_000_000_000).expect("1e17 fits the cap");
    supply_raw_try(&mut t, CAROL, "ETH", 90_000_000_000_000_000).expect("1.9e17 fits the cap");
    assert_contract_error(
        supply_raw_try(&mut t, CAROL, "ETH", 20_000_000_000_000_000),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    supply_raw_try(&mut t, CAROL, "ETH", 9_000_000_000_000_000)
        .expect("the remaining room of about 1e16 admits 9e15");
    assert_contract_error(
        supply_raw_try(&mut t, CAROL, "ETH", 1_100_000_000_000_000),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert!(
        usage(&t, HARNESS_SPOKE, "ETH").supplied_scaled_ray
            <= calculate_scaled_cap(&t.env, cap, 7, Ray::from(idx)).raw()
    );
}

/// INV-HALT-03 (conversion): the cap conversion can saturate only at an index below one RAY. At
/// every index from RAY upward the largest admissible cap converts exactly, for every decimal
/// count the protocol admits.
#[test]
fn rv_cap_saturation_needs_index_below_ray_for_every_admitted_decimals() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    for decimals in [0u32, 2, 3, 7, 12, 18] {
        let ceiling = max_cap_for_decimals(decimals);
        let at_ray = calculate_scaled_cap(&env, ceiling, decimals, Ray::from(RAY)).raw();
        assert_eq!(
            at_ray,
            ceiling * 10_i128.pow(27 - decimals),
            "decimals {decimals}: the ceiling converts exactly at index RAY"
        );
        for index in [RAY, RAY + 1, 2 * RAY, 1_000_000 * RAY] {
            assert!(
                calculate_scaled_cap(&env, ceiling, decimals, Ray::from(index)).raw() < i128::MAX,
                "decimals {decimals}: no admitted cap saturates at index {index}"
            );
        }
        assert_eq!(
            calculate_scaled_cap(&env, ceiling, decimals, Ray::from(RAY / 2)).raw(),
            i128::MAX,
            "decimals {decimals}: the ceiling does saturate below RAY"
        );
    }
    let at_floor = max_cap_for_decimals(7) / 1_000;
    assert!(
        calculate_scaled_cap(&env, at_floor, 7, Ray::from(FLOOR_INDEX)).raw() < i128::MAX,
        "1/1000 of the 7-decimal ceiling is still admitted at the floor"
    );
    assert_eq!(
        calculate_scaled_cap(&env, at_floor + 1_000, 7, Ray::from(FLOOR_INDEX)).raw(),
        i128::MAX,
        "a cap a little above 1/1000 of the ceiling saturates at the floor"
    );
}

// --- H3: halt flags and the ratchet ------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Flag {
    Paused,
    Frozen,
    NoSeize,
}

fn set_flag(t: &LendingTest, asset: &str, flag: Flag) {
    let (paused, frozen, no_seize) = match flag {
        Flag::Paused => (true, false, false),
        Flag::Frozen => (false, true, false),
        Flag::NoSeize => (false, false, true),
    };
    t.set_spoke_asset_flags(asset, paused, frozen, no_seize);
}

/// Seven probes on one fixture, in the order `expected_codes` lists them. Entry and exit probes
/// run first on the healthy book; the USDC price then falls for the liquidation probe, so no probe
/// sees another's price move.
fn probe_cell(asset: &str, flag: Flag) -> [Outcome; 7] {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(BOB, "USDC", 10_000.0);
    t.borrow(BOB, "USDC", 1_000.0);
    t.fund_router("USDC", 10_000.0);
    set_flag(&t, asset, flag);

    let supply = t.try_supply(ALICE, "USDC", 100.0).map(|_| ());
    let borrow = t.try_borrow(ALICE, "ETH", 0.1);
    let withdraw = t.try_withdraw(ALICE, "USDC", 100.0);
    let repay = t.try_repay(ALICE, "ETH", 0.1);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(3_000.0, 7));
    let multiply =
        multiply_try(&mut t, CAROL, HARNESS_SPOKE, "USDC", "ETH", ETH, &steps).map(|_| ());
    let empty = Bytes::new(&t.env);
    let netting = t.try_repay_debt_with_collateral(BOB, "USDC", 500.0, "USDC", &empty, false);
    t.set_price("USDC", usd_cents(50));
    let liquidate = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    [
        supply, borrow, withdraw, repay, multiply, netting, liquidate,
    ]
}

const ROW_NAMES: [&str; 7] = [
    "supply USDC",
    "borrow ETH",
    "withdraw USDC",
    "repay ETH",
    "multiply USDC/ETH",
    "netting USDC/USDC",
    "liquidate ETH->USDC",
];

/// The expected code per probe, from INV-HALT-02 and ADR-0008. `None` means the probe succeeds.
fn expected_codes(asset: &str, flag: Flag) -> [Option<u32>; 7] {
    let paused = errors::SPOKE_ASSET_PAUSED;
    let frozen = errors::SPOKE_ASSET_FROZEN;
    let seize = errors::SPOKE_ASSET_SEIZURE_HALTED;
    match (asset, flag) {
        ("USDC", Flag::Paused) => [
            Some(paused),
            None,
            Some(paused),
            None,
            Some(paused),
            Some(paused),
            None,
        ],
        ("USDC", Flag::Frozen) => [Some(frozen), None, None, None, Some(frozen), None, None],
        ("USDC", Flag::NoSeize) => [None, None, None, None, None, None, Some(seize)],
        ("ETH", Flag::Paused) => [
            None,
            Some(paused),
            None,
            Some(paused),
            Some(paused),
            None,
            Some(paused),
        ],
        ("ETH", Flag::Frozen) => [None, Some(frozen), None, None, Some(frozen), None, None],
        ("ETH", Flag::NoSeize) => [None; 7],
        _ => unreachable!("matrix covers USDC and ETH only"),
    }
}

fn show(outcome: &Outcome) -> String {
    match outcome {
        Ok(()) => "ok".to_string(),
        Err(err) => format!("{err:?}"),
    }
}

/// INV-HALT-02 and ADR-0008: each halt flag gates only the legs it names. The full 6-cell by
/// 7-probe grid must match the documented matrix exactly.
#[test]
fn rv_halt_flag_matrix_matches_documented_legs() {
    let mut mismatches: std::vec::Vec<String> = std::vec::Vec::new();
    for (asset, flag) in [
        ("USDC", Flag::Paused),
        ("USDC", Flag::Frozen),
        ("USDC", Flag::NoSeize),
        ("ETH", Flag::Paused),
        ("ETH", Flag::Frozen),
        ("ETH", Flag::NoSeize),
    ] {
        let actual = probe_cell(asset, flag);
        let expected = expected_codes(asset, flag);
        std::println!(
            "H3 {asset} {flag:?}: {}",
            actual
                .iter()
                .zip(ROW_NAMES)
                .map(|(outcome, name)| format!("{name}={}", show(outcome)))
                .collect::<std::vec::Vec<_>>()
                .join(", ")
        );
        for (row, (outcome, want)) in actual.iter().zip(expected.iter()).enumerate() {
            let ok = match want {
                None => outcome.is_ok(),
                Some(code) => {
                    let err = soroban_sdk::Error::from_contract_error(*code);
                    outcome.as_ref().err() == Some(&err)
                }
            };
            if !ok {
                mismatches.push(format!(
                    "{asset} {flag:?} / {}: got {}, want {:?}",
                    ROW_NAMES[row],
                    show(outcome),
                    want
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "halt matrix differs from the documented legs:\n{}",
        mismatches.join("\n")
    );
}

/// INV-HALT-02: cleanup of an insolvent account with dust collateral ignores every halt flag on
/// both assets, and it removes the account and drains both usage rows.
#[test]
fn rv_clean_bad_debt_ignores_every_halt_flag() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);
    let alice = t.account_id(ALICE);
    for asset in ["USDC", "ETH"] {
        t.set_spoke_asset_flags(asset, true, true, true);
    }
    t.set_price("USDC", usd_cents(1));
    assert!(
        t.can_be_liquidated(ALICE),
        "the account is insolvent with dust collateral"
    );

    t.try_clean_bad_debt_by_id(alice)
        .expect("cleanup ignores paused, frozen and no_seize");
    assert!(!t.account_exists(alice), "the account and its NFT are gone");
    assert_eq!(usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray, 0);
    assert_eq!(usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray, 0);
}

/// INV-AUTH-04 and ADR-0007: `set_spoke_asset_flags` and `edit_asset_in_spoke` only tighten. A
/// set flag is cleared only through `relax_spoke_asset_flags`.
#[test]
fn rv_flag_ratchet_blocks_clearing_by_set_and_edit() {
    let t = LendingTest::new().standard_two_asset().build();
    let ctrl = t.ctrl_client();
    let usdc = key(&t, "USDC");
    ctrl.set_spoke_asset_flags(&HARNESS_SPOKE, &usdc, &true, &false, &false);

    assert_contract_error(
        map_try_ok_unit(ctrl.try_set_spoke_asset_flags(
            &HARNESS_SPOKE,
            &usdc,
            &false,
            &false,
            &false,
        )),
        errors::SPOKE_ASSET_FLAG_RELAXATION,
    );
    assert!(
        listing(&t, HARNESS_SPOKE, "USDC").paused,
        "set could not clear the pause"
    );

    let mut args = listing_args(&t, HARNESS_SPOKE, "USDC");
    args.paused = false;
    assert_contract_error(
        map_try_ok_unit(ctrl.try_edit_asset_in_spoke(&args)),
        errors::SPOKE_ASSET_FLAG_RELAXATION,
    );
    assert!(
        listing(&t, HARNESS_SPOKE, "USDC").paused,
        "edit could not clear the pause"
    );

    let mut tighter = listing_args(&t, HARNESS_SPOKE, "USDC");
    tighter.frozen = true;
    tighter.no_seize = true;
    ctrl.edit_asset_in_spoke(&tighter);
    let cfg = listing(&t, HARNESS_SPOKE, "USDC");
    assert!(
        cfg.paused && cfg.frozen && cfg.no_seize,
        "edit may add flags"
    );
}

/// ADR-0007: every flag write advances the listing's epoch, including a no-op guardian call. A
/// relaxation with a stale epoch is rejected and leaves the epoch alone. A relaxation with the
/// current epoch clears the flags and advances the epoch once more.
#[test]
fn rv_relax_requires_current_epoch_and_every_flag_write_advances_it() {
    let t = LendingTest::new().standard_two_asset().build();
    let ctrl = t.ctrl_client();
    let usdc = key(&t, "USDC");
    // Adding the listing at build time already advanced the epoch once.
    let base = epoch(&t, HARNESS_SPOKE, "USDC");
    assert_eq!(base, 1, "a listing is a flags-epoch write");

    ctrl.set_spoke_asset_flags(&HARNESS_SPOKE, &usdc, &true, &false, &false);
    assert_eq!(
        epoch(&t, HARNESS_SPOKE, "USDC"),
        base + 1,
        "set advances the epoch"
    );
    ctrl.set_spoke_asset_flags(&HARNESS_SPOKE, &usdc, &true, &false, &false);
    assert_eq!(
        epoch(&t, HARNESS_SPOKE, "USDC"),
        base + 2,
        "a repeated set with unchanged flags still advances the epoch"
    );

    let unchanged = listing_args(&t, HARNESS_SPOKE, "USDC");
    ctrl.edit_asset_in_spoke(&unchanged);
    assert_eq!(
        epoch(&t, HARNESS_SPOKE, "USDC"),
        base + 2,
        "an edit that changes no flag does not advance the epoch"
    );
    let mut tighter = listing_args(&t, HARNESS_SPOKE, "USDC");
    tighter.frozen = true;
    ctrl.edit_asset_in_spoke(&tighter);
    assert_eq!(
        epoch(&t, HARNESS_SPOKE, "USDC"),
        base + 3,
        "an edit that adds a flag advances it"
    );

    assert_contract_error(
        map_try_ok_unit(ctrl.try_relax_spoke_asset_flags(
            &HARNESS_SPOKE,
            &usdc,
            &(base + 1),
            &false,
            &false,
            &false,
        )),
        errors::SPOKE_FLAGS_EPOCH_MISMATCH,
    );
    let cfg = listing(&t, HARNESS_SPOKE, "USDC");
    assert!(
        cfg.paused && cfg.frozen,
        "a stale relaxation changed nothing"
    );
    assert_eq!(epoch(&t, HARNESS_SPOKE, "USDC"), base + 3);

    ctrl.relax_spoke_asset_flags(&HARNESS_SPOKE, &usdc, &(base + 3), &false, &false, &false);
    let cfg = listing(&t, HARNESS_SPOKE, "USDC");
    assert!(
        !cfg.paused && !cfg.frozen && !cfg.no_seize,
        "the current epoch clears the flags"
    );
    assert_eq!(epoch(&t, HARNESS_SPOKE, "USDC"), base + 4);
}

// --- H4: position limits, delegates and whole-unit isolation ---------------

/// INV-RISK-04: with limits (2, 1) a third supply slot and a second borrow slot are rejected, while
/// top-ups of held assets are not.
#[test]
fn rv_position_limits_reject_third_supply_and_second_borrow_slot() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_position_limits(2, 1)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);
    assert_contract_error(
        t.try_supply(ALICE, "WBTC", 0.01),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "USDC", 1_000.0);
    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 0.1),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    t.borrow(ALICE, "USDC", 100.0);
    t.assert_supply_count(ALICE, 2);
    t.assert_borrow_count(ALICE, 1);
}

/// INV-RISK-04: lowering the limits below an account's counts keeps repayments, withdrawals and
/// top-ups working. Only a new slot is refused.
#[test]
fn rv_position_limit_lowered_below_count_keeps_repay_withdraw_topup() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    t.set_position_limits(1, 1);

    t.repay(ALICE, "USDC", 500.0);
    t.withdraw(ALICE, "ETH", 0.5);
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.05);
    t.assert_supply_count(ALICE, 2);
    t.assert_borrow_count(ALICE, 2);

    assert_contract_error(
        t.try_supply(ALICE, "WBTC", 0.01),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    assert_contract_error(
        t.try_borrow(ALICE, "WBTC", 0.001),
        errors::POSITION_LIMIT_EXCEEDED,
    );
}

/// INV-RISK-04: a credit receiver at its supply limit cannot take a new asset. Here it already holds
/// USDC and ETH (two of two), and the seized legs include WBTC.
#[test]
fn rv_credit_receiver_new_asset_beyond_limit_rejected() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_position_limits(2, 2)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(LIQUIDATOR, "USDC", 100.0);
    t.supply(LIQUIDATOR, "ETH", 0.5);
    let receiver = t.account_id(LIQUIDATOR);
    t.set_price("ETH", usd(5_000));
    assert!(t.can_be_liquidated(ALICE));

    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(receiver))
            .map(|_| ()),
        errors::POSITION_LIMIT_EXCEEDED,
    );
}

/// INV-RISK-04: a seized leg the receiver already holds opens no slot. The same receiver takes the
/// credit when the victim's only collateral is USDC.
#[test]
fn rv_credit_receiver_existing_asset_allowed_within_limit() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_position_limits(2, 2)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(LIQUIDATOR, "USDC", 100.0);
    let receiver = t.account_id(LIQUIDATOR);
    let before = scaled_of(&t, receiver, "USDC");
    t.set_price("ETH", usd(3_000));
    assert!(t.can_be_liquidated(ALICE));

    let credited = t
        .try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(receiver))
        .expect("the USDC leg already exists on the receiver");
    assert_eq!(credited, receiver);
    assert!(
        scaled_of(&t, receiver, "USDC") > before,
        "the receiver's existing USDC position grew by the credited shares"
    );
}

/// INV-RISK-04: an account holds at most 16 delegates. The 17th grant is rejected with the registry
/// cap error.
#[test]
fn rv_delegate_grant_rejects_seventeenth_delegate() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account = t.account_id(ALICE);
    let owner = t.get_or_create_user(ALICE);
    for i in 0..16 {
        let delegate = t.get_or_create_user(&format!("delegate{i}"));
        t.ctrl_client().set_position_manager(&delegate, &true);
        t.ctrl_client().add_delegate(&owner, &account, &delegate);
    }
    let seventeenth = t.get_or_create_user("delegate16");
    t.ctrl_client().set_position_manager(&seventeenth, &true);
    assert_contract_error(
        map_try_ok_unit(
            t.ctrl_client()
                .try_add_delegate(&owner, &account, &seventeenth),
        ),
        GenericError::RegistryCapReached as u32,
    );
}

fn two_decimal_preset(price_wad: i128) -> MarketPreset {
    MarketPreset {
        name: "TWO",
        decimals: 2,
        price_wad,
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_borrowable: false,
            is_flashloanable: false,
            flashloan_fee: 0,
            liquidation_fees: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn three_decimal_preset() -> MarketPreset {
    MarketPreset {
        name: "THREE",
        decimals: 3,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_borrowable: false,
            is_flashloanable: false,
            flashloan_fee: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// INV-RISK-04: a supply leg below 3 decimals is its account's only supply position. A 2-decimal
/// asset cannot sit beside USDC in either order, while a 3-decimal asset can.
#[test]
fn rv_two_decimal_collateral_isolation_both_directions_three_decimal_allowed() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(two_decimal_preset(usd(10)))
        .with_market(three_decimal_preset())
        .build();

    t.supply(ALICE, "TWO", 1.0);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 100.0),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    t.supply(BOB, "USDC", 100.0);
    assert_contract_error(
        t.try_supply(BOB, "TWO", 1.0),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    t.supply(CAROL, "THREE", 1.0);
    t.try_supply(CAROL, "USDC", 100.0)
        .expect("a 3-decimal leg is not isolated");
    t.assert_supply_count(CAROL, 2);
    t.supply(DAVE, "TWO", 1.0);
    t.assert_supply_count(DAVE, 1);
}

/// INV-RISK-04 and formulas.md: below 3 decimals, a leg must keep at least two whole units while the
/// account has debt. The floor is `MIN_WHOLE_UNIT_COLLATERAL` (2) compared against
/// `to_asset_floor(decimals)`, which is a raw-unit count. So one whole unit of a 2-decimal asset
/// (100 raw) already clears it. Safe expectation: one whole unit with debt is refused.
#[test]
#[ignore = "RV-NOTE (LOW, doc ambiguity): formulas.md says 2 whole units; code and the Liqvid runbook count base units (shares). Encodes the literal doc reading"]
fn rv_finding_two_decimal_debt_floor_counts_raw_units_not_whole_units() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(two_decimal_preset(usd(10)))
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "TWO", 1.0);
    let observed = t.try_borrow(ALICE, "USDC", 7.0);
    std::println!(
        "RV-FINDING observed: 1 whole unit of TWO (100 raw, $10) borrows 7 USDC -> {observed:?}"
    );
    assert_contract_error(observed, errors::MIN_BORROW_COLLATERAL_NOT_MET);
}

/// Same root cause through the withdrawal floor, which the brief names: two units of TWO back 7 USDC,
/// and withdrawing one unit leaves one whole unit (100 raw, LTV 7.5 against debt 7) with debt.
#[test]
#[ignore = "RV-NOTE (LOW, doc ambiguity): formulas.md says 2 whole units; code and the Liqvid runbook count base units (shares). Encodes the literal doc reading"]
fn rv_finding_two_decimal_withdraw_leaving_one_unit_with_debt_admitted() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(two_decimal_preset(usd(10)))
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "TWO", 2.0);
    t.borrow(ALICE, "USDC", 7.0);
    let observed = t.try_withdraw(ALICE, "TWO", 1.0);
    std::println!(
        "RV-FINDING observed: withdraw 1 of 2 units of TWO (leaves 100 raw, $10) with 7 USDC debt -> {observed:?}"
    );
    assert_contract_error(observed, errors::MIN_BORROW_COLLATERAL_NOT_MET);
}

/// The gates that do hold on a 2-decimal leg with debt: LTV and health factor. A withdrawal that
/// breaks either is refused as insolvency, and a debt-free account may exit completely.
#[test]
fn rv_two_decimal_leg_with_debt_stays_ltv_and_hf_gated() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(two_decimal_preset(usd(10)))
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "TWO", 2.0);
    assert_contract_error(
        t.try_borrow(ALICE, "USDC", 16.0),
        errors::INSUFFICIENT_COLLATERAL,
    );
    t.borrow(ALICE, "USDC", 7.0);
    assert_contract_error(
        t.try_withdraw(ALICE, "TWO", 1.5),
        errors::INSUFFICIENT_COLLATERAL,
    );
    t.repay(ALICE, "USDC", 7.0);
    t.try_withdraw(ALICE, "TWO", 2.0)
        .expect("a debt-free account may exit completely");
}

/// INV-RISK-04: crediting a 2-decimal leg into a receiver that already holds USDC is refused. The
/// same liquidation paid as a transfer is not refused by the isolation rule.
#[test]
fn rv_credit_two_decimal_into_usdc_holder_rejected() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(two_decimal_preset(usd(10)))
        .build();
    t.supply(ALICE, "TWO", 2.0);
    t.borrow(ALICE, "USDC", 7.0);
    t.supply(LIQUIDATOR, "USDC", 1_000.0);
    let receiver = t.account_id(LIQUIDATOR);
    t.set_price("TWO", usd(1));
    assert!(t.can_be_liquidated(ALICE));

    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "USDC", 1.0, SeizeMode::Credit(receiver))
            .map(|_| ()),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 1.0)
        .expect("the same liquidation as a transfer is not refused by isolation");
}

// --- H5: the usage identity after every share writer -------------------------

/// ADR-0015 and INV-ACCT-10: after each share writer, every spoke usage row equals the positions it
/// caps, and the pool's `supplied - revenue` equals the sum of account scaled supply.
#[test]
fn rv_usage_identity_holds_after_every_share_writer() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .with_spoke_asset(SPOKE_B, "USDC", true, true)
        .with_spoke_asset(SPOKE_B, "ETH", true, true)
        .with_position_limits(4, 4)
        .build();
    const ASSETS: [&str; 2] = ["USDC", "ETH"];
    t.fund_router("USDC", 100_000.0);
    t.fund_router("ETH", 100.0);
    let steps_eth_to_usdc = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(3_000.0, 7));
    let steps_usdc_to_eth = build_aggregator_swap(&t, "USDC", "ETH", 0, f64_to_i128(0.02, 7));
    assert_identity(&t, &ASSETS, "empty book");

    // Supply, borrow, and both sides of a second spoke.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 2.0);
    assert_identity(&t, &ASSETS, "supply and borrow");
    t.supply(BOB, "ETH", 50.0);
    t.borrow(BOB, "USDC", 4_000.0);
    let dave = t.create_spoke_account(DAVE, SPOKE_B);
    t.supply_to(DAVE, dave, "USDC", 10_000.0);
    t.borrow_to(DAVE, dave, "ETH", 2.0);
    assert_identity(&t, &ASSETS, "two spokes");

    // Partial exits, a withdraw-all that prunes the account, and a repay-all.
    t.withdraw(ALICE, "USDC", 500.0);
    t.repay(BOB, "USDC", 1_000.0);
    t.supply(EVE, "USDC", 10.0);
    let eve = t.account_id(EVE);
    t.withdraw_all(EVE, "USDC");
    assert!(!t.account_exists(eve), "withdraw-all removed the account");
    assert_identity(&t, &ASSETS, "partial exits and account removal");
    t.supply(FRANK, "USDC", 1_000.0);
    t.borrow(FRANK, "ETH", 0.3);
    t.repay(FRANK, "ETH", 0.31);
    t.assert_borrow_count(FRANK, 0);
    assert_identity(&t, &ASSETS, "repay-all");

    // Flash position, multiply, swap collateral, and the same-asset netting.
    flash_try(
        &mut t,
        HANK,
        HARNESS_SPOKE,
        "ETH",
        ETH,
        "USDC",
        3_000 * USDC,
    )
    .expect("flash_position");
    assert_identity(&t, &ASSETS, "flash_position");
    t.multiply(
        IVAN,
        "USDC",
        1.0,
        "ETH",
        PositionMode::Multiply,
        &steps_eth_to_usdc,
    );
    assert_identity(&t, &ASSETS, "multiply");
    t.try_swap_collateral(FRANK, "USDC", 100.0, "ETH", &steps_usdc_to_eth)
        .expect("swap_collateral");
    assert_identity(&t, &ASSETS, "swap_collateral");
    t.supply(JACK, "USDC", 10_000.0);
    t.borrow(JACK, "USDC", 1_000.0);
    let empty = Bytes::new(&t.env);
    t.try_repay_debt_with_collateral(JACK, "USDC", 500.0, "USDC", &empty, false)
        .expect("netting");
    assert_identity(&t, &ASSETS, "repay_debt_with_collateral netting");

    // Liquidations: Transfer, then Credit into an existing and into a new receiver.
    t.supply(LIQUIDATOR, "ETH", 5.0);
    let liquidator_account = t.account_id(LIQUIDATOR);
    t.supply(GINA, "USDC", 10.0);
    t.borrow(GINA, "ETH", 0.003);
    t.set_price("USDC", usd_cents(40));
    assert!(t.can_be_liquidated(ALICE));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_identity(&t, &ASSETS, "transfer liquidation");
    t.liquidate_with_mode(
        LIQUIDATOR,
        ALICE,
        "ETH",
        0.5,
        SeizeMode::Credit(liquidator_account),
    );
    assert_identity(&t, &ASSETS, "credit into existing receiver");
    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.5, SeizeMode::Credit(0));
    assert_identity(&t, &ASSETS, "credit into new receiver");

    // Bad-debt cleanup of Gina's dust account.
    t.set_price("USDC", usd_cents(1));
    t.clean_bad_debt_for(GINA);
    assert_identity(&t, &ASSETS, "clean_bad_debt");
}

/// ADR-0015 and the spoke-usage row lifecycle: a listing with live supply or borrow usage cannot be
/// removed. Once both sides are zero it can, and the other spoke's listing keeps serving its users.
#[test]
fn rv_remove_asset_blocked_by_usage_allowed_at_zero() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .with_spoke_asset(SPOKE_B, "USDC", true, true)
        .with_spoke_asset(SPOKE_B, "ETH", true, true)
        .build();
    let dave = t.create_spoke_account(DAVE, SPOKE_B);
    t.supply_to(DAVE, dave, "USDC", 10_000.0);
    t.borrow_to(DAVE, dave, "ETH", 1.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let usdc = key(&t, "USDC");
    let eth = key(&t, "ETH");
    let ctrl = t.ctrl_client();
    assert_contract_error(
        map_try_ok_unit(ctrl.try_remove_asset_from_spoke(&usdc, &SPOKE_B)),
        errors::SPOKE_ASSET_IN_USE,
    );
    assert_contract_error(
        map_try_ok_unit(ctrl.try_remove_asset_from_spoke(&eth, &SPOKE_B)),
        errors::SPOKE_ASSET_IN_USE,
    );

    t.repay(DAVE, "ETH", 1.01);
    t.withdraw_all(DAVE, "USDC");
    assert_eq!(usage(&t, SPOKE_B, "USDC").supplied_scaled_ray, 0);
    assert_eq!(usage(&t, SPOKE_B, "ETH").borrowed_scaled_ray, 0);

    t.ctrl_client().remove_asset_from_spoke(&usdc, &SPOKE_B);
    t.ctrl_client().remove_asset_from_spoke(&eth, &SPOKE_B);
    assert_contract_error(
        t.try_supply_with_spoke("kate", "USDC", 1.0, SPOKE_B),
        errors::ASSET_NOT_IN_SPOKE,
    );

    // The harness spoke lists the same assets and is unaffected.
    t.withdraw(ALICE, "USDC", 500.0);
    t.repay(ALICE, "ETH", 0.5);
    t.set_price("USDC", usd_cents(10));
    assert!(t.can_be_liquidated(ALICE));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.2);
    assert_identity(&t, &["USDC", "ETH"], "other spoke after removal");
}

// --- H6: deprecated spoke and immutable binding ----------------------------

/// INV-AUTH-06 and ADR-0009: after deprecation no account may be created in the spoke and no
/// existing account may enter it. Exits and liquidation stay open, and Credit(0) can still open a
/// receiver there.
#[test]
fn rv_deprecated_spoke_blocks_entry_keeps_exit_and_liquidation() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();
    let alice = t.create_spoke_account(ALICE, SPOKE_B);
    t.supply_to(ALICE, alice, "USDC", 10_000.0);
    t.borrow_to(ALICE, alice, "USDT", 5_000.0);
    let bob = t.create_spoke_account(BOB, SPOKE_B);
    t.supply_to(BOB, bob, "USDC", 10_000.0);
    t.borrow_to(BOB, bob, "USDT", 1_000.0);
    t.fund_router("USDC", 100_000.0);
    t.ensure_approved_blend();

    t.remove_spoke_category(SPOKE_B);
    assert_contract_error(
        map_try_ok_unit(t.ctrl_client().try_remove_spoke(&SPOKE_B)),
        errors::SPOKE_DEPRECATED,
    );

    // No account may be created in the deprecated spoke.
    assert_contract_error(
        t.try_supply_with_spoke(CAROL, "USDC", 1.0, SPOKE_B),
        errors::SPOKE_DEPRECATED,
    );
    let steps = build_aggregator_swap(&t, "USDT", "USDC", 0, f64_to_i128(3_000.0, 7));
    assert_contract_error(
        multiply_try(&mut t, CAROL, SPOKE_B, "USDC", "USDT", 1_000_000, &steps),
        errors::SPOKE_DEPRECATED,
    );
    assert_contract_error(
        flash_try(
            &mut t,
            CAROL,
            SPOKE_B,
            "USDT",
            1_000_000,
            "USDC",
            3_000 * USDC,
        ),
        errors::SPOKE_DEPRECATED,
    );
    assert_contract_error(
        migrate_try(&mut t, CAROL, 0, SPOKE_B, &["USDC"], &[], &[]),
        errors::SPOKE_DEPRECATED,
    );

    // No existing account may take new exposure in it.
    assert_contract_error(
        t.try_supply_to_account(ALICE, ALICE, "USDC", 1.0),
        errors::SPOKE_DEPRECATED,
    );
    assert_contract_error(t.try_borrow(ALICE, "USDT", 100.0), errors::SPOKE_DEPRECATED);

    // Exits and liquidation stay open.
    t.try_withdraw(BOB, "USDC", 1_000.0)
        .expect("withdraw remains open in a deprecated spoke");
    t.repay(BOB, "USDT", 500.0);
    t.set_price("USDC", usd_cents(50));
    assert!(t.can_be_liquidated(ALICE));
    t.try_liquidate(LIQUIDATOR, ALICE, "USDT", 1.0)
        .expect("transfer liquidation remains open");
    let receiver = t
        .try_liquidate_with_mode(LIQUIDATOR, ALICE, "USDT", 1.0, SeizeMode::Credit(0))
        .expect("Credit(0) opens a receiver in a deprecated spoke");
    assert_eq!(
        t.ctrl_client().get_account_attributes(&receiver).spoke_id,
        SPOKE_B
    );
}

/// INV-AUTH-06 and ADR-0009: an account keeps the spoke it was created in. Every path that names a
/// spoke for an existing account must match it, and borrow, withdraw and repay use the stored one.
#[test]
fn rv_account_spoke_binding_rejects_other_spoke_on_every_path() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();
    let alice = t.create_spoke_account(ALICE, SPOKE_B);
    t.supply_to(ALICE, alice, "USDC", 10_000.0);
    t.supply(BOB, "USDC", 10_000.0);
    t.fund_router("USDC", 100_000.0);
    t.ensure_approved_blend();

    assert_contract_error(
        t.try_supply_with_spoke(ALICE, "USDC", 1.0, HARNESS_SPOKE),
        errors::SPOKE_MISMATCH,
    );
    assert_contract_error(
        t.try_supply_with_spoke(BOB, "USDC", 1.0, SPOKE_B),
        errors::SPOKE_MISMATCH,
    );
    let steps = build_aggregator_swap(&t, "USDT", "USDC", 0, f64_to_i128(3_000.0, 7));
    assert_contract_error(
        multiply_try_on(&mut t, ALICE, alice, HARNESS_SPOKE, &steps),
        errors::SPOKE_MISMATCH,
    );
    assert_contract_error(
        flash_on_account_try(
            &mut t,
            ALICE,
            alice,
            HARNESS_SPOKE,
            "USDT",
            1_000_000,
            "USDC",
            3_000 * USDC,
        ),
        errors::SPOKE_MISMATCH,
    );
    assert_contract_error(
        migrate_try(&mut t, ALICE, alice, HARNESS_SPOKE, &["USDC"], &[], &[]),
        errors::SPOKE_MISMATCH,
    );

    t.borrow_to(ALICE, alice, "USDT", 100.0);
    t.repay(ALICE, "USDT", 100.0);
    assert_eq!(
        t.ctrl_client().get_account_attributes(&alice).spoke_id,
        SPOKE_B,
        "rejected calls never rebind the account"
    );
}

/// Existing-account multiply on a different spoke, through the controller so the spoke is explicit.
fn multiply_try_on(
    t: &mut LendingTest,
    user: &str,
    account_id: u64,
    spoke: u32,
    steps: &Bytes,
) -> Result<u64, soroban_sdk::Error> {
    let caller = t.get_or_create_user(user);
    let coll = key(t, "USDC");
    let debt = key(t, "USDT");
    map_try_ok_value(t.ctrl_client().try_multiply(
        &caller,
        &account_id,
        &spoke,
        &coll,
        &1_000_000,
        &debt,
        &PositionMode::Multiply,
        steps,
        &None,
        &None,
    ))
}
