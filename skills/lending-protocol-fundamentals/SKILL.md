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
  Same shape for per-listing halt flags: GUARDIAN `set_spoke_asset_flags`
  **ratchets** (may only tighten `paused`/`frozen`); clearing those flags is
  timelocked via `AdminOperation::EditAssetInSpoke`.
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
  `account_id == 0` creates an account and returns the id. Creation mints a
  **position NFT** ([`contracts/position-nft`](../../contracts/position-nft/README.md),
  token id == account id) to the
  creator; the account's owner is whoever holds that NFT. The controller
  resolves ownership live via `owner_of` on every account access (never
  cached), and transfer is a standard NFT operation the controller does not
  gate — account authority moves with the token, immediately. One address
  can own many accounts.
- Each account binds at creation to a **spoke** (`spoke_id: u32`) — its risk
  configuration (LTV, liquidation thresholds/bonuses, caps, and per-asset
  pause/freeze flags). The spoke is immutable after creation. **Spoke ids start
  at 1**; `spoke_id == 0` does not exist and account creation with it reverts
  `SpokeNotFound`. Read `get_spoke` / `get_spoke_asset` before choosing.

  Halt controls are layered:
  - Global controller pause (immediate): blocks risk-increasing actions
    (supply, borrow, strategies, flash loans, the index/revenue/threshold
    keeper verbs, and `add_delegate`) but leaves withdraw, repay, liquidate,
    clean_bad_debt, renew_account, `remove_delegate`, and `recapitalize`
    open. Revoking authority and injecting cash are never blocked.
    Recovery: timelocked `Unpause`.
  - Per-spoke-asset `paused`: blocks supply/borrow/withdraw/repay for that
    listing (both entries and exits). Immediate GUARDIAN may only set (not
    clear) via `set_spoke_asset_flags`; clearing is timelocked
    `EditAssetInSpoke`.
  - Per-spoke-asset `frozen`: blocks only new supply/borrow; exits remain
    possible. Same dual path as `paused` (ratchet on immediate flags API;
    clear only via timelocked edit).
  - Per-spoke-asset `no_seize`: blocks only the liquidation seizure leg
    (`SpokeAssetSeizureHalted` #318). It is the only flag that gates
    seizure; `paused` and `frozen` do not, because seizure is pro-rata over
    the whole collateral set.
  - Per-spoke-asset `supply_cap` / `borrow_cap`: always-enforced ceilings in
    asset units, orthogonal to the flags above. There is no unlimited
    sentinel — `0` means that side accepts nothing, and exits stay uncapped,
    so a zero cap is a soft wind-down rather than a freeze.
  Liquidations and clean_bad_debt survive global pause and frozen (narrow
  exception: repay leg on a paused debt listing reverts — "tainted debt").
- The owner can `add_delegate` / `remove_delegate`: a delegate may act on
  owner-gated verbs, but only while also registered as an active position
  manager by governance **and** while the granting owner still holds the
  account's position NFT — transferring the NFT lazily revokes every
  delegate the prior owner granted, with no revocation call needed.

## Units

| Value | Scale |
|---|---|
| Token amounts | native asset decimals, `i128` |
| Health factor, USD values | WAD = 1e18 |
| Interest indexes | RAY = 1e27 |
| Interest rates | RAY = 1e27. `get_borrow_rate` / `get_deposit_rate` return an **annual** APR; accrual internally compounds the per-millisecond form (`annual / MILLISECONDS_PER_YEAR`) |
| Risk ratios (LTV, thresholds, bonuses, fees) | basis points (10_000 = 100%) |

**Scaled balances**: Positions store scaled shares (not token amounts).
`scaled * current_index / RAY` is still in the 27-decimal RAY domain; rescale
27 → asset decimals to get token units, or you are off by `10^(27-decimals)`.
Indexes only increase on normal accrual (supply index has a floor during
bad-debt socialization).

Health factor < 1 WAD means liquidatable. `get_health_factor` returns
`i128::MAX` for debt-free accounts, missing accounts, and dust-debt accounts
whose ratio saturates.

## Payment semantics

- `repay` is permissionless: only caller auth (the caller funds the transfer).
  Anyone may repay any account's debt without the owner's consent, since repay
  only reduces debt. Overpayment refunds go to the caller/payer.
  Global pause does not block repay; spoke pause on the debt listing does.
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

Normative math: `docs/reference/formulas.md` (must match contracts). Behavior
also lives in contracts, interfaces, and tests. Layer skills cover auth,
flash-loan receivers, liquidation bots, indexing, and the SDK. Treat network
and contract addresses as configuration — never hardcode.
