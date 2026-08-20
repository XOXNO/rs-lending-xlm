# 0007. Emergency powers only tighten protection

**Status:** Accepted

**Implemented by:** contracts/controller/src/config/asset.rs (`set_spoke_asset_flags`, `require_flag_ratchet`, `edit_asset_in_spoke`), contracts/governance/src/timelock/immediate.rs (`GUARDIAN_ROLE`), contracts/controller/src/governance.rs (`pause`, `unpause`).

## Decision

The guardian may immediately pause the protocol and set restrictive listing
flags. It cannot immediately unpause or clear a restriction. Reopening always
requires the timelock.

A full timelocked listing update can clear flags, so operators must restate the
intended halted state when they edit a listing.

## Guarantees

- A hot emergency key can stop risk but cannot silently restart it.
- Restoring service receives the governance delay and review window.
- Deployment and upgrade transitions end in a safe paused state.

## Auditor focus

Distinguish the immediate ratchet from the delayed full-rewrite path. Test
omitted flags and all upgrade, initialization, and recovery transitions.
