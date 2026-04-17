//! Helpers shared by contract-level libFuzzer targets.
//!
//! Keep this module tiny: a minimal builder, input-decoding helpers, and
//! invariant assertion helpers. Each fuzz target should focus on its
//! scenario, not on boilerplate.

pub use test_harness::{eth_preset, usdc_preset, xlm_preset, LendingTest, ALICE, BOB, LIQUIDATOR};

/// Build a minimal two-market (USDC + ETH) lending context for fuzz targets.
pub fn build_min_context() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build()
}

/// Build a three-market (USDC + ETH + XLM) lending context. Used by
/// `flow_e2e` so op sequences can cross-pollinate across assets (USD stable,
/// volatile, native) without per-asset market setup noise in each target.
pub fn build_wide_context() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(xlm_preset())
        .build()
}

/// Decode a `u32` fuzz byte into a bounded `f64` amount in `[lo, hi]`.
///
/// Uses modulo against the inclusive range width so libFuzzer's mutation
/// engine gets smooth coverage across the domain. `lo` must be < `hi`.
#[inline]
pub fn arb_amount(raw: u32, lo: f64, hi: f64) -> f64 {
    debug_assert!(hi > lo);
    let span = (hi - lo).max(1.0);
    lo + (raw as f64 % span)
}

/// Required health-factor floor after a risk-increasing operation.
/// Matches the proptest harness's post-borrow / post-withdraw invariant.
pub const HF_WAD_FLOOR: f64 = 1.0;

/// Assert the cheap global invariants that should hold after any successful
/// user operation.
///
/// `min_hf` is the required health-factor floor for the *caller's* context:
/// pass `0.0` after risk-decreasing ops (supply, repay) where HF can be
/// anything positive, and `HF_WAD_FLOOR` (1.0) after risk-increasing ops
/// (borrow, withdraw) where the protocol must reject anything that would
/// drop HF below 1.0.
///
/// Invariants enforced:
///   - `health_factor(user) > min_hf` (strict, with a tiny epsilon for HF==1 edge)
///   - `pool_reserves(asset) >= 0` for every asset the target touches
pub fn assert_global_invariants(t: &LendingTest, user: &str, assets: &[&str], min_hf: f64) {
    let hf = t.health_factor(user);
    // Allow 1 ULP of float slack at the boundary (HF==WAD computed as 0.99999...).
    assert!(
        hf + 1e-9 >= min_hf && hf > 0.0,
        "health factor {} < required floor {} for {}",
        hf,
        min_hf,
        user
    );

    for a in assets {
        let r = t.pool_reserves(a);
        assert!(r >= 0.0, "{} reserves went negative: {}", a, r);
    }
}

/// Snapshot of fuzz-relevant pool + user state, for pre/post comparison
/// around fallible operations. Use with `assert_state_preserved_on_failure`
/// to detect silent state drift when a `try_*` call reverts (the property
/// the retired `flow_cache_atomicity` target used to assert).
#[derive(Clone, Debug)]
pub struct StateSnapshot {
    pub reserves: Vec<f64>,
    pub supply_raw: Vec<i128>,
    pub borrow_raw: Vec<i128>,
}

pub fn snapshot(t: &LendingTest, user: &str, assets: &[&str]) -> StateSnapshot {
    StateSnapshot {
        reserves: assets.iter().map(|a| t.pool_reserves(a)).collect(),
        supply_raw: assets
            .iter()
            .map(|a| t.supply_balance_raw(user, a))
            .collect(),
        borrow_raw: assets
            .iter()
            .map(|a| t.borrow_balance_raw(user, a))
            .collect(),
    }
}

/// Assert that a failed (`Err`) operation did not mutate reserves or the
/// user's raw supply/borrow balances. Tolerance is chosen to absorb the
/// ~1-ulp drift that index-rescale rounding can introduce when the cache
/// Drop fires with no actual write; anything larger is a bug.
pub fn assert_state_preserved_on_failure(before: &StateSnapshot, after: &StateSnapshot) {
    assert_eq!(before.reserves.len(), after.reserves.len());
    for (i, (b, a)) in before.reserves.iter().zip(&after.reserves).enumerate() {
        assert!(
            (b - a).abs() < 1e-4,
            "asset[{}] reserves drifted on failed op: {} -> {}",
            i,
            b,
            a
        );
    }
    for (i, (b, a)) in before.supply_raw.iter().zip(&after.supply_raw).enumerate() {
        assert!(
            (a - b).abs() <= 1,
            "asset[{}] user supply drifted on failed op: {} -> {}",
            i,
            b,
            a
        );
    }
    for (i, (b, a)) in before.borrow_raw.iter().zip(&after.borrow_raw).enumerate() {
        assert!(
            (a - b).abs() <= 1,
            "asset[{}] user borrow drifted on failed op: {} -> {}",
            i,
            b,
            a
        );
    }
}
