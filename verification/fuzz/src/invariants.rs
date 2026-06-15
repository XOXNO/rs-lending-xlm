//! Post-operation invariant checks shared by protocol fuzz targets.

use crate::context::LendingTest;

pub fn assert_global_invariants(t: &LendingTest, user: &str, assets: &[&str], min_hf: f64) {
    let hf = t.health_factor(user);
    assert!(
        hf + 1e-9 >= min_hf && hf > 0.0,
        "health factor {} < floor {} for {}",
        hf,
        min_hf,
        user
    );
    for a in assets {
        let r = t.pool_reserves(a);
        assert!(r >= 0.0, "{} reserves negative: {}", a, r);
    }
}

#[derive(Clone, Debug)]
pub struct StateSnapshot {
    pub health_raw: i128,
    pub token_raw: Vec<i128>,
    pub reserves: Vec<f64>,
    pub supply_raw: Vec<i128>,
    pub borrow_raw: Vec<i128>,
    pub active_accounts: usize,
}

pub fn snapshot(t: &LendingTest, user: &str, assets: &[&str]) -> StateSnapshot {
    StateSnapshot {
        health_raw: t.health_factor_raw(user),
        token_raw: assets
            .iter()
            .map(|a| t.token_balance_raw(user, a))
            .collect(),
        reserves: assets.iter().map(|a| t.pool_reserves(a)).collect(),
        supply_raw: assets
            .iter()
            .map(|a| t.supply_balance_raw(user, a))
            .collect(),
        borrow_raw: assets
            .iter()
            .map(|a| t.borrow_balance_raw(user, a))
            .collect(),
        active_accounts: t.get_active_accounts(user).len() as usize,
    }
}

pub fn assert_state_preserved_on_failure(before: &StateSnapshot, after: &StateSnapshot) {
    assert_eq!(
        before.health_raw, after.health_raw,
        "health factor drifted on failed op"
    );
    assert_eq!(before.token_raw.len(), after.token_raw.len());
    for (i, (b, a)) in before.token_raw.iter().zip(&after.token_raw).enumerate() {
        assert_eq!(b, a, "asset[{}] wallet balance drifted on failed op", i);
    }
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
    assert_eq!(
        before.active_accounts, after.active_accounts,
        "active account count drifted on failed op"
    );
}
