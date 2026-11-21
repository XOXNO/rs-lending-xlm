# Runtime invariants

Each `INV-…` heading names one runtime property under a stable id. Search the
repository for the id to find its evidence: test comments in
`contracts/*/tests/` and `tests/test-harness/tests/`, rule comments in
`certora/`, Lean proofs and notes in `verification/aeneas/` (indexed by the
[Aeneas README](../../verification/aeneas/README.md)), and justification lines
in `scripts/permissionless_entrypoints.txt`. `make access-control-check` fails
when a cited id is not a heading here.

Each section gives the property, its enforcement and any limit. Deployment
wiring, configuration and external contracts are checked at deployment. See
also [architecture](architecture.md),
[threat model](../explanation/threat-model.md) and
[exact arithmetic](formulas.md).

## Authority

<a id="inv-auth-01"></a>

### INV-AUTH-01 — Pool mutation requires its owner

The repository deployment path makes governance the controller owner and the
controller the pool owner. Every pool accounting mutator authenticates the
pool owner. The pool constructor takes an administrator argument, so
deployment must establish that chain. Authorized controller ownership changes
and code upgrades can change the wider authority boundary.

<a id="inv-auth-02"></a>

### INV-AUTH-02 — Account spending authority is explicit

Borrowing and withdrawal require the current NFT owner or an active position
manager that owner delegated. Delegates can choose external payout
recipients, so they hold broad economic control within the account's risk
limits. Only the owner grants or revokes delegation. A grant records the
owner's address, not a transfer epoch: it is inactive under another owner and
can reactivate if the NFT returns, unless an intervening owner overwrites or
purges it.

<a id="inv-auth-03"></a>

### INV-AUTH-03 — Third-party supply cannot create a foreign asset slot

Authenticated third parties can repay, liquidate, recapitalize and run
permissionless maintenance. Supply to an existing foreign account can only top
up assets already held, never create a new slot. Policy refresh and
liquidation have separate gates; permissionless access does not mean every
operation preserves health or collateral value.

<a id="inv-auth-04"></a>

### INV-AUTH-04 — Emergency power only tightens

Immediate guardian power can pause and add listing restrictions; under the
repository wiring, unpausing or clearing needs a timelock. Listing edits and
the guardian flag method only keep or tighten flags. Only the timelocked
`relax_spoke_asset_flags` clears one, and only while the listing's flags epoch
equals its `expected_epoch`. The guardian flag method rejects a no-op call, so
the epoch advances at most once per flag between relaxations and a guardian
cannot invalidate a pending relaxation indefinitely. The oracle role's
immediate `set_sanity_band` only keeps or narrows a band.

<a id="inv-auth-05"></a>

### INV-AUTH-05 — Delay updates cannot shorten the minimum below its floor

Construction requires a nonzero minimum delay and enforces neither update
bound. Every update is capped at 241920 ledgers. A decrease must stay at or
above 34560 ledgers (`TIMELOCK_MIN_DELAY_LEDGERS`, about 2 days); a smaller
constructor delay can be kept or raised while below that floor. Only the owner
proposes `UpdateGovDelay`, and a decrease takes effect only after the current,
longer delay. Sensitive and Recovery delays also apply their compiled
minimums. These checks bind the deployed implementation, not replacement
Wasm.

<a id="inv-auth-06"></a>

### INV-AUTH-06 — An account's spoke binding is immutable

An account keeps its creation-time spoke. Existing-account supply, migration,
multiply and flash-position calls must match it; borrow, withdraw and repay
use the stored binding. Credit-mode liquidation needs a same-spoke receiver.

## Accounting and cash

<a id="inv-acct-01"></a>

### INV-ACCT-01 — Revenue shares remain part of total supply

Supply, debt and revenue share totals stay non-negative, and revenue cannot
exceed total supply. Share subtraction rejects negative amounts and
insufficient balances. Revenue minting raises revenue and total supply
equally; revenue claims burn both equally. Reclassifying seized supply raises
revenue without raising total supply. This does not prove that cash and
collectible debt back every claim.

<a id="inv-acct-02"></a>

### INV-ACCT-02 — Cash is the reserve book

Reserve checks use tracked market cash; donations alone do not raise it. Cash
credits and debits reject negative amounts; credits reject overflow and debits
reject insufficient reserves. Outbound transfer and cash bookkeeping are
separate actions.

<a id="inv-acct-03"></a>

### INV-ACCT-03 — Token-funded credit uses measured receipt

Token-funded supply, repayment and recapitalization credit the pool's measured
balance increase, not the requested amount. Strategy supply uses the same
path, and strategy repayment also measures pool receipt. Repayment and
recapitalization can refund unused receipt. Share conversion keeps its own
rounding, and measurement depends on the token's balance reports. Liquidation
Credit mode moves existing collateral shares without incoming tokens.

<a id="inv-acct-04"></a>

### INV-ACCT-04 — Backing shortfall blocks new supply

New token-funded supply rejects a positive backing shortfall: floored supplied
claims less ceiled debt value, rounded down to native units once, compared
against tracked cash with saturating arithmetic. Recapitalization fills at
most that shortfall, refunds excess, mints no shares and cannot restore a
written-down index. The non-zero supply-index floor can leave residual claims
needing recapitalization; see the
[backing calculation](formulas.md#backing-and-cash-constraints).

<a id="inv-acct-05"></a>

### INV-ACCT-05 — Positive position settlement requires shares

Positive supply and borrow amounts must mint shares. Positive net repayments
and gross withdrawals must burn shares; positive same-asset net settlement
must burn both supply and debt shares. Zero-share results revert there.
Recapitalization mints no shares, and protocol-reward conversion can floor to
zero revenue shares.

<a id="inv-acct-06"></a>

### INV-ACCT-06 — Revenue claims respect accounting bounds

Revenue payout cannot exceed tracked cash or the floored revenue claim. Full
payout burns all revenue shares; cash-limited payout burns a proportional
ceiling of shares; positive payout cannot burn zero shares. Claims enforce
utilization below a 100% ceiling ([INV-ACCT-08](#inv-acct-08)) and reject zero
total supply with outstanding debt. They do not prove full backing. See the
[payout formula](formulas.md#revenue-payout).

<a id="inv-acct-07"></a>

### INV-ACCT-07 — Borrow draws reserve a cash buffer

Borrow debt minting requires post-draw cash to cover 200 BPS of the floored
supplied token value, rounded up. Strategy debt openings check gross principal
before withholding any fee. Exits do not preserve the buffer, and it does not
guarantee cash for every later liquidation.

<a id="inv-acct-08"></a>

### INV-ACCT-08 — Selected operations enforce the utilization ceiling

Borrow debt minting, ordinary withdrawal and revenue claims reject utilization
above the market ceiling. The gate compares ceiled debt value with floored
supply value times the ceiling, rounded down, so rounding cannot admit excess
utilization and no quotient overflows. It skips zero total supply shares and
zero debt value; debt against zero floored supply value fails.

Debt mints are checked at every ceiling, 100% included, so after a borrow or
strategy opening on a market with supply shares, debt value does not exceed
supply value. Ordinary withdrawal and revenue claims skip a 100% ceiling: they
cannot draw more than cash, so the check could fail only on rounding dust
between cash plus debt and supplier claims, blocking withdrawal of the last
cash. Liquidation withdrawal skips the gate. Accrual and bad-debt writeoff can
exceed the ceiling, so it is not a market-wide bound.

<a id="inv-acct-09"></a>

### INV-ACCT-09 — Debt mints and selected exits cannot leave debt without supply

Borrows, strategy openings, withdrawal, same-asset net settlement and revenue
claims reject a resulting zero supplied-share total with non-zero debt shares.
A debt mint on a market with no supply shares therefore reverts with
`PoolInsolvent` (123), even when the market holds cash. This prevents an
empty-supply state but proves neither full backing nor a liquidation cash
reserve. Liquidation withdrawal is not exempt: a Transfer seizure that burns a
market's last supply shares while it has debt reverts with `PoolInsolvent`
(123), unless the leg's protocol fee books revenue shares first.
`get_liquidation_estimate` does not detect it. `Credit` mode, a smaller
Transfer seizure, or repaying that debt avoids it.

<a id="inv-acct-10"></a>

### INV-ACCT-10 — Account books reconcile with pool totals

The pool stores market totals only and trusts the scaled position each pool
position call carries; it has no per-account book to check. Per market, the
sum of account supply shares equals pool supplied shares minus revenue shares,
and the sum of account debt shares equals pool borrowed shares.

The controller holds this by construction, not by runtime assertion. It takes
each new scaled position from the pool's returned mutation, merges duplicate
legs by hub asset before reading a position, and moves shares between
accounts only in credit-mode liquidation, where debit, credit and fee are
asserted to sum exactly. A controller path that writes a position from any
other source, or sends a stale position to the pool, breaks this invariant
silently.

The `prop_accounting_conservation` property test checks both equalities in
token units, within 4 units, after every operation. It and the seed-adjusted
cash tests start every market with no seeded liquidity, so their cash bound is
tight. No on-chain view enumerates accounts, so a live network exposes only
the spoke-level identity: pool supplied minus the sum of spoke usage equals
revenue.

<a id="inv-acct-11"></a>

### INV-ACCT-11 — Entries keep market totals under the entry ceiling

After a supply, borrow, strategy opening or flash loan, total supply and total
debt are each at most `MAX_MARKET_ENTRY_RAY` (about 21.27 billion whole
tokens), as a share count and as a half-up value at the live index; otherwise
the entry reverts with `MarketCeilingReached` (136). The admitted cap maximum
is the same ceiling in asset units. It sits a factor 8 below the
[INV-IDX-06](#inv-idx-06) value ceiling, leaving room for interest. Share
count is bounded too because a supply index at its floor mints 1,000 shares
per unit of value. Exits never check it. Interest can push a total above it;
the market then takes no entry until repayments, withdrawals or liquidations
bring it under. See [numeric limits](formulas.md#numeric-limits).

## Indexes and accrual

<a id="inv-idx-01"></a>

### INV-IDX-01 — Borrow index is monotone and bounded

Both indexes start at one RAY. Successful accrual with validated rate
parameters cannot lower the borrow index and caps it at the protocol constant
10^36 raw RAY, where accrual yields no borrower interest. The
[INV-IDX-06](#inv-idx-06) market value ceiling stops borrow-index growth
earlier when a market value would pass it; accrual does not revert on a market
value.

<a id="inv-idx-02"></a>

### INV-IDX-02 — Supply index is bounded

The supply index stays within 10^24 to 10^36 raw RAY, inclusive; these are
protocol constants, not per-market settings. Interest distribution cannot
lower it; bad-debt writeoff applies the lower bound, which prevents a zero
conversion divisor but can preserve an unbacked residual claim. The upper
bound is the index cap; the [INV-IDX-06](#inv-idx-06) value ceiling limits the
supply value interest can reach.

<a id="inv-idx-03"></a>

### INV-IDX-03 — Bad debt may lower the supply index

Debt writeoff reduces claims in the debt market, revenue-share claims
included, through its supply index, using two floors and a non-zero index
clamp. Zero supplied value leaves the index unchanged; an index at its floor
need not decrease. Seizure accrues its market first, so snapshots across time
can hide a writeoff behind interest. Each pool seizure writes only its
identified market. See the [write-down calculation](formulas.md#bad-debt).

<a id="inv-idx-04"></a>

### INV-IDX-04 — Accrual shares one bounded-step calculation

Zero elapsed time means no accrual and no timestamp advance. Longer intervals
use forward chunks of at most 31,556,926,000 milliseconds. Mutating accrual
and read-only index projection share the calculation, including revenue
shares in later supply totals. Each chunk takes its rate from starting
utilization, so call cadence can change rates and rounded results; there is no
guarantee of cadence independence, monotonic partition results or an exact
continuous-exponential bound. See [accrual cadence](formulas.md#accrual-cadence).

<a id="inv-idx-05"></a>

### INV-IDX-05 — Interest allocation retains explicit rounding limits

At the RAY-value split, borrower interest equals supplier rewards plus the
reserve-factor fee; supplier rewards the supply-index change misses join the
protocol reward. Revenue-share conversion floors at the new supply index and
caps at remaining total-supply share headroom, so it can leave reward value
unrepresented; booked supplier and revenue value are not exactly conserved.
See [interest allocation](formulas.md#compounding-and-interest-allocation).

<a id="inv-idx-06"></a>

### INV-IDX-06 — Accrual holds interest at the market value ceiling

Accrual does not overflow on a market value. Each chunk caps borrow-index
growth so borrower interest fits the room the larger of total debt value and
total supply value has below `MAX_MARKET_VALUE_RAY` (`i128::MAX - RAY`, about
170.14 billion whole tokens). A market within the ceiling stays within it.
With no room, or a total whose value overflows `i128`, a chunk changes no
index and books no revenue; time still advances, so the loop stays bounded.

Every pool operation accrues first, so for any stored state with non-negative
share totals and in-range indexes, accrual cannot block a repayment,
withdrawal or liquidation, whatever the elapsed time. Entries enforce
market-size limits instead ([INV-ACCT-11](#inv-acct-11)). The pool emits
`MarketValueCeilingEvent` for each accrual where the ceiling held interest;
that market's borrowers then accrue less than the rate implies. Accrual still
reverts on a negative share total or index, which no operation produces. See
[interest allocation](formulas.md#compounding-and-interest-allocation).

## Oracle validity

<a id="inv-oracle-01"></a>

### INV-ORACLE-01 — Required valuations fail closed

A missing or unusable required price aborts a valuation-dependent operation,
liquidation included. Strict reads reject resolution errors, stale prices,
source disagreement, nonpositive prices and sanity-band violations.
Diagnostic `quotes` can retain a nonzero candidate with `valid=false`, which
valuation never accepts.

<a id="inv-oracle-02"></a>

### INV-ORACLE-02 — A dual source requires both legs

A configured two-source price needs both usable legs; one surviving leg is no
fallback. The accepted price is their integer midpoint, rounded down, within
both the input range and the final sanity band. A partial reading is
unusable; a stale surviving leg reports staleness before disagreement.
Two `Market` legs are stale when their newest samples are more than
`MAX_LEG_AGE_SPREAD_SECONDS` apart; see
[price sources](formulas.md#price-sources). Separately admitted single-source
configurations keep freshness, positivity and sanity checks.

<a id="inv-oracle-03"></a>

### INV-ORACLE-03 — A context retains fetched prices

A controller context keeps each fetched asset price and requests only missing
assets later; missing cached prices fail closed. Aggregator sessions also
cache resolved keys. These caches keep repeated valuations consistent within
one context, not observation timestamps across sources or a snapshot shared by
independent contexts.

<a id="inv-oracle-04"></a>

### INV-ORACLE-04 — Observation timestamps have a bounded future allowance

Feed timestamps beyond ledger time plus 60 seconds are discarded; the exact
boundary is allowed. Multi-feed reads check package and write timestamps after
converting milliseconds to whole seconds. Reflector reads check the timestamp
of each sample they use. Chainlink round timestamps are already Unix seconds,
checked without conversion. Discarding a required leg makes its configuration
unusable under INV-ORACLE-01 and INV-ORACLE-02.

## Account risk

<a id="inv-risk-01"></a>

### INV-RISK-01 — Risk-increasing actions re-prove solvency

Ordinary borrowing, withdrawal and account strategies apply final solvency
gates after pool mutations. With debt remaining, LTV-weighted collateral must
cover debt, health factor must be at least one, and LTV-weighted collateral
must meet any nonzero configured floor. Debt-free accounts skip these
numerical gates.

Listed LTV snapshots refresh before the shared gate, with a liquidation tuple
that does not favor liquidators. Listing and count checks precede entry; spoke
caps use returned pool deltas and revert the whole transaction on failure.
Supply, repayment and liquidation do not universally apply the final gate.

A refresh never stores an LTV at or above the stored liquidation threshold.
When the stored threshold is below the listed one, it stores the listed LTV
minus that difference, floored at zero, keeping the listing's LTV-to-threshold
gap so borrow weight stays below liquidation weight. This holds for the
pre-gate refresh, a supply or withdrawal of the leg,
`update_account_threshold`, and the limit `get_ltv_collateral_usd` reports. An
unlisted leg keeps its stored tuple.

<a id="inv-risk-02"></a>

### INV-RISK-02 — Conservative valuation biases safety

Risk-gate collateral and its LTV or threshold weights round down. Risk debt
rounds up; health-factor division rounds down and saturates. LTV weighting
uses the smaller stored LTV and liquidation threshold. Unweighted total
collateral rounds half-up, including for bad-debt eligibility; debt display
helpers can also round half-up. See
[valuation and health](formulas.md#valuation-and-health).

<a id="inv-risk-03"></a>

### INV-RISK-03 — Risk configuration is coherent

Admitted listings require `0 <= LTV < threshold <= BPS` and
`threshold * (BPS + bonus) <= BPS * BPS`, with liquidation fees strictly below
BPS. The liquidation curve requires a target health factor above one WAD and
at most ten WAD, a positive full-ramp health threshold below the target, and a
positive bonus factor at most BPS. These bounds validate configuration; they
do not guarantee a profitable liquidation after rounding.

<a id="inv-risk-04"></a>

### INV-RISK-04 — Position and delegate counts are bounded

New supply and borrow slots must fit their configured counts; existing-asset
top-ups stay allowed if a limit falls below the account's count. An existing
Credit receiver obeys the same new-slot limit; a `Credit(0)` receiver skips
it. An account has at most 16 delegates. A supply leg below 3 decimals is its
account's only supply position: a new slot beside it, by deposit or credit, is
rejected. These bounds limit state size, not worst-case transaction budgets.

## Liquidation and bad debt

<a id="inv-liq-01"></a>

### INV-LIQ-01 — Only unhealthy debt can be liquidated

Ordinary liquidation requires outstanding debt and health factor below one.
The liquidator authenticates but need not own the target; owners can
self-liquidate.

A Credit receiver must be a different, Normal-mode account in the same spoke,
controlled by the liquidator as owner or active delegate. `Credit(0)` can
create a receiver in a deprecated spoke. Ownership and position limits apply,
except that a `Credit(0)` receiver skips the governed position limit.
Whole-unit isolation ([INV-RISK-04](#inv-risk-04)) applies to every receiver.
The 2-whole-unit floor on a leg below 3 decimals is not checked at credit,
which only adds shares; it binds at the receiver's next borrow, withdrawal or
account strategy that leaves debt.

<a id="inv-liq-02"></a>

### INV-LIQ-02 — Repayment and seizure stay coupled

Repayment planning caps each input at its leg's actual debt; a partial plan
also trims the excess above the curve quote before transfers. Planned refunds
are unused input: a partial plan never pulls them; a full-debt plan pulls the
offered amount and the pool returns the excess over each leg's debt, equal to
the planned refund. Seizure is proportional to collateral value and capped at
held collateral; rounding can leave repayment with no payable seizure. A
collateral leg below 3 decimals seizes whole units: rounded up when the plan
repays all debt, otherwise rounded down with the unbacked repayment refunded;
a plan left with no seizure reverts. Such a leg is the account's only supply
position, so seizure stays proportional.

On a solvent account whose leg holds a whole unit, the partial quote can
change. When one unit's value divided by `1 + bonus` covers the whole debt
plus one base unit of each debt leg, the quote becomes the whole debt and the
leg rounds up to one unit; that full close can pay more than the quoted bonus.
Otherwise, when the quote seizes less than one unit plus a `1e-6` margin, it
rises to the repayment backing one unit plus the margin, if below the whole
debt, and the seizure refunds the margin, rounded down to whole debt-token
units. Such an account thus stays liquidatable below `HF = 1`. In the narrow
debt band where neither applies, if the curve quote backs less than one unit,
every offer reverts until accrual or a price move ends that state. See
[whole-unit legs](formulas.md#whole-unit-collateral-below-3-decimals).

Transfer mode burns shares and pays underlying after fees; the planned
transfer fee never exceeds the whole base units paid above the leg's
principal. Credit mode splits seized shares exactly between receiver credit
and a ceiling-rounded fee on bonus shares. The fee reclassifies existing
supply as revenue, needs no collateral cash and reduces same-spoke usage only
by the fee. When the same call socializes remaining debt
([INV-LIQ-04](#inv-liq-04)) in a market where the receiver was just credited,
the receiver's shares bear their pro-rata write-down;
`get_liquidation_estimate` and `LiquidationEvent.bonus_bps` show the plan
before it. New receiver assets need a current listing; existing receiver risk
snapshots stay unchanged. See
[liquidation arithmetic](formulas.md#liquidation-sizing-and-fees).

<a id="inv-liq-03"></a>

### INV-LIQ-03 — Under-delivery reduces seizure

Receipt below planned repayment floor-scales seizure amounts, transfer fees,
seized shares and bonus shares by measured/planned USD, and caps each transfer
at the floor asset value of its scaled shares. Credit fees are recomputed from
the scaled bonus shares. A short receipt reverts with `InvalidPayments` when a
seized leg is below 3 decimals, so whole-unit collateral is never scaled into
a fraction. Scaling does not re-cap the transfer fee at the scaled payout
above principal, so a leg can pay the liquidator up to one token unit below
its scaled principal. Repayment USD is capped per leg at its planned value and
need not equal every rounded debt-share reduction or refund.

<a id="inv-liq-04"></a>

### INV-LIQ-04 — Bad-debt socialization is explicit and total

Permissionless cleanup requires ceil risk debt above half-up unweighted
collateral, and collateral at or below the fixed $5 dust threshold. Owner-only
forced cleanup omits the dust cap. Both require debt, readable account and NFT
state, valid required prices and no active flash guard. Listing flags and
global pause do not block standalone cleanup.

Cleanup reclassifies all remaining collateral shares as revenue, writes off
all remaining debt against each debt's market without netting same-market
supply, releases spoke usage, and atomically removes account entries and the
NFT. Standalone cleanup emits `CleanBadDebtEvent` with pre-cleanup USD totals
and no controller position-update batch. Liquidation and cleanup assert no
final account health or full backing. The index floor can leave a shortfall,
which recapitalization fills without restoring the lost index or deleted
account.

## Pauses, flags and caps

<a id="inv-halt-01"></a>

### INV-HALT-01 — Global pause blocks new risk

Global pause blocks supply, borrowing, flash and strategy entry, delegate
grants, index updates, revenue claims and threshold refresh. Withdrawal,
repayment, liquidation, bad-debt cleanup, recapitalization, account renewal
and delegate revocation stay callable under their own gates.

<a id="inv-halt-02"></a>

### INV-HALT-02 — Frozen, paused, and no_seize gate different legs

Entry rejects `paused` and `frozen`; user exits and liquidation repayment
reject `paused`; ordinary liquidation seizure rejects only `no_seize`. Missing
listings pass the flag helper, but entry and new Credit receiver assets
separately require a listing. Nonzero `no_seize` collateral can block the
whole proportional liquidation, even if supplied after the flag was set;
zero-token planned seizure legs are dropped before that check. Standalone
bad-debt cleanup bypasses these flags.

<a id="inv-halt-03"></a>

### INV-HALT-03 — Cap entry checks; scaled conversion can fail open

A zero supply or borrow cap admits no positive entry. Entry compares scaled
usage with the asset-unit cap converted at the returned live index. That
conversion saturates at `i128::MAX` instead of reverting, so a saturated cap
fails open and the asset-unit limit is not enforced. An admitted cap saturates
only at an index below an eighth of one RAY. At the supply-index floor
(`RAY / 1000`), a supply cap above about 1/125 of the admitted maximum
saturates; the entry ceiling ([INV-ACCT-11](#inv-acct-11)) still bounds that
market's share count. The borrow index never falls below one RAY, so an
admitted borrow cap cannot saturate. See
[cap conversion](formulas.md#caps-fees-and-numeric-limits).

Exits consume no cap. Missing usage rows and zero deltas are no-ops, and
stored usage cannot go negative. Same-spoke liquidation credit bypasses entry
caps and books its fee as an exit.

## Storage and account lifecycle

<a id="inv-stor-01"></a>

### INV-STOR-01 — Persistent state has lifecycle discipline

Successful user-entry reads renew their TTL. Metadata and delegate writes
renew their entries; position-map writes rely on the surrounding flow to renew
live sibling entries. Account removal atomically deletes metadata, both
position maps and delegates, and burns the NFT. Empty-account removal depends
on the flow: repayment saves only debt and does not check for empty-account
deletion.

<a id="inv-stor-02"></a>

### INV-STOR-02 — NFT TTL renewal is asymmetric with account renewal

Controller account entries and NFT entries have separate lifetimes and renewal
paths. Contract renewal, dependency renewal and archived-state restoration
have different requirements.

<a id="inv-stor-02a"></a>

#### INV-STOR-02a — The NFT instance renews with account lifecycle

NFT mint, burn, explicit renew and upgrade renew the instance holding the
controller pointer, collection metadata and ID counter. Ordinary ownership
reads do not.

<a id="inv-stor-02b"></a>

#### INV-STOR-02b — Ownership entries renew on three explicit paths

Mint and permissionless NFT `renew(token_id)` extend Owner and current-owner
Balance entries with the 30-day threshold and 120-day user window.
Owner-authenticated controller `renew_account` renews account entries and
invokes NFT renewal. NFT renewal alone renews neither controller account maps
nor OpenZeppelin enumeration entries. TTL extension neither transfers
authority nor shortens a lifetime.

<a id="inv-stor-02c"></a>

#### INV-STOR-02c — Passive ownership reads renew on OZ's shorter window

The pinned OpenZeppelin `owner_of` extends Owner to 30 days at its 29-day
threshold; reads neither add 30 days each time nor shorten a longer TTL.
Controller activity can therefore keep account entries alive longer than NFT
ownership entries.

<a id="inv-stor-02d"></a>

#### INV-STOR-02d — An archived `Owner` entry must be restored before use

Archived persistent entries need restoration before use. Account-loading
liquidation paths, NFT burn and NFT renewal resolve ownership. Integrations
must handle simulation, preparation and restore funding; the contracts do not
guarantee immediate access to every dormant position.

<a id="inv-stor-03"></a>

### INV-STOR-03 — Account existence and NFT existence are paired

Production creation pairs an account ID with its NFT, and production deletion
removes account entries and burns that NFT in one transaction. Loads that
resolve ownership fail closed when the NFT cannot be resolved. The pairing
does not imply matching TTLs. The `account_exists` view checks metadata only,
not the NFT.

## Flash loans and reentrancy

<a id="inv-flash-01"></a>

### INV-FLASH-01 — Cash flash repayment is exact

For pre-loan pool balance `B`, principal `P` and fee `F`, the pool requires
balance `B - P` after payout and after the callback, receiver allowance of at
least `P + F`, pulls exactly `P + F`, then requires `B + F`. A direct callback
push fails; extra allowance is allowed. Cash flash loans create no account
debt and have their own settlement checks.

<a id="inv-flash-02"></a>

### INV-FLASH-02 — Protected monetary entry rejects callback reentry

Protected monetary entrypoints reject an active flash guard. Six guarded
windows cover cash flash loans, flash-position funding and callback, router
calls, strategy withdrawal, strategy borrowing and Blend submission; nested
windows keep an outer guard. The windows do not wrap every token call or
freeze NFT transfers and public risk views. A native fixture does not prove
callback reachability under external-host execution.

## Account strategies

<a id="inv-strat-01"></a>

### INV-STRAT-01 — Controller router authority binds one input transfer

The controller authorizes one exact
`token_in.transfer(controller, configured_router, amount_in)` invocation,
without sub-invocations: invocation authority, not a token allowance. It
ignores the router's return value, rejects input-balance growth and rejects
measured spending above `amount_in`.

<a id="inv-strat-02"></a>

### INV-STRAT-02 — Account strategies settle measured flows and final risk

Distinct-token swaps require positive measured controller output. The current
swap's unspent controller-held input returns to the caller. Router-held
residue becomes admin revenue within the router's per-token limit; larger
residue reverts. The router checks the route minimum against its output vault
after fees and before payout, which does not guarantee measured recipient
receipt; the controller checks positive output and final account risk, not
the payload minimum.

All six account strategies refresh listed supply LTV and apply final
collateral coverage, health and collateral-floor gates before persistence;
debt-free accounts skip those three numerical gates. Cash flash loans settle
separately; ordinary supply and repayment do not universally apply that gate.

<a id="inv-strat-03"></a>

### INV-STRAT-03 — Blend migration requires an approved pool

Blend migration requires the source Blend pool on the controller's
governance-managed approval list, which does not prove permanent integrity of
external code. The router's separate token whitelist selects fee placement; it
admits no tokens, venues or pools for migration.

<a id="inv-strat-04"></a>

### INV-STRAT-04 — Flash position retains debt and supply

Flash position mints debt without an origination fee and never auto-repays
returned debt tokens. Every declared collateral minimum must be non-negative,
at least one positive. The controller measures callback receipts against the
minima, deposits positive receipts and remeasures the pool's receipt. Before
and after account finalization, the borrowed market must keep positive scaled
debt and the account must keep supply. Returned debt becomes collateral if
declared, returns to the caller if refund-listed, or stays uncredited.
Refunds cover only positive callback balance changes. An already healthy
account can support new debt with little added collateral; the controller
checks the declared minima, not how the receiver spends the borrowing.

## Sources

The [endpoint source map](endpoints.md#source-map) locates each contract
surface, the [formula source map](formulas.md#sources) links exact accounting,
rounding and numeric bounds, and
[shared validation](../../common/src/validation.rs) enforces listing and
liquidation-curve admission bounds.
