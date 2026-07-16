//! Integration tests for `migrate_from_blend`.
//!
//! Each test registers a faithful `MockBlend` (per-user position accounting),
//! seeds ALICE's Blend collateral/supply/debt, funds the mock with the
//! underlying tokens it must pay out, then migrates and asserts the resulting
//! controller positions, the refund-reconciled debt, and that Blend is emptied.
//!
//! Borrow-checker note: all `&mut t` operations (`get_or_create_user`,
//! `create_account`, `supply`) run BEFORE the `MockBlendClient` is created,
//! because the client holds an immutable borrow of `t.env`.

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, IntoVal, Val, Vec as SorobanVec};
use test_harness::mock_blend::{
    BlendRequest as MockBlendRequest, MockBlend, MockBlendClient, MockBlendError, KIND_COLLATERAL,
    KIND_LIABILITY, KIND_SUPPLY,
};
use test_harness::{
    assert_contract_error, errors, eth_preset, helpers::f64_to_i128, usdc_preset, LendingTest,
    ALICE, HARNESS_HUB,
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Records `user`'s Blend balance for `kind` and, for collateral/supply (which
/// the mock pays out from its own balance), mints the underlying to the mock.
fn seed_position(
    t: &LendingTest,
    blend: &MockBlendClient,
    blend_addr: &Address,
    user: &Address,
    asset_name: &str,
    kind: u32,
    amount: f64,
) {
    let market = t.resolve_market(asset_name);
    let raw = f64_to_i128(amount, market.decimals);
    let asset = t.resolve_asset(asset_name);
    blend.seed(user, &asset, &kind, &raw);
    if kind != KIND_LIABILITY {
        market.token_admin.mint(blend_addr, &raw);
    }
}

fn empty_assets(t: &LendingTest) -> SorobanVec<Address> {
    SorobanVec::new(&t.env)
}

fn empty_debt(t: &LendingTest) -> SorobanVec<(Address, i128)> {
    SorobanVec::new(&t.env)
}

fn register_approved_blend(t: &LendingTest) -> Address {
    let addr = t.env.register(MockBlend, ());
    let admin = t.admin();
    t.gov_client().execute_immediate(
        &admin,
        &governance_interface::AdminOperation::ApproveBlendPool(addr.clone()),
    );
    addr
}

/// Normalizes a `try_migrate_from_blend` result to `Result<u64, Error>` for
/// `assert_contract_error` (covers our typed errors and sub-contract traps).
macro_rules! revert_result {
    ($call:expr) => {
        match $call {
            Ok(Ok(_)) => panic!("expected a revert but migration succeeded"),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected a contract error, got InvokeError")),
        }
    };
}

// ── happy paths ───────────────────────────────────────────────────────────────

/// Collateral-only migration (no debt): Blend collateral is withdrawn and
/// re-supplied as collateral here; Blend is emptied.
#[test]
fn test_migrate_collateral_only() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        1000.0,
    );

    let usdc = t.resolve_asset("USDC");
    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc.clone()]),
        &empty_assets(&t),
        &empty_debt(&t),
    );

    assert!(account_id > 0, "should create a new account");
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (990.0..=1010.0).contains(&supply),
        "USDC supply should be ~1000, got {supply}"
    );
    assert_eq!(
        blend.position(&caller, &usdc, &KIND_COLLATERAL),
        0,
        "Blend collateral should be fully withdrawn"
    );
    assert!(t.health_factor_for(ALICE, account_id) > 1.0);
}

/// Non-collateral supply migration (REQ_WITHDRAW path): Blend `supply` becomes
/// collateral here (we have no non-collateral concept).
#[test]
fn test_migrate_supply_only() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(&t, &blend, &blend_addr, &caller, "USDC", KIND_SUPPLY, 500.0);

    let usdc = t.resolve_asset("USDC");
    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &empty_assets(&t),
        &SorobanVec::from_array(&t.env, [usdc.clone()]),
        &empty_debt(&t),
    );

    assert!(account_id > 0);
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (490.0..=510.0).contains(&supply),
        "USDC supply should be ~500, got {supply}"
    );
    assert_eq!(blend.position(&caller, &usdc, &KIND_SUPPLY), 0);
}

#[test]
fn test_migrate_ignores_listed_asset_with_zero_blend_balance() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let usdc = t.resolve_asset("USDC");

    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc]),
        &empty_assets(&t),
        &empty_debt(&t),
    );

    assert!(account_id > 0);
    assert!(!t.ctrl_client().account_exists(&account_id));
}

/// Debt + collateral migration (the flash-borrow flow). Blend: 2000 USDC
/// collateral + 0.5 ETH debt. Cap is 0.6 ETH (buffer); the controller borrows
/// 0.6 ETH, repays Blend (which refunds 0.1 ETH), and reconciles the refund so
/// the resulting debt is exactly the migrated 0.5 ETH — NOT the 0.6 cap.
#[test]
fn test_migrate_debt_and_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        2000.0,
    );
    seed_position(&t, &blend, &blend_addr, &caller, "ETH", KIND_LIABILITY, 0.5);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let cap = f64_to_i128(0.6, t.resolve_market("ETH").decimals);

    let collateral_assets = SorobanVec::from_array(&t.env, [usdc.clone()]);
    let supply_assets = empty_assets(&t);
    let debt_caps = SorobanVec::from_array(&t.env, [(eth.clone(), cap)]);
    let args: SorobanVec<Val> = (
        caller.clone(),
        0u64,
        1u32,
        HARNESS_HUB,
        blend_addr.clone(),
        collateral_assets.clone(),
        supply_assets.clone(),
        debt_caps.clone(),
    )
        .into_val(&t.env);
    let debt_submit_args: SorobanVec<Val> = (
        caller.clone(),
        t.controller.clone(),
        t.controller.clone(),
        SorobanVec::from_array(
            &t.env,
            [MockBlendRequest {
                request_type: 5,
                address: eth.clone(),
                amount: cap,
            }],
        ),
    )
        .into_val(&t.env);
    let withdraw_submit_args: SorobanVec<Val> = (
        caller.clone(),
        t.controller.clone(),
        t.controller.clone(),
        SorobanVec::from_array(
            &t.env,
            [MockBlendRequest {
                request_type: 3,
                address: usdc.clone(),
                amount: i128::MAX,
            }],
        ),
    )
        .into_val(&t.env);
    let sub_invokes = [
        MockAuthInvoke {
            contract: &blend_addr,
            fn_name: "submit",
            args: debt_submit_args,
            sub_invokes: &[],
        },
        MockAuthInvoke {
            contract: &blend_addr,
            fn_name: "submit",
            args: withdraw_submit_args,
            sub_invokes: &[],
        },
    ];
    let invoke = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "migrate_from_blend",
        args,
        sub_invokes: &sub_invokes,
    };
    let auths = [MockAuth {
        address: &caller,
        invoke: &invoke,
    }];
    let account_id = t.ctrl_client().mock_auths(&auths).migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &collateral_assets,
        &supply_assets,
        &debt_caps,
    );

    assert!(account_id > 0);
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (1990.0..=2010.0).contains(&supply),
        "USDC supply should be ~2000, got {supply}"
    );
    // The refund reconciliation must net the debt down to the migrated 0.5 ETH,
    // not the 0.6 cap that was transiently borrowed.
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.49..=0.51).contains(&borrow),
        "ETH borrow should be reconciled to ~0.5 (not the 0.6 cap), got {borrow}"
    );
    assert_eq!(blend.position(&caller, &usdc, &KIND_COLLATERAL), 0);
    assert_eq!(blend.position(&caller, &eth, &KIND_LIABILITY), 0);
    assert!(t.health_factor_for(ALICE, account_id) > 1.0);
}

/// Same-asset loop (the looping pattern): Blend holds USDC collateral AND USDC
/// debt in the same reserve. Migrates faithfully to USDC collateral + USDC debt
/// in the controller via the two-phase submit (repay phase, then withdraw phase),
/// so the repay-refund delta and the collateral-withdraw delta never alias.
#[test]
fn test_migrate_same_asset_loop() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        1000.0,
    );
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_LIABILITY,
        400.0,
    );

    let usdc = t.resolve_asset("USDC");
    // Cap 500 > the 400 actual debt: Blend refunds 100, which the reconcile nets
    // back so the resulting debt is exactly the migrated 400 — not the 500 cap.
    let cap = f64_to_i128(500.0, t.resolve_market("USDC").decimals);

    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc.clone()]),
        &empty_assets(&t),
        &SorobanVec::from_array(&t.env, [(usdc.clone(), cap)]),
    );

    assert!(account_id > 0);
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (990.0..=1010.0).contains(&supply),
        "USDC collateral should be ~1000, got {supply}"
    );
    let borrow = t.borrow_balance_for(ALICE, account_id, "USDC");
    assert!(
        (395.0..=405.0).contains(&borrow),
        "USDC debt should be reconciled to ~400 (not the 500 cap), got {borrow}"
    );
    assert_eq!(
        blend.position(&caller, &usdc, &KIND_COLLATERAL),
        0,
        "Blend collateral should be fully withdrawn"
    );
    assert_eq!(
        blend.position(&caller, &usdc, &KIND_LIABILITY),
        0,
        "Blend debt should be fully repaid"
    );
    assert!(t.health_factor_for(ALICE, account_id) > 1.0);
}

/// Migrating into an existing account adds positions to it.
#[test]
fn test_migrate_into_existing_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let account_id = t.create_account(ALICE);
    t.supply(ALICE, "USDC", 100.0);
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        500.0,
    );

    let usdc = t.resolve_asset("USDC");
    let returned_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &account_id,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc]),
        &empty_assets(&t),
        &empty_debt(&t),
    );

    assert_eq!(returned_id, account_id, "should reuse the existing account");
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (590.0..=610.0).contains(&supply),
        "USDC supply should be original 100 + migrated 500 = ~600, got {supply}"
    );
}

// ── reverts ───────────────────────────────────────────────────────────────────

/// All-empty params are rejected with INVALID_PAYMENTS.
#[test]
fn test_migrate_empty_params_rejected() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);

    let result: Result<u64, soroban_sdk::Error> =
        revert_result!(t.ctrl_client().try_migrate_from_blend(
            &caller,
            &0u64,
            &1u32,
            &HARNESS_HUB,
            &blend_addr,
            &empty_assets(&t),
            &empty_assets(&t),
            &empty_debt(&t),
        ));
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

/// Cap equals actual Blend debt: no over-repay refund. Net controller debt must
/// still equal the migrated liability (not zero, not inflated).
#[test]
fn test_migrate_debt_cap_exact_no_refund() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        2000.0,
    );
    seed_position(&t, &blend, &blend_addr, &caller, "ETH", KIND_LIABILITY, 0.5);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let cap = f64_to_i128(0.5, t.resolve_market("ETH").decimals);

    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc.clone()]),
        &empty_assets(&t),
        &SorobanVec::from_array(&t.env, [(eth.clone(), cap)]),
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.49..=0.51).contains(&borrow),
        "exact-cap migrate must leave ~0.5 ETH debt (no refund path), got {borrow}"
    );
    assert_eq!(blend.position(&caller, &eth, &KIND_LIABILITY), 0);
    assert_eq!(
        t.env.as_contract(&t.controller, || {
            soroban_sdk::token::Client::new(&t.env, &eth).balance(&t.controller)
        }),
        0,
        "controller must not retain debt-token dust after exact-cap reconcile"
    );
}

/// Debt asset listed with a positive cap but zero Blend liability: full amount is
/// refunded and repaid, so controller net debt on that asset is zero.
#[test]
fn test_migrate_zero_blend_liability_cap_nets_zero_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        2000.0,
    );
    // No ETH liability seeded — only a spurious debt_cap.

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let cap = f64_to_i128(0.4, t.resolve_market("ETH").decimals);

    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc]),
        &empty_assets(&t),
        &SorobanVec::from_array(&t.env, [(eth.clone(), cap)]),
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        borrow == 0.0,
        "zero Blend liability + full refund must net zero controller debt, got {borrow}"
    );
    assert_eq!(blend.position(&caller, &eth, &KIND_LIABILITY), 0);
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (1990.0..=2010.0).contains(&supply),
        "collateral sweep must still land, got {supply}"
    );
}

/// Two debt assets, each with a buffer above true liability: each refund is
/// reconciled independently so neither debt sticks at its cap.
#[test]
fn test_migrate_multi_debt_refund_reconciles_each() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        5000.0,
    );
    seed_position(&t, &blend, &blend_addr, &caller, "ETH", KIND_LIABILITY, 0.5);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_LIABILITY,
        200.0,
    );

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_cap = f64_to_i128(0.7, t.resolve_market("ETH").decimals);
    let usdc_cap = f64_to_i128(250.0, t.resolve_market("USDC").decimals);

    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc.clone()]),
        &empty_assets(&t),
        &SorobanVec::from_array(&t.env, [(eth.clone(), eth_cap), (usdc.clone(), usdc_cap)]),
    );

    let eth_borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    let usdc_borrow = t.borrow_balance_for(ALICE, account_id, "USDC");
    assert!(
        (0.49..=0.51).contains(&eth_borrow),
        "ETH debt must reconcile to ~0.5 not 0.7 cap, got {eth_borrow}"
    );
    assert!(
        (195.0..=205.0).contains(&usdc_borrow),
        "USDC debt must reconcile to ~200 not 250 cap, got {usdc_borrow}"
    );
    assert_eq!(blend.position(&caller, &eth, &KIND_LIABILITY), 0);
    assert_eq!(blend.position(&caller, &usdc, &KIND_LIABILITY), 0);
    assert!(t.health_factor_for(ALICE, account_id) > 1.0);
}

/// Pre-existing controller inventory of a debt asset is not swept into refund
/// math: snapshot is taken before the zero-fee borrow, so only Blend's refund
/// delta is repaid.
#[test]
fn test_migrate_refund_ignores_preexisting_controller_balance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        2000.0,
    );
    seed_position(&t, &blend, &blend_addr, &caller, "ETH", KIND_LIABILITY, 0.5);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_dec = t.resolve_market("ETH").decimals;
    let stuck = f64_to_i128(0.25, eth_dec);
    t.resolve_market("ETH")
        .token_admin
        .mint(&t.controller, &stuck);

    let cap = f64_to_i128(0.6, eth_dec);
    let account_id = t.ctrl_client().migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &SorobanVec::from_array(&t.env, [usdc]),
        &empty_assets(&t),
        &SorobanVec::from_array(&t.env, [(eth.clone(), cap)]),
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.49..=0.51).contains(&borrow),
        "debt must still reconcile to ~0.5, got {borrow}"
    );
    let controller_eth = t.env.as_contract(&t.controller, || {
        soroban_sdk::token::Client::new(&t.env, &eth).balance(&t.controller)
    });
    assert_eq!(
        controller_eth, stuck,
        "pre-existing controller ETH must remain (not used as refund or swept)"
    );
}

/// A debt asset listed twice in `debt_caps` is rejected before Blend calls.
/// An asset may appear in both a withdraw role and the debt role.
#[test]
fn test_migrate_duplicate_debt_rejected() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let usdc = t.resolve_asset("USDC");
    let cap = f64_to_i128(1.0, t.resolve_market("USDC").decimals);

    let result: Result<u64, soroban_sdk::Error> =
        revert_result!(t.ctrl_client().try_migrate_from_blend(
            &caller,
            &0u64,
            &1u32,
            &HARNESS_HUB,
            &blend_addr,
            &empty_assets(&t),
            &empty_assets(&t),
            &SorobanVec::from_array(&t.env, [(usdc.clone(), cap), (usdc, cap)]),
        ));
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}

#[test]
fn test_migrate_unlisted_withdraw_asset_rejected_before_token_call() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let unlisted = Address::generate(&t.env);

    let result: Result<u64, soroban_sdk::Error> =
        revert_result!(t.ctrl_client().try_migrate_from_blend(
            &caller,
            &0u64,
            &1u32,
            &HARNESS_HUB,
            &blend_addr,
            &SorobanVec::from_array(&t.env, [unlisted]),
            &empty_assets(&t),
            &empty_debt(&t),
        ));
    assert_contract_error(result, errors::PAIR_NOT_ACTIVE);
}

/// A debt cap below the actual Blend debt leaves Blend debt after the
/// collateral withdrawal, so Blend (the mock) reverts its post-action health
/// check and the whole migration rolls back.
#[test]
fn test_migrate_debt_cap_too_low_reverts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        2000.0,
    );
    seed_position(&t, &blend, &blend_addr, &caller, "ETH", KIND_LIABILITY, 0.5);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let cap = f64_to_i128(0.3, t.resolve_market("ETH").decimals); // < 0.5 actual debt

    let result: Result<u64, soroban_sdk::Error> =
        revert_result!(t.ctrl_client().try_migrate_from_blend(
            &caller,
            &0u64,
            &1u32,
            &HARNESS_HUB,
            &blend_addr,
            &SorobanVec::from_array(&t.env, [usdc]),
            &empty_assets(&t),
            &SorobanVec::from_array(&t.env, [(eth, cap)]),
        ));
    assert_contract_error(result, MockBlendError::HealthCheckFailed as u32);
}

/// A migrated position whose debt exceeds its collateral's borrowing power
/// reverts at the end-state health gate (INSUFFICIENT_COLLATERAL).
#[test]
fn test_migrate_unhealthy_end_state_reverts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    // Tiny collateral ($100) against a large 0.5 ETH (~$1000) debt.
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        100.0,
    );
    seed_position(&t, &blend, &blend_addr, &caller, "ETH", KIND_LIABILITY, 0.5);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let cap = f64_to_i128(0.6, t.resolve_market("ETH").decimals);

    let result: Result<u64, soroban_sdk::Error> =
        revert_result!(t.ctrl_client().try_migrate_from_blend(
            &caller,
            &0u64,
            &1u32,
            &HARNESS_HUB,
            &blend_addr,
            &SorobanVec::from_array(&t.env, [usdc]),
            &empty_assets(&t),
            &SorobanVec::from_array(&t.env, [(eth, cap)]),
        ));
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

/// A `blend_pool` not on the governance allow-list is rejected before any
/// borrow or external call (closes the arbitrary-pool / free-flash-loan vector).
#[test]
fn test_migrate_unapproved_blend_pool_reverts() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let caller = t.get_or_create_user(ALICE);
    // Register a MockBlend but do NOT approve it.
    let blend_addr = t.env.register(MockBlend, ());
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    seed_position(
        &t,
        &blend,
        &blend_addr,
        &caller,
        "USDC",
        KIND_COLLATERAL,
        1000.0,
    );

    let usdc = t.resolve_asset("USDC");
    let result: Result<u64, soroban_sdk::Error> =
        revert_result!(t.ctrl_client().try_migrate_from_blend(
            &caller,
            &0u64,
            &1u32,
            &HARNESS_HUB,
            &blend_addr,
            &SorobanVec::from_array(&t.env, [usdc]),
            &empty_assets(&t),
            &empty_debt(&t),
        ));
    assert_contract_error(result, errors::BLEND_POOL_NOT_APPROVED);
}
