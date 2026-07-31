
use crate::context::LendingTest;
use test_harness::hub_asset;

const ACCOUNTING_TOLERANCE_UNITS: i128 = 4;

pub fn assert_user_health(t: &LendingTest, user: &str, min_hf: f64) {

    let Some(hf_raw) = t.try_health_factor_raw(user) else {
        return;
    };
    let hf = test_harness::wad_to_f64(hf_raw);
    assert!(
        hf + 1e-9 >= min_hf && hf > 0.0,
        "health factor {} < floor {} for {}",
        hf,
        min_hf,
        user
    );
}

pub fn assert_pool_accounting(t: &LendingTest, assets: &[&str]) {
    for asset in assets {
        let key = hub_asset(t.resolve_asset(asset));
        let pool = t.pool_client(asset);
        let cash = pool.get_reserves(&key);
        let supplied = pool.get_supplied_amount(&key);
        let borrowed = pool.get_borrowed_amount(&key);
        let revenue = pool.get_revenue(&key);

        assert!(cash >= 0, "{} cash is negative: {}", asset, cash);
        assert!(supplied >= 0, "{} supply is negative: {}", asset, supplied);
        assert!(borrowed >= 0, "{} debt is negative: {}", asset, borrowed);
        assert!(revenue >= 0, "{} revenue is negative: {}", asset, revenue);
        assert!(
            revenue <= supplied + ACCOUNTING_TOLERANCE_UNITS,
            "{} revenue exceeds supply: revenue={} supplied={}",
            asset,
            revenue,
            supplied
        );
        assert!(
            cash + borrowed + ACCOUNTING_TOLERANCE_UNITS >= supplied,
            "{} pool insolvent: cash={} borrowed={} supplied={}",
            asset,
            cash,
            borrowed,
            supplied
        );
    }
}

pub fn assert_flash_guard_cleared(t: &LendingTest) {
    t.env.as_contract(&t.controller, || {
        assert!(
            !controller::test_support::is_flash_loan_ongoing(&t.env),
            "flash-loan guard remained set after operation"
        );
    });
}

#[derive(Clone, Debug)]
pub struct StateSnapshot {

    pub health_raw: Option<i128>,
    pub token_raw: Vec<i128>,
    pub pool_state: Vec<PoolStateSnapshot>,
    pub supply_raw: Vec<i128>,
    pub borrow_raw: Vec<i128>,
    pub active_accounts: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolStateSnapshot {
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    last_timestamp: u64,
    cash: i128,
}

impl From<common::types::PoolStateRaw> for PoolStateSnapshot {
    fn from(state: common::types::PoolStateRaw) -> Self {
        Self {
            supplied: state.supplied,
            borrowed: state.borrowed,
            revenue: state.revenue,
            borrow_index: state.borrow_index,
            supply_index: state.supply_index,
            last_timestamp: state.last_timestamp,
            cash: state.cash,
        }
    }
}

pub fn snapshot(t: &LendingTest, user: &str, assets: &[&str]) -> StateSnapshot {
    StateSnapshot {
        health_raw: t.try_health_factor_raw(user),
        token_raw: assets
            .iter()
            .map(|a| t.token_balance_raw(user, a))
            .collect(),
        pool_state: assets
            .iter()
            .map(|asset| {
                let key = hub_asset(t.resolve_asset(asset));
                t.pool_client(asset).get_sync_data(&key).state.into()
            })
            .collect(),
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
    assert_eq!(
        before.pool_state, after.pool_state,
        "pool state drifted on failed op"
    );
    assert_eq!(
        before.supply_raw, after.supply_raw,
        "user supply drifted on failed op"
    );
    assert_eq!(
        before.borrow_raw, after.borrow_raw,
        "user debt drifted on failed op"
    );
    assert_eq!(
        before.active_accounts, after.active_accounts,
        "active account count drifted on failed op"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::build_wide_context;
    use test_harness::ALICE;

    #[test]
    fn stale_oracle_snapshot_and_health_do_not_panic() {
        let assets = ["USDC", "XLM", "ETH"];
        let mut t = build_wide_context();
        t.supply(ALICE, "USDC", 50_000.0);
        t.borrow(ALICE, "XLM", 10_000.0);

        assert!(t.try_health_factor_raw(ALICE).is_some());

        t.advance_time_no_refresh(30 * 24 * 60 * 60);

        assert!(
            t.try_health_factor_raw(ALICE).is_none(),
            "expected fail-closed health on a stale feed"
        );

        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(std::boxed::Box::new(|_| {}));
        let eager_trapped =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.health_factor_raw(ALICE)))
                .is_err();
        std::panic::set_hook(prev_hook);
        assert!(eager_trapped, "eager health read must trap on a stale feed");

        let snap = snapshot(&t, ALICE, &assets);
        assert!(
            snap.health_raw.is_none(),
            "stale health must snapshot as None"
        );
        assert_user_health(&t, ALICE, 1.0);

        let snap_again = snapshot(&t, ALICE, &assets);
        assert_state_preserved_on_failure(&snap, &snap_again);
    }
}
