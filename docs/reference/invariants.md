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
clearing restrictions requires a timelock under the repository wiring.

A timelocked full listing rewrite can clear flags. The restriction to tightening
flags applies to the dedicated flag-setting method.

<a id="inv-auth-05"></a>
<a id="inv-auth-05--governance-delay-cannot-be-shortened"></a>

### INV-AUTH-05 — Delay updates cannot shorten the minimum

Construction requires a nonzero minimum delay. Delay updates can retain or
increase it, up to 241920 ledgers; construction does not enforce that upper
bound. Sensitive and Recovery delays also apply their compiled minimums.

These checks govern the deployed implementation and cannot constrain replacement
Wasm.

<a id="inv-auth-06"></a>

### INV-AUTH-06 — An account's spoke binding is immutable

An account retains its creation-time spoke. Existing-account supply, migration,
multiply and flash-position calls must match that spoke; borrow, withdraw and
repay use the stored binding. Credit-mode liquidation requires a receiver in
the same spoke.

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

New token-funded supply rejects a positive backing shortfall. The check compares
floored supplied claims against tracked cash plus ceiled debt value in native
token units, using saturating arithmetic.

Recapitalization fills at most that shortfall, refunds excess and mints no
shares. It cannot restore a written-down index. The non-zero supply-index floor
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

Revenue payout cannot exceed tracked cash or the floored revenue claim. Full
payout burns all revenue shares; cash-limited payout burns a proportional
ceiling of shares. Positive payout cannot burn zero shares.

Claims enforce utilization and reject zero total supply with outstanding debt.
These checks do not establish full backing. The [payout formula](formulas.md#revenue-payout)
defines the burn calculation.

<a id="inv-acct-07"></a>
<a id="inv-acct-07--borrow-draws-leave-a-liquidation-cash-buffer"></a>

### INV-ACCT-07 — Borrow draws reserve a cash buffer

Borrow debt minting requires cash after the requested draw to cover 200 BPS of
the floored supplied token value, with half-up BPS rounding. Strategy debt
openings check gross principal before withholding any fee.

Exits do not preserve this buffer. It does not guarantee enough cash for every
subsequent liquidation.

<a id="inv-acct-08"></a>
<a id="inv-acct-08--utilization-stays-below-the-market-ceiling"></a>

### INV-ACCT-08 — Selected operations enforce the utilization ceiling

Borrow debt minting, ordinary withdrawal and revenue claims reject utilization
above the market ceiling. The gate divides half-up-valued debt by half-up-valued
supply. It skips zero total supply and ceilings at least one RAY.

Liquidation withdrawal skips the gate. Accrual and bad-debt writeoff can exceed
the ceiling; it is not a market-wide bound maintained by every operation.

<a id="inv-acct-09"></a>
<a id="inv-acct-09--exits-cannot-leave-debt-without-supply"></a>

### INV-ACCT-09 — Selected exits cannot leave debt without supply

Withdrawal, same-asset net settlement and revenue claims reject a resulting
zero total supplied-share balance with non-zero debt shares. This prevents an
empty-supply state, without establishing full backing or a liquidation cash
reserve.

## Indexes and accrual

<a id="inv-idx-01"></a>

### INV-IDX-01 — Borrow index is monotone and bounded

Both indexes start at one RAY. Successful accrual with validated rate parameters
cannot lower the borrow index and caps it at the protocol constant 10^36 raw
RAY. At the ceiling, further accrual produces no borrower interest.

Debt-value overflow can still revert accrual before that ceiling is reached.
Bounded indexes do not guarantee representable position or market values.

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
read-only index projection use the same calculation, including revenue shares
in subsequent supply totals.

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
source disagreement, nonpositive prices and sanity-band violations.

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

## Account risk

<a id="inv-risk-01"></a>

### INV-RISK-01 — Risk-increasing actions re-prove solvency

Ordinary borrowing, withdrawal and account strategies apply final solvency gates
after pool mutations. With debt remaining, LTV-weighted collateral must cover
debt, health factor must be at least one, and LTV-weighted collateral must meet
any nonzero configured floor. Debt-free accounts skip these numerical gates.

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

<a id="inv-risk-04"></a>

### INV-RISK-04 — Position and delegate counts are bounded

New supply and borrow slots must fit their configured counts. Existing-asset
top-ups remain allowed if a count limit falls below the account's existing
count. Credit-mode receivers obey the same new-slot limit, and an account can
have at most 16 delegates.

These bounds limit state size without proving worst-case transaction-budget
sufficiency.

## Liquidation and bad debt

<a id="inv-liq-01"></a>

### INV-LIQ-01 — Only unhealthy debt can be liquidated

Ordinary liquidation requires outstanding debt and health factor below one.
The liquidator authenticates but need not own the target; owners can
self-liquidate.

A Credit receiver must be a different account in the same spoke, in Normal
mode, and controlled by the liquidator as owner or active delegate. `Credit(0)`
can create a receiver in a deprecated spoke. Ownership and position limits
still apply.

<a id="inv-liq-02"></a>

### INV-LIQ-02 — Repayment and seizure stay coupled

Repayment planning caps each input by actual debt and the liquidation curve,
then trims excess before transfers. Planned refunds are unused input that is
never pulled. Seizure is proportional to collateral value and capped at held
collateral; rounding can leave repayment with no payable seizure.

Transfer mode burns shares and pays underlying after fees. Credit mode splits
seized shares exactly between receiver credit and a ceiling-rounded fee on
bonus shares. The fee reclassifies existing supply as revenue, requires no
collateral cash and reduces same-spoke usage only by the fee.

New receiver assets require a current listing. Existing receiver risk snapshots
stay unchanged. See [liquidation arithmetic](formulas.md#liquidation-sizing-and-fees)
for bonus selection, sizing and rounding.

<a id="inv-liq-03"></a>

### INV-LIQ-03 — Under-delivery reduces seizure

Receipt below planned repayment floor-scales seizure amounts, transfer fees,
seized shares and bonus shares by measured/planned USD. Credit fees are
recomputed from the scaled bonus shares.

Repayment USD is capped per leg at its planned value. It does not exactly equal
every rounded debt-share reduction or refund.

<a id="inv-liq-04"></a>

### INV-LIQ-04 — Bad-debt socialization is explicit and total

Permissionless cleanup requires debt greater than total collateral and
collateral at or below the fixed $5 dust threshold. Owner-only forced cleanup
omits the dust cap. Both require debt, readable account and NFT state, valid
required prices and no active flash guard. Listing flags and global pause do
not block standalone cleanup.

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
Some maintenance operations therefore remain pause-gated.

<a id="inv-halt-02"></a>

### INV-HALT-02 — Frozen, paused, and no_seize gate different legs

Listing flags act independently: entry rejects `paused` and `frozen`; user
exits and liquidation repayment reject `paused`; ordinary liquidation seizure
rejects only `no_seize`. Missing listings pass the flag helper, while entry and
new Credit receiver assets separately require a listing.

Nonzero `no_seize` collateral can block the whole proportional liquidation,
even if supplied after the flag is set. Zero-token planned seizure legs are
omitted before that check. Standalone bad-debt cleanup bypasses these flags.

<a id="inv-halt-03"></a>

### INV-HALT-03 — Caps are literal and exit-safe

A zero supply or borrow cap admits no positive entry. Entry compares scaled
usage against the asset-unit cap converted at the returned live index.

Exits consume no cap. Missing usage rows and zero deltas are no-ops, and stored
usage cannot become negative. Same-spoke liquidation credit bypasses entry
caps and books its fee as an exit. See [cap conversion](formulas.md#caps-fees-and-numeric-limits).

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

Blend migration requires the destination pool on the controller's
governance-managed approval list. Admission does not prove permanent integrity
of external code.

The router's separate token whitelist selects fee placement. It does not admit
tokens, venues or pools for migration.

<a id="inv-strat-04"></a>
<a id="inv-strat-04--flash-position-cannot-round-trip-to-a-closed-account"></a>

### INV-STRAT-04 — Flash position retains debt and supply

Flash position mints debt without an origination fee and never automatically
repays returned debt tokens. Every declared collateral minimum must be
non-negative, with at least one positive minimum. The controller measures
callback receipts against those minima, deposits positive receipts and
remeasures the pool's receipt.

Before and after account finalization, the borrowed market must retain positive
scaled debt and the account must retain supply. Returned debt becomes
collateral if declared, returns to the caller if refund-listed, or remains
uncredited if in neither list. Refunds cover only positive callback balance
changes.

An already healthy account can support new debt with little additional
collateral. An open solvent position does not prove that the receiver spent
the borrowing on collateral.

## Sources

The [endpoint source map](endpoints.md#source-map) locates each contract surface.
Exact accounting, rounding and numeric bounds are linked from the
[formula source map](formulas.md#sources). Listing and liquidation-curve admission
bounds are enforced by [shared validation](../../common/src/validation.rs).
