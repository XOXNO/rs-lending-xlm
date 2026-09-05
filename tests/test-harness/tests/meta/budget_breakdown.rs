use soroban_sdk::testutils::budget::ContractCostType;
use soroban_sdk::Env;
use test_harness::{
    build_aggregator_swap, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset,
    LendingTest, ALICE, BOB,
};

/// Mainnet per-transaction limits, the same pair
/// `tests/fuzz/strategy_multiply_budget.rs:7-8` asserts against.
const MAINNET_CPU_BUDGET: u64 = 100_000_000;
const MAINNET_MEM_BUDGET: u64 = 41_943_040;

/// These six paths are held to a quarter of the mainnet limit. The heaviest
/// measures ~9.9M cpu / ~5.6M mem, so this is ~2x headroom: loose enough not to
/// flap on ordinary drift, tight enough that a 3x regression fails HERE instead
/// of silently eating the margin a bigger position or an extra market needs.
/// Bumping either constant is a reviewed diff, which is the point.
const CPU_CEILING: u64 = MAINNET_CPU_BUDGET / 4;
const MEM_CEILING: u64 = MAINNET_MEM_BUDGET / 4;

/// Prints the CPU breakdown AND enforces the mainnet budget it is measured
/// against -- printing alone left a 10x regression on any measured path green,
/// and the two `reset_unlimited()` paths had no bound at all. Same shape as
/// `meta/footprint_test.rs`'s `assert_res`.
fn dump(env: &Env, label: &str) {
    let b = env.cost_estimate().budget();
    let total_cpu = b.cpu_instruction_cost();
    let total_mem = b.memory_bytes_cost();
    std::println!("\n========== {label} ==========");
    std::println!("  TOTAL                          cpu={total_cpu:>12}   mem={total_mem:>10}");

    let mut rows: std::vec::Vec<(ContractCostType, u64, u64, Option<u64>)> = std::vec::Vec::new();
    for ct in ContractCostType::VARIANTS.iter().copied() {
        let tr = b.tracker(ct);
        if tr.cpu > 0 || tr.iterations > 0 {
            rows.push((ct, tr.cpu, tr.iterations, tr.inputs));
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.1));

    for (ct, cpu, iters, inputs) in rows {
        let pct = if total_cpu > 0 {
            cpu as f64 * 100.0 / total_cpu as f64
        } else {
            0.0
        };
        std::println!(
            "  {:<30?} cpu={:>11}  ({:>5.1}%)  iters={:>7}  inputs={:?}",
            ct,
            cpu,
            pct,
            iters,
            inputs
        );
    }

    assert!(
        total_cpu <= CPU_CEILING,
        "{label}: cpu={total_cpu} exceeds the ceiling {CPU_CEILING} \
         (mainnet limit {MAINNET_CPU_BUDGET})"
    );
    assert!(
        total_mem <= MEM_CEILING,
        "{label}: mem={total_mem} exceeds the ceiling {MEM_CEILING} \
         (mainnet limit {MAINNET_MEM_BUDGET})"
    );
}

#[test]
fn budget_withdraw_one_asset_no_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_budget_enabled()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);

    t.advance_time(86_400);

    let mut b = t.env.cost_estimate().budget();
    b.reset_default();
    t.withdraw(ALICE, "USDC", 1_000.0);
    dump(
        &t.env,
        "withdraw 1 asset, NO debt (1 accrual + 1 supply write)",
    );
}

#[test]
fn budget_withdraw_with_debt_hf_check() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_budget_enabled()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.advance_time(86_400);

    let mut b = t.env.cost_estimate().budget();
    b.reset_default();
    t.withdraw(ALICE, "USDC", 1_000.0);
    dump(
        &t.env,
        "withdraw 1 asset, WITH debt (2 accruals + HF valuation + supply write)",
    );
}

#[test]
fn budget_withdraw_5_collateral_double_pass() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(xlm_preset())
        .with_budget_enabled()
        .build();

    t.env.cost_estimate().budget().reset_unlimited();

    t.supply(ALICE, "USDC", 100_000.0);
    let a = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, a, "USDT", 100_000.0);
    t.supply_to(ALICE, a, "ETH", 50.0);
    t.supply_to(ALICE, a, "WBTC", 2.0);
    t.supply_to(ALICE, a, "XLM", 1_000_000.0);

    t.borrow(ALICE, "XLM", 50_000.0);
    t.advance_time(86_400);

    // Measured unbounded on purpose: enforcing the default budget here exhausts
    // it inside the test env's own auth recorder, not in protocol code. The
    // bound lives in `dump`'s assertion instead.
    let mut b = t.env.cost_estimate().budget();
    b.reset_unlimited();
    b.reset_tracker();
    t.withdraw(ALICE, "USDC", 1_000.0);
    dump(
        &t.env,
        "withdraw, 5 collateral + 1 debt (double-pass LTV+HF over 5)",
    );
}

#[test]
fn budget_supply_baseline() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_budget_enabled()
        .build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.advance_time(86_400);

    let mut b = t.env.cost_estimate().budget();
    b.reset_default();
    t.supply(ALICE, "USDC", 1_000.0);
    dump(&t.env, "supply (1 accrual + 1 supply write)");
}

#[test]
fn budget_borrow_baseline() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_budget_enabled()
        .build();

    t.supply(BOB, "ETH", 1_000.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.advance_time(86_400);

    let mut b = t.env.cost_estimate().budget();
    b.reset_default();
    t.borrow(ALICE, "ETH", 0.1);
    dump(&t.env, "borrow (2 accruals + HF valuation + borrow write)");
}

#[test]
fn budget_swap_collateral_full() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_budget_enabled()
        .build();

    t.env.cost_estimate().budget().reset_unlimited();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.fund_router("ETH", 10.0);
    t.advance_time(86_400);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 200_000_000_000, 10_0000000);

    // Measured unbounded on purpose: enforcing the default budget here exhausts
    // it inside the test env's own auth recorder, not in protocol code. The
    // bound lives in `dump`'s assertion instead.
    let mut b = t.env.cost_estimate().budget();
    b.reset_unlimited();
    b.reset_tracker();
    t.swap_collateral(ALICE, "USDC", 20_000.0, "ETH", &steps);
    dump(
        &t.env,
        "swap_collateral USDC->ETH (withdraw + swap + deposit + finalize)",
    );
}
