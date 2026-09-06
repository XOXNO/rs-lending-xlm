# Runtime invariants

These are audit properties and their enforcement surfaces. Runtime checks, test
assertions and formal specifications are different evidence. A cited rule is not
a claim of a completed proof; fixtures and models retain their own assumptions.
Deployment wiring, configuration, external behavior and archival recovery still
require integration verification.

<a id="inv-auth-01"></a>
<a id="inv-auth-01--one-ownership-chain"></a>
### INV-AUTH-01 — Pool mutation requires its owner

Under the repository deployment path, governance owns the controller and the controller owns the pool. Every pool accounting mutator
authenticates the pool owner. The pool constructor accepts an administrator argument, so correct wiring depends on deployment;
authorized controller ownership changes or code upgrades can alter the wider authority boundary.

<a id="inv-auth-02"></a>
<a id="inv-auth-02--risk-reducing-authority-is-explicit"></a>
### INV-AUTH-02 — Account spending authority is explicit

Borrowing and withdrawing require the current NFT owner or an active position manager delegated by that owner. Delegates may choose
external payout recipients and therefore have broad economic control within the account's risk limits. Only the owner grants or revokes
delegates. Grants are stamped by owner address, not transfer epoch: they are inactive under another owner but can reactivate when the
NFT returns unless an intervening owner overwrites or purges them.

<a id="inv-auth-03"></a>
<a id="inv-auth-03--permissionless-actions-do-not-create-foreign-risk"></a>
### INV-AUTH-03 — Third-party supply cannot create a foreign asset slot

Authenticated third parties may repay, liquidate, recapitalize and perform permissionless maintenance. Third-party supply to an existing
foreign account may only top up supply assets already held; it cannot open a new asset slot. Policy refresh and liquidation have their
own gates; this is not a universal promise that every permissionless operation preserves health or collateral value.

<a id="inv-auth-04"></a>
### INV-AUTH-04 — Emergency power only tightens

Immediate guardian power can pause and add listing restrictions, not unpause or clear them. Reopening is timelocked under the repository
wiring. A timelocked full listing rewrite can clear flags; the callee's ratchet applies to the dedicated flag-setting method.

<a id="inv-auth-05"></a>
<a id="inv-auth-05--governance-delay-cannot-be-shortened"></a>
### INV-AUTH-05 — Delay updates cannot shorten the minimum

Construction requires a nonzero configured minimum. The current delay-update path permits equal or greater values, capped at 241920
ledgers; construction does not enforce that update cap. Sensitive and Recovery delays are separately raised to their compiled floors.
These rules constrain the current implementation, not replacement Wasm.

<a id="inv-auth-06"></a>
### INV-AUTH-06 — An account's spoke binding is immutable

An account keeps its creation-time spoke binding. Existing-account supply, migration, multiply and flash-position calls must match the
supplied spoke; borrow, withdraw and repay use the stored binding. Credit-mode liquidation requires the receiver in the same spoke.

<a id="inv-acct-01"></a>
<a id="inv-acct-01--supply-revenue-and-debt-shares-are-non-negative"></a>
### INV-ACCT-01 — Revenue shares remain part of total supply

Share accounting maintains non-negative supply, debt and revenue totals, with revenue no greater than total supply. Share subtraction
rejects negative operands or an insufficient balance. Revenue minting increases revenue and total supply equally; revenue claims burn
them equally. Reclassifying seized supply increases revenue without increasing total supply. This share relationship does not establish
that every claim is backed by cash and collectible debt.

<a id="inv-acct-02"></a>
### INV-ACCT-02 — Cash is the reserve book

Reserve checks use tracked market cash. Token donations alone do not increase it. Cash credits and debits reject negative amounts;
credits reject overflow and debits reject insufficient reserves. Outbound token transfer and cash bookkeeping are separate actions.

<a id="inv-acct-03"></a>
<a id="inv-acct-03--credit-equals-measured-receipt"></a>
### INV-ACCT-03 — Token-funded credit uses measured receipt

Token-funded supply, repayment and recapitalization pass the pool's measured balance increase into accounting. Strategy supply uses the
same supply path; strategy repayment also measures the pool receipt. Requested amounts alone do not determine credit. Repayment and
recapitalization can refund unused receipt, and share conversion retains its own rounding. Liquidation credit that transfers existing
shares requires no incoming collateral tokens. Measurement relies on the token's balance reports.

<a id="inv-acct-04"></a>
### INV-ACCT-04 — Backing shortfall blocks new supply

New token-funded supply rejects a positive shortfall between floored supplied claims and tracked cash plus ceiled debt value, measured in
native token units with saturating arithmetic. Recapitalization fills at most that shortfall, refunds excess and mints no shares. It does
not restore a written-down index. The non-zero supply-index floor can leave claims that still need recapitalization.

<a id="inv-acct-05"></a>
<a id="inv-acct-05--positive-value-must-change-shares"></a>
### INV-ACCT-05 — Positive position settlement requires shares

Positive supply and borrow amounts must mint shares; positive net repayments and gross withdrawals must burn shares. Positive same-asset
net settlement must burn both supply and debt shares. Zero-share results revert on these boundaries. This is not a rule for every cash
credit or fee: recapitalization mints no shares, and protocol-reward conversion can floor to zero shares.

<a id="inv-acct-06"></a>
### INV-ACCT-06 — Revenue claims respect accounting bounds

Revenue payout cannot exceed tracked cash or the floored revenue claim. A full payout burns all revenue shares; a cash-limited payout
burns a proportional ceiling of shares. Positive payout cannot burn zero shares. Claims enforce the utilization gate and reject zero
total supply with outstanding debt. These checks do not establish full market backing.

<a id="inv-acct-07"></a>
<a id="inv-acct-07--borrow-draws-leave-a-liquidation-cash-buffer"></a>
### INV-ACCT-07 — Borrow draws reserve a cash buffer

Borrow debt minting checks that cash minus the requested draw covers 200 BPS of the current floored supplied token value, with half-up
BPS rounding. Strategy debt openings use the same check on gross principal before withholding any fee. Exits do not preserve this
buffer, and it does not guarantee that every later liquidation can obtain cash.

<a id="inv-acct-08"></a>
<a id="inv-acct-08--utilization-stays-below-the-market-ceiling"></a>
### INV-ACCT-08 — Selected operations enforce the utilization ceiling

Borrow debt minting, non-liquidation withdrawal and revenue claims reject utilization above the market ceiling. The gate uses
half-up-valued debt divided by half-up-valued supply and skips zero total supply or a ceiling at least one RAY. Liquidation withdrawal
skips this gate; accrual and bad-debt writeoff do not maintain a universal utilization ceiling.

<a id="inv-acct-09"></a>
<a id="inv-acct-09--exits-cannot-leave-debt-without-supply"></a>
### INV-ACCT-09 — Selected exits cannot leave debt without supply

Withdrawal, same-asset net settlement and revenue claims reject a resulting zero total supplied-share balance with non-zero debt
shares. This check prevents that particular empty-supply state; it does not establish full backing or a liquidation cash reserve.

<a id="inv-idx-01"></a>
### INV-IDX-01 — Borrow index is monotone and bounded

Both indexes start at one RAY. Successful accrual under validated rate parameters cannot lower the borrow index and caps it at the
protocol-wide constant 10^36 raw RAY. At that ceiling it stops growing, so further accrual produces no borrower interest. Bounded indexes
do not guarantee representable debt values: value overflow can revert accrual before the ceiling is reached.

<a id="inv-idx-02"></a>
### INV-IDX-02 — Supply index is bounded

The supply index remains within the inclusive protocol-wide bounds 10^24 to 10^36 raw RAY. Interest distribution cannot lower it;
bad-debt writeoff applies the lower bound. These constants are not per-market governance settings. The lower bound keeps conversion
away from zero but can preserve an unbacked residual claim.

<a id="inv-idx-03"></a>
### INV-IDX-03 — Bad debt may lower the supply index

Debt writeoff reduces claims through that debt market's supply index, including claims represented by revenue shares. The reduction uses
two floors and then the non-zero index floor; zero supplied value is a no-op. A strict decrease is not guaranteed at the floor. Seizure
accrues its market first, so comparing snapshots across time can hide the writeoff behind intervening interest. Each pool seizure writes
only its identified market.

<a id="inv-idx-04"></a>
<a id="inv-idx-04--accrual-is-time-consistent"></a>
### INV-IDX-04 — Accrual shares one bounded-step calculation

No elapsed time means no accrual or accrual-timestamp advance. Longer intervals use forward chunks of at most 31,556,926,000 milliseconds.
Mutating accrual and read-only index projection share the same step calculation, including revenue shares in subsequent supply totals.
Each step selects its rate from starting utilization. Different call cadences can change rates and rounded results; there is no general
cadence-independence, monotonic-partition or exact continuous-exponential guarantee.

<a id="inv-idx-05"></a>
<a id="inv-idx-05--accrued-interest-is-fully-assigned"></a>
### INV-IDX-05 — Interest allocation retains explicit rounding limits

At the RAY-value split, accrued borrower interest equals supplier rewards plus the reserve-factor fee. Supplier rewards not reflected in
the updated supply index are added to the protocol reward. Converting that reward into revenue shares floors at the new supply index and
caps at remaining total-supply share headroom. This final conversion can leave reward value unrepresented; exact conservation of booked
supplier and revenue value is not guaranteed.

<a id="inv-oracle-01"></a>
<a id="inv-oracle-01--valuation-fails-closed"></a>
### INV-ORACLE-01 — Required valuations fail closed

A missing or unusable required price aborts a valuation-dependent operation,
including liquidation. Strict reads reject resolution errors, staleness,
disagreement, nonpositive prices, and sanity-band violations. Diagnostic
`quotes` may retain a nonzero candidate with `valid=false`; that candidate is
not an accepted valuation.

<a id="inv-oracle-02"></a>
### INV-ORACLE-02 — A dual source requires both legs

A configured two-source price requires both usable legs. Accepted prices are
their integer midpoint, rounded down, within their input range and the final
sanity band. One surviving leg never becomes a fallback. A partial reading is
unusable; a stale surviving leg reports staleness before disagreement.
Single-source configurations are admitted separately and retain their
freshness, positivity, and sanity checks.

<a id="inv-oracle-03"></a>
<a id="inv-oracle-03--one-transaction-sees-one-snapshot"></a>
### INV-ORACLE-03 — A context retains fetched prices

The controller context retains each fetched asset price; later fetches request
only missing assets. A missing cached price fails closed. Aggregator sessions
also cache resolved keys. These caches preserve repeated valuations within
their context, not identical observation timestamps across sources or a
transaction-wide snapshot shared by independent contexts.

<a id="inv-oracle-04"></a>
<a id="inv-oracle-04--future-dated-feeds-are-not-accepted"></a>
### INV-ORACLE-04 — Observation timestamps have a bounded future allowance

Feed timestamps beyond ledger time plus 60 seconds are discarded; the exact
boundary is allowed. Multi-feed reads check both package and write timestamps
after converting milliseconds to whole seconds. Reflector reads check each
observation timestamp. A discarded required leg makes its single-source or
dual-source configuration unusable under INV-ORACLE-01/02.

<a id="inv-risk-01"></a>
### INV-RISK-01 — Risk-increasing actions re-prove solvency

Ordinary borrowing, withdrawal and account strategies apply final solvency gates after pool mutations. With debt remaining, LTV-weighted
collateral must cover debt, health factor must be at least one, and LTV-weighted collateral must meet the nonzero configured floor.
Debt-free accounts skip these checks. Listed LTV snapshots are refreshed before the shared gate. Listing/count checks precede entry;
spoke-cap checks use returned pool deltas and revert the entire transaction on failure. Supply, repayment and liquidation do not
universally run this final gate.

<a id="inv-risk-02"></a>
### INV-RISK-02 — Conservative valuation biases safety

Collateral used for risk gates and its LTV/threshold weights round down; risk debt rounds up; health factor divides downward with
saturation. LTV weighting uses the smaller stored LTV and liquidation threshold. Unweighted total collateral uses half-up rounding,
including the total used for bad-debt eligibility; display debt helpers can also use half-up rounding.

<a id="inv-risk-03"></a>
### INV-RISK-03 — Risk configuration is coherent

Admitted listing configuration requires LTV strictly below liquidation threshold. Liquidation bonus and fees cannot consume more collateral than the protocol permits.

<a id="inv-risk-04"></a>
### INV-RISK-04 — Position and delegate counts are bounded

New supply and borrow slots must fit their configured counts; account grants cannot exceed 16 delegates. Existing-asset top-ups remain
admitted after a limit is lowered below the current count. Credit-mode receivers obey the same new-slot limit. These bounds constrain
state size; they do not alone establish worst-case transaction-budget sufficiency.

<a id="inv-liq-01"></a>
### INV-LIQ-01 — Only unhealthy debt can be liquidated

Ordinary liquidation requires outstanding debt and health factor below one. The caller authenticates but need not own the target; owners
may self-liquidate. Credit receivers must be different accounts in the same spoke, in Normal mode, controlled by the liquidator as owner
or active delegate. `Credit(0)` creates a receiver even in a deprecated spoke. This relaxes spoke admission for cleanup, not ownership
or position limits.

<a id="inv-liq-02"></a>
### INV-LIQ-02 — Repayment and seizure stay coupled

Repayment planning caps each input by actual debt and the liquidation curve, then trims excess before transfers. Planned refunds
describe unused input that is never pulled. Seizure is pro-rata and capped by held collateral; rounding can leave a repayment with no
payable seizure. Transfer mode burns shares and pays underlying after its fee. Credit mode conserves seized shares as receiver credit
plus a ceiling-rounded fee on bonus shares; that fee reclassifies existing supply as revenue. The share move requires no collateral cash
and reduces same-spoke usage only by the fee. New receiver assets need a current listing; existing receiver risk stamps stay unchanged.

<a id="inv-liq-03"></a>
### INV-LIQ-03 — Under-delivery reduces seizure

Receipt below the planned repayment floor-scales seizure amounts, transfer fees, seized shares and bonus shares by measured/planned USD.
Credit fees are recomputed from scaled bonus shares. Repayment USD is capped per leg at its planned value; it is not an exact identity
with every rounded debt-share reduction or refund.

<a id="inv-liq-04"></a>
### INV-LIQ-04 — Bad-debt socialization is explicit and total

Permissionless cleanup requires debt greater than total collateral and collateral at or below the fixed $5 dust threshold. Owner-only
forced cleanup omits the dust cap. Both require debt, readable account/NFT state, valid required prices and no active flash guard.
Listing flags and global pause do not block standalone cleanup.

Cleanup reclassifies all remaining collateral shares into revenue, writes off all remaining debt against its own market, releases spoke
usage and atomically removes account entries and the NFT. It does not net same-market supply against debt. Standalone cleanup emits
`CleanBadDebtEvent` with pre-cleanup USD totals, without a controller position-update batch.

Limit: neither ordinary liquidation nor bad-debt cleanup applies a final account-health/full-backing assertion. The index floor can
leave a backing shortfall; recapitalization fills only that shortfall and does not restore the lost index or deleted account.

<a id="inv-halt-01"></a>
### INV-HALT-01 — Global pause blocks new risk

Global pause blocks supply, borrowing, flash/strategy entry, delegate grants, index updates, revenue claims and threshold refresh.
Withdraw, repay, liquidation, bad-debt cleanup, recapitalization, account renewal and delegate revocation remain callable under their
separate gates. Pause therefore does not promise every maintenance verb remains available.

<a id="inv-halt-02"></a>
### INV-HALT-02 — Frozen, paused, and no_seize gate different legs

Listing flags remain independent. Entry rejects `paused` and `frozen`; user exits and liquidation repayment reject `paused`; ordinary
liquidation seizure rejects only `no_seize`. Missing listings pass the flag helper, while entry and a new Credit receiver asset
separately require a listing. Nonzero no-seize collateral can block the whole pro-rata liquidation, including when supplied after the
flag is set. Zero-unit planned seizure legs are omitted before that check. Standalone bad-debt cleanup bypasses these flags.

<a id="inv-halt-03"></a>
### INV-HALT-03 — Caps are literal and exit-safe

Zero supply/borrow cap admits no positive entry. Entry compares scaled usage with the asset-unit cap converted at the returned live
index. Exit consumes no cap: missing usage rows and zero deltas are no-ops; stored usage cannot become negative. Same-spoke liquidation
credit bypasses entry caps and books the fee as an exit.

<a id="inv-stor-01"></a>
### INV-STOR-01 — Persistent state has lifecycle discipline

Successful user-entry reads renew their TTL; metadata/delegate writes renew theirs. Position-map writes rely on the surrounding flow to
renew live sibling entries. Account removal deletes metadata, both position maps and delegates and burns the NFT atomically.
Empty-account removal is flow-dependent; repayment deliberately persists only debt without an empty-account deletion check.

<a id="inv-stor-02"></a>
### INV-STOR-02 — NFT TTL renewal is asymmetric with account renewal

Controller account entries and NFT entries have distinct lifetimes and renewal paths. INV-STOR-02a through INV-STOR-02d separate
contract renewal, dependency behavior and restoration assumptions.

<a id="inv-stor-02a"></a>
#### INV-STOR-02a — The NFT instance renews with account lifecycle

NFT mint and burn renew the instance holding the controller pointer, collection metadata and id counter. Explicit renew and upgrade also
renew it; ordinary ownership reads do not provide the same lifecycle renewal.

<a id="inv-stor-02b"></a>
#### INV-STOR-02b — Ownership entries renew on three explicit paths

Mint and permissionless NFT `renew(token_id)` extend Owner and current-owner Balance entries using the 30-day threshold and 120-day user
window. Owner-authenticated controller `renew_account` renews account entries and invokes NFT renewal. NFT renewal alone does not renew
controller account maps or OZ enumeration entries. TTL extensions neither transfer authority nor shorten a lifetime.

<a id="inv-stor-02c"></a>
#### INV-STOR-02c — Passive ownership reads renew on OZ's shorter window

The pinned OpenZeppelin `owner_of` extends Owner to 30 days when it reaches its 29-day threshold; it does not add 30 days on each read
or shorten a longer TTL. Controller activity can therefore keep account entries alive longer than NFT ownership entries.

<a id="inv-stor-02d"></a>
#### INV-STOR-02d — An archived `Owner` entry must be restored before use

Archived persistent entries must be restored before their state can be consumed. Account-loading liquidation paths and NFT burn resolve
ownership; NFT renewal also needs an ownership read. Simulation/preparation and restore funding belong to the transaction integration,
not a contract guarantee that every dormant position remains immediately accessible.

<a id="inv-stor-03"></a>
### INV-STOR-03 — Account existence and NFT existence are paired

Production creation pairs an account id with its NFT; production deletion removes account entries and burns that NFT in one transaction.
Ownership-resolving loads fail closed on unresolved NFTs. This lifecycle rule does not imply matching TTLs or that `account_exists`
checks the NFT: that view checks metadata only.

<a id="inv-flash-01"></a>
<a id="inv-flash-01--flash-repayment-is-exact"></a>
### INV-FLASH-01 — Cash flash repayment is exact

For pre-loan pool balance B, principal P and fee F, the pool requires balance
B-P after payout and again after the callback. It requires receiver allowance
at least P+F, pulls exactly P+F, then requires B+F. A direct callback push fails;
extra allowance is permitted. Cash flash loans create no account debt and do
not use account strategy finalization.

<a id="inv-flash-02"></a>
<a id="inv-flash-02--monetary-reentrancy-is-blocked"></a>
### INV-FLASH-02 — Protected monetary entry rejects callback reentry

Protected monetary entrypoints reject an active flash guard. Six guarded
windows cover cash flash loan, flash-position funding/callback, router call,
strategy withdrawal, strategy borrowing and Blend submit. Nested windows
preserve an outer guard. This does not wrap every token call or freeze account
NFT transfers and public risk views; external-host reachability remains separate
from a native test fixture.

<a id="inv-strat-01"></a>
<a id="inv-strat-01--router-authority-is-narrowly-scoped"></a>
### INV-STRAT-01 — Controller router authority binds one input transfer

The controller authorizes one exact
`token_in.transfer(controller, configured_router, amount_in)` invocation with
no sub-invocations. This is invocation authorization, not a token allowance.
It ignores the router's return value, rejects input-balance growth, and rejects
measured spending above amount_in.

<a id="inv-strat-02"></a>
<a id="inv-strat-02--strategy-settlement-is-measured-and-solvent"></a>
### INV-STRAT-02 — Account strategies settle measured flows and final risk

Distinct-token swaps require positive measured controller output and refund
the current swap's unspent controller-held input to the caller. This does not
refund router-internal residuals: the router accrues those as admin revenue
within its per-token limit and rejects larger residue. The route minimum is
checked against the router's output vault after fees, before payout; it does
not guarantee the recipient's measured receipt. The controller independently
checks positive output and final account risk, not the payload minimum.

The six account strategies refresh listed supply LTV and apply the final
collateral-coverage, health and collateral-floor gates before persistence.
Debt-free accounts skip those three numerical gates. Cash flash loans use
separate pool settlement. These statements do not imply ordinary supply or
repay performs the same final account check.

<a id="inv-strat-03"></a>
<a id="inv-strat-03--external-integrations-run-against-an-allowlist"></a>
### INV-STRAT-03 — Blend migration requires an approved pool

Migration rejects a Blend pool absent from the controller's governance-managed
approval list. This is destination admission, not proof of permanent external
code integrity. The router's separate token whitelist only selects fee placement;
it is not token, venue or pool admission.

<a id="inv-strat-04"></a>
<a id="inv-strat-04--flash-position-cannot-round-trip-to-a-closed-account"></a>
### INV-STRAT-04 — Flash position retains debt and supply

Flash position mints debt without origination fee and never automatically repays
returned debt tokens. Each declared collateral minimum is nonnegative and at
least one must be positive. The controller measures callback receipts against
those minima, deposits positive receipts and remeasures the pool's receipt.
Before and after account finalization, the borrowed market must retain positive
scaled debt and the account must retain supply.

Returned debt can become declared collateral, return to caller when refund-listed,
or remain uncredited when in neither list. Refunds cover only positive callback
balance changes. An already healthy account may support new debt with little
additional collateral; an open solvent position does not prove the receiver
spent the borrowing on collateral.
