//! Contract-level property test: E-Mode vs Isolation mutual exclusivity.
//!
//! Invariants:
//!   - Creating an account with e_mode_category > 0 AND is_isolated=true panics.
//!   - An e-mode account always has `is_isolated` false.
//!   - An isolated account always has `e_mode_category_id` zero.

use common::types::{AccountMeta, ControllerKey, PositionMode};
use proptest::prelude::*;
use test_harness::{usdc_preset, xlm_preset, LendingTest, STABLECOIN_EMODE, ALICE};

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn prop_emode_isolation_xor_enforced(
        choose_emode in any::<bool>(),
        supply_amt in 100u64..50_000u64,
    ) {
        let mut t = LendingTest::new()
            .with_market({
                let mut p = usdc_preset();
                p.config.e_mode_enabled = true;
                p
            })
            .with_market({
                let mut p = xlm_preset();
                p.config.is_isolated_asset = true;
                p.config.isolation_borrow_enabled = true;
                p
            })
            .with_emode(1, STABLECOIN_EMODE)
            .with_emode_asset(1, "USDC", true, true)
            .build();

        if choose_emode {
            // Create a valid e-mode account.
            t.create_emode_account(ALICE, 1);
            t.supply(ALICE, "USDC", supply_amt as f64);

            let account_id = t.resolve_account_id(ALICE);
            let meta = t.env.as_contract(&t.controller, || {
                t.env.storage().persistent()
                    .get::<_, AccountMeta>(&ControllerKey::AccountMeta(account_id))
                    .unwrap()
            });
            prop_assert_eq!(meta.e_mode_category_id, 1);
            prop_assert!(!meta.is_isolated, "XOR broken: emode AND isolated");
        } else {
            // Invalid combo: must panic.
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                t.create_account_full(ALICE, 1, PositionMode::Normal, true);
            }));
            prop_assert!(
                caught.is_err(),
                "creating account with emode=1 AND isolated=true must panic"
            );
        }
    }
}
