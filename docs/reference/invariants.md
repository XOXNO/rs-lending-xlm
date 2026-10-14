# Runtime invariants

These properties describe the constraints enforced by protocol operations and
the limits of those checks. Use them with the [architecture](architecture.md),
[threat model](../explanation/threat-model.md) and [exact arithmetic](formulas.md).

Runtime checks, tests and formal specifications provide different evidence; a
listed invariant is not a completed proof. Deployment wiring, configuration,
external contracts and archived-state recovery require integration verification.

## Authority

<a id="inv-auth-01"></a>
<a id="inv-auth-01--one-ownership-chain"></a>

### INV-AUTH-01 — Pool mutation requires its owner

The repository deployment path makes governance the controller owner and the
controller the pool owner. Every pool accounting mutator authenticates the pool
owner.

The pool constructor accepts an administrator argument, so deployment must
establish that chain. Authorized controller ownership changes and code upgrades
can change the wider authority boundary.

<a id="inv-auth-02"></a>
<a id="inv-auth-02--risk-reducing-authority-is-explicit"></a>

### INV-AUTH-02 — Account spending authority is explicit

Borrowing and withdrawal require the current NFT owner or an active position
manager delegated by that owner. Delegates can choose external payout recipients,
giving them broad economic control within the account's risk limits. Only the
owner can grant or revoke delegation.

A grant records the owner's address, without a transfer epoch. It is inactive
under another owner but can reactivate if the NFT returns, unless an intervening
owner overwrites or purges the grant.

<a id="inv-auth-03"></a>
<a id="inv-auth-03--permissionless-actions-do-not-create-foreign-risk"></a>

### INV-AUTH-03 — Third-party supply cannot create a foreign asset slot

Authenticated third parties can repay, liquidate, recapitalize and perform
permissionless maintenance. Supply to an existing foreign account can only top
up supply assets already held; it cannot create a new asset slot.

Policy refresh and liquidation have separate gates. Permissionless access does
not imply that every operation preserves health or collateral value.

<a id="inv-auth-04"></a>

### INV-AUTH-04 — Emergency power only tightens

Immediate guardian power can pause and add listing restrictions. Unpausing or
clearing restrictions requires a timelock under the repository wiring. Every
guardian pause advances the governance emergency epoch, so an `Unpause`
proposed before a later pause reverts.

A listing edit and the guardian flag method can only keep or tighten flags.
Only the timelocked `relax_spoke_asset_flags` clears one, and only while the
listing's flags epoch equals its `expected_epoch`. An empty flagged listing
can be removed, since an absent listing admits no entry; every guardian flag
write advances the token's governance flag epoch, so an `AddAssetToSpoke` of
the token proposed before it, in any spoke or hub, reverts. The oracle role's
immediate `set_sanity_band` can only keep or narrow a band, and a
`ConfigureAssetOracle` proposed before it reverts
([INV-AUTH-07](#inv-auth-07)).

The guardian's immediate `exclude_account_asset_valuation` is a restriction of
the same kind. It removes one supply leg of one named account from that
account's valuation, and only while the asset's strict price read already
fails with a feed outage ([INV-HALT-04](#inv-halt-04)). A sanity-band
violation does not qualify, so the oracle role's immediate band narrowing
cannot be combined with it to zero a priced leg. Only the timelocked
`restore_account_asset_valuation` clears it, bound to the exclusion's ledger
stamp and only once the price is usable again.

<a id="inv-auth-05"></a>
<a id="inv-auth-05--governance-delay-cannot-be-shortened"></a>

### INV-AUTH-05 — Delay updates cannot shorten the minimum

Construction requires a nonzero minimum delay. Delay updates can retain or
increase it, up to 241920 ledgers; construction does not enforce that upper
bound. Only the owner can propose `UpdateGovDelay`. Sensitive and Recovery
delays also apply their compiled minimums.

These checks govern the deployed implementation and cannot constrain replacement
Wasm.

<a id="inv-auth-06"></a>

### INV-AUTH-06 — An account's spoke binding is immutable

An account retains its creation-time spoke. Existing-account supply, migration,
multiply and flash-position calls must match that spoke; borrow, withdraw and
repay use the stored binding. Credit-mode liquidation requires a receiver in
the same spoke.

<a id="inv-auth-07"></a>

### INV-AUTH-07 — A queued operation executes only under the state it was proposed under

Every governance proposal records a guard, and execution reverts while the
operation is pending if a counter the guard bound has moved since:

| Operation | Bound to | Advanced by | Error |
| --- | --- | --- | --- |
| Every proposal | Proposer epoch of its proposer | `revoke_role_immediate` or executed `RevokeGovRole` of `PROPOSER` from that address; `accept_ownership` to another address, for the previous owner | `NotAuthorized` |
| Canceller reset | Owner epoch | `accept_ownership` to a different address | `NotAuthorized` |
| Canceller reset, each listed account | `CANCELLER` revocation epoch of that account | Executed `RevokeGovRole` of `CANCELLER` from that account | The account is not re-seated; the rest of the list is |
| `Unpause` | Emergency epoch | Guardian `pause`, executed `UpgradeController` | `EmergencyEpochMismatch` |
| Every upgrade | Upgrade epoch of the contract it replaces | Every executed upgrade of that contract | `UpgradeSupersededAfterProposal` |
| `AddAssetToSpoke` | Flag epoch of its token | Guardian `set_spoke_asset_flags` on that token, any spoke or hub | `AssetFlagsChangedAfterProposal` |
| `GrantGovRole` | Revocation epoch of its account and role | `revoke_role_immediate`, executed `RevokeGovRole` of that pair | `RoleRevokedAfterProposal` |
| `ConfigureAssetOracle` | Band epoch of its key, and the band it read | `ORACLE` `set_sanity_band` on that key | `OracleBandChangedAfterProposal` |
| `SetPriceAggregator` | Any-key band epoch | `ORACLE` `set_sanity_band` on any key | `OracleBandChangedAfterProposal` |
| `EditAssetInSpoke` a non-owner proposed | The listing's liquidation threshold, bonus and fees | Any change to them | `ListingTermsChangedAfterProposal` |
| `RestoreAccountAssetValuation` | The exclusion's ledger stamp, carried in its arguments and checked by the controller | Every new exclusion of that account and leg | `SpokeAssetExclusionUnchanged` |
| `TransferGovOwnership` with deadline 0 | Nomination epoch | Every nomination | Completes as a no-op |
| `TransferCtrlOwnership` with deadline 0 | Controller nomination epoch | Every executed controller nomination | Completes as a no-op |

The counters only increase. The owner always holds `PROPOSER`, so the owner's
own proposals die exactly when it hands ownership to another address, and
operations another proposer queued survive a handover. An operation with no
guard record was scheduled before the governance upgrade that added guards
and is held to the owner epoch and the counter its kind binds today at 0,
the value every counter had at that upgrade; nothing binds it to its
proposer or to listing terms; a canceller reset without a record holds each
listed account to revocation epoch 0. A canceller reset voided by a handover
can be cancelled, but a live one cannot, so it carries the revocation binding
itself: it cannot re-seat a canceller the council removed while it waited. The guard binds the queue, not state: an operation executed before
the binding moves keeps its effect.

<a id="inv-auth-08"></a>

### INV-AUTH-08 — A reopening never runs ahead of a pending controller upgrade

An `Unpause` reverts `ControllerUpgradePending` while any `UpgradeController`
proposed since operation guards exist is pending, waiting or ready, voided or
expired included, until it executes or a canceller cancels it. An executed
controller upgrade advances the emergency epoch, so a reopening proposed
before it reverts too; reopening on the new code takes a proposal after the
upgrade executes. Every upgrade is bound to the upgrade epoch of the contract
it replaces, so a build proposed before another upgrade of that contract
executed never lands after it. Operations carry no predecessor: ordering
covers these two pairs, not arbitrary dependencies
([ADR-0022](../explanation/decisions.md#adr-0022)).

<a id="inv-auth-09"></a>

### INV-AUTH-09 — Liquidation terms and the canceller council belong to the owner

Only the owner can propose the spoke liquidation curve, forced
socialization, position-manager activation, and a listing edit that lowers
the live liquidation threshold, raises the bonus or lowers the fees. A
non-owner listing edit executes only while the listing still holds the
liquidation terms it was proposed against, so neither a later owner change
nor the proposer's own executed edits can be walked back by it. The owner can
revoke `PROPOSER` and `EXECUTOR` immediately, and a revoked proposer's whole
queue is void. `CANCELLER` keeps its timelocked removal. The council holds at
most `MAX_CANCELLERS` (24) members, the owner included, so a canceller reset
of a full council stays inside one transaction's mainnet limits
([ADR-0023](../explanation/decisions.md#adr-0023)).

## Accounting and cash

<a id="inv-acct-01"></a>
<a id="inv-acct-01--supply-revenue-and-debt-shares-are-non-negative"></a>

### INV-ACCT-01 — Revenue shares remain part of total supply

Supply, debt and revenue share totals remain non-negative, and revenue cannot
exceed total supply. Share subtraction rejects negative amounts and insufficient
balances.

Revenue minting increases revenue and total supply equally; revenue claims burn
both equally. Reclassifying seized supply increases revenue without increasing
total supply. This relationship between shares does not establish that every
claim is backed by cash and collectible debt.

<a id="inv-acct-02"></a>

### INV-ACCT-02 — Cash is the reserve book

Reserve checks use tracked market cash; token donations alone do not increase
it. Cash credits and debits reject negative amounts. Credits reject overflow,
and debits reject insufficient reserves. Outbound token transfer and cash
bookkeeping remain separate actions.

<a id="inv-acct-03"></a>
<a id="inv-acct-03--credit-equals-measured-receipt"></a>

### INV-ACCT-03 — Token-funded credit uses measured receipt

Token-funded supply, repayment and recapitalization credit the pool's measured
balance increase. Strategy supply follows the same supply path, and strategy
repayment also measures pool receipt. Requested amounts alone do not determine
credit.

Repayment and recapitalization can refund unused receipt. Share conversion
retains its own rounding, and measurement depends on the token's balance
reports. Liquidation Credit mode moves existing collateral shares without
incoming collateral tokens.

<a id="inv-acct-04"></a>

### INV-ACCT-04 — Backing shortfall blocks new supply

New token-funded supply rejects a positive backing shortfall. The check
subtracts ceiled debt value from floored supplied claims at RAY precision,
floors the difference once to native token units, then subtracts tracked cash.
The shortfall is the floor of that RAY-precision gap.

Recapitalization fills at most that shortfall, refunds excess and mints no
shares; a full fill leaves the RAY-precision gap below one native unit. Accrual adds the
same interest to claims and debt only up to RAY-precision rounding, up to two raw
RAY units per chunk, so a fill whose residue sits within those units of a whole
unit can report a one-unit shortfall again after accrual (about 2e-9 per fill at
18 decimals, 2e-20 at 7); supply then waits for a one-unit recapitalization. It
cannot restore a written-down index. The non-zero supply-index floor
can leave residual claims requiring recapitalization; see the
[backing calculation](formulas.md#backing-and-cash-constraints).

<a id="inv-acct-05"></a>
<a id="inv-acct-05--positive-value-must-change-shares"></a>

### INV-ACCT-05 — Positive position settlement requires shares

Positive supply and borrow amounts must mint shares. Positive net repayments
and gross withdrawals must burn shares; positive same-asset net settlement
must burn both supply and debt shares. Zero-share results revert at these
boundaries.

Cash credits and fees have separate rules: recapitalization mints no shares,
and protocol-reward conversion can floor to zero revenue shares.

<a id="inv-acct-06"></a>

### INV-ACCT-06 — Revenue claims respect accounting bounds

Revenue payout cannot exceed tracked cash or the floored revenue claim. While
the market carries debt it also cannot take the cash the 200 BPS liquidation
buffer keeps ([INV-ACCT-07](#inv-acct-07)), and a payout that would raise
utilization past `min(max_utilization, 95%)` (`MAX_CLAIM_UTILIZATION_RAY`) pays
nothing. Full payout burns all revenue shares; a partial payout burns a
proportional ceiling of shares. Positive payout cannot burn zero shares.

A claim never reverts for want of claimable cash: it pays less, or nothing. A
claim that pays nothing burns nothing, moves no cash and skips the utilization
and supply-for-debt checks, so a controller claim batch never fails over a
market it cannot pay. A positive payout still enforces both checks. These
checks do not establish full backing, and claims small enough to fit under the
bound still pay revenue accrued on interest an uncleaned insolvency never pays.
The [payout formula](formulas.md#revenue-payout) defines the calculation.

<a id="inv-acct-07"></a>
<a id="inv-acct-07--borrow-draws-leave-a-liquidation-cash-buffer"></a>

### INV-ACCT-07 — Borrow draws reserve a cash buffer

Borrow debt minting requires cash after the requested draw to cover 200 BPS of
the floored supplied token value, rounded up. Strategy debt openings check
gross principal before withholding any fee.

Exits do not preserve this buffer. It does not guarantee enough cash for every
subsequent liquidation.

<a id="inv-acct-08"></a>
<a id="inv-acct-08--utilization-stays-below-the-market-ceiling"></a>

### INV-ACCT-08 — Selected operations enforce the utilization ceiling

Borrow debt minting, ordinary withdrawal and paying revenue claims reject
utilization above the market ceiling; a paying claim also stays at or under
the 95% claim bound, and a claim that would cross either pays nothing instead
of reverting. The gate divides ceiled debt value by floored supply
value and rounds the ratio up, so rounding cannot admit utilization above the
ceiling. It skips zero total supply, zero debt value and ceilings at least one
RAY. Debt against a zero floored supply value fails the gate.

Liquidation withdrawal and a claim that pays nothing skip the gate. Accrual and
bad-debt writeoff can exceed the ceiling; it is not a market-wide bound
maintained by every operation.

<a id="inv-acct-09"></a>
<a id="inv-acct-09--exits-cannot-leave-debt-without-supply"></a>

### INV-ACCT-09 — Debt mints and selected exits cannot leave debt without supply

Borrowing, strategy debt mints, withdrawal, same-asset net settlement and
paying revenue claims reject a resulting zero total supplied-share balance with
non-zero debt shares. A debt mint into a market whose suppliers have all exited
therefore fails, even when rounding left cash behind. A paying claim stays
inside the claim utilization room, which keeps supply shares under the debt, so
for claims the check is defence in depth; a claim that pays nothing burns
nothing and skips it. This prevents an empty-supply state, without establishing
full backing or a liquidation cash reserve.

<a id="inv-acct-10"></a>

### INV-ACCT-10 — Account books reconcile with pool totals

The pool stores market totals only. Every pool position call carries the
account's scaled position, and the pool trusts it: it has no per-account book to
check the value against. It checks only the sign: every leg (supply, borrow,
withdraw, repay, strategy opening, net settlement and seizure) rejects a
negative scaled position with `AmountMustBePositive` before it mints or burns.
For each market, the sum of account supply shares
equals the pool's supplied shares minus its revenue shares, and the sum of
account debt shares equals the pool's borrowed shares.

The controller holds this by construction, not by a runtime assertion. It takes
each new scaled position from the pool's returned mutation, merges duplicate
legs by hub asset before it reads a position, and moves shares between two
accounts only in credit-mode liquidation, where the debit, the credit and the
fee are asserted to sum exactly. A controller path that writes a position from
any other source, or that sends a stale position to the pool, breaks this
invariant without a revert.

The `prop_accounting_conservation` property test checks both equalities in
token units, within 4 units, after every operation on markets without seeded
liquidity. The cash-conservation tests bound cash plus debt value minus supplier
claims, with the harness seed booked as supply, so their cash bound is tight. No
on-chain view enumerates accounts, so on a live network only the spoke-level
identity (pool supplied minus the sum of spoke usage equals revenue) can be
read.

## Indexes and accrual

<a id="inv-idx-01"></a>

### INV-IDX-01 — Borrow index is monotone and bounded

Both indexes start at one RAY. Successful accrual with validated rate parameters
cannot lower the borrow index and caps it at the protocol constant 10^36 raw
RAY. At the ceiling, further accrual produces no borrower interest.

Accrual also keeps total debt value and total supply value at or below the
market value ceiling, `i128::MAX` less one whole token in raw RAY. Interest that
would carry a total past it is not charged: the borrow index grows only as far
as the room left, and a market with no room keeps both indexes. The pool emits
`MarketValueCeilingEvent` when the ceiling holds interest. Market value
therefore cannot overflow accrual or the index projection that values accounts.
Supply entry and flash and strategy fee booking reject a total above the
ceiling. See [numeric limits](formulas.md#numeric-limits).

<a id="inv-idx-02"></a>

### INV-IDX-02 — Supply index is bounded

The supply index stays within 10^24 to 10^36 raw RAY, inclusive. Interest
distribution cannot lower it; bad-debt writeoff applies the lower bound. These
are protocol constants, not per-market governance settings.

The lower bound prevents a zero conversion divisor but can preserve an unbacked
residual claim.

<a id="inv-idx-03"></a>

### INV-IDX-03 — Bad debt may lower the supply index

Debt writeoff reduces claims in the debt market through its supply index,
including revenue-share claims. The calculation uses two floors and a non-zero
index clamp. Zero supplied value leaves the index unchanged, and an index
already at its floor need not decrease.

Seizure accrues its market first. Comparing snapshots across time can therefore
hide a writeoff behind intervening interest. Each pool seizure writes only its
identified market. See the [write-down calculation](formulas.md#bad-debt).

<a id="inv-idx-04"></a>
<a id="inv-idx-04--accrual-is-time-consistent"></a>

### INV-IDX-04 — Accrual shares one bounded-step calculation

No elapsed time means no accrual or accrual-timestamp advance. Longer intervals
use forward chunks of at most 31,556,926,000 milliseconds. Mutating accrual and
read-only index projection use the same calculation, including the market value
ceiling and revenue shares in subsequent supply totals.

Each chunk selects its rate from starting utilization. Call cadence can change
rates and rounded results; it does not guarantee cadence independence,
monotonic partition results or an exact continuous-exponential bound. See
[accrual cadence](formulas.md#accrual-cadence).

<a id="inv-idx-05"></a>
<a id="inv-idx-05--accrued-interest-is-fully-assigned"></a>

### INV-IDX-05 — Interest allocation retains explicit rounding limits

At the RAY-value split, borrower interest equals supplier rewards plus the
reserve-factor fee. Supplier rewards not reflected in the supply-index change
join the protocol reward.

Revenue-share conversion floors at the new supply index and caps at remaining
total-supply share headroom. That conversion can leave reward value unrepresented;
exact conservation of booked supplier and revenue value is not guaranteed. See
[interest allocation](formulas.md#compounding-and-interest-allocation).

## Oracle validity

<a id="inv-oracle-01"></a>
<a id="inv-oracle-01--valuation-fails-closed"></a>

### INV-ORACLE-01 — Required valuations fail closed

A missing or unusable required price aborts a valuation-dependent operation,
including liquidation. Strict reads reject resolution errors, stale prices,
source disagreement, nonpositive prices and sanity-band violations. A supply leg
excluded from its account's valuation is not a required price for that
account: it is valued at zero without a read ([INV-HALT-04](#inv-halt-04)).
Debt and bad-debt cleanup always read.

Diagnostic `quotes` can retain a nonzero candidate with `valid=false`. That
candidate is not accepted for valuation.

<a id="inv-oracle-02"></a>

### INV-ORACLE-02 — A dual source requires both legs

A configured two-source price requires both usable legs. The accepted price
is their integer midpoint, rounded down, within both the input range and the
final sanity band. One surviving leg cannot serve as a fallback.

A partial reading is unusable; a stale surviving leg reports staleness before
disagreement. Separately admitted single-source configurations retain freshness,
positivity and sanity checks.

<a id="inv-oracle-03"></a>
<a id="inv-oracle-03--one-transaction-sees-one-snapshot"></a>

### INV-ORACLE-03 — A context retains fetched prices

A controller context retains each fetched asset price and requests only missing
assets on subsequent fetches. Missing cached prices fail closed. Aggregator
sessions also cache resolved keys.

These caches preserve repeated valuations within their context. They do not
provide identical observation timestamps across sources or a transaction-wide
snapshot shared by independent contexts.

<a id="inv-oracle-04"></a>
<a id="inv-oracle-04--future-dated-feeds-are-not-accepted"></a>

### INV-ORACLE-04 — Observation timestamps have a bounded future allowance

Feed timestamps beyond ledger time plus 60 seconds are discarded; the exact
boundary is allowed. Multi-feed reads check both package and write timestamps
after converting milliseconds to whole seconds. Reflector reads check each
observation timestamp.

Discarding a required leg makes its single-source or dual-source configuration
unusable under INV-ORACLE-01 and INV-ORACLE-02.

<a id="inv-oracle-05"></a>

### INV-ORACLE-05 — A shared provider moves an exempt pair at most 11/9

A registered key prices within its sanity band, and a band wider than the
single-source cap (`max / min` above 11/9) is admitted only for a pair in
which every provider contract both legs trust is bounded: through a top-level
`Scaled` factor, the factor range times the contract's reach into that leg's
quote key times the pair's tolerance is at most 11/9. The quote reach is the
quote's tolerance when the contract serves one leg of a two-leg quote, and
unbounded when it serves every leg. Every admission path holds it:
`set_oracle` validates the key and revalidates its dependents, and
`set_tolerance` does the same, so a quote edit cannot widen a dependent past
the cap. `set_sanity_band` only narrows.

An Aquarius LP key is sole-source and is held to the LP band cap instead. A
provider that serves both legs outright, or several providers acting
together, are outside this bound; the band is then the only limit.

## Account risk

<a id="inv-risk-01"></a>

### INV-RISK-01 — Risk-increasing actions re-prove solvency

Ordinary borrowing, withdrawal and account strategies apply final solvency gates
after pool mutations. With debt remaining, LTV-weighted collateral must cover
debt, health factor must be at least one, and LTV-weighted collateral must meet
any nonzero configured floor. Debt-free accounts skip these numerical gates.
Supply legs excluded from the account's valuation count as zero in these gates,
in the final 1.05 gate of a keeper threshold refresh, and in the per-leg 1.05
check that decides whether a liquidator-favoring tuple applies to a refreshed
leg. That check values the same book liquidation does, so no caller, a third
party topping up a held leg included, can stamp a tuple the account's valued
book does not carry at 1.05, and it never reads the excluded leg's price.

Listed LTV snapshots refresh before the shared gate. Listing and count checks
precede entry; spoke caps use returned pool deltas and revert the whole
transaction on failure. Supply, repayment and liquidation do not universally
apply this final gate.

<a id="inv-risk-02"></a>

### INV-RISK-02 — Conservative valuation biases safety

Collateral used for risk gates and its LTV or threshold weights round down.
Risk debt rounds up; health-factor division rounds down and saturates. LTV
weighting uses the smaller stored LTV and liquidation threshold.

Unweighted total collateral rounds half-up, including the total used for
bad-debt eligibility. Debt display helpers can also round half-up. See
[valuation and health](formulas.md#valuation-and-health) for each boundary.

<a id="inv-risk-03"></a>

### INV-RISK-03 — Risk configuration is coherent

Admitted listing configuration requires `0 <= LTV < threshold <= BPS` and
`threshold * (BPS + bonus) <= BPS * BPS`. Liquidation fees must be strictly
below BPS.

The liquidation curve requires a target health factor above one WAD and at most
ten WAD. Its full-ramp health threshold must be positive and below the target;
the bonus factor must be positive and at most BPS. These bounds validate
configuration; they do not guarantee a profitable liquidation after rounding.

A position keeps its stored liquidation tuple until a refresh copies the
listed one: a supply, a normal withdrawal, an account strategy or
`update_account_threshold` with `has_risks`. On an account with debt, a tuple
that favors the liquidator is applied only if the health factor with it is at
least 1.05, valued without the legs excluded from the account's valuation
([INV-HALT-04](#inv-halt-04)). A batch merges every balance first and refreshes its touched legs
in asset-key order, so the caller's leg order does not change the outcome. Two
residuals remain. First, each gate reads the legs not yet refreshed at their
stored tuples: when one refresh tightens a lower-key leg and loosens a
higher-key leg, the tightening is gated against the sibling's older, tighter
threshold and can be skipped although the finished state clears 1.05. The
tightened leg then keeps its stale, looser tuple until the next refresh
applies the listing; one more `update_account_threshold` call is the repair. Second, the gate is a condition
of each refresh, not of the account afterwards: single-leg calls in sequence
can stamp a tighter tuple at 1.05 and then withdraw to a health factor between
one and 1.05. Neither residual weakens the final solvency gate.

<a id="inv-risk-04"></a>

### INV-RISK-04 — Position and delegate counts are bounded

New supply and borrow slots must fit their configured counts. Existing-asset
top-ups remain allowed if a count limit falls below the account's existing
count. Credit-mode receivers obey the same new-slot limit, and an account can
have at most 16 delegates. A supply leg below 3 decimals is its account's only
supply position: a new slot beside it, by deposit or credit, is rejected.

These bounds limit state size without proving worst-case transaction-budget
sufficiency.

## Liquidation and bad debt

<a id="inv-liq-01"></a>

### INV-LIQ-01 — Only unhealthy debt can be liquidated

Ordinary liquidation requires outstanding debt and health factor below one.
The health factor counts supply legs excluded from the account's valuation as
zero, so an exclusion can make that account liquidatable on its other legs.
The liquidator authenticates but need not own the target; owners can
self-liquidate.

A Credit receiver must be a different account in the same spoke, in Normal
mode, and controlled by the liquidator as owner or active delegate. `Credit(0)`
can create a receiver in a deprecated spoke. Ownership and position limits
still apply, except that an empty receiver skips the governed limit: its
legs are already bounded by the limit that admitted them, so a lowered limit
cannot block a `Credit(0)` receiver. No credit opens a supply slot past
`POSITION_LIMIT_MAX`, the bound the per-account resource budget is sized
for. The estimate (`get_liquidation_estimate`) applies every receiver gate
except ownership, which needs the caller a view does not have, so a
`Credit` estimate fails where execution would.

<a id="inv-liq-02"></a>

### INV-LIQ-02 — Repayment and seizure stay coupled

Repayment planning caps each input by its leg's actual debt. A partial plan
also trims the excess above the liquidation curve's quote before transfers.
Planned refunds are unused input: a partial plan never pulls them; a full-debt
plan pulls the offered amount and the pool returns what exceeds each leg's
debt, which equals the planned refund. Seizure is proportional to collateral
value over the valued legs and capped at held collateral; a leg excluded from
the account's valuation is never seized. Rounding can leave repayment with no
payable seizure. A partial seizure whose floored token amount reaches the
leg's half-up balance is the pool's full close, so the plan records it as one
in both modes: Transfer and Credit agree on the close, and no leg is stepped
back or skipped while its repayment is kept. Every repayment and seizure
leg's feed decimals must equal the pool's listed decimals, at execution and
in the estimate. A collateral leg below 3 decimals seizes whole units
(integer base units, as [defined in formulas.md](formulas.md#bonus-and-target-repayment)):
rounded up when the plan repays all debt, otherwise rounded down with the unbacked
repayment refunded, and a plan left with no seizure reverts. Such a leg is the
account's only supply position, so the seizure stays proportional. On a
solvent account whose leg holds a whole unit, the partial quote can change.
When one unit's value divided by `1 + bonus` covers the whole debt plus one base
unit of each debt leg, the quote becomes the whole debt and the leg rounds up to
one unit. That full close can pay the liquidator more than the quoted bonus.
Otherwise, when the quote seizes less than one unit plus a `1e-6` margin, it
rises to the repayment that backs one unit plus the margin, if that repayment
is below the whole debt. The seizure then refunds the margin, rounded down to
whole debt-token units. Thus such an account stays liquidatable below `HF = 1`, by a one-unit
sale or by a full close. The exception is a debt in the narrow band where
neither change applies: if the curve quote backs less than one unit, every
offer reverts until accrual or a price move ends that state. See
[whole-unit legs](formulas.md#bonus-and-target-repayment).

Transfer mode burns shares and pays underlying after fees. The planned transfer
fee never exceeds the whole token units paid above the leg's principal. Credit
mode splits seized shares exactly between receiver credit and a
ceiling-rounded fee on bonus shares. The fee reclassifies existing supply as
revenue, requires no collateral cash and reduces same-spoke usage only by the
fee.

New receiver assets require a current listing. Existing receiver risk snapshots
stay unchanged. See [liquidation arithmetic](formulas.md#liquidation-sizing-and-fees)
for bonus selection, sizing and rounding.

<a id="inv-liq-03"></a>

### INV-LIQ-03 — Under-delivery reduces seizure

Receipt below planned repayment floor-scales seizure amounts, transfer fees,
seized shares and bonus shares by measured/planned USD. Credit fees are
recomputed from the scaled bonus shares. A seize-all plan on an insolvent
account is scaled the same way, so an under-delivering debt token does not
seize every unit; the undelivered share stays with the account.

Repayment USD is capped per leg at its planned value. It does not exactly equal
every rounded debt-share reduction or refund.

<a id="inv-liq-04"></a>

### INV-LIQ-04 — Bad-debt socialization is explicit and total

Permissionless cleanup requires ceil risk debt greater than half-up unweighted
collateral and either collateral at or below the fixed $5 dust threshold or a
collateral-backed quote, `floor(C / (1 + base))`, below the smallest payment
every debt leg accepts (one native unit, or the whole debt when it rounds up
to one unit), so that no liquidation offer can repay anything. Owner-only
forced cleanup omits both conditions. Both require debt, readable account and NFT
state, valid required prices and no active flash guard. Listing flags and
global pause do not block standalone cleanup. Standalone and forced cleanup
ignore valuation exclusions and price every leg. The residual cleanup at the
end of a liquidation does not run while the account holds an excluded leg.
Debt is therefore never written off against a leg valued at zero for lack of a
price.

Cleanup reclassifies all remaining collateral shares as revenue and writes off
all remaining debt against each debt's market. It releases spoke usage and
atomically removes account entries and the NFT. It does not net same-market
supply against debt. Standalone cleanup emits `CleanBadDebtEvent` with
pre-cleanup USD totals, without a controller position-update batch.

Ordinary liquidation and cleanup apply no final account-health or full-backing
assertion. The index floor can leave a shortfall. Recapitalization fills that
shortfall without restoring the lost index or deleted account.

## Pauses, flags and caps

<a id="inv-halt-01"></a>

### INV-HALT-01 — Global pause blocks new risk

Global pause blocks supply, borrowing, flash and strategy entry, delegate
grants, index updates, revenue claims and threshold refresh.

Withdrawal, repayment, liquidation, bad-debt cleanup, recapitalization, account
renewal and delegate revocation remain callable under their separate gates.

<a id="inv-halt-02"></a>

### INV-HALT-02 — Frozen, paused, and no_seize gate different legs

Listing flags act independently: entry rejects `paused` and `frozen`; user
exits and liquidation repayment reject `paused`; ordinary liquidation seizure
rejects only `no_seize`. Missing listings pass the flag helper, while entry and
new Credit receiver assets separately require a listing.

Nonzero `no_seize` collateral can block the whole proportional liquidation,
even if supplied after the flag is set. Zero-token planned seizure legs are
omitted before that check, and a leg excluded from the account's valuation is
never planned, so its `no_seize` does not block. Standalone bad-debt cleanup
bypasses these flags.

<a id="inv-halt-03"></a>

### INV-HALT-03 — Cap entry checks; scaled conversion can fail open

A zero supply or borrow cap admits no positive entry. Entry compares scaled
usage against the asset-unit cap converted at the returned live index.

That conversion saturates at `i128::MAX` instead of reverting, so the scaled
cap can fail open: once saturated, the configured asset-unit limit is not
enforced. An admitted cap saturates only at an index below one RAY. At the
supply-index floor (`RAY / 1000`), a supply cap above 1/1000 of the admitted
maximum saturates. The borrow index never falls below one RAY, so an admitted
borrow cap cannot saturate. See
[cap conversion](formulas.md#caps-fees-and-numeric-limits).

Exits consume no cap. Missing usage rows and zero deltas are no-ops, and stored
usage cannot become negative. Same-spoke liquidation credit bypasses entry
caps and books its fee as an exit.

<a id="inv-halt-04"></a>

### INV-HALT-04 — A valuation exclusion zeroes one account leg during a feed outage

`exclude_account_asset_valuation` names one account and a supply leg it holds.
It is admitted only while the aggregator's `quotes` report the asset not
`valid`, which holds exactly when the strict `prices` read fails, and only
when the failure code is a feed outage: stale price, no last price, empty or
short TWAP history, invalid price, source disagreement, or an Aquarius pool
below its floor. A sanity-band violation, which the oracle role can cause
immediately, and configuration failures are refused. From then on that
account drops the leg before valuation. The leg adds no collateral to the
health factor, the LTV and threshold gates, the per-leg 1.05 tuple gate, the
views or the liquidation plan. It is never priced or seized there, and its
owner can withdraw it, in full or in part, whenever the remaining legs keep
the account solvent, also while its feed is still unusable. Other accounts holding the same asset
are valued as before. A debt leg of the asset is still priced, so the
exclusion cannot unblock an account whose unusable feed is a debt leg.

The exclusion carries the ledger sequence it was set at and outlives a feed
recovery. Only the timelocked `restore_account_asset_valuation` that names
that stamp clears it, and only while the asset's price is usable again.
Withdrawing the leg keeps the exclusion, so a later supply of the asset by
that account also counts as zero until the restore. Removing the account
deletes it. Bad-debt cleanup prices excluded legs
strictly, and the residual cleanup of a liquidation skips an account that
holds one, so the exclusion never writes off debt against collateral it has
hidden. The exclusion does not refuse entry; freeze the listing to stop new
legs.

## Storage and account lifecycle

<a id="inv-stor-01"></a>

### INV-STOR-01 — Persistent state has lifecycle discipline

Successful user-entry reads renew their TTL. Metadata and delegate writes
renew their entries; position-map writes rely on the surrounding flow to renew
live sibling entries.

Account removal deletes metadata, both position maps and delegates, and burns
the NFT atomically. Empty-account removal depends on the flow: repayment saves
only debt and does not check for empty-account deletion.

<a id="inv-stor-02"></a>

### INV-STOR-02 — NFT TTL renewal is asymmetric with account renewal

Controller account entries and NFT entries have separate lifetimes and renewal
paths. Contract renewal, dependency renewal and archived-state restoration have
different requirements.

<a id="inv-stor-02a"></a>

#### INV-STOR-02a — The NFT instance renews with account lifecycle

NFT mint and burn renew the instance containing the controller pointer,
collection metadata and ID counter. Explicit renew and upgrade also renew it.
Ordinary ownership reads do not perform that lifecycle renewal.

<a id="inv-stor-02b"></a>

#### INV-STOR-02b — Ownership entries renew on three explicit paths

Mint and permissionless NFT `renew(token_id)` extend Owner and current-owner
Balance entries using the 30-day threshold and 120-day user window.
Owner-authenticated controller `renew_account` renews account entries and
invokes NFT renewal.

NFT renewal alone does not renew controller account maps or OpenZeppelin
enumeration entries. TTL extension neither transfers authority nor shortens a
lifetime.

<a id="inv-stor-02c"></a>

#### INV-STOR-02c — Passive ownership reads renew on OZ's shorter window

The pinned OpenZeppelin `owner_of` extends Owner to 30 days at its 29-day
threshold. Reads neither add 30 days each time nor shorten a longer TTL.
Controller activity can therefore keep account entries alive longer than NFT
ownership entries.

<a id="inv-stor-02d"></a>

#### INV-STOR-02d — An archived `Owner` entry must be restored before use

Archived persistent entries require restoration before use. Account-loading
liquidation paths, NFT burn and NFT renewal resolve ownership.

Transaction integrations must handle simulation, preparation and restore
funding. The contracts do not guarantee immediate access to every dormant
position.

<a id="inv-stor-03"></a>

### INV-STOR-03 — Account existence and NFT existence are paired

Production creation pairs an account ID with its NFT. Production deletion
removes account entries and burns that NFT in one transaction. Loads that
resolve ownership fail closed when the NFT cannot be resolved.

This lifecycle pairing does not imply matching TTLs. The `account_exists` view
checks metadata only, without checking the NFT.

## Flash loans and reentrancy

<a id="inv-flash-01"></a>
<a id="inv-flash-01--flash-repayment-is-exact"></a>

### INV-FLASH-01 — Cash flash repayment is exact

For pre-loan pool balance `B`, principal `P` and fee `F`, the pool requires
balance `B - P` after payout and again after the callback. It requires receiver
allowance of at least `P + F`, pulls exactly `P + F`, then requires `B + F`.

A direct callback push fails; extra allowance is allowed. Cash flash loans
create no account debt and use their own settlement checks.

<a id="inv-flash-02"></a>
<a id="inv-flash-02--monetary-reentrancy-is-blocked"></a>

### INV-FLASH-02 — Protected monetary entry rejects callback reentry

Protected monetary entrypoints reject an active flash guard. Six guarded
windows cover cash flash loans, flash-position funding and callback, router
calls, strategy withdrawal, strategy borrowing and Blend submission. Nested
windows preserve an outer guard.

These windows do not wrap every token call or freeze NFT transfers and public
risk views. A native fixture does not establish callback reachability under
external-host execution.

## Account strategies

<a id="inv-strat-01"></a>
<a id="inv-strat-01--router-authority-is-narrowly-scoped"></a>

### INV-STRAT-01 — Controller router authority binds one input transfer

The controller authorizes one exact
`token_in.transfer(controller, configured_router, amount_in)` invocation,
without sub-invocations. This grants invocation authority, without a token
allowance.

The controller ignores the router's return value, rejects input-balance growth
and rejects measured spending above `amount_in`.

<a id="inv-strat-02"></a>
<a id="inv-strat-02--strategy-settlement-is-measured-and-solvent"></a>

### INV-STRAT-02 — Account strategies settle measured flows and final risk

Distinct-token swaps require positive measured controller output. The current
swap's unspent controller-held input returns to the caller. Router-held residue
instead becomes admin revenue within the router's per-token limit; larger
residue reverts.

The router checks the route minimum against its output vault after fees and
before payout. This does not guarantee measured recipient receipt. The
controller independently checks positive output and final account risk,
without checking the payload minimum.

All six account strategies refresh listed supply LTV and apply final collateral
coverage, health and collateral-floor gates before persistence. Debt-free
accounts skip those three numerical gates. Cash flash loans settle separately;
ordinary supply and repayment do not universally apply that final account gate.

<a id="inv-strat-03"></a>
<a id="inv-strat-03--external-integrations-run-against-an-allowlist"></a>

### INV-STRAT-03 — Blend migration requires an approved pool

Blend migration requires the source Blend pool on the controller's
governance-managed approval list. Admission does not prove permanent integrity
of external code.

The router's separate token whitelist selects fee placement. It does not admit
tokens, venues or pools for migration.

<a id="inv-strat-04"></a>
<a id="inv-strat-04--flash-position-cannot-round-trip-to-a-closed-account"></a>

### INV-STRAT-04 — Flash position retains debt and supply

Flash position mints debt without an origination fee and never automatically
repays returned debt tokens. Every declared collateral minimum must be
non-negative, with at least one positive minimum. In Long or Short mode no
declared collateral may be the debt token, on any hub, as `multiply` requires;
only Multiply mode may declare the debt token and loop it into collateral. The
controller measures callback receipts against those minima, deposits positive
receipts and remeasures the pool's receipt.

Before and after account finalization, the borrowed market must retain positive
scaled debt and the account must retain supply. Returned debt becomes
collateral if declared (Multiply mode only), returns to the caller if
refund-listed, or remains uncredited if in neither list. Refunds cover only positive callback balance
changes.

An already healthy account can support new debt with little additional
collateral. An open solvent position does not prove that the receiver spent
the borrowing on collateral.

## Sources

The [endpoint source map](endpoints.md#source-map) locates each contract surface.
Exact accounting, rounding and numeric bounds are linked from the
[formula source map](formulas.md#sources). Listing and liquidation-curve admission
bounds are enforced by [shared validation](../../common/src/validation.rs).
