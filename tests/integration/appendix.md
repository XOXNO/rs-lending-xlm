# Memory and resource budgets — where to get them

This file carries no budget numbers. The `meta` test binary in `test-harness`
prints them; read them from a test run.

`make integration-appendix` creates this file only when it is missing; it never
overwrites it, runs no test and captures no output. `full_e2e.sh`,
`flash_position.sh`, `blend.sh` and `strategies.sh` copy it into
`runs/<RUN_TS>/appendix.md`, and `report.md` ends with that copy.

Per-call CPU and memory totals, with CPU split by host cost type:

```bash
cargo test -p test-harness --test meta budget_breakdown -- --nocapture
```

Ledger footprint sizes against the mainnet limits:

```bash
cargo test -p test-harness --test meta footprints_fit_mainnet_limits -- --nocapture
```

Sources: `tests/test-harness/tests/meta/budget_breakdown.rs` and
`tests/test-harness/tests/meta/footprint_test.rs`.
