---
name: xoxno-lending
description: Use for XOXNO Lending tasks on Stellar: choose the contract, SDK, API, liquidation, data, or swap guidance; resolve network addresses and ids; or interpret hubs, spokes, accounts, position NFTs, health factors, WAD, RAY, and BPS.
user-invocable: true
argument-hint: "[XOXNO Lending task]"
---

# XOXNO Lending

Start here, select one `xoxno-*` task skill, then load only the companion
reference needed for the task. Reserve `evals/` for skill testing.

## Route the task

| You are… | Load |
|---|---|
| Writing a Soroban contract that supplies, borrows, holds a position, or receives a flash loan | [../xoxno-lending-contracts/SKILL.md](../xoxno-lending-contracts/SKILL.md) |
| Building a dApp, backend, or bot in TypeScript on the SDK and the REST API | [../xoxno-lending-sdk/SKILL.md](../xoxno-lending-sdk/SKILL.md) |
| Swapping tokens, or embedding a swap inside a lending action (leverage, collateral/debt swap, repay with collateral) | [../xoxno-swap-aggregator/SKILL.md](../xoxno-swap-aggregator/SKILL.md) |
| Building a liquidation bot, keeper, or risk monitor | [../xoxno-lending-liquidations/SKILL.md](../xoxno-lending-liquidations/SKILL.md) |
| Indexing events, building analytics, or calling the REST API from any language | [../xoxno-lending-data/SKILL.md](../xoxno-lending-data/SKILL.md) |
| Debugging a failed simulation or transaction, or setting up testnet | [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md) |

Selection is complete when one row covers the primary task. For a composed flow,
load the lending skill first and the swap skill only for its quote or payload.

## Load shared references on demand

| Task | File |
|------|------|
| Contract addresses, RPC and API endpoints, hub ids, spoke ids, listed markets and their token contracts | [addresses.md](addresses.md) (generated from `configs/`) |
| API `supplyAmount` / RAY → token units | [math.md](math.md#api-position-fields-are-ray-quantities) |
| Full repay / withdraw sizing and share rounding | [math.md](math.md#shares-and-token-amounts) |
| Health factor, LTV weights | [math.md](math.md#health-factor-and-ltv-weighting) |
| Borrow curve, deposit rate, APR vs APY | [math.md](math.md#borrow-rate-curve) |

Reference selection is complete when every interpreted value has an explicit
unit and every market identifier includes both `hub_id` and asset address.

## Shared model

- The **Controller is the only user-facing lending contract**. Integrations use
  it for accounts and lending actions, then discover the pool with
  `get_pool_address()`. The swap aggregator is separately user-facing and
  directly callable for standalone swaps.
- A market is `HubAssetKey { hub_id, asset }`; the same token in two hubs is two
  isolated markets. Carry both fields through reads, quotes, and transactions.
- A spoke is an account's lifetime risk profile and listing set. Read its
  current configuration before opening a position.
- A position belongs to a `u64` account id. Its position NFT
  (`token_id == account_id`) is the live ownership authority.
- Token transfers and caps use asset base units; USD values and health factors
  use WAD (`1e18`); shares, indexes, and annual rates use RAY (`1e27`); risk
  ratios and fees use BPS (`10_000 = 100%`).

## Execute safely

1. Resolve the network, contract address, `hub_id`, asset address, and
   `spoke_id` from [addresses.md](addresses.md) or live configuration.
2. Load the routed task skill and its task-specific reference. Use
   [math.md](math.md) whenever conversion, accrual, health, close sizing, or
   rounding affects the result.
3. Build with integer arithmetic and the authorization model documented by the
   task skill. Simulate the exact transaction before requesting a signature.
4. Complete the task only when the network and ids are explicit, units and
   rounding are identified, authorization is satisfied, simulation succeeds,
   and the routed skill's completion criteria pass.

For generic Stellar storage, auth, and contract testing, use the `stellar-dev`
`smart-contracts` skill. For wallet signing and submission, use its `dapp`
skill; for RPC or Horizon access, use its `data` skill.
