use soroban_sdk::testutils::budget::ContractCostType;
use soroban_sdk::Env;
use test_harness::{
    build_aggregator_swap, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset,
    LendingTest, ALICE,
};

/// Dump non-zero per-cost-type CPU, sorted by CPU descending, with each type's
/// share of the measured total. `iters`/`inputs` expose whether a type's cost
/// is iteration-driven (linear models) or constant.
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
}

/// 1 supply position, no debt. `withdraw` accrues interest on ONE market then
/// writes the supply position. The cleanest read on a single mutation's cost.
#[test]
fn budget_withdraw_one_asset_no_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_budget_enabled()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    // Non-zero time delta so `compound_interest` actually runs in the withdraw.
    t.advance_time(86_400);

    let mut b = t.env.cost_estimate().budget();
    b.reset_default();
    t.withdraw(ALICE, "USDC", 1_000.0);
    dump(
        &t.env,
        "withdraw 1 asset, NO debt (1 accrual + 1 supply write)",
    );
}

/// 1 supply + 1 borrow. `withdraw` accrues BOTH markets and runs the health
/// check. Isolates the cost added by a second-asset accrual + valuation vs the
/// no-debt case above.
#[test]
fn budget_withdraw_with_debt_hf_check() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
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

/// Mirrors testnet account 3 (5 collateral positions + 1 debt that overlaps a
/// collateral). `withdraw` runs the SAME double-pass valuation
/// (`require_within_ltv` → `require_healthy_account`, withdraw.rs:76-77) that
/// `strategy_finalize` runs — so this measures, with no router/oracle-reset
/// noise, the cost the repay-on-account-3 budget failure is dominated by.
/// Compare against the fused-valuation variant to read the savings.
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

    // Setup is not what this test measures, and the 5-market builder plus six
    // ops sits within a hair of the host's cumulative shadow budget (testutils
    // diagnostics + auth observation, never reset between invocations). Lift
    // limits for setup; `reset_default` below re-arms enforcement for the
    // measured withdraw.
    t.env.cost_estimate().budget().reset_unlimited();

    // 5 collateral positions on one account.
    t.supply(ALICE, "USDC", 100_000.0);
    let a = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, a, "USDT", 100_000.0);
    t.supply_to(ALICE, a, "ETH", 50.0);
    t.supply_to(ALICE, a, "WBTC", 2.0);
    t.supply_to(ALICE, a, "XLM", 1_000_000.0);
    // 1 debt position (small, so the post-withdraw HF stays healthy).
    t.borrow(ALICE, "XLM", 50_000.0);
    t.advance_time(86_400);

    // Capture only the withdraw's cost. We lift ALL limits (incl. the shadow
    // budget) rather than `reset_default`, then zero the tracker: the testutils
    // auth observation (`get_authenticated_authorizations`, "metering: free,
    // testutils") XDR-serializes the recorded auth-invocation tree under the
    // SHADOW budget — a harness-only cost absent on the real network, where
    // auth is verified by signature, not recorded and re-serialized. With 5
    // collateral positions that observation alone trips `reset_default`'s
    // shadow ceiling even though the real per-tx CPU for this withdraw is well
    // within budget (verified on testnet at 10 supply + 10 borrow). Limits are
    // unlimited here so the dump reflects the real op cost without that
    // harness-only shadow overhead aborting the run.
    let mut b = t.env.cost_estimate().budget();
    b.reset_unlimited();
    b.reset_tracker();
    t.withdraw(ALICE, "USDC", 1_000.0);
    dump(
        &t.env,
        "withdraw, 5 collateral + 1 debt (double-pass LTV+HF over 5)",
    );
}

/// Bare `supply` baseline — minimal mutation (1 accrual + 1 position write, no
/// valuation). Lower bound on a single mutating op.
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

/// `borrow` baseline — accrual + HF valuation + borrow write.
#[test]
fn budget_borrow_baseline() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_budget_enabled()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.advance_time(86_400);

    let mut b = t.env.cost_estimate().budget();
    b.reset_default();
    t.borrow(ALICE, "ETH", 0.1);
    dump(&t.env, "borrow (2 accruals + HF valuation + borrow write)");
}

/// Full `swap_collateral` strategy: withdraw collateral → swap (mock
/// aggregator) → deposit new collateral → finalize. Two position mutations
/// plus the strategy scaffolding. The swap VENUE cost is the native mock here
/// (cheap) — the real venue/hop cost is measured on the router suite — so this
/// isolates the CONTROLLER-side strategy cost.
#[test]
fn budget_swap_collateral_full() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_budget_enabled()
        .build();

    // The strategy scaffolding (withdraw + swap + deposit + finalize over two
    // markets, with a funded router) records a deep auth-invocation tree. The
    // testutils auth observation XDR-serializes that tree under the host's
    // cumulative SHADOW budget — a harness-only cost absent on the real network,
    // where auth is verified by signature rather than recorded and re-serialized
    // — which sits at `reset_default`'s shadow ceiling and trips it
    // nondeterministically. Lift limits for setup and measure the real op cost
    // via reset_unlimited + reset_tracker, matching
    // `budget_withdraw_5_collateral_double_pass`.
    t.env.cost_estimate().budget().reset_unlimited();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.fund_router("ETH", 10.0);
    t.advance_time(86_400);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 200_000_000_000, 10_0000000);

    let mut b = t.env.cost_estimate().budget();
    b.reset_unlimited();
    b.reset_tracker();
    t.swap_collateral(ALICE, "USDC", 20_000.0, "ETH", &steps);
    dump(
        &t.env,
        "swap_collateral USDC->ETH (withdraw + swap + deposit + finalize)",
    );
}
