# Threat model: XOXNO Lending (rs-lending-xlm)

XOXNO Lending is a lending protocol of Stellar Soroban contracts, live on
mainnet. This file sets the scanner's scope, the execution model it must
reason in, the four areas to concentrate on, the severity scale, and the
report format. The repository documents below are authoritative; read them
before you start.

- [docs/reference/invariants.md](../docs/reference/invariants.md): every property that must hold, with stable INV ids. Cite them.
- [docs/reference/formulas.md](../docs/reference/formulas.md): the exact arithmetic, rounding direction at each boundary, and numeric limits.
- [docs/reference/endpoints.md](../docs/reference/endpoints.md): every entrypoint, its authority, and its pause behaviour.
- [docs/explanation/threat-model.md](../docs/explanation/threat-model.md): trust roots, assumptions, residual risks, STRIDE register.
- [docs/explanation/decisions.md](../docs/explanation/decisions.md): why each design is the way it is (ADR-0001..0021).
- `scripts/permissionless_entrypoints.txt`: every entrypoint any address may call, with the reason it is safe.

## Components

In scope, most important first:

1. `contracts/controller`: accounts, authorization, risk gates, liquidation,
   bad debt, flash loans, the six account strategies, spoke caps.
2. `contracts/pool`: custody, cash, supply/debt/revenue share books, indexes,
   accrual, flash-loan settlement.
3. `common/`: WAD/RAY/BPS math, rate curves, accrual, oracle validation,
   listing validation, TTL.
4. `contracts/price-aggregator`, `contracts/xoxno-oracle`: source admission,
   dual-source agreement, staleness, sanity bands, signer quorum.
5. `contracts/governance`: operation tiers, delays, roles, emergency ratchet.
6. `contracts/position-nft`, `contracts/swap-aggregator`,
   `contracts/defindex-strategy`, `interfaces/`.

Out of scope: `mock/`, `tests/`, `certora/`, `vendor/`, `services/`,
`specula/`, `skills/`, `scripts/`, `configs/`, `docs/`, CI, and code behind
the `testing` cargo feature unless it reaches a deployable WASM (ADR-0017). A
flaw wholly inside `soroban-sdk`, the Soroban host, OpenZeppelin Stellar
contracts, Blend, Aquarius, Soroswap, Reflector or RedStone belongs to that
project.

## Attacker

Any address. It can deploy contracts, hold many lending accounts, be its own
delegate, liquidator or Credit receiver, be a flash-loan or flash-position
receiver, name a router venue in a swap payload, hold any listed token, and
spread a sequence over many ledgers so that interest accrues between steps.
Governance keys, role holders (PROPOSER, GUARDIAN, ORACLE), an honest oracle
signer quorum, standalone router and oracle administrators, and the Soroban
host are trusted.

## Execution model

Reason in these terms, not in EVM terms.

- **One transaction is one invocation tree, and it is atomic.** A trap or
  error anywhere in the tree reverts every state change and token transfer in
  it. There is no partial commit. An inconsistent intermediate state matters
  only if a later call in the same tree reads it, or if the tree can end with
  it still in storage.
- **One call can carry several legs and one caller contract can chain
  calls.** `supply`, `borrow`, `withdraw`, `repay` and `liquidate` take a
  `Vec` of legs. A contract can invoke any number of controller entrypoints
  in sequence from its own frame, all in one transaction. `mock/script-runner`
  is that contract: its `Op` enum lists the verbs it can chain, and
  `tests/test-harness/tests/composition/` proves the shapes that are known to
  be safe (every leg reverts together, delegate revocation and NFT transfer
  between legs, repeated loops extract nothing).
- **The host forbids re-entry.** A contract already on the call stack cannot
  be entered again; the host rejects the nested call with a
  Context/InvalidAction error before any protocol check runs
  (`tests/test-harness/tests/poc_multiply_reentrancy.rs`). INV-FLASH-02 is
  defence in depth. Do not report EVM-style reentrancy.
- **Authorization is per address, per invocation tree.** `require_auth` on an
  address the caller supplies proves only control of that address; the body
  then checks what that address may do to `account_id`
  (`require_owner_or_delegate`, `require_account_owner`). The controller
  authorizes exactly one token transfer to the router per swap, with no
  sub-invocations (INV-STRAT-01).
- **Prices are cached per controller context, not per transaction.** One
  call sees one price per asset (INV-ORACLE-03). Two calls in one transaction
  build two contexts. The oracle contracts' state cannot change between them
  unless the attacker's own tree changes it.
- **Checked arithmetic traps.** `MathOverflow` and `DivisionByZero` revert
  the transaction. A trap that reverts only the attacker's own transaction is
  not a finding. A trap that an attacker can force on another account's exit,
  liquidation or cleanup is.
- **Storage can be archived.** Persistent entries have a TTL; archived
  entries must be restored before use (INV-STOR-01..03).

## Where to concentrate

### Math and rounding

Units: shares, indexes and rates are RAY (10^27); USD values, prices per whole
token and the health factor are WAD (10^18); ratios and fees are BPS (10,000);
transfers and cash are token base units; accrual time is milliseconds. Every
expression must stay in one unit. Rounding must favour the protocol: risk
collateral floors at all three valuation steps, risk debt ceils, the health
factor floors and saturates at `i128::MAX`, supply shares mint floor and burn
ceil, debt shares mint ceil and burn floor (formulas.md "Shares and token
amounts"). Look for:

- a site where the direction is reversed, or where two roundings compound in
  the user's favour and the gain exceeds the transaction fee when repeated;
- the index bounds (both start at one RAY, ceiling 10^36, supply floor 10^24)
  and what a market near them does to repayment, withdrawal and liquidation;
- `calculate_scaled_cap`, which saturates instead of trapping (INV-HALT-03);
- decimals 0..=18, the whole-unit seizure rules for legs below 3 decimals, and
  mixed-decimal collateral/debt pairs (INV-LIQ-02);
- the liquidation bonus curve, its cap at `HF / LT - 1`, the target-repayment
  sizing, proportional seizure, and under-delivery scaling (INV-LIQ-03);
- accrual chunking and call cadence: can a caller choose `update_indexes`
  timing to move value (INV-IDX-04, INV-IDX-05)?

### Accounting

The pool stores market totals only; each account's scaled position travels
with the call and the pool trusts it (INV-ACCT-10). For every market, account
supply shares must sum to pool supplied shares minus revenue shares, and
account debt shares to pool borrowed shares. Cash is a tracked book, not the
token balance (INV-ACCT-02); supply, repay and recapitalize credit the measured
balance increase (INV-ACCT-03). Look for:

- a controller path that writes a position from any source other than the
  pool's returned mutation, or sends a stale position to the pool after an
  earlier leg in the same tree changed it;
- a path where cash, shares and tokens move in different amounts: same-asset
  net settlement, Credit-mode liquidation (debit + credit + fee must sum
  exactly), transfer fees, refunds, flash-loan settlement (INV-FLASH-01);
- the backing gate, recapitalization, revenue payout and the bad-debt index
  write-down (formulas.md "Bad debt"): value created or destroyed without the
  matching transfer, or socialized twice or not at all (INV-LIQ-04);
- the zero-share boundaries (INV-ACCT-05), the 200 BPS borrow buffer
  (INV-ACCT-07), the utilization ceiling and which operations skip it
  (INV-ACCT-08), and exits that could leave debt without supply (INV-ACCT-09).

### Authorization

The position NFT holder owns the account; transferring or approving the NFT
moves the whole position. A delegate must be an active governance-approved
manager, is granted only by the owner (at most 16), and its grant is stamped
with the granting owner's address, so it can revive when the NFT returns.
Third parties may supply only into asset slots the account already holds
(INV-AUTH-03), repay anyone, liquidate anyone, and run maintenance. Look for:

- any way to borrow, withdraw, swap or close against an account the caller
  neither owns nor is an active delegate of (INV-AUTH-02), including through
  `Credit(0)` receivers, `account_id = 0` creation, strategies, Blend
  migration and the DeFindex adapter's per-vault accounts;
- a third-party `supply` or `liquidate` that opens a new asset slot, changes
  a risk snapshot or spoke binding, or otherwise worsens a foreign account
  beyond what the entrypoint list states (INV-AUTH-06, INV-RISK-04);
- the guardian and oracle ratchets: any immediate path that loosens a flag, a
  band, a cap or a delay (INV-AUTH-04, INV-AUTH-05);
- the pool: any mutator reachable without the controller as owner
  (INV-AUTH-01); the NFT: mint, burn or upgrade without the stored controller.

### Chained atomic execution

This is the area the unit tests cover least. Build sequences, not single
calls. For each sequence ask what one contract, in one transaction, can do
that a human in separate transactions cannot, and what a sequence spread over
ledgers with accrual between steps can do. Start from these:

- open an account with dust, borrow, then self-liquidate in Credit mode into
  a second account the attacker owns, and repeat;
- `flash_loan` returned, then supply, borrow and withdraw in the same tree
  (the guard covers only the callback window);
- `flash_position` or `multiply` followed by `swap_collateral`,
  `repay_debt_with_collateral` with `close_position`, or `withdraw`, with
  the final gates skipped for a debt-free account (INV-RISK-01);
- `liquidate` then `clean_bad_debt` on the same account, or cleanup of an
  account that holds supply and debt in the same market;
- `recapitalize`, `claim_revenue` and `update_indexes` ordered around a
  supplier's exit so that the index write-down lands on others;
- an NFT transfer or delegate change between legs to revive or outrun a
  grant;
- two controller calls whose separate price contexts could disagree, and a
  governance operation executed permissionlessly between two user legs.

Every account strategy ends with the same final gates (INV-STRAT-02). The
question is what state the tree can leave behind that those gates do not
read.

## How to exercise it

The image is offline. Every crate is in the cargo cache, the contract WASM
and all test binaries are built in `/src`.

- All tests: `cargo test --workspace --features price-aggregator/testing`.
- Integration tests: `cargo test -p test-harness`. `tests/test-harness/tests/README.md`
  maps each binary to its coverage. The builder in `tests/test-harness/src/setup`
  deploys the whole protocol with governance as the controller owner, as on
  mainnet.
- One test: `make test-match PATTERN=<substring>`. `PATTERN` is required.
- After a contract patch: `make build`, because the harness loads
  `target/wasm32v1-none/release/pool.wasm` and `position_nft.wasm`.
- Gates a patch must keep green: `make access-control-check`, `make fmt-check`,
  `make clippy`.

Write a reproducer as a test in `tests/test-harness/tests`. For a multi-call
sequence, drive it through `script_runner` as
`tests/test-harness/tests/composition/helpers.rs` does, so the test proves
atomicity. Use the real contracts; mock only the oracle
(`mock/mock-oracle`), the receivers and the router the attack controls.

## How you rate severity

A finding counts when an unprivileged address reaches it through a deployable
entrypoint and breaks an invariant in `docs/reference/invariants.md` or causes
a loss. Cite the INV id. If it needs a non-default listing or risk
configuration, name it and show that admission validation accepts it
(INV-RISK-03).

- **Critical**: theft of supplier, pool or borrower funds. Debt an account
  escapes while it keeps collateral. Shares, debt or cash credited without the
  matching transfer. Acting on an account without its authorization. A
  permanent freeze of user funds. Unbounded or market-wide is Critical; the
  same bounded to one market and a small amount is High.
- **High**: a break of INV-ACCT-10 that any address can cause. Seizure above
  what the bonus curve allows, or liquidation of a healthy account. Bad debt
  created by an accounting error, or socialized twice or never. An
  unprivileged bypass of a pause, freeze, cap, utilization ceiling or
  governance delay. A validated price that violates the configured staleness,
  agreement or band rules. Blocking liquidation, withdrawal or repayment for
  other accounts beyond one ledger.
- **Medium**: rounding in the user's favour that repeats for a profit above
  the transaction fee. Griefing that blocks one account at a cost to the
  attacker. An event that disagrees with committed state in a way a keeper
  would act on.
- **Low**: rounding drift with no profitable repetition. A trap where the
  protocol would have rejected the call anyway. Resource use inside Soroban
  limits but close to them.

A defect with no path to fund loss, unauthorized state change or blocked
recovery is a bug, not a vulnerability. Do not report it.

## How to report

- A harness test that reproduces the issue, with the exact command, the
  observed and the expected result.
- The sequence as a table: step, caller, entrypoint, arguments with units,
  and the state that matters after it. State the ledgers and accrual between
  steps.
- The violated INV id, the attacker's capabilities, the required protocol
  state, and the impact in token base units or USD WAD, with whether it is
  bounded, repeatable and recoverable.
- A minimal patch against `main` with a regression test. Keep units explicit,
  keep rounding in the protocol's favour (ADR-0003), and do not rename or
  move controller internal modules: the Certora specs and unit tests mount
  them by path.
- One root cause once, listing every entrypoint that reaches it.

## Anything to leave alone

Documented and accepted. Report one only with an impact beyond what the cited
text states.

- Anything that needs a compromised governance, role, oracle-signer or
  standalone-administrator key (STRIDE Spoof.1, Elevation.1..10).
- Fail-closed price outages that block valuation-dependent actions, including
  liquidation, and the dust-leg and LP-leg variants (DoS.1); paused debt or
  `no_seize` collateral that blocks liquidation (DoS.2).
- Debt-free accounts skipping the final numerical gates (INV-RISK-01).
- `is_collateralizable` gating new supply only; existing collateral keeps its
  value.
- Scaled-cap saturation at a supply index below one RAY (INV-HALT-03), the
  explicit rounding limits of interest allocation (INV-IDX-05), the
  supply-index floor leaving an unbacked residual (INV-IDX-02, INV-ACCT-04).
- The supply index falling on bad-debt socialization, and a supplier who
  exits before cleanup avoiding it (INV-IDX-03, ADR-0012).
- Liquidation ordering, MEV, liquidator profitability, zero seizure on tiny
  repayments, and whole-unit effects on legs below 3 decimals (Info.2, DoS.9,
  INV-LIQ-02).
- Public positions, prices and pending governance operations (Info.1, Info.3).
- Token semantics outside the threat model's "Token assumptions", unbooked
  donations, and a malicious router inside the measured bounds (Tamper.3,
  Tamper.4).
- Reentrancy claims that rely on EVM behaviour (Tamper.5).
