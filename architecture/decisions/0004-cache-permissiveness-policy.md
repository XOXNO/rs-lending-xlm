# ADR 0004: Oracle Policy By Flow

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

A single global oracle reaction is wrong: strict checks on every flow trap users
during an outage; permissive pricing on every flow lets risk grow under bad
prices.

There is no market-status enum. An asset is price-active when token-rooted
`AssetOracle(asset)` exists.

Price resolution (ADR 0003) is always fail-closed. The remaining choice is which
flows call resolution, and what happens when it reverts.

## Decision

Encode oracle exposure in the **call graph**. There is no runtime `OraclePolicy`
value on the transaction cache.

- Flows that cannot increase risk and do not need a live valuation do not call
  the oracle. `repay` never prices. Debt-free `withdraw` skips
  `require_post_pool_risk_gates` (no-op when the borrow map is empty).  
- Flows that depend on solvency call `calculate_account_risk_totals`, which
  resolves every priced leg through the ADR 0003 fail-closed path: `borrow`,
  debt-bearing `withdraw`, `liquidate`, `clean_bad_debt`, and strategies that
  leave debt.  
- Price-resolving views use the same strict path. A successful view does not
  prove a later mutation will succeed.  

No flow relaxes validation once a price is read. “Permissiveness by omission”
means not reading a price.

The controller transaction `Cache` holds only returned `PriceFeedRaw` map
entries after a bulk `prices()` call. Resolution memos (`resolved_prices` /
cycle `resolving` stack / multi-feed bulk cache) live inside the
price-aggregator `ResolutionContext`.

### Flow assignment

| Flow | Reads price? |
|------|----------------|
| `supply` | No on the main path (entry only requires `AssetOracle` config present). An LT **decrease** refresh while the account has debt may price via risk totals. |
| `borrow` | Yes — fail-closed |
| `withdraw` without debt | No |
| `withdraw` with debt | Yes — fail-closed |
| `repay` | No |
| `liquidate` / `clean_bad_debt` | Yes — fail-closed |
| strategy flows | Yes if post-state still has debt (`strategy_finalize` → risk gates) |
| `flash_loan` | No account valuation; market must be active |
| price-resolving views | Yes — fail-closed |

## Alternatives considered

- Strict pricing on debt-free exits — forces reverts on flows with no risk
  outcome.  
- Runtime `OraclePolicy` on the cache — redundant once resolution is always
  fail-closed; the call graph already encodes “read or don’t.”  
- Per-asset switch — risk effect is defined by the flow, not the asset.  
- Caller-specified policy — oracle safety is not user input.  

## Consequences

**Positive:** risk paths fail closed by construction; repay and debt-free exits
remain available during an oracle outage without a weaker price path; new
entrypoints are reviewed by whether they call the price-aggregator
(`prices` / `price` / soft `prices_status` for views) or risk totals.

**Costs:** every new entrypoint needs that review; views stay conservative.

## References

- `contracts/price-aggregator/src/{price,compose,prefetch,context}.rs`  
- `contracts/controller/src/external/price_aggregator.rs`  
- `contracts/controller/src/risk/{validation,totals}.rs`  
- `contracts/controller/src/positions/repay.rs`  
- `contracts/controller/src/context/mod.rs`  
- [ADR 0003](./0003-oracle-dual-source-with-tolerance-bands.md)  
- [INVARIANTS.md](../INVARIANTS.md) §4.3  
