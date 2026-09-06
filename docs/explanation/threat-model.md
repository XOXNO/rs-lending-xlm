# Threat model

For reviewers and operators assessing authority, funds, prices, and availability.
Controls below describe current source, not proof of a safe deployment. Risk IDs
from the former STRIDE document remain stable; inherited severity scores are
not repeated because their assumptions and several source claims had drifted.
The [historical STRIDE matrix](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/STRIDE.md)
preserves those ratings without presenting them as a fresh assessment.
See [invariants](../reference/invariants.md) for precise properties and
[audit history](../audit/README.md) for historical finding dispositions.

## Assets and trust roots

Protect supplier claims, pool cash, borrower collateral, account authority,
market accounting, price integrity, governance authority, and liquidation/exit
availability. Router fee entitlements and oracle signer configuration are
separate assets exposed through external dependencies.

| Authority | Current repository boundary | What deployment must establish |
|---|---|---|
| Governance owner and roles | Owner bootstrap/recovery; typed delayed administration; restricted immediate roles | Actual owners, signers, role roster, and effective delays |
| Controller | Governance-owned under the deployment helpers; owner-gated administration | Correct deployed code, owner, pool/NFT/aggregator pointers |
| Pool | Controller-owned accounting mutations, no separate ownership-transfer endpoint | Correct controller wiring and artifact |
| Position NFT | Stored controller authority for mint/burn/upgrade; ordinary holder/approved transfers | Correct collection address and controller binding |
| Price aggregator | Governance-owned deployment; owner configuration and upgrade | Sources, bands, feed metadata, actual owner and artifact |
| Swap aggregator | Independent constructor administrator; owner-controlled fees, sweep and upgrade | Owner policy; no lending-governance router-upgrade route exists |
| XOXNO oracle | Independent constructor administrator; signer/configuration and upgrade powers | Honest quorum participation, owner policy, and monitoring |
| DeFindex adapter | Fixed asset/hub/spoke/controller configuration; per-vault authenticated accounts | Correct immutable constructor inputs; no admin rescue interface |

An address or owner gate proves neither multisig custody nor a timelock. An
external smart-contract owner may impose its own policy. Lending governance
cannot be assumed to control standalone router/oracle administration.

### Governance windows and emergency powers

Effective delays are in ledgers: Standard uses the configured minimum;
Sensitive uses `max(minimum, 12)`; Recovery uses `max(minimum, 518400)`.
The source's intended later Sensitive floor is not the compiled value.
Repository network inputs currently use a minimum of 12; this is not a live
chain reading. Verify the deployed minimum and review window before funding.
Raising the configured minimum and changing a compiled floor are different actions.

GUARDIAN can immediately pause, tighten listing flags, and create empty hubs
or spokes. ORACLE can immediately narrow sanity bands. The owner can revoke
those two hot roles immediately and perform one-time deployment bootstrap.
Reopening, global position-manager changes, and ordinary upgrades use delayed
operations. Controller construction/upgrade pauses the controller; this is
not a guarantee that every component upgrade pauses lending.

Typed proposals perform proposal-time checks; targets retain execution-time
validation. Ready operations must also be within the grace window. Anyone may
execute with no executor identity; supplying one requires its authorization
and EXECUTOR role. Executor/canceller separation exempts the governance owner.
A revocation target cannot cancel its own removal, but an independent canceller
can veto it. Owner-proposed Recovery operations cannot be cancelled and replace
cancellers after their delay; they do not recover a lost owner key.

## Account authority

The position NFT holder controls the lending account. Approving an NFT
operator enables transfer of the entire position, including collateral and
debt. This is not a narrowly scoped permission to handle a collectible.

An account delegate also needs active global position-manager registration.
Its grant is stamped with the granting owner's address, not a transfer epoch:
it becomes inactive while someone else holds the NFT and can revive if the
NFT returns before an intervening owner updates delegates. Owner revocation
is immediate; global deactivation takes effect when governance executes it.
Delegates can borrow/withdraw to their chosen recipient within account gates.
Those gates do not constrain them to acting in the owner's economic interest.

Controller account renewal requires the owner. Direct NFT renewal is
permissionless and extends the Owner entry, its holder's Balance entry, and
instance TTL; it does not renew the controller account. Archived persistent entries need restoration. Sequential
NFT IDs are finite and not recycled; limits and renewal do not establish
worst-case network-budget availability.

## Token assumptions

Measured inbound receipt prevents crediting a requested amount that never
arrived on supported supply, repay, recapitalization, and strategy paths.
It is not a generic safety guarantee for arbitrary token contracts. Listing
review must consider sender surcharges, false balances, rebases, clawbacks,
upgrades, and semantics that can change after admission.

One token listed in several hubs shares physical pool custody even though
market books are separate. Direct donations do not rewrite those books.
Cash flash loans impose exact balance transitions and allowance repayment;
receipt-tax compatibility elsewhere does not establish flash-loan compatibility.

## Routes, callbacks, and external integrations

Router swaps settle measured input/output changes. The controller grants
one exact input-transfer invocation, not a token allowance, and refunds
unspent input still held by the controller. The router checks its payload
minimum against output after fees but before payout; its own residuals
follow a capped admin-revenue policy. The controller requires positive
measured output and final account risk, not an independent slippage bound.
A compromised router may consume authorized input for dust output while the
final account passes its risk gates. Exposure is bounded by routed funds and
those gates, not by an independent controller slippage limit.

Flash position mints debt without origination fee, requires nonnegative declared
collateral minima with at least one positive, and leaves an open solvent position. An already healthy
account can support that debt with little additional collateral. Debt leftovers
are not auto-repaid. Refunds cover only positive callback deltas of refund-listed
tokens; declared collateral is supplied. Neither category sweeps prior balances.
This differs from cash flash loans and from multiply's
fee policy. The debt market's flash-loan flag also gates flash position.

Approved Blend pools retain their external upgrade trust until approval is
removed. DeFindex vault authentication isolates account bindings, but the
adapter has no recovery path for arbitrary stranded assets.

Protected monetary entrypoints reject guarded callback reentry; Soroban also
restricts indirect reentry. Do not infer an EVM token-hook exploit without
establishing host reachability. Risk views lack the monetary guard; consumers
must not treat them as a transaction-stable oracle for intermediate state.
A native callback fixture alone does not establish a deployed host call path.

## Price integrity and availability

Two configured price legs must both be usable and agree within tolerance;
one surviving leg is not a fallback. A single-source key has no top-level
agreement check, although its transitive source may have multiple dependencies.
Sanity bands constrain accepted prices but cannot establish economic correctness.

Admission checks source structure, provider-address overlap, smoothing policy,
and provider-specific metadata. Provider separation is not proof of independent
operators or upstream data. Feed-nature labels are configuration assertions.
Non-LP admission probes can accept temporary market-condition failures;
transitive dependent revalidation is structural rather than a new live attestation.
Later upstream changes do not erase the lending aggregator's runtime age checks,
but can invalidate assumptions made during admission.

XOXNO submissions authenticate one registered signer each. A new aggregate
requires a quorum of fresh, timestamp-clustered submissions and takes their
lower median. Future skew is bounded at 60 seconds, not forbidden entirely.
Equal package timestamps may replace a signer's observation. Honest quorum
participation must be evaluated against the accepted cluster; setting a
threshold above half the registered signers alone does not establish an
honest median when some signers are absent.

An ordinary submission below quorum leaves the prior aggregate unchanged;
reads can serve it until their freshness limits expire. Owner recomputation
removes an aggregate when its feed lacks quorum. Threshold, submission-age,
and skew setters do not recompute existing aggregates; follow them with
batched `recompute_feeds`. Later quorum submissions can also replace aggregates
under the new settings.
The cluster anchor is clamped to ledger time. Read paths still apply their
respective package/write timestamp and freshness rules.

Price failure can stop liquidation as well as borrowing and withdrawal.
`quotes` can return a nonzero candidate price with `valid=false`; consumers
must honor validity rather than use the price field alone. Required lending
valuations use strict prices and cache results within the operation context,
not a guarantee of identical timestamps across upstream observations.

## Bad debt and liquidation

Global pause leaves designated exit/recovery endpoints callable. Listing-level
paused debt blocks a repayment leg that selects it. Seizure uses its own
no_seize flag across nonzero planned collateral legs; one such flag can abort
a pro-rata liquidation. It does not prevent new supply by itself. Interest
continues while a listing is paused. Missing prices and liquidity limits remain
independent obstacles even when an entrypoint is not pause-gated.

Liquidators bear execution-time bonus, rounding, and route-quality risk.
A tiny repayment can retire debt while its pro-rata seizure rounds to zero.
Admission does not couple collateral price, decimals, bonus and fee to the
minimum-collateral floor. Expensive low-decimal collateral can yield zero
seizure even for a $5 repayment; assess that precision risk before listing.
The [seizure fixture tests](../../contracts/controller/tests/positions/liquidation_math.rs) reproduce this limit.
Share credit avoids collateral cash payout but still requires an authorized
same-spoke receiver in Normal mode, position capacity, and a listing for a newly credited asset.
The absence of a universal final health-factor assertion on liquidation is not
itself evidence of a profitable attack; the planning and measured-settlement
arithmetic require separate verification.

Cleanup converts all remaining account supply to protocol revenue and socializes
its gross debt, including same-market supply/debt pairs. Netting remains
[proposed](decisions.md#adr-0021). Current suppliers bear index write-downs;
a supplier exiting earlier can avoid that loss. The index floor can leave
material unpaid backing, so displayed supplier claims are not a universal
pro-rata cash-payout guarantee. New supply checks backing; recapitalization
repairs the book's measured shortfall without minting shares.

The compile-time dust cleanup threshold and governance-settable collateral
floor can diverge. Above the permissionless cleanup threshold, an insolvent
account may require the governed [force-socialize runbook](../reference/runbooks/force-socialize-bad-debt.md).
An account whose debt does not exceed unweighted collateral is ineligible,
even if its health factor is below one and listing flags block liquidation.

## Numeric and resource limits

Finite RAY value capacity can be exhausted before the index ceiling. Synchronizing
an overlarge book can then fail before an otherwise risk-reducing operation.
Choose caps with plausible index growth, not just today's token balance.
Accrual cadence changes utilization and subsequent rates; bounded chunks do not
make cadence neutral or prove exact conservation after integer rounding.
See [numeric limits](../reference/formulas.md#numeric-limits).

Position/route limits reduce work but do not prove every maximum-size operation
fits deployed CPU, memory, footprint, and oracle-call budgets. Caller-supplied
vectors still cost resources. Full-risk threshold refresh can fail atomically when an included account's
final health factor is below 1.05; isolate and investigate the account rather
than assuming earlier updates persisted. LTV-only refresh has no such final gate.

## STRIDE register

Each ID retains its original subject. Controls limit a scenario; they do not
mean zero residual risk. Operational assumptions above apply to these rows.

| ID | Threat and control boundary |
|---|---|
| Spoof.1 | Compromised privileged key acts legitimately as its role; delay, cancellation, restricted roles, and custody are the controls. |
| Spoof.2 | Wrong deployment identity; constructors/helpers wire authority atomically, but standalone arguments and later changes require verification. |
| Spoof.3 | Former/unlisted delegate; account grant, stamped owner, and active manager checks apply, including return-to-owner revival. |
| Spoof.4 | Non-contract flash receiver; contract-address validation and path-specific repayment/finalization apply. |
| Spoof.5 | Impersonated vault; authenticated vault addresses key separate adapter accounts. |
| Tamper.1 | Manipulated/unavailable prices; freshness, sanity, and configured source agreement do not establish source honesty. |
| Tamper.2 | Dishonest signer subset; median uses the fresh accepted cluster, whose honest participation remains a trust assumption. |
| Tamper.3 | Non-standard tokens/donations; measured receipts and separate books do not neutralize arbitrary token semantics. |
| Tamper.4 | Malicious router/venue; measured input/output bounds apply, while payload minimum output remains router-enforced. |
| Tamper.5 | Callback reentry/intermediate state; guards and host rules protect reachable paths, not hypothetical EVM behavior. |
| Tamper.6 | Accrual manipulation/extremes; bounded indexes and chunks coexist with cadence and value-overflow risks. |
| Tamper.7 | Malicious upgrade; core upgrades are delayed, including price aggregator; standalone owners retain their own upgrade policy. |
| Tamper.8 | Hostile Blend integration; allowlisting and measured settlement do not freeze an approved pool's code. |
| Tamper.9 | False feed-nature label bypasses intended smoothing selection; nature is operator-asserted. |
| Tamper.10 | Upstream configuration changes after admission; structural checks and runtime age limits remain, but admission assumptions need monitoring. |
| Repudiate.1 | Denied admin actions; inherited governance lifecycle/role/ownership events exist, but application-event coverage is not universal. |
| Repudiate.2 | Misread user/liquidation actions; use measured repayment and distinct gross/net batch tags, not one-batch assumptions. |
| Repudiate.3 | Oracle application-event gaps; submission/configuration lack dedicated events, while inherited ownership events remain available. |
| Info.1 | Public positions, prices, and governance state; confidentiality is not promised. |
| Info.2 | Liquidation competition and MEV; the bonus curve bounds terms, not ordering or liquidator profit. |
| Info.3 | Visible pending governance changes allow anticipatory positioning; observability is intentional. |
| Info.4 | Invalid quotes may contain prices; use validity flags or strict reads. |
| DoS.1 | Price outage blocks valuation-dependent actions, including liquidation; fail-closed availability cost. |
| DoS.2 | Selected paused debt or no_seize collateral blocks liquidation; distinct flag policies matter. |
| DoS.3 | False-alarm pause needs delayed reopening; emergency response is asymmetric. |
| DoS.4 | Lost governance keys; proposer safeguards and owner-dependent canceller recovery do not restore a lost owner. |
| DoS.5 | Resource exhaustion; limits do not substitute for real-WASM maximum-case measurements. |
| DoS.6 | Archived storage blocks access until restoration; controller renewal is owner-authorized. |
| DoS.7 | Signer liveness loss prevents new aggregates; a prior aggregate can remain usable until stale. |
| DoS.8 | Cash/utilization/cap limits reject otherwise desired actions; zero caps admit no new exposure. |
| DoS.9 | Dust and zero-share movements; rejection/floors reduce griefing but do not guarantee liquidation profitability. |
| DoS.10 | Router/oracle ownership renunciation disables administration; existing oracle signers may continue, but future repair powers are lost. |
| Elevation.1 | Governance-owner compromise; actual configured delay and approved replacement code determine exposure. |
| Elevation.2 | Guardian attempts reopening; immediate flag ratchets reject it, while delayed full rewrites can clear flags. |
| Elevation.3 | Role overlap/cancellation abuse; separation exempts owner and recovery has its own rules. |
| Elevation.4 | Delegate exceeds mandate; owner-only grant management and delayed global manager deactivation limit eligibility, not economic intent. |
| Elevation.5 | Third-party creation of foreign risk; supply top-ups require existing positions, while account creation belongs to its caller. |
| Elevation.6 | Router owner upgrades/sweeps/changes fee rights; no lending-governance router upgrade route supplies a delay. |
| Elevation.7 | Oracle owner changes code/signers; independent source comparison and bands constrain accepted prices, not all manipulation. |
| Elevation.8 | Test powers in release WASM; feature and artifact checks cover only their actual build/export scope. |
| Elevation.9 | Vault abuses adapter authority; current adapter exposes supply/withdrawal, not borrowing or other vault accounts. |
| Elevation.10 | ORACLE role narrows band to exclude market price; immediate fail-closed denial of service remains possible. |

## Review triggers and evidence limits

Recheck these boundaries after upgrades, new entrypoints/providers/venues,
authorization changes, storage or SDK changes, and governance/configuration
updates. Observe actual events plus transactions/state; no event-only model
covers all administration. Verify source-matched artifacts and deployed roles.

Historical native tests, mocked authorization, formal rules, and successful
static checks are different evidence. None alone proves arbitrary external
behavior, maximum network budgets, or live deployment correctness. The
[formal-model notes](certora-sunbeam-prover-tuning.md) state those boundaries.
