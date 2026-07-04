#![no_main]
//! Protocol user/keeper flows: supply, borrow, liquidate, flash loan, oracle jitter.
//! Asserts HF floors, non-negative reserves, and rollback on failed `try_*` calls.

use libfuzzer_sys::fuzz_target;
use stellar_fuzz::{
    amount_for_value, assert_global_invariants, assert_state_preserved_on_failure, asset_price_usd,
    build_wide_context, fraction, snapshot, ALICE, BOB, HF_WAD_FLOOR, LIQUIDATOR,
};

const ASSETS: [&str; 3] = ["USDC", "ETH", "XLM"];
const USERS: [&str; 2] = [ALICE, BOB];
const BAD_DEBTOR: &str = "bad_debtor";
const MAX_OPS: usize = 16;
const OP_WIDTH: usize = 5;

#[derive(Clone, Copy, Debug)]
enum Op {
    Supply {
        user: u8,
        asset: u8,
        size: u8,
        mode: u8,
    },
    Borrow {
        user: u8,
        asset: u8,
        size: u8,
        mode: u8,
    },
    Withdraw {
        user: u8,
        asset: u8,
        size: u8,
        mode: u8,
    },
    Repay {
        user: u8,
        asset: u8,
        size: u8,
        mode: u8,
    },
    Liquidate {
        debtor: u8,
        asset: u8,
        frac: u8,
        mode: u8,
    },
    FlashLoan {
        user: u8,
        asset: u8,
        size: u8,
        bad: bool,
    },
    OracleJitter {
        asset: u8,
        deviation: u8,
        direction_up: bool,
    },
    AdvanceAndSync {
        hours: u8,
    },
    AddRewards {
        asset: u8,
        size: u8,
    },
    ClaimRevenue {
        asset: u8,
    },
    CleanBadDebt,
}

impl Op {
    fn decode(chunk: [u8; OP_WIDTH]) -> Self {
        match chunk[0] % 11 {
            0 => Op::Supply {
                user: chunk[1],
                asset: chunk[2],
                size: chunk[3],
                mode: chunk[4],
            },
            1 => Op::Borrow {
                user: chunk[1],
                asset: chunk[2],
                size: chunk[3],
                mode: chunk[4],
            },
            2 => Op::Withdraw {
                user: chunk[1],
                asset: chunk[2],
                size: chunk[3],
                mode: chunk[4],
            },
            3 => Op::Repay {
                user: chunk[1],
                asset: chunk[2],
                size: chunk[3],
                mode: chunk[4],
            },
            4 => Op::Liquidate {
                debtor: chunk[1],
                asset: chunk[2],
                frac: chunk[3],
                mode: chunk[4],
            },
            5 => Op::FlashLoan {
                user: chunk[1],
                asset: chunk[2],
                size: chunk[3],
                bad: chunk[4] & 1 == 1,
            },
            6 => Op::OracleJitter {
                asset: chunk[2],
                deviation: chunk[3],
                direction_up: chunk[4] & 1 == 1,
            },
            7 => Op::AdvanceAndSync { hours: chunk[3] },
            8 => Op::AddRewards {
                asset: chunk[2],
                size: chunk[3],
            },
            9 => Op::ClaimRevenue { asset: chunk[2] },
            _ => Op::CleanBadDebt,
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

fn pick_user(idx: u8) -> &'static str {
    USERS[(idx as usize) % USERS.len()]
}

fn debt_asset(t: &stellar_fuzz::LendingTest, user: &str, preferred: &'static str) -> &'static str {
    if t.borrow_balance(user, preferred) > 0.0 {
        return preferred;
    }
    ASSETS
        .iter()
        .copied()
        .find(|asset| t.borrow_balance(user, asset) > 0.0)
        .unwrap_or(preferred)
}

fn supply_asset(
    t: &stellar_fuzz::LendingTest,
    user: &str,
    preferred: &'static str,
) -> &'static str {
    if t.supply_balance(user, preferred) > 0.0 {
        return preferred;
    }
    ASSETS
        .iter()
        .copied()
        .find(|asset| t.supply_balance(user, asset) > 0.0)
        .unwrap_or(preferred)
}

fn supply_amount(asset: &str, size: u8) -> f64 {
    amount_for_value(size, asset, 25.0, 50_000.0)
}

fn borrow_amount(asset: &str, size: u8) -> f64 {
    amount_for_value(size, asset, 10.0, 25_000.0)
}

fn wallet_supply_amount(
    t: &stellar_fuzz::LendingTest,
    user: &str,
    asset: &str,
    size: u8,
    mode: u8,
) -> f64 {
    let wallet = t.token_balance(user, asset);
    if wallet <= 0.0 {
        return supply_amount(asset, size);
    }

    let mix = match mode % 4 {
        0 => 0.10 + 0.35 * fraction(size),
        1 => 0.50 + 0.35 * fraction(size),
        2 => 0.90,
        _ => fraction(size),
    };
    (wallet * mix.clamp(0.01, 0.99))
        .max(f64::EPSILON)
        .min(wallet)
}

fn borrow_capacity_usd(t: &stellar_fuzz::LendingTest, user: &str) -> f64 {
    let debt = t.total_debt(user);
    let hf = t.health_factor(user);
    if debt > 0.0 && hf.is_finite() && hf > 0.0 {
        (debt * hf - debt).max(0.0)
    } else {
        t.total_collateral(user) * 0.75
    }
}

fn borrow_amount_for_state(
    t: &stellar_fuzz::LendingTest,
    user: &str,
    asset: &str,
    size: u8,
    mode: u8,
) -> f64 {
    let price = asset_price_usd(asset);
    let capacity_usd = borrow_capacity_usd(t, user);
    if capacity_usd <= 0.0 {
        return borrow_amount(asset, size);
    }

    let target_usd = match mode % 4 {
        0 => capacity_usd * (0.15 + 0.65 * fraction(size)),
        1 => capacity_usd * (0.92 + 0.06 * fraction(size)),
        2 => capacity_usd + 10.0,
        _ => (capacity_usd * 0.05).max(10.0),
    };
    (target_usd.max(10.0) / price).max(f64::EPSILON)
}

fn position_fraction_amount(balance: f64, raw: u8) -> f64 {
    if balance <= 0.0 {
        0.0
    } else {
        (balance * fraction(raw)).max(f64::EPSILON)
    }
}

fn set_oracle_price(t: &mut stellar_fuzz::LendingTest, asset: &str, bps: i128) {
    let price = default_spot(asset) * bps / 10_000;
    t.set_price(asset, price);
}

fn stress_debtor_prices(t: &mut stellar_fuzz::LendingTest, debtor: &str) {
    if debtor == ALICE {
        set_oracle_price(t, "USDC", 5_000);
        set_oracle_price(t, "XLM", 15_000);
    } else {
        set_oracle_price(t, "ETH", 5_000);
        set_oracle_price(t, "USDC", 15_000);
    }
}

fn bootstrap(t: &mut stellar_fuzz::LendingTest) {
    t.supply(ALICE, "USDC", 50_000.0);
    t.borrow(ALICE, "XLM", 100_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 5_000.0);
    t.supply(LIQUIDATOR, "USDC", 50_000.0);
}

fuzz_target!(|data: &[u8]| {
    let ops = decode_ops(data);
    if ops.is_empty() {
        return;
    }

    let mut t = build_wide_context();
    bootstrap(&mut t);
    for u in USERS {
        assert_global_invariants(&t, u, &ASSETS, HF_WAD_FLOOR);
    }

    for op in ops {
        // Price stress mutates oracle state; apply it before snapshotting so a
        // failed liquidation is compared against the post-stress baseline.
        if let Op::Liquidate { debtor, mode, .. } = op {
            if mode & 1 == 1 {
                stress_debtor_prices(&mut t, pick_user(debtor));
            }
        }

        let before_alice = snapshot(&t, ALICE, &ASSETS);
        let before_bob = snapshot(&t, BOB, &ASSETS);

        let (ok, hf_users) = dispatch(&mut t, &op);

        if ok {
            for (user, min_hf) in hf_users {
                assert_global_invariants(&t, user, &ASSETS, min_hf);
            }
            for a in ASSETS {
                let r = t.pool_reserves(a);
                assert!(
                    r >= 0.0,
                    "{} reserves went negative after {:?}: {}",
                    a,
                    op,
                    r
                );
            }
        } else {
            let after_alice = snapshot(&t, ALICE, &ASSETS);
            let after_bob = snapshot(&t, BOB, &ASSETS);
            assert_state_preserved_on_failure(&before_alice, &after_alice);
            assert_state_preserved_on_failure(&before_bob, &after_bob);
        }
    }
});

fn dispatch(t: &mut stellar_fuzz::LendingTest, op: &Op) -> (bool, Vec<(&'static str, f64)>) {
    match *op {
        Op::Supply {
            user,
            asset,
            size,
            mode,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = wallet_supply_amount(t, u, a, size, mode);
            let ok = t.try_supply(u, a, amt).is_ok();
            (ok, vec![(u, 0.0)])
        }
        Op::Borrow {
            user,
            asset,
            size,
            mode,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = borrow_amount_for_state(t, u, a, size, mode);
            let ok = t.try_borrow(u, a, amt).is_ok();
            (ok, vec![(u, HF_WAD_FLOOR)])
        }
        Op::Withdraw {
            user,
            asset,
            size,
            mode,
        } => {
            let u = pick_user(user);
            let a = supply_asset(t, u, pick_asset(asset));
            let supplied = t.supply_balance(u, a);
            let hf = t.health_factor(u);
            let safe_fraction = if supplied <= 0.0 || !hf.is_finite() || hf <= 1.0 {
                0.0
            } else {
                (1.0 - (1.0 / hf)).clamp(0.0, 1.0)
            };
            let amt = match mode % 4 {
                0 => {
                    let from_position = position_fraction_amount(supplied, size);
                    if from_position > 0.0 {
                        from_position
                    } else {
                        supply_amount(a, size)
                    }
                }
                1 => (supplied * safe_fraction * 0.95).max(f64::EPSILON),
                2 => (supplied * safe_fraction * 1.05).max(f64::EPSILON),
                _ => supplied.max(f64::EPSILON),
            };
            let ok = t.try_withdraw(u, a, amt).is_ok();
            (ok, vec![(u, HF_WAD_FLOOR)])
        }
        Op::Repay {
            user,
            asset,
            size,
            mode,
        } => {
            let u = pick_user(user);
            let a = debt_asset(t, u, pick_asset(asset));
            let debt = t.borrow_balance(u, a);
            let mut amt = match mode % 4 {
                0 => position_fraction_amount(debt, size),
                1 => debt,
                2 => debt * 1.25,
                _ => (debt * 0.05).max(f64::EPSILON),
            };
            if amt == 0.0 {
                amt = borrow_amount(a, size);
            }
            let ok = t.try_repay(u, a, amt).is_ok();
            (ok, vec![(u, 0.0)])
        }
        Op::Liquidate {
            debtor,
            asset,
            frac,
            mode: _,
        } => {
            let d = pick_user(debtor);
            let a = debt_asset(t, d, pick_asset(asset));
            let debt = t.borrow_balance(d, a);
            if debt <= 0.0 {
                return (true, vec![]);
            }
            let amt = position_fraction_amount(debt, frac);
            let ok = t.try_liquidate(LIQUIDATOR, d, a, amt).is_ok();
            (ok, vec![(d, 0.0)])
        }
        Op::FlashLoan {
            user,
            asset,
            size,
            bad,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = amount_for_value(size, a, 10.0, 25_000.0);
            let receiver = if bad {
                t.deploy_bad_flash_loan_receiver()
            } else {
                t.deploy_flash_loan_receiver()
            };
            let res = t.try_flash_loan(u, a, amt, &receiver);
            // Adversarial receiver must never succeed.
            if bad {
                assert!(res.is_err(), "bad flash-loan receiver returned Ok");
            }
            (res.is_ok(), vec![(u, 0.0)])
        }
        Op::OracleJitter {
            asset,
            deviation,
            direction_up,
        } => {
            let a = pick_asset(asset);
            // Keep spot/TWAP divergence inside the configured tolerance band:
            // an out-of-band pair fails closed (`UnsafePriceNotAllowed`) and
            // would poison every subsequent oracle read in the run. The oracle
            // gates the primary/anchor RATIO against a reciprocal-symmetric band
            // [10000^2/upper, upper] with upper = 10000 + tolerance_bps, so the
            // downside half-width (~tol*10000/upper) is smaller than tol and a
            // naive `% tol` multiplier still trips the guard near the edge (e.g.
            // dev=480 → ratio 10504 > upper 10500). Bound the deviation to that
            // safe half-width, minus 2 bps for the contract's half-up rounding.
            let tol = i128::from(test_harness::presets::DEFAULT_TOLERANCE.tolerance_bps);
            let upper = 10_000 + tol;
            let lower = 10_000 * 10_000 / upper; // floor of the contract's half-up lower bound
            let max_dev = (10_000 - lower).min(tol).saturating_sub(2).max(0);
            let dev = if max_dev == 0 {
                0
            } else {
                ((deviation as i128) * 20) % (max_dev + 1)
            };
            let mult = if direction_up {
                10_000 + dev
            } else {
                (10_000 - dev).max(1)
            };
            let spot = default_spot(a);
            let twap = spot * mult / 10_000;
            let reflector = t.mock_reflector_client();
            let addr = t.resolve_asset(a);
            reflector.set_price(&addr, &spot);
            reflector.set_twap_price(&addr, &twap);
            (true, vec![])
        }
        Op::AdvanceAndSync { hours } => {
            let secs = ((hours as u64 % 72) + 1) * 3_600;
            if secs > 0 {
                t.advance_and_sync(secs);
            }
            (true, vec![(ALICE, 0.0), (BOB, 0.0)])
        }
        Op::AddRewards { asset, size } => {
            let a = pick_asset(asset);
            let amt = amount_for_value(size, a, 1.0, 10_000.0);
            let ok = t.try_add_rewards(a, amt).is_ok();
            (ok, vec![])
        }
        Op::ClaimRevenue { asset } => {
            let a = pick_asset(asset);
            let ok = t.try_claim_revenue(a).is_ok();
            (ok, vec![])
        }
        Op::CleanBadDebt => {
            t.set_price("USDC", default_spot("USDC"));
            t.set_price("ETH", default_spot("ETH"));
            t.supply(BAD_DEBTOR, "USDC", 20.0);
            t.borrow(BAD_DEBTOR, "ETH", 0.006);
            t.set_price("USDC", test_harness::usd_cents(5));
            let account_id = t.resolve_account_id(BAD_DEBTOR);
            assert!(
                t.try_clean_bad_debt_by_id(account_id).is_ok(),
                "seeded bad-debt account must be cleanable"
            );
            t.set_price("USDC", default_spot("USDC"));
            (true, vec![])
        }
    }
}

/// Default spot price (1e18-scaled) for each fuzz asset, matching the
/// `build_wide_context()` presets.
fn default_spot(asset: &str) -> i128 {
    match asset {
        "USDC" => 10_i128.pow(18),
        "ETH" => 2000 * 10_i128.pow(18),
        "XLM" => 10_i128.pow(17), // $0.10
        _ => 10_i128.pow(18),
    }
}
