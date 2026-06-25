use super::*;
use common::constants::RAY;

#[test]
fn debt_close_amount_uses_pool_full_close_ceiling() {
    let env = Env::default();
    let position = DebtPosition {
        scaled_amount: Ray::from(RAY + RAY * 4 / 10),
    };

    assert_eq!(position.scaled_amount.mul(&env, Ray::ONE).to_asset(0), 1);
    assert_eq!(debt_close_amount(&env, &position, Ray::ONE, 0), 2);
}
