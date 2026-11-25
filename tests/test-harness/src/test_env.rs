use std::env::var_os;

use soroban_sdk::testutils::{EnvTestConfig, SnapshotSourceInput};
use soroban_sdk::Env;

/// Set this variable to `1` to write `test_snapshots/` JSON files again.
pub const TEST_SNAPSHOTS_VAR: &str = "HARNESS_TEST_SNAPSHOTS";

fn config() -> EnvTestConfig {
    EnvTestConfig {
        capture_snapshot_at_drop: var_os(TEST_SNAPSHOTS_VAR).is_some_and(|value| value == "1"),
    }
}

/// Creates a test `Env` that writes no ledger snapshot when it is dropped.
///
/// The SDK default serializes the whole ledger and auth tree of every test
/// `Env` to a JSON file, often several megabytes. No test reads those files.
/// Set `HARNESS_TEST_SNAPSHOTS=1` to write them for a debug session.
pub fn new_env() -> Env {
    Env::new_with_config(config())
}

/// Loads a ledger snapshot like `Env::from_ledger_snapshot`, with the snapshot
/// rule of [`new_env`].
pub fn env_from_ledger_snapshot(input: impl Into<SnapshotSourceInput>) -> Env {
    let mut env = Env::from_ledger_snapshot(input);
    env.set_config(config());
    env
}
