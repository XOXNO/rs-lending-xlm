#![no_main]
//! Strategy entrypoints: multiply, swap debt/collateral, repay-with-collateral.
//! Asserts HF floor, reserve non-negativity, router allowance cleanup, and rollback.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use soroban_sdk::token;
use stellar_fuzz::{
    arb_amount, assert_global_invariants, assert_state_preserved_on_failure, build_wide_context,
    snapshot, LendingTest, ALICE, HF_WAD_FLOOR,
};

use common::types::{PositionMode, StrategySwap};

const ASSETS: [&str; 3] = ["USDC", "ETH", "XLM"];
const MAX_OPS: usize = 6;

#[derive(Arbitrary, Debug)]
enum Op {
    /// Create a new leveraged account. `collateral_idx`/`debt_idx` pick
    /// asset pair from ASSETS; amount is the flash-loaned debt size.
    Multiply {
        collateral_idx: u8,
        debt_idx: u8,
        amount: u32,
        mode_bits: u8,
    },
    /// Replace existing debt with a new token. Requires a prior debt
    /// position — when absent, the try-call fails and the on-failure
    /// snapshot check kicks in.
    SwapDebt {
        existing_idx: u8,
        new_idx: u8,
        amount: u32,
    },
    /// Rotate collateral from `current_idx` to `new_idx`.
    SwapCollateral {
        current_idx: u8,
        new_idx: u8,
        amount: u32,
    },
    /// Repay debt by swapping collateral; `close_position` toggles the
    /// full-close flag (requires exact debt match).
    RepayWithCollateral {
        collateral_idx: u8,
        debt_idx: u8,
        amount: u32,
        close_position: bool,
    },
    /// Time advance + keeper sync. Lets accrual move the HF boundary.
    AdvanceAndSync { hours: u16 },
}

#[derive(Arbitrary, Debug)]
struct Input {
    ops: Vec<Op>,
}

fn pick_asset(idx: u8) -> &'static str {
    ASSETS[(idx as usize) % ASSETS.len()]
}

fn pick_mode(bits: u8) -> PositionMode {
    match bits % 3 {
        0 => PositionMode::Multiply,
        1 => PositionMode::Long,
        _ => PositionMode::Short,
    }
}

/// Build a bytes-only `StrategySwap` with a permissive output
/// (just 1 token-unit). The mock aggregator delivers exactly that, so a
/// small value keeps the op from
/// exhausting the router funding and lets many ops chain in one sequence.
/// Strategy correctness is asserted via HF floor + reserve checks, not via
/// matching a specific swap rate.
fn build_steps(t: &LendingTest, token_in: &str, token_out: &str) -> StrategySwap {
    test_harness::mock_swap_payload_xdr(
        &t.env,
        t.resolve_asset(token_in),
        t.resolve_asset(token_out),
        1,
    )
}

/// Pre-fund the aggregator with a generous amount of every asset so the
/// mock swap calls have tokens to transfer out. 10 million tokens per
/// asset is far more than any fuzz sequence can consume in `MAX_OPS`.
fn fund_aggregator(t: &LendingTest) {
    for a in ASSETS {
        t.fund_router(a, 10_000_000.0);
    }
}

/// Seed ALICE with a baseline supply position so the first few ops have
/// something to mutate (swap_collateral requires an existing supply;
/// swap_debt requires an existing borrow). Without this bootstrap,
/// ~80% of fuzzed ops would short-circuit on "no position".
fn bootstrap(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 10.0);
    // A small borrow primes swap_debt without leaving ALICE underwater.
    t.borrow(ALICE, "XLM", 1_000.0);
}

/// Asserts that no residual router allowance remains after a successful
/// strategy operation.
fn assert_router_allowance_zeroed(t: &LendingTest) {
    for a in ASSETS {
        let addr = t.resolve_asset(a);
        let tok = token::Client::new(&t.env, &addr);
        let allowance = tok.allowance(&t.controller, &t.aggregator);
        assert_eq!(
            allowance, 0,
            "router allowance for {} left at {} after strategy op",
            a, allowance
        );
    }
}

fuzz_target!(|inp: Input| {
    let ops: Vec<_> = inp.ops.into_iter().take(MAX_OPS).collect();
    if ops.is_empty() {
        return;
    }

    let mut t = build_wide_context();
    fund_aggregator(&t);
    bootstrap(&mut t);
    assert_global_invariants(&t, ALICE, &ASSETS, 0.0);

    for op in ops {
        let before = snapshot(&t, ALICE, &ASSETS);

        let (ok, min_hf) = dispatch(&mut t, &op);

        if ok {
            assert_global_invariants(&t, ALICE, &ASSETS, min_hf);
            for a in ASSETS {
                assert!(t.pool_reserves(a) >= 0.0, "{} reserves negative", a);
            }
            // AdvanceAndSync does not touch the router.
            if !matches!(op, Op::AdvanceAndSync { .. }) {
                assert_router_allowance_zeroed(&t);
            }
        } else {
            let after = snapshot(&t, ALICE, &ASSETS);
            assert_state_preserved_on_failure(&before, &after);
        }
    }
});

fn dispatch(t: &mut LendingTest, op: &Op) -> (bool, f64) {
    match *op {
        Op::Multiply {
            collateral_idx,
            debt_idx,
            amount,
            mode_bits,
        } => {
            let collateral = pick_asset(collateral_idx);
            let debt = pick_asset(debt_idx);
            if collateral == debt {
                // Same-asset multiply is degenerate; skip without mutation.
                return (true, 0.0);
            }
            let amt = arb_amount(amount, 0.1, 1_000.0);
            let steps = build_steps(t, debt, collateral);
            let ok = t
                .try_multiply(ALICE, collateral, amt, debt, pick_mode(mode_bits), &steps)
                .is_ok();
            (ok, HF_WAD_FLOOR)
        }
        Op::SwapDebt {
            existing_idx,
            new_idx,
            amount,
        } => {
            let existing = pick_asset(existing_idx);
            let new = pick_asset(new_idx);
            if existing == new {
                return (true, 0.0);
            }
            let amt = arb_amount(amount, 1.0, 5_000.0);
            let steps = build_steps(t, existing, new);
            let ok = t.try_swap_debt(ALICE, existing, amt, new, &steps).is_ok();
            (ok, HF_WAD_FLOOR)
        }
        Op::SwapCollateral {
            current_idx,
            new_idx,
            amount,
        } => {
            let current = pick_asset(current_idx);
            let new = pick_asset(new_idx);
            if current == new {
                return (true, 0.0);
            }
            let amt = arb_amount(amount, 1.0, 5_000.0);
            let steps = build_steps(t, current, new);
            let ok = t
                .try_swap_collateral(ALICE, current, amt, new, &steps)
                .is_ok();
            (ok, HF_WAD_FLOOR)
        }
        Op::RepayWithCollateral {
            collateral_idx,
            debt_idx,
            amount,
            close_position,
        } => {
            let collateral = pick_asset(collateral_idx);
            let debt = pick_asset(debt_idx);
            if collateral == debt {
                return (true, 0.0);
            }
            let amt = arb_amount(amount, 1.0, 5_000.0);
            let steps = build_steps(t, collateral, debt);
            let ok = t
                .try_repay_debt_with_collateral(
                    ALICE,
                    collateral,
                    amt,
                    debt,
                    &steps,
                    close_position,
                )
                .is_ok();
            // Repay is risk-decreasing: HF can be anything positive.
            (ok, 0.0)
        }
        Op::AdvanceAndSync { hours } => {
            let secs = (hours as u64) * 3_600;
            if secs > 0 {
                t.advance_and_sync(secs);
            }
            (true, 0.0)
        }
    }
}
