#![no_main]
//! Strategy entrypoints: multiply, swap debt/collateral, repay-with-collateral.
//! Asserts HF floor, reserve non-negativity, router allowance cleanup, and rollback.

use libfuzzer_sys::fuzz_target;
use soroban_sdk::token;
use stellar_fuzz::{
    amount_for_value, assert_flash_guard_cleared, assert_pool_accounting,
    assert_state_preserved_on_failure, assert_user_health, asset_price_usd, build_wide_context,
    fraction, snapshot, LendingTest, ALICE, HF_WAD_FLOOR,
};

use common::types::{PositionMode, StrategySwap};

const ASSETS: [&str; 3] = ["USDC", "ETH", "XLM"];
const MAX_OPS: usize = 6;
const OP_WIDTH: usize = 5;

#[derive(Clone, Copy, Debug)]
enum Op {
    Multiply {
        collateral_idx: u8,
        debt_idx: u8,
        size: u8,
        mode_bits: u8,
    },
    SwapDebt {
        existing_idx: u8,
        new_idx: u8,
        size: u8,
    },
    SwapCollateral {
        current_idx: u8,
        new_idx: u8,
        size: u8,
    },
    RepayWithCollateral {
        collateral_idx: u8,
        debt_idx: u8,
        size: u8,
        close_position: bool,
        allow_same_asset: bool,
    },
    AdvanceAndSync {
        hours: u8,
    },
}

impl Op {
    fn decode(chunk: [u8; OP_WIDTH]) -> Self {
        match chunk[0] % 5 {
            0 => Op::Multiply {
                collateral_idx: chunk[1],
                debt_idx: chunk[2],
                size: chunk[3],
                mode_bits: chunk[4],
            },
            1 => Op::SwapDebt {
                existing_idx: chunk[1],
                new_idx: chunk[2],
                size: chunk[3],
            },
            2 => Op::SwapCollateral {
                current_idx: chunk[1],
                new_idx: chunk[2],
                size: chunk[3],
            },
            3 => Op::RepayWithCollateral {
                collateral_idx: chunk[1],
                debt_idx: chunk[2],
                size: chunk[3],
                close_position: chunk[4] & 1 == 1,
                allow_same_asset: chunk[4] & 2 == 2,
            },
            _ => Op::AdvanceAndSync { hours: chunk[3] },
        }
    }
}

fn decode_ops(data: &[u8]) -> Vec<Op> {
    let mut ops = Vec::new();
    for idx in 0..MAX_OPS {
        let start = idx * OP_WIDTH;
        if start >= data.len() {
            break;
        }

        let mut chunk = [0u8; OP_WIDTH];
        let end = (start + OP_WIDTH).min(data.len());
        chunk[..end - start].copy_from_slice(&data[start..end]);
        ops.push(Op::decode(chunk));
    }
    ops
}

fn pick_asset(idx: u8) -> &'static str {
    ASSETS[(idx as usize) % ASSETS.len()]
}

fn next_asset(asset: &str) -> &'static str {
    match asset {
        "USDC" => "ETH",
        "ETH" => "XLM",
        _ => "USDC",
    }
}

fn pick_mode(bits: u8) -> PositionMode {
    match bits % 3 {
        0 => PositionMode::Multiply,
        1 => PositionMode::Long,
        _ => PositionMode::Short,
    }
}

fn swap_min_out_raw(t: &LendingTest, token_in: &str, token_out: &str, amount_in: f64) -> i128 {
    if token_in == token_out {
        return 0;
    }
    let out_amount = amount_in * asset_price_usd(token_in) / asset_price_usd(token_out) * 0.97;
    let decimals = t.resolve_market(token_out).decimals;
    test_harness::f64_to_i128(out_amount.max(f64::EPSILON), decimals).max(1)
}

fn build_steps(t: &LendingTest, token_in: &str, token_out: &str, amount_in: f64) -> StrategySwap {
    test_harness::mock_swap_payload_xdr(
        &t.env,
        t.resolve_asset(token_in),
        t.resolve_asset(token_out),
        swap_min_out_raw(t, token_in, token_out, amount_in),
    )
}

fn fund_aggregator(t: &LendingTest) {
    for a in ASSETS {
        t.fund_router(a, 10_000_000.0);
    }
}

fn bootstrap(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "XLM", 1_000.0);
}

/// The controller must never hold residual tokens: strategy flows pull from
/// the pool, route through the aggregator (which pulls via
/// `authorize_as_current_contract` + `transfer`, never token allowances), and
/// deposit/refund everything before returning. Any balance left on the
/// controller is stuck value.
fn assert_controller_residual_zero(t: &LendingTest) {
    for a in ASSETS {
        let addr = t.resolve_asset(a);
        let tok = token::Client::new(&t.env, &addr);
        let residual = tok.balance(&t.controller);
        assert_eq!(
            residual, 0,
            "controller holds residual {} of {} after strategy op",
            residual, a
        );
    }
}

fn debt_asset(t: &LendingTest, preferred: &'static str) -> &'static str {
    if t.borrow_balance(ALICE, preferred) > 0.0 {
        return preferred;
    }
    ASSETS
        .iter()
        .copied()
        .find(|asset| t.borrow_balance(ALICE, asset) > 0.0)
        .unwrap_or(preferred)
}

fn supply_asset(t: &LendingTest, preferred: &'static str) -> &'static str {
    if t.supply_balance(ALICE, preferred) > 0.0 {
        return preferred;
    }
    ASSETS
        .iter()
        .copied()
        .find(|asset| t.supply_balance(ALICE, asset) > 0.0)
        .unwrap_or(preferred)
}

fn position_amount(balance: f64, raw: u8, asset: &str, min_usd: f64, max_usd: f64) -> f64 {
    if balance > 0.0 {
        (balance * fraction(raw)).max(f64::EPSILON)
    } else {
        amount_for_value(raw, asset, min_usd, max_usd)
    }
}

#[derive(Clone, Copy)]
enum HealthCheck {
    User(&'static str, f64),
    Account(u64, f64),
}

fn assert_health_check(t: &LendingTest, check: HealthCheck) {
    match check {
        HealthCheck::User(user, min_hf) => assert_user_health(t, user, min_hf),
        HealthCheck::Account(account_id, min_hf) => {
            let hf = t.health_factor_for(ALICE, account_id);
            assert!(
                hf + 1e-9 >= min_hf && hf > 0.0,
                "account {} health factor {} < floor {}",
                account_id,
                hf,
                min_hf
            );
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let ops = decode_ops(data);
    if ops.is_empty() {
        return;
    }

    let mut t = build_wide_context();
    fund_aggregator(&t);
    bootstrap(&mut t);
    assert_user_health(&t, ALICE, 0.0);
    assert_pool_accounting(&t, &ASSETS);

    for op in ops {
        let before = snapshot(&t, ALICE, &ASSETS);
        let accounts_before = t.get_active_accounts(ALICE).len();

        let (ok, checks) = dispatch(&mut t, &op);

        if ok {
            for check in checks {
                assert_health_check(&t, check);
            }
            for a in ASSETS {
                assert!(t.pool_reserves(a) >= 0.0, "{} reserves negative", a);
            }
            // AdvanceAndSync does not touch the router.
            if !matches!(op, Op::AdvanceAndSync { .. }) {
                assert_controller_residual_zero(&t);
            }
        } else {
            let after = snapshot(&t, ALICE, &ASSETS);
            assert_state_preserved_on_failure(&before, &after);
            assert_eq!(
                accounts_before,
                t.get_active_accounts(ALICE).len(),
                "failed strategy op leaked or removed an account"
            );
            assert_controller_residual_zero(&t);
        }
        assert_pool_accounting(&t, &ASSETS);
        assert_flash_guard_cleared(&t);
    }
});

fn dispatch(t: &mut LendingTest, op: &Op) -> (bool, Vec<HealthCheck>) {
    match *op {
        Op::Multiply {
            collateral_idx,
            debt_idx,
            size,
            mode_bits,
        } => {
            let collateral = pick_asset(collateral_idx);
            let mut debt = pick_asset(debt_idx);
            if collateral == debt {
                debt = next_asset(collateral);
            }
            let amt = amount_for_value(size, debt, 25.0, 1_000.0);
            let steps = build_steps(t, debt, collateral, amt);
            match t.try_multiply(ALICE, collateral, amt, debt, pick_mode(mode_bits), &steps) {
                Ok(account_id) => (
                    true,
                    vec![
                        HealthCheck::User(ALICE, 0.0),
                        HealthCheck::Account(account_id, HF_WAD_FLOOR),
                    ],
                ),
                Err(_) => (false, vec![]),
            }
        }
        Op::SwapDebt {
            existing_idx,
            new_idx,
            size,
        } => {
            let existing = debt_asset(t, pick_asset(existing_idx));
            let mut new = pick_asset(new_idx);
            if existing == new {
                new = next_asset(existing);
            }
            let current_debt = t.borrow_balance(ALICE, existing);
            let amt = position_amount(current_debt, size, existing, 10.0, 1_000.0);
            let steps = build_steps(t, existing, new, amt);
            let ok = t.try_swap_debt(ALICE, existing, amt, new, &steps).is_ok();
            (ok, vec![HealthCheck::User(ALICE, HF_WAD_FLOOR)])
        }
        Op::SwapCollateral {
            current_idx,
            new_idx,
            size,
        } => {
            let current = supply_asset(t, pick_asset(current_idx));
            let mut new = pick_asset(new_idx);
            if current == new {
                new = next_asset(current);
            }
            let current_supply = t.supply_balance(ALICE, current);
            let amt = position_amount(current_supply, size, current, 10.0, 1_000.0);
            let steps = build_steps(t, current, new, amt);
            let ok = t
                .try_swap_collateral(ALICE, current, amt, new, &steps)
                .is_ok();
            (ok, vec![HealthCheck::User(ALICE, HF_WAD_FLOOR)])
        }
        Op::RepayWithCollateral {
            collateral_idx,
            debt_idx,
            size,
            close_position,
            allow_same_asset,
        } => {
            let collateral = supply_asset(t, pick_asset(collateral_idx));
            let mut debt = debt_asset(t, pick_asset(debt_idx));
            if collateral == debt && !allow_same_asset {
                debt = next_asset(collateral);
            }
            let current_supply = t.supply_balance(ALICE, collateral);
            let amt = position_amount(current_supply, size, collateral, 10.0, 1_000.0);
            let steps = build_steps(t, collateral, debt, amt);
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
            (ok, vec![HealthCheck::User(ALICE, 0.0)])
        }
        Op::AdvanceAndSync { hours } => {
            let secs = ((hours as u64 % 72) + 1) * 3_600;
            t.advance_and_sync(secs);
            (true, vec![HealthCheck::User(ALICE, 0.0)])
        }
    }
}
