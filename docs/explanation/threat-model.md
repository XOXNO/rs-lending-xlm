# Threat model

Threats to funds, account authority, prices, and availability, with the
source controls, assumptions and residual risks reviewers and operators must
assess. Read [Architecture](../reference/architecture.md) first;
[invariants](../reference/invariants.md) define the safety properties and
[Design decisions](decisions.md) their rationale.

## Audit coverage

Runtime Verification audited the core contracts and `common/` at `v1.1.0`.
Findings from the [report](../../audits/2026-11-runtime-verification.pdf) are
cited by id, such as S-03.

## Assets and trust roots

At risk: supplier claims, pool cash, borrower collateral, account and
governance authority, market accounting, price integrity, and the liquidation
and exit availability that fund recovery needs. External dependencies expose
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

An address or owner gate proves neither multisig custody nor a timelock; an
external smart-contract owner may impose its own policy. Do not assume lending
governance controls standalone router or oracle administration.

### Governance windows and emergency powers

Delays are in ledgers (about 5 seconds each). A tier's effective delay is
the larger of the one configured minimum and the tier's compiled floor:

| Tier | Effective delay | Compiled floor |
|---|---|---|
| Standard | Configured minimum | None |
| Sensitive | `max(minimum, TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS)` | 120,960 ledgers, about 7 days |
| Recovery | `max(minimum, TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS)` | 518,400 ledgers, about 30 days |

The [network configuration](../../configs/networks.json) sets the configured
minimum (`timelock_min_delay_ledgers`) to 12 ledgers on testnet and mainnet,
so Standard waits about one minute and the other tiers their floors. A
minimum above a floor raises that tier. Verify the deployed minimum and its
review window before funding.

`UpdateGovDelay` changes the configured minimum. Only a new governance build
changes a floor, so a deployment keeps its floors until an `UpgradeGov`.

The end-to-end `short-timelock` governance build lowers the Sensitive floor
to 12 ledgers; its `timelock` meta stamp makes the candidate check refuse it
for deployment and publication ([ADR-0017](decisions.md#adr-0017)).

GUARDIAN can immediately pause, tighten listing flags, deactivate a position
manager, and revoke a Blend pool. CURATOR_ROLE can immediately create an empty
hub or spoke, register a market on the deployed pool, and tighten listing
flags. Every other listing change, new listings included, is timelocked.
CURATOR_ROLE is neither granted to the owner at construction nor moved by an
ownership transfer.

Curator registrars ([ADR-0023](decisions.md#adr-0023)) follow the
[curator governance controls](../../contracts/curator/README.md#governance-control)
and [cap grant rules](../../contracts/curator/README.md#cap-grants). Their
residual risks:

- Registrar checks run at queue time, not execution. Queued proposals
  execute unless they expire or are cancelled by a governance CANCELLER or,
  through the curator's `cancel_proposal`, the registrar owning the named hub
  or spoke.
- GUARDIAN suspends a registrar at once with `suspend_curator_registrar`;
  its curator calls then revert. Resuming, reassigning its hubs and spokes,
  and replacing its admin are owner-proposed sensitive. An admin change voids
  the registrar's role bits and cap grants.
- The owner's `revoke_role_immediate` on CURATOR_ROLE stops the registry's
  immediate governance calls, not its PROPOSER authority or queued operations.
  Suspension and cancellation remain separate incident actions.
- A grant's borrow cap is the hub owner's whole loss budget in that spoke;
  the grantee picks the backing collateral and can halt its seizure at once.
  The hub owner can revoke, cancel and freeze at once (leaving the spoke
  owner's seizure flag) and lower caps only through the timelock.
- A spoke owner cannot pause a listing of another registrar's hub asset; that
  pause would block repayment and liquidation of that hub's debt.

ORACLE can immediately narrow sanity bands. The owner can immediately revoke
guardian, oracle and curator roles and run one-time deployment bootstrap.
Reopening, global position-manager changes and ordinary upgrades are
delayed. Controller construction and upgrade pause the controller; pool,
position NFT, price aggregator and governance upgrades do not pause lending.

A non-owner PROPOSER can schedule every operation outside the owner-only
list: hub, spoke and listing changes (`RemoveSpoke` and
`RelaxSpokeAssetFlags` included), caps, curves, position limits, the
minimum-borrow floor, liquidity-pool creation and parameter upgrades,
`ForceSocializeBadDebt`, `Unpause`, `RevokeBlendPool`, a position-manager
deactivation and role revocations.
Ownership transfers, code upgrades and migration, the minimum delay, price and
swap sources, Blend approvals, the revenue accumulator, role grants and
position-manager activation need the owner as proposer
([governance endpoints](../reference/endpoints.md#governance)).

A stolen non-owner PROPOSER key can thus schedule listing flags, risk
parameters, spoke removal, forced bad-debt socialization, role revocations, a
position-manager deactivation or an unpause,
but cannot replace code or prices or change the minimum delay. The
owner-proposed `UpdateGovDelay` can raise the minimum but not lower it below
`TIMELOCK_MIN_DELAY_LEDGERS` (34,560 ledgers, about 2 days). It waits the
minimum in force when it was proposed, not the minimum at execution. An
`UpdateGovDelay` queued before a raise can therefore still lower the raised
minimum to the floor after the older, shorter delay. A canceller must cancel
such a pending decrease to keep a raise. An owner `UpgradeGov` can still
replace governance code and its delay rules
([INV-AUTH-05](../reference/invariants.md#inv-auth-05)).

Per [ADR-0006](decisions.md#adr-0006), anyone may execute a ready operation
within its grace window. A revocation target cannot cancel its own
removal, but an independent canceller can veto it. Owner-proposed Recovery
operations cannot be cancelled and replace cancellers after their delay; they
do not recover a lost owner key.

## Account authority

The position NFT holder controls the account. An approved NFT operator can
transfer the whole position, collateral and debt included.

A delegate also needs active global position-manager registration. Its
grant is stamped with the owner's address, not a transfer epoch, so it revives
if the NFT returns ([INV-AUTH-02](../reference/invariants.md#inv-auth-02)). A
transfer is not a revocation: call `remove_delegate` for each delegate before
transferring. No view lists live delegates. Owner revocation and GUARDIAN
global manager deactivation are immediate; reactivation is timelocked.
Delegates can borrow or withdraw to any recipient within account gates, which
do not bind them to the owner's economic interest.

Controller account renewal needs the owner. Permissionless NFT renewal
extends the time to live (TTL) of the Owner entry, the holder's Balance entry
and the instance, not the controller account
([INV-STOR-02](../reference/invariants.md#inv-stor-02)). Archived entries need
restoration. Sequential NFT IDs are finite and not recycled. Renewal and
position limits do not guarantee maximum-size operations fit network
budgets.

## Token assumptions

Measured receipt stops crediting undelivered amounts on supported supply,
repay, recapitalization and strategy paths
([INV-ACCT-03](../reference/invariants.md#inv-acct-03)), but does not make
arbitrary tokens safe. Listing review must consider sender surcharges, false
balances, rebases, clawbacks, upgrades, and semantics that can change after
admission.

**Decimals changes.** An issuer can change decimals after listing, such as
with `set_metadata`. Pool decimals are fixed at listing; the pool never reads
token decimals. Governance calls the token's
`decimals()` and `symbol()` on every `CreateLiquidityPool` proposal and every
`ConfigureAssetOracle` proposal for a `PriceKey::Token` key, and rejects a
failing call or a mismatch with `InvalidAsset` (6). It compares against the
stored oracle's `asset_decimals` (through `resolve_oracle`), or against the
live decimals while the price aggregator holds no oracle for the token. The
aggregator rejects a replacement oracle that changes the stored
`asset_decimals` with `InvalidOracleDecimals` (221)
([errors](../reference/errors.md)). Oracle maintenance thus keeps listed
decimals after a relabel, and a second-hub listing must use them too.

With no oracle held, live decimals apply to every such proposal, in either
order, including after a `SetPriceAggregator` re-point to an empty registry.
Before execution, the operator must:

1. Compare the resolved `asset_decimals` of every pending listing and oracle
   operation for that token with each other.
2. Compare them with the existing pools of that token.
3. Cancel the operation on any mismatch.

After a mismatch executes, only an aggregator Wasm upgrade can correct the
stored unit: `set_oracle` rejects a decimals change, and `remove_oracle`
exists only in testing builds.

**Receipt-tax tokens.** Never list a receipt-tax or otherwise
under-delivering token as borrowable in a spoke holding collateral below
`MIN_BORROWABLE_ASSET_DECIMALS`. A short receipt against such collateral
reverts, so that borrower can only be fully closed by a fully delivered offer,
never partially liquidated.

**Shared custody.** A token listed in several hubs shares pool custody
across separate books ([ADR-0002](decisions.md#adr-0002)); donations do not
rewrite the books. Cash flash loans need exact balance transitions and
allowance repayment, so receipt-tax compatibility elsewhere does not imply
flash-loan compatibility.

**Aquarius LP rewards.** Venue rewards accrue to the holder, the pool, which
cannot claim or forward them. In a mainnet simulation the reward claim needed
no holder authorization, so anyone can push accrued rewards into the pool as
an unbooked donation. Gauge rewards need the pool's authorization and are
unclaimable. LP suppliers get no venue rewards: forgone yield, not lost
principal.

**Curve LP pricing.** Curve LP collateral is priced at the pool's virtual
price times the lowest coin price. On an imbalanced pool the factory admin's A
ramps move the mark either way, within the pool's rate limits and never above
the normalized balances per share times the lowest coin price. The aggregator
pins the pool executable, not its factory, so governance must trust the
factory admin and each coin's issuer; the
[price-aggregator README](../../contracts/price-aggregator/README.md#admission-policy)
lists these assumptions. The lowest-coin price understates an LP held as debt,
so LP sources are collateral-only and LP markets must not be borrowable. No
contract enforces this rule. It is an operator guard:
[`configs/script.sh`](../../configs/script.sh) refuses a borrowable LP listing
in `refuse_borrowable_lp` and in `validateConfigs`.

## Routes, callbacks, and external integrations

Router swaps settle measured balance changes
([ADR-0011](decisions.md#adr-0011)); the controller checks positive output and
final account risk, not slippage. A compromised router may take authorized
input for dust output while the account passes its gates; routed funds and
those gates bound the exposure.

That bound covers only the controller's own grant. The router calls whatever
pool and token addresses its payload names, with no allowlist, so a route can
put third-party code below the caller's authorization. That code can transfer
tokens from the caller; an honest simulation records the transfer as a child
of the caller's authorization entry, and it executes if the caller signs the
tree. The loss is then the caller's wallet, unbounded by the payload minimum
or the final risk gate. An honest swap strategy gives the caller no child
entry. A direct router swap gives one input transfer. When the sender's
output trustline is missing, it also gives `token_out.trust(sender)` on the
Stellar Asset Contract. A client must decode the route, allow only those
exact children, and refuse every other child. The direct `execute_strategy`
path has the same exposure for every swap user.

Flash position ([ADR-0020](decisions.md#adr-0020)) leaves fee-free debt that
an already healthy account can back with little added collateral. Refunds
cover only positive callback deltas of refund-listed tokens and declared
collateral is supplied; neither sweeps prior balances. The debt market's
flash-loan flag also gates flash position.

Approved Blend pools keep their external upgrade trust until approval is
removed. DeFindex vault authentication isolates account bindings, but the
adapter has no recovery path for arbitrary stranded assets.

Protected monetary entrypoints reject guarded callback reentry
([INV-FLASH-02](../reference/invariants.md#inv-flash-02)), and Soroban
restricts indirect reentry. A callback exploit needs a reachable host call
path, which a native fixture does not prove. Risk views lack the monetary
guard; do not treat intermediate risk reads as a transaction-stable oracle.

## Price integrity and availability

Both configured price legs must be usable and agree within tolerance
([ADR-0004](decisions.md#adr-0004)). A single-source key has no top-level
agreement check, though its transitive source may have several dependencies.
Sanity bands constrain accepted prices, not economic correctness.

A single-source feed can report any price `p` in its band `[min, max]`, so two
reports satisfy `p1 / p2 <= u = max / min`. The 10% single-source cap gives
`u <= 11/9`, about 1.222. The true NAV `P` is also in the band, so
`P / p <= u`.

Lender safety depends only on reported collateral prices: bad debt occurs only
when reported collateral is below the debt. Take one collateral leg with `n`
units and a debt `D`. A borrow at reported price `p1` gives
`D <= LTV * n * p1`. Bad debt at a later report `p2` needs `n * p2 < D`. Both
need `p1 / p2 > 1 / LTV >= 1 / LT`. When `LT < 1 / u`, `1 / LT > u`, and the
band forbids this ratio. An account healthy at report `p1` has
`D <= LT * n * p1`, so the result also applies from that report. A liquidation
does not decrease units held per unit of debt, so it holds after a
liquidation too. Lenders thus cannot lose while `LT < 1 / u`; the threshold is
the band ratio `u`, not the price deviation. The Liqvid hub (LT 60% or 53%,
`u` at most 11/9) has a large margin.

Limits: one collateral leg, debt at true value (a debt feed moving in its own
band makes the effective `u` the product of both ratios), and no interest
accrued after the borrow. After a band change, the result applies again only
from a new-band report at which the account is healthy.

Admission does not enforce this envelope: listing validation checks only
`LTV < LT <= 1` and `LT * (1 + bonus) <= 1`, and oracle admission reads no
listing. Check each collateral and debt pair of a spoke against each provider
contract. A provider's reach on a key is the largest ratio between two
accepted prices of that key with every other provider fixed:

- On a single-source key, the reach is the band ratio.
- On a two-source key where the provider serves one leg, the upper tolerance
  ratio, which bounds the midpoint move.
- On a scaled source (factor times quote), a provider serving the factor feed
  reaches `max_factor_wad / min_factor_wad` times its reach on the quote key.
- A two-source key whose legs have different provider sets is exempt from the
  10% single-source band cap, so its band does not bound reach to 11/9. If a
  provider serves both legs, the factor range of each scaled leg it feeds
  must stay within that 10% cap. The two legs only have to agree within the
  tolerance ratio, and the price is their midpoint. The provider's reach is
  therefore up to `max_factor_wad / min_factor_wad` times the upper tolerance
  ratio. A factor range of 11/9 with an upper ratio of 1.25 gives 1.53x. The
  live SolvBTC and xSolvBTC keys (factor range 1.03, upper ratio 1.05) reach
  about 1.08x.

A provider serving both a collateral key and a debt key moves both at once;
lenders are safe from it only while
`LT * reach(collateral) * reach(debt) < 1`.

`make ops-script-check` runs `scripts/check_lender_envelope.py`, which applies
this condition with a 1% margin to every enabled listing in `configs/mainnet`.
A pair that still breaks it needs a line in
`scripts/lender_envelope_exceptions.txt` naming its tracking audit finding
and largest accepted product. The check also fails when a declared pair rises
above that product or sits more than 0.001 below it.

Priced lag is a second path to an accepted price above the market. A leg
reads its last written price until its next write. After a market move, a
two-source key serves the midpoint of a leg that has printed the move and a
leg that has not. Both legs pass their own age checks. The age-spread check
applies only when both legs are `Market`. It compares the newest sample of
each leg, so a `Twap(records)` leg counts from its newest round. A spot leg
can lag that round by at most `MAX_LEG_AGE_SPREAD_SECONDS` (3600 seconds).

Let `U` be the upper tolerance ratio. Let `n` be the number of samples in the
leg that prints the move: `records` for `Twap(records)`, 1 for a spot leg. One
sample holds the new price; when every sample holds it, use `n = 1`. Let `g`
be the price of the other `n - 1` samples over the price of the late leg. The
band bounds the mean of the samples against the late leg. It does not bound
`g`. The served price over the market price is:

```text
fall:  (1 + 1/U) / (2 * (n/U - (n - 1) * g))
rise:  (1 + U) / (2 * (n * U - (n - 1) * g))
```

With `U = 1.10` and equal legs before the move (`g = 1`), the fall ratio is
1.3125 for `Twap(3)` and 1.05 for a spot leg. With `g = U` it is 1.81 for
`Twap(3)`. The ratio has no upper bound: after a rise that the late leg did
not print, the older samples sit above `U` while a fall brings the mean back
inside the band. When the fall denominator is not positive, the band admits
any fall. The band rounds its ratio half up, so add 0.5 BPS to `U`.

A borrow or a withdrawal at a fall ratio `r` leaves bad debt when
`LTV * r > 1`. Liquidation refuses an account whose health factor at the
market price is above `1 / r`. The rise ratio `q` is below 1, so the key
understates a debt in the rising asset. That borrow leaves bad debt when
`LTV > q`.

The lag lasts until the late leg writes or leaves its `max_stale_seconds`
budget. A late leg past its budget fails the key with `PriceFeedStale`. After
the late leg writes, the key fails closed while the legs disagree
([DoS.1](#dos1-detail)). Before either leg prints the move, the key serves the
old price and no check bounds the gap. The envelope check does not cover
priced lag.

Borrowers have less protection. The bonus `b` is the curve bonus at the
reported HF, capped at `HF / LT - 1`, not the base bonus. On the default curve
(target HF 1.10, maximum bonus at HF 0.80) with LT 60%, a reported HF of 0.98
gives `b` of about 30%, not 5%. The curve 1.06/0.90/598
with LT 53% keeps `b` at or below 10%. One liquidation at reported price `p`
that pays `R` retires exactly `R` of debt. At true NAV it costs the borrower at
most `min(E, R * ((1 + b) * P / p - 1))`, where `E` is the equity at true NAV.
A full close on a whole-unit leg can add one unit at true NAV. When `b` reaches
the cap, one liquidation takes all of `E`. With LT 60%, `u = 11/9`, `P` at the
band top and `p` at the band floor, this occurs for an account at true HF
1.01. The
[oracle-deviation bound tests](../../tests/test-harness/tests/controller/liqvid_oracle_deviation_bounds.rs)
pin these bounds.

Source admission ([ADR-0014](decisions.md#adr-0014)) proves neither
independent operators nor independent upstream data; feed-nature labels are
configuration assertions. Non-LP probes can accept temporary market-condition
failures. An upstream key change revalidates dependent structure with no new
live attestation per dependent; runtime age checks remain, but admission
assumptions can break.

XOXNO aggregation follows the
[oracle reference](../reference/endpoints.md#xoxno-oracle): a quorum of fresh,
timestamp-clustered submissions, one registered signer each, gives the lower
median. Future skew is bounded at 60 seconds, not forbidden. Equal package
timestamps may replace a signer's observation. Evaluate honest quorum
participation against the accepted cluster: a threshold above half the
registered signers does not alone give an honest median when some signers are
absent. A small cluster must fit inside `max_cluster_spread_bps`, so one
signer moves it by at most that bound.

Below quorum the prior aggregate stays readable until stale. Setters do not
recompute existing aggregates; follow them with batched `recompute_feeds`. The
cluster anchor is clamped to ledger time; reads still apply their
package/write timestamp and freshness rules.

Price failure can stop liquidation as well as borrowing and withdrawal
([ADR-0005](decisions.md#adr-0005)). `quotes` can return a nonzero price with
`valid=false`; honor validity, not the price field. Lending valuations use
strict prices cached per operation context
([INV-ORACLE-03](../reference/invariants.md#inv-oracle-03)), without identical
timestamps across upstream observations.

## Bad debt and liquidation

Global pause leaves designated exit/recovery endpoints callable. Per
[ADR-0008](decisions.md#adr-0008), paused debt blocks a repayment leg that
selects it, and one `no_seize` flag on a nonzero planned collateral leg can
abort a pro-rata liquidation without stopping new supply. Interest continues
during a listing pause. Missing prices and liquidity limits block even
non-pause-gated entrypoints.

Liquidators bear execution-time bonus, rounding, and route-quality risk. A
plan that seizes nothing refunds every repayment leg with a positive USD
value, and the call fails with `InvalidPayments` (16) when no leg remains.
Only a repayment leg whose USD value rounds to zero can retire debt with no
seizure. Admission does not couple collateral price, decimals, bonus and fee
to the minimum-collateral floor, so expensive low-decimal collateral can seize
zero even for a $5 repayment. If the whole plan seizes nothing, that
liquidation fails; assess this before listing
([seizure fixture tests](../../contracts/controller/tests/positions/liquidation_math.rs)).
Share credit avoids collateral cash payout under the
[ADR-0019](decisions.md#adr-0019) receiver rules.

A solvent account whose only leg is below 3 decimals can be closed by a
one-unit sale whose effective bonus exceeds the quoted `b`, and `bonus_bps`
shows only `b`
([effective bonus](../reference/formulas.md#effective-bonus-of-a-rule-1-full-close)).
An offer backing less than one unit still seizes nothing, so the plan refunds
it and the call fails with `InvalidPayments` (16). Listing review must
note that expensive units with a low liquidation threshold raise the
borrower's loss.

Planning and measured settlement enforce liquidation accounting bounds
([INV-LIQ-02](../reference/invariants.md#inv-liq-02)). The absence of a final
health-factor assertion is not itself an attack. A finding must show the
arithmetic and a reachable sequence.

Cleanup converts all remaining account supply to protocol revenue and
socializes its gross debt without netting same-market pairs
([ADR-0021](decisions.md#adr-0021)). Suppliers present at cleanup bear the
index write-down; one who exits first avoids it. The index floor can leave
unpaid backing ([ADR-0012](decisions.md#adr-0012)), so displayed claims do not
guarantee pro-rata cash payout. New supply checks backing; recapitalization
repairs the measured shortfall without minting shares.

A liquidation leaving collateral at or below $5 with debt above it runs this
cleanup in the same call, after seizure. A `Credit` receiver credited in a
written-down market is a supplier present at cleanup, so its shares lose the
same fraction as every claim there; `get_liquidation_estimate` share counts
and `LiquidationEvent.bonus_bps` show the plan before that. In a shallow
market a seize-all `Credit` liquidation can thus lose money where `Transfer`
would not. Use `Transfer`, or a partial size leaving more than $5 of
collateral so no cleanup runs.

An insolvent account whose collateral-backed quote `floor(C / (1 + base))` is
below one base unit of every debt leg cannot be liquidated: the trim drops
every leg, and `liquidate` reverts with `InvalidPayments` (16). With `C` above
$5, only the owner's force-socialize path clears it. Any borrowable token
whose base unit is worth more than `$5 / (1 + base)` (about $4.76 at a 500 BPS
base bonus: a 3-decimal token above about $4,762, a 4-decimal token above
about $47,620) can create such accounts; listing review must note it.

The compile-time dust cleanup threshold and the governed collateral floor can
diverge. Above the permissionless threshold, an insolvent account may need the
[force-socialize runbook](../reference/runbooks/force-socialize-bad-debt.md),
which requires ceil risk debt above half-up unweighted collateral; an account
below that is ineligible even with HF below one and liquidation blocked by
flags.

## Numeric and resource limits

RAY value capacity can run out before the index ceiling. At the market value
ceiling accrual holds interest instead of failing, never blocking a
risk-reducing operation, but that market's borrowers accrue less than the rate
implies ([INV-IDX-06](../reference/invariants.md#inv-idx-06)). Entries stop a
factor 8 earlier, at the entry ceiling and admitted cap maximum
([INV-ACCT-11](../reference/invariants.md#inv-acct-11)). Caps must still allow
for plausible index growth as well as token balances. Accrual cadence changes
utilization and later rates; bounded chunks neither make cadence neutral nor
prove exact conservation after integer rounding. See
[numeric limits](../reference/formulas.md#numeric-limits).

Position and route limits reduce work but do not prove every maximum-size
operation fits CPU, memory, footprint and oracle-call budgets;
caller-supplied vectors still cost resources. A full-risk threshold refresh
aborts the whole batch only when it lowers an account's HF below both 1.05 and
its pre-refresh value, so an account already below 1.05 does not abort it. An
LTV-only refresh has no final gate.

Curve LP sources take at most 3 coins (`MAX_CURVE_COINS`). Under the default
Soroban budget with mainnet per-invocation limits, the
[Curve collateral tests](../../tests/test-harness/tests/curve_collateral.rs)
measured 54.87M CPU and 11.19M memory to liquidate `POSITION_LIMIT_MAX` (5)
3-coin LP legs, 25.6M CPU for a multiply into LP, and 29.0M CPU for a repay out
of LP. These are lower bounds: the controller runs natively, so its own
instructions are not counted, and the tests use a native feed, SAC coins,
single-source coin oracles and one debt leg (the limit is 5).

The WASM replay `curve_lp_liquidation_worst_case_all_wasm` closes those gaps:
all contracts run as WASM, coins are custom-WASM tokens with two-source WASM
feeds, and the account holds 5 LP and 5 debt legs. Its heaviest case, a
`Transfer` seizure 5 seconds after the last accrual, used:

| Resource | Used | Mainnet limit |
|---|---:|---:|
| CPU instructions (`tx_max_instructions`) | 107.7M | 400,000,000 |
| Memory bytes (`tx_memory_limit`) | 15.1M | 41,943,040 |
| Footprint entries (`tx_max_footprint_entries`) | 212 | 400 |
| Disk read entries (`tx_max_disk_read_entries`) | 0 | 200 |
| Write entries (`tx_max_write_ledger_entries`) | 42 | 200 |

Limits are the mainnet values `stellar network settings` reported; the test
asserts all eight fixture ceilings in `MAINNET_LIMITS`. Both feed providers
(RedStone primary, Chainlink anchor) are `mock_redstone.wasm`, and every coin
and debt asset is the minimal `production_fixture.wasm` token, so real
contracts can cost more: re-measure with the production RedStone adapter,
Chainlink proxy and listed token executables before treating 107.7M as a
production bound. Larger pools need new code and measurements.

## STRIDE register

STRIDE: spoofing, tampering, repudiation, information disclosure, denial of
service, elevation of privilege. Each stable ID maps a scenario to its control
boundary; the assumptions above apply, and rows assign no severity or
exploitability.

| ID | Threat and control boundary |
|---|---|
| Spoof.1 | Compromised privileged key acts within its role; controls: delay, cancellation, restricted roles, custody. |
| Spoof.2 | Wrong deployment identity; helpers wire authority atomically; verify standalone arguments and later changes. |
| Spoof.3 | Former or unlisted delegate; grant, stamped-owner, and active-manager checks apply, including return-to-owner revival. |
| Spoof.4 | Non-contract flash receiver; contract-address validation and path-specific repayment/finalization apply. |
| Spoof.5 | Impersonated vault; authenticated vault addresses key separate adapter accounts. |
| Tamper.1 | Manipulated or unavailable prices; freshness, sanity, and source agreement do not prove source honesty. |
| Tamper.2 | Dishonest signer subset; the median uses the fresh accepted cluster, whose honest participation is assumed. |
| Tamper.3 | Non-standard tokens or donations; measured receipts and separate books do not neutralize arbitrary token semantics. |
| Tamper.4 | Malicious router or venue; measured input/output bounds apply; the router enforces the payload minimum output. |
| Tamper.5 | Callback reentry or intermediate state; guards and host rules protect reachable paths, not hypothetical EVM behavior. |
| Tamper.6 | Accrual manipulation or extremes; bounded indexes and chunks, with cadence effects. See [Tamper.6 detail](#tamper6-detail). |
| Tamper.7 | Malicious upgrade; core upgrades, price aggregator included, are delayed; standalone owners keep their own upgrade policy. |
| Tamper.8 | Hostile Blend integration; allowlisting and measured settlement do not freeze an approved pool's code. |
| Tamper.9 | False feed-nature label bypasses intended smoothing selection; nature is operator-asserted. |
| Tamper.10 | Upstream configuration changes after admission; structural checks and age limits remain; monitor admission assumptions. |
| Repudiate.1 | Denied admin actions; inherited governance lifecycle, role and ownership events exist; application events are partial. |
| Repudiate.2 | Misread user or liquidation actions; use measured repayment and distinct gross/net batch tags, not one-batch assumptions. |
| Repudiate.3 | Oracle event gaps; submission and configuration lack dedicated events; inherited ownership events remain. |
| Info.1 | Positions, prices and governance state are public; no confidentiality. |
| Info.2 | Liquidation competition and MEV; the bonus curve bounds terms, not ordering or liquidator profit. |
| Info.3 | Visible pending governance changes allow anticipatory positioning; observability is intentional. |
| Info.4 | Invalid quotes may contain prices; use validity flags or strict reads. |
| DoS.1 | Price outage blocks valuation-dependent actions, liquidation included (fail-closed cost). See [DoS.1 detail](#dos1-detail). |
| DoS.2 | Selected paused debt or no_seize collateral blocks liquidation. |
| DoS.3 | A false-alarm pause needs delayed reopening; emergency response is asymmetric. |
| DoS.4 | Lost governance keys; proposer safeguards and owner-dependent canceller recovery do not restore a lost owner. |
| DoS.5 | Resource exhaustion; limits do not replace real-WASM maximum-case measurements. |
| DoS.6 | Archived storage blocks access until restoration; controller renewal needs the owner. |
| DoS.7 | Signer liveness loss stops new aggregates; a prior aggregate serves until stale. |
| DoS.8 | Cash, utilization and cap limits reject otherwise desired actions. See [DoS.8 detail](#dos8-detail). |
| DoS.9 | Dust and zero-share movements; rejections and floors reduce griefing but do not guarantee liquidation profit. |
| DoS.10 | Router or oracle owner loss disables administration. See [DoS.10 detail](#dos10-detail). |
| DoS.11 | Receipt-tax debt token against collateral below 3 decimals blocks partial liquidation. See [DoS.11 detail](#dos11-detail). |
| Elevation.1 | Governance-owner compromise; configured delay and approved replacement code set the exposure. |
| Elevation.2 | Guardian attempts reopening; immediate flag ratchets reject it. See [Elevation.2 detail](#elevation2-detail). |
| Elevation.3 | Role overlap or cancellation abuse; separation exempts the owner, and recovery has its own rules. |
| Elevation.4 | Delegate exceeds mandate; owner-only grants and delayed global manager deactivation limit eligibility, not economic intent. |
| Elevation.5 | Third party creates foreign risk; supply top-ups need existing positions, and account creation belongs to its caller. |
| Elevation.6 | Router owner upgrades, sweeps or changes fee rights; no lending-governance route delays it. |
| Elevation.7 | Oracle owner changes code or signers; source comparison and bands constrain accepted prices, not all manipulation. |
| Elevation.8 | Test powers in release WASM; feature and artifact checks cover only their build and export scope. |
| Elevation.9 | Vault abuses adapter authority; the adapter exposes supply and withdrawal, not borrowing or other vault accounts. |
| Elevation.10 | ORACLE narrows a band to exclude the market price; immediate fail-closed denial of service remains possible. |

### Tamper.6 detail

At the market value ceiling, accrual holds interest instead of overflowing and
under-charges that market's borrowers; see
[Numeric and resource limits](#numeric-and-resource-limits) and
[INV-IDX-06](../reference/invariants.md#inv-idx-06).

### DoS.1 detail

- A supply into a debt-free account needs no price.
- On an indebted account, a new collateral leg needs its price within the
  feed's staleness limit. A halted feed that is not yet stale still reads.
- On an indebted account, a supply into an existing leg can price every leg,
  through the listing refresh gate below.
- Borrowing prices every leg.
- `repay` is permissionless and reads no price.

Each supply position stores its LTV, LT, base bonus and liquidation fee,
copied from the listing at creation. Supplying the asset refreshes them from
the live listing (`merge_supply_leg` calls `refresh_supply_risk_params`).
`apply_gated_liquidation_params` gates liquidator-favouring changes (lower LT,
higher bonus, lower fee): on an indebted account it applies them only if HF
with the new LT is at least 1.05 (`THRESHOLD_UPDATE_MIN_HF_RAW`); otherwise
the position keeps its old terms without failing. Computing that HF reads all
the account's prices, so a stale or out-of-band price fails the call, supply
included.

An indebted borrower can add a dust leg of any listed collateral before an
outage to pick which feed outage shields the account. A dust debt leg of the
outage asset shields too, but must be opened while the feed is live, since
borrowing prices every leg. Anyone can repay it during the outage to remove
the shield before liquidating, unless the listing is paused.

LP legs (Aquarius or Curve) add outage causes:

- Liquidity providers can withdraw pool value below `min_pool_value_wad`;
  set that floor well below pool TVL.
- `min_pool_value_wad` is a USD value (WAD), not a token amount. A fall in a
  coin price lowers the pool value and can trip the floor without any
  withdrawal. On a Curve pool, the lowest coin price sets that value. Size the
  floor for crash scenarios.
- A Curve leg fails when any coin cannot be priced, a coin's `decimals()`
  read fails or changes, an A ramp lowers pool value below the floor, or the
  executable differs from the pinned hash (a pool-admin upgrade does this
  until governance reviews and re-pins it).

The same failing leg blocks bad-debt cleanup and force-socialization.

### DoS.8 detail

- Zero caps admit no new exposure.
- A Transfer liquidation burning a market's last supply shares while it has
  debt reverts with `PoolInsolvent` (123), unless the leg's protocol fee books
  revenue shares first; `Credit` mode, a smaller Transfer seizure, or repaying
  that debt avoids it ([INV-ACCT-09](../reference/invariants.md#inv-acct-09)).

### DoS.10 detail

Existing oracle signers may continue, but repair powers are lost. Source ABIs
export no `renounce_ownership`; verify deployed artifacts match.

### DoS.11 detail

A short receipt reverts the liquidation, so only a fully delivered full close
settles. The rule under [Token assumptions](#token-assumptions) forbids the
pairing.

### Elevation.2 detail

Under the [ADR-0007](decisions.md#adr-0007) ratchet, only a delayed
epoch-bound relaxation clears a flag, and a no-op guardian write reverts
without advancing the epoch, so a guardian invalidates a pending relaxation at
most once per flag. The owner's immediate `revoke_role_immediate` stops a
guardian that keeps tightening.

## Review triggers

Recheck these boundaries after upgrades, new entrypoints, providers or venues,
authorization, storage or SDK changes, and governance or configuration
updates. Watch transactions and state, not only events; no event-only model
covers all administration. Tests, formal rules and static checks cover the
source tree; deployed artifacts, live roles, external contracts and network
budgets need the deployment checks above. [Reading a Sunbeam
verdict](../../certora/README.md#reading-a-sunbeam-verdict) states the scope of
the formal rules.
