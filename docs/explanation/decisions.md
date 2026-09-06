# Protocol decisions

For maintainers reviewing why the current design exists. ADR-0001 through
ADR-0020 are accepted decisions; ADR-0021 remains proposed. These summaries
replace the separate records, whose full history remains in Git.
Behavior, exceptions, and evidence limits are specified in [architecture](../reference/architecture.md),
[formulas](../reference/formulas.md), [invariants](../reference/invariants.md),
and the [threat model](threat-model.md).

<a id="adr-0001"></a>
## ADR-0001: Governance, controller, pool authority

Deploy governance as controller owner and the controller as pool owner.
This concentrates account/risk policy in the controller while the pool handles
accounting. The trade-off is a critical shared trust boundary: controller bugs,
upgrades, or an authorized ownership change can affect every market.

<a id="adr-0002"></a>
## ADR-0002: Central custody, separate market books

Key markets by hub and asset, giving each its own cash, shares, indexes, and
revenue. This avoids one custody contract per market. Repeated listings of one
token keep distinct books but share its physical pool balance and token risks.

<a id="adr-0003"></a>
## ADR-0003: Indexed shares and directed rounding

Store supply and debt as shares, applying interest through indexes rather than
updating every account. Supply mints floor shares, withdrawals burn ceiling
shares, borrowing mints ceiling debt, and partial repayment burns floor debt.
Close paths and dust rejection matter; [formulas](../reference/formulas.md)
define units and exceptions rather than assuming all rounding is identical.

<a id="adr-0004"></a>
## ADR-0004: Dual-source agreement

Serve the integer midpoint, rounded down, only when both configured legs pass
validity, freshness, and tolerance checks and the result passes its sanity
band. One healthy leg is not a fallback. Runtime tolerance uses the half-up
rounded larger/smaller ratio; the reciprocal pair is validated at admission.
Single-source keys have separate admission constraints.

<a id="adr-0005"></a>
## ADR-0005: Fail-closed valuation

A valuation-dependent operation must obtain every required valid price in its
context. Failure aborts rather than substitutes a guess. This protects risk
decisions at the cost of withdrawal/liquidation availability during outages.

<a id="adr-0006"></a>
## ADR-0006: Typed proposal, bound execution

Check typed administrative intent when proposing, then execute its bound
payload only when ready and within the grace window. Target contracts retain
execution-time checks. Anyone can execute by omitting the executor identity.
Owner-proposed canceller recovery uses its own delay and cannot be cancelled;
a revocation target cannot veto its own removal through ordinary cancellation.
Completed operations need a fresh proposal and delay before reuse. Effective
delays and key custody remain operational requirements.

<a id="adr-0007"></a>
## ADR-0007: Emergency ratchet

Immediate guardian actions can pause and tighten listing flags, not reopen.
Reopening uses delayed administration. A full listing rewrite can clear flags,
so an operator must preserve intended restrictions explicitly in that update.
The ORACLE role similarly narrows sanity bands; widening requires timelocked oracle reconfiguration.

<a id="adr-0008"></a>
## ADR-0008: Independent halt flags

Keep `frozen`, `paused`, and `no_seize` independent. Entry checks paused/frozen,
ordinary exits check paused, and liquidation seizure checks no_seize. A paused
unselected debt leg does not block repayment of a different selected leg.
Pausing collateral alone does not block its seizure.

The proposed coupling of no_seize to frozen was closed without adoption on
2026-09-05. Coupling would also halt unrelated debt activity and would not
restore liquidation of existing holders. Accepted cost: supplying non-dust
no_seize collateral can block the account's pro-rata liquidation. Interest
continues during listing pauses; insolvent cases have the governed
[force-socialize path](../reference/runbooks/force-socialize-bad-debt.md).
Operators should inspect existing usage before setting no_seize; use frozen
when the objective is to stop new entry. For prolonged pauses, consider a
timelocked rate reduction because pausing does not stop accrual.

<a id="adr-0009"></a>
## ADR-0009: Immutable spoke binding

Bind each account to its spoke once. Changing an argument cannot move existing
debt into a different risk regime. Governance can change listings and refresh
applicable stored risk values, but does not rebind the account.

<a id="adr-0010"></a>
## ADR-0010: Cash flash-loan repayment

Require a contract receiver and allowance at least principal plus fee.
The pool pulls exactly that amount and checks its expected balance after
payout, after callback and after collection. Callback pushes do not replace
repayment; excess allowance is permitted. Cash flash loans create no
account debt and are distinct from account strategy settlement.

<a id="adr-0011"></a>
## ADR-0011: Measure router settlement

Authorize one exact input-token transfer invocation to the configured
router; do not grant a token allowance. Ignore returned amounts, reject
excess measured spend and require positive measured output. Router minimum
output is checked after fees but before payout, not independently by the
controller. Controller-held unused input returns to caller; router-internal
residuals follow a capped admin-revenue policy. Final account risk checks
do not guarantee route quality.

<a id="adr-0012"></a>
## ADR-0012: Supplier-index loss allocation

Remove eligible bad debt and write down its market's supply index, subject to
the nonzero floor, rather than transfer debt into another market. Current supplier claims bear the
write-down. The nonzero floor protects conversions but may leave unpaid backing;
it is not a guarantee that every supplier can withdraw its displayed claim.

<a id="adr-0013"></a>
## ADR-0013: Credit measured receipts

Supply, repayment, recapitalization, and supported strategy legs account for
what arrived, rather than what a sender requested. Under-delivered liquidation
repayment reduces the associated seizure. This supports specific receipt-tax
behavior; it does not admit arbitrary rebases, sender surcharges, or false balances.

<a id="adr-0014"></a>
## ADR-0014: Governed source admission

Validate source structure, dependency relationships, provider-address overlap
and smoothing policy before accepting configuration. Smoothing governs source
selection; it is not a post-midpoint transform. Reflector `Twap` mode takes an
equal-weight mean of its accepted observations. LP configurations are
sole-source, waive smoothing and tolerance checks, and use a separate sanity
cap alongside pool and underlying-price validation.

Changed keys receive provider-specific attestation and a probe. Non-LP probes
permit market-condition failures; LP admission requires a usable price.
Transitive dependents receive structural revalidation without new live probes.
This avoids repeated provider calls consuming VM memory; an LP receives live
suitability checks on its own admission, not on each upstream-key edit.
Provider-address checks do not establish independent operators or upstream
data, and feed-nature labels remain configuration assertions.

<a id="adr-0015"></a>
## ADR-0015: Literal asset-unit caps

Convert caps to shares at the current index when exposure grows. Zero admits
no new exposure; exits do not consume cap headroom. Same-spoke liquidation
credit is not new aggregate supply: its move is usage-neutral and its protocol
fee reduces usage. Rechecking the cap on that move would obstruct liquidation.

<a id="adr-0016"></a>
## ADR-0016: Millisecond rates and chunked accrual

Use RAY rates per millisecond and bounded accrual chunks. Recompute from the
preceding chunk's market state rather than extrapolate one unlimited interval.
Chunking does not eliminate value overflow, cumulative work, cadence dependence,
or approximation/rounding error; numeric limits belong in [formulas](../reference/formulas.md).

<a id="adr-0017"></a>
## ADR-0017: Keep testing powers out of deployment

Feature-gate test/verification helpers and check release WASM for forbidden
exports. An artifact check is evidence only for the binaries, exports, and
build inputs it actually checks. Formal results also depend on harness and
external-call assumptions; source specifications alone are not successful proofs.

<a id="adr-0018"></a>
## ADR-0018: Compact route instructions

Use bounded indexed address/amount registries for route instructions.
Decode checks format, index bounds, Prev links and individual split weights;
runtime vault accounting checks available balances and venue receipts.
Split weights apply to the shrinking remainder and need not sum to one.
Registries need not be unique; Aquarius LP constituents come from the pool.
A valid instruction stream does not establish good economic execution.

<a id="adr-0019"></a>
## ADR-0019: Liquidation share credit

Offer a same-spoke share receiver to avoid collateral cash payout. Seized
shares split into receiver credit and protocol revenue by reclassification,
not fee-share minting against nonexistent cash. Position limits and receiver
authorization still apply; a newly introduced receiver asset needs a listing.
Existing receiver positions retain their risk tuple; new positions use current
listing values. This is not identical to ordinary supply's refresh behavior.

A liquidation may therefore change two accounts, not just its target. The
receiver's supply increase is bounded by target seizure; its debt does not move.
[Events](../reference/events.md) distinguish gross LiqSeize from net LiqCredit.

The account-isolation specification exempts the declared receiver while framing
a third account. Its receiver-credit bound does not prove the fee reaches pool
revenue: the controller model havocs that pool call. See [model boundaries](certora-sunbeam-prover-tuning.md#model-boundaries).

<a id="adr-0020"></a>
## ADR-0020: Zero-fee flash-position callback

Mint debt without origination fee and forward it to a caller-chosen
contract callback. Require nonnegative collateral minima with at least one
positive, the debt market's flash-loan eligibility, and an open solvent
final position. Minima bind measured controller receipts; pool supply
remeasures delivery. Returned debt is never auto-repaid: it can become
declared collateral, be refunded when refund-listed, or remain uncredited.

This is an authorized borrowing strategy, not a free cash-flash round trip.
An already healthy account may support the new debt with little added
collateral. Its fee difference from multiply is an accepted economic choice.

<a id="adr-0021"></a>
## ADR-0021: Same-market bad-debt netting

**Status: Proposed, deferred (2026-09-02).** Current cleanup seizes remaining
supply as revenue, then socializes gross debt, even when both are in one
market. Netting first would change accounting and event semantics; it has not
been adopted. Preserve this distinction when interpreting treasury gains,
supplier losses, and recapitalization. See [bad-debt residuals](threat-model.md#bad-debt-and-liquidation).
