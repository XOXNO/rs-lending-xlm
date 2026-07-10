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

Three core contracts:

- **Governance** — owns the controller; timelocks all admin changes.
- **Controller** — the only user-facing contract: accounts, risk checks,
  oracle validation, liquidations, flash loans, strategies.
- **Pool** — single central liquidity contract, owned by the controller. Its
  mutating entrypoints are `only_owner`: never call the pool directly; its
  read-only views are open to everyone.

## Markets: HubAssetKey

Every market is keyed by `HubAssetKey { hub_id: u32, asset: Address }`. The
same token listed on two hubs is **two isolated markets** — never identify a
market by asset address alone; always carry `hub_id`.

## Accounts, spokes, delegates

- Positions belong to a `u64` **account id**, not an address. `supply` with
  `account_id == 0` creates an account and returns the id. One address can
  own many accounts.
- Each account binds at creation to a **spoke** (`spoke_id: u32`) — its risk
  configuration (LTV, liquidation thresholds/bonuses, caps, pause/freeze
  flags per asset). The spoke is immutable after creation. **Spoke ids start
  at 1**; `spoke_id == 0` does not exist and account creation with it reverts
  `SpokeNotFound`. Read `get_spoke` / `get_spoke_asset` before choosing.
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

Health factor < 1 WAD means liquidatable. `get_health_factor` returns
`i128::MAX` for debt-free accounts, missing accounts, and dust-debt accounts
whose ratio saturates.

## Payment semantics

- `repay` is permissionless and refunds overpayment to the payer.
- `liquidate` only pulls the accepted close amounts from the liquidator;
  amounts above the cap are never transferred.
- `withdraw` with amount `0` closes the position and pays its full value;
  it returns the actual amounts paid, which can differ from the request.

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
