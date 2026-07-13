# ADR 0004: Oracle Policy By Flow

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-07-02
- Deciders: XOXNO Lending contract team

## Context

Oracle degradation should not have one global response. Strict behavior on every
flow can trap users during an oracle outage. Permissive behavior on every flow
can allow risk to increase under bad pricing.

The current protocol has no separate market-status enum. Price activation is
represented by the presence of token-rooted `AssetOracle(asset)` configuration.
Removing that entry disables price resolution for the asset.

Oracle price resolution itself (ADR 0003) is uniformly fail-closed: `token_price`
and the functions it calls never return a degraded price. There is no
strict/permissive switch inside price resolution. The remaining design question
is which flows call price resolution at all, and what a flow does when that
resolution reverts.

## Decision

Differentiate oracle exposure per flow structurally, at the call site, rather
than through a runtime policy value threaded through the transaction cache:

- A flow that cannot increase account or system risk, and does not need a live
  account valuation to decide its outcome, does not call the oracle. `repay`
  (`contracts/controller/src/positions/repay.rs`) never prices anything. A
  debt-free `withdraw` skips `require_post_pool_risk_gates`
  (`contracts/controller/src/risk/validation.rs`) entirely: that gate is a
  no-op when `account.borrow_positions.is_empty()`.
- A flow whose outcome depends on account solvency calls
  `calculate_account_risk_totals` (`contracts/controller/src/risk/totals.rs`),
  which resolves every priced position through the strict, fail-closed path
  defined in ADR 0003. This covers `borrow`, a debt-bearing `withdraw`,
  `liquidate`, `clean_bad_debt`, and any strategy leg that leaves the account
  with debt.
- Views that resolve a price (`resolve_market_oracle_config`, health-factor
  reads) use the same strict path as mutations and must not be read as proof
  that a subsequent mutation will succeed; a mutation re-evaluates state at its
  own ledger.

There is no flow-level override of oracle strictness: every price read either
resolves under the ADR 0003 rules or the call reverts. Permissiveness is
achieved only by a flow choosing not to read a price, never by relaxing how a
price is validated once read. This is sometimes called "permissiveness by
omission."

The transaction `Cache` (in `context/`) holds per-tx memos such as
`prices_cache` and a `resolving` stack (to detect quote/anchor cycles during
strict resolution), but no `OraclePolicy` value. The call-graph structure
itself encodes the policy.

## Flow Assignment

| Flow | Reads the oracle? |
| --- | --- |
| `supply` | No |
| `borrow` | Yes — fail-closed |
| `withdraw` without debt | No |
| `withdraw` with debt | Yes — fail-closed |
| `repay` | No |
| `liquidate` / `clean_bad_debt` | Yes — fail-closed |
| strategy flows | Yes, if the resulting account carries debt |
| price-resolving views | Yes — fail-closed |

## Alternatives Considered

- **Strict for every flow, including debt-free exits.** Rejected because it
  would force a price read, and a potential revert, onto flows that have no
  risk-relevant outcome to protect.
- **A runtime `OraclePolicy` value threaded through the cache (the original
  design).** Rejected on the 2026-07-02 revision: with price resolution
  already fail-closed everywhere (ADR 0003), a policy value that only ever
  selected "read strictly" or "don't read" added a parameter and a cache field
  for a distinction the call graph already encodes structurally.
- **Per-asset switch.** Rejected because the risk effect is defined by the flow,
  not the asset.
- **Caller-specified policy.** Rejected because oracle safety must not be user
  input.

## Consequences

Positive:

- Risk-increasing and solvency-dependent flows fail closed by construction:
  they call `calculate_account_risk_totals`, which cannot return a degraded
  price.
- Repay and debt-free exits stay available during an oracle outage because they
  never reach oracle code, not because a policy flag permitted a weaker check.
- The distinction is enforced by the call graph, not by a value that could be
  threaded incorrectly at a new call site.

Accepted costs:

- Every new entrypoint must be reviewed for whether it needs a price at all;
  the risk-effect analysis is "does this path call `calculate_account_risk_totals`
  or `token_price`," not "which policy value do I pass."
- Views must stay conservative and must not be treated as proof that a mutation
  will succeed.

## Revisions

### 2026-07-02: Removed the `OraclePolicy` runtime type

The original decision described an `OraclePolicy` enum (`RiskIncreasing` /
`RiskDecreasing` / `Repay` / `Liquidation` / `View`) threaded through the
transaction cache, with strict policies failing closed and permissive policies
allowing degraded reads (a stale, missing, or out-of-tolerance price accepted
under specific flows). No such type exists in the current tree: price
resolution (ADR 0003) has one fail-closed path with no permissive variant. The
`Cache` struct (see `context/mod.rs`) now only carries resolution memos
(`prices_cache`, cycle `resolving` stack, etc.).

The flow-level distinction this ADR records is unchanged in substance — some
flows must not proceed on bad pricing, others do not need pricing at all — but
it is now expressed structurally by whether a flow's code path calls price
resolution (permissiveness by omission), not by a policy value passed into it.

## References

- `contracts/controller/src/oracle`
- `contracts/controller/src/risk/validation.rs::require_post_pool_risk_gates`
- `contracts/controller/src/risk/totals.rs::calculate_account_risk_totals`
- `contracts/controller/src/positions/repay.rs`
- `contracts/controller/src/context/mod.rs` (Cache with prices_cache and resolving stack; no OraclePolicy)
- [ADR 0003](./0003-oracle-dual-source-with-tolerance-bands.md)
- `architecture/INVARIANTS.md` §4.3 (Call-site policy / permissiveness by omission)
