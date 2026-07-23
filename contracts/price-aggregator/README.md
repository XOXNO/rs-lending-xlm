# Price Aggregator

Single oracle entry point. Token-rooted `AssetOracle` configs; hard
`price`/`prices` fail closed; soft `*_status` diagnostics flag without
reverting.

| | |
| --- | --- |
| Owner | Governance (`#[only_owner]`) |
| Consumers | Controller (and views) |
| Interface | `interfaces/price-aggregator` |

## Surface

| Call | Role |
| --- | --- |
| `price` / `prices` | Fail-closed USD feeds |
| `price_status` / `prices_status` | Soft diagnostics (flags, no stale/deviation revert) |
| `oracle_config` | Read token-rooted config |
| `set_oracle_config` | Register/replace config (owner) |
| `set_sanity_band` / `set_tolerance` | Live band updates (owner) |

## Related

| Doc | Topic |
| --- | --- |
| Crate rustdoc (`//!`) | Semantics |
| [`architecture/INVARIANTS.md`](../../architecture/INVARIANTS.md) §4.3 | Price resolution |
| [ADR 0003](../../architecture/decisions/0003-oracle-dual-source-with-tolerance-bands.md) | Dual-source + bands |
| `contracts/xoxno-oracle` | Self-hosted multi-signer feed |
