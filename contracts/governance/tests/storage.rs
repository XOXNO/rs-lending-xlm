use super::*;
use soroban_sdk::Env;

use crate::test_support::fresh_governance;

#[test]
fn clear_operation_sidecars_removes_recovery_mark() {
    let env = Env::default();
    let id = fresh_governance(&env);
    env.as_contract(&id, || {
        let op = BytesN::from_array(&env, &[7u8; 32]);
        assert!(!is_recovery_op(&env, &op));
        mark_recovery_op(&env, &op);
        assert!(is_recovery_op(&env, &op));
        clear_operation_sidecars(&env, &op);
        assert!(
            !is_recovery_op(&env, &op),
            "clear_operation_sidecars must remove the recovery mark"
        );
    });
}
