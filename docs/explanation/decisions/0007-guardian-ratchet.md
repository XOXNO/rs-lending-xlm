# 0007. Emergency powers only tighten protection

**Status:** Accepted

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
