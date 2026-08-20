# Memory and resource budgets — where to get them

This file is a pointer, not a snapshot. It carries no budget numbers. The live
numbers are printed by the `meta` test binary in `test-harness`, so read them
from a test run rather than from this page.

`make integration-appendix` only creates this file when it is missing; it never
overwrites the checked-in text. It runs no test and captures no harness output.
`full_e2e.sh` copies whatever this file contains into `runs/<RUN_TS>/appendix.md`.

Per-call CPU, memory, and ledger-entry costs:

```bash
cargo test -p test-harness --test meta budget_breakdown -- --nocapture
```

Ledger footprint sizes against the mainnet limits:

```bash
cargo test -p test-harness --test meta footprints_fit_mainnet_limits -- --nocapture
```

Sources: `tests/test-harness/tests/meta/budget_breakdown.rs` and
`tests/test-harness/tests/meta/footprint_test.rs`.
