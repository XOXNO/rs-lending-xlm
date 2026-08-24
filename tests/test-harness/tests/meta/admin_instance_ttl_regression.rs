//! Pins instance-TTL renewal on the owner-gated admin surface.
//!
//! Every `ControllerAdmin` entrypoint bumps the controller's instance TTL
//! through exactly one mechanism: the `renew_then!` wrapper in
//! `contracts/controller/src/lib.rs`. The admin bodies in `governance.rs` and
//! `markets.rs` used to repeat that bump themselves; those duplicate calls were
//! removed, which leaves `renew_then!` a single point of failure for the whole
//! surface. Nothing else in the suite reads the *instance* TTL — the existing
//! TTL regressions all cover per-account persistent keys — so without this test,
//! dropping `renew_then!` from an admin entrypoint would pass CI silently and
//! surface only as an archived contract on a quiet network.

use controller::constants::TTL_THRESHOLD_INSTANCE;
use soroban_sdk::testutils::storage::Instance as _;
use test_harness::LendingTest;

/// Seconds of ledger time that drops a freshly bumped instance entry below
/// `TTL_THRESHOLD_INSTANCE` without letting it reach zero. `extend_ttl` is
/// threshold-gated, so a smaller jump is a legitimate no-op and would make the
/// assertion below vacuous.
const AGE_SECS: u64 = 176 * 24 * 60 * 60;

fn instance_ttl(t: &LendingTest) -> u32 {
    t.env
        .as_contract(&t.controller, || t.env.storage().instance().get_ttl())
}

fn assert_renews(name: &str, call: impl Fn(&LendingTest)) {
    // A fresh harness per case: aging is cumulative, so sharing one env would
    // let an earlier case's bump mask a later case's missing one.
    let t = LendingTest::new().standard_two_asset().build();
    t.advance_time_no_refresh(AGE_SECS);

    let aged = instance_ttl(&t);
    assert!(
        aged < TTL_THRESHOLD_INSTANCE,
        "{name}: harness must age past the renewal threshold or the assertion \
         below is vacuous: aged={aged}, threshold={TTL_THRESHOLD_INSTANCE}"
    );

    call(&t);

    // Not an equality check against TTL_BUMP_INSTANCE: `get_ttl` excludes the
    // current ledger and the harness caps `max_entry_ttl` at the bump target,
    // so the renewed value lands one short. The invariant that matters is that
    // the call pulled the entry back out of the renewal regime — a missing
    // `renew_then!` leaves it at `aged`, well below the threshold.
    let renewed = instance_ttl(&t);
    assert!(
        renewed > TTL_THRESHOLD_INSTANCE,
        "{name} must renew the controller instance TTL: aged={aged}, \
         after={renewed}, threshold={TTL_THRESHOLD_INSTANCE}; check that its \
         entrypoint still goes through renew_then!"
    );
}

#[test]
fn admin_entrypoints_renew_the_controller_instance_ttl() {
    assert_renews("pause", |t| t.pause());
    assert_renews("set_position_limits", |t| t.set_position_limits(4, 4));
    assert_renews("set_accumulator", |t| t.set_accumulator(&t.admin()));
}
