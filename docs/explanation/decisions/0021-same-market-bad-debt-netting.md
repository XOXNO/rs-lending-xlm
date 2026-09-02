# 0021. Net same-market supply against debt before socializing bad debt

**Status:** Proposed
**Date:** 2026-09-02

## Context

`execute_bad_debt_cleanup` seizes an insolvent account's supply positions as
protocol revenue, then writes each debt position off against its market's
supply index. When the account holds supply and debt in the same market, the
suppliers of that market absorb the whole debt while the protocol keeps the
account's supply. With 1,000 of supply and 500 of debt in one market, suppliers
lose about 500 and the protocol gains about 1,000 in revenue shares; netting
first would write down nothing.

Pinned by
`tests/test-harness/tests/controller/same_market_bad_debt_cleanup_arithmetic.rs`.

## Decision

Deferred. Netting changes accounting arithmetic and the bad-debt event shape,
which is outside the revert-only budget of the 2026-09 gap hunt. Revisit with a
storage-compatible design: net the same-market pair through `net_settle`
before the seize batch, then socialize only the residual.

## Consequences

Until adopted, the cross-market and same-market cases share the residual risk
recorded in `docs/explanation/threat-model.md`, and `recapitalize` is the path
that returns treasury value to a written-down market.
