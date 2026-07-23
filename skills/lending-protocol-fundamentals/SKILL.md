---
name: lending-protocol-fundamentals
description: Use when working with the XOXNO Lending protocol on Stellar Soroban in any capacity — before integrating contracts, SDK, bots, or indexers — or when confused about hubs, spokes, accounts, health factor, units (WAD/RAY/BPS), or which contract to call.
---

# XOXNO Lending Fundamentals

Shared model every integration layer builds on. The layer-specific skills
(`integrating-lending-from-soroban-contracts`, `using-lending-sdk`,
`reading-lending-protocol-state`, `building-lending-liquidation-bots`,
`writing-flash-loan-receivers`, `indexing-lending-events`) assume this one.

## Architecture

Three core contracts (strict ownership chain: Governance owns Controller owns Pool):

- **Governance** — owns the controller; timelocks admin changes. GUARDIAN can
  **pause** immediately; **unpause is timelocked** (`AdminOperation::Unpause`).
- **Controller** — the only user-facing contract: accounts, risk checks,
  oracle validation, liquidations, flash loans, and strategies. It is the
  sole caller of the pool for all mutations.
- **Pool** — single central liquidity contract. Mutating entrypoints are
  `#[only_owner]` (controller only); views are public. No risk, solvency, or
  oracle logic in the pool.

Fresh deployments start paused; resume after configuration via the timelock.

## Markets: HubAssetKey

Every market is keyed by `HubAssetKey { hub_id: u32, asset: Address }`. The
same token listed on two hubs is **two fully isolated markets** (separate
indexes, cash, revenue, debt, and bad-debt socialization) — never identify a
market by asset address alone; always carry `hub_id`. Hubs provide complete
isolation.

## Accounts, spokes, delegates

- Positions belong to a `u64` **account id**, not an address. `supply` with
  `account_id == 0` creates an account and returns the id. One address can
  own many accounts.
- Each account binds at creation to a **spoke** (`spoke_id: u32`) — its risk
  configuration (LTV, liquidation thresholds/bonuses, caps, and per-asset
  pause/freeze flags). The spoke is immutable after creation. **Spoke ids start
  at 1**; `spoke_id == 0` does not exist and account creation with it reverts
  `SpokeNotFound`. Read `get_spoke` / `get_spoke_asset` before choosing.

  Halt controls are layered:
  - Global controller pause (immediate): blocks risk-increasing actions
    (supply, borrow, most strategies, flash loans, update_indexes, etc.) but
    leaves withdraw, repay, liquidate, and clean_bad_debt open.
  - Per-spoke-asset `paused`: blocks supply/borrow/withdraw/repay for that
    listing (including exits).
  - Per-spoke-asset `frozen`: blocks only new supply/borrow; exits remain
    possible.
  Liquidations and clean_bad_debt survive global pause and frozen (narrow
  exception: repay leg on a paused debt listing reverts — "tainted debt").
- The owner can `add_delegate` / `remove_delegate`: a delegate may act on
  owner-gated verbs, but only while also registered as an active position
  manager by governance.

## Units

| Value | Scale |
|---|---|
| Token amounts | native asset decimals, `i128` |
| Health factor, USD values | WAD = 1e18 |
| Interest indexes, rates | RAY = 1e27 (rates are per **millisecond**) |
| Risk ratios (LTV, thresholds, bonuses, fees) | basis points (10_000 = 100%) |

**Scaled balances**: Positions store scaled shares (not token amounts). Actual
balance = `scaled * current_index / RAY`. Indexes only increase on normal
accrual (supply index has a floor during bad-debt socialization).

Health factor < 1 WAD means liquidatable. `get_health_factor` returns
`i128::MAX` for debt-free accounts, missing accounts, and dust-debt accounts
whose ratio saturates.

## Payment semantics

- `repay` is permissionless and refunds overpayment to the payer.
- `liquidate` only pulls the accepted close amounts from the liquidator;
  amounts above the cap are never transferred. Protocol fee is taken on the
  bonus; bad debt may be socialized if collateral <= 5 WAD USD threshold.
- `withdraw` with amount `0` closes the position and pays its full value;
  it returns the actual amounts paid, which can differ from the request.
- `supply`/`repay` (and strategy equivalents) require the caller to pre-authorize
  the exact token transfer to the pool for the next sub-invocation.

Never call the Pool contract directly from user or integrator code.

## Addresses and networks — never hardcode

Contract addresses, RPC endpoints, and network choice are deployment
configuration, not constants:

- Deployed addresses per network are published in `configs/networks.json` of
  the protocol repository (github.com/XOXNO/rs-lending-xlm) — the single
  source of truth.
- Off-chain code resolves addresses from environment/config (the SDK reads
  env vars and throws when unset — see `using-lending-sdk`).
- On-chain integrations take the controller address as a constructor or
  config parameter and discover the pool via `get_pool_address()`.

Docs and examples should treat network (`testnet`/`mainnet`) as a variable.

## Errors

Contract errors live in `common/src/errors.rs` of the protocol repo, grouped
as `GenericError`, `CollateralError`, `OracleError`, `SpokeError`,
`FlashLoanError`, `StrategyError`. Off-chain, map raw Soroban error codes to
these names with the SDK's `mapSorobanError`.

Normative rules: `docs/reference/invariants.md`. Topology:
`docs/reference/architecture.md`. Layer skills cover auth, flash-loan
receivers, liquidation bots, indexing, and the SDK. Treat network and contract
addresses as configuration — never hardcode.
