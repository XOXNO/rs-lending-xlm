# Test doubles

Test-only contracts for the harness, unit tests and live testnet coverage.
None of them is a deployable protocol contract. None of them gates its caller.

| Directory | Purpose |
| --- | --- |
| [`flash-loan-receiver`](flash-loan-receiver/README.md) | Receives `execute_flash_loan` callbacks and runs configurable repay, re-entry and failure plans |
| [`flash-position-receiver`](flash-position-receiver/README.md) | Receives `execute_flash_position` callbacks with a plan set through `set_plan` |
| [`mock-oracle`](mock-oracle/src/lib.rs) | Reflector SEP-40 price feed with settable prices and timestamps |
| [`mock-redstone`](mock-redstone/src/lib.rs) | RedStone-format feed and Chainlink `latest_round` source with settable prices and rounds |
| [`production-fixture`](production-fixture/src/lib.rs) | Decimal token and configurable Aquarius pool read surface; not a swap venue |
| [`script-runner`](script-runner/src/lib.rs) | Issues a sequence of controller calls from one contract frame, so a test can check what one attacker contract can do in one transaction |
