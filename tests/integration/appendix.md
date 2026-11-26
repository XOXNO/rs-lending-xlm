# Where to get memory and resource budgets

This file has no budget numbers. The `meta` test binary in `test-harness`
prints them. Read them from a test run.

`make integration-appendix` creates this file only when it is missing. It never
overwrites the file, runs no test, and captures no output. `full_e2e.sh`,
`flash_position.sh`, `blend.sh`, and `strategies.sh` copy the file into
`runs/<RUN_TS>/appendix.md`, and `report.md` ends with that copy.

The first test prints per-call CPU and memory totals, with CPU split by host cost type:

```bash
cargo test -p test-harness --test meta budget_breakdown -- --nocapture
```

The second test compares ledger footprint sizes against the mainnet limits:

```bash
cargo test -p test-harness --test meta footprints_fit_mainnet_limits -- --nocapture
```

The sources are `tests/test-harness/tests/meta/budget_breakdown.rs` and
`tests/test-harness/tests/meta/footprint_test.rs`.
