# Scanner brief: XOXNO Lending (rs-lending-xlm)

XOXNO Lending is a lending protocol of Stellar Soroban contracts, live on
mainnet. This brief sets scope, execution model, focus areas, severity scale
and report format. Read these authoritative documents first:

- [docs/reference/invariants.md](../docs/reference/invariants.md): properties that must hold, with stable INV ids. Cite them.
- [docs/reference/formulas.md](../docs/reference/formulas.md): exact arithmetic, rounding direction at each boundary, numeric limits.
- [docs/reference/endpoints.md](../docs/reference/endpoints.md): each entrypoint's authority and pause behavior.
- [docs/explanation/threat-model.md](../docs/explanation/threat-model.md): trust roots, assumptions, residual risks, STRIDE register.
- [docs/explanation/decisions.md](../docs/explanation/decisions.md): design rationale (ADR-0001..0023).
- `scripts/permissionless_entrypoints.txt`: every entrypoint any address may call, and why it is safe.

## Components

In scope, most important first:

1. `contracts/controller`: accounts, authorization, risk gates, liquidation,
   bad debt, flash loans, the six account strategies, spoke caps.
2. `contracts/pool`: custody, cash, supply, debt, and revenue share books, indexes,
   accrual, flash-loan settlement.
3. `common/`: WAD/RAY/BPS math, rate curves, accrual, oracle validation,
   listing validation, TTL.
4. `contracts/price-aggregator`, `contracts/xoxno-oracle`: source admission,
   dual-source agreement, staleness, sanity bands, signer quorum.
5. `contracts/governance`: operation tiers, delays, roles, emergency tightening.
6. `contracts/curator`: registrar roles, hub and spoke ownership, cap grants,
   proposals queued through governance.
7. `contracts/position-nft`, `contracts/swap-aggregator`,
   `contracts/defindex-strategy`, `interfaces/`.

Out of scope: `mock/`, `tests/`, `certora/`, `vendor/`, `services/`, `skills/`,
`scripts/`, `configs/`, `docs/`, CI, and code behind the `testing` cargo
feature unless it reaches a deployable WASM (ADR-0017). A flaw wholly inside
`soroban-sdk`, the Soroban host, OpenZeppelin Stellar contracts, Blend,
Aquarius, Soroswap, Reflector, or RedStone belongs to that project.

## Attacker

Any address. It can deploy contracts and hold many lending accounts. It can
also be its own delegate, liquidator, or Credit receiver, or be a flash-loan or
flash-position receiver. It can name a router venue in a swap payload and hold
any listed token. It can spread a sequence over many ledgers so interest
accrues between steps. The following parties are trusted: governance keys, role
holders (PROPOSER, GUARDIAN, ORACLE), an honest oracle signer quorum,
standalone router and oracle administrators, and the Soroban host.

## Execution model

Reason in Soroban terms, not EVM terms.

- **One transaction is one atomic invocation tree.** A trap anywhere reverts
  every state change and token transfer in it. An inconsistent intermediate
  state matters only if a later call in the same tree reads it, or if the tree
  can end with it in storage.
- **Calls take several legs, and one contract can chain calls.** `supply`,
  `borrow`, `withdraw`, `repay`, and `liquidate` take a `Vec` of legs. A
  contract can call any number of controller entrypoints in one transaction.
  `mock/script-runner` does this. Its `Op` enum lists the chainable verbs.
  `tests/test-harness/tests/composition/` proves the known-safe shapes. These
  shapes are: all legs revert together, delegate revocation and NFT transfer
  between legs, and repeated loops that extract nothing.
- **The host forbids re-entry.** It rejects a nested call into a contract
  already on the stack with a Context/InvalidAction error before any protocol
  check runs (`tests/test-harness/tests/poc_multiply_reentrancy.rs`).
  INV-FLASH-02 is defense in depth. Do not report EVM-style reentrancy.
- **Authorization is per address, per invocation tree.** `require_auth` on a
  caller-supplied address proves only control of that address. The function
  body then checks what the caller may do to `account_id`
  (`require_owner_or_delegate`, `require_account_owner`). The controller authorizes exactly one token
  transfer to the router per swap, with no sub-invocations (INV-STRAT-01).
- **Prices are cached per controller context, not per transaction.** One call
  sees one price per asset (INV-ORACLE-03). Two calls in one transaction build
  two contexts. Oracle state cannot change between the calls unless the
  attacker's own tree changes it.
- **Checked arithmetic traps.** `MathOverflow` and `DivisionByZero` revert the
  transaction. A trap that reverts only the attacker's transaction is not a
  finding. A trap forced on another account's exit, liquidation, or cleanup is
  a finding.
- **Storage can be archived.** Persistent entries have a TTL. Archived entries
  must be restored before use (INV-STOR-01..03).

## Where to concentrate

### Math and rounding

Units: RAY (10^27) applies to shares, indexes, and rates. WAD (10^18) applies
to USD values, prices per whole token, and the health factor. BPS (10,000)
applies to ratios and fees. Token base units apply to transfers and cash.
Milliseconds apply to accrual. Each expression uses one unit. Rounding favors
the protocol. Risk collateral floors at all three valuation steps. Risk debt
ceils. The health factor floors and saturates at `i128::MAX`. Supply shares
mint floor and burn ceil. Debt shares mint ceil and burn floor (see formulas.md
"Shares and token amounts").
Look for:

- A reversed direction, or two roundings compounding in the user's favor for
  a repeatable gain above the transaction fee.
- The index bounds (both start at one RAY, ceiling 10^36, supply floor 10^24)
  and what a market near them does to repayment, withdrawal, and liquidation.
- `calculate_scaled_cap`, which saturates instead of trapping (INV-HALT-03).
- Decimals 0..=18, whole-unit seizure on legs below 3 decimals, and
  collateral and debt pairs with mixed decimals (INV-LIQ-02).
- The liquidation bonus curve, its cap at `HF / LT - 1`, target-repayment
  sizing, proportional seizure, and under-delivery scaling (INV-LIQ-03).
- Accrual chunking and cadence. Can a caller time `update_indexes` to move
  value (INV-IDX-04, INV-IDX-05)?

### Accounting

The pool stores market totals only and trusts the scaled position each call
carries (INV-ACCT-10). Per market, account supply shares must sum to pool
supplied shares minus revenue shares, and account debt shares must sum to pool
borrowed shares. Cash is a tracked book, not the token balance (INV-ACCT-02).
Supply, repay, and recapitalize credit the measured balance increase
(INV-ACCT-03).
Look for:

- A controller path that writes a position from any source but the pool's
  returned mutation, or sends a stale position after an earlier leg in the
  same tree changed it.
- Cash, shares, and tokens that move in different amounts. Examples are
  same-asset net settlement, Credit-mode liquidation (debit + credit + fee must
  sum exactly), transfer fees, refunds, and flash-loan settlement
  (INV-FLASH-01).
- The backing gate, recapitalization, revenue payout, and the bad-debt index
  write-down (formulas.md "Bad debt"), where value is created or destroyed
  without the matching transfer, or socialized twice or not at all
  (INV-LIQ-04).
- Zero-share boundaries (INV-ACCT-05), the 200 BPS borrow buffer
  (INV-ACCT-07), the utilization ceiling and which operations skip it
  (INV-ACCT-08), and exits that could leave debt without supply (INV-ACCT-09).

### Authorization

The position NFT holder owns the account. Transferring or approving the NFT
moves the whole position. A delegate must be an active governance-approved
manager, granted only by the owner (at most 16). The grant is stamped with the
granting owner's address, so it can revive when the NFT returns. Third parties
may supply only into asset slots the account already holds (INV-AUTH-03),
repay or liquidate anyone, and run maintenance. Look for:

- Any way to borrow, withdraw, swap, or close against an account the caller
  neither owns nor actively delegates for (INV-AUTH-02), including through
  `Credit(0)` receivers, `account_id = 0` creation, strategies, Blend
  migration, and the DeFindex adapter's per-vault accounts.
- A third-party `supply` or `liquidate` that opens a new asset slot, changes
  a risk snapshot or spoke binding, or otherwise worsens a foreign account
  beyond what the entrypoint list states (INV-AUTH-06, INV-RISK-04).
- Any immediate guardian or oracle path that loosens a flag, band, cap, or
  delay (INV-AUTH-04, INV-AUTH-05).
- A pool mutator reachable without the controller as owner (INV-AUTH-01).
- An NFT mint, burn, or upgrade without the stored controller.

### Chained atomic execution

Unit tests cover this least. Build sequences. Ask what one contract can do in
one transaction that separate transactions cannot. Ask what a sequence over
ledgers, with accrual between steps, can do. Start from:

- Open an account with dust, borrow, then self-liquidate in Credit mode into
  a second attacker account, and repeat.
- `flash_loan` returned, then supply, borrow, and withdraw in the same tree.
  The guard covers only the callback window.
- `flash_position` or `multiply` followed by `swap_collateral`,
  `repay_debt_with_collateral` with `close_position`, or `withdraw`, with the
  final gates skipped for a debt-free account (INV-RISK-01).
- `liquidate` then `clean_bad_debt` on the same account, or cleanup of an
  account with supply and debt in the same market.
- `recapitalize`, `claim_revenue`, and `update_indexes` ordered around a
  supplier's exit so the index write-down lands on others.
- An NFT transfer or delegate change between legs to revive or outrun a
  grant.
- Two controller calls whose price contexts could disagree, and a governance
  operation executed permissionlessly between two user legs.

Every account strategy ends with the same final gates (INV-STRAT-02). What
state can the tree leave that they do not read?

## How to exercise it

The image is offline. Crates are cached. Contract WASM and test binaries
are built in `/src`.

- All tests: `cargo test --workspace --features price-aggregator/testing`.
- Integration tests: `cargo test -p test-harness`. `tests/test-harness/tests/README.md`
  maps each binary to its coverage. The builder in `tests/test-harness/src/setup`
  deploys the whole protocol with governance as the controller owner, as on
  mainnet.
- One test: `make test-match PATTERN=<substring>`. `PATTERN` is required.
- After a contract patch: `make build`, because the harness loads
  `target/wasm32v1-none/release/pool.wasm` and `position_nft.wasm`.
- Gates a patch must keep passing: `make access-control-check`, `make fmt-check`,
  `make clippy`.

Write reproducers in `tests/test-harness/tests`. Drive multi-call sequences
through `script_runner`, as in `tests/test-harness/tests/composition/helpers.rs`,
so the test proves atomicity. Use real contracts. Mock only the oracle
(`mock/mock-oracle`), the receivers, and the attacker's router.

## How you rate severity

A finding is a case where an unprivileged address, through a deployable
entrypoint, breaks an invariant in `docs/reference/invariants.md` or causes a
loss. Cite the INV id. If the finding needs a non-default listing or risk
configuration, name it and show that admission validation accepts it
(INV-RISK-03).

- **Critical**: theft of supplier, pool, or borrower funds. Debt an account
  escapes while keeping collateral. Shares, debt or cash credited without the
  matching transfer. Acting on an account without its authorization. A
  permanent freeze of user funds. Unbounded or market-wide is Critical. The
  same impact bounded to one market and a small amount is High.
- **High**: a break of INV-ACCT-10 that any address can cause. Seizure above
  what the bonus curve allows, or liquidation of a healthy account. Bad debt
  created by an accounting error, or socialized twice or never. An
  unprivileged bypass of a pause, freeze, cap, utilization ceiling, or
  governance delay. A validated price that violates the configured staleness,
  agreement, or band rules. Blocking liquidation, withdrawal or repayment for
  other accounts beyond one ledger.
- **Medium**: user-favoring rounding that repeats for a profit above the
  transaction fee. Griefing that blocks one account at a cost to the attacker.
  An event that disagrees with committed state in a way a keeper would act on.
- **Low**: rounding drift with no profitable repetition. A trap where the
  protocol would have rejected the call anyway. Resource use inside but close
  to Soroban limits.

A defect with no path to fund loss, unauthorized state change, or blocked
recovery is a bug, not a vulnerability. Do not report it.

## How to report

- A reproducing harness test, with exact command, observed and expected
  results.
- The sequence as a table: step, caller, entrypoint, arguments with units, and
  resulting state, with ledgers and accrual between steps.
- The violated INV id, the attacker's capabilities, the required protocol
  state, and the impact in token base units or USD WAD. State whether the impact is
  bounded, repeatable, or recoverable.
- A minimal patch against `main` with a regression test. Keep units explicit.
  Keep rounding in the protocol's favor (ADR-0003). Do not rename or move
  controller internal modules, because the Certora specs and unit tests mount
  them by path.
- Each root cause once, with every entrypoint that reaches it.

## Anything to leave alone

Each item is documented and accepted. Report an item only when its impact goes
beyond the cited text.

- Anything that needs a compromised governance, role, oracle-signer, or
  standalone-administrator key (STRIDE Spoof.1, Elevation.1..10).
- Fail-closed price outages that block valuation-dependent actions, including
  liquidation, and the dust-leg and LP-leg variants (DoS.1). Paused debt or
  `no_seize` collateral that blocks liquidation (DoS.2).
- Debt-free accounts that skip the final numerical gates (INV-RISK-01).
- `is_collateralizable` gates new supply only. Existing collateral keeps its
  value.
- Scaled-cap saturation at a supply index below one RAY (INV-HALT-03), the
  explicit rounding limits of interest allocation (INV-IDX-05), and the
  supply-index floor that leaves an unbacked residual (INV-IDX-02, INV-ACCT-04).
- A fall in the supply index on bad-debt socialization, and a supplier who
  exits before cleanup and avoids it (INV-IDX-03, ADR-0012).
- Liquidation ordering, MEV, liquidator profitability, zero seizure on tiny
  repayments, and whole-unit effects on legs below 3 decimals (Info.2, DoS.9,
  INV-LIQ-02).
- Public positions, prices, and pending governance operations (Info.1, Info.3).
- Token semantics outside the threat model's "Token assumptions", unbooked
  donations, and a malicious router inside the measured bounds (Tamper.3,
  Tamper.4).
- Reentrancy claims that rely on EVM behavior (Tamper.5).
