# Governance timelock file consolidation

**Date:** 2026-06-25
**Scope:** `contracts/governance/src/` — entry-point file/block organization only.
**Type:** Behavior-preserving structural refactor. No ABI, encoding, or logic change.

## Goal

Reduce file fragmentation and the number of `#[contractimpl] impl Governance`
blocks in the governance crate. The timelock operation lifecycle is currently
split across three files; collapse it into one cohesive file.

## Motivation

The timelock state machine is one concept fragmented three ways:

- `forward.rs` — `propose` (schedule a queued operation)
- `timelock.rs` — `execute`, `cancel`, lifecycle views, arg resolvers
- `self_timelock.rs` — `execute_self` (inline self-dispatch)

`self_timelock.rs` is 45 lines / one entry point; `forward.rs` carries three
separate `#[contractimpl]` blocks. Navigating the propose -> execute -> cancel
flow means hopping across files for no structural reason.

## Constraints honored

1. **`#[cfg]`-gated methods need their own `#[contractimpl]` block.** Under
   feature unification, a gated contract method merged into a production block
   triggers E0425. The two test-gated blocks (`set_controller` in `deploy.rs`,
   `execute_immediate` in `forward.rs`) must stay in dedicated blocks.
2. **Byte-/ABI-neutral.** Merging blocks does not shrink wasm (dispatch glue is
   per-method) and does not change the entrypoint set or the `AdminOperation`
   XDR encoding. The win is navigation/cohesion only.
3. **No Certora coupling** in any of the three target files (verified via grep).
4. **Single external reference** to the moved code: `forward::resolve_market_oracle`
   is called only by the `resolve_market_oracle_config` view in `timelock.rs`;
   after the merge it becomes a private same-file helper, dropping a `pub(crate)`
   surface.

## Target structure

`forward.rs` and `self_timelock.rs` are deleted. Their contents fold into
`timelock.rs` (name retained: the timelock is the dominant concern; pause/unpause
are a clearly-commented exception; renaming the largest file + its test mounts +
`lib.rs` is churn for marginal accuracy).

```
timelock.rs
  //! module doc: timelock op surface + immediate emergency controls;
  //! retains the self-reentry inline-dispatch rationale

  // module-level helpers (unioned):
  DelayTier, operation_delay, require_nonzero_delay, validate_delay_update,
  apply_update_delay, authorize_executor, require_operation_not_expired,
  controller_client, begin_proposal, begin_self_execute,
  resolve_market_oracle            // now private (single caller)

  #[contractimpl] impl Governance {
      propose                       // schedule          (from forward.rs)
      execute                       // run controller op
      execute_self                  // inline self-dispatch + self-reentry comment
                                    //                   (from self_timelock.rs)
      cancel
      pause / unpause               // #[only_owner], immediate, NOT timelocked,
                                    // commented section (from forward.rs)
      get_min_delay, get_operation_state, get_operation_ledger, hash_operation,
      resolve_market_oracle_config, resolve_oracle_tolerance   // views
  }

  #[cfg(any(test, feature = "testing"))]
  #[contractimpl] impl Governance {
      execute_immediate             // test bypass forwarder, own block
  }

  #[cfg(test)] #[path = "../tests/timelock.rs"]      mod tests;
  #[cfg(test)] #[path = "../tests/self_timelock.rs"] mod self_timelock_tests;
```

**Block count:** 8 -> 6. Mixing `#[only_owner]` (pause/unpause) with non-owner
methods in one production block is already proven safe in `deploy.rs:15`
(`deploy_controller` is `#[only_owner]`, `controller` is not, same block).

## Files touched

- **Delete:** `contracts/governance/src/forward.rs`, `contracts/governance/src/self_timelock.rs`
- **Rewrite:** `contracts/governance/src/timelock.rs` (absorbs the two deleted files)
- **Edit:** `contracts/governance/src/lib.rs` — remove `mod forward;` and `mod self_timelock;`
- **Edit:** `contracts/governance/src/access.rs:4` — doc comment pointer (`self_timelock.rs` -> `timelock.rs`)
- Test files `tests/timelock.rs` and `tests/self_timelock.rs` are unchanged in
  content; `self_timelock.rs` is re-mounted as `mod self_timelock_tests`.

## Mechanical details

- Code moves verbatim (cut/paste), not rewritten.
- Imports are unioned. `ORACLE_ROLE` stays fully-qualified (`crate::access::ORACLE_ROLE`)
  inside the test-gated block to avoid an unused-import warning when `testing`
  is off (matches current `forward.rs`).
- The self-reentry rationale comment from `self_timelock.rs` is preserved on
  `execute_self` / `begin_self_execute`.

## Verification bar

- `cargo check -p governance --tests`
- `cargo clippy -p governance --all-targets -- -D warnings`
- `cargo test -p governance` — includes `tests/op.rs` (XDR byte-parity gate),
  `tests/flows.rs`, `tests/timelock.rs`, `tests/self_timelock.rs`,
  `tests/access.rs`
- `cargo test --workspace` (no `--all-features`) — confirms no downstream breakage
- Build governance wasm; confirm size stays within cap (expect +/-0; per-method
  codegen unchanged)

## Non-goals / explicit non-claims

- Not byte-identical wasm: the contractspec entry *ordering* may shift (it is a
  set; semantically irrelevant). Verification is size + tests, not a wasm hash match.
- No change to `op.rs`, `validate/`, `deploy.rs`, `access.rs` logic, `storage.rs`,
  `events.rs`, or `constants.rs`.
- No renames beyond the deleted files; `timelock.rs` keeps its name.
