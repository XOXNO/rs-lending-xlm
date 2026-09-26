# Threat model

This model describes threats to funds, account authority, prices, and protocol
availability. It pairs source controls with the assumptions and residual risks
that reviewers and operators must assess.

Read [Architecture](../reference/architecture.md) first for markets, accounts,
and contract responsibilities. The [invariant reference](../reference/invariants.md)
defines precise safety properties.

## Assets and trust roots

The assets at risk are supplier claims, pool cash, borrower collateral, account
and governance authority, market accounting, and price integrity. Liquidation
and exit availability also affect fund recovery. External dependencies expose
router fee entitlements and oracle signer configuration.

| Authority | Source boundary | Deployment checks |
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

Governance delays are measured in ledgers. The effective delay depends on the
operation tier:

| Tier | Effective delay |
|---|---|
| Standard | Configured minimum |
| Sensitive | `max(minimum, 12)` |
| Recovery | `max(minimum, 518400)` |

The repository's [network configuration](../../configs/networks.json) sets the
minimum to 12 ledgers on testnet and mainnet. Verify the deployed minimum and
its review window before funding. Raising the configured minimum and changing a
compiled tier floor are different actions.

GUARDIAN can immediately pause, tighten listing flags, and create empty hubs
or spokes. ORACLE can immediately narrow sanity bands. The owner can revoke
those two hot roles immediately and perform one-time deployment bootstrap.
Reopening, global position-manager changes, and ordinary upgrades use delayed
operations. Controller construction and upgrade pause the controller. Pool,
position NFT, price aggregator and governance upgrades do not pause lending.

A PROPOSER that is not the owner can schedule listing, cap, curve and limit
changes. Ownership transfers, code upgrades and migration, the timelock
minimum delay, price and swap sources, Blend approvals, the revenue accumulator
and role grants need the owner as proposer. A stolen non-owner PROPOSER key can
therefore schedule disruptive changes, such as listing flags, risk parameters,
role revocations or an unpause, but cannot replace code or prices. It cannot
change the minimum delay either. `UpdateGovDelay` can only raise the minimum,
so no later delay update can lower it. An owner `UpgradeGov` can still replace
the governance code and its delay rules
([INV-AUTH-05](../reference/invariants.md#inv-auth-05)).

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
instance time to live (TTL); it does not renew the controller account. Archived
persistent entries need restoration. Sequential NFT IDs are finite and are not
recycled. Renewal and position limits do not guarantee that maximum-size
operations fit network budgets.

## Token assumptions

Measured inbound receipt prevents crediting a requested amount that never
arrived on supported supply, repay, recapitalization, and strategy paths.
It is not a generic safety guarantee for arbitrary token contracts. Listing
review must consider sender surcharges, false balances, rebases, clawbacks,
upgrades, and semantics that can change after admission.

A token issuer can change the token's decimals after listing, for example with
a `set_metadata` call. Pool decimals are fixed at listing, and the pool never
reads the token's decimals itself. Governance calls the token's `decimals()`
and `symbol()` on every `CreateLiquidityPool` proposal and on every
`ConfigureAssetOracle` proposal for a `PriceKey::Token` key, and a failing call
rejects the proposal with `InvalidAsset` (6). Governance uses the live decimals
only while the price aggregator holds no oracle for the token. Otherwise
`resolve_oracle` uses the stored oracle's `asset_decimals`, and
`CreateLiquidityPool` checks `asset_decimals` against the same value. The price
aggregator rejects a replacement oracle that changes the stored
`asset_decimals` with `InvalidOracleDecimals` (221). Thus oracle maintenance
keeps the listed decimals after a relabel, and a listing in a second hub must
use them too.

Live decimals still apply to every `CreateLiquidityPool` and
`ConfigureAssetOracle` proposed while the aggregator holds no oracle for the
token, in either order. This includes every such proposal after a
`SetPriceAggregator` re-point to an aggregator with an empty registry. Before
execution, the operator must compare the resolved `asset_decimals` of every
pending listing and oracle operation for that token with each other and with
the existing pools of that token. On any mismatch, the operator cancels the
operation. After a mismatch executes, only an aggregator Wasm upgrade can
correct the stored unit: `set_oracle` rejects a decimals change, and
`remove_oracle` exists only in testing builds.

One token listed in several hubs shares physical pool custody even though
market books are separate. Direct donations do not rewrite those books.
Cash flash loans impose exact balance transitions and allowance repayment;
receipt-tax compatibility elsewhere does not establish flash-loan compatibility.

Aquarius LP collateral earns venue rewards for its holder, which is the pool.
The pool has no entrypoint to claim or forward them. The venue's reward claim
did not require the holder's authorization when reviewed (mainnet simulation,
2026-09), so any caller can push accrued rewards into the pool address as an
unbooked donation. Gauge rewards need the pool's authorization and therefore
cannot be claimed at all. Suppliers of LP collateral should expect no venue
rewards; this is forgone yield, not a loss of principal.

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

That bound covers the controller's own grant only. The router calls the pool
and token addresses its payload names and keeps no allowlist of them, so a
route can put third-party code on the call stack below the caller's
authorization. A token transfer that such code makes from the caller is
recorded by an honest simulation as a child of the caller's authorization
entry, and it executes if the caller signs that tree. The loss is then the
caller's wallet, not the routed amount, and neither the payload minimum nor the
final risk gate bounds it. An honest swap strategy gives the caller no child
entry, and a direct router swap gives exactly one input transfer. A client must
decode the route it signs and refuse an authorization tree with any other
child. The direct `execute_strategy` path has the same exposure for every swap
user.

Flash position creates debt without an origination fee and leaves an open
solvent position. Declared collateral minima must be nonnegative, with at least
one positive. An already healthy account can support that debt with little
additional collateral. Debt leftovers are not automatically repaid.

Refunds cover only positive callback deltas of refund-listed tokens; declared
collateral is supplied. Neither category sweeps prior balances.
Cash flash loans require repayment, and multiply applies a different
origination-fee policy. The debt market's flash-loan flag also gates flash position.

Approved Blend pools retain their external upgrade trust until approval is
removed. DeFindex vault authentication isolates account bindings, but the
adapter has no recovery path for arbitrary stranded assets.

Protected monetary entrypoints reject guarded callback reentry; Soroban also
restricts indirect reentry. A callback exploit requires a reachable host call
path; a native fixture alone does not establish that path in a deployed contract.
Risk views lack the monetary guard, so consumers must not treat intermediate
risk reads as a transaction-stable oracle.

## Price integrity and availability

Two configured price legs must both be usable and agree within tolerance;
one surviving leg is not a fallback. A single-source key has no top-level
agreement check, although its transitive source may have multiple dependencies.
Sanity bands constrain accepted prices but cannot establish economic correctness.

A single-source feed can report any price `p` in its band `[min, max]`. The
true NAV `P` is also in the band, so `P / p <= u = max / min`. The 10%
single-source cap gives `u <= 11/9`. Bad debt occurs only when the reported
collateral is less than the debt. At true NAV, that is `C / D < P / p <= u`.
The price deviation is not the threshold; the band ratio `u` is. A healthy
account has `C / D >= 1 / LT`. A borrow at an in-band price keeps
`C / D >= 1 / (u * LTV)`. A liquidation does not decrease the units held for
each unit of debt. Thus lenders cannot lose while `LT < 1 / u`. The Liqvid hub
(LT 60% or 53%, `u` at most 1.22) has a large margin.

The borrower has less protection. The bonus `b` is the curve bonus at the
reported HF, capped at `HF / LT - 1`. It is not the base bonus. On the default
curve (target HF 1.10, maximum bonus at HF 0.80) with LT 60%, a reported HF of
0.98 gives `b` of about 29%, not 5%. The curve 1.06/0.90/598 with LT 53% keeps
`b` at or below 10%. One liquidation that pays `R` at price `p` costs the
borrower at most `min(E, R * ((1 + b) * P / p - 1))` at true NAV. `E` is the
equity at true NAV. A full close on a whole-unit leg can add one unit at true
NAV. When `b` reaches the cap, one liquidation takes all of `E`. With LT 60%
and `u = 11/9`, this occurs for an account at true HF 1.01 when `P` is the
band top and `p` is the band floor. The
[oracle-deviation bound tests](../../tests/test-harness/tests/controller/liqvid_oracle_deviation_bounds.rs)
pin these bounds.

Admission checks source structure, provider-address overlap, smoothing policy,
and provider-specific metadata. Provider separation is not proof of independent
operators or upstream data. Feed-nature labels are configuration assertions.
Non-LP admission probes can accept temporary market-condition failures.
Changing an upstream key revalidates dependent source structure without a new
live attestation for each dependent. Upstream changes do not erase the lending
aggregator's runtime age checks,
but can invalidate assumptions made during admission.

XOXNO submissions authenticate one registered signer each. A new aggregate
requires a quorum of fresh, timestamp-clustered submissions and takes their
lower median. Future skew is bounded at 60 seconds, not forbidden entirely.
Equal package timestamps may replace a signer's observation. Honest quorum
participation must be evaluated against the accepted cluster; setting a
threshold above half the registered signers alone does not establish an
honest median when some signers are absent. A cluster with fewer than
`2 * (signers - threshold) + 1` entries must fit inside `max_cluster_spread_bps`,
or the round is a quorum miss, so one signer moves such a cluster by at most
that bound. A cluster at or above that size uses the plain lower median.

An ordinary submission below quorum leaves the prior aggregate unchanged;
reads can serve it until their freshness limits expire. Owner recomputation
removes an aggregate when its feed lacks quorum. Threshold, submission-age,
skew, and spread setters do not recompute existing aggregates; follow them with
batched `recompute_feeds`. Quorum submissions also use the updated settings.

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
same-spoke receiver in Normal mode, position capacity, and a listing for a
newly credited asset.

On a solvent account whose only leg is below 3 decimals, the quote can rise to
one whole unit or to the whole debt
([whole-unit legs](../reference/formulas.md#bonus-and-target-repayment)). An
offer that backs less than one unit still seizes nothing. In the full-debt
case the liquidator repays the debt `D` and receives one unit worth `U`. Its
effective bonus is `U / D - 1`, not the quoted bonus `b`. With `k` held units
and liquidation threshold `LT`, `HF < 1` bounds it at about
`1 / (k * LT) - 1`. The borrower loses `U - D * (1 + b)` above the normal
bonus, and `bonus_bps` shows only `b`. Listing review must note that expensive
units with a low liquidation threshold raise this loss.

Liquidation planning and measured settlement enforce its accounting bounds.
A missing universal final health-factor assertion alone does not establish a
profitable attack; assessment must verify that arithmetic and a reachable sequence.

Cleanup converts all remaining account supply to protocol revenue and socializes
its gross debt, including same-market supply/debt pairs. It does not net those
pairs first. Suppliers present at cleanup bear index write-downs; a supplier
who exits before cleanup can avoid that loss. The index floor can leave
material unpaid backing, so displayed supplier claims are not a universal
pro-rata cash-payout guarantee. New supply checks backing; recapitalization
repairs the book's measured shortfall without minting shares.

The compile-time dust cleanup threshold and governance-settable collateral
floor can diverge. Above the permissionless cleanup threshold, an insolvent
account may require the governed [force-socialize runbook](../reference/runbooks/force-socialize-bad-debt.md).
An account whose ceil risk debt does not exceed half-up unweighted collateral
is ineligible, even if its health factor is below one and listing flags block
liquidation.

## Numeric and resource limits

Finite RAY value capacity can be exhausted before the index ceiling. Synchronizing
an overlarge book can then fail before an otherwise risk-reducing operation.
Caps must account for plausible index growth as well as token balances.
Accrual cadence changes utilization and subsequent rates; bounded chunks do not
make cadence neutral or prove exact conservation after integer rounding.
See [numeric limits](../reference/formulas.md#numeric-limits).

Position/route limits reduce work but do not prove every maximum-size operation
fits deployed CPU, memory, footprint, and oracle-call budgets. Caller-supplied
vectors still cost resources. A full-risk threshold refresh aborts the entire
batch if an included account's final health factor is below 1.05. Isolate and
investigate that account; no earlier updates from the failed batch persist.
An LTV-only refresh has no such final gate.

## STRIDE register

STRIDE groups threats into spoofing, tampering, repudiation, information
disclosure, denial of service, and elevation of privilege. The stable IDs below
map each scenario to its control boundary. The assumptions above still apply;
these rows do not assign severity or establish exploitability.

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
| DoS.1 | Price outage blocks valuation-dependent actions, including liquidation; fail-closed availability cost. Supply needs no price, so an indebted borrower can add a dust leg of any listed collateral and choose which feed outage shields the account. For an Aquarius LP leg, liquidity providers can cause that outage by withdrawing pool value below `min_pool_value_wad`. The same leg blocks bad-debt cleanup and force-socialization. |
| DoS.2 | Selected paused debt or no_seize collateral blocks liquidation; distinct flag policies matter. |
| DoS.3 | False-alarm pause needs delayed reopening; emergency response is asymmetric. |
| DoS.4 | Lost governance keys; proposer safeguards and owner-dependent canceller recovery do not restore a lost owner. |
| DoS.5 | Resource exhaustion; limits do not substitute for real-WASM maximum-case measurements. |
| DoS.6 | Archived storage blocks access until restoration; controller renewal is owner-authorized. |
| DoS.7 | Signer liveness loss prevents new aggregates; a prior aggregate can remain usable until stale. |
| DoS.8 | Cash/utilization/cap limits reject otherwise desired actions; zero caps admit no new exposure. |
| DoS.9 | Dust and zero-share movements; rejection/floors reduce griefing but do not guarantee liquidation profitability. |
| DoS.10 | Router/oracle owner loss disables administration; existing oracle signers may continue, but future repair powers are lost. The source ABIs export no `renounce_ownership`; verify that the deployed artifacts match. |
| Elevation.1 | Governance-owner compromise; actual configured delay and approved replacement code determine exposure. |
| Elevation.2 | Guardian attempts reopening; immediate flag ratchets reject it. Listing edits cannot clear flags either; only a delayed relaxation bound to the listing's flags epoch can, so one proposed before a later guardian action reverts. |
| Elevation.3 | Role overlap/cancellation abuse; separation exempts owner and recovery has its own rules. |
| Elevation.4 | Delegate exceeds mandate; owner-only grant management and delayed global manager deactivation limit eligibility, not economic intent. |
| Elevation.5 | Third-party creation of foreign risk; supply top-ups require existing positions, while account creation belongs to its caller. |
| Elevation.6 | Router owner upgrades/sweeps/changes fee rights; no lending-governance router upgrade route supplies a delay. |
| Elevation.7 | Oracle owner changes code/signers; independent source comparison and bands constrain accepted prices, not all manipulation. |
| Elevation.8 | Test powers in release WASM; feature and artifact checks cover only their actual build/export scope. |
| Elevation.9 | Vault abuses adapter authority; adapter exposes supply/withdrawal, not borrowing or other vault accounts. |
| Elevation.10 | ORACLE role narrows band to exclude market price; immediate fail-closed denial of service remains possible. |

## Review triggers and evidence limits

Recheck these boundaries after upgrades, new entrypoints/providers/venues,
authorization changes, storage or SDK changes, and governance/configuration
updates. Observe actual events plus transactions/state; no event-only model
covers all administration. Verify source-matched artifacts and deployed roles.

Native tests, mocked authorization, formal rules, and successful static checks
provide different evidence. None alone proves arbitrary external behavior,
maximum network budgets, or live deployment correctness. The
[formal-model notes](certora-sunbeam-prover-tuning.md) state those boundaries.
