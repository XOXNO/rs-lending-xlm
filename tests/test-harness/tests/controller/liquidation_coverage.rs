use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Vec};
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, liquidatable_usdc_eth, usd_cents,
    usdc_preset, HubAssetKey, LendingTest, ALICE, LIQUIDATOR,
};

fn try_liquidate_payments(
    t: &mut LendingTest,
    liquidator: &str,
    target_user: &str,
    payments: Vec<(HubAssetKey, i128)>,
) -> Result<(), soroban_sdk::Error> {
    let liquidator_addr = t.get_or_create_user(liquidator);
    let account_id = t.resolve_account_id(target_user);
    let ctrl = t.ctrl_client();

    match ctrl.try_liquidate(&liquidator_addr, &account_id, &payments) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

#[test]
fn test_liquidation_aggregates_duplicate_debt_payments() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_oracle_single_spot("USDC");
    t.set_oracle_single_spot("ETH");

    let alice = "alice_excess";

    // Alice supplies 100k USDC and borrows 2.5 ETH (~$5000).
    t.supply(alice, "USDC", 100_000.0);
    t.borrow(alice, "ETH", 2.5);

    // Drop USDC to $0.06 => Collateral $6000. Debt $5000.
    // Weighted = 4800. HF = 0.96.
    t.set_price("USDC", usd_cents(6));

    t.assert_liquidatable(alice);

    t.liquidate_multi(LIQUIDATOR, alice, &[("ETH", 2.0), ("ETH", 0.1)]);

    let debt = t.borrow_balance(alice, "ETH");
    assert!(
        debt > 0.0 && debt < 2.5,
        "capped repayment should leave residual debt below principal, got {debt}"
    );
}

// `token_price` rejects zero oracle prices globally. Reading any price for an
// asset whose oracle returns zero panics immediately with
// `OracleError::InvalidPrice` (#217).
#[test]
#[should_panic(expected = "Error(Contract, #217)")]
fn test_oracle_rejects_zero_price_before_liquidation_check() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = "alice_zero";

    t.supply(alice, "USDC", 100.0);
    t.borrow(alice, "ETH", 0.02); // ~$40

    t.set_oracle_single_spot("USDC");
    t.set_price("USDC", 0);

    // Any price fetch for USDC panics with InvalidPrice.
    t.assert_liquidatable(alice);
}

#[test]
fn test_liquidation_seize_proportional_subunit_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    t.set_oracle_single_spot("USDC");
    t.set_oracle_single_spot("ETH");

    let alice = "alice_dust";

    // Supply 20k USDC for enough collateral to back a 6 ETH borrow.
    t.supply(alice, "USDC", 20_000.0);
    t.supply(alice, "ETH", 0.0000001); // 1 unit
    t.borrow(alice, "ETH", 6.0); // $12,000. Initial weighted = 16,000.

    // Drop USDC to $0.50 => collateral value $10,000, weighted $8,000.
    // Debt = $12,000. HF = 8000/12000 = 0.66.
    t.set_price("USDC", usd_cents(50));

    t.assert_liquidatable(alice);

    let usdc_before = t.supply_balance(alice, "USDC");
    let eth_before = t.supply_balance(alice, "ETH");
    let debt_before = t.borrow_balance(alice, "ETH");

    t.liquidate(LIQUIDATOR, alice, "ETH", 0.01);

    let eth_after = t.supply_balance(alice, "ETH");
    let debt_after = t.borrow_balance(alice, "ETH");
    assert!(
        debt_after < debt_before,
        "liquidation should repay ETH debt: before={debt_before}, after={debt_after}"
    );
    assert!(
        eth_after < eth_before || t.supply_balance(alice, "USDC") < usdc_before,
        "liquidation should seize collateral from ETH dust and/or USDC: eth {eth_before}->{eth_after}"
    );
    assert!(
        t.supply_balance(alice, "USDC") <= usdc_before,
        "USDC collateral should not grow on ETH-targeted liquidation"
    );
}

#[test]
fn test_liquidation_rejects_if_no_debt_repaid() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_oracle_single_spot("USDC");
    t.set_oracle_single_spot("ETH");
    t.supply("alice_rej", "USDC", 100.0);
    t.borrow("alice_rej", "ETH", 0.03);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable("alice_rej");

    // Pay with a tiny amount that rescale might turn into 0.
    let result = t.try_liquidate(LIQUIDATOR, "alice_rej", "ETH", 0.000000001);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_liquidation_multi_debt_capped() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_oracle_single_spot("USDC");
    t.set_oracle_single_spot("ETH");

    let alice = "alice_multi";

    // Alice supplies $1000 USDC, borrows 0.15 ETH (~$300) and $100 USDC.
    // Total debt ~$400.
    t.supply(alice, "USDC", 1000.0);
    t.borrow(alice, "ETH", 0.15);
    t.borrow(alice, "USDC", 100.0);

    // Drop USDC to $0.42. The USDC debt leg reprices with the collateral:
    // weighted = $336, debt = $300 ETH + $42 USDC = $342, HF = 0.98 --
    // shallow enough that the target-1.10 ideal (~$204) stays below the
    // first payment's debt capacity.
    t.set_price("USDC", usd_cents(42));

    t.assert_liquidatable(alice);

    // The liquidator pays with two different tokens.
    // Ideal repayment restoring HF to 1.10: (1.10*342 - 336) / (1.10 - 0.8*1.128)
    // = 40.2 / ~0.197 = ~$204.

    // 1. Pay with ETH (0.15 ETH = $300). This fulfills the ideal (~$204).
    // 2. Pay with USDC ($50). The contract must skip this (continue) because remaining_ideal <= 0.

    t.liquidate_multi(LIQUIDATOR, alice, &[("ETH", 0.15), ("USDC", 50.0)]);

    // The ETH debt should have dropped by the capped amount (~$204 worth of ETH).
    let debt_eth = t.borrow_balance(alice, "ETH");
    assert!(debt_eth < 0.15);

    // The USDC debt must remain UNCHANGED because the contract skipped it.
    let debt_usdc = t.borrow_balance(alice, "USDC");
    assert_eq!(debt_usdc, 100.0);
}

#[test]
fn test_liquidation_rejects_empty_payments() {
    let mut t = liquidatable_usdc_eth();

    let payments = Vec::new(&t.env);
    let result = try_liquidate_payments(&mut t, LIQUIDATOR, ALICE, payments);

    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_liquidation_rejects_negative_raw_payment() {
    let mut t = liquidatable_usdc_eth();

    let eth = t.resolve_asset("ETH");
    let payments = soroban_sdk::vec![&t.env, (hub_asset(eth), -1i128)];
    let result = try_liquidate_payments(&mut t, LIQUIDATOR, ALICE, payments);

    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_liquidation_rejects_unsupported_payment_asset() {
    let mut t = liquidatable_usdc_eth();

    let unsupported = Address::generate(&t.env);
    let payments = soroban_sdk::vec![&t.env, (hub_asset(unsupported), 1i128)];
    let result = try_liquidate_payments(&mut t, LIQUIDATOR, ALICE, payments);

    // Unlisted asset fails on the oracle probe (first gate).
    assert_contract_error(result, errors::ORACLE_NOT_CONFIGURED);
}

#[test]
fn test_liquidation_rejects_supported_payment_asset_without_debt_position() {
    let mut t = liquidatable_usdc_eth();

    let usdc = t.resolve_asset("USDC");
    let payments = soroban_sdk::vec![&t.env, (hub_asset(usdc), 1i128)];
    let result = try_liquidate_payments(&mut t, LIQUIDATOR, ALICE, payments);

    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}
