use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn fresh_governance(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(
        crate::Governance,
        (admin, crate::constants::TIMELOCK_MIN_DELAY_LEDGERS),
    )
}

// `clear_recovery_op` must actually remove the Recovery-tier marker: after it
// runs, `is_recovery_op` reads false again so the operation id can no longer be
// treated as non-vetoable. A no-op body leaves the mark set.
#[test]
fn clear_recovery_op_removes_the_mark() {
    let env = Env::default();
    let id = fresh_governance(&env);
    env.as_contract(&id, || {
        let op = BytesN::from_array(&env, &[7u8; 32]);
        assert!(!is_recovery_op(&env, &op));
        mark_recovery_op(&env, &op);
        assert!(is_recovery_op(&env, &op));
        clear_recovery_op(&env, &op);
        assert!(
            !is_recovery_op(&env, &op),
            "clear_recovery_op must remove the recovery mark"
        );
    });
}
