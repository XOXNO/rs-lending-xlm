# A080 — apply_exit no-op on missing usage row

- Agent: A080 (revalidated)
- Theme: T5
- Severity: info
- Status: **defended** (live issue **withdrawn**)
- Paths: `spoke_usage.rs` (`apply_exit`); merge callers via `apply_leg_usage`
- Defense: Exit with zero delta returns early. Exit with missing storage row
  returns without writing — intentional INV-HALT-03 / Certora carve-out for
  never-written or both-zero-pruned keys. First positive supply/borrow always
  creates the row via `apply_entry` → `persist`. Ordinary money merges update
  usage and positions together.
- Gap: **None on live production paths.** Persistent TTL archive does **not**
  turn a key into `None` (restore or `ENTRY_ARCHIVED`). The over-admission
  scenario requires an artificially absent key while positions remain (test /
  Certora plant). That is not reachable via first-entry skip, everyday merge
  math, or persistent archival.
- Impact: N/A for production. Planted harness/unit cases only show the
  intentional no-op + cap-from-zero behavior of the carve-out.
- Evidence: Revalidation `synthesis/RESIDUAL_REVALIDATION.md`; Stellar state
  archival (persistent restore); `merge_*` + `apply_leg_usage` lockstep;
  Certora `usage_exit_without_usage_row_is_a_noop` documents the carve-out.
- Opinion: **Not a valid live finding.** Keep the no-op pin. Do not prioritize
  A080 in A110. Optional plant tests remain useful as PIN of current semantics
  only.

---

## Revalidation notes

| Claim | Result |
|---|---|
| First supply forgets usage | **False** — `apply_entry` always buffers; persist writes |
| Persistent archive → missing row | **False** — archived persistent restores original value |
| Money actions desync usage vs positions | **False** — same-leg `apply_leg_usage` + finalize |
| Exit no-op is a bug | **False** — intentional tolerance; only bites if key already absent |

Prior medium/partial ranking in PRELIMINARY / A103 / FINAL is **superseded** by
this file and `RESIDUAL_REVALIDATION.md`.
