#![no_main]
//! Central contract-level libFuzzer target for the lending protocol.
//!
//! Replaces the retired `flow_multi_op`, `flow_supply_borrow_liquidate`,
//! `flow_flash_loan`, and `flow_oracle_tolerance` targets. One op-sequence
//! fuzzer drives every user-facing entrypoint (supply/borrow/withdraw/repay/
//! liquidate/flash-loan) plus the keeper paths (advance+sync, clean bad debt)
//! and the revenue path (claim) across three markets and two borrowers.
//!
//! ### Invariants (per op, aligned with INVARIANTS.md)
//!
//! - **§9 Health factor floor**: after risk-increasing ops (Borrow, Withdraw)
//!   on a successful call, HF(user) ≥ 1.0. Risk-decreasing ops (Supply,
//!   Repay, Liquidate, FlashLoan, AdvanceAndSync, ClaimRevenue) require only
//!   HF > 0.
//! - **§13 Reserve availability**: `pool_reserves(asset) ≥ 0` for every asset
//!   after every op, success or failure.
//! - **Cache atomicity**: when a `try_*` call returns `Err`, pool reserves and
//!   *both* users' raw supply/borrow balances must be unchanged (±1 ulp drift
//!   from index-rescale rounding). This subsumes the retired
//!   `flow_cache_atomicity` proptest's core guarantee.
//! - **Flash-loan atomicity**: a `FlashLoan` with a receiver that fails to
//!   repay must return `Err`; a good receiver may succeed or fail (budget /
//!   auth may reject) but must never silently mutate state.
//!
//! ### Why no `catch_unwind`
//!
//! libfuzzer-sys's panic hook calls `std::process::abort()` before the
//! unwind reaches any user-level `catch_unwind` — see the Phase 2 note in
//! `rates_and_index.rs`. All contract calls go through `try_*` variants so
//! contract errors surface as `Result::Err`; only *host* panics (bugs)
//! abort, which is exactly what we want libFuzzer to flag as a crash.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{
    arb_amount, assert_global_invariants, assert_state_preserved_on_failure, build_wide_context,
    snapshot, ALICE, BOB, HF_WAD_FLOOR, LIQUIDATOR,
};

const ASSETS: [&str; 3] = ["USDC", "ETH", "XLM"];
const USERS: [&str; 2] = [ALICE, BOB];
const MAX_OPS: usize = 16;

#[derive(Arbitrary, Debug)]
enum Op {
    /// Supply from one of the two fuzz users.
    Supply { user: u8, asset: u8, amount: u32 },
    /// Borrow from one of the two fuzz users.
    Borrow { user: u8, asset: u8, amount: u32 },
    /// Withdraw; amount is an upper bound — protocol rejects over-withdraw.
    Withdraw { user: u8, asset: u8, amount: u32 },
    /// Repay own debt.
    Repay { user: u8, asset: u8, amount: u32 },
    /// Liquidator tries to seize debtor's collateral via a debt-asset payment.
    /// `frac` = 0..=255 → 0..1.0 of the debtor's outstanding debt.
    Liquidate { debtor: u8, asset: u8, frac: u8 },
    /// Flash loan through the test harness's good/bad receiver.
    FlashLoan {
        user: u8,
        asset: u8,
        amount: u32,
        bad: bool,
    },
    /// Push a spot/TWAP deviation onto an asset's mock oracle. `bps` is
    /// clamped to ±5000 bps (±50%); beyond that the protocol's tolerance
    /// bands will reject subsequent ops, which is the interesting behaviour.
    OracleJitter {
        asset: u8,
        deviation_bps: u16,
        direction_up: bool,
    },
    /// Advance ledger time + run keeper sync on every asset.
    AdvanceAndSync { hours: u16 },
    /// Admin claims accrued revenue on an asset.
    ClaimRevenue { asset: u8 },
    /// Keeper attempts to clean bad debt on a debtor.
    CleanBadDebt { debtor: u8 },
}

#[derive(Arbitrary, Debug)]
struct Input {
    ops: Vec<Op>,
}

fn pick_asset(idx: u8) -> &'static str {
    ASSETS[(idx as usize) % ASSETS.len()]
}

fn pick_user(idx: u8) -> &'static str {
    USERS[(idx as usize) % USERS.len()]
}

/// Bootstrap a non-trivial starting state so the first few ops can reach
/// interesting branches. Without seeding, ~90% of `Borrow` ops would fail
/// with "no collateral" and libFuzzer would explore only `Supply` combinations.
fn bootstrap(t: &mut stellar_fuzz::LendingTest) {
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "ETH", 10.0);
    // Liquidator needs a base position so `get_or_create_user` has a funded
    // account when the first `Liquidate` op fires.
    t.supply(LIQUIDATOR, "USDC", 10_000.0);
}

fuzz_target!(|inp: Input| {
    let ops: Vec<_> = inp.ops.into_iter().take(MAX_OPS).collect();
    if ops.is_empty() {
        return;
    }

    let mut t = build_wide_context();
    bootstrap(&mut t);
    // Fresh state after bootstrap: reserves ≥ 0, every user HF > 0.
    for u in USERS {
        assert_global_invariants(&t, u, &ASSETS, 0.0);
    }

    for op in ops {
        // Pre-snapshot captures *both* users; either may be touched by the op
        // (liquidation / clean-bad-debt affect the debtor, not the caller).
        let before_alice = snapshot(&t, ALICE, &ASSETS);
        let before_bob = snapshot(&t, BOB, &ASSETS);

        let (ok, hf_users) = dispatch(&mut t, &op);

        if ok {
            // On success, every op-affected user must still satisfy their
            // minimum-HF floor (1.0 for risk-increasing, 0.0 otherwise) and
            // reserves must be non-negative.
            for (user, min_hf) in hf_users {
                assert_global_invariants(&t, user, &ASSETS, min_hf);
            }
            // Reserves always ≥ 0 across all assets, regardless of which
            // user's HF the op tracks.
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
            // Failed op must not have silently mutated state.
            let after_alice = snapshot(&t, ALICE, &ASSETS);
            let after_bob = snapshot(&t, BOB, &ASSETS);
            assert_state_preserved_on_failure(&before_alice, &after_alice);
            assert_state_preserved_on_failure(&before_bob, &after_bob);
        }
    }
});

/// Execute a single op against the lending context.
/// Returns `(ok, users_to_check)` where `users_to_check` is a list of
/// `(user_name, min_hf_floor)` pairs whose HF the caller should verify.
///
/// `AdvanceAndSync` is modelled as always-OK because time advance cannot fail
/// at the protocol layer; any error would be a test-harness bug.
fn dispatch(t: &mut stellar_fuzz::LendingTest, op: &Op) -> (bool, Vec<(&'static str, f64)>) {
    match *op {
        Op::Supply {
            user,
            asset,
            amount,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = arb_amount(amount, 1.0, 50_000.0);
            let ok = t.try_supply(u, a, amt).is_ok();
            (ok, vec![(u, 0.0)])
        }
        Op::Borrow {
            user,
            asset,
            amount,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = arb_amount(amount, 1.0, 10_000.0);
            let ok = t.try_borrow(u, a, amt).is_ok();
            (ok, vec![(u, HF_WAD_FLOOR)])
        }
        Op::Withdraw {
            user,
            asset,
            amount,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = arb_amount(amount, 1.0, 50_000.0);
            let ok = t.try_withdraw(u, a, amt).is_ok();
            (ok, vec![(u, HF_WAD_FLOOR)])
        }
        Op::Repay {
            user,
            asset,
            amount,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = arb_amount(amount, 1.0, 10_000.0);
            let ok = t.try_repay(u, a, amt).is_ok();
            (ok, vec![(u, 0.0)])
        }
        Op::Liquidate {
            debtor,
            asset,
            frac,
        } => {
            let d = pick_user(debtor);
            let a = pick_asset(asset);
            // frac: 0..=255 maps to 0..1.0 of debtor's outstanding debt.
            let debt = t.borrow_balance(d, a);
            if debt <= 0.0 {
                // Nothing to liquidate — treat as a benign no-op; no
                // invariant needed because state is unchanged.
                return (true, vec![]);
            }
            let amt = debt * (frac as f64 / 255.0);
            if amt < 1e-9 {
                return (true, vec![]);
            }
            let ok = t.try_liquidate(LIQUIDATOR, d, a, amt).is_ok();
            // HF of debtor can remain < 1 if heavily underwater (see README
            // "What we do NOT assert"). Only the >0 invariant survives.
            (ok, vec![(d, 0.0)])
        }
        Op::FlashLoan {
            user,
            asset,
            amount,
            bad,
        } => {
            let u = pick_user(user);
            let a = pick_asset(asset);
            let amt = arb_amount(amount, 1.0, 20_000.0);
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
            // Flash loan is atomic — caller position net-neutral, so no
            // HF tightening beyond >0 on success.
            (res.is_ok(), vec![(u, 0.0)])
        }
        Op::OracleJitter {
            asset,
            deviation_bps,
            direction_up,
        } => {
            let a = pick_asset(asset);
            let dev = (deviation_bps.min(5_000)) as i128;
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
            // Oracle jitter does not invoke the protocol; no HF check needed.
            (true, vec![])
        }
        Op::AdvanceAndSync { hours } => {
            let secs = (hours as u64) * 3_600;
            if secs > 0 {
                t.advance_and_sync(secs);
            }
            // Accrual can drop HF below 1 for any user; only >0 holds.
            (true, vec![(ALICE, 0.0), (BOB, 0.0)])
        }
        Op::ClaimRevenue { asset } => {
            let a = pick_asset(asset);
            let ok = t.try_claim_revenue(a).is_ok();
            (ok, vec![])
        }
        Op::CleanBadDebt { debtor } => {
            let d = pick_user(debtor);
            let account_id = match t.try_resolve_account_id(d) {
                Ok(id) => id,
                Err(_) => return (true, vec![]), // no account, no state change
            };
            let ok = t.try_clean_bad_debt_by_id(account_id).is_ok();
            (ok, vec![(d, 0.0)])
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
