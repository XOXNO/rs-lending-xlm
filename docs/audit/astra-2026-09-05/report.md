# Astra public-mutation security review

**Revision:** `99613335b410f70ff42dd99d13ff530f6adaee67`  
**Date:** 2026-09-05  
**Result:** No new vulnerability confirmed in this review. Existing risks below remain material. This result is a source review with local execution evidence, not a security certification of the mainnet deployment.

## Scope and attacker

Reviewed controller public mutations and their paths through the pool, price aggregator, and shared arithmetic. The attacker can create accounts, submit arbitrary public-call arguments, supply and borrow listed assets, compose transactions, use routes/callbacks where allowed, liquidate other accounts, and invoke permissionless maintenance. Account owners may also act adversarially against other users and the pool.

Governance, authorized oracle operators, the Soroban host, and admitted external contracts are trust boundaries. A user cannot substitute an arbitrary token, oracle, or pool for a configured address. Measured-transfer handling was examined for non-exact delivery; it cannot establish the honesty of an arbitrary token's balance implementation. Router quality and callback behavior were considered at their controller boundaries. Upstream contract implementations, governance as a whole, and deployed state were not audited exhaustively.

The pinned source inventory contains 132 tracked files across the four requested source directories. Its SHA-256 manifest is [source-sha256.txt](evidence/source-sha256.txt), with digest `fe2869b7b408c0cb2d1ecb122106a4d83f261fc3c85e806e120110fe86351c68`. This is an inventory and integrity check, not a claim that every line received equal scrutiny. Production source remained unchanged.

## Risk-ranked endpoint groups

Priority denotes review priority, not a finding's severity.

| Priority | Public mutations | Main failure modes examined | Result |
|---|---|---|---|
| 1 | `liquidate`: `Transfer`, `Credit(0)`, `Credit(existing)`; `clean_bad_debt` | Unpaid seizure, toxic partial closes, rounding, duplicate payments, wrong receiver, fee minting, cash starvation, cleanup ordering, account/pool/spoke divergence | No new confirmed defect; both modes exercised in composition probes |
| 2 | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend`, `flash_position`, `flash_loan` | Temporary unbacked debt, callback reentry, stale positions, route overspend, stranded or stolen refunds, gross/net fee confusion, missing final risk checks | No new confirmed defect; external trust and route quality remain assumptions |
| 3 | `borrow`, `withdraw` | Stale LTV reuse, unauthorized recipient, rounding debt away, cap/utilization bypass, collateral removal without rechecking all positions | Shared final risk gate and conservative rounding traced |
| 4 | `supply`, `repay`, `recapitalize` | Over-crediting input, duplicate legs, refund theft, full-close rounding, supplying into impaired markets, third-party position injection | Measured input and pool accounting traced; refund/composition tests passed |
| 5 | `claim_revenue`, `update_indexes`, `update_account_threshold`, delegates, renewal | Permissionless value diversion, stale risk refresh, accrual cadence, delegated authority after transfer, callback mutation | No new authorization bypass; documented cadence/delegate/liveness limits remain |

Pool mutation entry points require their configured owner. A direct user call carrying fictitious positions or an unfunded payment is therefore not a demonstrated attack path. Controller authorization and the token transfer that precedes each owner-authorized pool mutation were reviewed together.

## Liquidation and cleanup

The execution path is plan, measured repayment, scaled seizure, account persistence, then eligible bad-debt cleanup. Estimate and execution share the plan builder. It rejects debt-free/healthy accounts, normalizes positive payments by hub asset, caps payments against ceiling-rounded debt, and applies listing gates before seizure.

Relevant anchors:

- `contracts/controller/src/positions/liquidation/mod.rs:46` — orchestration.
- `contracts/controller/src/positions/liquidation/plan.rs:19` — shared plan and eligibility.
- `contracts/controller/src/positions/liquidation/math.rs:113` — debt-payment cap.
- `contracts/controller/src/positions/liquidation/math.rs:173` — ideal-payment normalization and full-close gate.
- `contracts/controller/src/positions/liquidation/apply.rs:31` — measured debt receipts.
- `contracts/controller/src/positions/liquidation/bad_debt.rs:15` — cleanup and NFT removal.

### Bonus and partial-close algebra

Let collateral value be C, threshold-weighted collateral W, debt D, pro-rata seizure weight q = W/C, bonus b, and repayment x. Before discrete rounding and clamping:

```text
post_HF = (W - q * (1 + b) * x) / (D - x)
HF does not fall when b <= C/D - 1.
```

`curve.rs:113` derives the corresponding bonus ceiling from HF and seizure weight. In the solvent but toxic band where that ceiling is nonnegative and below the base bonus, the normalization path refuses an insufficient partial payment with `FullCloseRequired`. A ceiling-rounded valuation tolerance handles final debt-unit rounding. Already insolvent accounts have separate unwind behavior; improving their HF is not a universal liquidation invariant.

The apparent bypass where a full-close payment under-delivers was challenged. When the full-close plan clamps every collateral leg to the entire position, scaling those seizures by the delivered fraction approximately preserves the original C/D. The missing post-HF assertion alone does not establish a toxic-partial attack. No executable counterexample was established. This algebra is a screening argument, not proof over every discrete mixed-decimal state.

### Transfer and credit accounting

Transfer mode burns the borrower's supply shares, debits pool cash, and pays underlying after withholding the fee. It is cash-dependent. Credit mode debits the borrower and credits another account in scaled shares; it does not require collateral-market cash.

Credit mode enforces this exact identity:

```text
fee_shares = ceil(bonus_shares * fee_bps / 10_000)
borrower_debit = liquidator_credit + fee_shares
delta_pool_supplied = 0
delta_pool_cash = 0
delta_pool_revenue = fee_shares
delta_spoke_supply_usage = -fee_shares
```

The fee reclassifies existing supply shares. It does not use the transfer-mode fee-minting path. Full seizures use the exact original scaled position; partial seizures floor their scaled conversion. When repayment under-delivers, seizure shares and bonus-share base are scaled before the fee is recomputed. See `math.rs:249`, `math.rs:377`, `math.rs:465`, and `apply.rs:139` in the liquidation directory.

Credit receivers must be in the same spoke, in normal mode, and owned by or delegated to the liquidator. The target account cannot receive its own seizure. New accounts can be created in deprecated spokes to permit liquidation. Existing receiver positions retain their own risk tuple; new slots use current listing parameters. Supply caps do not block this within-spoke transfer of existing shares. Position limits still apply.

Seizures use `no_seize`; collateral `paused`/`frozen` flags do not themselves veto seizure. Debt-payment legs still respect the listing's pause gate. Eligible cleanup requires debt above collateral and collateral at or below the dust threshold. Cleanup removes supply/debt usage and the account NFT; its known loss-allocation issue is recorded below.

## Strategies, borrow, and withdraw

| Flow | Traced control |
|---|---|
| `multiply` | Account guard, measured inputs and route output, gross debt booking, shared final solvency |
| `swap_debt` | New debt and route funding, old-debt repayment, excess handling, final check over remaining debt |
| `swap_collateral` | Withdrawal, measured controller receipt, route spend/output bounds, destination admission, final solvency |
| `repay_debt_with_collateral` | Same-market net settlement or withdraw/swap/repay; exact returned positions and measured refund delta |
| `migrate_from_blend` | Unique debt assets, consolidated withdrawals, bounded repayment authorization, leftover reconciliation, final solvency |
| `flash_position` | Allowed debt market, contract receiver checks, collateral uniqueness by underlying, disjoint refund assets, callback guard, final collateral/debt check |
| `flash_loan` | Enabled market, guarded callback, exact pool-balance checks and repayment pull |

`contracts/controller/src/positions/mod.rs:96` restamps every listed supply LTV before applying the final account gates. Borrow and withdraw use this path; `contracts/controller/src/strategies/mod.rs:71` shares it through strategy finalization. The checks include LTV-weighted coverage, HF at least one, and the configured minimum collateral while debt remains. A debt-free account has different requirements.

Pool strategy origination books gross debt and transfers principal minus any fee. The fee remains pool backing and becomes protocol revenue. Strategy creation calls the ordinary debt-mint helper, including liquidity-buffer and utilization checks: `contracts/pool/src/ops/strategy.rs:58` and `contracts/pool/src/ops/borrow.rs:70`.

Partial withdrawal burns ceiling-rounded supply shares. Full withdrawal burns the complete position and pays its floor-valued amount. Repayment burns partial debt shares with floor rounding; a full close requires the ceiling-valued debt. Same-market net settlement uses `min(request, floor(supply), ceil(debt))` and rejects a positive settlement that burns zero shares. A zero-supply/nonzero-debt market cannot be created by that path.

## Receipts and refunds

Controller supply, repayment, recapitalization, liquidation repayment, and strategy funding paths use measured receipts. Pool cash and share mutations consume the measured amount. Raw duplicate legs are aggregated where required so an unchanged position snapshot cannot be spent twice within a batch.

For strategy repayment, the controller measures pool receipt, snapshots its own balance after sending input, executes repayment, then forwards only the positive refund delta. It does not refund its entire token balance. Anchor: `contracts/controller/src/strategies/legs.rs:49`.

Recapitalization applies only the current backing shortfall:

```text
applied = min(received, max(supplier_claims - cash - debt_value, 0))
refund = received - applied
```

Pool accounting adds only `applied` to cash and sends the excess to the payer. Anchors: `contracts/controller/src/keepers.rs:46`, `contracts/pool/src/ops/recapitalize.rs:44`, and `contracts/pool/src/guards.rs:58`.

An outbound token that taxes delivery can reduce what a refund recipient receives. That fact alone is not evidence that the controller retained or stole the difference. Arbitrary rebases and dishonest balance implementations exceed what measured deltas can guarantee. Existing adversarial-token tests were run; no new reachable refund diversion was established.

## Oracle and shared arithmetic

The aggregator fails closed on missing, stale, deviant, nonpositive, or sanity-band-violating prices. A configured two-source oracle does not silently accept a single surviving leg. Caching is per resolution session; nested cache hits recheck cycle/depth bounds. Reflector TWAP reads require the configured observation count, ordered timestamps, and minimum spacing; the oldest observation supplies the age bound.

Aquarius LP reads recheck token/share bindings, pool kind, and decimals. Constant-product fair value uses the geometric mean of reserve values. Stable-LP value uses the solved invariant D multiplied by the cheaper underlying price, divided by share supply. This avoids directly valuing LP collateral from a manipulable spot reserve ratio. Pool liquidity and sanity bounds remain relevant; this review does not verify the live external pools.

Core anchors: `contracts/price-aggregator/src/engine.rs`, `providers/reflector.rs:154`, `providers/aquarius.rs:66`, `common/src/oracle/lp.rs`, and `common/src/oracle/lp_stable.rs`. Directed fixed-point conversions, interest/revenue shares, supply-index write-downs, and cap conversions were examined with the pool/controller call sites. Extreme-domain overflow is an availability concern; checked failure does not make every allowed configuration operationally safe.

## Candidate dispositions and existing risks

These are not new findings from this run. A documented behavior can still carry serious operational or economic consequences.

| Candidate or concern | Disposition and evidence |
|---|---|
| Credit-mode fee creates unbacked supply | Rejected: fee reclassifies existing shares; exact borrower/receiver/revenue and spoke-usage identities hold in source and executed probes |
| Repay/refund can spend preexisting controller funds | No new path reproduced; strategy refund uses a measured delta, and existing excess/underspend tests passed |
| Borrow/withdraw can use stale generous LTVs on untouched collateral | Rejected at this revision: final gate restamps every listed supply LTV |
| Third-party dust supply can inject an arbitrary new collateral slot | Rejected at this revision: a non-owner/non-delegate can only top up an existing slot |
| Old global-pause bypass claims | Obsolete for the current intended policy: exits/repayment/liquidation remain open by design; strategies are globally gated. Listing pause and seizure policy are distinct |
| Full-close requirement bypassed by under-delivery | No violation established; proportional all-collateral seizure challenges the apparent counterexample. Discrete cases are not exhaustively proved |
| Same-market bad-debt cleanup socializes gross debt while retaining collateral as revenue | Confirmed existing design: `bad_debt.rs:15`; ADR-0021 explicitly defers netting. This changes who absorbs losses even though aggregate value is conserved |
| Supply-index floor can leave claims above backing | Confirmed existing boundary: `contracts/pool/src/interest.rs:81`, pool backing gates, and recapitalization. No new public loss-generation sequence established |
| Extreme index growth can freeze operations through overflow | Existing numeric-domain limitation, documented in `docs/explanation/threat-model.md` and exercised by `large_positions_and_long_horizons.rs`. Plausibility depends on market size, rates, and time |
| Permissionless accrual cadence changes realized interest | Existing behavior: more frequent accrual can change utilization/rates. `accrual_partition_bound.rs` passed; partition neutrality must not be assumed |
| Delegate grant reactivates after ownership A -> B -> A | Confirmed existing semantics, documented in `docs/explanation/threat-model.md:285`; no completed independent reviewer validation. Revocation is tied to the granting owner, not an ownership epoch |
| Coarse-decimal collateral and dust fee erase liquidator proceeds | Existing arithmetic/economic limitation in `docs/reference/numeric-bounds.md:289`. Credit fees avoid the underlying-unit fee bump, but plan construction still skips collateral legs below one asset unit |
| One unavailable collateral price blocks liquidation/valuation | Intentional fail-closed dependency. Small collateral does not exempt its oracle. This is a liveness tradeoff, not proof of an oracle bypass |
| Maximum-position liquidation fits production budgets | Unverified. Native benchmark tests tolerate some mock-auth budget failures; their pass result cannot establish live budget fit |

The most useful next assurance work is deployment-specific: establish deployed WASM/config equality, measure liquidation at active maximum position counts and real oracle compositions, and assess rate/cap/decimal settings against the documented economic and numeric boundaries. These were not live-tested here.

## Execution evidence

All final commands exited successfully:

```sh
RUSTC_WRAPPER= cargo test --offline -p controller -p pool -p common -p price-aggregator --lib
RUSTC_WRAPPER= cargo test --offline -p test-harness --tests
RUSTC_WRAPPER= cargo test --offline -p test-harness --test astra_audit -- --nocapture
```

| Run | Passed | Failed | Evidence |
|---|---:|---:|---|
| Four scoped crate unit suites | 1,055 | 0 | [unit-tests.log](evidence/unit-tests.log) |
| Existing test-harness suites, including its library tests | 1,063 | 0 | [integration-tests.log](evidence/integration-tests.log) |
| Added audit probes | 2 | 0 | [probes-final.log](evidence/probes-final.log) |
| Total | **2,120** | **0** | Counts exclude the corrected fixture's initial failed attempt |

The existing harness run began before the added test target existed. The separate final probe command supplies its two results. Tests include cases that intentionally pin known limitations; a passing count does not mean those limitations were fixed. The initial compiler-cache failure was resolved by clearing `RUSTC_WRAPPER` for these commands, without production edits.

The added test file is `tests/test-harness/tests/astra_audit.rs`. It enumerates every live position NFT, including newly created credit receivers, and compares these differences with fixture baseline after each transition:

```text
pool_supplied - pool_revenue - sum(all_account_supply_shares)
pool_borrowed - sum(all_account_debt_shares)
actual_pool_token_balance - tracked_pool_cash
```

It additionally checks revenue is a subset of supply, nonnegative cash/debt, zero controller balances for the fixture assets, and exact spoke usage against account positions. Fixture tokens use 6, 18, and 8 decimals.

The deterministic probe composes accrual, credit to new/existing receivers, transfer liquidation, repayment overpayments, receiver withdrawals, normal origination followed by a price collapse and bad-debt cleanup, recapitalization/refund, and supply/withdraw after the write-down. The initial 55-cent USDC fixture correctly hit `FullCloseRequired`; 60 cents exercises the intended partial-liquidation branch. That fixture correction did not change protocol behavior.

The stateful probe runs 160 fixed-seed attempted transitions. Successful counts for supply, borrow, repay, withdraw, transfer liquidation, credit liquidation, time/price/index changes, and revenue claims were `[26, 12, 8, 14, 1, 1, 17, 23]`; rejected counts were `[0, 3, 9, 9, 20, 17, 0, 0]`. It asserts conservation after both success and rejection. The two liquidation successes are useful composition evidence, not broad randomized liquidation coverage. These probes do not establish authorization correctness because the fixture uses mocked authorization; authorization was also inspected in source and existing rejection tests.

## Verification limits

- No mainnet transaction, state snapshot, deployed-bytecode comparison, ownership/config attestation, or production simulation was performed.
- Native Soroban tests use mocks and frequently unlimited budgets. No full WASM budget/footprint proof or new formal-verifier run was completed.
- Five context-free reviewers were dispatched. One was stopped by a cybersecurity service filter; four stopped on account usage limits. None returned a completed independent final review. Preliminary observations were leads only. The conclusions above are the primary reviewer's source/test assessment.
- Graph discovery had incomplete results and some inaccurate call links; source and executed tests supplied the authoritative checks.
- This is a manual risk-focused audit record. It is not a sealed Codex Security scan, exhaustive symbolic execution, or a claim that all possible transaction sequences were explored.
- No production fix, commit, push, or external disclosure was made. Only this report, evidence files, and the local audit test were added.
