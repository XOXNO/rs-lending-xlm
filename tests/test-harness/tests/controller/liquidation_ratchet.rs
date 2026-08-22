use common::types::SeizeMode;
use test_harness::{
    eth_preset, hub_asset, usd_cents, usdt_stable_preset, LendingTest, ALICE, LIQUIDATOR,
};

fn liquidate_once(
    t: &mut LendingTest,
    debt_asset: &str,
    debt_amount: f64,
    coll_asset: &str,
    coll_price: f64,
) -> (f64, f64) {
    let coll_before = t.token_balance(LIQUIDATOR, coll_asset);
    let debt_before = t.total_debt(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, debt_asset, debt_amount);
    let coll_usd = (t.token_balance(LIQUIDATOR, coll_asset) - coll_before) * coll_price;
    let debt_usd = debt_before - t.total_debt(ALICE);
    assert!(
        debt_usd > 0.0 && coll_usd > 0.0,
        "liquidation must move positive value: coll_usd={coll_usd}, debt_usd={debt_usd}"
    );
    (coll_usd, debt_usd)
}

#[test]
fn test_partial_chain_does_not_out_extract_single_recoverable() {
    let build = || {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.get_or_create_user(LIQUIDATOR);
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "ETH", 3.0);
        t.set_price("USDC", usd_cents(69));
        t.assert_liquidatable(ALICE);
        t
    };
    let coll_price = 0.69;

    let mut single = build();
    let (s_coll, s_debt) = liquidate_once(&mut single, "ETH", 0.5, "USDC", coll_price);
    let single_multiple = s_coll / s_debt;

    let mut chain = build();
    let (mut c_coll, mut c_debt) = (0.0_f64, 0.0_f64);
    let mut prev_slice = f64::INFINITY;
    for _ in 0..5 {
        if !chain.can_be_liquidated(ALICE) {
            break;
        }
        let (coll, debt) = liquidate_once(&mut chain, "ETH", 0.1, "USDC", coll_price);
        let slice = coll / debt;

        assert!(
            slice <= prev_slice + 0.005,
            "recoverable partials must not ratchet bonus up: {prev_slice:.5} -> {slice:.5}"
        );
        prev_slice = slice;
        c_coll += coll;
        c_debt += debt;
    }
    let chain_multiple = c_coll / c_debt;

    assert!(
        chain_multiple <= single_multiple * 1.01,
        "chained partials must not out-extract a single liquidation: \
         chain={chain_multiple:.5}, single={single_multiple:.5}"
    );
}

#[test]
fn test_partial_chain_deep_does_not_ratchet() {
    let build = || {
        let mut t = LendingTest::new().standard_two_asset_dust_disabled();
        t.get_or_create_user(LIQUIDATOR);
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "ETH", 3.0);
        t.set_price("USDC", usd_cents(25));
        t.assert_liquidatable(ALICE);
        t
    };
    let coll_price = 0.25;

    let mut single = build();
    let (s_coll, s_debt) = liquidate_once(&mut single, "ETH", 0.5, "USDC", coll_price);
    let single_multiple = s_coll / s_debt;

    assert!(
        single_multiple <= 1.26,
        "deep bonus bounded by the per-threshold max: multiple={single_multiple:.5}"
    );

    let mut chain = build();
    let (mut c_coll, mut c_debt) = (0.0_f64, 0.0_f64);
    let mut first_slice: Option<f64> = None;
    for _ in 0..5 {
        if !chain.can_be_liquidated(ALICE) {
            break;
        }
        let (coll, debt) = liquidate_once(&mut chain, "ETH", 0.1, "USDC", coll_price);
        let slice = coll / debt;
        match first_slice {
            None => first_slice = Some(slice),

            Some(first) => assert!(
                slice <= first * 1.01,
                "deep partials must not ratchet bonus: first={first:.5}, slice={slice:.5}"
            ),
        }
        c_coll += coll;
        c_debt += debt;
    }
    let chain_multiple = c_coll / c_debt;
    assert!(
        chain_multiple <= single_multiple * 1.01,
        "chained deep partials must not out-extract a single liquidation: \
         chain={chain_multiple:.5}, single={single_multiple:.5}"
    );
}

#[test]
fn test_partial_chain_no_ratchet_spoke() {
    let build = || {
        let mut t = LendingTest::new()
            .stablecoin_spoke_two_asset()
            .with_dust_disabled_all_markets()
            .build();
        t.get_or_create_user(LIQUIDATOR);
        t.create_spoke_account(ALICE, 2);
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "USDT", 9_500.0);
        t.set_price("USDC", usd_cents(85));
        t.assert_liquidatable(ALICE);
        t
    };
    let coll_price = 0.85;

    let mut single = build();
    let (s_coll, s_debt) = liquidate_once(&mut single, "USDT", 1_000.0, "USDC", coll_price);
    let single_multiple = s_coll / s_debt;

    let mut chain = build();
    let (mut c_coll, mut c_debt) = (0.0_f64, 0.0_f64);
    let mut first_slice: Option<f64> = None;
    for _ in 0..5 {
        if !chain.can_be_liquidated(ALICE) {
            break;
        }
        let (coll, debt) = liquidate_once(&mut chain, "USDT", 200.0, "USDC", coll_price);
        let slice = coll / debt;
        match first_slice {
            None => first_slice = Some(slice),
            Some(first) => assert!(
                slice <= first * 1.01,
                "spoke partials must not ratchet bonus: first={first:.5}, slice={slice:.5}"
            ),
        }
        c_coll += coll;
        c_debt += debt;
    }
    let chain_multiple = c_coll / c_debt;
    assert!(
        chain_multiple <= single_multiple * 1.01,
        "chained spoke partials must not out-extract a single liquidation: \
         chain={chain_multiple:.5}, single={single_multiple:.5}"
    );
}

#[test]
fn test_solvent_toxic_rejects_partial_and_accepts_full_close() {
    use test_harness::{assert_contract_error, errors, usd_cents as cents};

    let mut t = LendingTest::new()
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .build();
    t.get_or_create_user(LIQUIDATOR);

    t.supply(ALICE, "USDT", 10_000.0);
    t.borrow(ALICE, "ETH", 4.0);

    t.set_price("USDT", cents(81));
    t.assert_liquidatable(ALICE);

    let partial = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(partial, errors::FULL_CLOSE_REQUIRED);

    let liq_usdt_before = t.token_balance(LIQUIDATOR, "USDT");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 4.1);
    assert!(
        t.find_account_id(ALICE).is_none(),
        "full close must clean up the emptied account"
    );
    let seized_usdt = t.token_balance(LIQUIDATOR, "USDT") - liq_usdt_before;
    assert!(
        seized_usdt > 9_900.0,
        "full close seizes ~all 10k USDT collateral, got {seized_usdt}"
    );
}

/// Victim scaled-supply remaining after a liquidation. `process_liquidation`
/// (fees, measured repay, `scale_seizures_to_received`) is live here — not the
/// plan-only `liquidate_slice` helper.
fn scaled_supply(t: &LendingTest, user: &str, asset: &str) -> i128 {
    let Some(account_id) = t.find_account_id(user) else {
        return 0;
    };
    t.ctrl_client()
        .get_account_positions(&account_id)
        .0
        .get(hub_asset(t.resolve_asset(asset)))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

fn additivity_book() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.get_or_create_user(LIQUIDATOR);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(69));
    t.assert_liquidatable(ALICE);
    t
}

fn execute_n_vs_one(mode: SeizeMode, slices: i128, slice: f64) {
    let offer_sum = slice * slices as f64;

    let mut single = additivity_book();
    let single_coll_0 = scaled_supply(&single, ALICE, "USDC");
    let single_debt_0 = single.borrow_balance_raw(ALICE, "ETH");
    single.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", offer_sum, mode);
    let single_seized = single_coll_0 - scaled_supply(&single, ALICE, "USDC");
    let single_repaid = single_debt_0 - single.borrow_balance_raw(ALICE, "ETH");

    let mut chain = additivity_book();
    let chain_coll_0 = scaled_supply(&chain, ALICE, "USDC");
    let chain_debt_0 = chain.borrow_balance_raw(ALICE, "ETH");
    for _ in 0..slices {
        assert!(
            chain.can_be_liquidated(ALICE),
            "chain must stay liquidatable for every slice so the comparison is a real split"
        );
        chain.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", slice, mode);
    }
    let chain_seized = chain_coll_0 - scaled_supply(&chain, ALICE, "USDC");
    let chain_repaid = chain_debt_0 - chain.borrow_balance_raw(ALICE, "ETH");

    assert_eq!(
        chain_repaid, single_repaid,
        "both paths must retire the same debt or the seize comparison is meaningless"
    );
    assert!(single_repaid > 0, "the close must actually repay");
    assert!(single_seized > 0, "the close must actually seize");

    // One scaled unit of floor slack per slice, matching the plan-level chain test.
    let tolerance = slices;
    assert!(
        chain_seized <= single_seized + tolerance,
        "N execute partials out-seized one close of the sum: chain={chain_seized} \
         single={single_seized} mode={mode:?}"
    );
}

#[test]
fn execute_n_partials_do_not_out_seize_one_close_of_the_sum_transfer() {
    execute_n_vs_one(SeizeMode::Transfer, 4, 0.1);
}

#[test]
fn execute_n_partials_do_not_out_seize_one_close_of_the_sum_credit() {
    execute_n_vs_one(SeizeMode::Credit(0), 4, 0.1);
}
